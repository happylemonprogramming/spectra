//! What is on screen: an album, its tracks and its cover.
//!
//! An album is whatever disc is in the drive. For working on the screen
//! without one, it can also come from a small JSON file:
//!
//! ```json
//! { "title": "…", "artist": "…", "year": 1997, "cover": "cover.jpg",
//!   "tracks": [["Airbag", 284], ["Paranoid Android", 383]] }
//! ```

use std::path::Path;

use image::RgbaImage;
use serde::Deserialize;
use spectra_core::library::{Entry, Names};
use spectra_core::{DiscKind, GameSystem, Report, Toc};

use crate::watch::DriveState;

/// CD frames per second: one TOC address step.
const FRAMES_PER_SECOND: u32 = 75;

pub struct Album {
    pub title: String,
    pub artist: String,
    pub year: Option<u16>,
    pub tracks: Vec<Track>,
    pub cover: RgbaImage,
    /// A scan of the disc's printed side, to wear instead of the cover.
    pub face: Option<RgbaImage>,
    /// The line under the artist, where the track count would go.
    pub details: Option<String>,
    /// What Spectra can and cannot do with this disc yet.
    pub note: Option<String>,
    /// Whether choosing a track plays it.
    pub playable: bool,
}

pub struct Track {
    pub title: String,
    pub seconds: u32,
    /// Said at the end of the row instead of the track's length.
    pub detail: Option<String>,
}

#[derive(Deserialize)]
struct AlbumFile {
    title: String,
    artist: String,
    year: Option<u16>,
    cover: Option<String>,
    tracks: Vec<(String, u32)>,
}

impl Album {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file: AlbumFile =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let cover = match file.cover {
            Some(cover) => {
                let cover = path.parent().unwrap_or(Path::new(".")).join(cover);
                image::open(&cover)
                    .map_err(|e| format!("{}: {e}", cover.display()))?
                    .to_rgba8()
            }
            None => placeholder_cover(),
        };
        Ok(Self {
            title: file.title,
            artist: file.artist,
            year: file.year,
            tracks: file
                .tracks
                .into_iter()
                .map(|(title, seconds)| Track {
                    title,
                    seconds,
                    detail: None,
                })
                .collect(),
            cover,
            face: None,
            details: None,
            note: None,
            playable: true,
        })
    }

    /// Words on a blank disc: an invitation, or news from the drive.
    fn message(title: &str, artist: &str) -> Self {
        Self {
            title: title.into(),
            artist: artist.into(),
            year: None,
            tracks: Vec::new(),
            cover: placeholder_cover(),
            face: None,
            details: None,
            note: None,
            playable: false,
        }
    }

    /// A kept game, put on the stage from the library.
    pub fn from_kept_game(entry: &Entry) -> Self {
        let mut album = Self::message(
            &entry.meta.title,
            &[
                Some(
                    entry
                        .meta
                        .system
                        .map_or("Disc", GameSystem::name)
                        .to_string(),
                ),
                entry.meta.publisher.clone(),
                entry.meta.year.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("  ·  "),
        );
        album.details = [entry.meta.serial.clone(), entry.meta.region.clone()]
            .into_iter()
            .flatten()
            .reduce(|a, b| format!("{a}  ·  {b}"));
        album
    }

    /// A kept film.
    pub fn from_kept_film(entry: &Entry) -> Self {
        let mut album = Self::message(&entry.meta.title, "DVD-Video");
        album.details = entry.meta.year.clone();
        album
    }

    /// A kept audio CD, timed by its copy's table of contents.
    pub fn from_copy(entry: &Entry, toc: &Toc) -> Self {
        let mut album = Self::message("Audio CD", "Unknown artist");
        album.tracks = cd_tracks(toc);
        album.playable = !album.tracks.is_empty();
        album.name(&Names {
            title: entry.meta.title.clone(),
            artist: entry.meta.artist.clone().unwrap_or(album.artist.clone()),
            year: entry.meta.year.clone(),
            tracks: entry.meta.tracks.clone(),
        });
        album
    }

    /// Call an audio CD by its names, from MusicBrainz or a copy. The names
    /// are only the audio tracks'; the TOC already timed them.
    pub fn name(&mut self, names: &Names) {
        self.title.clone_from(&names.title);
        self.artist.clone_from(&names.artist);
        self.year = names
            .year
            .as_deref()
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse().ok());
        for (track, named) in self.tracks.iter_mut().zip(&names.tracks) {
            track.title = match &named.artist {
                Some(artist) => format!("{}  ·  {artist}", named.title),
                None => named.title.clone(),
            };
        }
    }

    pub fn no_disc() -> Self {
        Self::message("Insert a disc", "Music, films and games")
    }

    pub fn from_drive(state: &DriveState) -> Self {
        match state {
            DriveState::NoDrive => Self::message("Connect a disc drive", "Music, films and games"),
            DriveState::Empty => Self::no_disc(),
            DriveState::Reading => Self::message("Reading the disc…", "One moment"),
            DriveState::Unreadable(why) => Self::message("Can't read this disc", why),
            DriveState::Disc { report, .. } => Self::from_report(report),
        }
    }

    fn from_report(report: &Report) -> Self {
        let label = report.label.clone();
        let (title, artist, details, note) = match &report.kind {
            DiscKind::Audio { .. } => {
                let mut album = Self::message("Audio CD", "Unknown artist");
                album.tracks = report.toc.as_ref().map(cd_tracks).unwrap_or_default();
                album.playable = !album.tracks.is_empty();
                return album;
            }
            DiscKind::Game(game) => (
                game.title
                    .clone()
                    .or(label)
                    .unwrap_or("Unknown game".into()),
                // "PlayStation · Eidos Interactive · 1996"
                [
                    Some(game.system.name().to_string()),
                    game.publisher.clone(),
                    game.year.clone(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("  ·  "),
                [game.serial.clone(), game.region.clone()]
                    .into_iter()
                    .flatten()
                    .reduce(|a, b| format!("{a}  ·  {b}")),
                Some(match game.system.plan() {
                    Some(_) => "No emulator for this console is installed",
                    None => "Spectra can't play this console's games",
                }),
            ),
            DiscKind::DvdVideo { feature_seconds } => {
                let (title, artist, _, note) = film(label, "DVD-Video");
                let length = feature_seconds.map(|s| format!("{} min", (s + 30) / 60));
                (title, artist, length, note)
            }
            DiscKind::BluRayVideo => film(label, "Blu-ray"),
            DiscKind::VideoCd { super_vcd } => film(
                label,
                if *super_vcd {
                    "Super Video CD"
                } else {
                    "Video CD"
                },
            ),
            DiscKind::Pc => (
                label.unwrap_or("PC disc".into()),
                "PC disc".into(),
                None,
                Some("PC discs come later"),
            ),
            DiscKind::Data => (
                label.unwrap_or("Data disc".into()),
                "Data disc".into(),
                None,
                Some("Not a kind of disc Spectra plays"),
            ),
        };
        let mut album = Self::message(&title, &artist);
        album.details = details;
        album.note = note.map(Into::into);
        album
    }

    pub fn total_seconds(&self) -> u32 {
        self.tracks.iter().map(|t| t.seconds).sum()
    }
}

fn film(
    label: Option<String>,
    format: &str,
) -> (String, String, Option<String>, Option<&'static str>) {
    (
        label.map_or(format.into(), |l| crate::filmdb::tidy(&l)),
        format.into(),
        None,
        None,
    )
}

/// An audio CD's tracks, named by number and timed from the TOC: each runs
/// to the next one's start, the last to the lead-out. Data tracks are not
/// music, and are left out.
fn cd_tracks(toc: &spectra_core::Toc) -> Vec<Track> {
    let ends = toc
        .tracks
        .iter()
        .skip(1)
        .map(|t| t.lba)
        .chain([toc.leadout]);
    toc.tracks
        .iter()
        .zip(ends)
        .filter(|(track, _)| !track.data)
        .map(|(track, end)| Track {
            title: format!("Track {}", track.number),
            seconds: end.saturating_sub(track.lba) / FRAMES_PER_SECOND,
            detail: None,
        })
        .collect()
}

/// A cover for an album that has none: a soft diagonal sweep through the
/// spectrum.
pub fn placeholder_cover() -> RgbaImage {
    const SIZE: u32 = 512;
    RgbaImage::from_fn(SIZE, SIZE, |x, y| {
        let t = (x + y) as f32 / (2 * SIZE) as f32;
        let hue = 0.62 + t * 0.55;
        let [r, g, b] = hsv(hue.fract(), 0.55, 0.35 + 0.35 * (1.0 - t));
        image::Rgba([r, g, b, 255])
    })
}

fn hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

#[cfg(test)]
mod tests {
    use spectra_core::{Toc, Track};

    #[test]
    fn cd_tracks_run_to_the_next_start_and_skip_data() {
        let track = |number, lba, data| Track { number, lba, data };
        // An enhanced CD: two songs, then a data track in its own session.
        let toc = Toc {
            first: 1,
            last: 3,
            tracks: vec![
                track(1, 0, false),
                track(2, 75 * 200, false),
                track(3, 75 * 500, true),
            ],
            leadout: 75 * 900,
        };
        let tracks = super::cd_tracks(&toc);
        let got: Vec<_> = tracks
            .iter()
            .map(|t| (t.title.as_str(), t.seconds))
            .collect();
        assert_eq!(got, [("Track 1", 200), ("Track 2", 300)]);
    }
}
