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

/// Where a kept game's cover is, as a file: with the copy, or in the cache.
pub fn cover_file(entry: &Entry) -> Option<PathBuf> {
    let kept = entry.dir.join(COVER);
    if kept.is_file() {
        return Some(kept);
    }
    let mut game = GameIdentity::new(entry.meta.system?);
    game.serial = entry.meta.serial.clone();
    let (name, _) = cover_url(&game)?;
    Some(cache_dir()?.join(name)).filter(|p| p.is_file())
}

/// What is printed on a copy's disc, as a picture: a scan of the disc kept
/// with it or cached for its game, or failing that its cover. Never the
/// network. True for a scan, which wants cutting out.
fn print_path(entry: &Entry) -> Option<(PathBuf, bool)> {
    let kept = |name: &str| Some(entry.dir.join(name)).filter(|p| p.is_file());
    let cached = |name: Option<String>| {
        name.and_then(|n| cache_dir().map(|d| d.join(n)))
            .filter(|p| p.is_file())
    };
    let scan = kept(FACE).or_else(|| {
        cached(
            entry
                .meta
                .disc_art
                .as_deref()
                .filter(|f| is_file_name(f))
                .map(|f| format!("disc-{f}")),
        )
    });
    if let Some(scan) = scan {
        return Some((scan, true));
    }
    let cover = kept(COVER).or_else(|| {
        let mut game = GameIdentity::new(entry.meta.system?);
        game.serial = entry.meta.serial.clone();
        cached(cover_url(&game).map(|(name, _)| name))
    })?;
    Some((cover, false))
}

/// A copy's face: its disc as an icon. Decoding a full-size picture and
/// drawing the disc from it is most of a second's work for a desktop of
/// them, so the icon is kept, with its colour, as raw pixels: they open in
/// the time it takes to read them. Kept under the picture's own path, size
/// and time, so a new picture makes a new icon.
pub fn face_for_copy(entry: &Entry) -> crate::art::Face {
    use crate::art::{self, Face};
    let Some((print, scan)) = print_path(entry) else {
        return Face::blank();
    };
    let kept = std::fs::metadata(&print).ok().and_then(|meta| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&(&print, meta.len(), meta.modified().ok()), &mut hasher);
        let name = format!("icon-{:016x}.rgba", std::hash::Hasher::finish(&hasher));
        Some(cache_dir()?.join(name))
    });
    if let Some((icon, _)) = kept.as_deref().and_then(read_sleeve) {
        return Face::from_icon(icon);
    }
    let Some(image) = decode(&print) else {
        return Face::blank();
    };
    let print = if scan {
        art::cut_out(&image)
    } else {
        art::square(&image)
    };
    let icon = art::disc_icon(&print);
    let accent = art::accent(&print);
    if let Some(kept) = &kept {
        write_sleeve(kept, &icon, accent);
    }
    Face::from_icon(icon)
}

/// A small picture as kept: its width and height, its colour, its pixels.
fn read_sleeve(path: &Path) -> Option<(RgbaImage, [f32; 3])> {
    let bytes = std::fs::read(path).ok()?;
    let word = |i: usize| {
        bytes
            .get(i * 4..i * 4 + 4)
            .map(|b| [b[0], b[1], b[2], b[3]])
    };
    let (w, h) = (u32::from_le_bytes(word(0)?), u32::from_le_bytes(word(1)?));
    let accent = [2, 3, 4].map(|i| word(i).map_or(0.0, f32::from_le_bytes));
    let image = RgbaImage::from_raw(w, h, bytes.get(20..)?.to_vec())?;
    Some((image, accent))
}

fn write_sleeve(path: &Path, small: &RgbaImage, accent: [f32; 3]) {
    let mut bytes = Vec::with_capacity(20 + small.as_raw().len());
    bytes.extend(small.width().to_le_bytes());
    bytes.extend(small.height().to_le_bytes());
    for c in accent {
        bytes.extend(c.to_le_bytes());
    }
    bytes.extend(small.as_raw());
    // Whole or not at all: written aside, then put in place.
    let part = path.with_extension("part");
    if std::fs::write(&part, bytes).is_ok() {
        let _ = std::fs::rename(&part, path);
    }
}

pub const COVER: &str = "cover.jpg";
pub const FACE: &str = "face.png";

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
    #[test]
    fn a_kept_sleeve_reads_back_as_it_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sleeve.rgba");
        let small = image::RgbaImage::from_fn(3, 5, |x, y| image::Rgba([x as u8, y as u8, 7, 255]));
        super::write_sleeve(&path, &small, [0.25, 0.5, 0.75]);
        let (read, accent) = super::read_sleeve(&path).unwrap();
        assert_eq!(read, small);
        assert_eq!(accent, [0.25, 0.5, 0.75]);
    }

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
