//! A kept game's music, as the stage's track list.
//!
//! The game plays as before; its music plays as an album would, from the
//! copy. Only what `spectra_core::soundtrack` hears as music is listed:
//! speech, stings and silence stay with the game. Tracks keep the disc's own
//! numbers, so "Track 4" is track 4 wherever else it is looked up.

use spectra_core::library::Entry;
use spectra_core::soundtrack::{self, Kind};

use crate::album::{self, Track};
use crate::audio::{self, Source, Span};

#[derive(Debug, Clone)]
pub struct Soundtrack {
    pub tracks: Vec<Track>,
    pub source: Source,
    pub spans: Vec<Span>,
}

/// The music on a kept game's copy, or None if it has none. Reads the whole
/// of its CD audio, so off the UI thread.
pub fn load(entry: &Entry) -> Option<Soundtrack> {
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

fn keep<T>(items: Vec<T>, wanted: impl Fn(usize) -> bool) -> Vec<T> {
    items
        .into_iter()
        .enumerate()
        .filter_map(|(i, item)| wanted(i).then_some(item))
        .collect()
}
