//! Discs kept as copies, to play without the disc.
//!
//! Each copy is a folder under `~/.local/share/spectra/library`, named by the
//! disc's own ID - a game's serial:
//!
//! ```text
//! SLUS-00152/disc.bin    every sector, raw
//! SLUS-00152/disc.cue    where the tracks start; what an emulator opens
//! SLUS-00152/meta.json   what the disc is, written last
//! ```
//!
//! A copy is made in a folder with `.part` on the end and renamed when it is
//! whole, so a folder without it is always a finished copy; an unfinished one
//! is cleared away by the next attempt.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::copy::{self, Progress};
use crate::disc::RAW_SECTOR;
use crate::drive::Drive;
use crate::{DiscKind, Error, GameSystem, Report, Result};

pub const BIN: &str = "disc.bin";
pub const CUE: &str = "disc.cue";
const META: &str = "meta.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub title: String,
    pub system: Option<GameSystem>,
    pub serial: Option<String>,
    pub publisher: Option<String>,
    pub year: Option<String>,
    pub region: Option<String>,
    pub disc_art: Option<String>,
    pub sectors: u32,
    /// Sectors the drive could not read, kept as zeros.
    pub unreadable: u32,
    /// Seconds since 1970.
    pub created: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub dir: PathBuf,
    pub meta: Meta,
}

impl Entry {
    pub fn cue(&self) -> PathBuf {
        self.dir.join(CUE)
    }
}

/// Where copies are kept: `$XDG_DATA_HOME/spectra/library`.
pub fn root() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))?;
    Some(data.join("spectra/library"))
}

/// The ID a copy is kept under, if this disc can be kept: for now, CD games
/// with a serial.
pub fn id(report: &Report) -> Option<String> {
    let DiscKind::Game(game) = &report.kind else {
        return None;
    };
    report.toc.as_ref()?;
    let serial = game.serial.as_deref()?;
    let plain = !serial.is_empty()
        && serial
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    plain.then(|| serial.to_string())
}

/// Every finished copy, newest first.
pub fn list_in(root: &Path) -> Vec<Entry> {
    let Ok(dirs) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = dirs
        .flatten()
        .map(|e| e.path())
        .filter(|dir| dir.extension().is_none_or(|ext| ext != "part"))
        .filter_map(|dir| {
            let meta = serde_json::from_slice(&std::fs::read(dir.join(META)).ok()?).ok()?;
            Some(Entry { dir, meta })
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.meta.created));
    entries
}

pub fn list() -> Vec<Entry> {
    root().map(|r| list_in(&r)).unwrap_or_default()
}

pub fn find(id: &str) -> Option<Entry> {
    list().into_iter().find(|e| e.meta.id == id)
}

/// Copy the disc in `drive` into the library, as `report` describes it.
pub fn keep(
    drive: &Drive,
    report: &Report,
    cancel: &AtomicBool,
    progress: impl FnMut(Progress),
) -> Result<Entry> {
    let root = root().ok_or_else(|| Error::Unsupported("no home folder".into()))?;
    keep_in(
        &root,
        report,
        |lba, count, audio| drive.read_raw(lba, count, audio),
        cancel,
        progress,
    )
}

pub fn keep_in(
    root: &Path,
    report: &Report,
    read: impl FnMut(u32, u32, bool) -> Result<Vec<u8>>,
    cancel: &AtomicBool,
    progress: impl FnMut(Progress),
) -> Result<Entry> {
    let id = id(report).ok_or_else(|| Error::Unsupported("keeping a copy of this disc".into()))?;
    let toc = report.toc.as_ref().expect("id() checked for a TOC");
    let needed = u64::from(toc.leadout) * RAW_SECTOR as u64;
    std::fs::create_dir_all(root)?;
    if let Some(free) = free_space(root)
        && free < needed + needed / 20
    {
        return Err(Error::Unsupported(format!(
            "not enough space: the copy needs {} MB and {} MB is free",
            needed >> 20,
            free >> 20
        )));
    }

    let part = root.join(format!("{id}.part"));
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part)?;
    let result = (|| {
        let mut bin = BufWriter::with_capacity(1 << 20, File::create(part.join(BIN))?);
        let copied = copy::copy_cd(toc, read, &mut bin, BIN, cancel, progress)?;
        drop(bin);
        std::fs::write(part.join(CUE), &copied.cue)?;
        let game = match &report.kind {
            DiscKind::Game(game) => Some(game),
            _ => None,
        };
        let meta = Meta {
            id: id.clone(),
            title: game
                .and_then(|g| g.title.clone())
                .or_else(|| report.label.clone())
                .unwrap_or_else(|| id.clone()),
            system: game.map(|g| g.system),
            serial: game.and_then(|g| g.serial.clone()),
            publisher: game.and_then(|g| g.publisher.clone()),
            year: game.and_then(|g| g.year.clone()),
            region: game.and_then(|g| g.region.clone()),
            disc_art: game.and_then(|g| g.disc_art.clone()),
            sectors: copied.sectors,
            unreadable: copied.unreadable,
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        };
        std::fs::write(
            part.join(META),
            serde_json::to_vec_pretty(&meta).expect("meta serialises"),
        )?;
        Ok(meta)
    })();
    let meta = match result {
        Ok(meta) => meta,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&part);
            return Err(e);
        }
    };
    let dir = root.join(&id);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&part, &dir)?;
    Ok(Entry { dir, meta })
}

/// Bytes free for an unprivileged user on the filesystem holding `path`.
fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a valid C string, and a statvfs to fill in.
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disc::{Media, Toc, Track};
    use crate::{GameIdentity, identify};

    fn report() -> Report {
        let mut game = GameIdentity::new(GameSystem::Ps1);
        game.serial = Some("SLUS-00152".into());
        game.title = Some("Tomb Raider".into());
        Report {
            source: "test".into(),
            media: Media::Cd,
            kind: DiscKind::Game(game),
            label: Some("TOMBRAIDER".into()),
            sectors: Some(300),
            toc: Some(Toc {
                first: 1,
                last: 2,
                tracks: vec![
                    Track {
                        number: 1,
                        lba: 0,
                        data: true,
                    },
                    Track {
                        number: 2,
                        lba: 200,
                        data: false,
                    },
                ],
                leadout: 300,
            }),
        }
    }

    fn sectors(lba: u32, count: u32, audio: bool) -> Result<Vec<u8>> {
        let mut out = vec![0u8; count as usize * RAW_SECTOR];
        if !audio {
            for s in out.chunks_mut(RAW_SECTOR) {
                s[15] = 2;
            }
        }
        let _ = lba;
        Ok(out)
    }

    #[test]
    fn a_kept_copy_is_listed_and_opens() {
        let root = tempfile::tempdir().unwrap();
        let entry = keep_in(
            root.path(),
            &report(),
            sectors,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(entry.dir, root.path().join("SLUS-00152"));
        assert_eq!(
            std::fs::metadata(entry.dir.join(BIN)).unwrap().len(),
            300 * RAW_SECTOR as u64
        );
        let listed = list_in(root.path());
        assert_eq!(listed, std::slice::from_ref(&entry));
        assert_eq!(listed[0].meta.title, "Tomb Raider");
        let mut disc = crate::image::open(&entry.cue()).unwrap();
        assert_eq!(disc.toc().unwrap().unwrap().leadout, 300);
        // Not a real game's sectors, so it identifies as something else, but
        // it reads.
        let _ = identify(disc.as_mut());
    }

    #[test]
    fn a_failed_copy_leaves_nothing_behind() {
        let root = tempfile::tempdir().unwrap();
        let result = keep_in(
            root.path(),
            &report(),
            sectors,
            &AtomicBool::new(true),
            |_| {},
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        assert!(list_in(root.path()).is_empty());
    }

    #[test]
    fn only_games_with_a_plain_serial_are_kept() {
        let mut r = report();
        assert_eq!(id(&r).as_deref(), Some("SLUS-00152"));
        if let DiscKind::Game(g) = &mut r.kind {
            g.serial = Some("../x".into());
        }
        assert_eq!(id(&r), None);
        r.kind = DiscKind::Data;
        assert_eq!(id(&r), None);
    }
}
