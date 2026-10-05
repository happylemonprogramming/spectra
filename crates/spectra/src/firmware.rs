//! Console firmware - the BIOS a console starts from - which only the user
//! can bring. Spectra never ships or downloads any: the user dumps it from a
//! console of their own and hands Spectra the file, by any name.
//!
//! A file is recognised by its checksum, against the ones libretro's
//! core-info files and documentation give, and copied into RetroArch's
//! system folder under the name the core looks for. Anything else is left
//! where it is. A file already there by that name is never overwritten.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha1::{Digest, Sha1};
use spectra_core::GameSystem;
use spectra_core::GameSystem::{NeoGeoCd, PcEngineCd, Ps1, Saturn, SegaCd};

#[derive(Debug, Serialize)]
pub struct Firmware {
    #[serde(serialize_with = "system_name")]
    pub system: GameSystem,
    /// Where the core looks for it, under RetroArch's system folder.
    pub file: &'static str,
    pub what: &'static str,
    /// MD5, or SHA-1 where libretro gives that instead, in hex.
    #[serde(skip)]
    hash: &'static str,
}

fn system_name<S: serde::Serializer>(system: &GameSystem, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(system.name())
}

const fn known(
    system: GameSystem,
    file: &'static str,
    what: &'static str,
    hash: &'static str,
) -> Firmware {
    Firmware {
        system,
        file,
        what,
        hash,
    }
}

/// Every file Spectra knows, best first within a console. From
/// libretro-core-info (`*_libretro.info`), libretro's docs for PCSX-ReARMed
/// and Genesis Plus GX, and neocd_libretro's README.
pub const KNOWN: &[Firmware] = &[
    known(
        Ps1,
        "scph5501.bin",
        "US, 3.0",
        "490f666e1afb15b7362b406ed1cea246",
    ),
    known(
        Ps1,
        "scph7001.bin",
        "US, 4.1",
        "1e68c231d0896b7eadcad1d7d8e76129",
    ),
    known(
        Ps1,
        "scph101.bin",
        "US, PS one 4.4",
        "6e3735ff4c7dc899ee98981385f6f3d0",
    ),
    known(
        Ps1,
        "scph1001.bin",
        "US, 2.0",
        "924e392ed05558ffdb115408c263dccf",
    ),
    known(
        Ps1,
        "scph5500.bin",
        "Japan, 3.0",
        "8dd7d5296a650fac7319bce665a6a53c",
    ),
    known(
        Ps1,
        "scph5502.bin",
        "Europe, 3.0",
        "32736f17079d0b2b7024407c39bd3050",
    ),
    known(
        Ps1,
        "psxonpsp660.bin",
        "from a PSP, any region",
        "c53ca5908936d412331790f4426c6c33",
    ),
    known(
        Saturn,
        "mpr-17933.bin",
        "US and Europe",
        "3240872c70984b6cbfda1586cab68dbe",
    ),
    known(
        Saturn,
        "sega_101.bin",
        "Japan",
        "85ec9ca47d8f6807718151cbcca8b964",
    ),
    known(
        Saturn,
        "saturn_bios.bin",
        "for Yabause",
        "af5828fdff51384f99b3c4926be27762",
    ),
    known(
        SegaCd,
        "bios_CD_U.bin",
        "US",
        "854b9150240a198070150e4566ae1290",
    ),
    known(
        SegaCd,
        "bios_CD_E.bin",
        "Europe",
        "e66fa1dc5820d254611fdcdba0662372",
    ),
    known(
        SegaCd,
        "bios_CD_J.bin",
        "Japan",
        "278a9397d192149e84e820ac621a8edd",
    ),
    known(
        PcEngineCd,
        "syscard3.pce",
        "System Card 3.0",
        "38179df8f4ac870017db21ebcbf53114",
    ),
    known(
        NeoGeoCd,
        "neocd/neocd_f.rom",
        "front loader",
        "a5f4a7a627b3083c979f6ebe1fabc5d2df6d083b",
    ),
    known(
        NeoGeoCd,
        "neocd/neocd_t.rom",
        "top loader",
        "cc92b54a18a8bff6e595aabe8e5c360ba9e62eb5",
    ),
    known(
        NeoGeoCd,
        "neocd/neocd_z.rom",
        "CDZ",
        "b0f1c4fa8d4492a04431805f6537138b842b549f",
    ),
    known(
        NeoGeoCd,
        "neocd/front-sp1.bin",
        "front loader, MAME's",
        "53bc1f283cdf00fa2efbb79f2e36d4c8038d743a",
    ),
    known(
        NeoGeoCd,
        "neocd/top-sp1.bin",
        "top loader, MAME's",
        "235f4d1d74364415910f73c10ae5482d90b4274f",
    ),
    known(
        NeoGeoCd,
        "neocd/neocd.bin",
        "CDZ, MAME's",
        "7bb26d1e5d1e930515219cb18bcde5b7b23e2eda",
    ),
    known(
        NeoGeoCd,
        "neocd/neocd_sf.rom",
        "front loader, SMKDAN",
        "4a94719ee5d0e3f2b981498f70efc1b8f1cef325",
    ),
    known(
        NeoGeoCd,
        "neocd/neocd_st.rom",
        "top loader, SMKDAN",
        "19729b51bdab60c42aafef6e20ea9234c7eb8410",
    ),
    known(
        NeoGeoCd,
        "neocd/neocd_sz.rom",
        "CDZ, SMKDAN",
        "6a947457031dd3a702a296862446d7485aa89dbb",
    ),
    known(
        NeoGeoCd,
        "neocd/uni-bioscd.rom",
        "Universe BIOS",
        "5142f205912869b673a71480c5828b1eaed782a8",
    ),
];

/// The largest file worth reading: every BIOS above is 1 MB or less, so a
/// folder of other things is passed over quickly.
const LARGEST: u64 = 4 << 20;

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
}

/// RetroArch's system folder: `system_directory` in its settings, or where
/// it keeps it out of the box.
pub fn system_dir() -> PathBuf {
    let config = config_dir().unwrap_or_default().join("retroarch");
    std::fs::read_to_string(config.join("retroarch.cfg"))
        .ok()
        .and_then(|text| setting(&text, "system_directory"))
        .and_then(|dir| match dir.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME").map(|home| Path::new(&home).join(rest)),
            None => Some(PathBuf::from(dir)),
        })
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| config.join("system"))
}

/// `key = "value"` in a RetroArch settings file. `default`, as RetroArch
/// writes an unset folder, is none.
fn setting(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        let v = v.trim().trim_matches('"');
        (k.trim() == key && !v.is_empty() && v != "default").then(|| v.to_string())
    })
}

/// Whether this file is in the system folder, as the core would find it.
pub fn present(file: &str) -> bool {
    system_dir().join(file).is_file()
}

/// Which known firmware this file is, by its checksum.
pub fn recognise(path: &Path) -> Option<&'static Firmware> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > LARGEST {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(LARGEST).read_to_end(&mut bytes).ok()?;
    let md5 = hex(&md5(&bytes));
    let sha1 = hex(&Sha1::digest(&bytes));
    KNOWN.iter().find(|f| f.hash == md5 || f.hash == sha1)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What became of one file handed to `add`.
pub enum Added {
    Copied(&'static Firmware),
    AlreadyThere(&'static Firmware),
    /// A different file has that name already; it is left as it is.
    Taken(&'static Firmware),
    Unknown,
    Failed(String),
}

/// Copy every recognised file among these - files, or folders searched
/// through - into `system`.
pub fn add(paths: &[PathBuf], system: &Path) -> Vec<(PathBuf, Added)> {
    let mut files = Vec::new();
    for path in paths {
        collect(path, &mut files);
    }
    files
        .into_iter()
        .map(|path| {
            let added = match recognise(&path) {
                None => Added::Unknown,
                Some(firmware) => install(&path, firmware, system),
            };
            (path, added)
        })
        .collect()
}

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            entries.sort();
            for entry in entries {
                collect(&entry, files);
            }
        }
        Err(_) => files.push(path.to_path_buf()),
    }
}

fn install(path: &Path, firmware: &'static Firmware, system: &Path) -> Added {
    let to = system.join(firmware.file);
    if to.exists() {
        return match recognise(&to) {
            Some(there) if there.file == firmware.file => Added::AlreadyThere(firmware),
            _ => Added::Taken(firmware),
        };
    }
    let copied = to
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::copy(path, &to));
    match copied {
        Ok(_) => Added::Copied(firmware),
        Err(e) => Added::Failed(format!("{}: {e}", to.display())),
    }
}

/// MD5 (RFC 1321), which libretro gives most checksums in. Small enough to
/// keep here rather than take on a crate for.
fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: [u32; 64] =
        std::array::from_fn(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32);
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];

    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());

    for block in message.as_chunks::<64>().0 {
        let words = block.as_chunks::<4>().0;
        let m: [u32; 16] = std::array::from_fn(|i| u32::from_le_bytes(words[i]));
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    let mut digest = [0; 16];
    for (out, s) in digest.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        out.copy_from_slice(&s.to_le_bytes());
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_the_rfcs_examples() {
        let cases = [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ];
        for (text, want) in cases {
            assert_eq!(hex(&md5(text.as_bytes())), want, "{text:?}");
        }
    }

    #[test]
    fn every_checksum_is_md5_or_sha1_and_every_name_is_one_files() {
        for f in KNOWN {
            assert!(
                matches!(f.hash.len(), 32 | 40) && f.hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "{}",
                f.file
            );
            assert_eq!(f.hash, f.hash.to_lowercase(), "{}", f.file);
            assert_eq!(
                KNOWN.iter().filter(|g| g.file == f.file).count(),
                1,
                "{}",
                f.file
            );
            assert_eq!(
                KNOWN.iter().filter(|g| g.hash == f.hash).count(),
                1,
                "{}",
                f.file
            );
        }
    }

    #[test]
    fn retroarchs_system_folder_is_read_from_its_settings() {
        let text = "video_driver = \"vulkan\"\nsystem_directory = \"/games/bios\"\n";
        assert_eq!(
            setting(text, "system_directory").as_deref(),
            Some("/games/bios")
        );
        assert_eq!(
            setting("system_directory = \"default\"", "system_directory"),
            None
        );
        assert_eq!(setting("system_directory = \"\"", "system_directory"), None);
    }

    #[test]
    fn only_known_files_are_copied_and_nothing_is_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let system = dir.path().join("system");
        std::fs::create_dir_all(from.join("deeper")).unwrap();
        // Nobody's firmware: "abc" stands in, with its checksum, for a BIOS.
        std::fs::write(from.join("deeper/My Saturn.bin"), "abc").unwrap();
        std::fs::write(from.join("notes.txt"), "not firmware").unwrap();
        let fake = Firmware {
            system: Saturn,
            file: "sega_101.bin",
            what: "test",
            hash: "900150983cd24fb0d6963f7d28e17f72",
        };
        let fake: &'static Firmware = Box::leak(Box::new(fake));

        let path = from.join("deeper/My Saturn.bin");
        assert!(matches!(install(&path, fake, &system), Added::Copied(_)));
        assert_eq!(std::fs::read(system.join("sega_101.bin")).unwrap(), b"abc");
        // There already, as something this does not recognise: left alone.
        assert!(matches!(install(&path, fake, &system), Added::Taken(_)));

        let added = add(&[from], &system);
        assert_eq!(added.len(), 2);
        assert!(added.iter().all(|(_, a)| matches!(a, Added::Unknown)));
    }
}
