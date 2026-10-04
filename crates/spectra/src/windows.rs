//! More than one Spectra window at once: a film in each, say.
//!
//! Each window takes commands on a socket of its own, named by its process
//! ID. One window is the current one - the last one focused, or the one
//! whose film was, or the last one to start something - and only it hears
//! the gamepad, which every program hears wherever focus is, and only it
//! answers a command that names no window.
//!
//! The drive is one drive: a window that reads it - for a film, a game, a CD
//! or a copy - holds a lock on it, and another window is told so rather than
//! fighting it for reads.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use spectra_core::lock::{self, Lock};

/// `$XDG_RUNTIME_DIR`, or a folder of the user's own in the temporary one.
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    // SAFETY: getuid cannot fail.
    let dir = std::env::temp_dir().join(format!("spectra-{}", unsafe { libc::getuid() }));
    let _ = std::fs::DirBuilder::new().mode(0o700).create(&dir);
    dir
}

/// Where the window with this process ID takes commands.
pub fn socket(pid: u32) -> PathBuf {
    runtime_dir().join(format!("spectra-{pid}.sock"))
}

fn current_file() -> PathBuf {
    runtime_dir().join("spectra-current")
}

/// Make this window the current one.
pub fn claim() {
    let file = current_file();
    let part = file.with_extension("part");
    let written = std::fs::File::create(&part)
        .and_then(|mut f| write!(f, "{}", std::process::id()))
        .and_then(|()| std::fs::rename(&part, &file));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
}

/// The current window, if it is still open.
pub fn current() -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(current_file())
        .ok()?
        .trim()
        .parse()
        .ok()?;
    alive(pid).then_some(pid)
}

/// Whether this window is the one the gamepad is for: it is current, or no
/// other open window is.
pub fn is_current() -> bool {
    current().is_none_or(|pid| pid == std::process::id())
}

/// A Spectra with this process ID is running.
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|name| name.trim() == "spectra")
}

/// Every window taking commands, oldest first. Sockets left by windows that
/// have gone are cleared away.
pub fn running() -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(runtime_dir()) else {
        return Vec::new();
    };
    let mut windows: Vec<(std::time::SystemTime, u32)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let pid = name
                .to_str()?
                .strip_prefix("spectra-")?
                .strip_suffix(".sock")?
                .parse()
                .ok()?;
            if UnixStream::connect(entry.path()).is_err() {
                let _ = std::fs::remove_file(entry.path());
                return None;
            }
            let started = entry.metadata().and_then(|m| m.modified()).ok()?;
            Some((started, pid))
        })
        .collect();
    windows.sort();
    windows.into_iter().map(|(_, pid)| pid).collect()
}

/// Hold the drive for this window, saying what for. Refused, in words, when
/// another window holds it.
pub fn drive_lock(drive: &Path, doing: &str) -> Result<Lock, String> {
    // One lock per drive, however it is reached: /dev/sgN and /dev/srN are
    // the same drive.
    let node = crate::film::block_node(drive).unwrap_or_else(|| drive.to_path_buf());
    let name = node
        .file_name()
        .map_or_else(|| "drive".into(), |n| n.to_string_lossy().into_owned());
    let path = runtime_dir().join(format!("spectra-drive-{name}.lock"));
    let held = |path: &Path| match lock::holder(path) {
        Some(doing) => format!("The disc is {doing} in another Spectra window"),
        None => "The disc is in use in another Spectra window".into(),
    };
    match lock::take(&path, doing) {
        Ok(Some(lock)) => {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            Ok(lock)
        }
        Ok(None) => Err(held(&path)),
        Err(e) => Err(format!("Couldn't take the drive: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drive_held_by_one_window_is_refused_to_another_with_why() {
        // A drive that is not there: its own node names the lock.
        let drive = PathBuf::from(format!("/dev/sr-test-{}", std::process::id()));
        let held = drive_lock(&drive, "being kept").expect("free at first");
        let refused = drive_lock(&drive, "playing").unwrap_err();
        assert_eq!(refused, "The disc is being kept in another Spectra window");
        drop(held);
        let again = drive_lock(&drive, "playing").expect("free once let go");
        let _ = std::fs::remove_file(again.path());
    }
}
