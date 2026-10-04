//! How long a DVD's feature runs, from its IFO files.
//!
//! A DVD carries nothing to look it up by but its volume label, and a label
//! is often too short to name one film: "DUDE" is two films called exactly
//! that and a third that starts with it. The feature's running time tells
//! them apart. Each title set's `VTS_nn_0.IFO` lists its program chains with
//! how long each plays, and on a film's disc the longest is the film.
//!
//! Rainbow Player's `src/lib/dvd/disc.ts` adds up the chains a title's
//! chapters play from; the longest single chain is the same number for the
//! usual disc, and needs only the title sets' own IFO files, not the VMG's.

use crate::Result;
use crate::iso9660::{IsoVolume, ReadSectors, read_file, read_root_dir};

/// The longest program chain on the disc, in whole seconds, or None if the
/// disc has no IFO files that say.
pub fn feature_seconds(read: &mut ReadSectors, volume: &IsoVolume) -> Result<Option<u32>> {
    let Some(files) = read_root_dir(read, volume, "VIDEO_TS")? else {
        return Ok(None);
    };
    let mut longest: Option<u32> = None;
    for file in files.iter().filter(|f| is_title_set_ifo(&f.name)) {
        let ifo = read_file(read, file)?;
        if let Some(seconds) = longest_chain(&ifo) {
            longest = Some(longest.map_or(seconds, |l| l.max(seconds)));
        }
    }
    Ok(longest)
}

/// "VTS_01_0.IFO"; the backup copies end in `.BUP`.
fn is_title_set_ifo(name: &str) -> bool {
    name.len() == 12
        && name.starts_with("VTS_")
        && name.ends_with("_0.IFO")
        && name[4..6].bytes().all(|b| b.is_ascii_digit())
}

fn u16be(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32be(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// A `dvd_time_t`: hours, minutes and seconds in BCD, then frames. Frames
/// are left out; nothing here needs better than a second.
fn dvd_time(b: &[u8], o: usize) -> Option<u32> {
    let bcd = |v: u8| u32::from(v >> 4) * 10 + u32::from(v & 0x0f);
    let t = b.get(o..o + 3)?;
    Some(bcd(t[0]) * 3600 + bcd(t[1]) * 60 + bcd(t[2]))
}

/// The longest chain in one title set's IFO.
fn longest_chain(ifo: &[u8]) -> Option<u32> {
    if ifo.get(..12)? != b"DVDVIDEO-VTS" {
        return None;
    }
    // The program chain table's sector, from the start of the IFO.
    let table = usize::try_from(u32be(ifo, 0xcc)?).ok()? * crate::SECTOR;
    let chains = u16be(ifo, table)?;
    (0..usize::from(chains))
        .filter_map(|i| {
            let at = table + usize::try_from(u32be(ifo, table + 8 + i * 8 + 4)?).ok()?;
            dvd_time(ifo, at + 4)
        })
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A title set's IFO with its chain table at sector 1, and chains of
    /// these lengths, in (hours, minutes, seconds).
    fn ifo(chains: &[(u8, u8, u8)]) -> Vec<u8> {
        let bcd = |v: u8| ((v / 10) << 4) | (v % 10);
        let mut b = vec![0; 2 * crate::SECTOR];
        b[..12].copy_from_slice(b"DVDVIDEO-VTS");
        b[0xcc..0xd0].copy_from_slice(&1u32.to_be_bytes());
        let table = crate::SECTOR;
        b[table..table + 2].copy_from_slice(&(chains.len() as u16).to_be_bytes());
        for (i, &(h, m, s)) in chains.iter().enumerate() {
            let offset = 0x100 + i * 0x40;
            let entry = table + 8 + i * 8;
            b[entry + 4..entry + 8].copy_from_slice(&(offset as u32).to_be_bytes());
            let at = table + offset + 4;
            b[at..at + 4].copy_from_slice(&[bcd(h), bcd(m), bcd(s), 0xc0]);
        }
        b
    }

    #[test]
    fn the_longest_chain_is_the_feature() {
        assert_eq!(
            longest_chain(&ifo(&[(0, 2, 30), (1, 23, 5), (0, 40, 0)])),
            Some(4985)
        );
        assert_eq!(longest_chain(&ifo(&[])), None);
        assert_eq!(longest_chain(b"DVDVIDEO-VMG"), None);
    }

    #[test]
    fn only_title_set_ifos() {
        assert!(is_title_set_ifo("VTS_01_0.IFO"));
        assert!(!is_title_set_ifo("VTS_01_0.BUP"));
        assert!(!is_title_set_ifo("VTS_01_1.VOB"));
        assert!(!is_title_set_ifo("VIDEO_TS.IFO"));
    }
}
