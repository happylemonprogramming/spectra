//! Playing CD audio, from the drive or from a copy, and a game's music files.
//!
//! Red Book audio is already what a sound card wants: 44.1 kHz, 16-bit,
//! stereo, 588 frames to a 2352-byte sector. So there is nothing to decode;
//! the player reads a third of a second at a time and keeps about two
//! seconds queued for the sound card, which is enough to ride out the drive
//! re-seeking without racing a whole track into memory. A game's WAV files
//! are the same samples, at their own rate.
//!
//! The disc plays as one stream from the chosen track on, so tracks run into
//! each other without a gap, as they do on a CD player; which track is
//! playing follows from how far the sound card has got. Places in the
//! stream are counted in frames: on a CD, frame N is byte 4N of a copy.
//!
//! The player lives on a thread of its own, which owns the sound card's
//! stream (it may not leave the thread that made it) and what it reads from:
//! the drive, a kept copy's `.bin`, which holds the same raw sectors, or the
//! music files on a kept copy, end to end.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use iced::futures::channel::mpsc::UnboundedSender;
use spectra_core::Disc;
use spectra_core::drive::Drive;
use spectra_core::lock::Lock;
use spectra_core::soundtrack::MusicFile;

const CD_RATE: u32 = 44_100;
/// Stereo frames in a sector.
const FRAMES_PER_SECTOR: u32 = 588;
/// Frames per read: 27 sectors, about a third of a second.
const READ: u32 = 27 * FRAMES_PER_SECTOR;

/// Where the sound comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// The disc in this drive.
    Drive(PathBuf),
    /// A copy: every sector raw, sector N at N × 2352 bytes.
    Image(PathBuf),
    /// Music files on a copy, opened through its cue sheet, one after
    /// another. They share a rate.
    Files { cue: PathBuf, files: Vec<MusicFile> },
}

impl Source {
    fn rate(&self) -> u32 {
        match self {
            Self::Files { files, .. } => files.first().map_or(CD_RATE, |f| f.rate),
            _ => CD_RATE,
        }
    }
}

/// A source, opened. The drive is held for this window while it is, so
/// another Spectra window does not read it at the same time.
enum Reader {
    Drive {
        drive: Drive,
        _held: Lock,
    },
    Image(File),
    Files {
        disc: Box<dyn Disc>,
        /// Each file, and the frame it starts at.
        files: Vec<(u32, MusicFile)>,
    },
}

impl Reader {
    fn open(source: &Source) -> Result<Self, String> {
        match source {
            Source::Drive(path) => {
                let lock = crate::windows::drive_lock(path, "playing")?;
                Drive::open(path)
                    .map(|drive| Self::Drive { drive, _held: lock })
                    .map_err(|e| e.to_string())
            }
            Source::Image(path) => File::open(path).map(Self::Image).map_err(|e| e.to_string()),
            Source::Files { cue, files } => spectra_core::image::open(cue)
                .map(|disc| Self::Files {
                    disc,
                    files: file_spans(files)
                        .into_iter()
                        .map(|s| s.start)
                        .zip(files.iter().cloned())
                        .collect(),
                })
                .map_err(|e| e.to_string()),
        }
    }

    /// `count` frames from `at`, as 16-bit little-endian samples. A drive
    /// is given a couple more tries: a read that fails once often goes
    /// through when asked again. Its reads start and end on a sector, as
    /// every track does.
    fn read(&mut self, at: u32, count: u32) -> Result<Vec<u8>, String> {
        match self {
            Self::Drive { drive, .. } => {
                let (lba, sectors) = (at / FRAMES_PER_SECTOR, count / FRAMES_PER_SECTOR);
                let mut last = String::new();
                for _ in 0..3 {
                    match drive.read_raw(lba, sectors, true) {
                        Ok(data) => return Ok(data),
                        Err(e) if e.medium_not_present() => return Err(e.to_string()),
                        Err(e) => last = e.to_string(),
                    }
                }
                Err(last)
            }
            Self::Image(file) => {
                let mut data = vec![0; count as usize * 4];
                file.seek(SeekFrom::Start(u64::from(at) * 4))
                    .and_then(|_| file.read_exact(&mut data))
                    .map_err(|e| e.to_string())?;
                Ok(data)
            }
            Self::Files { disc, files } => {
                // A read may run from the end of one file into the next.
                let mut data = Vec::with_capacity(count as usize * 4);
                let (mut at, end) = (at, at + count);
                while at < end {
                    let Some((start, file)) = files
                        .iter()
                        .find(|(start, f)| (*start..start + f.frames()).contains(&at))
                    else {
                        return Err("read past the last file".into());
                    };
                    let frames = (end - at).min(start + file.frames() - at);
                    let samples = file
                        .read(disc.as_mut(), (at - start) * 4, frames * 4)
                        .map_err(|e| e.to_string())?;
                    data.extend(samples);
                    at += frames;
                }
                Ok(data)
            }
        }
    }
}

/// A track, as the frames it spans: on a CD, index 1 to the next track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Copy)]
enum Command {
    Play(usize),
    Pause,
    Resume,
    Stop,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Track `n` is playing, `seconds` in.
    At {
        track: usize,
        seconds: u32,
    },
    /// Played to the end of the last track, or stopped.
    Stopped,
    Failed(String),
}

/// A disc's player. Dropping it stops the sound.
pub struct Player {
    commands: mpsc::Sender<Command>,
}

impl Player {
    pub fn new(source: Source, spans: Vec<Span>, events: UnboundedSender<Event>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || run(source, spans, rx, events));
        Self { commands: tx }
    }

    pub fn play(&self, track: usize) {
        let _ = self.commands.send(Command::Play(track));
    }

    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    pub fn resume(&self) {
        let _ = self.commands.send(Command::Resume);
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }
}

/// What the sound card's callback and the player thread share.
#[derive(Default)]
struct Shared {
    queue: Mutex<VecDeque<i16>>,
    /// Frames the sound card has taken since the last Play.
    played: AtomicU64,
}

fn output(shared: Arc<Shared>, rate: u32) -> Result<cpal::Stream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no sound output")?;
    let config = cpal::StreamConfig {
        channels: 2,
        sample_rate: cpal::SampleRate(rate),
        buffer_size: cpal::BufferSize::Default,
    };
    device
        .build_output_stream(
            &config,
            move |out: &mut [f32], _| {
                let mut taken = 0;
                if let Ok(mut queue) = shared.queue.try_lock() {
                    taken = out.len().min(queue.len());
                    for (sample, value) in out.iter_mut().zip(queue.drain(..taken)) {
                        *sample = f32::from(value) / 32768.0;
                    }
                }
                // Run dry - the drive is slow to answer - and it is silence,
                // not a click, and the clock does not move.
                out[taken..].fill(0.0);
                shared.played.fetch_add(taken as u64 / 2, Ordering::Relaxed);
            },
            |e| eprintln!("spectra: sound: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Where the run of tracks starting at `track` ends: the last track, or the
/// first one followed by something that is not the next track - a data
/// track between audio ones, which read as audio would be noise.
fn run_end(spans: &[Span], track: usize) -> u32 {
    let mut end = spans[track].end;
    for span in &spans[track + 1..] {
        if span.start != end {
            break;
        }
        end = span.end;
    }
    end
}

/// Which track a frame is in, and how many seconds into it.
fn locate(spans: &[Span], at: u32, rate: u32) -> Option<(usize, u32)> {
    let track = spans.iter().rposition(|s| s.start <= at)?;
    Some((track, (at - spans[track].start) / rate))
}

fn run(
    source: Source,
    spans: Vec<Span>,
    commands: mpsc::Receiver<Command>,
    events: UnboundedSender<Event>,
) {
    let rate = source.rate();
    // Samples kept queued: two seconds.
    let ahead = 2 * rate as usize * 2;
    let shared = Arc::new(Shared::default());
    let mut stream: Option<cpal::Stream> = None;
    let mut reader: Option<Reader> = None;
    // Where playing started, where the next read goes, and where it stops.
    let mut end = 0;
    let mut origin = 0;
    let mut cursor = 0;
    let mut playing = false;
    let mut reported = None;
    let fail = |message: String| {
        let _ = events.unbounded_send(Event::Failed(message));
    };

    loop {
        // Wait for a command, briefly when there is reading to do.
        let wait = if playing {
            Duration::from_millis(30)
        } else {
            Duration::from_secs(3600)
        };
        match commands.recv_timeout(wait) {
            Ok(Command::Play(track)) => {
                let Some(span) = spans.get(track) else {
                    continue;
                };
                if reader.is_none() {
                    match Reader::open(&source) {
                        Ok(r) => reader = Some(r),
                        Err(e) => {
                            fail(e);
                            continue;
                        }
                    }
                }
                if stream.is_none() {
                    match output(shared.clone(), rate) {
                        Ok(s) => stream = Some(s),
                        Err(e) => {
                            fail(e);
                            continue;
                        }
                    }
                }
                shared.queue.lock().expect("queue").clear();
                shared.played.store(0, Ordering::Relaxed);
                (origin, cursor) = (span.start, span.start);
                end = run_end(&spans, track);
                reported = None;
                playing = true;
                if let Some(s) = &stream {
                    let _ = s.play();
                }
            }
            Ok(Command::Pause) => {
                if let Some(s) = &stream {
                    let _ = s.pause();
                }
                playing = false;
            }
            Ok(Command::Resume) => {
                if stream.is_some() && cursor < end {
                    if let Some(s) = &stream {
                        let _ = s.play();
                    }
                    playing = true;
                }
            }
            Ok(Command::Stop) => {
                // Let go of the sound card, so nothing holds it open, and of
                // the drive, for another window.
                stream = None;
                reader = None;
                shared.queue.lock().expect("queue").clear();
                playing = false;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if !playing {
            continue;
        }

        // Keep the queue topped up.
        let queued = shared.queue.lock().expect("queue").len();
        if queued < ahead && cursor < end {
            let count = READ.min(end - cursor);
            match reader.as_mut().expect("opened at play").read(cursor, count) {
                Ok(data) => {
                    let samples = data
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|&b| i16::from_le_bytes(b));
                    shared.queue.lock().expect("queue").extend(samples);
                    cursor += count;
                }
                Err(e) => {
                    stream = None;
                    reader = None;
                    playing = false;
                    fail(e);
                    continue;
                }
            }
        }

        // Say where the sound card has got to, once a second.
        let at = origin + shared.played.load(Ordering::Relaxed) as u32;
        if at >= end && queued == 0 {
            stream = None;
            reader = None;
            playing = false;
            let _ = events.unbounded_send(Event::Stopped);
            continue;
        }
        if let Some((track, seconds)) = locate(&spans, at, rate)
            && reported != Some((track, seconds))
        {
            reported = Some((track, seconds));
            if events.unbounded_send(Event::At { track, seconds }).is_err() {
                return;
            }
        }
    }
}

/// The spans of a disc's audio tracks, in track-list order.
pub fn spans(toc: &spectra_core::Toc) -> Vec<Span> {
    toc.tracks
        .iter()
        .zip(toc.track_sectors())
        .filter(|(t, _)| !t.data)
        .map(|(t, sectors)| Span {
            start: t.lba * FRAMES_PER_SECTOR,
            end: (t.lba + sectors) * FRAMES_PER_SECTOR,
        })
        .collect()
}

/// The spans of music files played end to end, in order.
pub fn file_spans(files: &[MusicFile]) -> Vec<Span> {
    let mut start = 0;
    files
        .iter()
        .map(|f| {
            let span = Span {
                start,
                end: start + f.frames(),
            };
            start = span.end;
            span
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW_SECTOR: usize = spectra_core::disc::RAW_SECTOR;

    #[test]
    fn a_frame_is_placed_in_its_track() {
        let spans = [
            Span {
                start: 0,
                end: 441_000,
            },
            Span {
                start: 441_000,
                end: 882_000,
            },
        ];
        assert_eq!(locate(&spans, 0, CD_RATE), Some((0, 0)));
        assert_eq!(locate(&spans, 440_999, CD_RATE), Some((0, 9)));
        assert_eq!(locate(&spans, 529_200, CD_RATE), Some((1, 2)));
    }

    #[test]
    fn a_copy_reads_by_frame() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("disc.bin");
        let sectors: Vec<u8> = (0..4u8).flat_map(|n| [n; RAW_SECTOR]).collect();
        std::fs::write(&bin, sectors).unwrap();
        let mut reader = Reader::open(&Source::Image(bin)).unwrap();
        let sector = FRAMES_PER_SECTOR;
        let data = reader.read(2 * sector, 2 * sector).unwrap();
        assert_eq!(data.len(), 2 * RAW_SECTOR);
        assert_eq!((data[0], data[RAW_SECTOR]), (2, 3));
        assert!(
            reader.read(3 * sector, 2 * sector).is_err(),
            "read past the end"
        );
    }

    #[test]
    fn a_run_stops_short_of_a_data_track() {
        let spans = [
            Span { start: 0, end: 750 },
            Span {
                start: 750,
                end: 1500,
            },
            // A data track sat in 1500..9000.
            Span {
                start: 9000,
                end: 9750,
            },
        ];
        assert_eq!(run_end(&spans, 0), 1500);
        assert_eq!(run_end(&spans, 2), 9750);
    }

    #[test]
    fn music_files_play_end_to_end() {
        let file = |bytes| MusicFile {
            name: String::new(),
            lba: 0,
            offset: 44,
            bytes,
            rate: 48_000,
        };
        let spans = file_spans(&[file(400), file(800)]);
        assert_eq!(
            spans,
            [
                Span { start: 0, end: 100 },
                Span {
                    start: 100,
                    end: 300
                }
            ]
        );
        assert_eq!(run_end(&spans, 0), 300);
    }

    #[test]
    fn a_read_runs_on_from_one_file_into_the_next() {
        // Two files of 3000 frames, the first byte of each frame its file's
        // number, after a 44-byte header, in a cooked image.
        let dir = tempfile::tempdir().unwrap();
        let sector = spectra_core::SECTOR;
        let mut image = vec![0u8; 16 * sector];
        for (n, lba) in [(1u8, 0), (2, 8)] {
            for frame in 0..3000 {
                image[lba * sector + 44 + frame * 4] = n;
            }
        }
        std::fs::write(dir.path().join("disc.iso"), image).unwrap();
        let cue = dir.path().join("disc.cue");
        std::fs::write(
            &cue,
            "FILE \"disc.iso\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        let file = |lba| MusicFile {
            name: String::new(),
            lba,
            offset: 44,
            bytes: 12_000,
            rate: 48_000,
        };
        let source = Source::Files {
            cue,
            files: vec![file(0), file(8)],
        };
        let mut reader = Reader::open(&source).unwrap();
        let data = reader.read(2990, 20).unwrap();
        let firsts: Vec<u8> = data.chunks(4).map(|f| f[0]).collect();
        assert_eq!(firsts, [[1; 10], [2; 10]].concat());
        assert!(reader.read(5990, 20).is_err(), "read past the last file");
    }
}
