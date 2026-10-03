//! Say what disc is in the drive, or in an image.
//!
//!   spectra-discid                 the first optical drive
//!   spectra-discid /dev/sr0        a particular drive
//!   spectra-discid game.cue        an image (.cue, .bin, .iso)
//!   spectra-discid --list          the drives the kernel knows about
//!   spectra-discid --keep          copy the drive's disc into the library

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use clap::Parser;
use spectra_core::drive::{self, Drive};
use spectra_core::{Disc, DiscKind, Error, Media, Report, SECTOR, identify, image, library};

#[derive(Parser)]
#[command(version, about = "Say what disc is in the drive, or in an image")]
struct Args {
    /// A drive (/dev/srN, /dev/sgN) or a disc image. Defaults to the first drive.
    source: Option<PathBuf>,
    /// Print JSON instead of text.
    #[arg(long)]
    json: bool,
    /// List optical drives and exit.
    #[arg(long)]
    list: bool,
    /// Copy the disc in the drive into Spectra's library, to play without it.
    #[arg(long)]
    keep: bool,
    /// Seconds to wait for a disc that is still spinning up.
    #[arg(long, default_value_t = 25)]
    wait: u64,
}

fn main() -> ExitCode {
    let args = Args::parse();
    if args.list {
        return list(args.json);
    }
    if args.keep {
        return keep(&args);
    }
    match run(&args) {
        Ok(report) => {
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).expect("reports serialise")
                );
            } else {
                print_report(&report);
            }
            ExitCode::SUCCESS
        }
        Err(e) if e.medium_not_present() => {
            eprintln!("no disc in the drive");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("spectra-discid: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<Report, Error> {
    let mut disc: Box<dyn Disc> = match &args.source {
        Some(path) if !path.starts_with("/dev") => image::open(path)?,
        Some(path) => Box::new(open_drive(Drive::open(path)?, args.wait)?),
        None => Box::new(open_drive(Drive::open_first()?, args.wait)?),
    };
    identify(disc.as_mut())
}

fn keep(args: &Args) -> ExitCode {
    let result = (|| {
        let drive = match &args.source {
            Some(path) => Drive::open(path)?,
            None => Drive::open_first()?,
        };
        let mut drive = open_drive(drive, args.wait)?;
        let report = identify(&mut drive)?;
        let started = std::time::Instant::now();
        let mut last = 0;
        library::keep(&drive, &report, &AtomicBool::new(false), |p| {
            let percent = u64::from(p.done) * 100 / u64::from(p.total.max(1));
            if percent != last {
                last = percent;
                eprint!("\rcopying: {percent}%");
            }
        })
        .map(|entry| (entry, started.elapsed()))
    })();
    match result {
        Ok((entry, took)) => {
            eprintln!();
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&entry.meta).expect("meta serialises")
                );
            } else {
                println!("Kept:    {}", entry.meta.title);
                println!("Where:   {}", entry.dir.display());
                println!("Took:    {} s", took.as_secs());
                if entry.meta.unreadable > 0 {
                    println!("Missing: {} unreadable sectors", entry.meta.unreadable);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("\nspectra-discid: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open_drive(drive: Drive, wait: u64) -> Result<Drive, Error> {
    drive.wait_until_ready(Duration::from_secs(wait))?;
    Ok(drive)
}

fn list(json: bool) -> ExitCode {
    let drives = drive::list();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&drives).expect("drives serialise")
        );
    } else if drives.is_empty() {
        println!("No optical drives found.");
    } else {
        for d in &drives {
            let node =
                |p: &Option<PathBuf>| p.as_ref().map_or("-".into(), |p| p.display().to_string());
            println!(
                "{}  {}  {} {} {}",
                node(&d.block),
                node(&d.generic),
                d.vendor,
                d.model,
                d.revision
            );
        }
        if drives.iter().any(|d| d.generic.is_none()) {
            println!("\nNo /dev/sg node: load the sg module (sudo modprobe sg) for raw access.");
        }
    }
    ExitCode::SUCCESS
}

fn media_name(media: Media) -> &'static str {
    match media {
        Media::Cd => "CD",
        Media::Dvd => "DVD",
        Media::BluRay => "Blu-ray",
        Media::HdDvd => "HD DVD",
        Media::Unknown => "unknown",
    }
}

fn print_report(r: &Report) {
    let row = |name: &str, value: &str| println!("{:<9}{value}", format!("{name}:"));
    row("Source", &r.source);
    row("Media", media_name(r.media));

    let plan = match &r.kind {
        DiscKind::Audio {
            tracks,
            enhanced,
            musicbrainz,
        } => {
            let extra = if *enhanced {
                " (enhanced CD: data track left out)"
            } else {
                ""
            };
            row("Disc", &format!("Audio CD, {tracks} tracks{extra}"));
            if let Some(mb) = musicbrainz {
                row("DiscID", &mb.disc_id);
                row(
                    "Lookup",
                    &format!("https://musicbrainz.org/cdtoc/{}", mb.disc_id),
                );
            }
            Some("Phase 1, Rainbow Player's audio path")
        }
        DiscKind::DvdVideo => {
            row("Disc", "DVD-Video");
            Some("Phase 1, libmpv")
        }
        DiscKind::BluRayVideo => {
            row("Disc", "Blu-ray video");
            Some("Phase 1, libmpv (needs a libaacs key database)")
        }
        DiscKind::VideoCd { super_vcd } => {
            row(
                "Disc",
                if *super_vcd {
                    "Super Video CD"
                } else {
                    "Video CD"
                },
            );
            Some("Phase 2, libmpv")
        }
        DiscKind::Game(g) => {
            let name = g.title.as_deref().unwrap_or("unknown title");
            let details: Vec<&str> = [g.serial.as_deref(), g.region.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            let details = if details.is_empty() {
                String::new()
            } else {
                format!(" ({})", details.join(", "))
            };
            row(
                "Disc",
                &format!("{} game: {name}{details}", g.system.name()),
            );
            if let Some(boot) = &g.boot {
                row("Boot", boot);
            }
            match g.system.plan() {
                Some((phase, engine)) => row("Spectra", &format!("Phase {phase}, {engine}")),
                None => row("Spectra", "out of scope (encrypted, or no usable emulator)"),
            }
            None
        }
        DiscKind::Pc => {
            row("Disc", "PC disc");
            Some("Phase 4")
        }
        DiscKind::Data => {
            row("Disc", "Data disc (not a format Spectra plays)");
            None
        }
    };
    if let Some(label) = &r.label {
        row("Label", label);
    }
    if let Some(sectors) = r
        .sectors
        .filter(|_| !matches!(r.kind, DiscKind::Audio { .. }))
    {
        let gb = f64::from(sectors) * SECTOR as f64 / 1e9;
        row("Size", &format!("{sectors} sectors ({gb:.2} GB)"));
    }
    if let Some(plan) = plan {
        row("Spectra", plan);
    }
}
