//! Just enough of Hyprland: its events, its windows and screens, and a few
//! dispatches. Elsewhere none of this happens: VLC's own full screen is
//! relied on, and there is no TV to play on.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub struct Window {
    pub address: String,
    pub pid: u32,
    pub fullscreen: bool,
}

fn dir() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let instance = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    Some(PathBuf::from(runtime).join("hypr").join(instance))
}

/// Spectra is running under Hyprland.
pub fn running() -> bool {
    dir().is_some_and(|dir| dir.join(".socket.sock").exists())
}

/// Hyprland's events, a line each to `each` until it returns false or the
/// returned socket is shut.
pub fn events(each: impl Fn(String) -> bool + Send + 'static) -> Option<UnixStream> {
    let socket = UnixStream::connect(dir()?.join(".socket2.sock")).ok()?;
    let reader = socket.try_clone().ok()?;
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            if !each(line) {
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

fn hyprctl(args: &[&str]) -> Option<String> {
    let out = Command::new("hyprctl")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn json(what: &str) -> Option<serde_json::Value> {
    serde_json::from_str(&hyprctl(&["-j", what])?).ok()
}

/// Every window open.
pub fn windows() -> Vec<Window> {
    let Some(clients) = json("clients") else {
        return Vec::new();
    };
    clients
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|client| {
            Some(Window {
                address: plain(client["address"].as_str()?).to_string(),
                pid: u32::try_from(client["pid"].as_u64()?).ok()?,
                fullscreen: client["fullscreen"].as_u64().unwrap_or(0) != 0,
            })
        })
        .collect()
}

/// The window that this process has open.
pub fn window_of(pid: u32) -> Option<Window> {
    windows().into_iter().find(|window| window.pid == pid)
}

/// The window at this address, as an event gives it.
pub fn window_at(address: &str) -> Option<Window> {
    let address = plain(address);
    windows()
        .into_iter()
        .find(|window| window.address == address)
}

/// The screens, by name, and the workspace showing on each.
pub fn screens() -> Vec<(String, i64)> {
    let Some(monitors) = json("monitors") else {
        return Vec::new();
    };
    monitors
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some((
                m["name"].as_str()?.to_string(),
                m["activeWorkspace"]["id"].as_i64()?,
            ))
        })
        .collect()
}

/// Make a screen with nothing behind it, with a workspace of its own named
/// `workspace`, so that it takes none of the user's numbered ones.
pub fn add_screen(name: &str, mode: &str, scale: u32, workspace: &str) -> Result<(), String> {
    let lua = format!(
        "hl.workspace_rule({{ workspace = \"name:{workspace}\", monitor = \"{name}\", default = true }})"
    );
    if hyprctl(&["eval", &lua]).as_deref() != Some("ok") {
        let _ = hyprctl(&[
            "keyword",
            "workspace",
            &format!("name:{workspace},monitor:{name},default:true"),
        ]);
    }
    // Its own rule first, so the screen is made as it should be. Made first
    // and set after, it starts as a catch-all rule has it - the laptop's
    // scale - and a window put there meanwhile is drawn for that screen,
    // a corner of it, ever after.
    let lua = format!(
        "hl.monitor({{ output = \"{name}\", mode = \"{mode}\", position = \"auto-right\", scale = {scale} }})"
    );
    if hyprctl(&["eval", &lua]).as_deref() != Some("ok") {
        let _ = hyprctl(&[
            "keyword",
            "monitor",
            &format!("{name},{mode},auto-right,{scale}"),
        ]);
    }
    if !screens().iter().any(|(screen, _)| screen == name) {
        let said = hyprctl(&["output", "create", "headless", name]).unwrap_or_default();
        if said != "ok" {
            return Err(format!("Hyprland would not make a screen: {said}"));
        }
    }
    // Then wait until it is the size and scale asked for.
    let size = mode.split('@').next().unwrap_or(mode);
    for _ in 0..40 {
        let ready = json("monitors").is_some_and(|monitors| {
            monitors.as_array().into_iter().flatten().any(|m| {
                m["name"].as_str() == Some(name)
                    && format!("{}x{}", m["width"], m["height"]) == size
                    && m["scale"].as_f64() == Some(f64::from(scale))
            })
        });
        if ready {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err(format!("Hyprland made the screen, but not at {size}"))
}

/// Take the screen away. Its windows go to the screens left.
pub fn remove_screen(name: &str) {
    let _ = hyprctl(&["output", "remove", name]);
}

/// Run a dispatch, written for Hyprland's Lua config first and its
/// older one if that is refused.
fn dispatch(lua: &str, legacy: &[&str]) {
    if hyprctl(&["dispatch", lua]).as_deref() == Some("ok") {
        return;
    }
    let mut args = vec!["dispatch"];
    args.extend_from_slice(legacy);
    let _ = hyprctl(&args);
}

fn target(window: &Window) -> String {
    format!("address:0x{}", window.address)
}

/// Focus the window and fill the screen with it.
pub fn present(window: &Window) {
    let target = target(window);
    dispatch(
        &format!("hl.dsp.focus({{ window = \"{target}\" }})"),
        &["focuswindow", &target],
    );
    if !window.fullscreen {
        fullscreen(window);
    }
}

/// Fill the screen with the window, or stop filling it, without focusing
/// it where Hyprland lets that be done.
pub fn fullscreen(window: &Window) {
    let target = target(window);
    dispatch(
        &format!("hl.dsp.window.fullscreen({{ mode = \"fullscreen\", window = \"{target}\" }})"),
        &["fullscreen", "0"],
    );
}

/// Move the window to the screen, leaving focus where it is.
pub fn move_to(window: &Window, screen: &str) {
    let Some((_, workspace)) = screens().into_iter().find(|(name, _)| name == screen) else {
        return;
    };
    let target = target(window);
    dispatch(
        &format!(
            "hl.dsp.window.move({{ monitor = \"{screen}\", window = \"{target}\", follow = false }})"
        ),
        &["movetoworkspacesilent", &format!("{workspace},{target}")],
    );
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
