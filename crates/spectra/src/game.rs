//! Games, handed to an emulator that runs as its own process.
//!
//! RetroArch plays straight from the drive: `cdrom://driveN.cue` is a cue
//! sheet it builds from the TOC of `/dev/sgN`, so nothing is copied to disk
//! first. Cores are looked for where RetroArch's own core updater puts them,
//! then where Arch's packages do. A core that needs no BIOS comes first, so a
//! disc plays without anything else to set up.
//!
//! Not every core reads through RetroArch's drive: Play!, for the PS2, opens
//! its files itself, so it plays a kept copy rather than the disc.
//!
//! The user's own RetroArch settings are left as they are. What Spectra needs
//! goes in a file of its own, passed with `--appendconfig`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use spectra_core::GameSystem;

pub struct Emulator {
    program: PathBuf,
    core: PathBuf,
    /// Plays from the drive, not only from a copy.
    pub reads_drive: bool,
}

/// The cores that play a system, best first, and whether each reads the
/// drive.
fn cores(system: GameSystem) -> &'static [(&'static str, bool)] {
    match system {
        // PCSX-ReARMed has a BIOS of its own; Beetle PSX needs Sony's.
        GameSystem::Ps1 => &[("pcsx_rearmed_libretro.so", true)],
        // Play! too; PCSX2 needs Sony's.
        GameSystem::Ps2 => &[("play_libretro.so", false)],
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
    let (core, reads_drive) = cores(system)
        .iter()
        .flat_map(|&(name, reads)| dirs.iter().map(move |dir| (dir.join(name), reads)))
        .find(|(path, _)| path.is_file())?;
    Some(Emulator {
        program,
        core,
        reads_drive,
    })
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
        let cache = cache_dir();
        let log = cache
            .as_ref()
            .and_then(|dir| std::fs::File::create(dir.join("game.log")).ok())
            .map_or_else(Stdio::null, Stdio::from);
        let mut command = Command::new(&self.program);
        if let Some(dir) = &cache {
            let settings = dir.join("retroarch.cfg");
            std::fs::write(&settings, settings_text(&user_profiles(), SYSTEM_PROFILES))?;
            command.arg("--appendconfig").arg(settings);
        }
        command
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

/// Where `retroarch-joypad-autoconfig` puts its gamepad profiles.
const SYSTEM_PROFILES: &str = "/usr/share/libretro/autoconfig";

/// RetroArch's own gamepad profile folder, as it is out of the box.
fn user_profiles() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
        .unwrap_or_default()
        .join("retroarch/autoconfig")
}

fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.path().is_file() || has_files(&e.path()))
    })
}

/// Spectra's settings for RetroArch:
///
///   - Holding Start quits, for a player with only a gamepad. RetroArch
///     fires the combo after two seconds of holding, and its usual "press
///     again to quit" makes that four.
///   - Gamepads are recognised from the packaged profiles, unless the user
///     has profiles of their own (RetroArch's online updater puts them in its
///     own folder).
///   - None of this is saved into the user's retroarch.cfg on exit, which
///     RetroArch would otherwise do with appended settings.
fn settings_text(user: &Path, system: &str) -> String {
    let mut text = String::from(
        "# Written by Spectra for the games it starts; yours are in retroarch.cfg.\n\
         config_save_on_exit = \"false\"\n\
         input_quit_gamepad_combo = \"7\"\n",
    );
    if !has_files(user) && Path::new(system).is_dir() {
        text.push_str(&format!("joypad_autoconfig_dir = \"{system}\"\n"));
    }
    text
}

fn cache_dir() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))?
        .join("spectra");
    std::fs::create_dir_all(&cache).ok()?;
    Some(cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_gamepad_profiles_only_when_the_user_has_none() {
        let dir = std::env::temp_dir().join(format!("spectra-profiles-{}", std::process::id()));
        let user = dir.join("user");
        let system = dir.join("system");
        std::fs::create_dir_all(user.join("udev")).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        let system = system.to_str().unwrap();

        let text = settings_text(&user, system);
        assert!(text.contains("config_save_on_exit = \"false\""));
        assert!(text.contains("input_quit_gamepad_combo = \"7\""));
        assert!(text.contains(&format!("joypad_autoconfig_dir = \"{system}\"")));

        std::fs::write(user.join("udev/pad.cfg"), "").unwrap();
        assert!(!settings_text(&user, system).contains("joypad_autoconfig_dir"));
        assert!(!settings_text(&dir.join("none"), "/nowhere").contains("joypad_autoconfig_dir"));
        std::fs::remove_dir_all(dir).unwrap();
    }

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
