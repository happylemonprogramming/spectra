//! Spectra: put a disc in, and it plays.
//!
//! Phase 0's UI spike: one finished screen - the rainbow disc, the album's
//! blurred cover behind it, and a track list driven by keyboard or gamepad -
//! to find out whether a native GPU UI can look right while staying within
//! the budgets in PLAN.md.
//!
//!   spectra [ALBUM.json] [--play] [--reduced-motion]

mod album;
mod art;
mod disc;
mod gamepad;
mod motion;

use std::path::PathBuf;
use std::time::Instant;

use iced::alignment::Vertical;
use iced::font::Weight;
use iced::gradient::Linear;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{column, container, image, row, shader, space, stack, text};
use iced::{
    Background, Border, Color, ContentFit, Element, Fill, FillPortion, Font, Radians, Shadow,
    Subscription, Task, Theme, Vector, window,
};

use album::Album;
use art::Art;
use motion::Motion;

const FONT: Font = Font::with_name("Adwaita Sans");
const BOLD: Font = Font {
    weight: Weight::Bold,
    ..FONT
};
/// Rows visible at once; the list scrolls to keep the focus inside it.
const VISIBLE_ROWS: usize = 10;
/// A CD's track pitch in nanometres.
const CD_PITCH: f32 = 1600.0;

/// The remote control: keyboard and gamepads both speak it.
#[derive(Debug, Clone, Copy)]
pub enum Remote {
    Up,
    Down,
    Select,
    PlayPause,
    Back,
    Quit,
}

#[derive(Debug, Clone)]
enum Message {
    Remote(Remote),
    Frame(Instant),
}

struct Spectra {
    album: Album,
    art: Art,
    motion: Motion,
    focus: usize,
    /// First visible row.
    scroll: usize,
    playing: Option<usize>,
    paused: bool,
    /// Per row, 0 to 1: how strongly it glows as the focus.
    glow: Vec<f32>,
    last_frame: Option<Instant>,
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
            None => Album::placeholder(),
        };
        let art = Art::new(&album.cover);
        let mut glow = vec![0.0; album.tracks.len()];
        if let Some(first) = glow.first_mut() {
            *first = 1.0;
        }
        let mut app = Self {
            album,
            art,
            motion: Motion::new(options.reduced_motion),
            focus: 0,
            scroll: 0,
            playing: None,
            paused: false,
            glow,
            last_frame: None,
        };
        if options.play {
            app.play(0);
        }
        (app, Task::none())
    }

    fn play(&mut self, track: usize) {
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
            }
            Message::Remote(remote) => {
                let count = self.album.tracks.len();
                match remote {
                    Remote::Up => self.focus = self.focus.saturating_sub(1),
                    Remote::Down => self.focus = (self.focus + 1).min(count.saturating_sub(1)),
                    Remote::Select if count > 0 => self.play(self.focus),
                    Remote::PlayPause => match self.playing {
                        Some(_) => {
                            self.paused = !self.paused;
                            self.motion.set_spinning(!self.paused);
                        }
                        None if count > 0 => self.play(self.focus),
                        None => {}
                    },
                    Remote::Back => {
                        self.playing = None;
                        self.motion.set_spinning(false);
                    }
                    Remote::Quit => return iced::exit(),
                    Remote::Select => {}
                }
                if self.focus < self.scroll {
                    self.scroll = self.focus;
                } else if self.focus >= self.scroll + VISIBLE_ROWS {
                    self.scroll = self.focus + 1 - VISIBLE_ROWS;
                }
            }
        }
        Task::none()
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
        // Frames are only asked for while something moves. At rest, nothing
        // is drawn and the GPU sleeps.
        if self.animating() {
            Subscription::batch([remote, window::frames().map(Message::Frame)])
        } else {
            remote
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let accent = Color::from_rgb(self.art.accent[0], self.art.accent[1], self.art.accent[2]);

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

        let disc = shader(disc::Disc {
            pose: self.motion.pose(),
            label: self.art.label.clone(),
            accent: self.art.accent,
            pitch: CD_PITCH,
        })
        .width(FillPortion(11))
        .height(Fill);

        let album = &self.album;
        let total = album.total_seconds();
        let details = [
            album.year.map(|y| y.to_string()),
            Some(format!("{} tracks", album.tracks.len())),
            Some(format!("{} min", total.div_ceil(60))),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("  ·  ");
        let header = column![
            text(&album.title).font(BOLD).size(42).color(Color::WHITE),
            text(&album.artist)
                .font(FONT)
                .size(22)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.78)),
            text(details)
                .font(FONT)
                .size(15)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.5)),
        ]
        .spacing(6);

        let rows = album
            .tracks
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(VISIBLE_ROWS)
            .map(|(i, track)| {
                let glow = self.glow[i];
                let is_playing = self.playing == Some(i);
                let marker = match (is_playing, self.paused) {
                    (true, false) => "▶".to_string(),
                    (true, true) => "❚❚".to_string(),
                    _ => format!("{}", i + 1),
                };
                let strong =
                    Color::from_rgba(1.0, 1.0, 1.0, 0.72 + 0.28 * glow.max(f32::from(is_playing)));
                let row = row![
                    text(marker)
                        .font(FONT)
                        .size(15)
                        .width(34)
                        .color(if is_playing { accent } else { strong }),
                    text(&track.title)
                        .font(if is_playing { BOLD } else { FONT })
                        .size(18)
                        .color(strong)
                        .width(Fill),
                    text(format!("{}:{:02}", track.seconds / 60, track.seconds % 60))
                        .font(FONT)
                        .size(15)
                        .color(Color::from_rgba(1.0, 1.0, 1.0, 0.5)),
                ]
                .align_y(Vertical::Center);
                container(row)
                    .padding([11, 18])
                    .width(Fill)
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
                            radius: 12.0.into(),
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
                    })
                    .into()
            });
        let list = column(rows).spacing(4);

        let hints = text("↑↓ Choose    ✕ Play    Start Pause    ○ Stop")
            .font(FONT)
            .size(13)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.4));

        let panel = container(column![header, list, hints].spacing(28).max_width(560))
            .width(FillPortion(9))
            .height(Fill)
            .padding([48, 56])
            .align_y(Vertical::Center);

        stack![backdrop, shade, row![disc, panel]].into()
    }
}

fn key(event: keyboard::Event) -> Option<Remote> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };
    match key.as_ref() {
        Key::Named(Named::ArrowUp) => Some(Remote::Up),
        Key::Named(Named::ArrowDown) => Some(Remote::Down),
        Key::Named(Named::Enter) => Some(Remote::Select),
        Key::Named(Named::Space) => Some(Remote::PlayPause),
        Key::Named(Named::Backspace) => Some(Remote::Back),
        Key::Named(Named::Escape) => Some(Remote::Quit),
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
        .window_size((1280.0, 760.0))
        .antialiasing(true)
        .run()
}
