//! What film a DVD is, and its pictures. Ported from Rainbow Player's
//! `src/lib/dvd/art.ts` and `src/lib/dvdLabel.ts`.
//!
//! A DVD carries nothing a catalogue indexes it by, only the volume label its
//! authoring tool was given: "THE_BIG_LEBOWSKI_WS", or just "DUDE". The label,
//! tidied into a title, is searched for on Wikidata, which knows films' TMDB
//! and IMDb IDs and running times; fanart.tv, keyed by those, has scans of
//! the printed side of the disc, already cut out round, and posters.
//!
//! Picking the film is the delicate part. A label names a film exactly, or
//! is the start of a longer name, and the feature's running time tells apart
//! films that share either. One change from Rainbow Player: a film whose
//! length fits beats one whose name fits better but whose length does not -
//! "DUDE" is two films called exactly that, at 97 minutes and of unknown
//! length, and *Dude, Where's My Car?* at 80, on an 83-minute disc.
//!
//! Answers are kept in `~/.cache/spectra/art`, so a disc is asked about once.
//! Finding nothing is remembered for a month; a network failure is not
//! remembered at all.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use spectra_core::library::Names;

use crate::artwork::{self, Pictures};
use crate::net::{self, Get};

const WIKIDATA: &str = "https://query.wikidata.org/sparql";
const FANART: &str = "https://webservice.fanart.tv/v3/movies/";
/// fanart.tv's project key, which identifies an app and is meant to ship in
/// it. This one is Rainbow Player's.
const FANART_KEY: &str = "cdd106b8c5182ca58670822dc47e9398";

/// How far a film's listed running time may stray from the disc's feature:
/// cuts differ, and PAL runs 4% fast.
const RUNTIME_SLACK: u32 = 10 * 60;
const MISS_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// What a disc was found to be.
#[derive(Debug, Clone, Default)]
pub struct Found {
    pub title: Option<String>,
    pub year: Option<u16>,
    pub pictures: Pictures,
}

/// "THE_BIG_LEBOWSKI_WS" -> "The Big Lebowski", for the screen and for the
/// search.
pub fn tidy(label: &str) -> String {
    // Authoring leftovers: widescreen and fullscreen, TV systems, and which
    // disc or side of a set this is.
    let leftover = |w: &str| {
        let w = w.to_ascii_uppercase();
        let number = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
        matches!(w.as_str(), "WS" | "FS" | "16X9" | "4X3" | "NTSC" | "PAL")
            || w.strip_prefix("DISC").is_some_and(number)
            || w.strip_prefix('D').is_some_and(number)
            || w.strip_prefix("SIDE")
                .is_some_and(|n| number(n) || n == "A" || n == "B")
    };
    let words: Vec<String> = label
        .split(['_', '.', ' '])
        .filter(|w| !w.is_empty() && !leftover(w))
        .map(|w| {
            let lower = w.to_lowercase();
            let mut chars = lower.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        })
        .collect();
    if words.is_empty() {
        "DVD".into()
    } else {
        words.join(" ")
    }
}

/// "The Big Lebowski!" and "BIG_LEBOWSKI" alike -> "big lebowski".
fn normalize(title: &str) -> String {
    let lower = title.to_lowercase().replace('&', " and ");
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let words = match words.first() {
        Some(&("the" | "a" | "an")) if words.len() > 1 => &words[1..],
        _ => &words[..],
    };
    words.join(" ")
}

/// 2 for the same title, 1 where the query is the start of it - a label cut
/// short, or a film with more to its name - and 0 otherwise.
fn title_match(query: &str, candidate: &str) -> u8 {
    let (q, c) = (normalize(query), normalize(candidate));
    if q.is_empty() || c.is_empty() {
        0
    } else if q == c {
        2
    } else {
        u8::from(c.starts_with(&format!("{q} ")))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    tmdb: Option<String>,
    imdb: Option<String>,
    title: String,
    year: Option<u16>,
    seconds: Option<u32>,
    /// Where the search put it.
    rank: u32,
}

/// The film a disc is. Among titles that match, one whose length fits the
/// feature comes first, however well its name matched - the best name among
/// those, then the one the search ranked highest. Not the nearest length:
/// listed lengths are rough, and *The Dude Goes West* is two seconds nearer
/// this disc than the film on it. If no length fits, the best-matching name
/// whose length is not known, as the search ranked them. A disc whose feature fits none of
/// the films that list a length, and that has no unknown-length match left,
/// is taken to be none of them.
fn pick(query: &str, candidates: &[Candidate], feature: Option<u32>) -> Option<Candidate> {
    let mut matched: Vec<(u8, &Candidate)> = candidates
        .iter()
        .map(|c| (title_match(query, &c.title), c))
        .filter(|(m, _)| *m > 0)
        .collect();
    matched.sort_by_key(|(m, c)| (std::cmp::Reverse(*m), c.rank));
    let Some(feature) = feature.filter(|&f| f > 0) else {
        return matched.first().map(|(_, c)| (*c).clone());
    };
    // Sorted by match, then rank: the first that fits is the one.
    let fit = matched
        .iter()
        .find(|(_, c)| {
            c.seconds
                .is_some_and(|s| s.abs_diff(feature) <= RUNTIME_SLACK)
        })
        .map(|(_, c)| (*c).clone());
    fit.or_else(|| {
        matched
            .iter()
            .find(|(_, c)| c.seconds.is_none())
            .map(|(_, c)| (*c).clone())
    })
}

fn sparql(title: &str) -> String {
    // A full-text search, so a label missing its "The" still finds the film;
    // only items with a TMDB movie ID, which is to say films.
    let search: String = title
        .chars()
        .map(|c| if matches!(c, '"' | '\\') { ' ' } else { c })
        .collect();
    format!(
        r#"SELECT ?item ?itemLabel ?tmdb ?imdb ?date ?seconds ?ord WHERE {{
  SERVICE wikibase:mwapi {{
    bd:serviceParam wikibase:endpoint "www.wikidata.org";
                    wikibase:api "Search";
                    mwapi:srsearch "{search} haswbstatement:P4947";
                    mwapi:srlimit "20".
    ?item wikibase:apiOutputItem mwapi:title.
    ?ord wikibase:apiOrdinal true.
  }}
  OPTIONAL {{ ?item wdt:P4947 ?tmdb }}
  OPTIONAL {{ ?item wdt:P345 ?imdb }}
  OPTIONAL {{ ?item wdt:P577 ?date }}
  OPTIONAL {{ ?item p:P2047/psn:P2047/wikibase:quantityAmount ?seconds }}
  SERVICE wikibase:label {{ bd:serviceParam wikibase:language "en,mul". }}
}}"#
    )
}

/// One candidate per film, out of a row per combination of its values.
fn candidates(answer: &Value) -> Vec<Candidate> {
    let mut films: Vec<(String, Candidate)> = Vec::new();
    let rows = answer["results"]["bindings"].as_array();
    for row in rows.into_iter().flatten() {
        let value = |name: &str| row[name]["value"].as_str().map(str::to_string);
        let Some(item) = value("item") else { continue };
        let year = value("date").and_then(|d| d.trim_start_matches('+').get(..4)?.parse().ok());
        let seconds = value("seconds")
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(|s| s.round() as u32);
        match films.iter_mut().find(|(i, _)| *i == item) {
            Some((_, film)) => {
                // A film released in several countries lists every date;
                // the first is the year it is known by.
                if let Some(year) = year
                    && film.year.is_none_or(|y| year < y)
                {
                    film.year = Some(year);
                }
                film.tmdb = film.tmdb.take().or(value("tmdb"));
                film.imdb = film.imdb.take().or(value("imdb"));
                film.seconds = film.seconds.or(seconds);
            }
            None => films.push((
                item,
                Candidate {
                    tmdb: value("tmdb"),
                    imdb: value("imdb"),
                    title: value("itemLabel").unwrap_or_default(),
                    year,
                    seconds,
                    rank: value("ord").and_then(|o| o.parse().ok()).unwrap_or(99),
                },
            )),
        }
    }
    films.into_iter().map(|(_, film)| film).collect()
}

/// The best of fanart.tv's images of one kind: a DVD's face over a
/// Blu-ray's, then one with no words or English ones, then the best liked.
fn best_image(images: &Value, prefer_dvd: bool) -> Option<String> {
    let rank = |i: &Value| {
        let lang = i["lang"].as_str().unwrap_or("");
        let dvd = prefer_dvd && i["disc_type"].as_str() == Some("dvd");
        let readable = matches!(lang, "" | "en" | "00");
        let likes: u32 = i["likes"]
            .as_str()
            .and_then(|l| l.parse().ok())
            .unwrap_or(0);
        (dvd, readable, likes)
    };
    images
        .as_array()?
        .iter()
        .filter(|i| i["url"].as_str().is_some_and(|u| u.starts_with("http")))
        .max_by_key(|i| rank(i))
        .and_then(|i| i["url"].as_str())
        .map(|u| u.replacen("http:", "https:", 1))
}

/// What is remembered about a disc.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Match {
    title: Option<String>,
    year: Option<u16>,
    /// The film's TMDB or IMDb ID, which names its pictures in the cache.
    id: Option<String>,
    disc: Option<String>,
    poster: Option<String>,
    /// Seconds since 1970 when this was found.
    at: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// "DUDE", 4981 -> "film-DUDE-4981.json": nothing that leaves the cache.
fn match_name(label: &str, feature: Option<u32>) -> String {
    let plain: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("film-{plain}-{}.json", feature.unwrap_or(0))
}

/// Ask Wikidata and fanart.tv. None if either could not be reached, so
/// that nothing is remembered.
fn ask(label: &str, feature: Option<u32>) -> Option<Match> {
    let title = tidy(label);
    let mut found = Match {
        at: now(),
        ..Match::default()
    };
    // A label tidied down to nothing names no film.
    if title == "DVD" {
        return Some(found);
    }
    let url = format!("{WIKIDATA}?format=json&query={}", encode(&sparql(&title)));
    let Get::Found(body) = net::get(&url, net::SPECTRA) else {
        return None;
    };
    let answer: Value = serde_json::from_slice(&body).ok()?;
    let Some(film) = pick(&title, &candidates(&answer), feature) else {
        return Some(found);
    };
    found.title = Some(film.title);
    found.year = film.year;
    let Some(id) = film
        .tmdb
        .or(film.imdb)
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()))
    else {
        return Some(found);
    };
    match net::get(&format!("{FANART}{id}?api_key={FANART_KEY}"), net::SPECTRA) {
        Get::Found(body) => {
            let images: Value = serde_json::from_slice(&body).unwrap_or_default();
            found.disc = best_image(&images["moviedisc"], true);
            found.poster = best_image(&images["movieposter"], false);
        }
        // fanart.tv has nothing for this film.
        Get::Missing => {}
        Get::Failed => return None,
    }
    found.id = Some(id);
    Some(found)
}

/// What film this DVD is and its pictures, from the cache or the network.
/// Blocks; call it off the UI thread.
pub fn look_up(label: &str, feature: Option<u32>) -> Found {
    let Some(dir) = artwork::cache_dir() else {
        return Found::default();
    };
    let path = dir.join(match_name(label, feature));
    let cached = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Match>(&b).ok())
        .filter(|m| m.disc.is_some() || now().saturating_sub(m.at) < MISS_TTL.as_secs());
    let found = match cached {
        Some(found) => found,
        None => {
            let Some(found) = ask(label, feature) else {
                return Found::default();
            };
            if let Ok(json) = serde_json::to_vec(&found) {
                let _ = std::fs::write(&path, json);
            }
            found
        }
    };
    let picture = |kind: &str, url: &Option<String>| {
        let (id, url) = (found.id.as_ref()?, url.as_ref()?);
        let name = format!("film-{kind}-{id}.{}", url.rsplit('.').next()?);
        artwork::fetch(&dir, &name, url).and_then(|path| artwork::decode(&path))
    };
    Found {
        title: found.title.clone(),
        year: found.year,
        pictures: Pictures {
            cover: picture("poster", &found.poster),
            face: picture("disc", &found.disc),
        },
    }
}

/// What the lookup found, from the cache alone, without asking anyone.
fn cached(label: &str, feature: Option<u32>) -> Option<Match> {
    let path = artwork::cache_dir()?.join(match_name(label, feature));
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The film's name, for a copy of the disc.
pub fn names(label: &str, feature: Option<u32>) -> Option<Names> {
    let found = cached(label, feature)?;
    Some(Names {
        title: found.title?,
        artist: String::new(),
        year: found.year.map(|y| y.to_string()),
        tracks: Vec::new(),
    })
}

/// Keep the film's pictures with its copy in the library, so the copy keeps
/// its face if the cache is cleared.
pub fn store(dir: &std::path::Path, label: &str, feature: Option<u32>) {
    let (Some(cache), Some(found)) = (artwork::cache_dir(), cached(label, feature)) else {
        return;
    };
    let Some(id) = &found.id else { return };
    for (kind, url, name) in [
        ("poster", &found.poster, artwork::COVER),
        ("disc", &found.disc, artwork::FACE),
    ] {
        if let Some(ext) = url.as_ref().and_then(|u| u.rsplit('.').next()) {
            let _ = std::fs::copy(
                cache.join(format!("film-{kind}-{id}.{ext}")),
                dir.join(name),
            );
        }
    }
}

/// Percent-encoding for a query string value.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn film(title: &str, seconds: Option<u32>, rank: u32) -> Candidate {
        Candidate {
            tmdb: Some(rank.to_string()),
            imdb: None,
            title: title.into(),
            year: None,
            seconds,
            rank,
        }
    }

    #[test]
    fn labels_tidy_into_titles() {
        assert_eq!(tidy("THE_BIG_LEBOWSKI_WS"), "The Big Lebowski");
        assert_eq!(tidy("DUDE"), "Dude");
        assert_eq!(tidy("MATRIX_D1_16X9"), "Matrix");
        assert_eq!(tidy("ALIEN_DISC2"), "Alien");
        assert_eq!(tidy("D2"), "DVD");
        assert_eq!(tidy("DRIVE"), "Drive");
        assert_eq!(tidy("DA_VINCI_CODE"), "Da Vinci Code");
    }

    #[test]
    fn names_match_whole_or_as_the_start() {
        assert_eq!(title_match("Big Lebowski", "The Big Lebowski"), 2);
        assert_eq!(title_match("Dude", "Dude, Where's My Car?"), 1);
        assert_eq!(title_match("Dude", "Surfer, Dude"), 0);
        assert_eq!(title_match("Fast & Furious", "Fast and Furious"), 2);
    }

    #[test]
    fn a_fitting_length_beats_a_closer_name() {
        // What Wikidata says for "Dude", with this disc's 83 minutes.
        let found = [
            film("Dude", Some(5820), 0),
            film("Dude, Where's My Car?", Some(4800), 1),
            film("Dude", None, 2),
            film("Surfer, Dude", Some(4920), 5),
            film("The Dude Goes West", Some(5160), 10),
        ];
        let picked = pick("Dude", &found, Some(4981)).unwrap();
        assert_eq!(picked.title, "Dude, Where's My Car?");
        // Without the disc's length, the name is all there is.
        assert_eq!(pick("Dude", &found, None).unwrap().rank, 0);
    }

    #[test]
    fn a_length_that_fits_nothing_falls_back_to_an_unknown_length() {
        let found = [film("Dude", Some(5820), 0), film("Dude", None, 2)];
        assert_eq!(pick("Dude", &found, Some(3000)).unwrap().rank, 2);
        let timed = [film("Dude", Some(5820), 0)];
        assert_eq!(pick("Dude", &timed, Some(3000)), None);
    }

    #[test]
    fn a_dvd_face_with_readable_words_first() {
        let images: Value = serde_json::json!([
            {"url": "https://a/bluray.png", "disc_type": "bluray", "lang": "en", "likes": "5"},
            {"url": "https://a/dvd-fr.png", "disc_type": "dvd", "lang": "fr", "likes": "9"},
            {"url": "https://a/dvd-en.png", "disc_type": "dvd", "lang": "en", "likes": "3"},
        ]);
        assert_eq!(
            best_image(&images, true).as_deref(),
            Some("https://a/dvd-en.png")
        );
    }

    #[test]
    fn cache_names_are_plain() {
        assert_eq!(match_name("DUDE", Some(4981)), "film-DUDE-4981.json");
        assert_eq!(match_name("../X", None), "film-___X-0.json");
    }
}
