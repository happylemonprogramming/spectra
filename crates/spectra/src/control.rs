//! A running Spectra, driven from outside: `spectra play`, `spectra pause`
//! and the rest, for scripts and agents.
//!
//! Each window listens on a Unix socket of its own in `$XDG_RUNTIME_DIR`,
//! `spectra-<pid>.sock`, which only its user can reach (see `windows` for
//! more than one). A command is one line of JSON and so is the answer:
//! what is on the stage afterwards, or why the command could not be done.
//! Commands do what the remote does, so the window always shows the result.
//!
//! ```text
//! → {"command":"play","target":{"copy":"cd-Pz1GkG9F…"},"track":3}
//! ← {"ok":true,"status":{"title":"Blue","playing":3,…}}
//! ```
//!
//! Nothing is read or polled while nobody calls: a thread waits on the socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use iced::Subscription;
use iced::futures::SinkExt;
use iced::futures::channel::mpsc::UnboundedSender;
use serde::{Deserialize, Serialize};

/// How long the window has to answer before the caller gives up.
const ANSWER: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "lowercase")]
pub enum Request {
    Status,
    /// Put something on the stage and play it. `track` counts from 1.
    Play {
        target: Target,
        track: Option<usize>,
    },
    Pause,
    Resume,
    /// Pause what plays, or play what is paused or focused.
    Toggle,
    Next,
    Previous,
    Stop,
    /// Play on the TV, through Sunshine, or come back from it. `mode` is the
    /// size the TV asked for, `WIDTHxHEIGHT@FPS`.
    Tv {
        on: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
    },
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    /// Whatever is on the stage now.
    Stage,
    /// The disc in the drive.
    Disc,
    /// A kept copy, by its library ID.
    Copy(String),
}

/// What is on the stage, as an agent would want to read it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// The window's process ID, which `--window` takes.
    pub window: u32,
    /// The window the gamepad and unaddressed commands go to.
    pub current: bool,
    /// `stage` or `library`: which screen the window shows.
    pub screen: String,
    /// `drive`, `copy` (a kept copy from the library) or `file` (an album
    /// file given on the command line).
    pub source: String,
    /// `no-drive`, `empty`, `reading`, `disc`, `unreadable`, or
    /// `not-watched` when an album file stands in for the drive.
    pub drive: String,
    /// The library ID of what is on the stage, if it has one.
    pub id: Option<String>,
    /// `music`, `game`, `film` or `none`.
    pub kind: String,
    pub title: String,
    pub artist: String,
    pub tracks: Vec<TrackStatus>,
    /// The track playing, counting from 1.
    pub playing: Option<usize>,
    pub paused: bool,
    /// Seconds into the playing track.
    pub elapsed: u32,
    pub in_game: bool,
    /// A film from the drive is playing in VLC.
    pub in_film: bool,
    /// The drive is busy keeping a copy of the disc.
    pub copying: bool,
    /// The TV: `off`, `starting` (Sunshine is getting ready), `ready` (for
    /// Moonlight to open Spectra) or `on` (on the TV's screen).
    #[serde(default)]
    pub tv: String,
    /// The line under the title: what can be done now, or what went wrong.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackStatus {
    pub number: usize,
    pub title: String,
    pub seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

impl Reply {
    pub fn done(status: Status) -> Self {
        Self {
            ok: true,
            error: None,
            status: Some(status),
        }
    }

    pub fn failed(error: String, status: Status) -> Self {
        Self {
            ok: false,
            error: Some(error),
            status: Some(status),
        }
    }
}

/// Where the caller waits for its answer. It answers once.
#[derive(Clone)]
pub struct Responder(Arc<Mutex<Option<mpsc::Sender<Reply>>>>);

impl Responder {
    pub fn send(&self, reply: Reply) {
        if let Some(tx) = self.0.lock().ok().and_then(|mut tx| tx.take()) {
            let _ = tx.send(reply);
        }
    }
}

impl std::fmt::Debug for Responder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Responder")
    }
}

/// Commands from outside, each with where to send its answer.
pub fn subscription() -> Subscription<(Request, Responder)> {
    Subscription::run(|| {
        iced::stream::channel(4, async |mut output| {
            let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
            std::thread::spawn(move || serve(&tx));
            use iced::futures::StreamExt;
            while let Some(request) = rx.next().await {
                if output.send(request).await.is_err() {
                    break;
                }
            }
        })
    })
}

fn serve(tx: &UnboundedSender<(Request, Responder)>) {
    let listener = match listen() {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("spectra: not taking commands: {e}");
            return;
        }
    };
    for stream in listener.incoming().flatten() {
        if tx.is_closed() {
            return;
        }
        // A thread each, so a caller that connects and says nothing holds
        // up nobody else.
        let tx = tx.clone();
        std::thread::spawn(move || answer(stream, &tx));
    }
}

/// This window's socket. One left by a window that had this process ID
/// before is cleared away.
fn listen() -> std::io::Result<UnixListener> {
    let path = crate::windows::socket(std::process::id());
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn answer(stream: UnixStream, tx: &UnboundedSender<(Request, Responder)>) {
    let _ = stream.set_read_timeout(Some(ANSWER));
    let _ = stream.set_write_timeout(Some(ANSWER));
    let mut line = String::new();
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    // Nothing said: a caller seeing whether this window is open.
    if BufReader::new(stream).read_line(&mut line).is_err() || line.trim().is_empty() {
        return;
    }
    let reply = match serde_json::from_str::<Request>(&line) {
        Ok(request) => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let responder = Responder(Arc::new(Mutex::new(Some(reply_tx))));
            if tx.unbounded_send((request, responder)).is_err() {
                return;
            }
            reply_rx.recv_timeout(ANSWER).unwrap_or(Reply {
                ok: false,
                error: Some("Spectra did not answer in time".into()),
                status: None,
            })
        }
        Err(e) => Reply {
            ok: false,
            error: Some(format!("not a command: {e}")),
            status: None,
        },
    };
    let mut text = serde_json::to_string(&reply).expect("a reply serialises");
    text.push('\n');
    let _ = writer.write_all(text.as_bytes());
}

/// Send a command to the window with this process ID. None if it is not
/// running.
pub fn send(window: u32, request: &Request) -> Option<Result<Reply, String>> {
    let stream = UnixStream::connect(crate::windows::socket(window)).ok()?;
    Some(exchange(stream, request).map_err(|e| e.to_string()))
}

fn exchange(mut stream: UnixStream, request: &Request) -> std::io::Result<Reply> {
    stream.set_read_timeout(Some(ANSWER + Duration::from_secs(2)))?;
    let mut text = serde_json::to_string(request).expect("a request serialises");
    text.push('\n');
    stream.write_all(text.as_bytes())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    serde_json::from_str(&line).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_read_as_they_are_written() {
        let play = Request::Play {
            target: Target::Copy("SLUS-00152".into()),
            track: Some(3),
        };
        let text = serde_json::to_string(&play).unwrap();
        assert_eq!(
            text,
            r#"{"command":"play","target":{"copy":"SLUS-00152"},"track":3}"#
        );
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), play);
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"command":"pause"}"#).unwrap(),
            Request::Pause
        );
        assert_eq!(
            serde_json::to_string(&Request::Tv {
                on: true,
                mode: Some("3840x2160@60".into())
            })
            .unwrap(),
            r#"{"command":"tv","on":true,"mode":"3840x2160@60"}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"command":"tv","on":false}"#).unwrap(),
            Request::Tv {
                on: false,
                mode: None
            }
        );
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"command":"play","target":"disc","track":null}"#)
                .unwrap(),
            Request::Play {
                target: Target::Disc,
                track: None
            }
        );
    }
}
