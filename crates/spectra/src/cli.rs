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
use crate::firmware::{self, Added};
use crate::{game, windows};

const USAGE: &str = "\
Spectra: put a disc in, and it plays.

Usage:
  spectra [ALBUM.json] [--play] [--reduced-motion]
                          Open a window here, watching the drive
                          (or showing an album file instead)
  spectra open [--new] [--reduced-motion]
                          Open the window in the background and return;
                          --new opens another beside one already open
  spectra library [--json]
                          List kept copies: music, films and games
  spectra play [disc | ID | TITLE] [--track N] [--new] [--json]
                          Put something on the stage and play it: the disc in
                          the drive, a kept copy by ID, or by (part of) its
                          title or artist. Nothing named: what is on the stage.
                          Opens a window first if none is open; --new always
                          opens another for it
  spectra pause | resume | toggle | next | previous | stop [--json]
                          The transport, for what is on the stage
  spectra status [--json] What is on the stage, the drive, and what plays
  spectra windows [--json]
                          Every open window, the current one marked *
  spectra tv on | off [--size WxH@FPS] [--json]
                          Over to the TV's screen, which Sunshine streams to
                          Moonlight, with what Spectra starts, or back from
                          it. Sunshine runs this as Spectra's prep command;
                          T in the window gets it ready. on goes to the
                          window that got it ready, or opens one. The size is
                          the TV's, read from Sunshine, or 1920x1080@60
  spectra firmware [--json]
                          Each console's firmware: which Spectra has, and
                          which games need. It comes from your own console;
                          Spectra never provides it
  spectra firmware add FILE|FOLDER... [--json]
                          Add firmware, recognised by its checksum whatever
                          its name, to RetroArch's system folder. Other
                          files are left alone, and nothing is overwritten
  spectra quit [--json]   Close the window

Commands go to the current window - the one last looked at - or to the
one named with --window PID (from `spectra windows`).

Exit status: 0 done, 1 refused or failed, 2 bad usage, 3 Spectra not running.
";

const NOT_RUNNING: i32 = 3;
/// How long a window started in the background has to start listening.
const STARTUP: Duration = Duration::from_secs(20);

/// Which window, and how to answer: options any command takes.
#[derive(Debug, Default, PartialEq)]
struct Options {
    json: bool,
    window: Option<u32>,
    new: bool,
}

/// Take the options every command shares out of the arguments, wherever
/// they are, leaving the command and its own.
fn options(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut options = Options::default();
    let mut rest = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--window" | "-w" => {
                let pid = args.next().ok_or("--window needs a window's number")?;
                options.window = Some(
                    pid.parse()
                        .map_err(|_| format!("--window {pid}: `spectra windows` lists them"))?,
                );
            }
            "--new" => options.new = true,
            "--json" => {
                options.json = true;
                rest.push(arg.clone());
            }
            _ => rest.push(arg.clone()),
        }
    }
    Ok((options, rest))
}

const COMMANDS: &[&str] = &[
    "help", "--help", "-h", "open", "library", "play", "pause", "resume", "toggle", "next",
    "previous", "prev", "stop", "status", "windows", "tv", "firmware", "quit",
];

/// Run a command. None when the arguments are for opening the window in
/// this process.
pub fn run(args: &[String]) -> Option<i32> {
    let (options, args) = match options(args) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("spectra: {e}");
            return Some(2);
        }
    };
    let Some((command, rest)) = args
        .split_first()
        .filter(|(command, _)| COMMANDS.contains(&command.as_str()))
    else {
        if options.window.is_some() || options.new {
            eprintln!("spectra: --window and --new go with a command; see spectra --help");
            return Some(2);
        }
        return None;
    };
    // Piped into `head` and the like, end quietly when the reader does, as
    // command-line tools do. Only here: the window writes to callers that
    // may have hung up, and must not die of it.
    // SAFETY: restoring a signal's default, before any thread is started.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let json = options.json;
    Some(match command.as_str() {
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            0
        }
        "open" => open(&options, rest.iter().any(|a| a == "--reduced-motion")),
        "library" => list(json),
        "windows" => list_windows(json),
        "firmware" => firmware(rest, json),
        "tv" => match tv_request(rest) {
            Ok(request) => tv(&options, &request),
            Err(e) => {
                eprintln!("spectra: {e}");
                2
            }
        },
        "play" => match play_request(rest) {
            Ok(request) => {
                let window = match target(options.window) {
                    _ if options.new => start(false),
                    Some(window) => Ok(window),
                    None if options.window.is_some() => return Some(not_running(&options)),
                    None => start(false),
                };
                match window {
                    Ok(window) => send(window, &request, json),
                    Err(e) => fail(&e, json),
                }
            }
            Err(e) => {
                eprintln!("spectra: {e}");
                2
            }
        },
        command => {
            let request = match command {
                "pause" => Request::Pause,
                "resume" => Request::Resume,
                "toggle" => Request::Toggle,
                "next" => Request::Next,
                "previous" | "prev" => Request::Previous,
                "stop" => Request::Stop,
                "status" => Request::Status,
                _ => Request::Quit,
            };
            match target(options.window) {
                Some(window) => send(window, &request, json),
                None => not_running(&options),
            }
        }
    })
}

/// The window a command goes to: the one named, or the current one, or else
/// the newest.
fn target(named: Option<u32>) -> Option<u32> {
    let running = windows::running();
    match named {
        Some(window) => running.contains(&window).then_some(window),
        None => windows::current()
            .filter(|window| running.contains(window))
            .or_else(|| running.last().copied()),
    }
}

fn not_running(options: &Options) -> i32 {
    let error = match options.window {
        Some(window) => format!("no Spectra window {window} is open; `spectra windows` lists them"),
        None => "Spectra is not running; `spectra open` starts it".into(),
    };
    if options.json {
        println!(
            "{}",
            serde_json::json!({ "ok": false, "running": false, "error": error })
        );
    } else {
        eprintln!("spectra: {error}");
    }
    NOT_RUNNING
}

fn list_windows(json: bool) -> i32 {
    let statuses: Vec<Status> = windows::running()
        .into_iter()
        .filter_map(|window| control::send(window, &Request::Status)?.ok()?.status)
        .collect();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&statuses).expect("statuses serialise")
        );
        return 0;
    }
    if statuses.is_empty() {
        println!("No Spectra window is open");
        return NOT_RUNNING;
    }
    for status in &statuses {
        let mark = if status.current { '*' } else { ' ' };
        println!("{mark} {:<8} {}", status.window, summary(status));
    }
    0
}

/// `on` or `off`, and the size of the TV's screen: given, or what Sunshine
/// says the TV asked for when it runs this before a stream.
fn tv_request(args: &[String]) -> Result<Request, String> {
    let mut on = None;
    let mut size = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => {}
            "on" => on = Some(true),
            "off" => on = Some(false),
            "--size" => {
                let given = args
                    .next()
                    .ok_or("--size needs WIDTHxHEIGHT, as 1920x1080@60")?;
                size = Some(
                    crate::tv::mode(given)
                        .ok_or(format!("--size {given}: give it as 1920x1080@60"))?,
                );
            }
            other => return Err(format!("tv takes on or off, not {other}")),
        }
    }
    let on = on.ok_or("tv takes on or off")?;
    let sunshine = || {
        let var = |name| std::env::var(name).ok();
        let (w, h) = (
            var("SUNSHINE_CLIENT_WIDTH")?,
            var("SUNSHINE_CLIENT_HEIGHT")?,
        );
        let fps = var("SUNSHINE_CLIENT_FPS").unwrap_or_else(|| "60".into());
        crate::tv::mode(&format!("{w}x{h}@{fps}"))
    };
    let mode = if on { size.or_else(sunshine) } else { None };
    Ok(Request::Tv { on, mode })
}

/// On the TV: the window named, the current one, or a new one. Off: the
/// window that has the TV, wherever it is; with none, a screen left by a
/// window that went without taking it away is cleared, and that is done.
fn tv(options: &Options, request: &Request) -> i32 {
    let json = options.json;
    let window = match request {
        Request::Tv { on: false, .. } => match options.window.or_else(crate::tv::holder) {
            Some(window) => target(Some(window)),
            None => {
                crate::tv::clear_leftover();
                if json {
                    println!("{}", serde_json::json!({ "ok": true, "running": false }));
                } else {
                    println!("Spectra is not on the TV");
                }
                return 0;
            }
        },
        _ => match options.window.map_or_else(
            || waiting_for_tv().or_else(|| target(None)),
            |named| target(Some(named)),
        ) {
            _ if options.new => start(false).ok(),
            Some(window) => Some(window),
            None if options.window.is_some() => None,
            None => match start(false) {
                Ok(window) => Some(window),
                Err(e) => return fail(&e, json),
            },
        },
    };
    match window {
        Some(window) => send(window, request, json),
        None => not_running(options),
    }
}

/// The window that got the TV ready, which Moonlight opening Spectra is for.
fn waiting_for_tv() -> Option<u32> {
    windows::running().into_iter().find(|&window| {
        control::send(window, &Request::Status)
            .and_then(Result::ok)
            .and_then(|reply| reply.status)
            .is_some_and(|status| status.tv == "ready" || status.tv == "starting")
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
        kind: if music {
            "music"
        } else if entry.is_film() {
            "film"
        } else {
            "game"
        },
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

fn firmware(args: &[String], json: bool) -> i32 {
    let args: Vec<&String> = args.iter().filter(|a| *a != "--json").collect();
    let system = firmware::system_dir();
    match args.split_first() {
        None => list_firmware(&system, json),
        Some((add, paths)) if *add == "add" && !paths.is_empty() => {
            let paths: Vec<_> = paths.iter().map(std::path::PathBuf::from).collect();
            add_firmware(&paths, &system, json)
        }
        _ => {
            eprintln!("spectra: firmware, or firmware add FILE|FOLDER...");
            2
        }
    }
}

/// Whether the file a core looks for is there, and is the one it should be.
fn firmware_state(system: &std::path::Path, f: &firmware::Firmware) -> &'static str {
    let path = system.join(f.file);
    if !path.is_file() {
        "missing"
    } else if firmware::recognise(&path).is_some_and(|there| there.file == f.file) {
        "have"
    } else {
        "different"
    }
}

fn list_firmware(system: &std::path::Path, json: bool) -> i32 {
    #[derive(Serialize)]
    struct Known {
        #[serde(flatten)]
        firmware: &'static firmware::Firmware,
        needed: bool,
        state: &'static str,
    }
    let known: Vec<Known> = firmware::KNOWN
        .iter()
        .map(|f| Known {
            firmware: f,
            needed: game::needs_firmware(f.system),
            state: firmware_state(system, f),
        })
        .collect();
    if json {
        let list = serde_json::json!({ "folder": system, "firmware": known });
        println!(
            "{}",
            serde_json::to_string_pretty(&list).expect("firmware serialises")
        );
        return 0;
    }
    println!("In {}, from your own consoles:", system.display());
    let width = known
        .iter()
        .map(|k| k.firmware.file.len())
        .max()
        .unwrap_or(0);
    let mut last = None;
    for k in &known {
        let f = k.firmware;
        if last != Some(f.system) {
            let need = if k.needed {
                "games need one of these"
            } else {
                "optional: more games play with one"
            };
            println!("\n{}, {need}", f.system.name());
            last = Some(f.system);
        }
        let state = match k.state {
            "have" => "  have it",
            "different" => "  a different file has this name",
            _ => "",
        };
        println!("  {:width$}  {}{state}", f.file, f.what);
    }
    0
}

fn add_firmware(paths: &[std::path::PathBuf], system: &std::path::Path, json: bool) -> i32 {
    let added = firmware::add(paths, system);
    let ok = added
        .iter()
        .any(|(_, a)| matches!(a, Added::Copied(_) | Added::AlreadyThere(_)));
    if json {
        let list: Vec<_> = added
            .iter()
            .map(|(path, a)| {
                let (result, f) = match a {
                    Added::Copied(f) => ("added", Some(f)),
                    Added::AlreadyThere(f) => ("already there", Some(f)),
                    Added::Taken(f) => ("a different file has its name", Some(f)),
                    Added::Unknown => ("not firmware Spectra knows", None),
                    Added::Failed(e) => (e.as_str(), None),
                };
                serde_json::json!({ "file": path, "result": result, "firmware": f })
            })
            .collect();
        let reply = serde_json::json!({ "ok": ok, "folder": system, "files": list });
        println!(
            "{}",
            serde_json::to_string_pretty(&reply).expect("firmware serialises")
        );
        return if ok { 0 } else { 1 };
    }
    let others = added
        .iter()
        .filter(|(_, a)| matches!(a, Added::Unknown))
        .count();
    for (path, a) in added.iter().filter(|(_, a)| !matches!(a, Added::Unknown)) {
        let what = |f: &firmware::Firmware| format!("{} firmware ({})", f.system.name(), f.what);
        let line = match a {
            Added::Copied(f) => format!("{}, added as {}", what(f), f.file),
            Added::AlreadyThere(f) => format!("{}, already there", what(f)),
            Added::Taken(f) => format!(
                "{}, but a different {} is there already; left as it is",
                what(f),
                f.file
            ),
            Added::Unknown => continue,
            Added::Failed(e) => format!("could not copy: {e}"),
        };
        println!("{}: {line}", path.display());
    }
    match others {
        0 if added.is_empty() => eprintln!("spectra: no files there"),
        0 => {}
        1 if added.len() == 1 => eprintln!("spectra: not firmware Spectra knows"),
        n => println!("{n} other files are not firmware Spectra knows; left alone"),
    }
    if ok { 0 } else { 1 }
}

/// Send a command to a window and say how it went.
fn send(window: u32, request: &Request, json: bool) -> i32 {
    let Some(reply) = control::send(window, request) else {
        return not_running(&Options {
            json,
            window: Some(window),
            new: false,
        });
    };
    let mut reply = match reply {
        Ok(reply) => reply,
        Err(e) => return fail(&format!("no answer from Spectra: {e}"), json),
    };
    if starts_sound(request) && reply.ok {
        reply = confirm(window, reply);
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
fn confirm(window: u32, reply: Reply) -> Reply {
    if reply.status.as_ref().is_none_or(|s| s.playing.is_none()) {
        return reply;
    }
    std::thread::sleep(Duration::from_millis(600));
    match control::send(window, &Request::Status) {
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
        Request::Tv { on: true, .. } => println!("On the TV"),
        Request::Tv { on: false, .. } => println!("Back from the TV"),
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
    match s.tv.as_str() {
        "on" => println!("On the TV"),
        "ready" => println!("TV: ready for Moonlight to open Spectra"),
        "starting" => println!("TV: Sunshine is getting ready"),
        _ => {}
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

/// Open a window in the background, unless one is open already and
/// another was not asked for.
fn open(options: &Options, reduced_motion: bool) -> i32 {
    if !options.new
        && let Some(window) = target(None)
    {
        println!("Spectra is already open (window {window}); `spectra open --new` opens another");
        return 0;
    }
    match start(reduced_motion) {
        Ok(window) => {
            println!("Opened Spectra (window {window})");
            0
        }
        Err(e) => fail(&e, options.json),
    }
}

/// Start a window as a process of its own, which outlives this one, and
/// wait until it takes commands. Its process ID names it.
fn start(reduced_motion: bool) -> Result<u32, String> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let log = log_path();
    // Added to, as other windows may be writing to it too.
    let output = log.as_ref().and_then(|path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });
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
    let window = child.id();
    let step = Duration::from_millis(100);
    for _ in 0..STARTUP.as_millis() / step.as_millis() {
        std::thread::sleep(step);
        if control::send(window, &Request::Status).is_some() {
            return Ok(window);
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
    fn a_window_can_be_named_anywhere_in_the_line() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (parsed, rest) = options(&args(&["--window", "4242", "pause", "--json"])).unwrap();
        assert_eq!(
            parsed,
            Options {
                json: true,
                window: Some(4242),
                new: false
            }
        );
        assert_eq!(rest, args(&["pause", "--json"]));
        let (parsed, rest) = options(&args(&["play", "dude", "--new", "-w", "7"])).unwrap();
        assert!(parsed.new);
        assert_eq!(parsed.window, Some(7));
        assert_eq!(rest, args(&["play", "dude"]));
        assert!(options(&args(&["pause", "--window", "the-big-one"])).is_err());
        // Opening a window with an album file is not a command.
        assert_eq!(run(&args(&["album.json", "--play"])), None);
        assert_eq!(run(&args(&["--window", "7"])), Some(2));
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

    #[test]
    fn tv_reads_on_or_off_and_a_size() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            tv_request(&args(&["on", "--size", "3840x2160@60"])),
            Ok(Request::Tv {
                on: true,
                mode: Some("3840x2160@60".into())
            })
        );
        assert_eq!(
            tv_request(&args(&["off", "--json"])),
            Ok(Request::Tv {
                on: false,
                mode: None
            })
        );
        assert!(tv_request(&args(&[])).is_err());
        assert!(tv_request(&args(&["sideways"])).is_err());
        assert!(tv_request(&args(&["on", "--size", "huge"])).is_err());
    }
}
