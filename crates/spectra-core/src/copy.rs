//! Copying a CD into an image: one raw `.bin` for the whole disc, and a cue
//! sheet that says where its tracks start. A DVD is simpler, an `.iso` of
//! its 2048-byte sectors, but each of its VOB files may need decrypting.
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
    for (i, (track, sectors)) in toc.tracks.iter().zip(toc.track_sectors()).enumerate() {
        let audio = !track.data;
        let end = track.lba + sectors;
        // From sector 0, so sector N is always at N × 2352 in the copy: some
        // CDs start their first track a little way in, after a gap.
        let mut lba = if i == 0 { 0 } else { track.lba };
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

/// Where a DVD's sectors come from: the drive through libdvdcss, or a test's
/// fake.
pub trait DvdSectors {
    /// Get the title key for the VOB file that starts at `lba`.
    fn key(&mut self, lba: u32) -> Result<()>;
    /// `count` 2048-byte sectors, decrypted with the last key if `decrypt`.
    fn read(&mut self, lba: u32, count: u32, decrypt: bool) -> Result<Vec<u8>>;
}

/// Sectors per DVD read: 64 KB.
const DVD_BATCH: u32 = 32;
const DVD_SECTOR: usize = 2048;

/// Copy a DVD's every sector, `total` of them, in the clear. `vobs` are the
/// VOB files' sectors, start and end, in order: each is read with its own
/// key, and a read never crosses from one into another or out of one.
pub fn copy_dvd(
    disc: &mut impl DvdSectors,
    vobs: &[(u32, u32)],
    total: u32,
    out: &mut impl Write,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<Copied> {
    let mut unreadable = 0;
    let mut lba = 0;
    while lba < total {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Unsupported("the copy was cancelled".into()));
        }
        let vob = vobs.iter().find(|&&(start, end)| start <= lba && lba < end);
        let (end, decrypt) = match vob {
            Some(&(start, end)) => {
                if lba == start {
                    disc.key(start)?;
                }
                (end, true)
            }
            None => {
                let next = vobs.iter().map(|&(start, _)| start).filter(|&s| s > lba);
                (next.min().unwrap_or(total), false)
            }
        };
        let count = DVD_BATCH.min(end.min(total) - lba);
        let data = match retry(|| disc.read(lba, count, decrypt)) {
            Ok(data) if data.len() == count as usize * DVD_SECTOR => data,
            _ => {
                let mut data = Vec::with_capacity(count as usize * DVD_SECTOR);
                for n in lba..lba + count {
                    match retry(|| disc.read(n, 1, decrypt)) {
                        Ok(sector) if sector.len() == DVD_SECTOR => data.extend(sector),
                        _ => {
                            unreadable += 1;
                            data.resize(data.len() + DVD_SECTOR, 0);
                        }
                    }
                }
                data
            }
        };
        out.write_all(&data)?;
        lba += count;
        progress(Progress { done: lba, total });
    }
    out.flush()?;
    Ok(Copied {
        sectors: total,
        unreadable,
        cue: String::new(),
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
    fn a_late_first_track_keeps_its_gap() {
        let toc = Toc {
            first: 1,
            last: 1,
            tracks: vec![Track {
                number: 1,
                lba: 32,
                data: false,
            }],
            leadout: 100,
        };
        let mut out = Vec::new();
        let copied = copy_cd(
            &toc,
            |lba, count, audio| Ok(fake(lba, count, audio)),
            &mut out,
            "disc.bin",
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(out.len(), 100 * RAW_SECTOR);
        assert_eq!(out[32 * RAW_SECTOR..][..4], 32u32.to_le_bytes());
        assert!(copied.cue.contains("INDEX 01 00:00:32"));
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

    /// A scrambled DVD: each sector holds its number, XORed with the key of
    /// the VOB it is in, which only reads with that key undo.
    struct FakeDvd {
        vobs: Vec<(u32, u32)>,
        key: Option<u32>,
        keys_asked: Vec<u32>,
        reads: Vec<(u32, u32, bool)>,
        bad: u32,
    }

    impl DvdSectors for FakeDvd {
        fn key(&mut self, lba: u32) -> Result<()> {
            self.keys_asked.push(lba);
            self.key = Some(lba);
            Ok(())
        }

        fn read(&mut self, lba: u32, count: u32, decrypt: bool) -> Result<Vec<u8>> {
            self.reads.push((lba, count, decrypt));
            if (lba..lba + count).contains(&self.bad) {
                return Err(Error::Unsupported("a scratch".into()));
            }
            let mut out = Vec::new();
            for n in lba..lba + count {
                let vob = self.vobs.iter().find(|&&(s, e)| s <= n && n < e);
                let scrambled = vob.map_or(0, |&(s, _)| s);
                let undone = if decrypt { self.key.unwrap_or(0) } else { 0 };
                let mut sector = vec![0; DVD_SECTOR];
                sector[..4].copy_from_slice(&(n ^ scrambled ^ undone).to_le_bytes());
                out.extend(sector);
            }
            Ok(out)
        }
    }

    #[test]
    fn a_dvd_copies_in_the_clear_vob_by_vob() {
        let vobs = vec![(10, 50), (50, 60), (90, 100)];
        let mut disc = FakeDvd {
            vobs: vobs.clone(),
            key: None,
            keys_asked: Vec::new(),
            reads: Vec::new(),
            bad: 95,
        };
        let mut out = Vec::new();
        let copied = copy_dvd(
            &mut disc,
            &vobs,
            120,
            &mut out,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(out.len(), 120 * DVD_SECTOR);
        for n in (0..120u32).filter(|&n| n != 95) {
            let at = n as usize * DVD_SECTOR;
            assert_eq!(out[at..at + 4], n.to_le_bytes(), "sector {n}");
        }
        assert_eq!(out[95 * DVD_SECTOR..][..4], [0; 4]);
        assert_eq!(copied.unreadable, 1);
        assert_eq!(disc.keys_asked, [10, 50, 90]);
        // No read crosses into or out of a VOB.
        for &(lba, count, decrypt) in &disc.reads {
            let inside = vobs.iter().any(|&(s, e)| s <= lba && lba + count <= e);
            let outside = vobs.iter().all(|&(s, e)| lba + count <= s || e <= lba);
            assert!(if decrypt { inside } else { outside }, "{lba}+{count}");
        }
    }
}
