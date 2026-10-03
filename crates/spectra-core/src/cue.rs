//! CD images described by a cue sheet: one `.bin` per track, as Redump dumps
//! them, or one for the whole disc.
//!
//! Binary files hold raw 2352-byte sectors (or, rarely, cooked 2048-byte
//! ones). A data sector's 2048 bytes of user data sit behind a 16-byte header
//! in mode 1, and behind 8 more bytes of XA subheader in mode 2 form 1 - which
//! is what PlayStation and Saturn discs use.
//!
//! The table of contents is laid out the way a drive would report it, so the
//! same disc gives the same MusicBrainz ID from an image as from the drive. A
//! second session (an enhanced CD's data) starts 11400 sectors after the first
//! ends; images do not store that gap, so it is put back here.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::disc::{Disc, Media, RAW_SECTOR, SECTOR, SESSION_GAP, Toc, Track};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Audio,
    /// Raw 2352-byte sectors.
    Mode1Raw,
    Mode2Raw,
    /// Cooked 2048-byte sectors.
    Cooked,
}

impl Mode {
    fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_uppercase().as_str() {
            "AUDIO" => Ok(Self::Audio),
            "MODE1/2352" => Ok(Self::Mode1Raw),
            "MODE2/2352" => Ok(Self::Mode2Raw),
            "MODE1/2048" | "MODE2/2048" => Ok(Self::Cooked),
            other => Err(Error::Unsupported(format!("cue track mode {other}"))),
        }
    }

    fn sector_size(self) -> usize {
        if self == Self::Cooked {
            SECTOR
        } else {
            RAW_SECTOR
        }
    }
}

#[derive(Debug, Clone)]
struct CueTrack {
    number: u8,
    mode: Mode,
    file: usize,
    /// Index 1, in sectors from the start of its file.
    file_sector: u32,
    lba: u32,
}

#[derive(Debug)]
struct BinFile {
    path: PathBuf,
    handle: Option<File>,
    sectors: u32,
}

pub struct CueDisc {
    cue: PathBuf,
    files: Vec<BinFile>,
    tracks: Vec<CueTrack>,
    leadout: u32,
}

/// `mm:ss:ff` to sectors.
fn msf(s: &str) -> Result<u32> {
    let parts: Vec<u32> = s
        .split(':')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| Error::format(format!("bad cue time {s}")))?;
    match parts[..] {
        [m, s, f] => Ok((m * 60 + s) * 75 + f),
        _ => Err(Error::format(format!("bad cue time {s}"))),
    }
}

/// A quoted or bare file name after `FILE`, without the trailing type.
fn file_name(rest: &str) -> String {
    let rest = rest.trim();
    if let Some(quoted) = rest.strip_prefix('"') {
        quoted.split('"').next().unwrap_or_default().to_string()
    } else {
        rest.rsplit_once(' ')
            .map_or(rest, |(name, _)| name)
            .to_string()
    }
}

impl CueDisc {
    pub fn open(cue: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(cue)?;
        let dir = cue.parent().unwrap_or(Path::new("."));

        struct Pending {
            number: u8,
            mode: Mode,
            file: usize,
            index1: Option<u32>,
            session: u32,
        }
        let mut files: Vec<(PathBuf, Mode)> = Vec::new();
        let mut pending: Vec<Pending> = Vec::new();
        let mut session = 1;

        for line in text.lines() {
            let line = line.trim();
            let (keyword, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            match keyword.to_ascii_uppercase().as_str() {
                "FILE" => files.push((dir.join(file_name(rest)), Mode::Audio)),
                "TRACK" => {
                    let mut words = rest.split_whitespace();
                    let number = words
                        .next()
                        .and_then(|n| n.parse().ok())
                        .ok_or_else(|| Error::format(format!("bad cue line: {line}")))?;
                    let mode = Mode::parse(words.next().unwrap_or_default())?;
                    let file = files
                        .len()
                        .checked_sub(1)
                        .ok_or_else(|| Error::format("TRACK before FILE in cue sheet"))?;
                    if !pending.iter().any(|t| t.file == file) {
                        files[file].1 = mode;
                    }
                    pending.push(Pending {
                        number,
                        mode,
                        file,
                        index1: None,
                        session,
                    });
                }
                "INDEX" => {
                    let mut words = rest.split_whitespace();
                    if words.next().and_then(|n| n.parse::<u8>().ok()) == Some(1) {
                        let track = pending
                            .last_mut()
                            .ok_or_else(|| Error::format("INDEX before TRACK in cue sheet"))?;
                        track.index1 = Some(msf(words.next().unwrap_or_default())?);
                    }
                }
                "REM" => {
                    let mut words = rest.split_whitespace();
                    if words
                        .next()
                        .is_some_and(|w| w.eq_ignore_ascii_case("SESSION"))
                    {
                        session = words.next().and_then(|n| n.parse().ok()).unwrap_or(session);
                    }
                }
                _ => {}
            }
        }
        if pending.is_empty() {
            return Err(Error::format("the cue sheet has no tracks"));
        }

        let mut bins = Vec::new();
        let mut starts = Vec::new();
        let mut next = 0u32;
        for (path, mode) in files {
            let size = std::fs::metadata(&path)
                .map_err(|e| Error::format(format!("{}: {e}", path.display())))?
                .len();
            let sectors = (size / mode.sector_size() as u64) as u32;
            starts.push(next);
            next += sectors;
            bins.push(BinFile {
                path,
                handle: None,
                sectors,
            });
        }

        let tracks: Vec<CueTrack> = pending
            .into_iter()
            .map(|t| {
                let file_sector = t.index1.unwrap_or(0);
                let gap = SESSION_GAP * (t.session.max(1) - 1);
                CueTrack {
                    number: t.number,
                    mode: t.mode,
                    file: t.file,
                    file_sector,
                    lba: starts[t.file] + file_sector + gap,
                }
            })
            .collect();
        let last = tracks.last().expect("checked above");
        let leadout = last.lba - last.file_sector - starts[last.file] + next;
        Ok(Self {
            cue: cue.to_path_buf(),
            files: bins,
            tracks,
            leadout,
        })
    }

    fn track_at(&self, lba: u32) -> Option<&CueTrack> {
        self.tracks.iter().rev().find(|t| t.lba <= lba)
    }

    fn read_sector(&mut self, lba: u32) -> Result<Vec<u8>> {
        let track = self
            .track_at(lba)
            .cloned()
            .ok_or_else(|| Error::format(format!("sector {lba} is before track 1")))?;
        if lba >= self.leadout {
            return Err(Error::format(format!(
                "sector {lba} is past the end of the disc"
            )));
        }
        let within = track.file_sector + (lba - track.lba);
        let file = &mut self.files[track.file];
        if within >= file.sectors {
            return Err(Error::format(format!(
                "sector {lba} is past the end of {}",
                file.path.display()
            )));
        }
        let size = track.mode.sector_size();
        let handle = match &mut file.handle {
            Some(h) => h,
            None => file.handle.insert(File::open(&file.path)?),
        };
        let mut raw = vec![0; size];
        handle.seek(SeekFrom::Start(u64::from(within) * size as u64))?;
        handle.read_exact(&mut raw)?;

        let user = match track.mode {
            Mode::Audio => return Err(Error::format(format!("sector {lba} is audio, not data"))),
            Mode::Cooked => 0..SECTOR,
            Mode::Mode1Raw => 16..16 + SECTOR,
            // Mode 2 form 1: an XA subheader, written twice, before the data.
            Mode::Mode2Raw => 24..24 + SECTOR,
        };
        Ok(raw[user].to_vec())
    }
}

impl Disc for CueDisc {
    fn describe(&self) -> String {
        self.cue.display().to_string()
    }

    fn media(&mut self) -> Result<Media> {
        Ok(Media::Cd)
    }

    fn toc(&mut self) -> Result<Option<Toc>> {
        Ok(Some(Toc {
            first: self.tracks[0].number,
            last: self.tracks.last().map_or(1, |t| t.number),
            tracks: self
                .tracks
                .iter()
                .map(|t| Track {
                    number: t.number,
                    lba: t.lba,
                    data: t.mode != Mode::Audio,
                })
                .collect(),
            leadout: self.leadout,
        }))
    }

    fn read(&mut self, lba: u32, count: u32) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(count as usize * SECTOR);
        for i in 0..count {
            out.extend(self.read_sector(lba + i)?);
        }
        Ok(out)
    }

    fn capacity(&mut self) -> Result<Option<u32>> {
        Ok(Some(self.leadout))
    }
}
