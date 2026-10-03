//! Pictures for a game disc: its cover, and a scan of its printed side.
//!
//! Fetched once and kept in `~/.cache/spectra/art`, so a disc is only ever
//! asked about the first time it goes in. A picture nobody has is remembered
//! as missing, for the same reason. The requests name only the picture - a
//! serial, or a scan's file name - and say nothing about who is asking.
//!
//! `curl` does the fetching. It is on every system Spectra runs on, and
//! saves Spectra carrying an HTTP and TLS stack of its own for a handful of
//! requests per disc.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use image::RgbaImage;
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
fn fetch(dir: &Path, name: &str, url: &str) -> Option<PathBuf> {
    let path = dir.join(name);
    let missing = dir.join(format!("{name}.missing"));
    if path.is_file() {
        return Some(path);
    }
    if missing.exists() {
        return None;
    }
    let partial = dir.join(format!("{name}.part"));
    let output = Command::new("curl")
        .args(["--silent", "--location", "--max-time", "30"])
        .args(["--user-agent", "Mozilla/5.0"])
        .args(["--write-out", "%{http_code}", "--output"])
        .arg(&partial)
        .arg(url)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let status = String::from_utf8_lossy(&output.stdout);
    match status.trim() {
        "200" if output.status.success() => {
            std::fs::rename(&partial, &path).ok()?;
            Some(path)
        }
        // Nobody has scanned this one. Asking again will not change that.
        "404" => {
            let _ = std::fs::remove_file(&partial);
            let _ = std::fs::write(&missing, "");
            None
        }
        // Offline, or the host is having a bad day: try again next time.
        _ => {
            let _ = std::fs::remove_file(&partial);
            None
        }
    }
}

fn decode(path: &Path) -> Option<RgbaImage> {
    image::open(path).ok().map(|image| image.to_rgba8())
}

fn cache_dir() -> Option<PathBuf> {
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
