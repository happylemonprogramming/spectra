//! The Spectra shell: a window, and the commands the UI calls into the core
//! through.

use std::path::PathBuf;
use std::time::Duration;

use spectra_core::drive::{self, Drive, DriveInfo};
use spectra_core::{Disc, Report, identify, image};

/// How long to wait for a disc that has just gone in to spin up.
const SPIN_UP: Duration = Duration::from_secs(25);

#[tauri::command]
fn list_drives() -> Vec<DriveInfo> {
    drive::list()
}

/// Identify the disc in a drive (`/dev/...`), in an image, or in the first
/// drive when no source is given. Drive commands block, so this runs off the
/// main thread.
#[tauri::command]
async fn identify_disc(source: Option<String>) -> Result<Report, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut disc: Box<dyn Disc> = match source.map(PathBuf::from) {
            Some(path) if !path.starts_with("/dev") => image::open(&path)?,
            Some(path) => Box::new(ready(Drive::open(&path)?)?),
            None => Box::new(ready(Drive::open_first()?)?),
        };
        identify(disc.as_mut())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| {
        if e.medium_not_present() {
            "No disc in the drive.".into()
        } else {
            e.to_string()
        }
    })
}

fn ready(drive: Drive) -> spectra_core::Result<Drive> {
    drive.wait_until_ready(SPIN_UP)?;
    Ok(drive)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list_drives, identify_disc])
        .run(tauri::generate_context!())
        .expect("error while running Spectra");
}
