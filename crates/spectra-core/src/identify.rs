//! What a disc is: music, a film, a game and for which console, or a PC disc.
//!
//! Every console that boots from an optical disc has to find its boot program
//! somehow, and that is what gives a disc away:
//!
//!   - Sony's put a SYSTEM.CNF in the root of an ISO 9660 volume. `BOOT2`
//!     names a PS2 executable and `BOOT` a PS1 one, and the executable is
//!     named after the game's serial ("SCUS_971.11" is SCUS-97111).
//!   - Sega's put a header at the very start of the disc, naming the system
//!     and then the game, the way a Mega Drive cartridge does.
//!   - Nintendo's put a magic number in the disc header; Microsoft's a volume
//!     descriptor at a fixed offset into the game partition.
//!
//! Films announce themselves by directory: VIDEO_TS, BDMV, VCD. A PC disc has
//! an AUTORUN.INF. Each check is one or two small reads, and games are asked
//! about first, because any data disc could be one.
//!
//! Ported from Rainbow Player's `src/lib/game/identify.ts` and extended.

use serde::{Deserialize, Serialize};

use crate::disc::{Disc, Media, Toc};
use crate::discid::{MusicBrainzId, musicbrainz};
use crate::iso9660::{IsoVolume, ReadSectors, read_iso, read_root_file};
use crate::udf::read_udf;
use crate::{Result, catalog};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GameSystem {
    Ps1,
    Ps2,
    Ps3,
    Saturn,
    SegaCd,
    PcEngineCd,
    NeoGeoCd,
    GameCube,
    Wii,
    Xbox,
    Xbox360,
}

impl GameSystem {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ps1 => "PlayStation",
            Self::Ps2 => "PlayStation 2",
            Self::Ps3 => "PlayStation 3",
            Self::Saturn => "Sega Saturn",
            Self::SegaCd => "Sega CD",
            Self::PcEngineCd => "PC Engine CD",
            Self::NeoGeoCd => "Neo Geo CD",
            Self::GameCube => "GameCube",
            Self::Wii => "Wii",
            Self::Xbox => "Xbox",
            Self::Xbox360 => "Xbox 360",
        }
    }

    /// The PLAN.md phase that makes this playable, and the engine it uses.
    /// None for systems that are out of scope.
    pub fn plan(self) -> Option<(u8, &'static str)> {
        match self {
            Self::Ps2 => Some((1, "PCSX2")),
            Self::Ps1 => Some((1, "PCSX-ReARMed")),
            Self::Saturn => Some((2, "Kronos")),
            Self::SegaCd => Some((2, "Genesis Plus GX")),
            Self::PcEngineCd => Some((2, "Beetle PCE")),
            Self::NeoGeoCd => Some((2, "NeoCD")),
            Self::GameCube | Self::Wii => Some((3, "Dolphin")),
            Self::Xbox => Some((3, "xemu")),
            Self::Ps3 | Self::Xbox360 => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameIdentity {
    pub system: GameSystem,
    /// The publisher's product code, normalised: "SCUS-97111".
    pub serial: Option<String>,
    /// The best name to be had without asking anyone.
    pub title: Option<String>,
    /// "NTSC-U", "PAL-E" and so on, where known.
    pub region: Option<String>,
    /// The executable a PlayStation boots.
    pub boot: Option<String>,
    pub publisher: Option<String>,
    /// Year of release, as written: "1996".
    pub year: Option<String>,
    /// File name of a scan of the disc's printed side, on LaunchBox's image
    /// host, where there is one that can only be this disc.
    pub disc_art: Option<String>,
}

impl GameIdentity {
    pub fn new(system: GameSystem) -> Self {
        Self {
            system,
            serial: None,
            title: None,
            region: None,
            boot: None,
            publisher: None,
            year: None,
            disc_art: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DiscKind {
    /// Red Book audio, possibly with an enhanced CD's data track after it.
    Audio {
        tracks: usize,
        enhanced: bool,
        musicbrainz: Option<MusicBrainzId>,
    },
    DvdVideo,
    BluRayVideo,
    VideoCd {
        super_vcd: bool,
    },
    Game(GameIdentity),
    /// A PC disc: something with an installer or autorun.
    Pc,
    /// A data disc that is none of the above.
    Data,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub source: String,
    pub media: Media,
    #[serde(flatten)]
    pub kind: DiscKind,
    /// The volume label, where the disc has one.
    pub label: Option<String>,
    pub sectors: Option<u32>,
    pub toc: Option<Toc>,
}

/// Run a probe whose failure means "not this kind of disc". An empty drive is
/// still an error: nothing else will read either.
fn probe<T>(result: Result<Option<T>>) -> Result<Option<T>> {
    match result {
        Err(e) if e.medium_not_present() => Err(e),
        Err(_) => Ok(None),
        ok => ok,
    }
}

pub fn identify(disc: &mut dyn Disc) -> Result<Report> {
    let source = disc.describe();
    let media = disc.media()?;
    let toc = probe(disc.toc())?;
    let sectors = probe(disc.capacity())?;
    let report = |kind, label| Report {
        source: source.clone(),
        media,
        kind,
        label,
        sectors,
        toc: toc.clone(),
    };

    // A CD with no track a console could boot from is music.
    let base = match &toc {
        Some(t) => match t.boot_track() {
            Some(track) => track.lba,
            None => {
                let kind = DiscKind::Audio {
                    tracks: t.audio_tracks(),
                    enhanced: t.enhanced_data_track().is_some(),
                    musicbrainz: musicbrainz(t),
                };
                return Ok(report(kind, None));
            }
        },
        None => 0,
    };
    let mut read = |lba: u32, count: u32| disc.read(base + lba, count);

    let iso = probe(read_iso(&mut read))?;
    if let Some(mut game) = identify_game(&mut read, iso.as_ref(), media)? {
        if game.system == GameSystem::Ps1 {
            // The disc's size, from its table of contents, tells apart the
            // discs of a set that all carry the set's serial.
            let sectors = toc.as_ref().map(|t| t.leadout);
            if let Some(meta) = catalog::ps1_meta(game.serial.as_deref(), sectors) {
                game.title = Some(meta.title);
                game.region = meta.region.or(game.region);
                game.publisher = meta.publisher;
                game.year = meta.year;
                game.disc_art = meta.disc_art;
            }
        }
        let label = iso.map(|v| v.volume_id).filter(|l| !l.is_empty());
        return Ok(report(DiscKind::Game(game), label));
    }

    let udf = if media == Media::Cd {
        None
    } else {
        probe(read_udf(&mut read))?
    };
    let label = udf
        .as_ref()
        .map(|u| u.label.clone())
        .or_else(|| iso.as_ref().map(|v| v.volume_id.clone()))
        .filter(|l| !l.is_empty());
    let has_dir = |name: &str| {
        udf.as_ref().is_some_and(|u| u.has_dir(name))
            || iso.as_ref().is_some_and(|v| v.has_dir(name))
    };

    let kind = if has_dir("VIDEO_TS") {
        DiscKind::DvdVideo
    } else if has_dir("BDMV") {
        DiscKind::BluRayVideo
    } else if has_dir("SVCD") {
        DiscKind::VideoCd { super_vcd: true }
    } else if has_dir("VCD") {
        DiscKind::VideoCd { super_vcd: false }
    } else if iso
        .as_ref()
        .is_some_and(|v| v.has_file("AUTORUN.INF") || v.has_file("SETUP.EXE"))
    {
        DiscKind::Pc
    } else {
        DiscKind::Data
    };
    Ok(report(kind, label))
}

/// The console and game on a data disc, or None if it is not a game this knows.
pub fn identify_game(
    read: &mut ReadSectors,
    iso: Option<&IsoVolume>,
    media: Media,
) -> Result<Option<GameIdentity>> {
    let Some(first) = probe(read(0, 1).map(Some))? else {
        return Ok(None);
    };
    if let Some(game) = sega_header(&first).or_else(|| nintendo_header(&first)) {
        return Ok(Some(game));
    }
    if media == Media::Cd
        && probe(read(1, 1).map(Some))?
            .is_some_and(|s| s[0x20..].starts_with(b"PC Engine CD-ROM SYSTEM"))
    {
        return Ok(Some(game(GameSystem::PcEngineCd)));
    }
    if media != Media::Cd
        && let Some(system) = xbox(read)?
    {
        return Ok(Some(game(system)));
    }

    let Some(iso) = iso else { return Ok(None) };
    if let Some(cnf) = probe(read_root_file(read, iso, "SYSTEM.CNF"))? {
        let fields = parse_system_cnf(&String::from_utf8_lossy(&cnf));
        let ps2 = fields.iter().any(|(k, _)| k == "BOOT2");
        let Some(boot) = fields
            .iter()
            .find(|(k, _)| k == "BOOT2" || k == "BOOT")
            .map(|(_, v)| v.clone())
        else {
            return Ok(None);
        };
        let serial = serial_from_boot(&boot);
        let known = serial
            .as_deref()
            .filter(|_| ps2)
            .and_then(catalog::ps2_title);
        let region = known.map(|(_, r)| r.to_string()).or_else(|| {
            fields
                .iter()
                .find(|(k, _)| k == "VMODE")
                .map(|(_, v)| v.clone())
        });
        return Ok(Some(GameIdentity {
            system: if ps2 {
                GameSystem::Ps2
            } else {
                GameSystem::Ps1
            },
            title: known.map(|(t, _)| t.to_string()),
            serial,
            region,
            boot: Some(boot),
            ..GameIdentity::new(GameSystem::Ps1)
        }));
    }
    if iso.has_file("PS3_DISC.SFB") {
        return Ok(Some(game(GameSystem::Ps3)));
    }
    if iso.has_file("IPL.TXT") {
        return Ok(Some(game(GameSystem::NeoGeoCd)));
    }
    Ok(None)
}

fn game(system: GameSystem) -> GameIdentity {
    GameIdentity::new(system)
}

/// `KEY = value` lines. Keys are upper-cased; the rest is kept as written.
pub fn parse_system_cnf(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            (!key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .then(|| (key.to_ascii_uppercase(), value.trim().to_string()))
        })
        .collect()
}

/// "cdrom0:\SCUS_971.11;1" -> "SCUS-97111". The executable is named after the
/// serial with an underscore and a dot thrown in to fit 8.3, so taking those
/// back out recovers it. Anything that does not look like a serial -
/// homebrew, some prototypes - gives None.
pub fn serial_from_boot(boot: &str) -> Option<String> {
    let file = boot.rsplit(['\\', '/', ':']).next()?;
    let file = file.split(';').next()?;
    let (prefix, rest) = file.split_at_checked(4)?;
    if !prefix.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let rest = rest.strip_prefix(['_', '-']).unwrap_or(rest);
    let (head, tail) = match rest.len() {
        5 => rest.split_at_checked(3)?,
        6 if rest.as_bytes()[3] == b'.' => (&rest[..3], &rest[4..]),
        _ => return None,
    };
    (head.len() == 3
        && tail.len() == 2
        && (head.chars().chain(tail.chars())).all(|c| c.is_ascii_digit()))
    .then(|| format!("{}-{head}{tail}", prefix.to_ascii_uppercase()))
}

fn ascii(b: &[u8], from: usize, length: usize) -> Option<String> {
    let s: String = b
        .get(from..from + length)?
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| {
            if (0x20..0x7f).contains(&c) {
                char::from(c)
            } else {
                ' '
            }
        })
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    (!s.is_empty()).then_some(s)
}

/// Sega's disc header: the system name in the first sixteen bytes, then the
/// product code and title at fixed offsets.
fn sega_header(b: &[u8]) -> Option<GameIdentity> {
    let (system, serial, title) = match ascii(b, 0, 16)?.as_str() {
        "SEGA SEGASATURN" => (GameSystem::Saturn, ascii(b, 0x20, 10), ascii(b, 0x60, 112)),
        // The cartridge-style header begins at 0x100; the overseas title is
        // the one most likely to be in Latin script.
        "SEGADISCSYSTEM" => (
            GameSystem::SegaCd,
            ascii(b, 0x180, 14),
            ascii(b, 0x150, 48).or_else(|| ascii(b, 0x120, 48)),
        ),
        _ => return None,
    };
    Some(GameIdentity {
        system,
        serial,
        title,
        ..GameIdentity::new(system)
    })
}

/// GameCube and Wii discs open with a six-character game ID, then a magic
/// number at 0x1c (GameCube) or 0x18 (Wii), then the title.
fn nintendo_header(b: &[u8]) -> Option<GameIdentity> {
    let system = if b.get(0x1c..0x20)? == [0xc2, 0x33, 0x9f, 0x3d] {
        GameSystem::GameCube
    } else if b.get(0x18..0x1c)? == [0x5d, 0x1c, 0x9e, 0xa3] {
        GameSystem::Wii
    } else {
        return None;
    };
    Some(GameIdentity {
        system,
        serial: ascii(b, 0, 6),
        title: ascii(b, 0x20, 64),
        ..GameIdentity::new(system)
    })
}

/// An Xbox game partition opens its filesystem 32 sectors in with
/// "MICROSOFT*XBOX*MEDIA". A bare game partition (an XISO) has it at 32; a
/// whole-disc image has it that far past where the partition starts, which
/// depends on the disc format.
fn xbox(read: &mut ReadSectors) -> Result<Option<GameSystem>> {
    const MAGIC: &[u8] = b"MICROSOFT*XBOX*MEDIA";
    // (partition start in sectors, system): XISO, XGD1, XGD2, XGD3.
    for (start, system) in [
        (0, GameSystem::Xbox),
        (198_144, GameSystem::Xbox),
        (129_824, GameSystem::Xbox360),
        (16_640, GameSystem::Xbox360),
    ] {
        if probe(read(start + 32, 1).map(Some))?.is_some_and(|s| s.starts_with(MAGIC)) {
            return Ok(Some(system));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serials() {
        assert_eq!(
            serial_from_boot(r"cdrom0:\SCUS_971.11;1").as_deref(),
            Some("SCUS-97111")
        );
        assert_eq!(
            serial_from_boot(r"cdrom:\SLUS_000.67;1").as_deref(),
            Some("SLUS-00067")
        );
        assert_eq!(
            serial_from_boot("cdrom0:\\SLES-50330;1").as_deref(),
            Some("SLES-50330")
        );
        assert_eq!(serial_from_boot(r"cdrom0:\MAIN.ELF;1"), None);
    }

    #[test]
    fn system_cnf() {
        let fields =
            parse_system_cnf("BOOT2 = cdrom0:\\SCUS_971.11;1\r\nVER = 1.00\r\nvmode = NTSC\r\n");
        assert_eq!(fields[0], ("BOOT2".into(), "cdrom0:\\SCUS_971.11;1".into()));
        assert_eq!(fields[2], ("VMODE".into(), "NTSC".into()));
    }
}
