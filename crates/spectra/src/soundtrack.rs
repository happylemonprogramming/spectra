//! A kept game's music, as an album of its own in the library.
//!
//! The game stays as it was; beside it on the shelf goes its soundtrack,
//! which plays as an album does, from the game's copy. It is not a copy of
//! its own: nothing is written for it, and it goes when the game does. Only
//! what `spectra_core::soundtrack` hears as music is listed: speech, stings
//! and silence stay with the game. CD audio tracks keep the disc's own
//! numbers, so "Track 4" is track 4 wherever else it is looked up; music
//! files are named by their file names.

use spectra_core::library::Entry;
use spectra_core::soundtrack::{self, Kind, MusicFile};

use crate::album::{self, Track};
use crate::audio::{self, Source, Span};

#[derive(Debug, Clone)]
pub struct Soundtrack {
    pub tracks: Vec<Track>,
    pub source: Source,
    pub spans: Vec<Span>,
}

/// What a soundtrack's ID adds to its game's. Never in a copy's own ID,
/// which is plain, so the library will not delete a folder by it.
const SUFFIX: &str = "+soundtrack";

/// The shelf's entry for a game's soundtrack: the game's copy, under an ID
/// and a title of its own.
pub fn entry(game: &Entry) -> Entry {
    let mut entry = game.clone();
    entry.meta.id.push_str(SUFFIX);
    entry.meta.title.push_str(" (Soundtrack)");
    entry
}

pub fn is_entry(entry: &Entry) -> bool {
    entry.meta.id.ends_with(SUFFIX)
}

/// The ID of the game a soundtrack entry is for.
pub fn game_id(entry: &Entry) -> &str {
    entry.meta.id.strip_suffix(SUFFIX).unwrap_or(&entry.meta.id)
}

/// The music on a kept game's copy - its CD audio, or else music files -
/// or None if it has none. Reads all of it, so off the UI thread.
pub fn load(entry: &Entry) -> Option<Soundtrack> {
    cd_music(entry).or_else(|| file_music(entry))
}

fn cd_music(entry: &Entry) -> Option<Soundtrack> {
    let toc = entry.toc()?;
    let kinds = soundtrack::sort_cd(&entry.bin(), &toc)
        .inspect_err(|e| eprintln!("spectra: {}: {e}", entry.meta.id))
        .ok()?;
    // Both lists are the audio tracks in order, as `kinds` is.
    let music = |i: usize| kinds.get(i) == Some(&Kind::Music);
    let tracks: Vec<Track> = keep(album::cd_tracks(&toc), music);
    let spans = keep(audio::spans(&toc), music);
    (!tracks.is_empty()).then(|| Soundtrack {
        tracks,
        source: Source::Image(entry.bin()),
        spans,
    })
}

fn file_music(entry: &Entry) -> Option<Soundtrack> {
    let mut disc = spectra_core::image::open(&entry.cue()).ok()?;
    let files = soundtrack::music_files(disc.as_mut())
        .inspect_err(|e| eprintln!("spectra: {}: {e}", entry.meta.id))
        .ok()?;
    (!files.is_empty()).then(|| Soundtrack {
        tracks: files.iter().map(file_track).collect(),
        spans: audio::file_spans(&files),
        source: Source::Files {
            cue: entry.cue(),
            files,
        },
    })
}

fn file_track(file: &MusicFile) -> Track {
    Track {
        title: title(&file.name),
        seconds: file.frames() / file.rate,
        detail: None,
    }
}

/// A file name as a title: `LONDON1` is "London 1", `BGM_TITLE` "Bgm Title".
fn title(name: &str) -> String {
    let mut out = String::new();
    let mut last: Option<char> = None;
    for c in name.chars() {
        let c = if c == '_' || c == '-' { ' ' } else { c };
        let starts_word = last.is_none_or(|l| l == ' ');
        if last.is_some_and(|l| l != ' ' && c != ' ' && l.is_ascii_digit() != c.is_ascii_digit()) {
            out.push(' ');
            out.push(c.to_ascii_uppercase());
        } else if starts_word {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push(c.to_ascii_lowercase());
        }
        last = Some(c);
    }
    out
}

fn keep<T>(items: Vec<T>, wanted: impl Fn(usize) -> bool) -> Vec<T> {
    items
        .into_iter()
        .enumerate()
        .filter_map(|(i, item)| wanted(i).then_some(item))
        .collect()
}

#[cfg(test)]
mod tests {
    use spectra_core::library::{Entry, Meta};

    #[test]
    fn a_soundtrack_is_its_games_copy_under_an_id_the_library_will_not_delete() {
        let game = Entry {
            dir: "/library/SLUS-00152".into(),
            meta: Meta {
                id: "SLUS-00152".into(),
                title: "Tomb Raider".into(),
                system: Some(spectra_core::GameSystem::Ps1),
                serial: None,
                publisher: None,
                year: None,
                region: None,
                disc_art: None,
                artist: None,
                tracks: Vec::new(),
                sectors: 0,
                unreadable: 0,
                created: 0,
            },
        };
        let music = super::entry(&game);
        assert!(super::is_entry(&music) && !super::is_entry(&game));
        assert_eq!(super::game_id(&music), "SLUS-00152");
        assert_eq!(music.meta.title, "Tomb Raider (Soundtrack)");
        assert_eq!(music.dir, game.dir);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("SLUS-00152")).unwrap();
        assert!(spectra_core::library::remove_in(root.path(), &music).is_err());
        assert!(root.path().join("SLUS-00152").exists());
    }

    #[test]
    fn file_names_read_as_titles() {
        assert_eq!(super::title("LONDON1"), "London 1");
        assert_eq!(super::title("BGM_TITLE"), "Bgm Title");
        assert_eq!(super::title("STAGE12B"), "Stage 12 B");
    }
}
