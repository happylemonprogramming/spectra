//! Games, handed to an emulator that runs as its own process.
//!
//! RetroArch plays straight from the drive: `cdrom://driveN.cue` is a cue
//! sheet it builds from the TOC of `/dev/sgN`, so nothing is copied to disk
//! first. Cores are looked for where RetroArch's own core updater puts them,
//! then where Arch's packages do. A core that needs no BIOS comes first, so a
//! disc plays without anything else to set up.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use spectra_core::GameSystem;

pub struct Emulator {
    program: PathBuf,
    core: PathBuf,
}

/// The cores that play a system, best first.
fn cores(system: GameSystem) -> &'static [&'static str] {
    match system {
        // PCSX-ReARMed has a BIOS of its own; Beetle PSX needs Sony's.
        GameSystem::Ps1 => &["pcsx_rearmed_libretro.so"],
        _ => &[],
    }
}

fn core_dirs() -> Vec<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")));
    config
        .map(|config| config.join("retroarch/cores"))
        .into_iter()
        .chain([PathBuf::from("/usr/lib/libretro")])
        .collect()
}

fn on_path(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}

/// RetroArch and a core for this system, if both are installed.
pub fn find(system: GameSystem) -> Option<Emulator> {
    let program = on_path("retroarch")?;
    let dirs = core_dirs();
    let core = cores(system)
        .iter()
        .flat_map(|name| dirs.iter().map(move |dir| dir.join(name)))
        .find(|path| path.is_file())?;
    Some(Emulator { program, core })
}

/// What RetroArch calls a drive: `/dev/sg0` is `cdrom://drive0.cue`. Only
/// the sg node will do, and only a one-digit one, as RetroArch reads a
/// single character.
pub fn cdrom_uri(drive: &Path) -> Option<String> {
    let n = drive.to_str()?.strip_prefix("/dev/sg")?;
    (n.len() == 1 && n.as_bytes()[0].is_ascii_digit()).then(|| format!("cdrom://drive{n}.cue"))
}

impl Emulator {
    /// Start the game full screen. Its output goes to Spectra's cache, for
    /// when a game will not start.
    pub fn launch(&self, uri: &str) -> std::io::Result<Child> {
        let log = log_file()
            .and_then(|path| std::fs::File::create(path).ok())
            .map_or_else(Stdio::null, Stdio::from);
        Command::new(&self.program)
            .arg("--verbose")
            .arg("--fullscreen")
            .arg("--libretro")
            .arg(&self.core)
            .arg(uri)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
    }
}

fn log_file() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))?
        .join("spectra");
    std::fs::create_dir_all(&cache).ok()?;
    Some(cache.join("game.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retroarch_names_drives_by_sg_number() {
        assert_eq!(
            cdrom_uri(Path::new("/dev/sg0")).as_deref(),
            Some("cdrom://drive0.cue")
        );
        assert_eq!(cdrom_uri(Path::new("/dev/sr0")), None);
        assert_eq!(cdrom_uri(Path::new("/dev/sg12")), None);
    }
}
