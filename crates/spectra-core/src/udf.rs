//! Just enough UDF to see what is in the root directory.
//!
//! DVD-Video puts VIDEO_TS there and a Blu-ray puts BDMV there, which is how
//! a film is told apart from any other disc. DVDs carry UDF 1.02 alongside an
//! ISO 9660 bridge; Blu-rays carry UDF 2.50 alone, with the directories in a
//! *metadata partition* layered over the physical one.
//!
//! The walk is the standard one: anchor at sector 256, the volume descriptor
//! sequence it points to, the partition and logical volume descriptors found
//! there, the file set descriptor, the root directory.
//!
//! Ported from Rainbow Player's `src/lib/dvd/udf.ts`, with metadata
//! partitions added for Blu-ray.

use crate::iso9660::{ReadSectors, u32le as u32_at};
use crate::{Error, Result, SECTOR};

const TAG_PVD: u16 = 1;
const TAG_ANCHOR: u16 = 2;
const TAG_PARTITION: u16 = 5;
const TAG_LOGICAL_VOLUME: u16 = 6;
const TAG_TERMINATOR: u16 = 8;
const TAG_FILE_SET: u16 = 256;
const TAG_FID: u16 = 257;
const TAG_FILE_ENTRY: u16 = 261;
const TAG_EXTENDED_FILE_ENTRY: u16 = 266;

const METADATA_PARTITION: &[u8] = b"*UDF Metadata Partition";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdfEntry {
    pub name: String,
    pub directory: bool,
}

#[derive(Debug, Clone)]
pub struct UdfVolume {
    pub label: String,
    pub root: Vec<UdfEntry>,
}

impl UdfVolume {
    pub fn has_dir(&self, name: &str) -> bool {
        self.root
            .iter()
            .any(|e| e.directory && e.name.eq_ignore_ascii_case(name))
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn tag_id(b: &[u8], o: usize) -> u16 {
    if b.len() < o + 2 { 0 } else { u16_at(b, o) }
}

/// An OSTA compressed Unicode string: one byte of width, then 8- or 16-bit
/// characters.
fn dstring(b: &[u8], offset: usize, length: usize) -> String {
    let Some(bytes) = b.get(offset..offset + length).filter(|s| !s.is_empty()) else {
        return String::new();
    };
    match bytes[0] {
        8 => bytes[1..].iter().map(|&c| char::from(c)).collect(),
        16 => char::decode_utf16(
            bytes[1..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_be_bytes([p[0], p[1]])),
        )
        .map(|c| c.unwrap_or('\u{fffd}'))
        .collect(),
        _ => String::new(),
    }
}

/// A fixed-size dstring field, whose last byte holds the used length.
fn dstring_field(b: &[u8], offset: usize, size: usize) -> String {
    dstring(b, offset, usize::from(b[offset + size - 1]))
        .trim_end_matches('\0')
        .to_string()
}

/// How a partition reference number turns into a sector on the disc.
enum Map {
    Physical,
    /// UDF 2.50: blocks are numbered within the metadata file, whose extents
    /// sit in the physical partition.
    Metadata {
        extents: Vec<(u32, u32)>,
    },
}

struct Partitions {
    start: u32,
    maps: Vec<Map>,
}

impl Partitions {
    fn lba(&self, reference: u16, lbn: u32) -> Result<u32> {
        match self
            .maps
            .get(usize::from(reference))
            .unwrap_or(&Map::Physical)
        {
            Map::Physical => Ok(self.start + lbn),
            Map::Metadata { extents } => {
                let mut skip = lbn;
                for &(position, blocks) in extents {
                    if skip < blocks {
                        return Ok(self.start + position + skip);
                    }
                    skip -= blocks;
                }
                Err(Error::format(
                    "block is past the end of the UDF metadata partition",
                ))
            }
        }
    }
}

/// The volume, or None if there is no UDF anchor at sector 256.
pub fn read_udf(read: &mut ReadSectors) -> Result<Option<UdfVolume>> {
    let anchor = read(256, 1)?;
    if tag_id(&anchor, 0) != TAG_ANCHOR {
        return Ok(None);
    }
    let vds_length = u32_at(&anchor, 16);
    let vds_start = u32_at(&anchor, 20);

    let mut partition_start = None;
    let mut logical_volume = None;
    let mut label = String::new();
    let vds = read(vds_start, vds_length.div_ceil(SECTOR as u32).clamp(1, 32))?;
    for d in vds.as_chunks::<SECTOR>().0 {
        match tag_id(d, 0) {
            TAG_PVD if label.is_empty() => label = dstring_field(d, 24, 32),
            TAG_PARTITION if partition_start.is_none() => partition_start = Some(u32_at(d, 188)),
            TAG_LOGICAL_VOLUME => logical_volume = Some(d.to_vec()),
            TAG_TERMINATOR => break,
            _ => {}
        }
    }
    let (Some(start), Some(lvd)) = (partition_start, logical_volume) else {
        return Err(Error::format("the UDF volume is incomplete"));
    };
    let volume_label = dstring_field(&lvd, 84, 128);
    if !volume_label.is_empty() {
        label = volume_label;
    }

    let mut partitions = Partitions {
        start,
        maps: Vec::new(),
    };
    let map_count = u32_at(&lvd, 268).min(8);
    let mut at = 440;
    for _ in 0..map_count {
        let (kind, length) = (lvd[at], usize::from(lvd[at + 1]));
        if length == 0 || at + length > lvd.len() {
            break;
        }
        let is_metadata = kind == 2 && lvd[at + 5..].starts_with(METADATA_PARTITION);
        partitions.maps.push(if is_metadata {
            let entry = read_file_entry(read, &partitions, 0, u32_at(&lvd, at + 40))?;
            Map::Metadata {
                extents: entry.extents,
            }
        } else {
            Map::Physical
        });
        at += length;
    }

    // Logical volume contents use: a long_ad pointing at the file set descriptor.
    let fsd_lbn = u32_at(&lvd, 252);
    let fsd_ref = u16_at(&lvd, 256);
    let fsd = read(partitions.lba(fsd_ref, fsd_lbn)?, 1)?;
    if tag_id(&fsd, 0) != TAG_FILE_SET {
        return Err(Error::format("the UDF file set is missing"));
    }
    let root_lbn = u32_at(&fsd, 404);
    let root_ref = u16_at(&fsd, 408);

    let root = read_directory(read, &partitions, root_ref, root_lbn)?;
    Ok(Some(UdfVolume { label, root }))
}

struct FileEntry {
    size: u64,
    /// Embedded data, for directories small enough to live inside their ICB.
    embedded: Option<Vec<u8>>,
    /// (block within the entry's partition, blocks) for each extent.
    extents: Vec<(u32, u32)>,
}

fn read_file_entry(
    read: &mut ReadSectors,
    partitions: &Partitions,
    reference: u16,
    lbn: u32,
) -> Result<FileEntry> {
    let fe = read(partitions.lba(reference, lbn)?, 1)?;
    let (ad_offset, ad_length) = match tag_id(&fe, 0) {
        TAG_FILE_ENTRY => (176 + u32_at(&fe, 168) as usize, u32_at(&fe, 172) as usize),
        TAG_EXTENDED_FILE_ENTRY => (216 + u32_at(&fe, 208) as usize, u32_at(&fe, 212) as usize),
        _ => {
            return Err(Error::format(format!(
                "expected a UDF file entry at block {lbn}"
            )));
        }
    };
    let ad_end = (ad_offset + ad_length).min(fe.len());
    let size = u64::from(u32_at(&fe, 56)) | (u64::from(u32_at(&fe, 60)) << 32);
    let flags = u16_at(&fe, 16 + 18) & 7;

    if flags == 3 {
        return Ok(FileEntry {
            size,
            embedded: Some(fe[ad_offset.min(ad_end)..ad_end].to_vec()),
            extents: vec![],
        });
    }
    // Short allocation descriptors (0) are 8 bytes; long ones (1) are 16 and
    // add a partition reference, which on these discs is always the entry's own.
    let stride = if flags == 1 { 16 } else { 8 };
    let mut extents = Vec::new();
    let mut at = ad_offset;
    while at + stride <= ad_end {
        let length = u32_at(&fe, at) & 0x3fff_ffff;
        if length == 0 {
            break;
        }
        extents.push((u32_at(&fe, at + 4), length.div_ceil(SECTOR as u32)));
        at += stride;
    }
    Ok(FileEntry {
        size,
        embedded: None,
        extents,
    })
}

fn read_directory(
    read: &mut ReadSectors,
    partitions: &Partitions,
    reference: u16,
    lbn: u32,
) -> Result<Vec<UdfEntry>> {
    let entry = read_file_entry(read, partitions, reference, lbn)?;
    let data = match entry.embedded {
        Some(data) => data,
        None => {
            let &(position, _) = entry
                .extents
                .first()
                .ok_or_else(|| Error::format("empty UDF directory"))?;
            let sectors = (entry.size.div_ceil(SECTOR as u64) as u32).clamp(1, 64);
            read(partitions.lba(reference, position)?, sectors)?
        }
    };

    let end = data.len().min(entry.size as usize);
    let mut entries = Vec::new();
    let mut at = 0;
    while at + 38 <= end {
        if tag_id(&data, at) != TAG_FID {
            break;
        }
        let characteristics = data[at + 18];
        let name_length = usize::from(data[at + 19]);
        let implementation_use = usize::from(u16_at(&data, at + 36));
        // Bit 3 is the parent directory entry.
        if characteristics & 0x08 == 0 {
            entries.push(UdfEntry {
                name: dstring(&data, at + 38 + implementation_use, name_length),
                directory: characteristics & 0x02 != 0,
            });
        }
        at += (38 + implementation_use + name_length + 3) & !3;
    }
    Ok(entries)
}
