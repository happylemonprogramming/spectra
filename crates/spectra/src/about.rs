//! What a film or a game is: a few lines of what it is about, its genres,
//! who made it, when, how long it runs or how many can play.
//!
//! Wikidata knows the facts, and which Wikipedia article is the thing's;
//! the article's summary says what it is about. The search is by the name
//! alone, narrowed to films or to video games, and a game is told from its
//! namesakes - there are many games called Tomb Raider - by its console and
//! its year. The requests carry the name and nothing about who is asking.
//!
//! Answers are kept in `~/.cache/spectra/art`, so a disc is asked about
//! once. Finding nothing is remembered for a month; a network failure is
//! not remembered at all.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use spectra_core::GameSystem;

use crate::artwork;
use crate::filmdb::encode;
use crate::net::{self, Get};

const WIKIDATA: &str = "https://query.wikidata.org/sparql";
const SUMMARY: &str = "https://en.wikipedia.org/api/rest_v1/page/summary/";
const MISS_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Enough of a summary to say what it is, not the whole story.
const SENTENCES: usize = 4;

/// What is being asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Film,
    Game(Option<GameSystem>),
}

/// What was found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct About {
    /// What it is about, in a few sentences, from Wikipedia.
    pub summary: Option<String>,
    pub genres: Vec<String>,
    /// Who directed a film, or developed a game.
    pub makers: Vec<String>,
    pub year: Option<u16>,
    pub minutes: Option<u32>,
    /// "Single-player", "Multiplayer", "Co-op".
    pub modes: Vec<String>,
    /// Seconds since 1970 when this was found.
    #[serde(default)]
    pub at: u64,
}

impl About {
    /// Whether anything at all was found.
    pub fn is_empty(&self) -> bool {
        self.summary.is_none()
            && self.genres.is_empty()
            && self.makers.is_empty()
            && self.year.is_none()
    }
}

/// Where the answer is kept: by kind and name, so a disc in the drive and
/// its copy share it. Nothing in it can leave the cache directory.
pub fn key(kind: Kind, title: &str) -> String {
    let kind = match kind {
        Kind::Film => "film".to_string(),
        Kind::Game(system) => format!(
            "game-{}",
            system.map_or("any".into(), |s| format!("{s:?}").to_lowercase())
        ),
    };
    format!("about-{kind}-{}", normalize(title).replace(' ', "_"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// From the cache, or else from Wikidata and Wikipedia. None when nothing
/// could be asked, so the stage can try again next time. Blocks; call it
/// off the UI thread.
pub fn look_up(kind: Kind, title: &str, year: Option<u16>) -> Option<About> {
    let dir = artwork::cache_dir()?;
    let path = dir.join(format!("{}.json", key(kind, title)));
    if let Some(kept) = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<About>(&b).ok())
        .filter(|a| !a.is_empty() || now().saturating_sub(a.at) < MISS_TTL.as_secs())
    {
        return Some(kept);
    }
    let found = ask(kind, title, year)?;
    if let Ok(json) = serde_json::to_vec(&found) {
        // Whole or not at all: written aside, then put in place.
        let part = path.with_extension("part");
        if std::fs::write(&part, json).is_ok() {
            let _ = std::fs::rename(&part, &path);
        }
    }
    Some(found)
}

fn ask(kind: Kind, title: &str, year: Option<u16>) -> Option<About> {
    let url = format!(
        "{WIKIDATA}?format=json&query={}",
        encode(&sparql(kind, title))
    );
    let Get::Found(body) = net::get(&url, net::SPECTRA) else {
        return None;
    };
    let answer: Value = serde_json::from_slice(&body).ok()?;
    let mut found = About {
        at: now(),
        ..About::default()
    };
    let Some(chosen) = pick(title, kind, year, &candidates(&answer)) else {
        return Some(found);
    };
    if let Some(article) = &chosen.article {
        match net::get(&format!("{SUMMARY}{article}"), net::SPECTRA) {
            Get::Found(body) => {
                let summary: Value = serde_json::from_slice(&body).unwrap_or_default();
                found.summary = summary["extract"]
                    .as_str()
                    .map(|s| shorten(s, SENTENCES))
                    .filter(|s| !s.is_empty());
            }
            Get::Missing => {}
            Get::Failed => return None,
        }
    }
    found.genres = chosen.genres.iter().take(3).map(|g| genre(g)).collect();
    found.genres.dedup();
    found.makers = chosen.makers.iter().take(2).cloned().collect();
    found.year = chosen.year;
    found.minutes = chosen.seconds.map(|s| s.div_ceil(60));
    found.modes = chosen.modes.iter().filter_map(|m| mode(m)).collect();
    Some(found)
}

/// Wikidata's item for a console, as a game lists the ones it came out on.
fn platform(system: GameSystem) -> Option<&'static str> {
    Some(match system {
        GameSystem::Ps1 => "Q10677",
        GameSystem::Ps2 => "Q10680",
        GameSystem::Ps3 => "Q10683",
        GameSystem::Saturn => "Q200912",
        GameSystem::GameCube => "Q182172",
        GameSystem::Wii => "Q8079",
        GameSystem::Xbox => "Q132020",
        GameSystem::Xbox360 => "Q48263",
        _ => return None,
    })
}

fn sparql(kind: Kind, title: &str) -> String {
    let search: String = title
        .chars()
        .map(|c| if matches!(c, '"' | '\\') { ' ' } else { c })
        .collect();
    // Films by their TMDB ID, as filmdb finds them; games as video games.
    let only = match kind {
        Kind::Film => "haswbstatement:P4947",
        Kind::Game(_) => "haswbstatement:P31=Q7889",
    };
    format!(
        r#"SELECT ?item ?itemLabel ?ord ?article (MIN(?date) AS ?first) (MAX(?secs) AS ?seconds)
  (GROUP_CONCAT(DISTINCT STR(?platform); separator="|") AS ?platforms)
  (GROUP_CONCAT(DISTINCT ?genreLabel; separator="|") AS ?genres)
  (GROUP_CONCAT(DISTINCT ?makerLabel; separator="|") AS ?makers)
  (GROUP_CONCAT(DISTINCT ?modeLabel; separator="|") AS ?modes) WHERE {{
  SERVICE wikibase:mwapi {{
    bd:serviceParam wikibase:endpoint "www.wikidata.org";
                    wikibase:api "Search";
                    mwapi:srsearch "{search} {only}";
                    mwapi:srlimit "20".
    ?item wikibase:apiOutputItem mwapi:title.
    ?ord wikibase:apiOrdinal true.
  }}
  OPTIONAL {{ ?item wdt:P400 ?platform }}
  OPTIONAL {{ ?item wdt:P577 ?date }}
  OPTIONAL {{ ?item p:P2047/psn:P2047/wikibase:quantityAmount ?secs }}
  OPTIONAL {{ ?item wdt:P136 ?genre . ?genre rdfs:label ?genreLabel FILTER(LANG(?genreLabel) = "en") }}
  OPTIONAL {{ ?item wdt:P178|wdt:P57 ?maker . ?maker rdfs:label ?makerLabel FILTER(LANG(?makerLabel) = "en") }}
  OPTIONAL {{ ?item wdt:P404 ?mode . ?mode rdfs:label ?modeLabel FILTER(LANG(?modeLabel) = "en") }}
  OPTIONAL {{ ?article schema:about ?item; schema:isPartOf <https://en.wikipedia.org/> }}
  SERVICE wikibase:label {{ bd:serviceParam wikibase:language "en,mul". }}
}} GROUP BY ?item ?itemLabel ?ord ?article"#
    )
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Candidate {
    title: String,
    /// Where the search put it.
    rank: u32,
    year: Option<u16>,
    seconds: Option<u32>,
    /// Wikidata's items for the consoles it came out on.
    platforms: Vec<String>,
    genres: Vec<String>,
    makers: Vec<String>,
    modes: Vec<String>,
    /// The English Wikipedia article's name, as it goes in a URL.
    article: Option<String>,
}

fn candidates(answer: &Value) -> Vec<Candidate> {
    let rows = answer["results"]["bindings"].as_array();
    rows.into_iter()
        .flatten()
        .filter_map(|row| {
            let value = |name: &str| {
                row[name]["value"]
                    .as_str()
                    .map(str::to_string)
                    .filter(|v| !v.is_empty())
            };
            let list = |name: &str| -> Vec<String> {
                value(name)
                    .map(|v| v.split('|').map(str::to_string).collect())
                    .unwrap_or_default()
            };
            Some(Candidate {
                title: value("itemLabel")?,
                rank: value("ord")?.parse().ok()?,
                year: value("first").and_then(|d| d.trim_start_matches('+').get(..4)?.parse().ok()),
                seconds: value("seconds")
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|s| s.is_finite() && *s > 0.0)
                    .map(|s| s.round() as u32),
                platforms: list("platforms")
                    .into_iter()
                    .filter_map(|p: String| p.rsplit('/').next().map(str::to_string))
                    .collect(),
                genres: list("genres"),
                makers: list("makers"),
                modes: list("modes"),
                article: value("article")
                    .and_then(|a| a.split_once("/wiki/").map(|(_, name)| name.to_string()))
                    .filter(|name| !name.contains(['/', '?', '#'])),
            })
        })
        .collect()
}

/// "Midnight Club - Street Racing" and "Midnight Club: Street Racing" alike
/// -> "midnight club street racing".
fn normalize(title: &str) -> String {
    let lower = title.to_lowercase().replace('&', " and ");
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The one the disc is: the same name before a longer one, then on the
/// disc's console, then from the year the disc says, then as the search
/// ranked them. A name that does not fit at all is nothing found.
fn pick(title: &str, kind: Kind, year: Option<u16>, candidates: &[Candidate]) -> Option<Candidate> {
    let want = normalize(title);
    let console = match kind {
        Kind::Game(Some(system)) => platform(system),
        _ => None,
    };
    candidates
        .iter()
        .filter_map(|c| {
            let name = normalize(&c.title);
            let named = if name == want {
                2
            } else if name.starts_with(&format!("{want} ")) || want.starts_with(&format!("{name} "))
            {
                1
            } else {
                return None;
            };
            let on = console.is_some_and(|q| c.platforms.iter().any(|p| p == q));
            let when = year.is_some_and(|y| c.year == Some(y));
            Some(((named, on, when, std::cmp::Reverse(c.rank)), c))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, c)| c.clone())
}

/// "action-adventure game" -> "Action-adventure"; "comedy film" -> "Comedy".
fn genre(label: &str) -> String {
    let bare = [" video game", " game", " film"]
        .iter()
        .find_map(|end| label.strip_suffix(end))
        .unwrap_or(label)
        .trim();
    let mut chars = bare.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn mode(label: &str) -> Option<String> {
    Some(
        match label {
            "single-player video game" => "Single-player",
            "multiplayer video game" => "Multiplayer",
            "co-op mode" | "cooperative video game" => "Co-op",
            _ => return None,
        }
        .to_string(),
    )
}

/// The first few sentences.
fn shorten(text: &str, sentences: usize) -> String {
    match text.match_indices(". ").nth(sentences.saturating_sub(1)) {
        Some((i, _)) => text[..=i].trim().to_string(),
        None => text.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(title: &str, rank: u32, year: u16, platforms: &[&str]) -> Candidate {
        Candidate {
            title: title.into(),
            rank,
            year: Some(year),
            platforms: platforms.iter().map(|p| p.to_string()).collect(),
            ..Candidate::default()
        }
    }

    #[test]
    fn a_game_is_told_from_its_namesakes_by_its_console_and_year() {
        let found = [
            game("Tomb Raider", 0, 1996, &["Q200912", "Q10677"]),
            game("Tomb Raider", 1, 2013, &["Q48263"]),
            game("Tomb Raider II", 4, 1997, &["Q10677"]),
            game("Tomb Raider", 6, 2000, &["Q203992"]),
        ];
        let ps1 = Kind::Game(Some(GameSystem::Ps1));
        assert_eq!(
            pick("Tomb Raider", ps1, None, &found).unwrap().year,
            Some(1996)
        );
        let x360 = Kind::Game(Some(GameSystem::Xbox360));
        assert_eq!(
            pick("Tomb Raider", x360, None, &found).unwrap().year,
            Some(2013)
        );
        assert_eq!(
            pick("Tomb Raider", Kind::Game(None), Some(2000), &found)
                .unwrap()
                .year,
            Some(2000)
        );
        assert!(pick("Crash Bandicoot", ps1, None, &found).is_none());
    }

    #[test]
    fn names_match_whatever_their_punctuation() {
        let found = [game("Midnight Club: Street Racing", 0, 2000, &["Q10680"])];
        let ps2 = Kind::Game(Some(GameSystem::Ps2));
        assert!(pick("Midnight Club - Street Racing", ps2, None, &found).is_some());
    }

    #[test]
    fn genres_and_modes_are_said_plainly() {
        assert_eq!(genre("action-adventure game"), "Action-adventure");
        assert_eq!(genre("racing video game"), "Racing");
        assert_eq!(genre("comedy film"), "Comedy");
        assert_eq!(
            mode("single-player video game").as_deref(),
            Some("Single-player")
        );
        assert_eq!(mode("virtual reality"), None);
    }

    #[test]
    fn a_summary_is_cut_to_its_first_sentences() {
        let text = "One. Two. Three. Four. Five.";
        assert_eq!(shorten(text, 4), "One. Two. Three. Four.");
        assert_eq!(shorten("Only one.", 4), "Only one.");
    }

    #[test]
    fn keys_stay_in_the_cache() {
        let k = key(Kind::Film, "../Dude, Where's My Car?");
        assert_eq!(k, "about-film-dude_where_s_my_car");
        assert_eq!(
            key(Kind::Game(Some(GameSystem::Ps1)), "Tomb Raider"),
            "about-game-ps1-tomb_raider"
        );
    }
}
