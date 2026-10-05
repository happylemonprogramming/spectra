//! FLAC, written here: a game's music saved as files any player can open.
//!
//! FLAC is lossless, so a track comes back sample for sample as it was on
//! the disc, at about half the size. It is small enough to write by hand:
//! each block of 4096 samples is guessed from the few before it - by one of
//! FLAC's five fixed guesses, whichever is closest - and only how far each
//! guess was off is written, packed short for the small misses that most
//! of them are. Two channels are written as left and right, or as one of
//! them and the difference, or as their middle and the difference: for most
//! music the difference is the quieter, and so the smaller.
//!
//! What the reference encoder adds on top - longer guesses worked out for
//! each block - makes a file a twentieth smaller for a PS1 game's music,
//! and a sixth for a loud club mix: not worth the code.

/// Samples to a block, as the reference encoder writes them.
const BLOCK: usize = 4096;
/// The longest of FLAC's fixed guesses.
const MAX_ORDER: usize = 4;
/// Finer partitions than this hardly ever pay for their headers.
const MAX_PARTITION_ORDER: u32 = 4;
/// Rice parameters are four bits, and 15 means something else.
const MAX_RICE: u32 = 14;

/// A picture to go in the file, as the bytes of a JPEG or a PNG.
pub struct Picture {
    pub data: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}

/// 16-bit stereo, as little-endian pairs - CD audio as it is on the disc -
/// as a whole FLAC file: its tags, as `NAME=value`, and its cover.
pub fn encode(
    pcm: &[u8],
    rate: u32,
    tags: &[(&str, String)],
    picture: Option<&Picture>,
) -> Vec<u8> {
    let frames = pcm.len() / 4;
    let sample = |i: usize| i32::from(i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]));
    let left: Vec<i32> = (0..frames).map(|i| sample(i * 2)).collect();
    let right: Vec<i32> = (0..frames).map(|i| sample(i * 2 + 1)).collect();

    let mut audio = Vec::with_capacity(pcm.len() / 2);
    let (mut smallest, mut largest) = (u32::MAX, 0);
    for (number, start) in (0..frames).step_by(BLOCK).enumerate() {
        let end = (start + BLOCK).min(frames);
        let before = audio.len();
        frame(
            &mut audio,
            number as u64,
            rate,
            &left[start..end],
            &right[start..end],
        );
        let size = (audio.len() - before) as u32;
        smallest = smallest.min(size);
        largest = largest.max(size);
    }

    let mut out = b"fLaC".to_vec();
    let mut info = Bits::default();
    info.put(BLOCK as u64, 16);
    info.put(BLOCK as u64, 16);
    info.put(u64::from(if largest == 0 { 0 } else { smallest }), 24);
    info.put(u64::from(largest), 24);
    info.put(u64::from(rate), 20);
    info.put(1, 3); // two channels
    info.put(15, 5); // sixteen bits
    info.put(frames as u64, 36);
    let mut info = info.finish();
    // The samples' fingerprint, as they are: a decoder checks it.
    info.extend(md5(&pcm[..frames * 4]));
    block(&mut out, 0, &info, false);

    let mut comment = Vec::new();
    let vendor = b"Spectra";
    comment.extend((vendor.len() as u32).to_le_bytes());
    comment.extend(vendor);
    comment.extend((tags.len() as u32).to_le_bytes());
    for (name, value) in tags {
        let field = format!("{name}={value}");
        comment.extend((field.len() as u32).to_le_bytes());
        comment.extend(field.as_bytes());
    }
    block(&mut out, 4, &comment, picture.is_none());

    if let Some(picture) = picture {
        let mut data = Vec::with_capacity(picture.data.len() + 64);
        data.extend(3u32.to_be_bytes()); // the front cover
        data.extend((picture.mime.len() as u32).to_be_bytes());
        data.extend(picture.mime.as_bytes());
        data.extend(0u32.to_be_bytes()); // no description
        data.extend(picture.width.to_be_bytes());
        data.extend(picture.height.to_be_bytes());
        data.extend(24u32.to_be_bytes());
        data.extend(0u32.to_be_bytes()); // not a palette
        data.extend((picture.data.len() as u32).to_be_bytes());
        data.extend(&picture.data);
        block(&mut out, 6, &data, true);
    }
    out.extend(audio);
    out
}

fn block(out: &mut Vec<u8>, kind: u8, data: &[u8], last: bool) {
    out.push(if last { 0x80 | kind } else { kind });
    out.extend(&(data.len() as u32).to_be_bytes()[1..]);
    out.extend(data);
}

/// How the two channels go into a frame, by FLAC's numbers for them.
#[derive(Clone, Copy)]
enum Stereo {
    LeftRight = 1,
    LeftSide = 8,
    SideRight = 9,
    MidSide = 10,
}

fn frame(out: &mut Vec<u8>, number: u64, rate: u32, left: &[i32], right: &[i32]) {
    let side: Vec<i32> = left.iter().zip(right).map(|(l, r)| l - r).collect();
    let mid: Vec<i32> = left.iter().zip(right).map(|(l, r)| (l + r) >> 1).collect();
    let (l, r) = (Plan::best(left, 16), Plan::best(right, 16));
    let (s, m) = (Plan::best(&side, 17), Plan::best(&mid, 16));
    let (stereo, first, second) = [
        (Stereo::LeftRight, (&l, left), (&r, right)),
        (Stereo::LeftSide, (&l, left), (&s, &side[..])),
        (Stereo::SideRight, (&s, &side[..]), (&r, right)),
        (Stereo::MidSide, (&m, &mid[..]), (&s, &side[..])),
    ]
    .into_iter()
    .min_by_key(|(_, a, b)| a.0.bits + b.0.bits)
    .expect("four ways");

    let start = out.len();
    let mut head = Bits::default();
    head.put(0xFFF8, 16); // in sync, blocks of a fixed size
    head.put(0b0111, 4); // the size at the end of the header
    head.put(
        match rate {
            44_100 => 0b1001,
            48_000 => 0b1010,
            _ => 0, // as STREAMINFO says
        },
        4,
    );
    head.put(stereo as u64, 4);
    head.put(0b100, 3); // sixteen bits
    head.put(0, 1);
    let mut head = head.finish();
    head.extend(utf8(number));
    head.extend(((left.len() - 1) as u16).to_be_bytes());
    head.push(crc8(&head));
    out.extend(head);

    let mut body = Bits::default();
    first.0.write(&mut body, first.1);
    second.0.write(&mut body, second.1);
    out.extend(body.finish());
    let crc = crc16(&out[start..]);
    out.extend(crc.to_be_bytes());
}

/// How a channel's block is best written, and how many bits that takes.
struct Plan {
    bits: u64,
    bps: u32,
    how: How,
}

enum How {
    Constant,
    Verbatim,
    Fixed {
        order: usize,
        partitions: u32,
        rice: Vec<u32>,
    },
}

impl Plan {
    fn best(samples: &[i32], bps: u32) -> Self {
        let n = samples.len();
        if samples.iter().all(|&s| s == samples[0]) {
            return Self {
                bits: 8 + u64::from(bps),
                bps,
                how: How::Constant,
            };
        }
        let verbatim = Self {
            bits: 8 + n as u64 * u64::from(bps),
            bps,
            how: How::Verbatim,
        };
        // The guess that misses by least overall; then how best to pack
        // its misses.
        let order = (0..=MAX_ORDER.min(n - 1))
            .min_by_key(|&order| {
                residuals(samples, order)
                    .map(|r| r.unsigned_abs() as u64)
                    .sum::<u64>()
            })
            .unwrap_or(0);
        let packed: Vec<u32> = residuals(samples, order).map(zigzag).collect();
        let (partitions, rice, bits) = (0..=MAX_PARTITION_ORDER)
            .take_while(|&p| n.is_multiple_of(1 << p) && n >> p > order)
            .map(|p| partition(&packed, n, order, p))
            .min_by_key(|(_, _, bits)| *bits)
            .expect("order 0 always fits");
        let fixed = 8 + order as u64 * u64::from(bps) + 6 + bits;
        if fixed >= verbatim.bits {
            return verbatim;
        }
        Self {
            bits: fixed,
            bps,
            how: How::Fixed {
                order,
                partitions,
                rice,
            },
        }
    }

    fn write(&self, out: &mut Bits, samples: &[i32]) {
        let raw =
            |out: &mut Bits, s: i32| out.put(u64::from(s as u32) & ((1 << self.bps) - 1), self.bps);
        match &self.how {
            How::Constant => {
                out.put(0, 8);
                raw(out, samples[0]);
            }
            How::Verbatim => {
                out.put(0b0000_0010, 8);
                samples.iter().for_each(|&s| raw(out, s));
            }
            How::Fixed {
                order,
                partitions,
                rice,
            } => {
                out.put(((0b1000 | *order) << 1) as u64, 8);
                samples[..*order].iter().for_each(|&s| raw(out, s));
                out.put(0, 2); // four-bit Rice parameters
                out.put(u64::from(*partitions), 4);
                let mut packed = residuals(samples, *order).map(zigzag);
                let each = samples.len() >> partitions;
                for (i, &k) in rice.iter().enumerate() {
                    out.put(u64::from(k), 4);
                    let count = if i == 0 { each - order } else { each };
                    for u in packed.by_ref().take(count) {
                        out.zeros(u >> k);
                        out.put(1, 1);
                        out.put(u64::from(u) & ((1 << k) - 1), k);
                    }
                }
            }
        }
    }
}

/// How far each of FLAC's fixed guesses of an order is off: none at order
/// 0, the last sample at 1, the line through the last two at 2, and so on.
fn residuals(samples: &[i32], order: usize) -> impl Iterator<Item = i32> + '_ {
    (order..samples.len()).map(move |i| {
        let x = |back: usize| i64::from(samples[i - back]);
        let r = match order {
            0 => x(0),
            1 => x(0) - x(1),
            2 => x(0) - 2 * x(1) + x(2),
            3 => x(0) - 3 * x(1) + 3 * x(2) - x(3),
            _ => x(0) - 4 * x(1) + 6 * x(2) - 4 * x(3) + x(4),
        };
        r as i32
    })
}

/// A miss as a count from zero: 0, -1, 1, -2, 2 are 0, 1, 2, 3, 4.
fn zigzag(r: i32) -> u32 {
    ((r << 1) ^ (r >> 31)) as u32
}

/// The misses cut into 2^p runs, each with the Rice parameter that packs
/// it smallest: the parameters, and the bits all of it takes.
fn partition(packed: &[u32], n: usize, order: usize, p: u32) -> (u32, Vec<u32>, u64) {
    let each = n >> p;
    let mut rice = Vec::with_capacity(1 << p);
    let mut bits = 0;
    let mut at = 0;
    for i in 0..1usize << p {
        let count = if i == 0 { each - order } else { each };
        let run = &packed[at..at + count];
        at += count;
        let (k, cost) = best_rice(run);
        rice.push(k);
        bits += 4 + cost;
    }
    (p, rice, bits)
}

/// The Rice parameter for a run of misses, worked out from their mean -
/// the best is near its logarithm - and checked either side of it.
fn best_rice(run: &[u32]) -> (u32, u64) {
    if run.is_empty() {
        return (0, 0);
    }
    let sum: u64 = run.iter().map(|&u| u64::from(u)).sum();
    let mean = sum / run.len() as u64;
    let guess = (64 - mean.leading_zeros()).min(MAX_RICE);
    let cost = |k: u32| {
        run.iter().map(|&u| u64::from(u >> k)).sum::<u64>() + run.len() as u64 * u64::from(k + 1)
    };
    (guess.saturating_sub(1)..=(guess + 1).min(MAX_RICE))
        .map(|k| (k, cost(k)))
        .min_by_key(|(_, c)| *c)
        .expect("one at least")
}

/// A frame's number, as FLAC writes it: the way UTF-8 writes a character,
/// stretched to 36 bits.
fn utf8(n: u64) -> Vec<u8> {
    if n < 0x80 {
        return vec![n as u8];
    }
    let bytes = (2..=6).find(|&b| n < 1 << (5 * b + 1)).unwrap_or(7);
    let mut out = vec![(0xFF00u16 >> bytes) as u8 | (n >> (6 * (bytes - 1))) as u8];
    for i in (0..bytes - 1).rev() {
        out.push(0x80 | ((n >> (6 * i)) & 0x3F) as u8);
    }
    out
}

fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |mut crc, &byte| {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
        crc
    })
}

fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |mut crc, &byte| {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
        crc
    })
}

/// Bits written first to last, high bit first, as FLAC wants them.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    held: u64,
    count: u32,
}

impl Bits {
    /// The low `bits` of `value`, up to 36 of them.
    fn put(&mut self, value: u64, bits: u32) {
        if bits > 32 {
            self.put(value >> 32, bits - 32);
            self.put(value & 0xFFFF_FFFF, 32);
            return;
        }
        if bits == 0 {
            return;
        }
        self.held = (self.held << bits) | (value & ((1 << bits) - 1));
        self.count += bits;
        while self.count >= 8 {
            self.count -= 8;
            self.bytes.push((self.held >> self.count) as u8);
        }
        self.held &= (1 << self.count) - 1;
    }

    fn zeros(&mut self, mut n: u32) {
        while n > 0 {
            let some = n.min(32);
            self.put(0, some);
            n -= some;
        }
    }

    /// Out to the end of the last byte, with zeros.
    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.put(0, 8 - self.count);
        }
        self.bytes
    }
}

/// MD5, which FLAC keeps of the samples so a decoder can tell they came out
/// as they went in.
fn md5(data: &[u8]) -> [u8; 16] {
    const SHIFTS: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];
    let table: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut tail = data[data.len() / 64 * 64..].to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend((data.len() as u64 * 8).to_le_bytes());
    for chunk in data[..data.len() / 64 * 64]
        .chunks(64)
        .chain(tail.chunks(64))
    {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(table[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(SHIFTS[i / 16 * 4 + i % 4]));
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    let mut out = [0; 16];
    for (i, s) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads back what `encode` writes - not FLAC at large - to be sure it
    /// comes back as it went in.
    struct Reader<'a> {
        data: &'a [u8],
        bit: usize,
    }

    impl Reader<'_> {
        fn get(&mut self, bits: u32) -> u64 {
            (0..bits).fold(0, |v, _| {
                let b = (self.data[self.bit / 8] >> (7 - self.bit % 8)) & 1;
                self.bit += 1;
                (v << 1) | u64::from(b)
            })
        }
        fn signed(&mut self, bits: u32) -> i32 {
            let v = self.get(bits) as i64;
            (if v >> (bits - 1) == 1 {
                v - (1 << bits)
            } else {
                v
            }) as i32
        }
        fn align(&mut self) {
            self.bit = self.bit.div_ceil(8) * 8;
        }
    }

    fn decode(file: &[u8]) -> (Vec<(String, String)>, Vec<i16>) {
        assert_eq!(&file[..4], b"fLaC");
        let mut at = 4;
        let mut tags = Vec::new();
        let mut frames = 0;
        loop {
            let (head, len) = (
                file[at],
                u32::from_be_bytes([0, file[at + 1], file[at + 2], file[at + 3]]) as usize,
            );
            let data = &file[at + 4..at + 4 + len];
            if head & 0x7F == 0 {
                let mut r = Reader { data, bit: 0 };
                r.get(16 + 16 + 24 + 24 + 20 + 3 + 5);
                frames = r.get(36) as usize;
            }
            if head & 0x7F == 4 {
                let word =
                    |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap()) as usize;
                let mut i = 4 + word(0);
                let count = word(i);
                i += 4;
                for _ in 0..count {
                    let field = std::str::from_utf8(&data[i + 4..i + 4 + word(i)]).unwrap();
                    let (k, v) = field.split_once('=').unwrap();
                    tags.push((k.to_string(), v.to_string()));
                    i += 4 + word(i);
                }
            }
            at += 4 + len;
            if head & 0x80 != 0 {
                break;
            }
        }
        let mut r = Reader {
            data: &file[at..],
            bit: 0,
        };
        let mut out = Vec::new();
        while out.len() < frames * 2 {
            let start = r.bit / 8;
            assert_eq!(r.get(16), 0xFFF8);
            r.get(4);
            r.get(4);
            let stereo = r.get(4);
            r.get(4);
            // The frame number: as many continuation bytes as leading ones.
            let first = r.get(8) as u8;
            for _ in 1..first.leading_ones().max(1) {
                r.get(8);
            }
            let n = r.get(16) as usize + 1;
            let crc = r.get(8) as u8;
            assert_eq!(crc8(&r.data[start..r.bit / 8 - 1]), crc);
            // Which channel is the difference, a bit wider than the rest.
            let side = match stereo {
                8 | 10 => Some(1),
                9 => Some(0),
                _ => None,
            };
            let bps = |ch: usize| if side == Some(ch) { 17 } else { 16 };
            let mut ch = [Vec::new(), Vec::new()];
            for (c, samples) in ch.iter_mut().enumerate() {
                let bps = bps(c);
                r.get(1);
                let kind = r.get(6);
                r.get(1);
                match kind {
                    0 => *samples = vec![r.signed(bps); n],
                    1 => *samples = (0..n).map(|_| r.signed(bps)).collect(),
                    8..=12 => {
                        let order = (kind - 8) as usize;
                        *samples = (0..order).map(|_| r.signed(bps)).collect();
                        assert_eq!(r.get(2), 0);
                        let p = r.get(4) as u32;
                        for i in 0..1usize << p {
                            let k = r.get(4) as u32;
                            let count = (n >> p) - if i == 0 { order } else { 0 };
                            for _ in 0..count {
                                let mut q = 0;
                                while r.get(1) == 0 {
                                    q += 1;
                                }
                                let u = (q << k) | r.get(k) as u32;
                                let res = i64::from((u >> 1) as i32 ^ -((u & 1) as i32));
                                let x = |back: usize| i64::from(samples[samples.len() - back]);
                                let guess = match order {
                                    0 => 0,
                                    1 => x(1),
                                    2 => 2 * x(1) - x(2),
                                    3 => 3 * x(1) - 3 * x(2) + x(3),
                                    _ => 4 * x(1) - 6 * x(2) + 4 * x(3) - x(4),
                                };
                                samples.push((guess + res) as i32);
                            }
                        }
                    }
                    other => panic!("subframe {other}"),
                }
            }
            r.align();
            let crc = r.get(16) as u16;
            assert_eq!(crc16(&r.data[start..r.bit / 8 - 2]), crc);
            let [a, b] = ch;
            for (a, b) in a.into_iter().zip(b) {
                let (l, rr) = match stereo {
                    1 => (a, b),
                    8 => (a, a - b),
                    9 => (a + b, b),
                    _ => {
                        let mid = (a << 1) | (b & 1);
                        ((mid + b) >> 1, (mid - b) >> 1)
                    }
                };
                out.push(l as i16);
                out.push(rr as i16);
            }
        }
        (tags, out)
    }

    fn pcm(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn music_comes_back_sample_for_sample() {
        // Two tones and a little noise, a block and a bit long, so the last
        // block is a short one; then a stretch of silence.
        let mut seed = 1u32;
        let mut samples = Vec::new();
        for i in 0..(BLOCK * 3 + 777) {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let noise = (seed >> 16) as f64 / 65536.0 - 0.5;
            let t = i as f64 / 44_100.0;
            let l = 12_000.0 * (t * 440.0 * std::f64::consts::TAU).sin() + 300.0 * noise;
            let r = 9_000.0 * (t * 660.0 * std::f64::consts::TAU).sin() - 200.0 * noise;
            samples.push(if i > BLOCK * 2 { 0 } else { l as i16 });
            samples.push(if i > BLOCK * 2 { 0 } else { r as i16 });
        }
        samples[0] = i16::MIN;
        samples[1] = i16::MAX;
        let file = encode(&pcm(&samples), 44_100, &[("TITLE", "Track 2".into())], None);
        let (tags, back) = decode(&file);
        assert_eq!(tags, [("TITLE".to_string(), "Track 2".to_string())]);
        assert_eq!(back, samples);
        assert!(file.len() < samples.len() * 2, "smaller than the samples");
    }

    #[test]
    fn the_same_in_both_ears_is_written_as_one_and_no_difference() {
        let samples: Vec<i16> = (0..BLOCK as i32)
            .flat_map(|i| [(i * 7 % 3000) as i16; 2])
            .collect();
        let (_, back) = decode(&encode(&pcm(&samples), 48_000, &[], None));
        assert_eq!(back, samples);
    }

    #[test]
    fn md5_gives_the_rfcs_answers() {
        let hex = |d: [u8; 16]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(md5(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            )),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    #[test]
    fn frame_numbers_are_written_as_utf8_is() {
        assert_eq!(utf8(0x41), [0x41]);
        assert_eq!(utf8(0xE9), "é".as_bytes());
        assert_eq!(utf8(0x20AC), "€".as_bytes());
        assert_eq!(utf8(0x1F600), "😀".as_bytes());
    }
}
