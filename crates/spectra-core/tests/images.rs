//! Identification from real files: cue sheets with raw bins, bare bins and
//! ISOs, laid out the way Redump and the usual rippers write them.

use std::fs;
use std::path::Path;

use spectra_core::disc::{RAW_SECTOR, SECTOR};
use spectra_core::{DiscKind, GameSystem, identify, image};

/// A cooked ISO 9660 image holding one SYSTEM.CNF: PVD at 16, root at 18,
/// the file at 20.
fn ps1_iso() -> Vec<u8> {
    let mut iso = vec![0u8; 32 * SECTOR];
    let cnf = b"BOOT = cdrom:\\SCES_000.01;1\n";
    let sector = |n: usize| n * SECTOR..(n + 1) * SECTOR;

    let pvd = sector(16);
    iso[pvd.start] = 1;
    iso[pvd.start + 1..pvd.start + 6].copy_from_slice(b"CD001");
    let root = record(18, SECTOR as u32, true, &[0]);
    iso[pvd.start + 156..pvd.start + 156 + root.len()].copy_from_slice(&root);

    let mut dir = record(18, SECTOR as u32, true, &[0]);
    dir.extend(record(18, SECTOR as u32, true, &[1]));
    dir.extend(record(20, cnf.len() as u32, false, b"SYSTEM.CNF;1"));
    let at = sector(18).start;
    iso[at..at + dir.len()].copy_from_slice(&dir);
    let at = sector(20).start;
    iso[at..at + cnf.len()].copy_from_slice(cnf);
    iso
}

fn record(lba: u32, size: u32, directory: bool, name: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; 33];
    r[2..6].copy_from_slice(&lba.to_le_bytes());
    r[10..14].copy_from_slice(&size.to_le_bytes());
    r[25] = if directory { 2 } else { 0 };
    r[32] = name.len() as u8;
    r.extend(name);
    if r.len() % 2 == 1 {
        r.push(0);
    }
    r[0] = r.len() as u8;
    r
}

/// Cooked sectors wrapped as raw mode 2 form 1: sync, header, XA subheader.
fn raw_mode2(cooked: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for sector in cooked.chunks(SECTOR) {
        let mut raw = vec![0u8; RAW_SECTOR];
        raw[1..11].fill(0xff);
        raw[15] = 2;
        raw[24..24 + SECTOR].copy_from_slice(sector);
        out.extend(raw);
    }
    out
}

fn ps1_system(path: &Path) -> Option<GameSystem> {
    let mut disc = image::open(path).unwrap();
    match identify(disc.as_mut()).unwrap().kind {
        DiscKind::Game(g) => Some(g.system),
        _ => None,
    }
}

#[test]
fn cue_with_a_bin_per_track() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("game (Track 1).bin"), raw_mode2(&ps1_iso())).unwrap();
    fs::write(
        dir.path().join("game (Track 2).bin"),
        vec![0u8; 300 * RAW_SECTOR],
    )
    .unwrap();
    let cue = dir.path().join("game.cue");
    fs::write(
        &cue,
        "FILE \"game (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n\
         FILE \"game (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 00 00:00:00\n    INDEX 01 00:02:00\n",
    )
    .unwrap();

    let mut disc = image::open(&cue).unwrap();
    let report = identify(disc.as_mut()).unwrap();
    let DiscKind::Game(game) = report.kind else {
        panic!("{:?}", report.kind)
    };
    assert_eq!(
        (game.system, game.serial.as_deref()),
        (GameSystem::Ps1, Some("SCES-00001"))
    );

    let toc = report.toc.unwrap();
    assert_eq!(
        toc.tracks
            .iter()
            .map(|t| (t.lba, t.data))
            .collect::<Vec<_>>(),
        [(0, true), (32 + 150, false)]
    );
    assert_eq!(toc.leadout, 32 + 300);
}

#[test]
fn enhanced_cd_cue_matches_the_drive() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.bin"), vec![0u8; 20_000 * RAW_SECTOR]).unwrap();
    fs::write(dir.path().join("d.bin"), raw_mode2(&ps1_iso())).unwrap();
    let cue = dir.path().join("cd.cue");
    fs::write(
        &cue,
        "REM SESSION 01\nFILE \"a.bin\" BINARY\n  TRACK 01 AUDIO\n    INDEX 01 00:00:00\n\
         TRACK 02 AUDIO\n    INDEX 01 02:00:00\n\
         REM SESSION 02\nFILE \"d.bin\" BINARY\n  TRACK 03 MODE2/2352\n    INDEX 01 00:00:00\n",
    )
    .unwrap();

    let mut disc = image::open(&cue).unwrap();
    let toc = disc.toc().unwrap().unwrap();
    // The data session starts 11400 sectors after the audio one ends, as a
    // drive would report it, so the disc ID sees the audio session alone.
    assert_eq!(toc.tracks[2].lba, 20_000 + 11_400);
    let DiscKind::Audio {
        tracks,
        enhanced,
        musicbrainz: id,
    } = identify(disc.as_mut()).unwrap().kind
    else {
        panic!()
    };
    assert_eq!((tracks, enhanced), (2, true));
    assert_eq!(id.unwrap().toc, "1+2+20150+150+9150");
}

#[test]
fn bare_raw_bin_and_iso() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("game.bin");
    fs::write(&bin, raw_mode2(&ps1_iso())).unwrap();
    assert_eq!(ps1_system(&bin), Some(GameSystem::Ps1));

    let iso = dir.path().join("game.iso");
    fs::write(&iso, ps1_iso()).unwrap();
    assert_eq!(ps1_system(&iso), Some(GameSystem::Ps1));
}
