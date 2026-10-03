//! Pictures for a game disc: its cover, and a scan of its printed side.
//!
//! Fetched once and kept in `~/.cache/spectra/art`, so a disc is only ever
//! asked about the first time it goes in. A picture nobody has is remembered
//! as missing, for the same reason. The requests name only the picture - a
//! serial, or a scan's file name - and say nothing about who is asking.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use spectra_core::library::Entry;

use crate::net::{self, Get};
use spectra_core::{GameIdentity, GameSystem};

const COVERS_PS1: &str =
    "https://raw.githubusercontent.com/xlenore/psx-covers/main/covers/default/";
const COVERS_PS2: &str =
    "https://raw.githubusercontent.com/xlenore/ps2-covers/main/covers/default/";
const DISC_SCANS: &str = "https://images.launchbox-app.com/";

#[derive(Debug, Clone, Default)]
pub struct Pictures {
    pub cover: Option<RgbaImage>,
    /// The printed side of the disc, cut out on a transparent background.
    pub face: Option<RgbaImage>,
}

/// Everything there is to be had for a game, from the cache or the network.
/// Blocks; call it off the UI thread.
pub fn for_game(game: &GameIdentity) -> Pictures {
    let Some(dir) = cache_dir() else {
        return Pictures::default();
    };
    let cover = cover_url(game).and_then(|(name, url)| fetch(&dir, &name, &url));
    let face = game
        .disc_art
        .as_deref()
        .filter(|file| is_file_name(file))
        .and_then(|file| {
            fetch(
                &dir,
                &format!("disc-{file}"),
                &format!("{DISC_SCANS}{file}"),
            )
        });
    Pictures {
        cover: cover.and_then(|path| decode(&path)),
        face: face.and_then(|path| decode(&path)),
    }
}

/// Keep a game's pictures with its copy in the library, so the copy keeps
/// its face if the cache is cleared.
pub fn store(dir: &Path, game: &GameIdentity) {
    let Some(cache) = cache_dir() else { return };
    if let Some((name, _)) = cover_url(game) {
        let _ = std::fs::copy(cache.join(name), dir.join(COVER));
    }
    if let Some(file) = game.disc_art.as_deref().filter(|f| is_file_name(f)) {
        let _ = std::fs::copy(cache.join(format!("disc-{file}")), dir.join(FACE));
    }
}

/// The pictures kept with a copy, or failing that - a copy made from the
/// command line - whatever the cache has for its game. Never the network:
/// this runs as the shelf is browsed.
pub fn for_copy(entry: &Entry) -> Pictures {
    let mut game = GameIdentity::new(entry.meta.system.unwrap_or(GameSystem::Ps1));
    game.serial = entry.meta.serial.clone();
    game.disc_art = entry.meta.disc_art.clone();
    let cache = cache_dir();
    let cached = |name: Option<String>| cache.as_ref().zip(name).map(|(dir, n)| dir.join(n));
    let cover_name = entry
        .meta
        .system
        .and(cover_url(&game))
        .map(|(name, _)| name);
    let face_name = game
        .disc_art
        .as_deref()
        .filter(|f| is_file_name(f))
        .map(|f| format!("disc-{f}"));
    Pictures {
        cover: decode(&entry.dir.join(COVER))
            .or_else(|| cached(cover_name).and_then(|p| decode(&p))),
        face: decode(&entry.dir.join(FACE)).or_else(|| cached(face_name).and_then(|p| decode(&p))),
    }
}

pub const COVER: &str = "cover.jpg";
const FACE: &str = "face.png";

fn cover_url(game: &GameIdentity) -> Option<(String, String)> {
    let (system, base) = match game.system {
        GameSystem::Ps1 => ("ps1", COVERS_PS1),
        GameSystem::Ps2 => ("ps2", COVERS_PS2),
        _ => return None,
    };
    let serial = game.serial.as_deref().filter(|s| is_serial(s))?;
    Some((
        format!("cover-{system}-{serial}.jpg"),
        format!("{base}{serial}.jpg"),
    ))
}

/// "SLUS-00152": nothing that could step outside a URL path or a directory.
fn is_serial(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// "r2_a2a54e70-….png", as the title table gives it.
fn is_file_name(s: &str) -> bool {
    let Some((stem, ext)) = s.rsplit_once('.') else {
        return false;
    };
    matches!(ext, "png" | "jpg")
        && !stem.is_empty()
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The cached file, fetching it first if need be. None if there is no such
/// picture, or it could not be had this time.
pub fn fetch(dir: &Path, name: &str, url: &str) -> Option<PathBuf> {
    let path = dir.join(name);
    let missing = dir.join(format!("{name}.missing"));
    if path.is_file() {
        return Some(path);
    }
    if missing.exists() {
        return None;
    }
    match net::get(url, net::ANONYMOUS) {
        Get::Found(bytes) => {
            let partial = dir.join(format!("{name}.part"));
            std::fs::write(&partial, bytes).ok()?;
            std::fs::rename(&partial, &path).ok()?;
            Some(path)
        }
        // Nobody has this one. Asking again will not change that.
        Get::Missing => {
            let _ = std::fs::write(&missing, "");
            None
        }
        Get::Failed => None,
    }
}

/// By what the file holds rather than its name: a scan kept as `face.png`
/// may be a JPEG.
pub fn decode(path: &Path) -> Option<RgbaImage> {
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    reader.decode().ok().map(|image| image.to_rgba8())
}

pub fn cache_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))?
        .join("spectra/art");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_reach_a_url_are_plain() {
        assert!(is_serial("SLUS-00152"));
        assert!(!is_serial("../etc"));
        assert!(!is_serial("SLUS 00152"));
        assert!(is_file_name("r2_a2a54e70-0e8f-440a-8a41-e85802bb835d.png"));
        assert!(!is_file_name("../x.png"));
        assert!(!is_file_name("x.svg"));
        assert!(!is_file_name(".png"));
    }

    #[test]
    fn covers_by_system_and_serial() {
        let mut game = GameIdentity::new(GameSystem::Ps1);
        game.serial = Some("SLUS-00152".into());
        let (name, url) = cover_url(&game).unwrap();
        assert_eq!(name, "cover-ps1-SLUS-00152.jpg");
        assert_eq!(url, format!("{COVERS_PS1}SLUS-00152.jpg"));
        game.system = GameSystem::Saturn;
        assert!(cover_url(&game).is_none());
    }
}
