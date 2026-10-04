//! Films, handed to VLC, which reads the disc and draws its menus. Spectra
//! stays the remote.
//!
//! VLC runs as its own program with only its control socket for an
//! interface. A gamepad's presses go through that socket - the d-pad moves
//! through the disc's menus - and the keyboard reaches VLC directly, with
//! Esc and Backspace set to quit, as they leave everything else in Spectra.
//!
//! VLC 3 draws through XWayland, and two things go wrong there that must not
//! reach the user: its own full screen is ignored, and if its window is
//! closed it plays on, sound and all, with nothing on screen. So the window
//! and the program are kept as one. Hyprland's event socket says when the
//! window opens - it is then made full screen and focused - and when it
//! closes, which ends VLC. A VLC that never opens a window is ended too.
//! Ending asks politely, then less so: a VLC that has lost its window can
//! ignore SIGTERM.
//!
//! While a film plays the screen is kept awake with a logind idle inhibitor,
//! which hypridle honours; VLC's own way, xdg-screensaver, needs xset.

use std::io::Write;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use spectra_core::DiscKind;

use crate::game;

/// Long enough for a slow drive to spin up and a disc to reach its menu.
const WINDOW_WAIT: Duration = Duration::from_secs(30);
/// A closed window ends the film unless VLC puts another up within this.
const WINDOW_GAP: Duration = Duration::from_millis(400);
/// How long each way of ending VLC is given before the next.
const GRACE: Duration = Duration::from_secs(1);
/// The same, once its window has gone: VLC is playing unseen, and in that
/// state it ignores being asked anyway.
const GRACE_UNSEEN: Duration = Duration::from_millis(300);

/// The remote's buttons, as VLC's actions.
#[derive(Debug, Clone, Copy)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Activate,
    PlayPause,
    PreviousChapter,
    NextChapter,
    DiscMenu,
}

impl Key {
    fn action(self) -> &'static str {
        match self {
            Self::Up => "key-nav-up",
            Self::Down => "key-nav-down",
            Self::Left => "key-nav-left",
            Self::Right => "key-nav-right",
            Self::Activate => "key-nav-activate",
            Self::PlayPause => "key-play-pause",
            Self::PreviousChapter => "key-chapter-prev",
            Self::NextChapter => "key-chapter-next",
            Self::DiscMenu => "key-disc-menu",
        }
    }
}

/// VLC's keys, set to match Spectra's: Esc and Backspace leave, M is the
/// disc's menu, Page Up and Down move a chapter. Full screen cannot be left,
/// as a windowed film is one closed window away from playing unseen.
const KEYS: &[&str] = &[
    "--key-quit=Esc\tBackspace\tCtrl+q",
    "--key-leave-fullscreen=",
    "--key-toggle-fullscreen=",
    "--key-disc-menu=m\tShift+m",
    "--key-chapter-prev=Page Up\tShift+p",
    "--key-chapter-next=Page Down\tShift+n",
];

/// A film playing in VLC. Dropping it ends VLC.
pub struct Film {
    socket: PathBuf,
    signals: mpsc::Sender<Signal>,
}

enum Signal {
    /// A line from Hyprland's event socket.
    Hyprland(String),
    Stop,
}

pub fn installed() -> bool {
    game::on_path("vlc").is_some()
}

/// What VLC calls the disc in this drive, if it is a film.
pub fn uri(kind: &DiscKind, drive: &Path) -> Option<String> {
    let scheme = match kind {
        DiscKind::DvdVideo { .. } => "dvd",
        DiscKind::BluRayVideo => "bluray",
        DiscKind::VideoCd { .. } => "vcd",
        _ => return None,
    };
    Some(format!("{scheme}://{}", block_node(drive)?.display()))
}

/// VLC reads `/dev/srN`; Spectra talks to the drive through `/dev/sgN`.
pub fn block_node(drive: &Path) -> Option<PathBuf> {
    if drive.to_str()?.starts_with("/dev/sr") {
        return Some(drive.to_path_buf());
    }
    spectra_core::drive::list()
        .into_iter()
        .find(|info| info.generic.as_deref() == Some(drive))?
        .block
}

impl Film {
    /// Start VLC full screen on the disc. `done` is called once VLC has gone,
    /// with what went wrong, in words, if anything did.
    pub fn start(
        uri: &str,
        done: impl FnOnce(Option<String>) + Send + 'static,
    ) -> std::io::Result<Self> {
        let program =
            game::on_path("vlc").ok_or_else(|| std::io::Error::other("VLC isn't installed"))?;
        // Named for this window: another window's film has a VLC of its own.
        let socket =
            crate::windows::runtime_dir().join(format!("spectra-vlc-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let log = game::cache_dir()
            .and_then(|dir| std::fs::File::create(dir.join("film.log")).ok())
            .map_or_else(Stdio::null, Stdio::from);

        let (signals, rx) = mpsc::channel();
        // Listening before VLC starts, so its window cannot open unseen.
        let events = hyprland::events(signals.clone());

        let mut command = Command::new(program);
        command
            .args(["-I", "oldrc", "--rc-fake-tty", "--rc-unix"])
            .arg(&socket)
            .args(["--fullscreen", "--no-video-title-show"])
            .args(KEYS)
            .arg(uri)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        // SAFETY: prctl is async-signal-safe. VLC goes if Spectra does, and
        // at once: with nothing left to watch its window, it cannot be left.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
        let child = command.spawn()?;
        keep_awake(child.id());

        let film_socket = socket.clone();
        std::thread::spawn(move || {
            let trouble = supervise(child, &film_socket, &rx, events.is_some());
            if let Some(events) = events {
                let _ = events.shutdown(Shutdown::Both);
            }
            let _ = std::fs::remove_file(&film_socket);
            hyprland::focus_pid(std::process::id());
            done(trouble);
        });
        Ok(Self { socket, signals })
    }

    pub fn press(&self, key: Key) {
        if let Ok(mut socket) = UnixStream::connect(&self.socket) {
            let _ = writeln!(socket, "key {}", key.action());
        }
    }
}

impl Drop for Film {
    fn drop(&mut self) {
        let _ = self.signals.send(Signal::Stop);
    }
}

/// Watch VLC and its window until the film is over, and make sure VLC is
/// gone when it is.
fn supervise(
    mut child: Child,
    socket: &Path,
    signals: &mpsc::Receiver<Signal>,
    watching: bool,
) -> Option<String> {
    let pid = child.id();
    let started = Instant::now();
    let mut window: Option<String> = None;
    let mut closed: Option<Instant> = None;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return None,
            Ok(Some(status)) => return Some(format!("VLC stopped with {status}")),
            Ok(None) => {}
            Err(e) => {
                end(&mut child, socket);
                return Some(e.to_string());
            }
        }
        match signals.recv_timeout(Duration::from_millis(100)) {
            Ok(Signal::Stop) | Err(RecvTimeoutError::Disconnected) => {
                end(&mut child, socket);
                return None;
            }
            Ok(Signal::Hyprland(line)) => {
                if line.starts_with("openwindow>>")
                    && let Some(found) = hyprland::window_of(pid)
                {
                    if window.as_ref() != Some(&found.address) {
                        hyprland::present(&found);
                    }
                    window = Some(found.address);
                    closed = None;
                } else if let Some(address) = line.strip_prefix("closewindow>>")
                    && window.as_deref() == Some(hyprland::plain(address))
                {
                    window = None;
                    closed = Some(Instant::now());
                } else if let Some(address) = line.strip_prefix("activewindowv2>>")
                    && window.as_deref() == Some(hyprland::plain(address))
                {
                    // The film was looked at: the gamepad is for it, and so
                    // for this window, whichever other films are open.
                    crate::windows::claim();
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if closed.is_some_and(|at| at.elapsed() > WINDOW_GAP) {
            end_within(&mut child, socket, GRACE_UNSEEN);
            return None;
        }
        if watching && window.is_none() && closed.is_none() && started.elapsed() > WINDOW_WAIT {
            end(&mut child, socket);
            return Some("VLC didn't open a window".into());
        }
    }
}

fn end(child: &mut Child, socket: &Path) {
    end_within(child, socket, GRACE);
}

/// Ask VLC to quit, then tell it, then make it, giving each a while.
fn end_within(child: &mut Child, socket: &Path, grace: Duration) {
    if let Ok(mut socket) = UnixStream::connect(socket) {
        let _ = socket.write_all(b"quit\n");
    }
    if gone_within(child, grace) {
        return;
    }
    // SAFETY: a plain kill(2) of our own child, which has not been reaped.
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    if gone_within(child, grace) {
        return;
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn gone_within(child: &mut Child, wait: Duration) -> bool {
    let until = Instant::now() + wait;
    while Instant::now() < until {
        if !matches!(child.try_wait(), Ok(None)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Hold off the screen blanking for as long as VLC runs. The inhibitor
/// lives exactly as long as VLC, so there is nothing to clean up after.
fn keep_awake(pid: u32) {
    if game::on_path("systemd-inhibit").is_none() {
        return;
    }
    let inhibitor = Command::new("systemd-inhibit")
        .args(["--what=idle", "--who=Spectra", "--why=Playing a film"])
        .args(["tail", &format!("--pid={pid}"), "-f", "/dev/null"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut inhibitor) = inhibitor {
        std::thread::spawn(move || inhibitor.wait());
    }
}

/// Just enough of Hyprland: its events, its windows, and two dispatches.
/// Elsewhere none of this happens, and VLC's own full screen is relied on.
mod hyprland {
    use super::Signal;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;

    pub struct Window {
        pub address: String,
        fullscreen: bool,
    }

    fn dir() -> Option<PathBuf> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
        let instance = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
        Some(PathBuf::from(runtime).join("hypr").join(instance))
    }

    /// Hyprland's events, a line each, until the returned socket is shut.
    pub fn events(tx: mpsc::Sender<Signal>) -> Option<UnixStream> {
        let socket = UnixStream::connect(dir()?.join(".socket2.sock")).ok()?;
        let reader = socket.try_clone().ok()?;
        std::thread::spawn(move || {
            for line in BufReader::new(reader).lines() {
                let Ok(line) = line else { break };
                if tx.send(Signal::Hyprland(line)).is_err() {
                    break;
                }
            }
        });
        Some(socket)
    }

    /// Events give addresses without the `0x` that `hyprctl clients` has.
    pub fn plain(address: &str) -> &str {
        address.trim().trim_start_matches("0x")
    }

    /// The window that this process has open.
    pub fn window_of(pid: u32) -> Option<Window> {
        let output = Command::new("hyprctl")
            .args(["-j", "clients"])
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let clients: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let client = clients
            .as_array()?
            .iter()
            .find(|client| client["pid"].as_u64() == Some(u64::from(pid)))?;
        Some(Window {
            address: plain(client["address"].as_str()?).to_string(),
            fullscreen: client["fullscreen"].as_u64().unwrap_or(0) != 0,
        })
    }

    /// Run a dispatch, written for Hyprland's Lua config first and its
    /// older one if that is refused.
    fn dispatch(lua: &str, legacy: &[&str]) {
        let out = Command::new("hyprctl")
            .args(["dispatch", lua])
            .stderr(Stdio::null())
            .output();
        let ok = out.is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "ok");
        if !ok {
            let _ = Command::new("hyprctl")
                .arg("dispatch")
                .args(legacy)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    /// Focus the window and fill the screen with it.
    pub fn present(window: &Window) {
        let target = format!("address:0x{}", window.address);
        dispatch(
            &format!("hl.dsp.focus({{ window = \"{target}\" }})"),
            &["focuswindow", &target],
        );
        if !window.fullscreen {
            dispatch(
                &format!(
                    "hl.dsp.window.fullscreen({{ mode = \"fullscreen\", window = \"{target}\" }})"
                ),
                &["fullscreen", "0"],
            );
        }
    }

    /// Back to Spectra's own window once the film is over.
    pub fn focus_pid(pid: u32) {
        if dir().is_none() {
            return;
        }
        let target = format!("pid:{pid}");
        dispatch(
            &format!("hl.dsp.focus({{ window = \"{target}\" }})"),
            &["focuswindow", &target],
        );
    }
}
