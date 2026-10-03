//! Titles for discs that only carry a serial.
//!
//! A PlayStation disc names itself only by its boot executable, so the title
//! comes from a table. Both are Rainbow Player's (see `data/README.md`):
//!
//!   - PS2: PCSX2's GPL-3.0 game index (`data/ps2.json`).
//!   - PS1: libretro-database's Redump and developer data, CC BY-SA 4.0, with
//!     publisher, year, and a LaunchBox scan of the disc's printed side where
//!     one can only be this disc (`data/ps1.json`).

use std::collections::HashMap;
use std::sync::OnceLock;

type TitleTable = HashMap<String, (String, String)>;

fn ps2_titles() -> &'static TitleTable {
    static TABLE: OnceLock<TitleTable> = OnceLock::new();
    TABLE.get_or_init(|| serde_json::from_str(include_str!("../data/ps2.json")).unwrap_or_default())
}

/// A PS2 game's title and region ("NTSC-U", "PAL-E", ...) by serial.
pub fn ps2_title(serial: &str) -> Option<(&'static str, &'static str)> {
    ps2_titles()
        .get(serial)
        .filter(|(name, _)| !name.is_empty())
        .map(|(name, region)| (name.as_str(), region.as_str()))
}

/// What the PS1 table knows about a disc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameMeta {
    pub title: String,
    pub region: Option<String>,
    pub publisher: Option<String>,
    pub year: Option<String>,
    pub disc_art: Option<String>,
}

/// `[title, region, publisher, year, set serial, disc image]`, with "" for
/// unknown and trailing unknowns left off; keyed by serial, or by
/// `@<sectors>` for the discs of a set, which all carry the set's serial.
type Ps1Table = HashMap<String, Vec<String>>;

fn ps1_table() -> &'static Ps1Table {
    static TABLE: OnceLock<Ps1Table> = OnceLock::new();
    TABLE.get_or_init(|| serde_json::from_str(include_str!("../data/ps1.json")).unwrap_or_default())
}

/// A PS1 disc by serial and size. A size match counts only where it agrees
/// with the serial, or the serial alone would have found nothing.
pub fn ps1_meta(serial: Option<&str>, sectors: Option<u32>) -> Option<GameMeta> {
    let table = ps1_table();
    let by_serial = serial.and_then(|s| table.get(s));
    let by_size = sectors.and_then(|n| table.get(&format!("@{n}")));
    let entry = match (by_size, by_serial) {
        (Some(size), None) => size,
        (Some(size), Some(_)) if size.get(4).map(String::as_str) == serial => size,
        (_, Some(serial)) => serial,
        (None, None) => return None,
    };
    let field = |i: usize| entry.get(i).filter(|v| !v.is_empty()).cloned();
    Some(GameMeta {
        title: field(0)?,
        region: field(1),
        publisher: field(2),
        year: field(3),
        disc_art: field(5),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_tomb_raider() {
        let meta = ps1_meta(Some("SLUS-00152"), Some(267_847)).unwrap();
        assert_eq!(meta.title, "Tomb Raider");
        assert_eq!(meta.publisher.as_deref(), Some("Eidos Interactive"));
        assert_eq!(meta.year.as_deref(), Some("1996"));
        assert!(meta.disc_art.is_some_and(|f| f.ends_with(".png")));
    }

    #[test]
    fn a_set_disc_is_told_apart_by_its_size() {
        let table = ps1_table();
        let (key, entry) = table
            .iter()
            .find(|(k, v)| k.starts_with('@') && v.get(4).is_some_and(|s| !s.is_empty()))
            .unwrap();
        let sectors = key[1..].parse().unwrap();
        let meta = ps1_meta(Some(&entry[4]), Some(sectors)).unwrap();
        assert_eq!(meta.title, entry[0]);
        // A size that disagrees with the serial is not trusted.
        let other = ps1_meta(Some("SLUS-00152"), Some(sectors)).unwrap();
        assert_eq!(other.title, "Tomb Raider");
    }

    #[test]
    fn knows_a_famous_serial() {
        let (name, region) = super::ps2_title("SCUS-97111").unwrap();
        assert!(!name.is_empty());
        assert_eq!(region, "NTSC-U");
    }
}
