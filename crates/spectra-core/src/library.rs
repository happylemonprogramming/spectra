//! Discs kept as copies, to play without the disc.
//!
//! Each copy is a folder under `~/.local/share/spectra/library`, named by the
//! disc's own ID - a game's serial, `cd-` and an audio CD's MusicBrainz disc
//! ID, or `dvd-` and a DVD's label and size:
//!
//! ```text
//! SLUS-00152/disc.bin    every sector, raw
//! SLUS-00152/disc.cue    where the tracks start; what an emulator opens
//! SLUS-00152/meta.json   what the disc is, written last
//! dvd-DUDE-3456810/disc.iso   every sector, in the clear
//! ```
//!
//! An audio CD's copy is the same raw audio the disc holds, nothing lost; an
//! enhanced CD's is its music alone, without the data session after it. A
//! DVD's is decrypted on the way in, through the system's libdvdcss, so it
//! plays anywhere without the drive (see `dvdcss`).
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

use crate::copy::{self, DvdSectors, Progress};
use crate::disc::RAW_SECTOR;
use crate::disc::Toc;
use crate::drive::Drive;
use crate::{DiscKind, Error, GameSystem, Report, Result};

pub const BIN: &str = "disc.bin";
pub const CUE: &str = "disc.cue";
pub const ISO: &str = "disc.iso";
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
    /// An album's artist.
    #[serde(default)]
    pub artist: Option<String>,
    /// An album's track names, in track-list order.
    #[serde(default)]
    pub tracks: Vec<TrackName>,
    pub sectors: u32,
    /// Sectors the drive could not read, kept as zeros.
    pub unreadable: u32,
    /// Seconds since 1970.
    pub created: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackName {
    pub title: String,
    /// Only where it differs from the album's: a compilation's.
    pub artist: Option<String>,
}

/// What an audio CD is called, as found elsewhere: the disc does not say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names {
    pub title: String,
    pub artist: String,
    pub year: Option<String>,
    pub tracks: Vec<TrackName>,
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

    pub fn bin(&self) -> PathBuf {
        self.dir.join(BIN)
    }

    /// The copy's table of contents, from its cue sheet.
    pub fn toc(&self) -> Option<Toc> {
        crate::image::open(&self.cue()).ok()?.toc().ok()?
    }

    pub fn iso(&self) -> PathBuf {
        self.dir.join(ISO)
    }

    /// Music, rather than a game.
    pub fn is_album(&self) -> bool {
        self.meta.system.is_none() && self.meta.id.starts_with(CD)
    }

    /// A DVD's film.
    pub fn is_film(&self) -> bool {
        self.meta.system.is_none() && self.meta.id.starts_with(DVD)
    }
}

/// What an audio CD's ID starts with.
const CD: &str = "cd-";
/// And a DVD's.
const DVD: &str = "dvd-";

/// Where copies are kept: `$XDG_DATA_HOME/spectra/library`.
pub fn root() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))?;
    Some(data.join("spectra/library"))
}

/// The ID a copy is kept under, if this disc can be kept: CD games with a
/// serial, audio CDs, and DVDs. A DVD has no ID of its own, so its label and
/// size stand in: the same film pressed again may differ in size, which only
/// means a second copy.
pub fn id(report: &Report) -> Option<String> {
    if let DiscKind::DvdVideo { .. } = report.kind {
        let label = report.label.as_deref().filter(|l| !l.is_empty())?;
        let plain: String = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        return Some(format!("{DVD}{plain}-{}", report.sectors?));
    }
    report.toc.as_ref()?;
    match &report.kind {
        DiscKind::Game(game) => {
            let serial = game.serial.as_deref()?;
            let plain = !serial.is_empty()
                && serial
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            plain.then(|| serial.to_string())
        }
        // Base64 with `.`, `_` and `-`: nothing that leaves a directory.
        DiscKind::Audio {
            musicbrainz: Some(mb),
            ..
        } => {
            let plain = !mb.disc_id.is_empty()
                && mb
                    .disc_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
            plain.then(|| format!("{CD}{}", mb.disc_id))
        }
        _ => None,
    }
}

/// The part of the disc a copy holds.
fn to_copy(report: &Report) -> Option<Toc> {
    let toc = report.toc.as_ref()?;
    match report.kind {
        DiscKind::Audio { .. } => toc.audio_session(),
        _ => Some(toc.clone()),
    }
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

/// Throw a kept copy away.
pub fn remove(entry: &Entry) -> Result<()> {
    let root = root().ok_or_else(|| Error::Unsupported("no home folder".into()))?;
    remove_in(&root, entry)
}

/// Only ever a folder directly in the library: an entry's own `dir` is
/// not trusted to say where that is.
pub fn remove_in(root: &Path, entry: &Entry) -> Result<()> {
    let id = &entry.meta.id;
    let plain = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && id != "."
        && id != "..";
    if !plain {
        return Err(Error::Unsupported(format!("removing a copy called {id:?}")));
    }
    std::fs::remove_dir_all(root.join(id))?;
    Ok(())
}

/// Copy the disc in `drive` into the library, as `report` describes it, and
/// an audio CD by `names` where they are known.
pub fn keep(
    drive: &Drive,
    report: &Report,
    names: Option<&Names>,
    cancel: &AtomicBool,
    progress: impl FnMut(Progress),
) -> Result<Entry> {
    let root = root().ok_or_else(|| Error::Unsupported("no home folder".into()))?;
    keep_in(
        &root,
        report,
        names,
        |lba, count, audio| drive.read_raw(lba, count, audio),
        cancel,
        progress,
    )
}

pub fn keep_in(
    root: &Path,
    report: &Report,
    names: Option<&Names>,
    read: impl FnMut(u32, u32, bool) -> Result<Vec<u8>>,
    cancel: &AtomicBool,
    progress: impl FnMut(Progress),
) -> Result<Entry> {
    let id = id(report).ok_or_else(|| Error::Unsupported("keeping a copy of this disc".into()))?;
    let toc =
        &to_copy(report).ok_or_else(|| Error::Unsupported("keeping a copy of this disc".into()))?;
    let needed = u64::from(toc.leadout) * RAW_SECTOR as u64;
    make(root, &id, needed, |part| {
        let mut bin = BufWriter::with_capacity(1 << 20, File::create(part.join(BIN))?);
        let copied = copy::copy_cd(toc, read, &mut bin, BIN, cancel, progress)?;
        drop(bin);
        std::fs::write(part.join(CUE), &copied.cue)?;
        let game = match &report.kind {
            DiscKind::Game(game) => Some(game),
            _ => None,
        };
        Ok(Meta {
            id: id.clone(),
            title: names
                .map(|n| n.title.clone())
                .or_else(|| game.and_then(|g| g.title.clone()))
                .or_else(|| report.label.clone())
                .unwrap_or_else(|| match game {
                    Some(_) => id.clone(),
                    None => "Audio CD".into(),
                }),
            system: game.map(|g| g.system),
            serial: game.and_then(|g| g.serial.clone()),
            publisher: game.and_then(|g| g.publisher.clone()),
            year: names
                .and_then(|n| n.year.clone())
                .or_else(|| game.and_then(|g| g.year.clone())),
            region: game.and_then(|g| g.region.clone()),
            disc_art: game.and_then(|g| g.disc_art.clone()),
            artist: names.map(|n| n.artist.clone()),
            tracks: names.map(|n| n.tracks.clone()).unwrap_or_default(),
            sectors: copied.sectors,
            unreadable: copied.unreadable,
            created: now(),
        })
    })
}

/// Copy the DVD in `drive`, a `/dev/srN`, into the library in the clear,
/// named by `names` where they are known.
pub fn keep_dvd(
    drive: &Path,
    report: &Report,
    names: Option<&Names>,
    cancel: &AtomicBool,
    progress: impl FnMut(Progress),
) -> Result<Entry> {
    let root = root().ok_or_else(|| Error::Unsupported("no home folder".into()))?;
    let mut disc = crate::dvdcss::Dvdcss::open(drive)?;
    keep_dvd_in(&root, report, names, &mut disc, cancel, progress)
}

pub fn keep_dvd_in(
    root: &Path,
    report: &Report,
    names: Option<&Names>,
    disc: &mut impl DvdSectors,
    cancel: &AtomicBool,
    progress: impl FnMut(Progress),
) -> Result<Entry> {
    let unsupported = || Error::Unsupported("keeping a copy of this disc".into());
    let id = id(report).ok_or_else(unsupported)?;
    let total = report.sectors.ok_or_else(unsupported)?;
    let vobs = vob_sectors(disc)?;
    make(root, &id, u64::from(total) * 2048, |part| {
        let mut iso = BufWriter::with_capacity(1 << 20, File::create(part.join(ISO))?);
        let copied = copy::copy_dvd(disc, &vobs, total, &mut iso, cancel, progress)?;
        drop(iso);
        Ok(Meta {
            id: id.clone(),
            title: names
                .map(|n| n.title.clone())
                .or_else(|| report.label.clone())
                .unwrap_or_else(|| "DVD".into()),
            system: None,
            serial: None,
            publisher: None,
            year: names.and_then(|n| n.year.clone()),
            region: None,
            disc_art: None,
            artist: None,
            tracks: Vec::new(),
            sectors: copied.sectors,
            unreadable: copied.unreadable,
            created: now(),
        })
    })
}

/// Where each VOB file is, start and end, in order: the parts of a DVD
/// that may be scrambled, each with a key of its own.
fn vob_sectors(disc: &mut impl DvdSectors) -> Result<Vec<(u32, u32)>> {
    let mut read = |lba: u32, count: u32| disc.read(lba, count, false);
    let missing = || Error::Unsupported("the DVD's files could not be found".into());
    let iso = crate::iso9660::read_iso(&mut read)?.ok_or_else(missing)?;
    let files = crate::iso9660::read_root_dir(&mut read, &iso, "VIDEO_TS")?.ok_or_else(missing)?;
    let mut vobs: Vec<(u32, u32)> = files
        .iter()
        .filter(|f| !f.directory && f.name.ends_with(".VOB") && f.size > 0)
        .map(|f| (f.lba, f.lba + f.size.div_ceil(2048)))
        .collect();
    vobs.sort_unstable();
    Ok(vobs)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Make a copy: in a `.part` folder that `write` fills and describes, renamed
/// once it is whole. Refused up front if the copy, and a little over, would
/// not fit.
fn make(
    root: &Path,
    id: &str,
    needed: u64,
    write: impl FnOnce(&Path) -> Result<Meta>,
) -> Result<Entry> {
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
    let meta = match write(&part).and_then(|meta| {
        std::fs::write(
            part.join(META),
            serde_json::to_vec_pretty(&meta).expect("meta serialises"),
        )?;
        Ok(meta)
    }) {
        Ok(meta) => meta,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&part);
            return Err(e);
        }
    };
    let dir = root.join(id);
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

    #[test]
    fn a_dvd_is_kept_by_its_label_and_size() {
        let mut dvd = report();
        dvd.media = Media::Dvd;
        dvd.kind = DiscKind::DvdVideo {
            feature_seconds: Some(4981),
        };
        dvd.label = Some("DUDE".into());
        dvd.sectors = Some(3_456_810);
        dvd.toc = None;
        assert_eq!(id(&dvd).as_deref(), Some("dvd-DUDE-3456810"));
        dvd.label = Some("../ME".into());
        assert_eq!(id(&dvd).as_deref(), Some("dvd-___ME-3456810"));
        dvd.label = None;
        assert_eq!(id(&dvd), None);
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
            None,
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
    fn a_removed_copy_is_gone_and_nothing_else_is() {
        let root = tempfile::tempdir().unwrap();
        let keep = |r: &Report| {
            keep_in(
                root.path(),
                r,
                None,
                sectors,
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap()
        };
        let entry = keep(&report());
        let mut other = report();
        if let DiscKind::Game(g) = &mut other.kind {
            g.serial = Some("SLUS-00001".into());
        }
        keep(&other);
        remove_in(root.path(), &entry).unwrap();
        let left = list_in(root.path());
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].meta.id, "SLUS-00001");

        let mut outside = left[0].clone();
        outside.meta.id = "..".into();
        assert!(remove_in(root.path(), &outside).is_err());
        assert_eq!(list_in(root.path()).len(), 1);
    }

    #[test]
    fn a_failed_copy_leaves_nothing_behind() {
        let root = tempfile::tempdir().unwrap();
        let result = keep_in(
            root.path(),
            &report(),
            None,
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

    fn album() -> Report {
        // Two songs, then an enhanced CD's data session.
        let toc = Toc {
            first: 1,
            last: 3,
            tracks: vec![
                Track {
                    number: 1,
                    lba: 0,
                    data: false,
                },
                Track {
                    number: 2,
                    lba: 100,
                    data: false,
                },
                Track {
                    number: 3,
                    lba: 300 + crate::disc::SESSION_GAP,
                    data: true,
                },
            ],
            leadout: 20_000,
        };
        Report {
            source: "test".into(),
            media: Media::Cd,
            kind: DiscKind::Audio {
                tracks: 2,
                enhanced: true,
                musicbrainz: crate::discid::musicbrainz(&toc),
            },
            label: None,
            sectors: None,
            toc: Some(toc),
        }
    }

    #[test]
    fn an_audio_cd_is_kept_as_its_music_alone() {
        let root = tempfile::tempdir().unwrap();
        let names = Names {
            title: "Blue".into(),
            artist: "A Band".into(),
            year: Some("1999".into()),
            tracks: vec![TrackName {
                title: "One".into(),
                artist: None,
            }],
        };
        let entry = keep_in(
            root.path(),
            &album(),
            Some(&names),
            |lba, count, audio| {
                assert!(audio && lba + count <= 300, "read past the music");
                sectors(lba, count, audio)
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert!(entry.meta.id.starts_with("cd-"));
        assert!(entry.is_album());
        assert_eq!(entry.meta.title, "Blue");
        assert_eq!(entry.meta.artist.as_deref(), Some("A Band"));
        assert_eq!(entry.meta.tracks, names.tracks);
        assert_eq!(
            std::fs::metadata(entry.bin()).unwrap().len(),
            300 * RAW_SECTOR as u64
        );
        let mut disc = crate::image::open(&entry.cue()).unwrap();
        let toc = disc.toc().unwrap().unwrap();
        assert_eq!(toc.tracks.len(), 2);
        assert_eq!(toc.leadout, 300);
        assert_eq!(list_in(root.path())[0].meta.title, "Blue");
    }
}
