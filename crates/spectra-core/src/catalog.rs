//! Titles for discs that only carry a serial.
//!
//! A PlayStation 2 disc names itself only by its boot executable, so the
//! title comes from a table built from PCSX2's GPL-3.0 game index
//! (`scripts/ps2-titles.mjs` writes `data/ps2.json`).

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

#[cfg(test)]
mod tests {
    #[test]
    fn knows_a_famous_serial() {
        let (name, region) = super::ps2_title("SCUS-97111").unwrap();
        assert!(!name.is_empty());
        assert_eq!(region, "NTSC-U");
    }
}
