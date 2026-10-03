//! The drive, watched: plugged in or not, a disc in it or not, and what the
//! disc is.
//!
//! A thread asks the drive every two seconds whether a disc is in it, with
//! GET EVENT/STATUS NOTIFICATION - the same question the kernel polls with,
//! and one that never spins the disc up. Only a change is read further, and
//! only a change is reported, so an idle drive costs next to nothing.

use std::path::PathBuf;
use std::time::Duration;

use iced::Subscription;
use iced::futures::SinkExt;
use spectra_core::drive::Drive;
use spectra_core::{Error, Report, identify};

const POLL: Duration = Duration::from_secs(2);
/// A slim USB drive can take seven seconds or more to read a new disc.
const SPIN_UP: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub enum DriveState {
    NoDrive,
    Empty,
    /// A disc went in, and is spinning up or being read.
    Reading,
    Disc {
        report: Box<Report>,
        /// The node it was read through: `/dev/sgN`, or `/dev/srN` without
        /// the sg module.
        drive: PathBuf,
    },
    /// A disc is in, but reading it failed.
    Unreadable(String),
}

/// What a poll saw, before anything is read.
#[derive(Clone, Copy, PartialEq)]
enum Seen {
    NoDrive,
    Empty,
    Disc,
}

pub fn subscription() -> Subscription<DriveState> {
    Subscription::run(|| {
        iced::stream::channel(4, async |mut output| {
            let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
            std::thread::spawn(move || {
                let mut drive: Option<Drive> = None;
                let mut last: Option<Seen> = None;
                // Until the window that wanted this is gone.
                while !tx.is_closed() {
                    if drive.is_none() {
                        drive = Drive::open_first().ok();
                    }
                    let (seen, changed) = match drive.as_ref().map(look) {
                        Some(Ok(look)) => look,
                        // Unplugged: the open node only answers with errors.
                        Some(Err(())) => {
                            drive = None;
                            (Seen::NoDrive, false)
                        }
                        None => (Seen::NoDrive, false),
                    };
                    if last != Some(seen) || changed {
                        last = Some(seen);
                        let state = match (seen, drive.as_mut()) {
                            (Seen::Disc, Some(drive)) => {
                                if tx.unbounded_send(DriveState::Reading).is_err() {
                                    return;
                                }
                                read(drive)
                            }
                            (Seen::Empty, _) => DriveState::Empty,
                            _ => DriveState::NoDrive,
                        };
                        if let DriveState::Empty = state {
                            last = Some(Seen::Empty);
                        }
                        if tx.unbounded_send(state).is_err() {
                            return;
                        }
                    }
                    std::thread::sleep(POLL);
                }
            });
            use iced::futures::StreamExt;
            while let Some(state) = rx.next().await {
                if output.send(state).await.is_err() {
                    break;
                }
            }
        })
    })
}

/// Whether a disc is in, and whether the drive says it was swapped since the
/// last look. Err when the drive no longer answers at all.
fn look(drive: &Drive) -> Result<(Seen, bool), ()> {
    match drive.media_status() {
        Ok(status) => Ok((
            if status.present {
                Seen::Disc
            } else {
                Seen::Empty
            },
            matches!(status.event, 2 | 4),
        )),
        // A drive without media events still answers TEST UNIT READY.
        Err(Error::Scsi(_)) => match drive.test_unit_ready() {
            Ok(()) => Ok((Seen::Disc, false)),
            Err(e) if e.medium_not_present() => Ok((Seen::Empty, false)),
            Err(Error::Scsi(_)) => Ok((Seen::Disc, false)),
            Err(_) => Err(()),
        },
        Err(_) => Err(()),
    }
}

fn read(drive: &mut Drive) -> DriveState {
    let result = drive
        .wait_until_ready(SPIN_UP)
        .and_then(|()| identify(drive));
    match result {
        Ok(report) => DriveState::Disc {
            report: Box::new(report),
            drive: drive.path().to_path_buf(),
        },
        // Taken out again while it was being read.
        Err(e) if e.medium_not_present() => DriveState::Empty,
        Err(e) => DriveState::Unreadable(e.to_string()),
    }
}
