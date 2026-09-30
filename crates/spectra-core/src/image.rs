//! Disc images on disk, so everything above can be tested and used without a
//! drive.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::cue::CueDisc;
use crate::disc::{Disc, Media, RAW_SECTOR, SECTOR, Toc};
use crate::{Error, Result};

/// The sync pattern that opens every raw data sector.
const SYNC: [u8; 12] = [
    0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0,
];

/// Open an image by what it is: a cue sheet, a raw `.bin` with no cue, or a
/// plain 2048-byte-sector `.iso`.
pub fn open(path: &Path) -> Result<Box<dyn Disc>> {
    if let Err(e) = std::fs::metadata(path) {
        return Err(std::io::Error::new(e.kind(), format!("{}: {e}", path.display())).into());
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if extension == "cue" {
        return Ok(Box::new(CueDisc::open(path)?));
    }
    if !["iso", "bin", "img", "gcm", "xiso"].contains(&extension.as_str()) {
        return Err(Error::Unsupported(format!(
            "{}: not a disc image this knows (.iso, .bin, .cue)",
            path.display()
        )));
    }
    Ok(Box::new(FlatImage::open(path)?))
}

/// One track's worth of sectors in a file: cooked, or raw with a sync header
/// on each.
pub struct FlatImage {
    path: PathBuf,
    file: File,
    sectors: u32,
    raw: bool,
}

impl FlatImage {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        let mut head = [0u8; 12];
        let raw =
            size % RAW_SECTOR as u64 == 0 && file.read_exact(&mut head).is_ok() && head == SYNC;
        let sector = if raw { RAW_SECTOR } else { SECTOR };
        Ok(Self {
            path: path.to_path_buf(),
            file,
            sectors: (size / sector as u64) as u32,
            raw,
        })
    }
}

impl Disc for FlatImage {
    fn describe(&self) -> String {
        self.path.display().to_string()
    }

    /// Guessed from the size, which is all an image says: nothing over a CD's
    /// 900 MB is a CD, and nothing over a dual-layer DVD's 8.5 GB is a DVD.
    fn media(&mut self) -> Result<Media> {
        let bytes = u64::from(self.sectors) * SECTOR as u64;
        Ok(if bytes <= 900_000_000 {
            Media::Cd
        } else if bytes <= 8_600_000_000 {
            Media::Dvd
        } else {
            Media::BluRay
        })
    }

    fn toc(&mut self) -> Result<Option<Toc>> {
        Ok(None)
    }

    fn read(&mut self, lba: u32, count: u32) -> Result<Vec<u8>> {
        if lba.checked_add(count).is_none_or(|end| end > self.sectors) {
            return Err(Error::format(format!(
                "sectors {lba}+{count} are past the end of the image"
            )));
        }
        if !self.raw {
            let mut out = vec![0; count as usize * SECTOR];
            self.file
                .seek(SeekFrom::Start(u64::from(lba) * SECTOR as u64))?;
            self.file.read_exact(&mut out)?;
            return Ok(out);
        }
        let mut raw = vec![0; count as usize * RAW_SECTOR];
        self.file
            .seek(SeekFrom::Start(u64::from(lba) * RAW_SECTOR as u64))?;
        self.file.read_exact(&mut raw)?;
        Ok(raw
            .as_chunks::<RAW_SECTOR>()
            .0
            .iter()
            // Byte 15 is the mode: 2 puts an XA subheader before the data.
            .flat_map(|s| {
                if s[15] == 2 {
                    &s[24..24 + SECTOR]
                } else {
                    &s[16..16 + SECTOR]
                }
            })
            .copied()
            .collect())
    }

    fn capacity(&mut self) -> Result<Option<u32>> {
        Ok(Some(self.sectors))
    }
}
