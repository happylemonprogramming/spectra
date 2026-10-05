//! Playing on the TV: the film or game Spectra starts, on a screen of its
//! own that Sunshine streams to Moonlight on the TV. Spectra itself stays on
//! the computer's screen, where it is the remote: what plays, pausing,
//! stopping, picking the next thing. The TV's own remote reaches the film
//! too (`moonlight`).
//!
//! The screen is a headless output in Hyprland named `SPECTRA-TV`, so that
//! Sunshine can be pointed at it by name (`output_name = SPECTRA-TV`); while
//! there is none, Sunshine falls back to the first screen. It is as large as
//! the TV asked for, when Sunshine says, and 1920×1080 otherwise. What
//! Spectra starts - VLC, RetroArch - is moved there as it opens, full
//! screen, without focus following, so the keyboard stays where the user is
//! working. Between them the TV shows a card of Spectra's, so it is never
//! the bare desktop.
//!
//! "Play on the TV" only gets the TV ready: it starts Sunshine if it is not
//! running, and says when Sunshine is taking connections. Moonlight opening
//! Spectra runs its prep command, `spectra tv on`, which makes the screen;
//! ending the stream runs `spectra tv off`, which takes it away and brings
//! anything playing back to the computer. Sunshine is stopped when the user
//! is done, but only if Spectra started it: one already running stays for
//! whoever runs it.
//!
//! One window has the TV at a time. Helpers outlive Spectra to take the
//! screen away, let the computer sleep again and stop a Sunshine it started,
//! if Spectra goes without doing so, crashes included.

use std::net::{Shutdown, SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use spectra_core::lock::{self, Lock};

use crate::{game, hyprland, windows};

/// The screen's name in Hyprland, and so in Sunshine's `output_name`.
pub const SCREEN: &str = "SPECTRA-TV";
/// Its workspace's: a name, not one of the user's numbers.
const WORKSPACE: &str = "TV";
/// A TV's, when Sunshine has not said what this one wants.
const MODE: &str = "1920x1080@60";
/// Sunshine's port for Moonlight: open once it takes connections.
const SUNSHINE_PORT: u16 = 47989;
/// Sunshine checks the encoders as it starts, which takes a few seconds.
const SUNSHINE_WAIT: Duration = Duration::from_secs(30);

static ON: AtomicBool = AtomicBool::new(false);

/// This window has the TV: what it starts goes there.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// The TV, held by this window. Dropping it brings everything back.
pub struct Tv {
    _lock: Lock,
    events: Option<UnixStream>,
    keeper: Option<Child>,
    /// The card the TV shows between films and games.
    card: Option<Child>,
}

/// Sunshine, ready for the TV to connect. Dropping it stops Sunshine if this
/// window started it.
pub struct Host {
    sunshine: Option<Child>,
    keeper: Option<Child>,
}

impl Host {
    /// Start Sunshine, unless it is running already. It is not ready yet:
    /// `wait_until_ready` says when it is.
    pub fn start() -> Result<Self, String> {
        if let Some(why) = unavailable() {
            return Err(why);
        }
        if sunshine_running() {
            return Ok(Self {
                sunshine: None,
                keeper: None,
            });
        }
        // Its own process group, so it is not ended with the terminal or
        // the window; its log is its own, in ~/.config/sunshine.
        let sunshine = Command::new("sunshine")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("Couldn't start Sunshine: {e}"))?;
        let wait = format!(
            "tail --pid={} -f /dev/null; kill {}",
            std::process::id(),
            sunshine.id()
        );
        Ok(Self {
            keeper: helper(Command::new("sh").args(["-c", &wait])),
            sunshine: Some(sunshine),
        })
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(keeper) = self.keeper.take() {
            end_group(keeper);
        }
        if let Some(mut sunshine) = self.sunshine.take() {
            // SAFETY: a plain kill(2) of a child this window started.
            unsafe { libc::kill(sunshine.id() as libc::pid_t, libc::SIGTERM) };
            std::thread::spawn(move || sunshine.wait());
        }
    }
}

/// Wait for Sunshine to take connections, which is when Moonlight can
/// reach it. False if it does not within half a minute.
pub fn wait_until_ready() -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], SUNSHINE_PORT));
    let until = Instant::now() + SUNSHINE_WAIT;
    while Instant::now() < until {
        if TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

/// Say so on the desktop, for a user looking at something else.
pub fn notify(title: &str, body: &str) {
    if game::on_path("notify-send").is_none() {
        return;
    }
    let sent = Command::new("notify-send")
        .args(["--app-name=Spectra", title, body])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut sent) = sent {
        std::thread::spawn(move || sent.wait());
    }
}

fn sunshine_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        std::fs::read_to_string(entry.path().join("comm"))
            .is_ok_and(|name| name.trim() == "sunshine")
    })
}

fn lock_path() -> PathBuf {
    windows::runtime_dir().join("spectra-tv.lock")
}

/// The window that has the TV, if one does.
pub fn holder() -> Option<u32> {
    let path = lock_path();
    // The file names the last window to hold it, which may have let go.
    if let Ok(Some(_free)) = lock::take(&path, "") {
        return None;
    }
    let pid = lock::holder(&path)?.parse().ok()?;
    windows::running().contains(&pid).then_some(pid)
}

/// Whatever is in the way of playing on the TV, in words.
pub fn unavailable() -> Option<String> {
    if !hyprland::running() {
        return Some("Playing on the TV needs Hyprland".into());
    }
    if game::on_path("sunshine").is_none() {
        return Some(
            "Playing on the TV needs Sunshine: omarchy-install-service-sunshine sets it up".into(),
        );
    }
    None
}

/// What the screen is made as: `WIDTHxHEIGHT@FPS`, if `mode` is one.
pub fn mode(mode: &str) -> Option<String> {
    let (size, fps) = mode.split_once('@').unwrap_or((mode, "60"));
    let (w, h) = size.split_once('x')?;
    let n = |s: &str| s.parse::<u32>().ok().filter(|&n| n > 0 && n <= 16384);
    Some(format!("{}x{}@{}", n(w)?, n(h)?, n(fps)?))
}

/// Drawn as large on a 4K TV as on a 1080p one, seen from the sofa.
fn scale(mode: &str) -> u32 {
    let height: u32 = mode
        .split(['x', '@'])
        .nth(1)
        .and_then(|h| h.parse().ok())
        .unwrap_or(1080);
    (height / 1080).max(1)
}

impl Tv {
    /// Make the TV's screen, with Spectra's card on it, and send there what
    /// this window starts. `asked` is the size the TV wants, as `mode` reads
    /// it.
    pub fn start(asked: Option<&str>) -> Result<Self, String> {
        if let Some(why) = unavailable() {
            return Err(why);
        }
        let me = std::process::id();
        let path = lock_path();
        let lock = match lock::take(&path, &me.to_string()) {
            Ok(Some(lock)) => lock,
            Ok(None) => {
                return Err(match holder() {
                    Some(pid) => format!("Spectra window {pid} is on the TV"),
                    None => "Another Spectra window is on the TV".into(),
                });
            }
            Err(e) => return Err(format!("Couldn't take the TV: {e}")),
        };
        let mode = asked.and_then(mode).unwrap_or_else(|| MODE.into());
        hyprland::add_screen(SCREEN, &mode, scale(&mode), WORKSPACE)?;
        ON.store(true, Ordering::Relaxed);
        // Watching before anything opens, so nothing is missed. VLC is put
        // full screen by `film` itself, and RetroArch asks to be; the card
        // is put full screen here.
        let card_pid = Arc::new(AtomicU32::new(0));
        let card_seen = card_pid.clone();
        let events = hyprland::events(move |line| {
            if let Some(rest) = line.strip_prefix("openwindow>>")
                && let Some(address) = rest.split(',').next()
                && let Some(window) = hyprland::window_at(address)
                && started(window.pid)
            {
                hyprland::move_to(&window, SCREEN);
                if window.pid == card_seen.load(Ordering::Relaxed) && !window.fullscreen {
                    hyprland::fullscreen(&window);
                }
            }
            true
        });
        // A film already playing goes over too.
        for window in hyprland::windows() {
            if started(window.pid) {
                hyprland::move_to(&window, SCREEN);
            }
        }
        let card = card();
        if let Some(card) = &card {
            card_pid.store(card.id(), Ordering::Relaxed);
        }
        Ok(Self {
            _lock: lock,
            events,
            keeper: keeper(me),
            card,
        })
    }
}

impl Drop for Tv {
    fn drop(&mut self) {
        if let Some(events) = self.events.take() {
            let _ = events.shutdown(Shutdown::Both);
        }
        ON.store(false, Ordering::Relaxed);
        if let Some(mut card) = self.card.take() {
            let _ = card.kill();
            let _ = card.wait();
        }
        // What is playing there comes back to the computer's screen.
        hyprland::remove_screen(SCREEN);
        if let Some(keeper) = self.keeper.take() {
            end_group(keeper);
        }
    }
}

/// Take the screen away after a window that went without doing so itself.
pub fn clear_leftover() {
    if holder().is_none() && hyprland::running() {
        hyprland::remove_screen(SCREEN);
    }
}

/// Spectra's card for the TV, as a program of its own: `spectra --tv-card`.
/// It goes if this window does.
fn card() -> Option<Child> {
    let mut command = Command::new(std::env::current_exe().ok()?);
    command
        .arg(crate::TV_CARD)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: prctl is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    command.spawn().ok()
}

/// A program this window started - VLC, RetroArch, the card - and not the
/// window itself, which stays on the computer's screen.
fn started(pid: u32) -> bool {
    pid != std::process::id() && ours(pid)
}

/// Spectra, or a program it started.
fn ours(pid: u32) -> bool {
    let me = std::process::id();
    let mut pid = pid;
    // Parents only go back so far; the limit is for a /proc that lies.
    for _ in 0..16 {
        if pid == me {
            return true;
        }
        match parent(pid) {
            Some(up) if up > 1 => pid = up,
            _ => return false,
        }
    }
    false
}

fn parent(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The name, in brackets, can hold spaces: count from after it.
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Keep the computer awake while the TV is on, and take the screen away if
/// this window goes without doing it.
fn keeper(window: u32) -> Option<Child> {
    let wait =
        format!("tail --pid={window} -f /dev/null; hyprctl output remove {SCREEN} >/dev/null 2>&1");
    if game::on_path("systemd-inhibit").is_none() {
        return helper(Command::new("sh").args(["-c", &wait]));
    }
    helper(
        Command::new("systemd-inhibit")
            .args([
                "--what=idle:sleep",
                "--who=Spectra",
                "--why=Playing on the TV",
            ])
            .args(["sh", "-c", &wait]),
    )
}

/// A helper in a process group of its own, so it outlives the window.
fn helper(command: &mut Command) -> Option<Child> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()
}

/// End a helper's whole group: the shell, and what it waits on.
fn end_group(mut helper: Child) {
    // SAFETY: a plain kill(2) of a process group this window started.
    unsafe { libc::kill(-(helper.id() as libc::pid_t), libc::SIGTERM) };
    let _ = helper.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_a_tv_asks_for_is_read_or_refused() {
        assert_eq!(mode("3840x2160@60").as_deref(), Some("3840x2160@60"));
        assert_eq!(mode("1280x720").as_deref(), Some("1280x720@60"));
        assert_eq!(mode("0x1080@60"), None);
        assert_eq!(mode("big"), None);
        assert_eq!(mode("1920x1080@fast"), None);
    }

    #[test]
    fn a_4k_tv_is_drawn_twice_as_large() {
        assert_eq!(scale("1920x1080@60"), 1);
        assert_eq!(scale("1280x720@60"), 1);
        assert_eq!(scale("2560x1440@60"), 1);
        assert_eq!(scale("3840x2160@60"), 2);
    }

    #[test]
    fn this_process_is_its_own_but_did_not_start_itself() {
        assert!(ours(std::process::id()));
        assert!(!started(std::process::id()));
        assert!(!ours(1));
    }
}
