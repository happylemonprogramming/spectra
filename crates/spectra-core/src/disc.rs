use serde::Serialize;

use crate::Result;

/// A cooked data sector: user data only, no sync, header or error correction.
pub const SECTOR: usize = 2048;
/// A raw CD sector: 588 stereo frames of 16-bit PCM, or a data sector with
/// everything around its 2048 bytes.
pub const RAW_SECTOR: usize = 2352;
/// The two-second gap before track 1. Drives count logical blocks from after
/// it; the CD standard, and MusicBrainz, count from before it.
pub const PREGAP: u32 = 150;

/// Something a disc can be read from: a drive, or an image of one.
///
/// Addresses are logical block addresses as a drive uses them: LBA 0 is the
/// first sector of track 1's user area.
pub trait Disc {
    /// A human description of where this disc is: a device or a file.
    fn describe(&self) -> String;

    /// What kind of disc this is physically, as far as the source knows.
    fn media(&mut self) -> Result<Media>;

    /// The CD table of contents. DVDs and Blu-rays have a nominal one-track
    /// TOC, which sources may return or not.
    fn toc(&mut self) -> Result<Option<Toc>>;

    /// `count` cooked 2048-byte sectors from `lba`.
    fn read(&mut self, lba: u32, count: u32) -> Result<Vec<u8>>;

    /// Size of the readable area in sectors, where known.
    fn capacity(&mut self) -> Result<Option<u32>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Media {
    Cd,
    Dvd,
    BluRay,
    HdDvd,
    Unknown,
}

impl Media {
    /// From GET CONFIGURATION's current profile. Zero means the drive has no
    /// opinion, which usually means it has no disc.
    pub fn from_profile(profile: u16) -> Self {
        match profile {
            0x08..=0x0a => Self::Cd,
            0x10..=0x2b => Self::Dvd,
            0x40..=0x43 => Self::BluRay,
            0x50..=0x5a => Self::HdDvd,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Track {
    pub number: u8,
    /// Where index 1 starts, as a drive LBA.
    pub lba: u32,
    /// A data track rather than Red Book audio.
    pub data: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Toc {
    pub first: u8,
    pub last: u8,
    pub tracks: Vec<Track>,
    /// Where the lead-out starts, as a drive LBA.
    pub leadout: u32,
}

impl Toc {
    pub fn audio_tracks(&self) -> usize {
        self.tracks.iter().filter(|t| !t.data).count()
    }

    /// The track a console would boot from, if the disc has one.
    ///
    /// Usually track 1. A PC Engine CD puts an audio warning first and its
    /// data second, so any data track counts - except one that comes last
    /// after audio, which is an enhanced CD's bonus content, not a program.
    pub fn boot_track(&self) -> Option<&Track> {
        let enhanced = self.enhanced_data_track();
        self.tracks
            .iter()
            .find(|t| t.data && Some(t.number) != enhanced.map(|e| e.number))
    }

    /// The trailing data track of an enhanced CD (audio first, data last, in
    /// a second session).
    pub fn enhanced_data_track(&self) -> Option<&Track> {
        let last = self.tracks.last()?;
        let first = self.tracks.first()?;
        (last.data && !first.data && self.tracks.len() > 1).then_some(last)
    }

    /// Sectors per track, index 1 to the next track or the lead-out.
    pub fn track_sectors(&self) -> Vec<u32> {
        self.tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let end = self.tracks.get(i + 1).map_or(self.leadout, |n| n.lba);
                end.saturating_sub(t.lba)
            })
            .collect()
    }
}
