//! What is on screen: an album, its tracks and its cover.
//!
//! Until Phase 1 wires the drive and MusicBrainz in, an album comes from a
//! small JSON file, or is a built-in placeholder with a generated cover:
//!
//! ```json
//! { "title": "…", "artist": "…", "year": 1997, "cover": "cover.jpg",
//!   "tracks": [["Airbag", 284], ["Paranoid Android", 383]] }
//! ```

use std::path::Path;

use image::RgbaImage;
use serde::Deserialize;

pub struct Album {
    pub title: String,
    pub artist: String,
    pub year: Option<u16>,
    pub tracks: Vec<Track>,
    pub cover: RgbaImage,
}

pub struct Track {
    pub title: String,
    pub seconds: u32,
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
                .map(|(title, seconds)| Track { title, seconds })
                .collect(),
            cover,
        })
    }

    pub fn placeholder() -> Self {
        let tracks = [
            ("Pit and Land", 214),
            ("Track Pitch", 187),
            ("First Order", 243),
            ("Lead-in", 96),
            ("Grating Equation", 312),
            ("Red Book", 201),
            ("Subchannel Q", 176),
            ("Lead-out", 258),
        ];
        Self {
            title: "Spectra".into(),
            artist: "No disc".into(),
            year: None,
            tracks: tracks
                .into_iter()
                .map(|(t, s)| Track {
                    title: t.into(),
                    seconds: s,
                })
                .collect(),
            cover: placeholder_cover(),
        }
    }

    pub fn total_seconds(&self) -> u32 {
        self.tracks.iter().map(|t| t.seconds).sum()
    }
}

/// A cover for an album that has none: a soft diagonal sweep through the
/// spectrum.
fn placeholder_cover() -> RgbaImage {
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
