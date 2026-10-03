//! Copying a CD into an image: one raw `.bin` for the whole disc, and a cue
//! sheet that says where its tracks start.
//!
//! Every sector is read raw, 2352 bytes, so the copy keeps what a game needs
//! beyond its files: XA audio and video in mode 2 sectors, and CD audio
//! tracks after the data. Reads go a batch at a time and never cross from a
//! data track into an audio one, since the two are asked for differently.
//!
//! A read that fails is tried again, then a sector at a time, so a scratch
//! costs only the sectors it covers. Those are written as zeros, to keep
//! everything after them at its proper offset, and counted, so whoever asked
//! for the copy can say it is not perfect.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::disc::{RAW_SECTOR, Toc};
use crate::{Error, Result};

/// Sectors per read: 27 raw sectors fit a 64 KB transfer.
const BATCH: u32 = 27;
const TRIES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    pub sectors: u32,
    /// Sectors that could not be read, and were written as zeros.
    pub unreadable: u32,
    /// The cue sheet for the copy, naming `bin` as its file.
    pub cue: String,
}

/// Copy a disc's every track. `read(lba, count, audio)` returns `count` raw
/// sectors; `cancel` stops the copy between reads.
pub fn copy_cd(
    toc: &Toc,
    mut read: impl FnMut(u32, u32, bool) -> Result<Vec<u8>>,
    out: &mut impl Write,
    bin: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<Copied> {
    if toc.enhanced_data_track().is_some() {
        return Err(Error::Unsupported(
            "copying an enhanced CD's second session".into(),
        ));
    }
    let total = toc.leadout;
    let mut unreadable = 0;
    let mut modes = Vec::with_capacity(toc.tracks.len());
    for (track, sectors) in toc.tracks.iter().zip(toc.track_sectors()) {
        let audio = !track.data;
        let end = track.lba + sectors;
        let mut lba = track.lba;
        let mut mode = None;
        while lba < end {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Unsupported("the copy was cancelled".into()));
            }
            let count = BATCH.min(end - lba);
            let data = match retry(|| read(lba, count, audio)) {
                Ok(data) if data.len() == count as usize * RAW_SECTOR => data,
                _ => {
                    // A sector at a time, to lose only the bad ones.
                    let mut data = Vec::with_capacity(count as usize * RAW_SECTOR);
                    for n in lba..lba + count {
                        match retry(|| read(n, 1, audio)) {
                            Ok(sector) if sector.len() == RAW_SECTOR => data.extend(sector),
                            _ => {
                                unreadable += 1;
                                data.resize(data.len() + RAW_SECTOR, 0);
                            }
                        }
                    }
                    data
                }
            };
            if !audio && mode.is_none() {
                // Byte 15 of a raw data sector's header is its mode.
                mode = data.get(15).copied().filter(|m| matches!(m, 1 | 2));
            }
            out.write_all(&data)?;
            lba += count;
            progress(Progress { done: lba, total });
        }
        modes.push(mode);
    }
    out.flush()?;
    Ok(Copied {
        sectors: total,
        unreadable,
        cue: cue_sheet(toc, &modes, bin),
    })
}

fn retry(mut read: impl FnMut() -> Result<Vec<u8>>) -> Result<Vec<u8>> {
    let mut last = None;
    for _ in 0..TRIES {
        match read() {
            Ok(data) => return Ok(data),
            // No disc: no point asking again.
            Err(e) if e.medium_not_present() => return Err(e),
            Err(e) => last = Some(e),
        }
    }
    Err(last.expect("tried at least once"))
}

/// A cue sheet for a single `.bin` holding every track. `modes` gives each
/// data track's mode as its first sector said, where it could be read.
pub fn cue_sheet(toc: &Toc, modes: &[Option<u8>], bin: &str) -> String {
    let mut cue = format!("FILE \"{bin}\" BINARY\n");
    for (i, track) in toc.tracks.iter().enumerate() {
        let kind = match (track.data, modes.get(i).copied().flatten()) {
            (false, _) => "AUDIO",
            (true, Some(1)) => "MODE1/2352",
            // PlayStation and Saturn discs are mode 2, and most CD games are.
            (true, _) => "MODE2/2352",
        };
        let (m, s, f) = (track.lba / 4500, track.lba / 75 % 60, track.lba % 75);
        cue.push_str(&format!(
            "  TRACK {:02} {kind}\n    INDEX 01 {m:02}:{s:02}:{f:02}\n",
            track.number
        ));
    }
    cue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disc::Track;
    use crate::{ScsiError, cue};

    fn toc() -> Toc {
        Toc {
            first: 1,
            last: 3,
            tracks: vec![
                Track {
                    number: 1,
                    lba: 0,
                    data: true,
                },
                Track {
                    number: 2,
                    lba: 60,
                    data: false,
                },
                Track {
                    number: 3,
                    lba: 4600,
                    data: false,
                },
            ],
            leadout: 4700,
        }
    }

    /// Mode 2 data sectors, and audio sectors that say where they are.
    fn fake(lba: u32, count: u32, audio: bool) -> Vec<u8> {
        let mut out = Vec::new();
        for n in lba..lba + count {
            let mut sector = vec![0u8; RAW_SECTOR];
            if audio {
                sector[..4].copy_from_slice(&n.to_le_bytes());
            } else {
                sector[15] = 2;
                // Mode 2 form 1: user data after the 8-byte subheader.
                sector[24..28].copy_from_slice(&n.to_be_bytes());
            }
            out.extend(sector);
        }
        out
    }

    #[test]
    fn every_sector_lands_at_its_own_offset() {
        let mut out = Vec::new();
        let mut last = Progress { done: 0, total: 0 };
        let copied = copy_cd(
            &toc(),
            |lba, count, audio| {
                assert_eq!(audio, lba >= 60, "asked for the wrong kind at {lba}");
                assert!(lba + count <= 60 || lba >= 60, "a read crossed a track");
                Ok(fake(lba, count, audio))
            },
            &mut out,
            "disc.bin",
            &AtomicBool::new(false),
            |p| last = p,
        )
        .unwrap();
        assert_eq!(out.len(), 4700 * RAW_SECTOR);
        assert_eq!(copied.unreadable, 0);
        assert_eq!(
            last,
            Progress {
                done: 4700,
                total: 4700
            }
        );
        let at = |lba: usize| &out[lba * RAW_SECTOR..][..RAW_SECTOR];
        assert_eq!(at(61)[..4], 61u32.to_le_bytes());
        assert_eq!(at(4650)[..4], 4650u32.to_le_bytes());
        assert_eq!(
            copied.cue,
            "FILE \"disc.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n  \
             TRACK 02 AUDIO\n    INDEX 01 00:00:60\n  TRACK 03 AUDIO\n    INDEX 01 01:01:25\n"
        );
    }

    #[test]
    fn a_bad_sector_costs_only_itself() {
        let mut out = Vec::new();
        let copied = copy_cd(
            &toc(),
            |lba, count, audio| {
                if (lba..lba + count).contains(&100) {
                    Err(ScsiError {
                        command: "read CD",
                        key: 3,
                        asc: 0x11,
                        ascq: 0,
                    }
                    .into())
                } else {
                    Ok(fake(lba, count, audio))
                }
            },
            &mut out,
            "disc.bin",
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(copied.unreadable, 1);
        assert_eq!(out.len(), 4700 * RAW_SECTOR);
        assert!(
            out[100 * RAW_SECTOR..101 * RAW_SECTOR]
                .iter()
                .all(|&b| b == 0)
        );
        assert_eq!(out[101 * RAW_SECTOR..][..4], 101u32.to_le_bytes());
    }

    #[test]
    fn the_cue_sheet_reads_back_as_the_same_disc() {
        let dir = std::env::temp_dir().join(format!("spectra-copy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bin = std::fs::File::create(dir.join("disc.bin")).unwrap();
        let copied = copy_cd(
            &toc(),
            |lba, count, audio| Ok(fake(lba, count, audio)),
            &mut bin,
            "disc.bin",
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        std::fs::write(dir.join("disc.cue"), &copied.cue).unwrap();
        let mut disc = cue::CueDisc::open(&dir.join("disc.cue")).unwrap();
        use crate::Disc;
        let back = disc.toc().unwrap().unwrap();
        assert_eq!(back.tracks, toc().tracks);
        assert_eq!(back.leadout, toc().leadout);
        // Sector 7's user data, through the image's mode 2 reader.
        assert_eq!(disc.read(7, 1).unwrap()[..4], 7u32.to_be_bytes()[..]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancelling_stops_the_copy() {
        let mut out = Vec::new();
        let cancel = AtomicBool::new(true);
        let result = copy_cd(
            &toc(),
            |lba, count, audio| Ok(fake(lba, count, audio)),
            &mut out,
            "disc.bin",
            &cancel,
            |_| {},
        );
        assert!(result.is_err());
    }
}
