//! A game's music, told apart from the rest of its audio.
//!
//! Many PlayStation games play their music straight off the disc, as CD
//! audio tracks after the data. Those tracks are not all music, though: the
//! same disc carries the characters' lines, short stings for a secret found,
//! and sometimes a long silent track at the end. Nothing on the disc says
//! which is which, so this listens:
//!
//! - **Silence** is next to no signal at all.
//! - **Speech** is mono and broken by pauses: a voice recorded once and put
//!   in both channels, so left and right are all but the same, with quiet
//!   between the words. Music is mixed in stereo, and where it is nearly
//!   mono - a loud club mix, its bass in the middle - it does not pause.
//! - **A sting** is short, a few seconds of fanfare.
//! - **Music** is the rest.
//!
//! Mono music with pauses would be taken for speech, and a game that mixes
//! its voices in stereo would have them taken for music; on the discs tried
//! so far neither happens, and both only mean a track shown or hidden that
//! should not be.
//!
//! A PlayStation 2 game has no CD audio: its music is files. Some keep it
//! as plain WAV in a folder of its own - Midnight Club's `MUSIC/LONDON1.WAV`
//! and the rest - and those are found, heard the same way, and played as
//! they are. Music in Sony's ADPCM, or packed into a game's archives, is not.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::disc::{Disc, RAW_SECTOR, SECTOR, Toc};
use crate::{Result, iso9660};

/// CD audio's frames per second.
const CD_RATE: u32 = 44_100;
/// Quieter than this, as RMS of full scale, is silence: well under the
/// noise of any recording.
const SILENT: f64 = 0.001;
/// Left and right closer than this - what differs between them, against
/// what they share - is one voice in both channels. Speech measures about
/// 0.02; music mostly upwards of 0.15, but a loud mix as low as 0.05.
const MONO: f64 = 0.1;
/// Frames in one stretch of listening, for pauses: about 45 ms.
const WINDOW: u32 = 2048;
/// A stretch quieter than this, as RMS of full scale, is a pause.
const PAUSE: f64 = 0.01;
/// Speech pauses for upwards of 0.3 of its stretches; music, under 0.2.
const PAUSES: f64 = 0.25;
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
    /// The stretch being heard, and the stretches heard and paused in.
    window: f64,
    windows: u32,
    pauses: u32,
}

/// Root mean square of a sum of squared mids, as a fraction of full scale.
/// Mid is the sum of the channels, so twice a channel's level.
fn level(mid: f64, frames: f64) -> f64 {
    (mid / frames.max(1.0)).sqrt() / 2.0 / 32768.0
}

impl Ear {
    /// Interleaved stereo, 16-bit little-endian.
    fn hear(&mut self, pcm: &[u8]) {
        for frame in pcm.as_chunks::<4>().0 {
            let left = f64::from(i16::from_le_bytes([frame[0], frame[1]]));
            let right = f64::from(i16::from_le_bytes([frame[2], frame[3]]));
            let mid = (left + right) * (left + right);
            self.mid += mid;
            self.side += (left - right) * (left - right);
            self.window += mid;
            self.frames += 1;
            if self.frames.is_multiple_of(u64::from(WINDOW)) {
                self.windows += 1;
                self.pauses += u32::from(level(self.window, f64::from(WINDOW)) < PAUSE);
                self.window = 0.0;
            }
        }
    }

    fn kind(&self, rate: u32) -> Kind {
        let mono = (self.side / self.mid).sqrt() < MONO;
        let pausing = f64::from(self.pauses) / f64::from(self.windows.max(1)) > PAUSES;
        if level(self.mid, self.frames as f64) < SILENT {
            Kind::Silence
        } else if mono && pausing {
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

/// Folders of the root a game might keep its music files in.
const FOLDERS: [&str; 8] = [
    "MUSIC", "BGM", "SOUND", "SOUNDS", "AUDIO", "STREAM", "STREAMS", "SND",
];
/// Cooked sectors read at a time while listening to a file: a megabyte.
const FILE_READ: u32 = 512;

/// A piece of music kept as a file: stereo 16-bit PCM, as a WAV holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicFile {
    /// As the disc names it, without its extension.
    pub name: String,
    /// The file's first sector.
    pub lba: u32,
    /// Where in the file its samples start, and how many bytes they run to.
    pub offset: u32,
    pub bytes: u32,
    pub rate: u32,
}

impl MusicFile {
    pub fn frames(&self) -> u32 {
        self.bytes / 4
    }

    /// Up to `len` bytes of samples from `from`, cut out of the sectors
    /// that hold them.
    pub fn read(&self, disc: &mut dyn Disc, from: u32, len: u32) -> Result<Vec<u8>> {
        let len = len.min(self.bytes.saturating_sub(from)) as usize;
        let start = self.offset + from;
        let skip = (start as usize) % SECTOR;
        let count = (skip + len).div_ceil(SECTOR) as u32;
        let data = disc.read(self.lba + start / SECTOR as u32, count)?;
        Ok(data[skip..skip + len].to_vec())
    }
}

/// Where a WAV's samples are, if it is stereo 16-bit PCM and says so within
/// its first sector: rate, offset and length.
fn wav(head: &[u8]) -> Option<(u32, u32, u32)> {
    if head.get(..4)? != b"RIFF" || head.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut format = None;
    let mut at = 12;
    while at + 8 <= head.len() {
        let id = &head[at..at + 4];
        let size = iso9660::u32le(head, at + 4);
        let body = at + 8;
        if id == b"fmt " {
            let field = |o: usize| head.get(body + o..body + o + 2);
            let pcm = field(0)? == [1, 0] && field(2)? == [2, 0] && field(14)? == [16, 0];
            format = pcm.then(|| iso9660::u32le(head, body + 4));
        } else if id == b"data" {
            return Some((format?, body as u32, size));
        }
        // Chunks are padded to an even length.
        at = body + size as usize + (size as usize & 1);
    }
    None
}

/// The music a game keeps as WAV files in a folder of the disc's root, in
/// the disc's order. Speech, stings and silence are left out, as are files
/// at another rate than the first piece's, so all of it plays as one.
pub fn music_files(disc: &mut dyn Disc) -> Result<Vec<MusicFile>> {
    let mut read = |lba, count| disc.read(lba, count);
    let Some(volume) = iso9660::read_iso(&mut read)? else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for folder in FOLDERS {
        let Some(files) = iso9660::read_root_dir(&mut read, &volume, folder)? else {
            continue;
        };
        for file in files.iter().filter(|f| !f.directory && f.size > 0) {
            let Some((rate, offset, bytes)) = wav(&read(file.lba, 1)?) else {
                continue;
            };
            let name = file.name.rsplit_once('.').map_or(&*file.name, |(n, _)| n);
            found.push(MusicFile {
                name: name.to_string(),
                lba: file.lba,
                offset,
                // A header that claims more than the file holds is held to it.
                bytes: bytes.min(file.size.saturating_sub(offset)) & !3,
                rate,
            });
        }
    }
    let mut music = Vec::new();
    for file in found {
        if music
            .first()
            .is_some_and(|m: &MusicFile| m.rate != file.rate)
        {
            continue;
        }
        let mut ear = Ear::default();
        let step = FILE_READ * SECTOR as u32;
        for from in (0..file.bytes).step_by(step as usize) {
            ear.hear(&file.read(disc, from, step)?);
        }
        if ear.kind(file.rate) == Kind::Music {
            music.push(file);
        }
    }
    Ok(music)
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

    /// A tone that stops as often as it sounds, as speech pauses.
    fn words(n: u32) -> i16 {
        if (n / 8192).is_multiple_of(2) {
            tone(n, 90)
        } else {
            0
        }
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
    fn one_voice_in_both_channels_with_pauses_is_speech() {
        assert_eq!(heard(30, |n| (words(n), words(n))), Kind::Speech);
        // A little difference, as a voice recorded once picks up.
        assert_eq!(heard(30, |n| (words(n), words(n) / 50 * 49)), Kind::Speech);
    }

    #[test]
    fn nearly_mono_without_pauses_is_music() {
        // A loud mix with its bass in the middle.
        let mix = |n| (tone(n, 90), tone(n, 90) / 20 * 19);
        assert_eq!(heard(30, mix), Kind::Music);
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
            bin.extend(second(CD_RATE, |n| (words(n), words(n))));
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

    /// A WAV of `seconds` of stereo at `rate`, with a chunk before the
    /// samples, as tools often write.
    fn wav_file(rate: u32, seconds: u32, frame: impl Fn(u32) -> (i16, i16)) -> Vec<u8> {
        let samples: Vec<u8> = (0..seconds).flat_map(|_| second(rate, &frame)).collect();
        let mut fmt = vec![1, 0, 2, 0];
        fmt.extend(rate.to_le_bytes());
        fmt.extend((rate * 4).to_le_bytes());
        fmt.extend([4, 0, 16, 0]);
        let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
        for (id, body) in [(b"fmt ", &fmt), (b"LIST", &vec![0; 5]), (b"data", &samples)] {
            out.extend(id);
            out.extend((body.len() as u32).to_le_bytes());
            out.extend(body);
            if body.len() % 2 == 1 {
                out.push(0);
            }
        }
        out
    }

    #[test]
    fn a_wav_says_where_its_samples_are() {
        let file = wav_file(48_000, 1, |_| (0, 0));
        // 12 of RIFF, 8 + 16 of fmt, 8 + 5 + 1 of LIST, then data's header.
        assert_eq!(wav(&file[..SECTOR]), Some((48_000, 58, 192_000)));
        // Mono, or 8-bit, is not what the player plays.
        let mut mono = file.clone();
        mono[22] = 1;
        assert_eq!(wav(&mono[..SECTOR]), None);
        assert_eq!(wav(b"RIFF\0\0\0\0AVI LIST"), None);
    }

    #[test]
    fn music_files_are_found_in_their_folder_and_speech_left_out() {
        use crate::testdisc::{MemDisc, record};
        let rate = 48_000;
        let song = wav_file(rate, 25, |n| (tone(n, 90), tone(n, 130)));
        let voice = wav_file(rate, 25, |n| (words(n), words(n)));
        let sectors = |b: &Vec<u8>| b.len().div_ceil(SECTOR);
        let (song_at, voice_at) = (40, 40 + sectors(&song));
        let mut disc =
            MemDisc::new(crate::Media::Cd, voice_at + sectors(&voice)).iso("GAME", &[], &["MUSIC"]);
        let mut dir = record(19, SECTOR as u32, true, &[0]);
        dir.extend(record(18, SECTOR as u32, true, &[1]));
        dir.extend(record(
            song_at as u32,
            song.len() as u32,
            false,
            b"SONG.WAV;1",
        ));
        dir.extend(record(
            voice_at as u32,
            voice.len() as u32,
            false,
            b"VOICE.WAV;1",
        ));
        disc.sector(19)[..dir.len()].copy_from_slice(&dir);
        disc.sectors[song_at * SECTOR..][..song.len()].copy_from_slice(&song);
        disc.sectors[voice_at * SECTOR..][..voice.len()].copy_from_slice(&voice);

        let music = music_files(&mut disc).unwrap();
        assert_eq!(music.len(), 1);
        let song_file = &music[0];
        assert_eq!(
            (song_file.name.as_str(), song_file.rate, song_file.frames()),
            ("SONG", rate, 25 * rate)
        );
        // Samples cut out across a sector boundary are the file's own.
        let at = 2000;
        let start = song_file.offset as usize + at;
        assert_eq!(
            song_file.read(&mut disc, at as u32, 100).unwrap(),
            song[start..start + 100]
        );
    }
}
