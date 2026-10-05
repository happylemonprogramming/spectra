//! Spectra: put a disc in, and it plays.
//!
//! Two screens, stacked. The stage - the rainbow disc, the album's blurred
//! cover behind it, and a track list driven by keyboard or gamepad - shows
//! whatever disc is in the drive. Below it is the library: every disc kept
//! as a copy, on a shelf of small discs, any of which can be put on the
//! stage and played without the disc. Moving between them slides the pair
//! up or down a screen, as Rainbow Player's do. An album file stands in for
//! the drive while working on the screen.
//!
//!   spectra [ALBUM.json] [--play] [--reduced-motion]
//!
//! The same window can be driven from a terminal or by an agent - `spectra
//! play`, `spectra pause`, `spectra status` - see `cli` and `control`.

mod album;
mod art;
mod artwork;
mod audio;
mod cli;
mod control;
mod disc;
mod film;
mod filmdb;
mod game;
mod gamepad;
mod motion;
mod musicbrainz;
mod net;
mod platform;
mod shelf;
mod soundtrack;
mod theme;
mod ui;
mod watch;
mod windows;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::alignment::Vertical;
use iced::font::Weight;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::scrollable::{AbsoluteOffset, Direction, Scrollbar, Viewport};
use iced::widget::text::Wrapping;
use iced::widget::{
    Id, button, column, container, image, mouse_area, operation, progress_bar, responsive, row,
    scrollable, shader, space, stack, text,
};
use iced::{
    Background, Border, Color, ContentFit, Element, Fill, Font, Point, Shadow, Shrink, Size,
    Subscription, Task, Theme, Vector, window,
};

use album::Album;
use art::{Art, Face};
use motion::Motion;
use shelf::{Section, Shelf, Spot};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::futures::SinkExt;
use spectra_core::drive::Drive;
use spectra_core::{DiscKind, GameIdentity, Report, library};
use watch::DriveState;

/// Silkscreen, a pixel face, built in: see `theme.rs`.
const FONT: Font = Font::with_name("Silkscreen");
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
/// The stage's disc, among the discs on the GPU; the shelf's come after.
const STAGE: u64 = 0;
const ROW_GAP: f32 = 4.0;
/// How long the stage and the library take to slide past each other.
const SLIDE_SECONDS: f32 = 0.7;
/// How long a delete waits to be confirmed.
const ARMED: Duration = Duration::from_secs(4);

/// The remote control: keyboard and gamepads both speak it.
#[derive(Debug, Clone, Copy)]
pub enum Remote {
    Up,
    Down,
    Left,
    Right,
    Select,
    PlayPause,
    Previous,
    Next,
    Back,
    /// Keep a copy of the disc, to play without it.
    Keep,
    /// The pad's select button, or M: the library from the stage, and
    /// delete on the shelf.
    Menu,
    /// Over to the kept copies, and back to the stage.
    Library,
    /// A folder's own key: S, V or G.
    Folder(Section),
    /// The drive's own key, D: its disc onto the stage.
    Drive,
}

/// The two screens, the stage above the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Stage,
    Library,
}

#[derive(Debug, Clone)]
enum Message {
    /// The keyboard.
    Remote(Remote),
    /// A gamepad, and which kind, for the prompts.
    Pad(ui::Pad, Remote),
    /// A track clicked.
    Pick(usize),
    /// The track list scrolled or changed size.
    Scrolled(Viewport),
    Frame(Instant),
    Drive(DriveState),
    /// The emulator closed: how it went, in words, if badly.
    GameOver(Option<String>),
    /// VLC closed, the same.
    FilmOver(Option<String>),
    /// What the DVD with this label turned out to be.
    FilmFound(String, Box<filmdb::Found>),
    /// Pictures for the game with this serial arrived.
    Pictures(String, artwork::Pictures),
    Keeping(Keeping),
    Audio(audio::Event),
    /// MusicBrainz's answer for the audio CD with this disc ID.
    Release(String, Box<musicbrainz::Found>),
    /// The music found on kept games, by their IDs.
    Soundtracks(Vec<(String, Option<Arc<soundtrack::Soundtrack>>)>),
    OpenLibrary,
    CloseLibrary,
    /// From a copy on the stage back to the disc in the drive.
    KeepCopy,
    ShelfFocus(usize),
    ShelfPick(usize),
    /// The pointer over a spot on the desktop, and a click on one.
    DeskFocus(usize),
    DeskOpen(usize),
    /// A copy onto the desktop, or off it.
    ShelfPin(usize),
    OpenFolder(Section),
    CloseFolder,
    /// Every window put away: the desktop.
    ShowDesktop,
    /// The stage's window closed.
    CloseStage,
    ShelfMenu(usize),
    ShelfDelete(usize),
    MenuClose,
    ShelfScrolled(Viewport),
    /// A copy's face for the shelf, made off the UI thread.
    ShelfFace(String, Option<Arc<Face>>),
    /// A delete nobody confirmed in time.
    Disarm(String),
    /// The pointer moved: the mouse is in use, and here.
    Pointer(Point),
    /// A command from outside, and where its answer goes.
    Control(control::Request, control::Responder),
    /// The window was focused: the gamepad is its now.
    Focused,
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
                title: 32.0,
                artist: 18.0,
                detail: 13.0,
                track: 15.0,
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
    /// The drive, held while a game or film plays from it, so that another
    /// Spectra window does not read it too.
    drive_lock: Option<spectra_core::lock::Lock>,
    /// What VLC would open for the film in the drive.
    film_uri: Option<String>,
    /// A film playing in VLC: the gamepad is its remote.
    film: Option<film::Film>,
    /// The label of the DVD on screen, which a lookup's answer is for.
    film_label: Option<String>,
    /// The serial of the game disc on screen, which its pictures are for.
    serial: Option<String>,
    disc: Option<Inserted>,
    copying: Option<Copying>,
    /// The audio CD's player, while there is one in.
    player: Option<audio::Player>,
    /// Seconds into the playing track.
    elapsed: u32,
    /// The audio CD on screen, which a MusicBrainz answer is for.
    disc_id: Option<String>,
    /// A kept copy on the stage in place of the drive's disc, picked from
    /// the library. Back returns to the drive.
    picked: Option<library::Entry>,
    /// Whether the stage's window is open: once something has been opened
    /// on it, until its window is closed. A disc sitting in the drive does
    /// not open it by itself.
    stage_open: bool,
    /// The drive as the wall shows it while a copy has the stage, and so
    /// the stage's own art and words are the copy's.
    drive_shown: Option<DriveShown>,
    /// What the drive last said, kept while a copy is on the stage so that
    /// Back can return to the disc.
    drive_state: Option<DriveState>,
    /// The screen with the remote: where the slide is going.
    screen: Screen,
    slide: Slide,
    shelf: Shelf,
    /// Each kept game listened to so far, by ID, and the music it has. A
    /// game being listened to is here already, with none yet.
    soundtracks: HashMap<String, Option<Arc<soundtrack::Soundtrack>>>,
    /// What the prompts are drawn as: keys, or the pad last used.
    style: ui::Style,
    /// The mouse is what is in use, so there is a pointer to show where
    /// things are and prompts can step back.
    pointing: bool,
    cursor: Point,
    reduced_motion: bool,
}

/// Where the screens are: 0 the stage, 1 the library, eased between.
struct Slide {
    from: f32,
    to: f32,
    t: f32,
}

impl Slide {
    fn at(&self) -> f32 {
        // Rainbow Player's cubic-bezier(0.65, 0, 0.35, 1), near enough.
        let t = self.t;
        let eased = if t < 0.5 {
            4.0 * t * t * t
        } else {
            1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
        };
        self.from + (self.to - self.from) * eased
    }

    fn go(&mut self, to: f32, reduced: bool) {
        self.from = self.at();
        self.to = to;
        self.t = if reduced || self.from == to { 1.0 } else { 0.0 };
    }

    fn moving(&self) -> bool {
        self.t < 1.0
    }

    fn step(&mut self, dt: f32) {
        self.t = (self.t + dt / SLIDE_SECONDS).min(1.0);
    }
}

/// The disc in the drive, as its icon on the desktop shows it.
#[derive(Clone)]
struct DriveShown {
    disc: Option<art::Icon>,
    title: String,
}

struct Launch {
    emulator: game::Emulator,
    /// What the emulator opens: the drive, or a kept copy's cue sheet.
    content: String,
}

/// The game, audio CD or DVD in the drive.
struct Inserted {
    report: Box<Report>,
    drive: PathBuf,
    /// None for music.
    game: Option<GameIdentity>,
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
            drive_lock: None,
            film_uri: None,
            film: None,
            film_label: None,
            serial: None,
            disc: None,
            copying: None,
            player: None,
            elapsed: 0,
            disc_id: None,
            picked: None,
            stage_open: false,
            drive_shown: None,
            drive_state: None,
            screen: Screen::Stage,
            slide: Slide {
                from: 0.0,
                to: 0.0,
                t: 1.0,
            },
            shelf: Shelf::new(),
            soundtracks: HashMap::new(),
            style: ui::Style::Keys,
            pointing: false,
            cursor: Point::ORIGIN,
            reduced_motion: options.reduced_motion,
        };
        app.rewind();
        // A window just opened is the one its user is looking at.
        windows::claim();
        if options.play {
            app.play(0);
        }
        // The wall is home. An album file opened by name has no drive and
        // no wall; it goes straight on the stage.
        if !app.watching {
            return (app, Task::none());
        }
        app.slide = Slide {
            from: 1.0,
            to: 1.0,
            t: 1.0,
        };
        let wall = app.open_library();
        (app, wall)
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
        if let Some(player) = &self.player {
            player.play(track);
        }
        self.playing = Some(track);
        self.elapsed = 0;
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
                self.slide.step(dt);
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
                let arrived = matches!(state, DriveState::Disc { .. })
                    && !matches!(self.drive_state, Some(DriveState::Disc { .. }));
                self.drive_state = Some(state.clone());
                // A disc going in is the thing to look at: the wall turns to
                // the drive, on its top shelf.
                if arrived && self.screen == Screen::Library {
                    self.shelf.focus_drive();
                }
                let look = Task::none();
                // A kept copy is on the stage: it keeps it, and the wall
                // shows the disc by its name alone until it is chosen.
                if self.picked.is_some() {
                    let album = Album::from_drive(&state);
                    self.drive_shown = Some(DriveShown {
                        disc: matches!(state, DriveState::Disc { .. }).then(|| Face::blank().icon),
                        title: album.title,
                    });
                    return look;
                }
                // Otherwise the stage follows the drive, under the wall, so it
                // is ready when the disc is chosen.
                Task::batch([self.show_drive(state), look])
            }
            Message::Audio(event) => {
                match event {
                    audio::Event::At { track, seconds } => {
                        self.elapsed = seconds;
                        if self.playing != Some(track) && !self.paused {
                            // On to the next track by itself.
                            self.playing = Some(track);
                            self.focus = track;
                            return self.reveal_focus();
                        }
                    }
                    audio::Event::Stopped => {
                        self.playing = None;
                        self.motion.set_spinning(false);
                    }
                    audio::Event::Failed(why) => {
                        self.playing = None;
                        self.motion.set_spinning(false);
                        self.album.note = Some(format!("Couldn't play: {why}"));
                    }
                }
                Task::none()
            }
            Message::Release(disc_id, found) => {
                if self.disc_id.as_deref() != Some(disc_id.as_str()) {
                    return Task::none();
                }
                let musicbrainz::Found { release, cover } = *found;
                if let Some(release) = release {
                    self.album.name(&release.names());
                }
                if let Some(cover) = cover {
                    self.album.cover = cover;
                    self.art = Art::new(&self.album.cover, None);
                    platform::release_memory();
                }
                Task::none()
            }
            Message::Soundtracks(found) => {
                self.soundtracks.extend(found);
                // Each one found goes on the shelf beside its game.
                let missing = self.shelf.load(self.shelf_entries());
                shelf_faces(missing)
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
                    note.push_str("  ·  {back} Stop");
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
                if self.picked.is_none() {
                    self.film_uri = self.drive_state.as_ref().and_then(film_uri);
                }
                // A kept CD plays from its copy from now on.
                let mut task = Task::none();
                if let Ok(entry) = &result
                    && entry.is_album()
                    && let Some(toc) = entry.toc()
                {
                    task = self.start_player(audio::Source::Image(entry.bin()), audio::spans(&toc));
                }
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
                task
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
                self.drive_lock = None;
                self.motion.set_spinning(false);
                let ready = self.ready_note();
                self.album.note = Some(trouble.map_or(ready, |why| {
                    format!("{why} - see ~/.cache/spectra/game.log")
                }));
                Task::none()
            }
            Message::FilmFound(label, found) => {
                if self.film_label.as_deref() != Some(label.as_str()) {
                    return Task::none();
                }
                let filmdb::Found {
                    title,
                    year,
                    pictures,
                } = *found;
                if let Some(title) = title {
                    self.album.title = title;
                }
                if let Some(year) = year {
                    self.album.details = Some(match self.album.details.take() {
                        Some(length) => format!("{year}  ·  {length}"),
                        None => year.to_string(),
                    });
                }
                // The poster behind, the disc's own face on it; either
                // stands in for the other.
                if let Some(cover) = pictures.cover.clone().or_else(|| pictures.face.clone()) {
                    self.album.cover = cover;
                }
                if pictures.face.is_some() {
                    self.album.face = pictures.face;
                }
                self.art = Art::new(&self.album.cover, self.album.face.as_ref());
                platform::release_memory();
                Task::none()
            }
            Message::FilmOver(trouble) => {
                self.film = None;
                self.drive_lock = None;
                self.motion.set_spinning(false);
                let ready = self.ready_note();
                self.album.note = Some(trouble.map_or(ready, |why| {
                    format!("{why} - see ~/.cache/spectra/film.log")
                }));
                Task::none()
            }
            // The drive is busy copying.
            Message::Pick(_) if self.copying.is_some() => Task::none(),
            Message::Pick(track) => {
                self.focus = track;
                self.play(track);
                Task::none()
            }
            Message::Pointer(at) => {
                // Only a pointer that has moved is in use. The wall
                // scrolling under one left resting still reports it, at the
                // same place, and must not take the focus from the keys.
                if at != self.cursor {
                    self.pointing = true;
                }
                self.cursor = at;
                Task::none()
            }
            Message::OpenLibrary => self.open_library(),
            Message::CloseLibrary => self.close_library(),
            Message::KeepCopy => self.keep_copy(),
            Message::ShelfScrolled(viewport) => {
                self.shelf.scrolled(viewport);
                Task::none()
            }
            Message::ShelfFace(id, face) => {
                self.shelf
                    .set_face(id, face.unwrap_or_else(|| Arc::new(Face::blank())));
                Task::none()
            }
            Message::DeskFocus(at) => {
                if self.pointing && self.shelf.menu.is_none() && self.desk_active() {
                    self.shelf.point_desk(at);
                }
                Task::none()
            }
            Message::DeskOpen(at) => {
                self.shelf.menu = None;
                // A click on the desktop puts away the window in front of it.
                self.shelf.open = None;
                self.screen = Screen::Library;
                self.shelf.point_desk(at);
                self.activate()
            }
            Message::ShelfPin(index) => {
                self.shelf.toggle_pin(index);
                Task::none()
            }
            Message::OpenFolder(section) => self.open_folder(section),
            Message::CloseFolder => {
                self.shelf.close_folder();
                Task::none()
            }
            Message::CloseStage => {
                // Closing the window stops what plays in it.
                self.stop();
                self.stage_open = false;
                self.open_library()
            }
            Message::ShowDesktop => {
                self.shelf.close_folder();
                self.open_library()
            }
            Message::ShelfFocus(index) => {
                if self.pointing && self.shelf.menu.is_none() {
                    // From the pointer: the folder does not scroll under it.
                    self.shelf.point_folder(index);
                }
                Task::none()
            }
            Message::ShelfPick(index) => {
                self.shelf.menu = None;
                self.pick(index)
            }
            Message::ShelfMenu(index) => {
                self.shelf.focus_entry(index);
                self.shelf.menu = Some(shelf::Menu {
                    index,
                    at: Some(self.cursor),
                    item: 0,
                });
                Task::none()
            }
            Message::MenuClose => {
                self.shelf.menu = None;
                self.shelf.armed = None;
                Task::none()
            }
            Message::ShelfDelete(index) => self.delete(index),
            Message::Disarm(id) => {
                if self.shelf.armed.as_deref() == Some(id.as_str()) {
                    self.shelf.armed = None;
                }
                Task::none()
            }
            Message::Remote(remote) => {
                self.style = ui::Style::Keys;
                self.pointing = false;
                self.remote(remote)
            }
            // Every Spectra hears every pad: only the current window acts.
            Message::Pad(..) if !windows::is_current() => Task::none(),
            Message::Pad(pad, remote) => {
                self.style = ui::Style::Pad(pad);
                self.pointing = false;
                self.remote(remote)
            }
            Message::Focused => {
                windows::claim();
                Task::none()
            }
            Message::Control(request, responder) => {
                let (result, task) = self.control(request);
                let status = self.status();
                responder.send(match result {
                    Ok(()) => control::Reply::done(status),
                    Err(why) => control::Reply::failed(why, status),
                });
                task
            }
        }
    }

    /// A command from outside: what the remote would do, and an error in
    /// words where the remote would quietly do nothing.
    fn control(&mut self, request: control::Request) -> (Result<(), String>, Task<Message>) {
        use control::Request;
        match request {
            Request::Status => return (Ok(()), Task::none()),
            Request::Quit => {
                let _ = std::fs::remove_file(windows::socket(std::process::id()));
                return (Ok(()), iced::exit());
            }
            _ if self.copying.is_some() => {
                return (
                    Err("the drive is busy keeping a copy of the disc".into()),
                    Task::none(),
                );
            }
            _ if self.in_game => {
                return (
                    Err("a game is running: quit it first (hold Start)".into()),
                    Task::none(),
                );
            }
            // VLC has the screen: the transport is its remote, as the pad's
            // buttons are.
            _ if self.film.is_some() => {
                let key = match request {
                    Request::Pause | Request::Resume | Request::Toggle => film::Key::PlayPause,
                    Request::Next => film::Key::NextChapter,
                    Request::Previous => film::Key::PreviousChapter,
                    Request::Stop => {
                        self.film = None;
                        return (Ok(()), Task::none());
                    }
                    _ => {
                        return (
                            Err("a film is playing: `spectra stop` ends it".into()),
                            Task::none(),
                        );
                    }
                };
                if let Some(film) = &self.film {
                    film.press(key);
                }
                return (Ok(()), Task::none());
            }
            Request::Play { target, track } => return self.control_play(target, track),
            _ => {}
        }
        if self.album.tracks.is_empty() || !self.album.playable {
            return (Err("nothing on the stage plays".into()), Task::none());
        }
        match request {
            Request::Pause | Request::Resume if self.playing.is_none() => {
                return (Err("nothing is playing".into()), Task::none());
            }
            Request::Pause => self.pause(true),
            Request::Resume => self.pause(false),
            Request::Toggle => match self.playing {
                Some(_) => self.pause(!self.paused),
                None => self.play(self.focus),
            },
            Request::Next => self.skip(true),
            Request::Previous => self.skip(false),
            Request::Stop => self.stop(),
            Request::Status | Request::Quit | Request::Play { .. } => {}
        }
        (Ok(()), self.reveal_focus())
    }

    fn control_play(
        &mut self,
        target: control::Target,
        track: Option<usize>,
    ) -> (Result<(), String>, Task<Message>) {
        use control::Target;
        let mut task = Task::none();
        match target {
            Target::Stage => {}
            Target::Disc => {
                if self.picked.is_some() {
                    task = self.return_to_drive();
                }
                if self.film_uri.is_some() {
                    if !film::installed() {
                        return (Err("films play in VLC, which isn't installed".into()), task);
                    }
                    let film = self.start_film();
                    return self.started(Task::batch([task, film]));
                }
                if self.disc.is_none() {
                    return (Err("no disc Spectra plays is in the drive".into()), task);
                }
                if self.launch.is_some() {
                    let game = self.start_game();
                    return self.started(Task::batch([task, game]));
                }
                if self.disc.as_ref().is_some_and(|d| d.game.is_some()) {
                    return (Err(plain(&self.ready_note())), task);
                }
            }
            Target::Copy(id) => {
                let missing = self.shelf.load(self.shelf_entries());
                let faces = shelf_faces(missing);
                let Some(index) = self.shelf.entries.iter().position(|e| e.meta.id == id) else {
                    return (Err(format!("no kept copy called {id}")), faces);
                };
                let entry = &self.shelf.entries[index];
                if let Some(n) = track
                    && entry.is_album()
                    && entry.toc().is_some_and(|toc| n > audio::spans(&toc).len())
                {
                    return (Err(format!("{} has no track {n}", entry.meta.title)), faces);
                }
                // A game's soundtrack is music, though it shares the game's
                // description.
                let album = entry.is_album() || soundtrack::is_entry(entry);
                if entry.is_film() && !film::installed() {
                    return (
                        Err("films play in VLC, which isn't installed".into()),
                        faces,
                    );
                }
                let picked = self.pick(index);
                if self.picked.as_ref().is_none_or(|p| p.meta.id != id) {
                    return (Err("that copy would not open".into()), faces);
                }
                task = Task::batch([faces, picked]);
                // A film or game starts as it goes on the stage. An album
                // waits there for a track, so one is played below: the one
                // asked for, or the first.
                if !album {
                    if self.film_uri.is_none() && self.launch.is_none() {
                        return (
                            Err("no emulator for this console is installed".into()),
                            task,
                        );
                    }
                    return self.started(task);
                }
                if track.is_none() {
                    self.focus = 0;
                    self.play(0);
                    return (Ok(()), Task::batch([task, self.reveal_focus()]));
                }
            }
        }
        if self.launch.is_some() && track.is_none() && self.playing.is_none() {
            let game = self.start_game();
            return self.started(Task::batch([task, game]));
        }
        if self.film_uri.is_some() && track.is_none() {
            let film = self.start_film();
            return self.started(Task::batch([task, film]));
        }
        if !self.album.playable {
            return (Err("nothing on the stage plays".into()), task);
        }
        let count = self.album.tracks.len();
        match track {
            Some(n) if n > count => {
                return (
                    Err(format!("there is no track {n}; there are {count}")),
                    task,
                );
            }
            Some(n) => {
                self.focus = n - 1;
                self.play(n - 1);
            }
            None if self.playing.is_some() && self.paused => self.pause(false),
            None if self.playing.is_some() => {}
            None => self.play(self.focus),
        }
        (Ok(()), Task::batch([task, self.reveal_focus()]))
    }

    /// A game or film was asked to start: whether it did, and if not, why,
    /// as the note under the title says.
    fn started(&self, task: Task<Message>) -> (Result<(), String>, Task<Message>) {
        if self.in_game || self.film.is_some() {
            return (Ok(()), task);
        }
        let why = self.album.note.as_deref().map(plain);
        (Err(why.unwrap_or_else(|| "it did not start".into())), task)
    }

    /// What is on the stage, for whoever asked from outside.
    fn status(&self) -> control::Status {
        let (source, id) = match (&self.picked, &self.drive_state) {
            (Some(entry), _) => ("copy", Some(entry.meta.id.clone())),
            _ if !self.watching => ("file", None),
            (None, Some(DriveState::Disc { report, .. })) => ("drive", library::id(report)),
            (None, _) => ("drive", None),
        };
        let kind = match (&self.picked, &self.disc) {
            (Some(entry), _) if entry.is_album() || soundtrack::is_entry(entry) => "music",
            (Some(entry), _) if entry.is_film() => "film",
            (Some(_), _) => "game",
            _ if !self.watching => "music",
            (None, _) if self.film_uri.is_some() => "film",
            (None, Some(disc)) if disc.game.is_some() => "game",
            (None, Some(_)) => "music",
            (None, None) => "none",
        };
        let drive = match &self.drive_state {
            _ if !self.watching => "not-watched",
            None | Some(DriveState::NoDrive) => "no-drive",
            Some(DriveState::Empty) => "empty",
            Some(DriveState::Reading) => "reading",
            Some(DriveState::Disc { .. }) => "disc",
            Some(DriveState::Unreadable(_)) => "unreadable",
        };
        control::Status {
            window: std::process::id(),
            current: windows::is_current(),
            screen: match self.screen {
                Screen::Stage => "stage",
                Screen::Library => "library",
            }
            .into(),
            source: source.into(),
            drive: drive.into(),
            id,
            kind: kind.into(),
            title: self.album.title.clone(),
            artist: self.album.artist.clone(),
            tracks: self
                .album
                .tracks
                .iter()
                .enumerate()
                .map(|(i, t)| control::TrackStatus {
                    number: i + 1,
                    title: t.title.clone(),
                    seconds: t.seconds,
                })
                .collect(),
            playing: self.playing.map(|t| t + 1),
            paused: self.paused,
            elapsed: self.elapsed,
            in_game: self.in_game,
            in_film: self.film.is_some(),
            copying: self.copying.is_some(),
            note: self
                .album
                .note
                .as_deref()
                .map(plain)
                .filter(|n| !n.is_empty()),
        }
    }

    /// A press of the remote, for whichever screen has it.
    fn remote(&mut self, remote: Remote) -> Task<Message> {
        // Gamepads reach every program at once: while a game runs, the
        // buttons are the game's.
        if self.in_game {
            return Task::none();
        }
        if self.film.is_some() {
            return self.film_remote(remote);
        }
        // A folder's key opens it from anywhere: the stage, or the desktop.
        if let Remote::Folder(section) = remote {
            return self.open_folder(section);
        }
        if matches!(remote, Remote::Drive) {
            return self.open_drive();
        }
        match self.screen {
            Screen::Library => self.library_remote(remote),
            Screen::Stage => self.stage_remote(remote),
        }
    }

    fn library_remote(&mut self, remote: Remote) -> Task<Message> {
        // A menu is up: it has the keys until it goes.
        if let Some(menu) = self.shelf.menu {
            match remote {
                Remote::Up => self.shelf.menu_step(-1),
                Remote::Down => self.shelf.menu_step(1),
                Remote::Select | Remote::PlayPause => {
                    let index = menu.index;
                    return match self.shelf.acts(index)[menu.item] {
                        shelf::Act::Play => {
                            self.shelf.menu = None;
                            self.pick(index)
                        }
                        shelf::Act::Pin | shelf::Act::Unpin => {
                            self.shelf.toggle_pin(index);
                            Task::none()
                        }
                        shelf::Act::Delete => self.delete(index),
                    };
                }
                _ => {
                    self.shelf.menu = None;
                    self.shelf.armed = None;
                }
            }
            return Task::none();
        }
        match remote {
            Remote::Left => self.shelf.step(-1, 0),
            Remote::Right => self.shelf.step(1, 0),
            Remote::Up => self.shelf.step(0, -1),
            Remote::Down => self.shelf.step(0, 1),
            Remote::Select | Remote::PlayPause => self.activate(),
            Remote::Menu => {
                if let Some(index) = self.shelf.focused_index() {
                    self.shelf.menu = Some(shelf::Menu {
                        index,
                        at: None,
                        item: 0,
                    });
                }
                Task::none()
            }
            // Back closes the folder; on the desktop, back to the stage, if
            // anything is on it.
            Remote::Back | Remote::Library => {
                if self.shelf.close_folder() {
                    Task::none()
                } else {
                    self.close_library()
                }
            }
            Remote::Keep | Remote::Previous | Remote::Next | Remote::Folder(_) | Remote::Drive => {
                Task::none()
            }
        }
    }

    /// Enter on what is chosen: the drive's disc onto the stage, a folder
    /// open, a copy onto the stage.
    fn activate(&mut self) -> Task<Message> {
        match self.shelf.chosen() {
            Some(Spot::Drive) => self.pick_drive(),
            Some(Spot::Folder(section)) => self.open_folder(section),
            Some(Spot::Copy(index)) => self.pick(index),
            None => Task::none(),
        }
    }

    /// The drive, from its key: its disc onto the stage, or with none in,
    /// the desktop with the drive chosen, saying why.
    fn open_drive(&mut self) -> Task<Message> {
        if self.disc_in_drive() {
            return self.pick_drive();
        }
        self.shelf.close_folder();
        self.shelf.focus_drive();
        self.open_library()
    }

    /// A folder's window, from its icon, its key, or the taskbar.
    fn open_folder(&mut self, section: Section) -> Task<Message> {
        let opened = self.open_library();
        if self.screen != Screen::Library {
            return opened;
        }
        Task::batch([opened, self.shelf.open_folder(section)])
    }

    /// Whether the desktop has the keys: no window is in front of it.
    fn desk_active(&self) -> bool {
        self.screen == Screen::Library && self.shelf.open.is_none()
    }

    fn stage_remote(&mut self, remote: Remote) -> Task<Message> {
        if self.copying.is_some() {
            // The drive is busy copying: a game would only fight it for
            // reads, and the library waits until the copy is kept.
            return match remote {
                Remote::Back => {
                    if let Some(copying) = &self.copying {
                        copying.cancel.store(true, Ordering::Relaxed);
                    }
                    Task::none()
                }
                _ => Task::none(),
            };
        }
        match remote {
            Remote::Library | Remote::Menu => return self.open_library(),
            // Only the disc in the drive can be kept.
            Remote::Keep if self.picked.is_none() => return self.keep_copy(),
            Remote::Keep => return Task::none(),
            // Back from whatever is on the stage, not playing, is back to
            // the wall it was chosen from.
            Remote::Back if self.playing.is_none() => return self.open_library(),
            Remote::Select | Remote::PlayPause if self.launch.is_some() => {
                return self.start_game();
            }
            Remote::Select | Remote::PlayPause if self.film_uri.is_some() => {
                return self.start_film();
            }
            _ => {}
        }
        let count = self.album.tracks.len();
        // Down from a disc with no tracks goes down to the library, as the
        // screens are stacked. From the last track it stays: scrolling to
        // the end of a list should not leave it.
        if matches!(remote, Remote::Down) && count == 0 {
            return self.open_library();
        }
        if count == 0 {
            return Task::none();
        }
        match remote {
            Remote::Up => self.focus = self.focus.saturating_sub(1),
            Remote::Down => self.focus = (self.focus + 1).min(count - 1),
            Remote::Select => self.play(self.focus),
            Remote::PlayPause => match self.playing {
                Some(_) => self.pause(!self.paused),
                None => self.play(self.focus),
            },
            Remote::Previous | Remote::Left => self.skip(false),
            Remote::Next | Remote::Right => self.skip(true),
            Remote::Back => self.stop(),
            Remote::Keep | Remote::Menu | Remote::Library | Remote::Folder(_) | Remote::Drive => {}
        }
        self.reveal_focus()
    }

    fn pause(&mut self, paused: bool) {
        self.paused = paused;
        self.motion.set_spinning(!paused);
        if let Some(player) = &self.player {
            if paused {
                player.pause();
            } else {
                player.resume();
            }
        }
    }

    /// On to the next track or back to the one before, playing it if a
    /// track was playing. The album has tracks.
    fn skip(&mut self, forward: bool) {
        let current = self.playing.unwrap_or(self.focus);
        self.focus = if forward {
            (current + 1).min(self.album.tracks.len() - 1)
        } else {
            current.saturating_sub(1)
        };
        if self.playing.is_some() {
            self.play(self.focus);
        }
    }

    fn stop(&mut self) {
        self.playing = None;
        self.motion.set_spinning(false);
        if let Some(player) = &self.player {
            player.stop();
        }
    }

    /// Slide down to the library, with its shelf brought up to date and
    /// faces made for any copy new to it.
    fn open_library(&mut self) -> Task<Message> {
        if self.copying.is_some() || self.in_game || self.film.is_some() {
            return Task::none();
        }
        let missing = self.shelf.load(self.shelf_entries());
        self.shelf.notice = None;
        self.screen = Screen::Library;
        self.slide.go(1.0, self.reduced_motion);
        Task::batch([shelf_faces(missing), self.listen_to_games()])
    }

    /// The library as the shelf shows it: every copy, and after each game
    /// with music, its soundtrack.
    fn shelf_entries(&self) -> Vec<library::Entry> {
        library::list()
            .into_iter()
            .flat_map(|entry| {
                // Found to have music now, or the last time it was listened
                // to, while it is being listened to again.
                let music = match self.soundtracks.get(&entry.meta.id) {
                    Some(Some(_)) => true,
                    _ => {
                        entry.meta.system.is_some() && soundtrack::remembered(&entry) == Some(true)
                    }
                }
                .then(|| soundtrack::entry(&entry));
                std::iter::once(entry).chain(music)
            })
            .collect()
    }

    /// Listen for music on the kept games not yet listened to, off the UI
    /// thread; their soundtracks join the shelf when it is done.
    fn listen_to_games(&mut self) -> Task<Message> {
        let games: Vec<library::Entry> = self
            .shelf
            .entries
            .iter()
            .filter(|e| e.meta.system.is_some() && !soundtrack::is_entry(e))
            .filter(|e| !self.soundtracks.contains_key(&e.meta.id))
            .cloned()
            .collect();
        if games.is_empty() {
            return Task::none();
        }
        for game in &games {
            self.soundtracks.insert(game.meta.id.clone(), None);
        }
        Task::perform(
            blocking(move || {
                games
                    .iter()
                    .map(|game| {
                        let found = soundtrack::load(game).map(Arc::new);
                        soundtrack::remember(game, found.is_some());
                        (game.meta.id.clone(), found)
                    })
                    .collect()
            }),
            Message::Soundtracks,
        )
    }

    /// Back to the stage from the wall: only if something is on it.
    fn close_library(&mut self) -> Task<Message> {
        if self.screen == Screen::Stage || self.on_stage().is_none() {
            return Task::none();
        }
        self.screen = Screen::Stage;
        self.stage_open = true;
        self.slide.go(0.0, self.reduced_motion);
        self.shelf.menu = None;
        Task::none()
    }

    /// What is on the stage, by name: a copy chosen from the wall, or the
    /// disc in the drive. None when it would only say there is no disc.
    fn on_stage(&self) -> Option<&str> {
        if let Some(entry) = &self.picked {
            return Some(entry.meta.title.as_str());
        }
        self.disc_in_drive().then_some(self.album.title.as_str())
    }

    /// The drive as the wall's top shelf shows it.
    fn drive_shown(&self) -> DriveShown {
        if self.picked.is_some()
            && let Some(shown) = &self.drive_shown
        {
            return shown.clone();
        }
        DriveShown {
            disc: matches!(self.drive_state, Some(DriveState::Disc { .. }))
                .then(|| self.art.icon.clone()),
            title: self.album.title.clone(),
        }
    }

    /// The drive's disc, chosen on the wall: on to the stage, in place of
    /// any copy there.
    fn pick_drive(&mut self) -> Task<Message> {
        self.shelf.focus_drive();
        if !self.disc_in_drive() {
            return Task::none();
        }
        let shown = if self.picked.is_some() {
            self.return_to_drive()
        } else {
            Task::none()
        };
        Task::batch([shown, self.close_library()])
    }

    /// Put a kept copy on the stage in place of the drive's disc: an album
    /// to be played when a track is chosen, a game or a film straight away.
    fn pick(&mut self, index: usize) -> Task<Message> {
        let Some(entry) = self.shelf.entries.get(index).cloned() else {
            return Task::none();
        };
        // The stage's art and words are about to be the copy's: the wall
        // keeps the drive's, for its top shelf.
        if self.picked.is_none() {
            self.drive_shown = Some(self.drive_shown());
        }
        self.shelf.focus_entry(index);
        // On the stage before leaving the wall, which goes only to a stage
        // with something on it.
        self.picked = Some(entry.clone());
        let close = self.close_library();
        let pictures = artwork::for_copy(&entry);
        self.player = None;
        self.serial = None;
        self.disc_id = None;
        self.film_uri = None;
        self.film_label = None;
        if soundtrack::is_entry(&entry) {
            let Some(music) = self
                .soundtracks
                .get(soundtrack::game_id(&entry))
                .cloned()
                .flatten()
            else {
                return Task::none();
            };
            let soundtrack::Soundtrack {
                tracks,
                source,
                spans,
            } = Arc::unwrap_or_clone(music);
            let mut album = Album::from_soundtrack(&entry, tracks);
            if let Some(cover) = pictures.cover.or_else(|| pictures.face.clone()) {
                album.cover = cover;
            }
            album.face = pictures.face;
            self.picked = Some(entry.clone());
            album.note = Some(self.ready_note());
            self.launch = None;
            self.show(album);
            let player = self.start_player(source, spans);
            return Task::batch([close, player, self.scroll_to_top()]);
        }
        if entry.is_album() {
            let Some(toc) = entry.toc() else {
                self.shelf.notice = Some("That copy would not open".into());
                return Task::none();
            };
            let mut album = Album::from_copy(&entry, &toc);
            album.cover = pictures.cover.unwrap_or_else(album::placeholder_cover);
            self.picked = Some(entry.clone());
            album.note = Some(self.ready_note());
            self.launch = None;
            self.show(album);
            let player = self.start_player(audio::Source::Image(entry.bin()), audio::spans(&toc));
            return Task::batch([close, player, self.scroll_to_top()]);
        }
        if entry.is_film() {
            let mut album = Album::from_kept_film(&entry);
            if let Some(cover) = pictures.cover.or_else(|| pictures.face.clone()) {
                album.cover = cover;
            }
            album.face = pictures.face;
            self.picked = Some(entry.clone());
            self.launch = None;
            self.film_uri = Some(format!("dvd://{}", entry.iso().display()));
            album.note = Some(self.ready_note());
            self.show(album);
            return Task::batch([close, self.start_film()]);
        }
        let mut album = Album::from_kept_game(&entry);
        if let Some(cover) = pictures.cover.or_else(|| pictures.face.clone()) {
            album.cover = cover;
        }
        album.face = pictures.face;
        self.picked = Some(entry.clone());
        self.launch = entry
            .meta
            .system
            .and_then(game::find)
            .map(|emulator| Launch {
                emulator,
                content: entry.cue().to_string_lossy().into_owned(),
            });
        album.note = Some(self.ready_note());
        self.show(album);
        if self.launch.is_none() {
            return close;
        }
        Task::batch([close, self.start_game()])
    }

    /// Delete pressed on a copy: the first press arms it, the second
    /// throws the copy away.
    fn delete(&mut self, index: usize) -> Task<Message> {
        if self
            .shelf
            .entries
            .get(index)
            .is_some_and(soundtrack::is_entry)
        {
            self.shelf.menu = None;
            self.shelf.notice = Some("A soundtrack goes when its game is deleted".into());
            return Task::none();
        }
        let Some(entry) = self.shelf.delete(index) else {
            // Armed: it disarms by itself if nobody confirms.
            let Some(id) = self.shelf.armed.clone() else {
                return Task::none();
            };
            return Task::perform(blocking(|| std::thread::sleep(ARMED)), move |()| {
                Message::Disarm(id.clone())
            });
        };
        if let Err(e) = library::remove(&entry) {
            self.shelf.notice = Some(format!("Couldn't delete that copy: {e}"));
            return Task::none();
        }
        let mut task = Task::none();
        // The copy itself on the stage, or its soundtrack: both are gone.
        if self.picked.as_ref().is_some_and(|p| p.dir == entry.dir) {
            task = self.return_to_drive();
        } else if self.picked.is_none() && self.disc.is_some() {
            // The disc in the drive may have been the copy: it plays from the
            // drive again.
            self.launch = self.disc_launch();
            self.film_uri = self.drive_state.as_ref().and_then(film_uri);
            self.album.note = Some(self.ready_note());
        }
        let _ = self.shelf.load(self.shelf_entries());
        task
    }

    /// Put the drive's disc on the stage, with whatever it needs started:
    /// its player, its names, its pictures.
    fn show_drive(&mut self, state: DriveState) -> Task<Message> {
        let mut album = Album::from_drive(&state);
        // A new disc, or none: whatever was playing stops.
        self.player = None;
        self.film = None;
        self.disc_id = None;
        self.picked = None;
        let mut audio_tasks = Vec::new();
        if let DriveState::Disc { report, drive } = &state
            && let DiscKind::Audio { musicbrainz, .. } = &report.kind
            && let Some(toc) = &report.toc
        {
            // A kept CD plays from its copy, named as it was kept:
            // the drive can rest, and nothing is looked up.
            let kept = library::id(report)
                .and_then(|id| library::find(&id))
                .and_then(|entry| Some((entry.toc()?, entry)));
            let look_up = kept.is_none();
            if let Some((toc, entry)) = kept {
                album = Album::from_copy(&entry, &toc);
                if let Some(cover) = artwork::for_copy(&entry).cover {
                    album.cover = cover;
                }
                audio_tasks
                    .push(self.start_player(audio::Source::Image(entry.bin()), audio::spans(&toc)));
            } else {
                audio_tasks.push(
                    self.start_player(audio::Source::Drive(drive.clone()), audio::spans(toc)),
                );
            }
            if look_up && let Some(id) = musicbrainz.clone() {
                self.disc_id = Some(id.disc_id.clone());
                audio_tasks.push(Task::perform(
                    blocking(move || Box::new(musicbrainz::look_up(&id))),
                    {
                        let disc_id = self.disc_id.clone().unwrap_or_default();
                        move |found| Message::Release(disc_id.clone(), found)
                    },
                ));
            }
        }
        self.film_uri = film_uri(&state);
        // A DVD's name and pictures follow, from the cache or the network.
        let mut film_task = None;
        self.film_label = None;
        if let DriveState::Disc { report, .. } = &state
            && let DiscKind::DvdVideo { feature_seconds } = report.kind
            && let Some(label) = report.label.clone()
        {
            self.film_label = Some(label.clone());
            film_task = Some(Task::perform(
                blocking({
                    let label = label.clone();
                    move || Box::new(filmdb::look_up(&label, feature_seconds))
                }),
                move |found| Message::FilmFound(label.clone(), found),
            ));
        }
        self.disc = match state {
            DriveState::Disc { report, drive } => match &report.kind {
                DiscKind::Game(game) => Some(Inserted {
                    game: Some(game.clone()),
                    report,
                    drive,
                }),
                DiscKind::Audio { .. } | DiscKind::DvdVideo { .. } => Some(Inserted {
                    game: None,
                    report,
                    drive,
                }),
                _ => None,
            },
            _ => None,
        };
        self.launch = self.disc_launch();
        let music = self
            .disc
            .as_ref()
            .is_some_and(|d| matches!(d.report.kind, DiscKind::Audio { .. }));
        let emulator = self
            .disc
            .as_ref()
            .and_then(|d| d.game.as_ref())
            .is_some_and(|g| game::find(g.system).is_some());
        if self.launch.is_some() || music || emulator || self.film_uri.is_some() {
            album.note = Some(self.ready_note());
        }
        self.show(album);
        self.serial = self
            .disc
            .as_ref()
            .and_then(|d| d.game.as_ref()?.serial.clone());
        // The disc goes on screen now; its pictures follow when they
        // arrive, from the cache or the network.
        let pictures = self.disc.as_ref().and_then(|disc| {
            let game = disc.game.clone()?;
            let serial = game.serial.clone()?;
            Some(Task::perform(
                blocking(move || artwork::for_game(&game)),
                move |pictures| Message::Pictures(serial.clone(), pictures),
            ))
        });
        let top = self.scroll_to_top();
        Task::batch(
            [top]
                .into_iter()
                .chain(pictures)
                .chain(film_task)
                .chain(audio_tasks),
        )
    }

    /// How the disc in the drive would be played: from its kept copy if
    /// there is one, which is quicker and quieter, or else from the drive.
    fn disc_launch(&self) -> Option<Launch> {
        let disc = self.disc.as_ref()?;
        let emulator = game::find(disc.game.as_ref()?.system)?;
        let kept = library::id(&disc.report)
            .and_then(|id| library::find(&id))
            .map(|entry| entry.cue().to_string_lossy().into_owned());
        let content = kept.or_else(|| {
            emulator
                .reads_drive
                .then(|| game::cdrom_uri(&disc.drive))
                .flatten()
        })?;
        Some(Launch { emulator, content })
    }

    /// What can be done now, for the line under the title. Buttons are
    /// written `{accept}`, and drawn as the controller in hand has them.
    fn ready_note(&self) -> String {
        if let Some(entry) = &self.picked {
            return if entry.is_album() || soundtrack::is_entry(entry) {
                "Your copy  ·  {accept} Play".into()
            } else if entry.is_film() {
                "Your copy  ·  {accept} Play in VLC".into()
            } else if self.launch.is_none() {
                "No emulator for this console is installed".into()
            } else {
                "Your copy  ·  {accept} Play".into()
            };
        }
        if self.film_uri.is_some() {
            if !film::installed() {
                return "Films play in VLC, which isn't installed".into();
            }
            let id = self.disc.as_ref().and_then(|d| library::id(&d.report));
            return match id.map(|id| library::find(&id).is_some()) {
                Some(true) => "{accept} Play in VLC  ·  Kept: plays without the disc".into(),
                Some(false) => "{accept} Play in VLC  ·  {alt} Keep a copy".into(),
                None => "{accept} Play in VLC".into(),
            };
        }
        let Some(disc) = &self.disc else {
            return String::new();
        };
        if let Some(game) = &disc.game
            && self.launch.is_none()
        {
            // An emulator that only plays copies.
            return match (game::find(game.system), library::id(&disc.report)) {
                (Some(_), Some(_)) => "{alt} Keep a copy, then play it from the copy".into(),
                _ => "No emulator for this console is installed".into(),
            };
        }
        match library::id(&disc.report) {
            Some(id) if library::find(&id).is_some() => {
                "{accept} Play  ·  Kept: plays without the disc".into()
            }
            Some(_) => "{accept} Play  ·  {alt} Keep a copy".into(),
            None => "{accept} Play".into(),
        }
    }

    /// A disc is in the drive, read or not, for Back to return to.
    fn disc_in_drive(&self) -> bool {
        matches!(
            self.drive_state,
            Some(DriveState::Disc { .. } | DriveState::Unreadable(_))
        )
    }

    /// The disc in the drive could be kept now, from the stage.
    fn keepable(&self) -> bool {
        self.picked.is_none()
            && self.copying.is_none()
            && !self.in_game
            && self.film.is_none()
            && self.disc.as_ref().is_some_and(|disc| {
                library::id(&disc.report).is_some_and(|id| library::find(&id).is_none())
            })
    }

    /// Put the drive's disc back on the stage in place of a copy, as the
    /// drive is now.
    fn return_to_drive(&mut self) -> Task<Message> {
        self.picked = None;
        self.player = None;
        let state = self.drive_state.clone().unwrap_or(DriveState::Empty);
        self.show_drive(state)
    }

    fn start_player(&mut self, source: audio::Source, spans: Vec<audio::Span>) -> Task<Message> {
        let (tx, rx) = iced::futures::channel::mpsc::unbounded();
        self.player = Some(audio::Player::new(source, spans, tx));
        Task::run(rx, Message::Audio)
    }

    fn scroll_to_top(&self) -> Task<Message> {
        operation::scroll_to(
            Id::from(TRACKS),
            AbsoluteOffset {
                x: None,
                y: Some(0.0),
            },
        )
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
        let drive_lock = match windows::drive_lock(&disc.drive, "being kept") {
            Ok(lock) => lock,
            Err(why) => {
                self.album.note = Some(why);
                return Task::none();
            }
        };
        // Reading the disc for two things at once would make both stutter.
        if let Some(player) = &self.player {
            player.stop();
            self.playing = None;
            self.paused = false;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.copying = Some(Copying {
            cancel: cancel.clone(),
            started: Instant::now(),
        });
        self.motion.set_spinning(true);
        self.album.note = Some("Keeping a copy  ·  {back} Stop".into());
        let (report, drive, game) = (disc.report.clone(), disc.drive.clone(), disc.game.clone());
        Task::run(
            iced::stream::channel(4, async move |mut output| {
                let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
                std::thread::spawn(move || {
                    // Held until the copy is done, kept or not.
                    let _drive = drive_lock;
                    let mut last = u64::MAX;
                    // An album is kept by its names, which are cached from
                    // when it went in.
                    let release = match &report.kind {
                        DiscKind::Audio {
                            musicbrainz: Some(id),
                            ..
                        } => musicbrainz::names(id),
                        _ => None,
                    };
                    // A film by what the lookup found when it went in.
                    let film = match (&report.kind, &report.label) {
                        (DiscKind::DvdVideo { feature_seconds }, Some(label)) => {
                            Some((label.clone(), *feature_seconds))
                        }
                        _ => None,
                    };
                    let names = match &film {
                        Some((label, feature)) => filmdb::names(label, *feature),
                        None => release.as_ref().map(musicbrainz::Release::names),
                    };
                    let progress = |p: spectra_core::copy::Progress| {
                        // A message per percent, not per read.
                        let percent = u64::from(p.done) * 100 / u64::from(p.total.max(1));
                        if percent != last {
                            last = percent;
                            let _ = tx.unbounded_send(Keeping::Progress(p.done, p.total));
                        }
                    };
                    let result = if film.is_some() {
                        film::block_node(&drive)
                            .ok_or_else(|| {
                                spectra_core::Error::Unsupported("no /dev/sr for the drive".into())
                            })
                            .and_then(|block| {
                                library::keep_dvd(
                                    &block,
                                    &report,
                                    names.as_ref(),
                                    &cancel,
                                    progress,
                                )
                            })
                    } else {
                        Drive::open(&drive).and_then(|drive| {
                            library::keep(&drive, &report, names.as_ref(), &cancel, progress)
                        })
                    };
                    if let Ok(entry) = &result {
                        match (&game, &release, &film) {
                            (Some(game), _, _) => artwork::store(&entry.dir, game),
                            (None, Some(release), _) => musicbrainz::store(&entry.dir, release),
                            (None, None, Some((label, feature))) => {
                                filmdb::store(&entry.dir, label, *feature);
                            }
                            (None, None, None) => {}
                        }
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
        // Straight from the drive: hold it, so another window does not
        // read it under the game.
        let lock = match self.drive_for(launch.content.starts_with("cdrom://")) {
            Ok(lock) => lock,
            Err(why) => {
                self.album.note = Some(why);
                return Task::none();
            }
        };
        let mut child = match launch.emulator.launch(&launch.content) {
            Ok(child) => child,
            Err(e) => {
                self.album.note = Some(format!("Couldn't start RetroArch: {e}"));
                return Task::none();
            }
        };
        self.drive_lock = lock;
        windows::claim();
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

    /// Hand the film in the drive to VLC, which takes the screen until it
    /// is over.
    fn start_film(&mut self) -> Task<Message> {
        let Some(uri) = &self.film_uri else {
            return Task::none();
        };
        // A kept copy is a file; anything else is read from the drive.
        let lock = match self.drive_for(!uri.ends_with(".iso")) {
            Ok(lock) => lock,
            Err(why) => {
                self.album.note = Some(why);
                return Task::none();
            }
        };
        let (tx, rx) = iced::futures::channel::oneshot::channel();
        match film::Film::start(uri, move |trouble| {
            let _ = tx.send(trouble);
        }) {
            Ok(film) => self.film = Some(film),
            Err(e) => {
                self.album.note = Some(format!("Couldn't start VLC: {e}"));
                return Task::none();
            }
        }
        self.drive_lock = lock;
        windows::claim();
        self.motion.set_spinning(true);
        self.album.note = Some("Playing in VLC  ·  Esc to come back".into());
        Task::perform(rx, |trouble| Message::FilmOver(trouble.ok().flatten()))
    }

    /// The gamepad, while VLC has the screen. The keyboard reaches VLC
    /// itself; this is the pad, which Spectra hears wherever focus is.
    fn film_remote(&mut self, remote: Remote) -> Task<Message> {
        let key = match remote {
            Remote::Up => film::Key::Up,
            Remote::Down => film::Key::Down,
            Remote::Left => film::Key::Left,
            Remote::Right => film::Key::Right,
            Remote::Select => film::Key::Activate,
            Remote::PlayPause => film::Key::PlayPause,
            Remote::Previous => film::Key::PreviousChapter,
            Remote::Next => film::Key::NextChapter,
            Remote::Menu => film::Key::DiscMenu,
            // Back always gets out: dropping the film ends VLC.
            Remote::Back => {
                self.film = None;
                return Task::none();
            }
            Remote::Keep | Remote::Library | Remote::Folder(_) | Remote::Drive => {
                return Task::none();
            }
        };
        if let Some(film) = &self.film {
            film.press(key);
        }
        Task::none()
    }

    /// The drive, held for this window if `reading` it. Refused, in words,
    /// when another window holds it.
    fn drive_for(&self, reading: bool) -> Result<Option<spectra_core::lock::Lock>, String> {
        match &self.drive_state {
            Some(DriveState::Disc { drive, .. }) if reading => {
                windows::drive_lock(drive, "playing").map(Some)
            }
            _ => Ok(None),
        }
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
        // The stage's disc goes on turning under the library, but nothing
        // is drawn for it while it cannot be seen.
        let stage = self.screen == Screen::Stage || self.slide.moving();
        self.slide.moving()
            || (stage
                && (self.motion.moving()
                    || self
                        .glow
                        .iter()
                        .enumerate()
                        .any(|(i, &g)| g != if i == self.focus { 1.0 } else { 0.0 })))
    }

    fn subscription(&self) -> Subscription<Message> {
        let remote = Subscription::batch([
            keyboard::listen().filter_map(key).map(Message::Remote),
            gamepad::subscription().map(|(pad, remote)| Message::Pad(pad, remote)),
            control::subscription()
                .map(|(request, responder)| Message::Control(request, responder)),
            window::events().filter_map(|(_, event)| {
                matches!(event, window::Event::Focused).then_some(Message::Focused)
            }),
        ]);
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

    /// The desktop: the sky, its icons, the window in front of them if one
    /// is open - a folder, or the stage - and the taskbar along the bottom.
    fn view(&self) -> Element<'_, Message> {
        let sky = image(theme::backdrop())
            .width(Fill)
            .height(Fill)
            .content_fit(ContentFit::Cover)
            .filter_method(image::FilterMethod::Nearest);
        let desk = responsive(move |size| {
            let cx = self.desk_context();
            let margin = (size.width.min(size.height) * 0.035).clamp(8.0, 32.0);
            // Room on the right and below for the window's hard shadow.
            // Every window the one size: the desktop, less a margin and room
            // for its shadow, and no wider than a track list needs.
            let standard = Size::new(
                (size.width - 2.0 * margin - 6.0).clamp(1.0, 1280.0),
                (size.height - 2.0 * margin - 6.0).max(1.0),
            );
            let window = match self.screen {
                Screen::Library => self
                    .shelf
                    .folder_window(standard, &cx)
                    .map(|(title, contents)| (standard, title, contents, Message::CloseFolder)),
                Screen::Stage => Some((
                    standard,
                    self.album.title.clone(),
                    self.stage(),
                    Message::CloseStage,
                )),
            };
            let mut layers = stack![self.shelf.desktop(size, &cx)];
            if let Some((body, title, contents, close)) = window {
                layers = layers.push(
                    container(
                        container(theme::window(title, contents, Some(close)))
                            .width(body.width)
                            .height(body.height),
                    )
                    .center(Fill),
                );
            }
            if let Some(menu) = self.shelf.menu {
                layers = layers.push(self.shelf.menu_view(menu, size));
            }
            // The one place the pointer is followed from, so a menu opens
            // where it was asked for, whatever it was asked of.
            mouse_area(layers).on_move(Message::Pointer).into()
        });
        stack![sky, column![desk, self.taskbar()]].into()
    }

    /// What the desktop and its folders need to draw themselves.
    fn desk_context(&self) -> shelf::Context<'_> {
        let shown = self.drive_shown();
        let title = match &self.drive_state {
            None | Some(DriveState::NoDrive) => "No drive".to_string(),
            Some(DriveState::Empty) => "Drive: empty".into(),
            Some(DriveState::Reading) => "Reading…".into(),
            Some(DriveState::Unreadable(_)) => "Can't read it".into(),
            Some(DriveState::Disc { .. }) => shown.title,
        };
        shelf::Context {
            style: self.style,
            picked: self.picked.as_ref().map(|p| p.meta.id.as_str()),
            pointing: self.pointing,
            desk_active: self.desk_active(),
            drive: shelf::Drive {
                icon: shown.disc,
                title,
            },
        }
    }

    /// The taskbar: Spectra's button, which shows the desktop, and a button
    /// for each window, the one in front pressed in. The drive is on the
    /// desktop, and D: one place for it is enough.
    fn taskbar(&self) -> Element<'_, Message> {
        let start = button(
            row![
                text("◉").font(FONT).size(14).color(Color::WHITE),
                text("SPECTRA").font(BOLD).size(13).color(Color::WHITE),
            ]
            .spacing(6)
            .align_y(Vertical::Center),
        )
        .padding([6, 14])
        .on_press(Message::ShowDesktop)
        .style(|_, status| button::Style {
            background: Some(Background::Color(
                if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                    Color::from_rgb8(0x3c, 0xb8, 0x52)
                } else {
                    Color::from_rgb8(0x2e, 0x9e, 0x44)
                },
            )),
            border: Border {
                color: theme::INK,
                width: 2.0,
                radius: 999.0.into(),
            },
            shadow: theme::hard_shadow(2.0),
            ..button::Style::default()
        });
        let folder = self.shelf.open.map(|section| {
            theme::button_text(
                section.name(),
                Some(Message::OpenFolder(section)),
                self.screen == Screen::Library,
            )
        });
        let stage = self.on_stage().filter(|_| self.stage_open).map(|title| {
            let playing = if self.playing.is_some() { "▶ " } else { "" };
            theme::button_text(
                &format!("{playing}{}", shorten(title, 18)),
                Some(Message::CloseLibrary),
                self.screen == Screen::Stage,
            )
        });
        container(
            row![start]
                .push(folder)
                .push(stage)
                .spacing(8)
                .align_y(Vertical::Center),
        )
        .padding([5, 8])
        .width(Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::SILVER)),
            border: Border {
                color: Color::WHITE,
                width: 0.0,
                radius: 0.0.into(),
            },
            shadow: Shadow {
                color: Color::WHITE,
                offset: Vector::new(0.0, -2.0),
                blur_radius: 0.0,
            },
            ..Default::default()
        })
        .into()
    }

    /// The stage, in its window: the disc and what is on it, and a bar of
    /// what can be done with it along the bottom.
    fn stage(&self) -> Element<'_, Message> {
        mouse_area(column![
            container(responsive(|size| self.screen(size))).height(Fill),
            self.actions(),
        ])
        .into()
    }

    /// Along the bottom of the stage's window: keep a copy of the disc, and
    /// back to your discs.
    fn actions(&self) -> Element<'_, Message> {
        let label = |pad: &str, words: &str| -> Element<'_, Message> {
            if self.pointing {
                text(words.to_uppercase()).font(FONT).size(12).into()
            } else {
                ui::prompt(
                    &format!("{pad} {}", words.to_uppercase()),
                    self.style,
                    12.0,
                    theme::faint(0.8),
                )
            }
        };
        let mut buttons = row![].spacing(10).align_y(Vertical::Center);
        if self.keepable() {
            buttons = buttons.push(theme::button_with(
                label("{alt}", "Keep a copy"),
                Some(Message::KeepCopy),
                false,
            ));
        }
        if self.copying.is_none() && !self.in_game && self.film.is_none() {
            buttons = buttons.push(theme::button_with(
                label("{back}", "My discs"),
                Some(Message::OpenLibrary),
                false,
            ));
        }
        container(buttons)
            .padding([8, 10])
            .width(Fill)
            .align_right(Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(theme::SILVER)),
                ..Default::default()
            })
            .into()
    }

    /// Everything in the stage's window, arranged for the room there is.
    fn screen(&self, size: Size) -> Element<'_, Message> {
        let scale = Scale::for_size(size);
        match Layout::for_size(size) {
            Layout::Beside { disc } => {
                let album = column![
                    self.header(scale),
                    self.tracks(scale),
                    self.player_bar(scale),
                    self.hints(scale)
                ]
                .spacing(scale.gap)
                .max_width(560)
                // As tall as what is in it, to be centred.
                .height(Shrink);
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
                    column![
                        self.header(scale),
                        self.tracks(scale),
                        self.player_bar(scale),
                        self.hints(scale)
                    ]
                    .spacing(scale.gap)
                    .max_width(560)
                    .height(Shrink),
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
                    self.player_bar(scale),
                ]
                .spacing(scale.gap),
            )
            .padding([16, 14])
            .into(),
            Layout::Controls => self.controls(size),
        }
    }

    fn disc(&self) -> iced::widget::Shader<Message, disc::Disc> {
        shader(disc::Disc {
            slot: STAGE,
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
                .color(theme::faint((alpha + 0.2).min(1.0)))
        };
        container(
            column![
                text(&album.title)
                    .font(BOLD)
                    .size(scale.title)
                    .color(theme::INK),
                text(&album.artist)
                    .font(FONT)
                    .size(scale.artist)
                    .color(theme::faint(0.93)),
            ]
            .push(details.map(|details| dim(&details, 0.5)))
            .push(
                album
                    .note
                    .as_deref()
                    .map(|note| ui::prompt(note, self.style, scale.detail, theme::faint(0.53))),
            )
            .spacing(scale.gap / 4.0),
        )
        .padding([0.0, LIST_PAD])
        .into()
    }

    fn tracks(&self, scale: Scale) -> Element<'_, Message> {
        let rows = self.album.tracks.iter().enumerate().map(|(i, track)| {
            let glow = self.glow[i];
            let is_playing = self.playing == Some(i);
            let marker = match (is_playing, self.paused) {
                (true, false) => "▶".to_string(),
                (true, true) => "❚❚".to_string(),
                _ => format!("{}", i + 1),
            };
            // The chosen row is picked out in blue, its words white, as a
            // chosen row in a list box was.
            let mix = |a: Color, b: Color| Color {
                r: a.r + (b.r - a.r) * glow,
                g: a.g + (b.g - a.g) * glow,
                b: a.b + (b.b - a.b) * glow,
                a: a.a + (b.a - a.a) * glow,
            };
            let white = Color::from_rgb(1.0, 1.0, 1.0);
            let strong = mix(theme::INK, white);
            let quiet = mix(theme::faint(0.55), Color { a: 0.8, ..white });
            let line = row![
                text(marker)
                    .font(FONT)
                    .size(scale.track - 3.0)
                    .width(scale.track * 1.9)
                    .color(if is_playing {
                        mix(theme::BLUE, white)
                    } else {
                        strong
                    }),
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
                    if is_playing && self.player.is_some() {
                        format!("{} / {}", clock(self.elapsed), clock(track.seconds))
                    } else {
                        clock(track.seconds)
                    }
                }))
                .font(FONT)
                .size(scale.track - 3.0)
                .color(quiet),
            ]
            .spacing(8)
            .align_y(Vertical::Center);
            let row = container(line)
                .padding([0.0, scale.row * 0.36])
                .width(Fill)
                .height(scale.row)
                .center_y(scale.row)
                .style(move |_| container::Style {
                    background: Some(Background::Color(theme::BLUE.scale_alpha(glow))),
                    ..Default::default()
                });
            mouse_area(row).on_press(Message::Pick(i)).into()
        });
        let list = scrollable(column(rows).spacing(ROW_GAP).padding(LIST_PAD))
            .id(TRACKS)
            .on_scroll(Message::Scrolled)
            .direction(Direction::Vertical(
                Scrollbar::new().width(8).scroller_width(8).margin(2),
            ))
            .style(|look, status| {
                let mut style = scrollable::default(look, status);
                style.vertical_rail.scroller.background = Background::Color(theme::SILVER);
                style.vertical_rail.scroller.border = Border {
                    color: theme::INK,
                    width: 2.0,
                    radius: 0.0.into(),
                };
                style
            });
        // Filling what the header and the player leave, so a long list
        // scrolls rather than pushing the player off the screen; but no
        // taller than its rows, so a short one does not stretch.
        let count = self.album.tracks.len() as f32;
        let rows = count * scale.row + (count - 1.0).max(0.0) * ROW_GAP + 2.0 * LIST_PAD;
        container(list).height(Fill).max_height(rows).into()
    }

    fn hints(&self, scale: Scale) -> Element<'_, Message> {
        if scale.gap < 20.0 || !self.album.playable || self.album.tracks.is_empty() {
            return space().into();
        }
        container(ui::prompt(
            "{up}{down} Choose    {accept} Play    {prev}{next} Skip    {start} Pause    {back} Stop",
            self.style,
            13.0,
            theme::faint(0.55),
        ))
        .padding([0.0, LIST_PAD])
        .into()
    }

    /// The track playing, or the one that would if Play were pressed, and
    /// how many seconds into it.
    fn now(&self) -> Option<(&album::Track, u32)> {
        if !self.album.playable {
            return None;
        }
        let track = self.album.tracks.get(self.playing.unwrap_or(self.focus))?;
        Some((
            track,
            if self.playing.is_some() {
                self.elapsed
            } else {
                0
            },
        ))
    }

    /// Previous, play or pause, and next, as buttons.
    fn transport(&self) -> iced::widget::Row<'_, Message> {
        let control = |glyph: &'static str, remote: Remote| {
            theme::button_with(
                text(glyph).font(FONT).size(13).center(),
                Some(Message::Remote(remote)),
                false,
            )
            .width(40)
            .height(36)
            .padding(0)
        };
        let playing = self.playing.is_some() && !self.paused;
        row![
            control("❚◀", Remote::Previous),
            control(if playing { "❚❚" } else { "▶" }, Remote::PlayPause),
            control("▶❚", Remote::Next),
        ]
        .spacing(4)
    }

    /// How far into the track, as a bar.
    fn progress(&self) -> Option<Element<'_, Message>> {
        let (track, elapsed) = self.now()?;
        let total = track.seconds.max(1);
        Some(
            progress_bar(0.0..=total as f32, elapsed.min(total) as f32)
                .girth(10)
                .style(move |_| progress_bar::Style {
                    background: Background::Color(Color::WHITE),
                    bar: Background::Color(theme::BLUE),
                    border: Border {
                        color: theme::INK,
                        width: 2.0,
                        radius: 0.0.into(),
                    },
                })
                .into(),
        )
    }

    /// The player under the track list: the buttons, what is playing, how
    /// far into it, and its time against its length.
    fn player_bar(&self, scale: Scale) -> Element<'_, Message> {
        let Some((track, elapsed)) = self.now() else {
            return space().into();
        };
        let dim = theme::faint(0.75);
        let about = column![
            row![
                container(
                    text(&track.title)
                        .font(FONT)
                        .size(scale.detail)
                        .wrapping(Wrapping::None)
                        .color(theme::INK),
                )
                .width(Fill)
                .clip(true),
                text(format!("{} / {}", clock(elapsed), clock(track.seconds)))
                    .font(FONT)
                    .size(scale.detail)
                    .color(dim),
            ]
            .spacing(8),
        ]
        .push(self.progress())
        .spacing(6)
        .width(Fill);
        container(
            row![self.transport(), about]
                .spacing(14)
                .align_y(Vertical::Center),
        )
        .padding([0.0, LIST_PAD])
        .into()
    }

    /// The smallest layout: the cover, what is playing (or would play), and
    /// buttons for the mouse.
    fn controls(&self, size: Size) -> Element<'_, Message> {
        let now = self.playing.unwrap_or(self.focus);
        let title = self
            .album
            .tracks
            .get(now)
            .map_or(self.album.title.as_str(), |t| t.title.as_str());
        let buttons = self.transport();
        let line = |text_size: f32| {
            column![
                text(title)
                    .font(BOLD)
                    .size(text_size)
                    .wrapping(Wrapping::None)
                    .color(theme::INK),
                text(&self.album.artist)
                    .font(FONT)
                    .size(text_size - 2.0)
                    .wrapping(Wrapping::None)
                    .color(theme::faint(0.85)),
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
            let line = line(15.0).push(self.progress());
            side.push(container(line).width(Fill).clip(true))
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

/// A title cut to fit a taskbar button, as taskbars cut them: `Midnight
/// Club - St…`.
fn shorten(title: &str, most: usize) -> String {
    if title.chars().count() <= most {
        return title.to_string();
    }
    let cut: String = title.chars().take(most - 1).collect();
    format!("{}…", cut.trim_end())
}

/// Seconds as a player shows them: `4:28`.
fn clock(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Faces for copies new to the shelf, made off the UI thread.
fn shelf_faces(missing: Vec<library::Entry>) -> Task<Message> {
    Task::batch(missing.into_iter().map(|entry| {
        let id = entry.meta.id.clone();
        Task::perform(
            blocking(move || {
                let face = artwork::face_for_copy(&entry);
                Some(Arc::new(face))
            }),
            move |face| Message::ShelfFace(id.clone(), face),
        )
    }))
}

/// What VLC opens for the film in the drive: its kept copy if there is one,
/// which is quicker and quieter, or else the disc.
fn film_uri(state: &DriveState) -> Option<String> {
    let DriveState::Disc { report, drive } = state else {
        return None;
    };
    let kept = library::id(report)
        .and_then(|id| library::find(&id))
        .filter(library::Entry::is_film);
    match kept {
        Some(entry) => Some(format!("dvd://{}", entry.iso().display())),
        None => film::uri(&report.kind, drive),
    }
}

/// A note with its buttons named as keys, for the command line:
/// `"{accept} Play"` is `"Enter Play"`.
fn plain(note: &str) -> String {
    note.replace("{accept}", "Enter")
        .replace("{back}", "Backspace")
        .replace("{alt}", "C")
        .replace("{library}", "L")
        .replace("{select}", "M")
}

fn key(event: keyboard::Event) -> Option<Remote> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };
    match key.as_ref() {
        Key::Named(Named::ArrowUp) => Some(Remote::Up),
        Key::Named(Named::ArrowDown) => Some(Remote::Down),
        Key::Named(Named::ArrowLeft) => Some(Remote::Left),
        Key::Named(Named::ArrowRight) => Some(Remote::Right),
        Key::Named(Named::PageUp) => Some(Remote::Previous),
        Key::Named(Named::PageDown) => Some(Remote::Next),
        Key::Named(Named::Delete) => Some(Remote::Menu),
        Key::Named(Named::Enter) => Some(Remote::Select),
        Key::Named(Named::Space) => Some(Remote::PlayPause),
        Key::Named(Named::Backspace) => Some(Remote::Back),
        // Escape is Back, as it is in most windows; closing Spectra is the
        // desktop's business (Super+W, say).
        Key::Named(Named::Escape) => Some(Remote::Back),
        Key::Character("c") => Some(Remote::Keep),
        Key::Character("l") => Some(Remote::Library),
        Key::Character("m") => Some(Remote::Menu),
        Key::Character("s") => Some(Remote::Folder(Section::Sounds)),
        Key::Character("v") => Some(Remote::Folder(Section::Videos)),
        Key::Character("g") => Some(Remote::Folder(Section::Games)),
        Key::Character("d") => Some(Remote::Drive),
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    prefer_vulkan();
    iced::application(Spectra::new, Spectra::update, Spectra::view)
        .title("Spectra")
        .subscription(Spectra::subscription)
        .theme(|_: &Spectra| Theme::Light)
        .font(theme::FONTS[0])
        .font(theme::FONTS[1])
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

    #[test]
    fn a_long_title_is_cut_with_an_ellipsis_and_a_short_one_is_not() {
        assert_eq!(
            shorten("Midnight Club - Street Racing", 14),
            "Midnight Club…"
        );
        assert_eq!(shorten("Tomb Raider", 14), "Tomb Raider");
    }

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
