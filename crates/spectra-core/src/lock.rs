//! Saying "this is in use" between Spectra windows, so that two of them do
//! not read one drive, or write one copy, at the same time.
//!
//! A lock is an `flock` on a small file. The system lets it go when the
//! process does, crashes included, so a lock is never left stuck. The file
//! says what the holder is doing, for the window that is turned away.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

/// Held until dropped.
#[derive(Debug)]
pub struct Lock {
    _file: File,
    path: PathBuf,
}

impl Lock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Take the lock at `path`, saying what it is for. `Ok(None)` when another
/// process holds it.
pub fn take(path: &Path, doing: &str) -> std::io::Result<Option<Lock>> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    // SAFETY: flock on a descriptor this function owns.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let e = std::io::Error::last_os_error();
        return match e.raw_os_error() {
            Some(libc::EWOULDBLOCK) => Ok(None),
            _ => Err(e),
        };
    }
    file.set_len(0)?;
    file.write_all(doing.as_bytes())?;
    Ok(Some(Lock {
        _file: file,
        path: path.to_path_buf(),
    }))
}

/// What the holder of the lock at `path` said it was doing.
pub fn holder(path: &Path) -> Option<String> {
    let mut doing = String::new();
    File::open(path).ok()?.read_to_string(&mut doing).ok()?;
    let doing = doing.trim();
    (!doing.is_empty()).then(|| doing.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_turns_others_away_until_it_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drive.lock");
        let held = take(&path, "playing a film")
            .unwrap()
            .expect("free at first");
        // flock is per open file, so a second open in this process stands in
        // for another window.
        assert!(take(&path, "keeping a copy").unwrap().is_none());
        assert_eq!(holder(&path).as_deref(), Some("playing a film"));
        drop(held);
        let again = take(&path, "keeping a copy").unwrap();
        assert!(again.is_some());
        assert_eq!(holder(&path).as_deref(), Some("keeping a copy"));
    }
}
