//! Names and a cover for an audio CD, from MusicBrainz and the Cover Art
//! Archive.
//!
//! An exact disc ID lookup first, then a fuzzy one by table of contents,
//! which finds pressings nobody has submitted this exact disc ID for. A disc
//! ID is a pure function of the TOC, so the answer is kept in
//! `~/.cache/spectra/musicbrainz` and a disc is only ever looked up once.
//! Ported from Rainbow Player's `src/lib/cd/musicbrainz.ts`.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use spectra_core::discid::MusicBrainzId;
use spectra_core::library::{Names, TrackName};

use crate::artwork;
use crate::net::{self, Get};

const MB: &str = "https://musicbrainz.org/ws/2";
const CAA: &str = "https://coverartarchive.org";

/// What MusicBrainz says is on the disc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub id: String,
    pub title: String,
    pub artist: String,
    /// "1997-05-21", or as much of it as is known.
    pub date: Option<String>,
    pub tracks: Vec<TrackName>,
}

impl Release {
    pub fn names(&self) -> Names {
        Names {
            title: self.title.clone(),
            artist: self.artist.clone(),
            year: self.date.clone(),
            tracks: self.tracks.clone(),
        }
    }

    /// The cover's file in the picture cache, fetched or not.
    fn cover_name(&self) -> String {
        format!("cover-mb-{}.jpg", self.id)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Found {
    pub release: Option<Release>,
    pub cover: Option<RgbaImage>,
}

/// Look the disc up, from the cache if it has been seen before. Blocks; call
/// it off the UI thread.
pub fn look_up(id: &MusicBrainzId) -> Found {
    let Some(dir) = cache_dir() else {
        return Found::default();
    };
    let Some(release) = release(&dir, id) else {
        return Found::default();
    };
    let cover = artwork::cache_dir()
        .and_then(|art| {
            artwork::fetch(
                &art,
                &release.cover_name(),
                &format!("{CAA}/release/{}/front-500", release.id),
            )
        })
        .and_then(|path| artwork::decode(&path));
    Found {
        release: Some(release),
        cover,
    }
}

/// The names alone, without the cover. Blocks.
pub fn names(id: &MusicBrainzId) -> Option<Release> {
    release(&cache_dir()?, id)
}

/// Keep an album's cover with its copy, if the cache has it.
pub fn store(dir: &Path, release: &Release) {
    if let Some(art) = artwork::cache_dir() {
        let _ = std::fs::copy(art.join(release.cover_name()), dir.join(artwork::COVER));
    }
}

fn release(dir: &Path, id: &MusicBrainzId) -> Option<Release> {
    if !is_disc_id(&id.disc_id) {
        return None;
    }
    let cached = dir.join(format!("{}.json", id.disc_id));
    if let Ok(bytes) = std::fs::read(&cached)
        && let Ok(release) = serde_json::from_slice(&bytes)
    {
        return Some(release);
    }
    let inc = "artist-credits+recordings";
    let exact = format!("{MB}/discid/{}?fmt=json&inc={inc}", id.disc_id);
    let fuzzy = format!(
        "{MB}/discid/-?toc={}&fmt=json&inc={inc}&media-format=all",
        id.toc.replace(' ', "+")
    );
    let release = [exact, fuzzy]
        .iter()
        .find_map(|url| match net::get(url, net::SPECTRA) {
            Get::Found(body) => {
                let lookup: Lookup = serde_json::from_slice(&body).ok()?;
                pick(lookup, &id.disc_id)
            }
            Get::Missing | Get::Failed => None,
        })?;
    // MusicBrainz asks for no more than one request a second; a disc makes
    // at most two, and only once.
    let _ = std::fs::write(&cached, serde_json::to_vec(&release).ok()?);
    Some(release)
}

/// A disc ID is base64 with `.`, `_` and `-`: nothing that could leave a URL
/// path or a directory.
fn is_disc_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

#[derive(Deserialize)]
struct Lookup {
    #[serde(default)]
    releases: Vec<MbRelease>,
}

#[derive(Deserialize)]
struct MbRelease {
    id: String,
    title: String,
    date: Option<String>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<MbCredit>,
    #[serde(default)]
    media: Vec<MbMedium>,
}

#[derive(Deserialize)]
struct MbMedium {
    format: Option<String>,
    #[serde(default)]
    discs: Vec<MbDisc>,
    #[serde(default)]
    tracks: Vec<MbTrack>,
}

#[derive(Deserialize)]
struct MbDisc {
    id: String,
}

#[derive(Deserialize)]
struct MbTrack {
    title: String,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<MbCredit>,
}

#[derive(Deserialize)]
struct MbCredit {
    name: String,
    #[serde(default)]
    joinphrase: String,
}

fn credit(credits: &[MbCredit]) -> String {
    credits
        .iter()
        .map(|c| format!("{}{}", c.name, c.joinphrase))
        .collect()
}

/// The first release, and on it the medium this disc is: the one listing
/// its disc ID, or failing that the first CD.
fn pick(lookup: Lookup, disc_id: &str) -> Option<Release> {
    let release = lookup.releases.into_iter().next()?;
    let artist = credit(&release.artist_credit);
    let medium = release
        .media
        .iter()
        .position(|m| m.discs.iter().any(|d| d.id == disc_id))
        .or_else(|| {
            release
                .media
                .iter()
                .position(|m| m.format.as_deref() == Some("CD"))
        })
        .unwrap_or(0);
    let tracks = release
        .media
        .into_iter()
        .nth(medium)
        .map(|m| m.tracks)
        .unwrap_or_default()
        .into_iter()
        .map(|t| {
            let by = credit(&t.artist_credit);
            TrackName {
                title: t.title,
                artist: (!by.is_empty() && by != artist).then_some(by),
            }
        })
        .collect();
    Some(Release {
        id: release.id,
        title: release.title,
        artist: if artist.is_empty() {
            "Unknown artist".into()
        } else {
            artist
        },
        date: release.date.filter(|d| !d.is_empty()),
        tracks,
    })
}

fn cache_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))?
        .join("spectra/musicbrainz");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_medium_with_this_disc() {
        let json = r#"{"releases":[{"id":"r1","title":"Box","date":"1999",
            "artist-credit":[{"name":"A","joinphrase":" & "},{"name":"B"}],
            "media":[
              {"format":"CD","discs":[{"id":"other"}],"tracks":[{"title":"One"}]},
              {"format":"CD","discs":[{"id":"mine"}],"tracks":[
                {"title":"Two","artist-credit":[{"name":"A"},{"name":"B"}]},
                {"title":"Guest","artist-credit":[{"name":"C"}]}]}
            ]}]}"#;
        let release = pick(serde_json::from_str(json).unwrap(), "mine").unwrap();
        assert_eq!(release.artist, "A & B");
        assert_eq!(release.tracks.len(), 2);
        assert_eq!(release.tracks[0].title, "Two");
        // "AB" differs from "A & B", so a credit spelled differently shows.
        assert_eq!(release.tracks[1].artist.as_deref(), Some("C"));
    }

    #[test]
    fn disc_ids_are_plain() {
        assert!(is_disc_id("Pz1GkG9FBzjSGqgRz2EMHCW1qT4-"));
        assert!(!is_disc_id("../x"));
        assert!(!is_disc_id(""));
    }
}
