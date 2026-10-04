//! A game's music, told apart from the rest of its audio.
//!
//! Many PlayStation games play their music straight off the disc, as CD
//! audio tracks after the data. Those tracks are not all music, though: the
//! same disc carries the characters' lines, short stings for a secret found,
//! and sometimes a long silent track at the end. Nothing on the disc says
//! which is which, so this listens:
//!
//! - **Silence** is next to no signal at all.
//! - **Speech** is mono: a voice recorded once and put in both channels, so
//!   left and right are all but the same. Music is mixed in stereo.
//! - **A sting** is stereo but short, a few seconds of fanfare.
//! - **Music** is the rest.
//!
//! Mono music would be taken for speech, and a game that mixes its voices in
//! stereo would have them taken for music; on the discs tried so far neither
//! happens, and both only mean a track shown or hidden that should not be.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::Result;
use crate::disc::{RAW_SECTOR, Toc};

/// CD audio's frames per second.
const CD_RATE: u32 = 44_100;
/// Quieter than this, as RMS of full scale, is silence: well under the
/// noise of any recording.
const SILENT: f64 = 0.001;
/// Left and right closer than this - what differs between them, against
/// what they share - is one voice in both channels. Speech measures about
/// 0.02 and music upwards of 0.25.
const MONO: f64 = 0.1;
/// Shorter than this is a sting, not a piece of music.
const STING_SECONDS: u32 = 20;
/// Sectors read at a time: about a megabyte.
const READ: u32 = 450;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Music,
    Sting,
    Speech,
    Silence,
}

/// What has been heard of a track so far: sums enough to judge it by.
#[derive(Debug, Default)]
struct Ear {
    frames: u64,
    /// Sums of squares of what the channels share and where they differ.
    mid: f64,
    side: f64,
}

impl Ear {
    /// Interleaved stereo, 16-bit little-endian.
    fn hear(&mut self, pcm: &[u8]) {
        for frame in pcm.as_chunks::<4>().0 {
            let left = f64::from(i16::from_le_bytes([frame[0], frame[1]]));
            let right = f64::from(i16::from_le_bytes([frame[2], frame[3]]));
            self.mid += (left + right) * (left + right);
            self.side += (left - right) * (left - right);
        }
        self.frames += (pcm.len() / 4) as u64;
    }

    fn kind(&self, rate: u32) -> Kind {
        let frames = self.frames.max(1) as f64;
        // Mid is the sum of the channels, so twice a channel's level.
        let level = (self.mid / frames).sqrt() / 2.0 / 32768.0;
        if level < SILENT {
            Kind::Silence
        } else if (self.side / self.mid).sqrt() < MONO {
            Kind::Speech
        } else if self.frames < u64::from(STING_SECONDS * rate) {
            Kind::Sting
        } else {
            Kind::Music
        }
    }
}

/// What each audio track of a kept CD is, in track-list order: data tracks
/// are left out, as they are from the track list. `bin` holds every sector
/// raw, sector N at N × 2352 bytes.
pub fn sort_cd(bin: &Path, toc: &Toc) -> Result<Vec<Kind>> {
    let mut file = File::open(bin)?;
    let mut buffer = Vec::new();
    let mut kinds = Vec::new();
    for (track, sectors) in toc.tracks.iter().zip(toc.track_sectors()) {
        if track.data {
            continue;
        }
        let mut ear = Ear::default();
        file.seek(SeekFrom::Start(u64::from(track.lba) * RAW_SECTOR as u64))?;
        let mut left = sectors;
        while left > 0 {
            let count = READ.min(left);
            buffer.resize(count as usize * RAW_SECTOR, 0);
            file.read_exact(&mut buffer)?;
            ear.hear(&buffer);
            left -= count;
        }
        kinds.push(ear.kind(CD_RATE));
    }
    Ok(kinds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disc::Track;

    /// A second of stereo, from a function of the frame number.
    fn second(rate: u32, frame: impl Fn(u32) -> (i16, i16)) -> Vec<u8> {
        (0..rate)
            .flat_map(|n| {
                let (l, r) = frame(n);
                [l.to_le_bytes(), r.to_le_bytes()].concat()
            })
            .collect()
    }

    fn tone(n: u32, period: u32) -> i16 {
        let phase = n as f32 / period as f32 * std::f32::consts::TAU;
        (phase.sin() * 8000.0) as i16
    }

    fn heard(seconds: u32, frame: impl Fn(u32) -> (i16, i16)) -> Kind {
        let mut ear = Ear::default();
        let pcm = second(CD_RATE, frame);
        for _ in 0..seconds {
            ear.hear(&pcm);
        }
        ear.kind(CD_RATE)
    }

    #[test]
    fn nothing_at_all_is_silence() {
        assert_eq!(heard(5, |_| (0, 0)), Kind::Silence);
        assert_eq!(heard(5, |n| ((n % 3) as i16, 0)), Kind::Silence);
    }

    #[test]
    fn the_same_in_both_channels_is_speech() {
        assert_eq!(heard(8, |n| (tone(n, 90), tone(n, 90))), Kind::Speech);
        // A little difference, as a voice recorded once picks up.
        assert_eq!(
            heard(8, |n| (tone(n, 90), tone(n, 90) / 50 * 49)),
            Kind::Speech
        );
    }

    #[test]
    fn short_stereo_is_a_sting_and_long_stereo_music() {
        let stereo = |n| (tone(n, 90), tone(n, 130));
        assert_eq!(heard(8, stereo), Kind::Sting);
        assert_eq!(heard(30, stereo), Kind::Music);
    }

    #[test]
    fn a_kept_cd_is_sorted_track_by_track() {
        let sectors_per_second = CD_RATE / 588;
        let track = |number, lba, data| Track { number, lba, data };
        let s = |seconds| seconds * sectors_per_second;
        // Data, then music, a line of speech, and silence.
        let toc = Toc {
            first: 1,
            last: 4,
            tracks: vec![
                track(1, 0, true),
                track(2, s(2), false),
                track(3, s(32), false),
                track(4, s(37), false),
            ],
            leadout: s(40),
        };
        let mut bin = vec![0u8; s(2) as usize * RAW_SECTOR];
        for _ in 0..30 {
            bin.extend(second(CD_RATE, |n| (tone(n, 90), tone(n, 130))));
        }
        for _ in 0..5 {
            bin.extend(second(CD_RATE, |n| (tone(n, 90), tone(n, 90))));
        }
        bin.resize(s(40) as usize * RAW_SECTOR, 0);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disc.bin");
        std::fs::write(&path, bin).unwrap();
        assert_eq!(
            sort_cd(&path, &toc).unwrap(),
            [Kind::Music, Kind::Speech, Kind::Silence]
        );
    }
}
