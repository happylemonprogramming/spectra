//! The command line, for people and agents alike. Opening the window - no
//! command, or an album file - is left to `main`.
//!
//! `library` reads the library itself. Everything else is sent to the
//! running window (`control`), which is started first if a command needs it.

use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;
use spectra_core::library::{self, Entry};

use crate::control::{self, Reply, Request, Status, Target, TrackStatus};

const USAGE: &str = "\
Spectra: put a disc in, and it plays.

Usage:
  spectra [ALBUM.json] [--play] [--reduced-motion]
                          Open the window here, watching the drive
                          (or showing an album file instead)
  spectra open [--reduced-motion]
                          Open the window in the background and return
  spectra library [--json]
                          List kept copies: music and games
  spectra play [disc | ID | TITLE] [--track N] [--json]
                          Put something on the stage and play it: the disc in
                          the drive, a kept copy by ID, or by (part of) its
                          title or artist. Nothing named: what is on the stage.
                          Opens the window first if it is not open
  spectra pause | resume | toggle | next | previous | stop [--json]
                          The transport, for what is on the stage
  spectra status [--json] What is on the stage, the drive, and what plays
  spectra quit [--json]   Close the window

Exit status: 0 done, 1 refused or failed, 2 bad usage, 3 Spectra not running.
";

const NOT_RUNNING: i32 = 3;
/// How long a window started in the background has to start listening.
const STARTUP: Duration = Duration::from_secs(20);

/// Run a command. None when the arguments are for opening the window in
/// this process.
pub fn run(args: &[String]) -> Option<i32> {
    let (command, rest) = args.split_first()?;
    let json = rest.iter().any(|a| a == "--json");
    Some(match command.as_str() {
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            0
        }
        "open" => open(rest.iter().any(|a| a == "--reduced-motion")),
        "library" => list(json),
        "play" => match play_request(rest) {
            Ok(request) => {
                if control::send(&Request::Status).is_none()
                    && let Err(e) = start(false)
                {
                    return Some(fail(&e, json));
                }
                send(&request, json)
            }
            Err(e) => {
                eprintln!("spectra: {e}");
                2
            }
        },
        "pause" => send(&Request::Pause, json),
        "resume" => send(&Request::Resume, json),
        "toggle" => send(&Request::Toggle, json),
        "next" => send(&Request::Next, json),
        "previous" | "prev" => send(&Request::Previous, json),
        "stop" => send(&Request::Stop, json),
        "status" => send(&Request::Status, json),
        "quit" => send(&Request::Quit, json),
        _ => return None,
    })
}

fn play_request(args: &[String]) -> Result<Request, String> {
    let mut track = None;
    let mut words = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => {}
            "--track" | "-t" => {
                let n = args.next().ok_or("--track needs a number")?;
                track = Some(
                    n.parse::<usize>()
                        .ok()
                        .filter(|&n| n > 0)
                        .ok_or(format!("--track {n}: tracks count from 1"))?,
                );
            }
            flag if flag.starts_with("--") => return Err(format!("play does not take {flag}")),
            word => words.push(word),
        }
    }
    let query = words.join(" ");
    let target = match query.as_str() {
        "" => Target::Stage,
        "disc" => Target::Disc,
        _ => Target::Copy(find(&query, &library::list())?.meta.id.clone()),
    };
    Ok(Request::Play { target, track })
}

/// A kept copy by its ID, its whole title, or the one copy whose title or
/// artist contains the words, ignoring case.
fn find<'a>(query: &str, entries: &'a [Entry]) -> Result<&'a Entry, String> {
    let lower = query.to_lowercase();
    if let Some(entry) = entries
        .iter()
        .find(|e| e.meta.id.eq_ignore_ascii_case(query))
        .or_else(|| {
            entries
                .iter()
                .find(|e| e.meta.title.to_lowercase() == lower)
        })
    {
        return Ok(entry);
    }
    let matches: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            e.meta.title.to_lowercase().contains(&lower)
                || e.meta
                    .artist
                    .as_ref()
                    .is_some_and(|a| a.to_lowercase().contains(&lower))
        })
        .collect();
    match matches.as_slice() {
        [entry] => Ok(entry),
        [] => Err(format!(
            "nothing in the library matches {query:?}; `spectra library` lists what is there"
        )),
        several => Err(format!(
            "{query:?} matches more than one copy; name one by ID:\n{}",
            several
                .iter()
                .map(|e| format!("  {}  {}", e.meta.id, e.meta.title))
                .collect::<Vec<_>>()
                .join("\n")
        )),
    }
}

#[derive(Serialize)]
struct Kept {
    id: String,
    kind: &'static str,
    title: String,
    artist: Option<String>,
    system: Option<&'static str>,
    year: Option<String>,
    tracks: Vec<TrackStatus>,
}

fn kept(entry: &Entry) -> Kept {
    let music = entry.is_album();
    let tracks = if music {
        entry
            .toc()
            .map(|toc| crate::album::cd_tracks(&toc))
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, track)| TrackStatus {
                number: i + 1,
                title: entry
                    .meta
                    .tracks
                    .get(i)
                    .map_or(track.title, |t| t.title.clone()),
                seconds: track.seconds,
            })
            .collect()
    } else {
        Vec::new()
    };
    Kept {
        id: entry.meta.id.clone(),
        kind: if music { "music" } else { "game" },
        title: entry.meta.title.clone(),
        artist: entry.meta.artist.clone(),
        system: entry.meta.system.map(|s| s.name()),
        year: entry.meta.year.clone(),
        tracks,
    }
}

fn list(json: bool) -> i32 {
    let entries: Vec<Kept> = library::list().iter().map(kept).collect();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).expect("the library serialises")
        );
        return 0;
    }
    if entries.is_empty() {
        println!("Nothing kept yet: put a disc in and keep a copy (C in the window).");
        return 0;
    }
    let width = entries.iter().map(|e| e.id.len()).max().unwrap_or(0);
    for e in &entries {
        let by = match (&e.artist, e.system) {
            (Some(artist), _) => format!(" - {artist}"),
            (None, Some(system)) => format!("  ({system})"),
            (None, None) => String::new(),
        };
        let year = e.year.as_ref().map(|y| format!(" {y}")).unwrap_or_default();
        let tracks = match e.tracks.len() {
            0 => String::new(),
            1 => "  1 track".into(),
            n => format!("  {n} tracks"),
        };
        println!(
            "{:width$}  {:5}  {}{by}{year}{tracks}",
            e.id, e.kind, e.title
        );
    }
    0
}

/// Send a command to the window and say how it went.
fn send(request: &Request, json: bool) -> i32 {
    let Some(reply) = control::send(request) else {
        if json {
            println!(r#"{{"ok":false,"running":false,"error":"Spectra is not running"}}"#);
        } else {
            eprintln!("spectra: Spectra is not running; `spectra open` starts it");
        }
        return NOT_RUNNING;
    };
    let mut reply = match reply {
        Ok(reply) => reply,
        Err(e) => return fail(&format!("no answer from Spectra: {e}"), json),
    };
    if starts_sound(request) && reply.ok {
        reply = confirm(reply);
    }
    if json {
        let mut value = serde_json::to_value(&reply).expect("a reply serialises");
        value["running"] = true.into();
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("a reply serialises")
        );
    } else {
        say(request, &reply);
    }
    if reply.ok { 0 } else { 1 }
}

fn starts_sound(request: &Request) -> bool {
    matches!(
        request,
        Request::Play { .. }
            | Request::Resume
            | Request::Toggle
            | Request::Next
            | Request::Previous
    )
}

/// The window answers before the sound card does, and a track that will
/// not play only says so a moment later: ask again, so "Playing" is true.
fn confirm(reply: Reply) -> Reply {
    if reply.status.as_ref().is_none_or(|s| s.playing.is_none()) {
        return reply;
    }
    std::thread::sleep(Duration::from_millis(600));
    match control::send(&Request::Status) {
        Some(Ok(now)) => match now.status {
            Some(status)
                if status.playing.is_none()
                    && status
                        .note
                        .as_deref()
                        .is_some_and(|n| n.starts_with("Couldn't play")) =>
            {
                Reply::failed(status.note.clone().unwrap_or_default(), status)
            }
            Some(status) => Reply::done(status),
            None => reply,
        },
        _ => reply,
    }
}

fn fail(error: &str, json: bool) -> i32 {
    if json {
        println!("{}", serde_json::json!({ "ok": false, "error": error }));
    } else {
        eprintln!("spectra: {error}");
    }
    1
}

fn say(request: &Request, reply: &Reply) {
    if let Some(error) = &reply.error {
        eprintln!("spectra: {error}");
        return;
    }
    let Some(status) = &reply.status else {
        return;
    };
    match request {
        Request::Quit => println!("Closed Spectra"),
        Request::Status => describe(status),
        _ => println!("{}", summary(status)),
    }
}

/// One line: what plays, or what is on the stage.
fn summary(s: &Status) -> String {
    let by = if s.artist.is_empty() {
        String::new()
    } else {
        format!(" - {}", s.artist)
    };
    if s.in_game {
        return format!("Playing {}{by} in RetroArch", s.title);
    }
    if s.in_film {
        return format!("Playing {} in VLC", s.title);
    }
    match s.playing.and_then(|n| Some((n, s.tracks.get(n - 1)?))) {
        Some((n, track)) => format!(
            "{} {n}/{}: {} ({} of {})  ·  {}{by}",
            if s.paused { "Paused" } else { "Playing" },
            s.tracks.len(),
            track.title,
            time(s.elapsed),
            time(track.seconds),
            s.title,
        ),
        None => format!("On the stage: {}{by}  ·  stopped", s.title),
    }
}

fn describe(s: &Status) {
    println!("{}", summary(s));
    let from = match (s.source.as_str(), &s.id) {
        ("copy", Some(id)) => format!("kept copy {id}"),
        ("file", _) => "album file".into(),
        _ => "the drive".into(),
    };
    println!("Kind: {}, from {from}", s.kind);
    println!("Drive: {}", s.drive);
    println!("Screen: {}", s.screen);
    if s.copying {
        println!("Keeping a copy of the disc");
    }
    if let Some(note) = &s.note {
        println!("Note: {note}");
    }
    for t in &s.tracks {
        let mark = if s.playing == Some(t.number) {
            '>'
        } else {
            ' '
        };
        println!("{mark} {:2}. {}  {}", t.number, t.title, time(t.seconds));
    }
}

fn time(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Open the window in the background, unless it is open already.
fn open(reduced_motion: bool) -> i32 {
    if control::send(&Request::Status).is_some() {
        println!("Spectra is already open");
        return 0;
    }
    match start(reduced_motion) {
        Ok(()) => {
            println!("Opened Spectra");
            0
        }
        Err(e) => fail(&e, false),
    }
}

/// Start the window as a process of its own, which outlives this one, and
/// wait until it takes commands.
fn start(reduced_motion: bool) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let log = log_path();
    let output = log
        .as_ref()
        .and_then(|path| std::fs::File::create(path).ok());
    let mut command = Command::new(exe);
    if reduced_motion {
        command.arg("--reduced-motion");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(output.map_or_else(Stdio::null, Stdio::from))
        // Its own process group, so closing the terminal does not close it.
        .process_group(0);
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start Spectra: {e}"))?;
    let see = log
        .map(|p| format!("; see {}", p.display()))
        .unwrap_or_default();
    let step = Duration::from_millis(100);
    for _ in 0..STARTUP.as_millis() / step.as_millis() {
        std::thread::sleep(step);
        if control::send(&Request::Status).is_some() {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("Spectra stopped as it started ({status}){see}"));
        }
    }
    Err(format!("Spectra did not start taking commands{see}"))
}

/// What a window started in the background says: `~/.cache/spectra/spectra.log`.
fn log_path() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".cache"))
        })?
        .join("spectra");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("spectra.log"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectra_core::library::Meta;

    fn entry(id: &str, title: &str, artist: Option<&str>) -> Entry {
        Entry {
            dir: id.into(),
            meta: Meta {
                id: id.into(),
                title: title.into(),
                system: None,
                serial: None,
                publisher: None,
                year: None,
                region: None,
                disc_art: None,
                artist: artist.map(Into::into),
                tracks: Vec::new(),
                sectors: 0,
                unreadable: 0,
                created: 0,
            },
        }
    }

    #[test]
    fn copies_are_found_by_id_title_or_words() {
        let entries = [
            entry("cd-abc", "Blue", Some("Joni Mitchell")),
            entry("cd-def", "Blue Lines", Some("Massive Attack")),
            entry("SLUS-00152", "Tomb Raider", None),
        ];
        let id = |q| find(q, &entries).map(|e| e.meta.id.as_str());
        assert_eq!(id("slus-00152"), Ok("SLUS-00152"));
        // A whole title wins over a title it is part of.
        assert_eq!(id("blue"), Ok("cd-abc"));
        assert_eq!(id("massive"), Ok("cd-def"));
        assert_eq!(id("tomb"), Ok("SLUS-00152"));
        assert!(id("bl").unwrap_err().contains("more than one"));
        assert!(id("zzz").unwrap_err().contains("nothing"));
    }

    #[test]
    fn play_reads_its_target_and_track() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            play_request(&args(&["disc", "--track", "3"])),
            Ok(Request::Play {
                target: Target::Disc,
                track: Some(3)
            })
        );
        assert_eq!(
            play_request(&args(&["--json"])),
            Ok(Request::Play {
                target: Target::Stage,
                track: None
            })
        );
        assert!(play_request(&args(&["disc", "--track", "0"])).is_err());
        assert!(play_request(&args(&["disc", "--loud"])).is_err());
    }
}
