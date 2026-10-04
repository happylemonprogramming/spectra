//! Just enough ISO 9660 to find a console's boot file.
//!
//! Discs identify themselves through a file in the root directory -
//! SYSTEM.CNF on a PlayStation, AUTORUN.INF on a PC disc - so all this needs
//! is the primary volume descriptor, the root directory, and the ability to
//! read one small file out of it. A DVD's IFO files are a folder down, so a
//! folder of the root can be listed too; no deeper. No Joliet, no Rock Ridge.
//!
//! Ported from Rainbow Player's `src/lib/game/iso9660.ts`.

use crate::{Result, SECTOR};

/// Reads `count` cooked sectors from `lba`, relative to the start of the
/// track the filesystem is on.
pub type ReadSectors<'a> = dyn FnMut(u32, u32) -> Result<Vec<u8>> + 'a;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoFile {
    /// Upper-cased, with the `;1` version suffix removed.
    pub name: String,
    pub lba: u32,
    pub size: u32,
    pub directory: bool,
}

#[derive(Debug, Clone)]
pub struct IsoVolume {
    /// Who the disc says it is for: "PLAYSTATION" on Sony's consoles.
    pub system_id: String,
    pub volume_id: String,
    /// Volume size in sectors.
    pub sectors: u32,
    pub root: Vec<IsoFile>,
}

impl IsoVolume {
    pub fn find(&self, name: &str) -> Option<&IsoFile> {
        let name = name.to_ascii_uppercase();
        self.root.iter().find(|f| f.name == name)
    }

    pub fn has_file(&self, name: &str) -> bool {
        self.find(name).is_some_and(|f| !f.directory)
    }

    pub fn has_dir(&self, name: &str) -> bool {
        self.find(name).is_some_and(|f| f.directory)
    }
}

fn text(b: &[u8], from: usize, len: usize) -> String {
    String::from_utf8_lossy(&b[from..from + len])
        .trim_matches(|c: char| c == ' ' || c == '\0')
        .to_string()
}

pub(crate) fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// The volume, or None if sector 16 is not an ISO 9660 primary volume
/// descriptor.
pub fn read_iso(read: &mut ReadSectors) -> Result<Option<IsoVolume>> {
    let pvd = read(16, 1)?;
    if pvd.len() < SECTOR || pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return Ok(None);
    }

    // The root directory record is embedded in the descriptor at 156.
    let root_lba = u32le(&pvd, 156 + 2);
    let root_size = u32le(&pvd, 156 + 10);
    let sectors = root_size.div_ceil(SECTOR as u32).clamp(1, 64);
    let dir = read(root_lba, sectors)?;
    Ok(Some(IsoVolume {
        system_id: text(&pvd, 8, 32),
        volume_id: text(&pvd, 40, 32),
        sectors: u32le(&pvd, 80),
        root: parse_directory(&dir, root_size as usize),
    }))
}

fn parse_directory(data: &[u8], size: usize) -> Vec<IsoFile> {
    let mut files = Vec::new();
    let end = size.min(data.len());
    let mut at = 0;
    while at < end {
        let length = usize::from(data[at]);
        // Records never straddle a sector; a zero length pads to the next one.
        if length == 0 {
            at = (at / SECTOR + 1) * SECTOR;
            continue;
        }
        if at + 33 > end || at + length > data.len() {
            break;
        }
        let name_len = usize::from(data[at + 32]);
        let raw = &data[at + 33..(at + 33 + name_len).min(data.len())];
        // Skip "." and "..", which are the single bytes 0 and 1.
        if !(name_len == 1 && (raw[0] == 0 || raw[0] == 1)) {
            let name = String::from_utf8_lossy(raw).to_ascii_uppercase();
            let name = name.split(';').next().unwrap_or_default();
            files.push(IsoFile {
                name: name.strip_suffix('.').unwrap_or(name).to_string(),
                lba: u32le(data, at + 2),
                size: u32le(data, at + 10),
                directory: data[at + 25] & 2 != 0,
            });
        }
        at += length;
    }
    files
}

/// A file from the root directory, or None if there is no such file.
pub fn read_root_file(
    read: &mut ReadSectors,
    volume: &IsoVolume,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    let Some(file) = volume.find(name).filter(|f| !f.directory) else {
        return Ok(None);
    };
    read_file(read, file).map(Some)
}

/// What is in a folder of the root, or None if there is no such folder.
pub fn read_root_dir(
    read: &mut ReadSectors,
    volume: &IsoVolume,
    name: &str,
) -> Result<Option<Vec<IsoFile>>> {
    let Some(dir) = volume.find(name).filter(|f| f.directory) else {
        return Ok(None);
    };
    let sectors = dir.size.div_ceil(SECTOR as u32).clamp(1, 64);
    let data = read(dir.lba, sectors)?;
    Ok(Some(parse_directory(&data, dir.size as usize)))
}

/// A small file's contents. Boot, config and IFO files are small; a bogus
/// size is not let read a whole disc.
pub fn read_file(read: &mut ReadSectors, file: &IsoFile) -> Result<Vec<u8>> {
    let size = file.size.min(1 << 20);
    let mut data = read(file.lba, size.div_ceil(SECTOR as u32).max(1))?;
    data.truncate(size as usize);
    Ok(data)
}
