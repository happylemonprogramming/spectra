//! Playing CD audio, from the drive or from a copy.
//!
//! Red Book audio is already what a sound card wants: 44.1 kHz, 16-bit,
//! stereo, 588 frames to a 2352-byte sector. So there is nothing to decode;
//! the player reads sectors a third of a second at a time and keeps about two
//! seconds queued for the sound card, which is enough to ride out the drive
//! re-seeking without racing a whole track into memory.
//!
//! The disc plays as one stream from the chosen track on, so tracks run into
//! each other without a gap, as they do on a CD player; which track is
//! playing follows from how far the sound card has got.
//!
//! The player lives on a thread of its own, which owns the sound card's
//! stream (it may not leave the thread that made it) and the drive. A kept
//! game's soundtrack, read from its copy, is the next source to add.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use iced::futures::channel::mpsc::UnboundedSender;
use spectra_core::drive::Drive;

const RATE: u32 = 44_100;
/// Stereo frames in a sector.
const FRAMES_PER_SECTOR: u64 = 588;
const SECTORS_PER_SECOND: u32 = 75;
/// Sectors per read: about a third of a second.
const READ: u32 = 27;
/// Samples kept queued: two seconds.
const AHEAD: usize = 2 * RATE as usize * 2;

/// A track, as the sectors it spans: index 1 to the next track.
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
    pub fn new(drive: PathBuf, spans: Vec<Span>, events: UnboundedSender<Event>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || run(drive, spans, rx, events));
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

/// Read audio sectors, giving a drive a couple more tries: a read that
/// fails once often goes through when asked again.
fn read(drive: &Drive, lba: u32, count: u32) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for _ in 0..3 {
        match drive.read_raw(lba, count, true) {
            Ok(data) => return Ok(data),
            Err(e) if e.medium_not_present() => return Err(e.to_string()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn output(shared: Arc<Shared>) -> Result<cpal::Stream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no sound output")?;
    let config = cpal::StreamConfig {
        channels: 2,
        sample_rate: cpal::SampleRate(RATE),
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

/// Which track a sector is in, and how far into it.
fn locate(spans: &[Span], lba: u32) -> Option<(usize, u32)> {
    let track = spans.iter().rposition(|s| s.start <= lba)?;
    Some((track, (lba - spans[track].start) / SECTORS_PER_SECOND))
}

fn run(
    path: PathBuf,
    spans: Vec<Span>,
    commands: mpsc::Receiver<Command>,
    events: UnboundedSender<Event>,
) {
    let shared = Arc::new(Shared::default());
    let mut stream: Option<cpal::Stream> = None;
    let mut drive: Option<Drive> = None;
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
                if drive.is_none() {
                    match Drive::open(&path) {
                        Ok(d) => drive = Some(d),
                        Err(e) => {
                            fail(e.to_string());
                            continue;
                        }
                    }
                }
                if stream.is_none() {
                    match output(shared.clone()) {
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
                // Let go of the sound card, so nothing holds it open.
                stream = None;
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
        if queued < AHEAD && cursor < end {
            let count = READ.min(end - cursor);
            match read(drive.as_ref().expect("opened at play"), cursor, count) {
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
                    playing = false;
                    fail(e);
                    continue;
                }
            }
        }

        // Say where the sound card has got to, once a second.
        let at = origin + (shared.played.load(Ordering::Relaxed) / FRAMES_PER_SECTOR) as u32;
        if at >= end && queued == 0 {
            stream = None;
            playing = false;
            let _ = events.unbounded_send(Event::Stopped);
            continue;
        }
        if let Some((track, seconds)) = locate(&spans, at)
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
            start: t.lba,
            end: t.lba + sectors,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sector_is_placed_in_its_track() {
        let spans = [
            Span { start: 0, end: 750 },
            Span {
                start: 750,
                end: 1500,
            },
        ];
        assert_eq!(locate(&spans, 0), Some((0, 0)));
        assert_eq!(locate(&spans, 749), Some((0, 9)));
        assert_eq!(locate(&spans, 900), Some((1, 2)));
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
}
