//! Games, handed to an emulator that runs as its own process.
//!
//! RetroArch plays straight from the drive: `cdrom://driveN.cue` is a cue
//! sheet it builds from the TOC of `/dev/sgN`, so nothing is copied to disk
//! first. Cores are looked for in Spectra's own folder first, where
//! `emulators/*/build.sh` puts the ones it builds with fixes upstream has not
//! merged yet; then where RetroArch's own core updater puts them, then where
//! Arch's packages do. The best core comes first, and one that needs the
//! console's firmware is passed over until the user has added it
//! (`firmware`); a core that needs none comes after, so a disc still plays.
//!
//! Not every core reads through RetroArch's drive: Play!, for the PS2, opens
//! its files itself, so it plays a kept copy rather than the disc.
//!
//! The user's own RetroArch settings are left as they are. What Spectra needs
//! goes in a file of its own, passed with `--appendconfig`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use spectra_core::GameSystem;

use crate::firmware;

pub struct Emulator {
    program: PathBuf,
    core: PathBuf,
    /// Plays from the drive, not only from a copy.
    pub reads_drive: bool,
}

struct Core {
    file: &'static str,
    name: &'static str,
    /// Plays from the drive, not only from a copy.
    reads_drive: bool,
    /// Firmware it cannot start without: any one of these.
    needs: &'static [&'static str],
}

const fn core(file: &'static str, name: &'static str, reads_drive: bool) -> Core {
    Core {
        file,
        name,
        reads_drive,
        needs: &[],
    }
}

/// The cores that play a system, best first. Only PCSX-ReARMed has been
/// tried on a disc in the drive; the others play copies until they have
/// been, except PC Engine CD games, which cannot be kept yet as they carry
/// no serial number.
fn cores(system: GameSystem) -> &'static [Core] {
    // PCSX-ReARMed has a BIOS of its own, and uses the console's if the user
    // has added it; Beetle PSX needs Sony's.
    const PS1: &[Core] = &[core("pcsx_rearmed_libretro.so", "PCSX-ReARMed", true)];
    // Play! too; PCSX2 needs Sony's.
    const PS2: &[Core] = &[core("play_libretro.so", "Play!", false)];
    const SATURN: &[Core] = &[
        Core {
            needs: &["mpr-17933.bin", "sega_101.bin"],
            ..core("mednafen_saturn_libretro.so", "Beetle Saturn", false)
        },
        // Imitates the firmware, and plays fewer games for it.
        core("yabause_libretro.so", "Yabause", false),
    ];
    const SEGA_CD: &[Core] = &[Core {
        needs: &["bios_CD_U.bin", "bios_CD_E.bin", "bios_CD_J.bin"],
        ..core("genesis_plus_gx_libretro.so", "Genesis Plus GX", false)
    }];
    const PC_ENGINE_CD: &[Core] = &[
        Core {
            needs: &["syscard3.pce"],
            ..core("mednafen_pce_libretro.so", "Beetle PCE", true)
        },
        Core {
            needs: &["syscard3.pce"],
            ..core("mednafen_pce_fast_libretro.so", "Beetle PCE Fast", true)
        },
    ];
    // Has a firmware of its own, and uses the console's if it is there.
    const NEO_GEO_CD: &[Core] = &[core("neocd_libretro.so", "NeoCD", false)];
    match system {
        GameSystem::Ps1 => PS1,
        GameSystem::Ps2 => PS2,
        GameSystem::Saturn => SATURN,
        GameSystem::SegaCd => SEGA_CD,
        GameSystem::PcEngineCd => PC_ENGINE_CD,
        GameSystem::NeoGeoCd => NEO_GEO_CD,
        _ => &[],
    }
}

/// Whether Spectra hands this console's games to an emulator at all.
pub fn hosted(system: GameSystem) -> bool {
    !cores(system).is_empty()
}

/// Whether the console's games need its firmware to play at all.
pub fn needs_firmware(system: GameSystem) -> bool {
    hosted(system) && cores(system).iter().all(|c| !c.needs.is_empty())
}

/// Where Spectra keeps the cores it builds: `$XDG_DATA_HOME/spectra/cores`.
fn own_cores() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")))
        .map(|data| data.join("spectra/cores"))
}

fn core_dirs() -> Vec<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")));
    own_cores()
        .into_iter()
        .chain(config.map(|config| config.join("retroarch/cores")))
        .chain([PathBuf::from("/usr/lib/libretro")])
        .collect()
}

pub fn on_path(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}

fn installed(core: &Core, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(core.file))
        .find(|path| path.is_file())
}

fn ready(core: &Core) -> bool {
    core.needs.is_empty() || core.needs.iter().any(|file| firmware::present(file))
}

/// RetroArch and a core for this system, if both are installed, with the
/// firmware the core needs.
pub fn find(system: GameSystem) -> Option<Emulator> {
    let program = on_path("retroarch")?;
    let dirs = core_dirs();
    let (core, path) = cores(system)
        .iter()
        .filter(|core| ready(core))
        .find_map(|core| Some((core, installed(core, &dirs)?)))?;
    Some(Emulator {
        program,
        core: path,
        reads_drive: core.reads_drive,
    })
}

/// Why `find` found nothing, in a sentence for the window.
pub fn missing(system: GameSystem) -> String {
    let Some(best) = cores(system).first() else {
        return format!("Spectra doesn't play {} games yet", system.name());
    };
    if on_path("retroarch").is_none() {
        return "Games play in RetroArch, which isn't installed".into();
    }
    let dirs = core_dirs();
    match cores(system)
        .iter()
        .find(|core| installed(core, &dirs).is_some())
    {
        None => format!(
            "Needs RetroArch's {} core, from its Online Updater",
            best.name
        ),
        Some(_) => format!(
            "Needs the {} firmware, from your own console: spectra firmware add FILE",
            system.name()
        ),
    }
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

pub fn cache_dir() -> Option<PathBuf> {
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
    fn spectras_own_cores_come_before_retroarchs() {
        let dirs = core_dirs();
        let own = dirs.iter().position(|dir| dir.ends_with("spectra/cores"));
        let retroarch = dirs.iter().position(|dir| dir.ends_with("retroarch/cores"));
        assert!(own.is_some() && own < retroarch, "{dirs:?}");
        assert_eq!(dirs.last(), Some(&PathBuf::from("/usr/lib/libretro")));
    }

    #[test]
    fn every_firmware_a_core_needs_is_one_spectra_recognises() {
        for system in [
            GameSystem::Ps1,
            GameSystem::Ps2,
            GameSystem::Saturn,
            GameSystem::SegaCd,
            GameSystem::PcEngineCd,
            GameSystem::NeoGeoCd,
        ] {
            for file in cores(system).iter().flat_map(|c| c.needs) {
                let known = firmware::KNOWN.iter();
                assert!(
                    known
                        .filter(|f| f.system == system)
                        .any(|f| f.file == *file),
                    "{file}"
                );
            }
        }
        assert!(needs_firmware(GameSystem::SegaCd));
        assert!(needs_firmware(GameSystem::PcEngineCd));
        assert!(!needs_firmware(GameSystem::Saturn));
        assert!(!needs_firmware(GameSystem::Ps1));
        assert!(!needs_firmware(GameSystem::Xbox));
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
