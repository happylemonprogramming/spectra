//! What the person chose, and what Spectra has to work with.
//!
//! The choices are few - day or night, and whether to look things up
//! online - and are kept in `settings.json` beside the library, read once
//! at start. What Spectra has to work with - the players it hands discs
//! to, the drives, how much the library and the cache hold - is gathered
//! when the Spectra window opens, off the UI thread, as it reads the disk.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use spectra_core::GameSystem;

use crate::{game, theme};

/// Whether names, covers and details may be asked for online.
static ONLINE: AtomicBool = AtomicBool::new(true);

pub fn online() -> bool {
    ONLINE.load(Ordering::Relaxed)
}

pub fn set_online(on: bool) {
    ONLINE.store(on, Ordering::Relaxed);
    save();
}

#[derive(Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    night: bool,
    #[serde(default = "yes")]
    online: bool,
}

fn yes() -> bool {
    true
}

fn file() -> Option<PathBuf> {
    Some(
        spectra_core::library::root()?
            .parent()?
            .join("settings.json"),
    )
}

/// As they were left last time; the defaults - day, online - if never set.
pub fn load() {
    let saved: Option<Saved> = file()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    if let Some(saved) = saved {
        theme::set_night(saved.night);
        ONLINE.store(saved.online, Ordering::Relaxed);
    }
}

pub fn save() {
    let Some(path) = file() else { return };
    let saved = Saved {
        night: theme::night(),
        online: online(),
    };
    if let Ok(json) = serde_json::to_vec_pretty(&saved) {
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
        // Whole or not at all: written aside, then put in place.
        let part = path.with_extension("part");
        if std::fs::write(&part, json).is_ok() {
            let _ = std::fs::rename(&part, path);
        }
    }
}

/// Something Spectra hands discs to, or needs, and whether it is there.
#[derive(Debug, Clone)]
pub struct Part {
    pub name: &'static str,
    /// What it is for.
    pub does: &'static str,
    /// Where it was found, or None if it is not installed.
    pub found: Option<String>,
}

/// What Spectra has to work with, as the Spectra window shows it.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub parts: Vec<Part>,
    /// Each optical drive: its maker and model, and its node.
    pub drives: Vec<String>,
    pub copies: usize,
    pub library_bytes: u64,
    pub cache_bytes: u64,
}

/// Look round. Blocks on the disk; call it off the UI thread.
pub fn facts() -> Facts {
    let home = |path: PathBuf| path.display().to_string();
    let parts = vec![
        Part {
            name: "RetroArch",
            does: "Plays games",
            found: game::on_path("retroarch").map(home),
        },
        Part {
            name: "PCSX-ReARMed",
            does: "PlayStation games",
            found: game::core_for(GameSystem::Ps1).map(home),
        },
        Part {
            name: "Play!",
            does: "PlayStation 2 games",
            found: game::core_for(GameSystem::Ps2).map(home),
        },
        Part {
            name: "VLC",
            does: "Plays films",
            found: game::on_path("vlc").map(home),
        },
        Part {
            name: "libdvdcss",
            does: "Reads most DVDs",
            found: dvdcss().map(home),
        },
        Part {
            name: "sg",
            does: "Full drive access",
            found: Path::new("/sys/module/sg")
                .exists()
                .then(|| "Kernel module, loaded".to_string()),
        },
    ];
    let drives = spectra_core::drive::list()
        .into_iter()
        .map(|d| {
            let name = format!("{} {}", d.vendor, d.model);
            match d.path() {
                Some(node) => format!("{}  ·  {}", name.trim(), node.display()),
                None => name.trim().to_string(),
            }
        })
        .collect();
    let library = spectra_core::library::root();
    let cache = crate::artwork::cache_dir().and_then(|art| Some(art.parent()?.to_path_buf()));
    Facts {
        parts,
        drives,
        copies: spectra_core::library::list().len(),
        library_bytes: library.as_deref().map_or(0, size),
        cache_bytes: cache.as_deref().map_or(0, |dir| {
            CACHES.iter().map(|name| size(&dir.join(name))).sum()
        }),
    }
}

/// What in `~/.cache/spectra` is pictures and answers kept from the
/// network - not logs, not RetroArch's settings - and so can go.
const CACHES: [&str; 2] = ["art", "musicbrainz"];

/// Throw away what was kept from the network: asked for again as needed.
pub fn clear_cache() -> std::io::Result<()> {
    let Some(dir) = crate::artwork::cache_dir().and_then(|art| Some(art.parent()?.to_path_buf()))
    else {
        return Ok(());
    };
    for name in CACHES {
        match std::fs::remove_dir_all(dir.join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// libdvdcss where the system's libraries are.
fn dvdcss() -> Option<PathBuf> {
    [
        "/usr/lib",
        "/usr/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/local/lib",
    ]
    .iter()
    .map(|dir| Path::new(dir).join("libdvdcss.so.2"))
    .find(|path| path.exists())
}

/// Everything under a directory, in bytes; a file, its own.
fn size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| size(&entry.path()))
        .sum()
}

/// "1.2 GB", "340 MB", "12 KB".
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut value = n as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value < 10.0 && unit > 0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_said_as_people_say_them() {
        assert_eq!(bytes(12_000), "12 KB");
        assert_eq!(bytes(340_000_000), "340 MB");
        assert_eq!(bytes(1_234_000_000), "1.2 GB");
    }

    #[test]
    fn a_directory_is_as_big_as_what_is_in_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), [0u8; 100]).unwrap();
        std::fs::create_dir(dir.path().join("b")).unwrap();
        std::fs::write(dir.path().join("b/c"), [0u8; 50]).unwrap();
        assert_eq!(size(dir.path()), 150);
    }
}
