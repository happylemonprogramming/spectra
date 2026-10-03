//! Spectra: put a disc in, and it plays.
//!
//! One screen - the rainbow disc, the album's blurred cover behind it, and a
//! track list driven by keyboard or gamepad - showing whatever disc is in the
//! drive. An album file stands in for the drive while working on the screen.
//!
//!   spectra [ALBUM.json] [--play] [--reduced-motion]

mod album;
mod art;
mod artwork;
mod disc;
mod game;
mod gamepad;
mod motion;
mod platform;
mod watch;

use std::path::PathBuf;
use std::time::Instant;

use iced::alignment::Vertical;
use iced::font::Weight;
use iced::gradient::Linear;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::scrollable::{AbsoluteOffset, Direction, Scrollbar, Viewport};
use iced::widget::text::Wrapping;
use iced::widget::{
    Id, button, column, container, image, mouse_area, operation, responsive, row, scrollable,
    shader, space, stack, text,
};
use iced::{
    Background, Border, Color, ContentFit, Element, Fill, Font, Radians, Shadow, Size,
    Subscription, Task, Theme, Vector, window,
};

use album::Album;
use art::Art;
use motion::Motion;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::futures::SinkExt;
use spectra_core::drive::Drive;
use spectra_core::{DiscKind, GameIdentity, Report, library};
use watch::DriveState;

const FONT: Font = Font::with_name("Adwaita Sans");
const BOLD: Font = Font {
    weight: Weight::Bold,
    ..FONT
};
/// A CD's track pitch in nanometres.
const CD_PITCH: f32 = 1600.0;
const TRACKS: &str = "tracks";
/// Room around the rows inside the scrolling list, so the focus glow is not
/// clipped at its edges.
const LIST_PAD: f32 = 14.0;
const ROW_GAP: f32 = 4.0;

/// The remote control: keyboard and gamepads both speak it.
#[derive(Debug, Clone, Copy)]
pub enum Remote {
    Up,
    Down,
    Select,
    PlayPause,
    Previous,
    Next,
    Back,
    /// Keep a copy of the disc, to play without it.
    Keep,
    Quit,
}

#[derive(Debug, Clone)]
enum Message {
    Remote(Remote),
    /// A track clicked.
    Pick(usize),
    /// The track list scrolled or changed size.
    Scrolled(Viewport),
    Frame(Instant),
    Drive(DriveState),
    /// The emulator closed: how it went, in words, if badly.
    GameOver(Option<String>),
    /// Pictures for the game with this serial arrived.
    Pictures(String, artwork::Pictures),
    Keeping(Keeping),
}

/// How the screen is arranged, from the most room to the least.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Layout {
    /// The disc beside the album: landscape windows with room for both.
    Beside { disc: f32 },
    /// The disc above the album: portrait windows.
    Above { disc: f32 },
    /// No disc: the cover as a thumbnail, and the room goes to the tracks.
    List,
    /// Too small for a list: what is playing, and the controls.
    Controls,
}

/// Width the album needs beside the disc.
const PANEL_MIN: f32 = 440.0;
/// Height the album keeps under a disc placed above it.
const BELOW_MIN: f32 = 340.0;
/// Smaller than this, the disc is a decoration; the room is better spent on
/// the tracks.
const DISC_MIN: f32 = 260.0;

impl Layout {
    fn for_size(size: Size) -> Self {
        let (w, h) = (size.width, size.height);
        if w < 240.0 || h < 220.0 {
            return Self::Controls;
        }
        if h <= w * 1.05 {
            let disc = h.min(w - PANEL_MIN).min(w * 0.55);
            if disc >= DISC_MIN {
                return Self::Beside { disc };
            }
        }
        let disc = (h - BELOW_MIN).min(w).min(h * 0.45);
        if disc >= DISC_MIN {
            return Self::Above { disc };
        }
        Self::List
    }
}

/// Type and spacing: roomy when the window is, tighter when it is not.
#[derive(Clone, Copy)]
struct Scale {
    title: f32,
    artist: f32,
    detail: f32,
    track: f32,
    row: f32,
    gap: f32,
}

impl Scale {
    fn for_size(size: Size) -> Self {
        if size.height >= 600.0 && size.width >= 560.0 {
            Self {
                title: 42.0,
                artist: 22.0,
                detail: 15.0,
                track: 18.0,
                row: 46.0,
                gap: 24.0,
            }
        } else {
            Self {
                title: 24.0,
                artist: 16.0,
                detail: 13.0,
                track: 15.0,
                row: 36.0,
                gap: 12.0,
            }
        }
    }
}

struct Spectra {
    album: Album,
    art: Art,
    motion: Motion,
    focus: usize,
    playing: Option<usize>,
    paused: bool,
    /// Per row, 0 to 1: how strongly it glows as the focus.
    glow: Vec<f32>,
    /// The track list's last reported viewport, while it overflows.
    list: Option<Viewport>,
    last_frame: Option<Instant>,
    /// Showing the drive's disc, rather than an album file.
    watching: bool,
    /// The game on the disc, with an emulator ready to play it.
    launch: Option<Launch>,
    /// A game is running, and has the gamepad.
    in_game: bool,
    /// The serial of the game disc on screen, which its pictures are for.
    serial: Option<String>,
    disc: Option<Inserted>,
    copying: Option<Copying>,
    /// The kept copies on screen, row for row, when there is no disc.
    shelf: Vec<library::Entry>,
    /// The row whose copy the disc on screen is wearing.
    shelf_art: Option<usize>,
}

struct Launch {
    emulator: game::Emulator,
    /// What the emulator opens: the drive, or a kept copy's cue sheet.
    content: String,
}

/// The game disc in the drive.
struct Inserted {
    report: Box<Report>,
    drive: PathBuf,
    game: GameIdentity,
}

/// A copy of the disc being kept.
struct Copying {
    cancel: Arc<AtomicBool>,
    started: Instant,
}

#[derive(Debug, Clone)]
enum Keeping {
    Progress(u32, u32),
    Done(Result<Box<library::Entry>, String>),
}

struct Options {
    album: Option<PathBuf>,
    play: bool,
    reduced_motion: bool,
}

fn options() -> Options {
    let mut options = Options {
        album: None,
        play: false,
        reduced_motion: false,
    };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--play" => options.play = true,
            "--reduced-motion" => options.reduced_motion = true,
            _ => options.album = Some(PathBuf::from(arg)),
        }
    }
    options
}

impl Spectra {
    fn new() -> (Self, Task<Message>) {
        let options = options();
        let album = match &options.album {
            Some(path) => Album::load(path).unwrap_or_else(|e| {
                eprintln!("spectra: {e}");
                std::process::exit(1)
            }),
            None => Album::no_disc(),
        };
        let mut app = Self {
            art: Art::new(&album.cover, album.face.as_ref()),
            album,
            motion: Motion::new(options.reduced_motion),
            focus: 0,
            playing: None,
            paused: false,
            glow: Vec::new(),
            list: None,
            last_frame: None,
            watching: options.album.is_none(),
            launch: None,
            in_game: false,
            serial: None,
            disc: None,
            copying: None,
            shelf: Vec::new(),
            shelf_art: None,
        };
        app.rewind();
        if options.play {
            app.play(0);
        }
        (app, Task::none())
    }

    /// Put an album on screen, from the top, with nothing playing.
    fn show(&mut self, album: Album) {
        // Blurring a cover is the costly part, and every state of the drive
        // shows the same placeholder one.
        if album.cover != self.album.cover || album.face != self.album.face {
            self.art = Art::new(&album.cover, album.face.as_ref());
            platform::release_memory();
        }
        self.album = album;
        self.rewind();
    }

    fn rewind(&mut self) {
        self.glow = vec![0.0; self.album.tracks.len()];
        if let Some(first) = self.glow.first_mut() {
            *first = 1.0;
        }
        self.focus = 0;
        self.playing = None;
        self.paused = false;
        self.motion.set_spinning(false);
    }

    fn play(&mut self, track: usize) {
        if !self.album.playable {
            return;
        }
        self.playing = Some(track);
        self.paused = false;
        self.motion.set_spinning(true);
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Frame(now) => {
                // After a pause in drawing, start again from one frame's step
                // rather than jumping by however long nothing moved.
                let dt = self
                    .last_frame
                    .map_or(1.0 / 60.0, |last| now.duration_since(last).as_secs_f32())
                    .min(0.05);
                self.last_frame = Some(now);
                self.motion.step(dt);
                for (i, glow) in self.glow.iter_mut().enumerate() {
                    let target = if i == self.focus { 1.0 } else { 0.0 };
                    *glow += (target - *glow) * (1.0 - (-14.0 * dt).exp());
                    if (*glow - target).abs() < 0.004 {
                        *glow = target;
                    }
                }
                Task::none()
            }
            Message::Scrolled(viewport) => {
                // A new size can leave the focus out of sight; a scroll by
                // hand is left where it was put.
                let resized = self
                    .list
                    .is_none_or(|old| old.bounds().size() != viewport.bounds().size());
                self.list = Some(viewport);
                if resized {
                    self.reveal_focus()
                } else {
                    Task::none()
                }
            }
            Message::Drive(state) => {
                let mut album = Album::from_drive(&state);
                let idle = matches!(state, DriveState::NoDrive | DriveState::Empty);
                self.disc = match state {
                    DriveState::Disc { report, drive } => match &report.kind {
                        DiscKind::Game(game) => Some(Inserted {
                            game: game.clone(),
                            report,
                            drive,
                        }),
                        _ => None,
                    },
                    _ => None,
                };
                self.shelf.clear();
                self.shelf_art = None;
                if idle {
                    let entries = library::list();
                    if !entries.is_empty() {
                        album = Album::shelf(&entries);
                        self.shelf = entries;
                    }
                }
                self.launch = self.disc_launch();
                if self.launch.is_some() {
                    album.note = Some(self.ready_note());
                }
                self.show(album);
                self.serial = self.disc.as_ref().and_then(|d| d.game.serial.clone());
                self.show_shelf_art();
                // The disc goes on screen now; its pictures follow when they
                // arrive, from the cache or the network.
                let pictures = self.disc.as_ref().and_then(|disc| {
                    let serial = disc.game.serial.clone()?;
                    let game = disc.game.clone();
                    Some(Task::perform(
                        blocking(move || artwork::for_game(&game)),
                        move |pictures| Message::Pictures(serial.clone(), pictures),
                    ))
                });
                let top = operation::scroll_to(
                    Id::from(TRACKS),
                    AbsoluteOffset {
                        x: None,
                        y: Some(0.0),
                    },
                );
                Task::batch([top].into_iter().chain(pictures))
            }
            Message::Keeping(Keeping::Progress(done, total)) => {
                if let Some(copying) = &self.copying {
                    let fraction = f64::from(done) / f64::from(total.max(1));
                    let elapsed = copying.started.elapsed().as_secs_f64();
                    let left = (fraction > 0.02).then(|| elapsed / fraction - elapsed);
                    let mut note = format!("Keeping a copy  ·  {:.0}%", fraction * 100.0);
                    if let Some(left) = left {
                        note.push_str(&match (left / 60.0).round() as u32 {
                            0 => "  ·  under a minute left".into(),
                            1 => "  ·  about a minute left".into(),
                            m => format!("  ·  about {m} minutes left"),
                        });
                    }
                    note.push_str("  ·  ○ or Backspace to stop");
                    self.album.note = Some(note);
                }
                Task::none()
            }
            Message::Keeping(Keeping::Done(result)) => {
                let cancelled = self
                    .copying
                    .take()
                    .is_some_and(|c| c.cancel.load(Ordering::Relaxed));
                self.motion.set_spinning(false);
                self.launch = self.disc_launch();
                let ready = self.ready_note();
                self.album.note = Some(match result {
                    Ok(entry) if entry.meta.unreadable > 0 => format!(
                        "Kept, but {} sectors could not be read  ·  {ready}",
                        entry.meta.unreadable
                    ),
                    Ok(_) => ready,
                    Err(_) if cancelled => format!("Stopped keeping a copy  ·  {ready}"),
                    Err(e) => format!("Couldn't keep a copy: {e}"),
                });
                Task::none()
            }
            Message::Pictures(serial, pictures) => {
                // Only if that disc is still the one on screen.
                if self.serial.as_deref() == Some(serial.as_str()) {
                    let mut album = std::mem::replace(&mut self.album, Album::no_disc());
                    if let Some(cover) = pictures.cover.or_else(|| pictures.face.clone()) {
                        album.cover = cover;
                    }
                    album.face = pictures.face;
                    self.art = Art::new(&album.cover, album.face.as_ref());
                    self.album = album;
                    platform::release_memory();
                }
                Task::none()
            }
            Message::GameOver(trouble) => {
                self.in_game = false;
                self.motion.set_spinning(false);
                let ready = self.ready_note();
                self.album.note = Some(trouble.map_or(ready, |why| {
                    format!("{why} - see ~/.cache/spectra/game.log")
                }));
                Task::none()
            }
            Message::Pick(track) if !self.shelf.is_empty() && !self.in_game => {
                self.focus = track;
                self.show_shelf_art();
                self.play_kept()
            }
            Message::Pick(track) => {
                self.focus = track;
                self.play(track);
                Task::none()
            }
            // Gamepads reach every program at once: while a game runs, the
            // buttons are the game's.
            Message::Remote(_) if self.in_game => Task::none(),
            // The drive is busy copying: a game would only fight it for reads.
            Message::Remote(Remote::Back) if self.copying.is_some() => {
                if let Some(copying) = &self.copying {
                    copying.cancel.store(true, Ordering::Relaxed);
                }
                Task::none()
            }
            Message::Remote(Remote::Select | Remote::PlayPause | Remote::Keep)
                if self.copying.is_some() =>
            {
                Task::none()
            }
            Message::Remote(Remote::Keep) => self.keep_copy(),
            Message::Remote(Remote::Select | Remote::PlayPause) if !self.shelf.is_empty() => {
                self.play_kept()
            }
            Message::Remote(Remote::Select | Remote::PlayPause) if self.launch.is_some() => {
                self.start_game()
            }
            Message::Remote(remote) => {
                let count = self.album.tracks.len();
                if count == 0 {
                    return if let Remote::Quit = remote {
                        iced::exit()
                    } else {
                        Task::none()
                    };
                }
                match remote {
                    Remote::Up => self.focus = self.focus.saturating_sub(1),
                    Remote::Down => self.focus = (self.focus + 1).min(count - 1),
                    Remote::Select => self.play(self.focus),
                    Remote::PlayPause => match self.playing {
                        Some(_) => {
                            self.paused = !self.paused;
                            self.motion.set_spinning(!self.paused);
                        }
                        None => self.play(self.focus),
                    },
                    Remote::Previous | Remote::Next => {
                        let current = self.playing.unwrap_or(self.focus);
                        self.focus = match remote {
                            Remote::Previous => current.saturating_sub(1),
                            _ => (current + 1).min(count - 1),
                        };
                        if self.playing.is_some() {
                            self.play(self.focus);
                        }
                    }
                    Remote::Back => {
                        self.playing = None;
                        self.motion.set_spinning(false);
                    }
                    Remote::Keep => {}
                    Remote::Quit => return iced::exit(),
                }
                self.show_shelf_art();
                self.reveal_focus()
            }
        }
    }

    /// How the disc in the drive would be played: from its kept copy if
    /// there is one, which is quicker and quieter, or else from the drive.
    fn disc_launch(&self) -> Option<Launch> {
        let disc = self.disc.as_ref()?;
        let emulator = game::find(disc.game.system)?;
        let kept = library::id(&disc.report)
            .and_then(|id| library::find(&id))
            .map(|entry| entry.cue().to_string_lossy().into_owned());
        let content = kept.or_else(|| game::cdrom_uri(&disc.drive))?;
        Some(Launch { emulator, content })
    }

    /// What can be done now, for the line under a game.
    fn ready_note(&self) -> String {
        if !self.shelf.is_empty() {
            return "✕ or Enter to play".into();
        }
        let Some(disc) = &self.disc else {
            return String::new();
        };
        if self.launch.is_none() {
            return "No emulator for this console is installed".into();
        }
        match library::id(&disc.report) {
            Some(id) if library::find(&id).is_some() => {
                "✕ or Enter to play  ·  Kept: plays without the disc".into()
            }
            Some(_) => "✕ or Enter to play  ·  △ or C to keep a copy".into(),
            None => "✕ or Enter to play".into(),
        }
    }

    /// Play the kept copy in focus on the shelf.
    fn play_kept(&mut self) -> Task<Message> {
        let Some(entry) = self.shelf.get(self.focus) else {
            return Task::none();
        };
        self.launch = entry
            .meta
            .system
            .and_then(game::find)
            .map(|emulator| Launch {
                emulator,
                content: entry.cue().to_string_lossy().into_owned(),
            });
        if self.launch.is_none() {
            self.album.note = Some("No emulator for this console is installed".into());
        }
        self.start_game()
    }

    /// On the shelf, the disc on screen is the copy in focus.
    fn show_shelf_art(&mut self) {
        let Some(entry) = self.shelf.get(self.focus) else {
            return;
        };
        if self.shelf_art == Some(self.focus) {
            return;
        }
        self.shelf_art = Some(self.focus);
        let pictures = artwork::for_copy(entry);
        if let Some(cover) = pictures.cover.or_else(|| pictures.face.clone()) {
            self.album.cover = cover;
        }
        self.album.face = pictures.face;
        self.art = Art::new(&self.album.cover, self.album.face.as_ref());
        platform::release_memory();
    }

    fn keep_copy(&mut self) -> Task<Message> {
        let Some(disc) = &self.disc else {
            return Task::none();
        };
        let Some(id) = library::id(&disc.report) else {
            return Task::none();
        };
        if library::find(&id).is_some() {
            return Task::none();
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.copying = Some(Copying {
            cancel: cancel.clone(),
            started: Instant::now(),
        });
        self.motion.set_spinning(true);
        self.album.note = Some("Keeping a copy  ·  ○ or Backspace to stop".into());
        let (report, drive, game) = (disc.report.clone(), disc.drive.clone(), disc.game.clone());
        Task::run(
            iced::stream::channel(4, async move |mut output| {
                let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
                std::thread::spawn(move || {
                    let mut last = u64::MAX;
                    let result = Drive::open(&drive).and_then(|drive| {
                        library::keep(&drive, &report, &cancel, |p| {
                            // A message per percent, not per read.
                            let percent = u64::from(p.done) * 100 / u64::from(p.total.max(1));
                            if percent != last {
                                last = percent;
                                let _ = tx.unbounded_send(Keeping::Progress(p.done, p.total));
                            }
                        })
                    });
                    if let Ok(entry) = &result {
                        artwork::store(&entry.dir, &game);
                    }
                    let _ = tx.unbounded_send(Keeping::Done(
                        result.map(Box::new).map_err(|e| e.to_string()),
                    ));
                });
                use iced::futures::StreamExt;
                while let Some(event) = rx.next().await {
                    if output.send(event).await.is_err() {
                        break;
                    }
                }
            }),
            Message::Keeping,
        )
    }

    fn start_game(&mut self) -> Task<Message> {
        let Some(launch) = &self.launch else {
            return Task::none();
        };
        let mut child = match launch.emulator.launch(&launch.content) {
            Ok(child) => child,
            Err(e) => {
                self.album.note = Some(format!("Couldn't start RetroArch: {e}"));
                return Task::none();
            }
        };
        self.in_game = true;
        self.motion.set_spinning(true);
        self.album.note = Some("Playing in RetroArch  ·  Esc twice to quit".into());
        let (tx, rx) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let trouble = match child.wait() {
                Ok(status) if status.success() => None,
                Ok(status) => Some(format!("The game stopped with {status}")),
                Err(e) => Some(e.to_string()),
            };
            let _ = tx.send(trouble);
        });
        Task::perform(rx, |trouble| Message::GameOver(trouble.ok().flatten()))
    }

    /// Scroll the track list just enough to show the focused row, glow
    /// included. Rows are all one height, so where a row sits follows from
    /// the list's content height.
    fn reveal_focus(&self) -> Task<Message> {
        let Some(list) = self.list else {
            return Task::none();
        };
        let count = self.album.tracks.len() as f32;
        let pitch = (list.content_bounds().height - 2.0 * LIST_PAD + ROW_GAP) / count;
        let top = self.focus as f32 * pitch;
        let bottom = top + pitch - ROW_GAP + 2.0 * LIST_PAD;
        let offset = list.absolute_offset().y;
        let height = list.bounds().height;
        let y = if top < offset {
            top
        } else if bottom > offset + height {
            bottom - height
        } else {
            return Task::none();
        };
        operation::scroll_to(
            Id::from(TRACKS),
            AbsoluteOffset {
                x: None,
                y: Some(y.max(0.0)),
            },
        )
    }

    fn animating(&self) -> bool {
        self.motion.moving()
            || self
                .glow
                .iter()
                .enumerate()
                .any(|(i, &g)| g != if i == self.focus { 1.0 } else { 0.0 })
    }

    fn subscription(&self) -> Subscription<Message> {
        let remote =
            Subscription::batch([keyboard::listen().filter_map(key), gamepad::subscription()])
                .map(Message::Remote);
        let remote = if self.watching {
            Subscription::batch([remote, watch::subscription().map(Message::Drive)])
        } else {
            remote
        };
        // Frames are only asked for while something moves. At rest, nothing
        // is drawn and the GPU sleeps.
        if self.animating() {
            Subscription::batch([remote, window::frames().map(Message::Frame)])
        } else {
            remote
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let backdrop = image(self.art.backdrop.clone())
            .width(Fill)
            .height(Fill)
            .content_fit(ContentFit::Cover);
        let shade = container(space())
            .width(Fill)
            .height(Fill)
            .style(|_| container::Style {
                background: Some(Background::Gradient(
                    Linear::new(Radians(std::f32::consts::FRAC_PI_2))
                        .add_stop(0.0, Color::from_rgba8(6, 6, 12, 0.62))
                        .add_stop(0.55, Color::from_rgba8(6, 6, 12, 0.76))
                        .add_stop(1.0, Color::from_rgba8(6, 6, 12, 0.9))
                        .into(),
                )),
                ..Default::default()
            });
        stack![backdrop, shade, responsive(|size| self.screen(size))].into()
    }

    /// Everything over the backdrop, arranged for the room there is.
    fn screen(&self, size: Size) -> Element<'_, Message> {
        let scale = Scale::for_size(size);
        match Layout::for_size(size) {
            Layout::Beside { disc } => {
                let album = column![self.header(scale), self.tracks(scale), self.hints(scale)]
                    .spacing(scale.gap)
                    .max_width(560);
                let pad = if scale.gap > 20.0 { [40, 44] } else { [16, 20] };
                row![
                    self.disc().width(disc).height(Fill),
                    container(album).padding(pad).height(Fill).center_y(Fill),
                ]
                .into()
            }
            Layout::Above { disc } => column![
                self.disc().width(Fill).height(disc),
                container(
                    column![self.header(scale), self.tracks(scale), self.hints(scale)]
                        .spacing(scale.gap)
                        .max_width(560),
                )
                .padding([0, 20])
                .center_x(Fill)
                .height(Fill),
            ]
            .into(),
            Layout::List => container(
                column![
                    row![self.cover(64.0), self.header(scale)]
                        .spacing(14)
                        .align_y(Vertical::Center),
                    self.tracks(scale),
                ]
                .spacing(scale.gap),
            )
            .padding([16, 14])
            .into(),
            Layout::Controls => self.controls(size),
        }
    }

    fn accent(&self) -> Color {
        Color::from_rgb(self.art.accent[0], self.art.accent[1], self.art.accent[2])
    }

    fn disc(&self) -> iced::widget::Shader<Message, disc::Disc> {
        shader(disc::Disc {
            pose: self.motion.pose(),
            label: self.art.label.clone(),
            accent: self.art.accent,
            pitch: CD_PITCH,
        })
    }

    fn cover(&self, side: f32) -> Element<'_, Message> {
        image(self.art.thumbnail.clone())
            .width(side)
            .height(side)
            .border_radius(side * 0.08)
            .into()
    }

    fn header(&self, scale: Scale) -> Element<'_, Message> {
        let album = &self.album;
        let summary = (!album.tracks.is_empty()).then(|| {
            [
                album.year.map(|y| y.to_string()),
                Some(format!("{} tracks", album.tracks.len())),
                Some(format!("{} min", album.total_seconds().div_ceil(60))),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("  ·  ")
        });
        let details = album.details.clone().or(summary);
        let dim = |line: &str, alpha: f32| {
            text(line.to_string())
                .font(FONT)
                .size(scale.detail)
                .color(Color::from_rgba(1.0, 1.0, 1.0, alpha))
        };
        container(
            column![
                text(&album.title)
                    .font(BOLD)
                    .size(scale.title)
                    .color(Color::WHITE),
                text(&album.artist)
                    .font(FONT)
                    .size(scale.artist)
                    .color(Color::from_rgba(1.0, 1.0, 1.0, 0.78)),
            ]
            .push(details.map(|details| dim(&details, 0.5)))
            .push(album.note.as_deref().map(|note| dim(note, 0.38)))
            .spacing(scale.gap / 4.0),
        )
        .padding([0.0, LIST_PAD])
        .into()
    }

    fn tracks(&self, scale: Scale) -> Element<'_, Message> {
        let accent = self.accent();
        let rows = self.album.tracks.iter().enumerate().map(|(i, track)| {
            let glow = self.glow[i];
            let is_playing = self.playing == Some(i);
            let marker = match (is_playing, self.paused) {
                (true, false) => "▶".to_string(),
                (true, true) => "❚❚".to_string(),
                _ => format!("{}", i + 1),
            };
            let strong =
                Color::from_rgba(1.0, 1.0, 1.0, 0.72 + 0.28 * glow.max(f32::from(is_playing)));
            let line = row![
                text(marker)
                    .font(FONT)
                    .size(scale.track - 3.0)
                    .width(scale.track * 1.9)
                    .color(if is_playing { accent } else { strong }),
                container(
                    text(&track.title)
                        .font(if is_playing { BOLD } else { FONT })
                        .size(scale.track)
                        .wrapping(Wrapping::None)
                        .color(strong),
                )
                .width(Fill)
                .clip(true),
                text(track.detail.clone().unwrap_or_else(|| {
                    format!("{}:{:02}", track.seconds / 60, track.seconds % 60)
                }))
                .font(FONT)
                .size(scale.track - 3.0)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.5)),
            ]
            .spacing(8)
            .align_y(Vertical::Center);
            let row = container(line)
                .padding([0.0, scale.row * 0.36])
                .width(Fill)
                .height(scale.row)
                .center_y(scale.row)
                .style(move |_| container::Style {
                    background: Some(Background::Color(
                        Color {
                            a: 0.07 + 0.13 * glow,
                            ..accent
                        }
                        .scale_alpha(glow),
                    )),
                    border: Border {
                        color: Color {
                            a: 0.55 * glow,
                            ..accent
                        },
                        width: 1.0,
                        radius: (scale.row * 0.26).into(),
                    },
                    shadow: Shadow {
                        color: Color {
                            a: 0.45 * glow,
                            ..accent
                        },
                        offset: Vector::ZERO,
                        blur_radius: 22.0 * glow,
                    },
                    ..Default::default()
                });
            mouse_area(row).on_press(Message::Pick(i)).into()
        });
        scrollable(column(rows).spacing(ROW_GAP).padding(LIST_PAD))
            .id(TRACKS)
            .on_scroll(Message::Scrolled)
            .direction(Direction::Vertical(
                Scrollbar::new().width(3).scroller_width(3).margin(2),
            ))
            .into()
    }

    fn hints(&self, scale: Scale) -> Element<'_, Message> {
        if scale.gap < 20.0 || !self.album.playable || self.album.tracks.is_empty() {
            return space().into();
        }
        container(
            text("↑↓ Choose    ✕ Play    L1 R1 Skip    Start Pause    ○ Stop")
                .font(FONT)
                .size(13)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.4)),
        )
        .padding([0.0, LIST_PAD])
        .into()
    }

    /// The smallest layout: the cover, what is playing (or would play), and
    /// buttons for the mouse.
    fn controls(&self, size: Size) -> Element<'_, Message> {
        let accent = self.accent();
        let now = self.playing.unwrap_or(self.focus);
        let title = self
            .album
            .tracks
            .get(now)
            .map_or(self.album.title.as_str(), |t| t.title.as_str());
        let control = |glyph: &'static str, remote: Remote| {
            button(text(glyph).font(FONT).size(15).center())
                .width(40)
                .height(40)
                .on_press(Message::Remote(remote))
                .style(move |_, status| button::Style {
                    background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                        .then_some(Background::Color(Color { a: 0.3, ..accent })),
                    text_color: Color::WHITE,
                    border: Border {
                        radius: 20.0.into(),
                        ..Border::default()
                    },
                    ..button::Style::default()
                })
        };
        let playing = self.playing.is_some() && !self.paused;
        let buttons = row![
            control("❚◀", Remote::Previous),
            control(if playing { "❚❚" } else { "▶" }, Remote::PlayPause),
            control("▶❚", Remote::Next),
        ]
        .spacing(4);
        let line = |text_size: f32| {
            column![
                text(title)
                    .font(BOLD)
                    .size(text_size)
                    .wrapping(Wrapping::None)
                    .color(Color::WHITE),
                text(&self.album.artist)
                    .font(FONT)
                    .size(text_size - 2.0)
                    .wrapping(Wrapping::None)
                    .color(Color::from_rgba(1.0, 1.0, 1.0, 0.7)),
            ]
            .spacing(2)
        };
        // Side by side when wide; stacked and centred when narrow; and when
        // there is no room for words, only the buttons.
        let content: Element<'_, Message> = if size.width >= 300.0 {
            let mut side = row![].spacing(12).align_y(Vertical::Center);
            if size.width >= 360.0 {
                side = side.push(self.cover(48.0));
            }
            side.push(container(line(15.0)).width(Fill).clip(true))
                .push(buttons)
                .into()
        } else if size.height >= 150.0 {
            let mut stacked = column![].spacing(10).align_x(iced::Center);
            if size.height >= 230.0 {
                stacked = stacked.push(self.cover(56.0));
            }
            stacked
                .push(container(line(14.0).align_x(iced::Center)).clip(true))
                .push(buttons)
                .into()
        } else {
            buttons.into()
        };
        container(content).padding([8, 12]).center(Fill).into()
    }
}

/// Blocking work - the network, the disc - on a thread of its own, as a
/// future the UI can wait on without stopping.
fn blocking<T: Default + Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = T> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    async move { rx.await.unwrap_or_default() }
}

fn key(event: keyboard::Event) -> Option<Remote> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };
    match key.as_ref() {
        Key::Named(Named::ArrowUp) => Some(Remote::Up),
        Key::Named(Named::ArrowDown) => Some(Remote::Down),
        Key::Named(Named::ArrowLeft) => Some(Remote::Previous),
        Key::Named(Named::ArrowRight) => Some(Remote::Next),
        Key::Named(Named::Enter) => Some(Remote::Select),
        Key::Named(Named::Space) => Some(Remote::PlayPause),
        Key::Named(Named::Backspace) => Some(Remote::Back),
        Key::Named(Named::Escape) => Some(Remote::Quit),
        Key::Character("c") => Some(Remote::Keep),
        _ => None,
    }
}

/// wgpu starts every backend it was built with, and OpenGL costs about 90 ms
/// of startup even when Vulkan is what gets used. So where a Vulkan driver is
/// installed, ask for Vulkan alone; where none is - old GPUs - leave OpenGL
/// available as the fallback. An explicit WGPU_BACKEND always wins.
fn prefer_vulkan() {
    if std::env::var_os("WGPU_BACKEND").is_some() {
        return;
    }
    let has_driver = ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d"]
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .any(|entry| entry.path().extension().is_some_and(|e| e == "json"));
    if has_driver {
        // SAFETY: called first thing in main, before any other thread exists.
        unsafe { std::env::set_var("WGPU_BACKEND", "vulkan") };
    }
}

fn main() -> iced::Result {
    prefer_vulkan();
    iced::application(Spectra::new, Spectra::update, Spectra::view)
        .title("Spectra")
        .subscription(Spectra::subscription)
        .theme(|_: &Spectra| Theme::Dark)
        .default_font(FONT)
        .window(window::Settings {
            size: Size::new(1280.0, 760.0),
            // Matches the desktop entry's name, so launchers and window rules
            // know the window is Spectra's.
            #[cfg(target_os = "linux")]
            platform_specific: window::settings::PlatformSpecific {
                application_id: "spectra".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .antialiasing(true)
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(w: f32, h: f32) -> Layout {
        Layout::for_size(Size::new(w, h))
    }

    #[test]
    fn landscape_windows_put_the_disc_beside() {
        assert!(matches!(layout(1536.0, 950.0), Layout::Beside { disc } if disc > 800.0));
        assert!(matches!(layout(1536.0, 420.0), Layout::Beside { disc } if disc == 420.0));
        assert!(matches!(layout(760.0, 470.0), Layout::Beside { .. }));
    }

    #[test]
    fn portrait_windows_put_the_disc_above() {
        assert!(matches!(layout(760.0, 950.0), Layout::Above { .. }));
        assert!(matches!(layout(500.0, 950.0), Layout::Above { .. }));
        assert!(matches!(layout(400.0, 620.0), Layout::Above { .. }));
    }

    #[test]
    fn small_windows_drop_the_disc_then_the_list() {
        assert_eq!(layout(420.0, 300.0), Layout::List);
        assert_eq!(layout(640.0, 460.0), Layout::List);
        assert_eq!(layout(300.0, 500.0), Layout::List);
        assert_eq!(layout(600.0, 160.0), Layout::Controls);
        assert_eq!(layout(220.0, 500.0), Layout::Controls);
    }
}
