//! The TV's remote, through Moonlight, as the same remote control the
//! keyboard is.
//!
//! Moonlight sends a TV remote's presses to Sunshine, which plays them on a
//! keyboard and a mouse of its own: the Siri Remote's ring is the arrow keys
//! and its click is the left button; Back and Play/Pause stay with Moonlight.
//! So the click is two buttons: a click is Select, and holding it is Back.
//! Hyprland would hand those to whichever window has focus, which is on the
//! computer's screen, where the user is working. So while Spectra has the TV
//! it reads those two devices itself, as it reads gamepads, and holds them
//! (`EVIOCGRAB`) so a press on the TV never types into the user's work.
//! They are let go when the TV is.
//!
//! Sunshine's devices are found by name, and looked for again every second:
//! they come with a stream, which can start after Spectra is on the TV.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::Subscription;
use iced::futures::SinkExt;
use iced::futures::channel::mpsc::UnboundedSender;

use crate::Remote;

/// Sunshine's keyboard and mouse. Its absolute mouse, touchscreen and pen
/// are the touch surface, which is left alone.
const DEVICES: &[&str] = &["libvirtualhid Keyboard", "libvirtualhid Mouse"];
/// `_IOW('E', 0x90, int)`: hold a device, so nothing else hears it.
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
const EV_KEY: u16 = 1;
/// How often to look for Sunshine's devices, and to see whether the TV is
/// still wanted.
const LOOK: Duration = Duration::from_secs(1);
/// The left button: the Siri Remote's click.
const CLICK: u16 = 272;
/// A click held this long is Back.
const HOLD: Duration = Duration::from_millis(500);

pub fn subscription() -> Subscription<Remote> {
    Subscription::run(|| {
        iced::stream::channel(16, async |mut output| {
            let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
            std::thread::spawn(move || listen(&tx));
            use iced::futures::StreamExt;
            while let Some(press) = rx.next().await {
                if output.send(press).await.is_err() {
                    break;
                }
            }
        })
    })
}

/// A key or button, as the remote's.
fn remote(code: u16) -> Option<Remote> {
    Some(match code {
        103 => Remote::Up,
        108 => Remote::Down,
        105 => Remote::Left,
        106 => Remote::Right,
        // The Siri Remote's click is the mouse's left button; Enter for a
        // keyboard at the TV.
        CLICK | 28 | 96 => Remote::Select,
        // Escape, Backspace, the right button: for remotes and keyboards
        // that send them.
        1 | 14 | 273 => Remote::Back,
        57 | 164 => Remote::PlayPause,
        163 => Remote::Next,
        165 => Remote::Previous,
        _ => return None,
    })
}

/// Hold Sunshine's devices and pass their presses on, until nobody listens.
/// Dropping a device lets it go.
fn listen(tx: &UnboundedSender<Remote>) {
    let mut held: Vec<(PathBuf, File)> = Vec::new();
    let mut looked: Option<Instant> = None;
    let mut clicked: Option<Instant> = None;
    while !tx.is_closed() {
        if looked.is_none_or(|at| at.elapsed() >= LOOK) {
            looked = Some(Instant::now());
            held.retain(|(path, _)| path.exists());
            for path in devices() {
                if !held.iter().any(|(p, _)| *p == path)
                    && let Some(file) = hold(&path)
                {
                    held.push((path, file));
                }
            }
        }
        if held.is_empty() {
            std::thread::sleep(LOOK);
            continue;
        }
        let mut polls: Vec<libc::pollfd> = held
            .iter()
            .map(|(_, file)| libc::pollfd {
                fd: file.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
        // SAFETY: the descriptors are open for as long as `held` is.
        let n = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, 500) };
        if n <= 0 {
            continue;
        }
        let mut gone = Vec::new();
        for (i, poll) in polls.iter().enumerate() {
            if poll.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                gone.push(i);
                continue;
            }
            if poll.revents & libc::POLLIN == 0 {
                continue;
            }
            let Some(keys) = read(&held[i].1) else {
                gone.push(i);
                continue;
            };
            for (code, value) in keys {
                // Only presses count: a held key repeats, and a remote's
                // press is one step. The click counts when it is let go,
                // once it is known whether it was held.
                let press = match (code, value) {
                    (CLICK, 1) => {
                        clicked = Some(Instant::now());
                        None
                    }
                    (CLICK, 0) => clicked.take().map(|at| {
                        if at.elapsed() >= HOLD {
                            Remote::Back
                        } else {
                            Remote::Select
                        }
                    }),
                    (code, 1) => remote(code),
                    _ => None,
                };
                if let Some(press) = press {
                    let _ = tx.unbounded_send(press);
                }
            }
        }
        for i in gone.into_iter().rev() {
            held.remove(i);
        }
    }
}

/// The keys and buttons waiting on a device, with 1 for down, 0 for up and
/// 2 for a held key repeating. None once it has gone.
fn read(mut file: &File) -> Option<Vec<(u16, i32)>> {
    let size = std::mem::size_of::<libc::input_event>();
    let mut buffer = vec![0u8; size * 32];
    let n = match file.read(&mut buffer) {
        Ok(n) => n,
        Err(e) if e.kind() == ErrorKind::WouldBlock => return Some(Vec::new()),
        Err(_) => return None,
    };
    // After the time: type, code and value, a u16, a u16 and an i32.
    let at = size - 8;
    Some(
        buffer[..n]
            .chunks_exact(size)
            .filter_map(|event| {
                let kind = u16::from_ne_bytes([event[at], event[at + 1]]);
                let code = u16::from_ne_bytes([event[at + 2], event[at + 3]]);
                let value = i32::from_ne_bytes(event[at + 4..at + 8].try_into().ok()?);
                (kind == EV_KEY).then_some((code, value))
            })
            .collect(),
    )
}

/// Open a device and hold it for this window alone.
fn hold(path: &PathBuf) -> Option<File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    // SAFETY: an ioctl on a descriptor this function owns, with an int.
    let held = unsafe { libc::ioctl(file.as_raw_fd(), EVIOCGRAB as _, 1 as libc::c_int) };
    (held == 0).then_some(file)
}

/// Sunshine's keyboard and mouse, by their nodes in `/dev/input`.
fn devices() -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string("/proc/bus/input/devices") else {
        return Vec::new();
    };
    named(&text)
}

fn named(text: &str) -> Vec<PathBuf> {
    text.split("\n\n")
        .filter(|block| {
            block.lines().any(|line| {
                line.strip_prefix("N: Name=\"")
                    .and_then(|name| name.strip_suffix('"'))
                    .is_some_and(|name| DEVICES.contains(&name))
            })
        })
        .filter_map(|block| {
            let handlers = block.lines().find_map(|l| l.strip_prefix("H: Handlers="))?;
            let event = handlers
                .split_whitespace()
                .find(|h| h.starts_with("event"))?;
            Some(PathBuf::from("/dev/input").join(event))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sunshines_keyboard_and_mouse_are_found_but_not_the_touch_surface() {
        let text = "\
I: Bus=0003 Vendor=0000 Product=0000 Version=0000
N: Name=\"libvirtualhid Keyboard\"
H: Handlers=sysrq kbd event16 rfkill

I: Bus=0003 Vendor=0000 Product=0000 Version=0000
N: Name=\"libvirtualhid Mouse\"
H: Handlers=event17 mouse3

I: Bus=0003 Vendor=0000 Product=0000 Version=0000
N: Name=\"libvirtualhid Mouse (Absolute)\"
H: Handlers=event18 mouse5 js0

I: Bus=0011 Vendor=0001 Product=0001 Version=ab83
N: Name=\"AT Translated Set 2 keyboard\"
H: Handlers=sysrq kbd event3 leds";
        assert_eq!(
            named(text),
            [
                PathBuf::from("/dev/input/event16"),
                PathBuf::from("/dev/input/event17")
            ]
        );
    }

    #[test]
    fn the_siri_remote_is_the_arrows_and_a_click() {
        assert!(matches!(remote(103), Some(Remote::Up)));
        assert!(matches!(remote(106), Some(Remote::Right)));
        assert!(matches!(remote(272), Some(Remote::Select)));
        assert!(remote(30).is_none());
    }
}
