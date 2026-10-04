//! The library: every kept copy, as a shelf of discs below the stage.
//!
//! After Rainbow Player's `LibraryShelf.tsx` and `shelfScene.ts`. The discs
//! sit in a grid that wraps to the window, each a small 3D disc of its own
//! wearing its label. The one in focus turns end over end, as the disc on
//! the stage does when it arrives, and is drawn a little larger; the others
//! rest label side out. The art says which disc is which, so the line under
//! each says only what is happening to it - unless it has no art, when it
//! says its name.
//!
//! Each disc is its own shader widget, all drawn by the one pipeline; see
//! `disc.rs`. Nothing moves once the focused disc has shown both faces, so
//! an open library that is left alone draws nothing.

use std::cell::Cell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use iced::alignment::{Horizontal, Vertical};
use iced::widget::scrollable::{AbsoluteOffset, Direction, Scrollbar, Viewport};
use iced::widget::{
    Id, button, column, container, image, mouse_area, operation, pin, row, scrollable, shader,
    space, stack, text,
};
use iced::{
    Background, Border, Color, ContentFit, Element, Fill, Point, Shadow, Size, Task, Vector,
};
use spectra_core::GameSystem;
use spectra_core::library::Entry;

use crate::art::Face;
use crate::motion::Motion;
use crate::ui::{self, Style};
use crate::{BOLD, CD_PITCH, FONT, Message};

/// Narrowest a cell gets before the grid drops a column, as Rainbow
/// Player's `minmax(280px, 1fr)`.
const CELL_MIN: f32 = 280.0;
const GAP_X: f32 = 32.0;
const GAP_Y: f32 = 24.0;
/// Room around each disc, inside its cell.
const CELL_PAD: f32 = 8.0;
/// The status line under each disc.
const STATUS: f32 = 20.0;
const HEADER_TOP: f32 = 40.0;
const HEADER_BOTTOM: f32 = 24.0;
const GRID_BOTTOM: f32 = 24.0;
const GRID: &str = "shelf";
/// Slots on the GPU for shelf discs start here; the stage's is 0.
const SLOT_BIT: u64 = 1 << 63;

/// A right-click menu on one disc.
#[derive(Debug, Clone, Copy)]
pub struct Menu {
    pub index: usize,
    pub at: Point,
}

pub struct Shelf {
    pub entries: Vec<Entry>,
    pub focus: usize,
    faces: HashMap<String, Arc<Face>>,
    motions: HashMap<String, Motion>,
    /// A copy whose delete is waiting to be confirmed.
    pub armed: Option<String>,
    pub menu: Option<Menu>,
    /// Something that went wrong, said where it was asked for.
    pub notice: Option<String>,
    viewport: Option<Viewport>,
    /// Columns and row pitch as last laid out: what up and down move by,
    /// and how far the grid scrolls to show the focus.
    columns: Cell<usize>,
    pitch: Cell<f32>,
    reduced: bool,
}

/// Where a move on the shelf went.
pub enum Moved {
    To,
    /// Past the top row: back up to the stage.
    Out,
    Nowhere,
}

impl Shelf {
    pub fn new(reduced: bool) -> Self {
        Self {
            entries: Vec::new(),
            focus: 0,
            faces: HashMap::new(),
            motions: HashMap::new(),
            armed: None,
            menu: None,
            notice: None,
            viewport: None,
            columns: Cell::new(1),
            pitch: Cell::new(CELL_MIN + GAP_Y),
            reduced,
        }
    }

    /// Bring the shelf into line with the library on disk. Returns the
    /// copies whose faces still have to be made.
    pub fn load(&mut self, entries: Vec<Entry>) -> Vec<Entry> {
        let focused = self.entries.get(self.focus).map(|e| e.meta.id.clone());
        self.entries = entries;
        self.focus = focused
            .and_then(|id| self.entries.iter().position(|e| e.meta.id == id))
            .unwrap_or(0);
        let ids: Vec<_> = self.entries.iter().map(|e| e.meta.id.clone()).collect();
        self.faces.retain(|id, _| ids.contains(id));
        self.motions.retain(|id, _| ids.contains(id));
        for id in &ids {
            self.motions
                .entry(id.clone())
                .or_insert_with(|| Motion::shelved(self.reduced));
        }
        if self.armed.as_ref().is_some_and(|id| !ids.contains(id)) {
            self.armed = None;
        }
        self.menu = None;
        self.entries
            .iter()
            .filter(|e| !self.faces.contains_key(&e.meta.id))
            .cloned()
            .collect()
    }

    pub fn set_face(&mut self, id: String, face: Arc<Face>) {
        if self.entries.iter().any(|e| e.meta.id == id) {
            self.faces.insert(id, face);
        }
    }

    pub fn focused(&self) -> Option<&Entry> {
        self.entries.get(self.focus)
    }

    /// The shelf came on screen, or went: the focused disc shows itself
    /// off, or settles.
    pub fn shown(&mut self, shown: bool) {
        if let Some(id) = self.focused().map(|e| e.meta.id.clone())
            && let Some(motion) = self.motions.get_mut(&id)
        {
            motion.set_focused(shown);
        }
        if !shown {
            self.menu = None;
            self.armed = None;
        }
    }

    pub fn focus_on(&mut self, index: usize) -> Task<Message> {
        if index >= self.entries.len() || index == self.focus {
            return Task::none();
        }
        for (i, focused) in [(self.focus, false), (index, true)] {
            if let Some(motion) = self
                .entries
                .get(i)
                .and_then(|e| self.motions.get_mut(&e.meta.id))
            {
                motion.set_focused(focused);
            }
        }
        self.focus = index;
        self.armed = None;
        self.reveal_focus()
    }

    pub fn left(&mut self) -> (Moved, Task<Message>) {
        if self.focus == 0 {
            return (Moved::Nowhere, Task::none());
        }
        (Moved::To, self.focus_on(self.focus - 1))
    }

    pub fn right(&mut self) -> (Moved, Task<Message>) {
        if self.focus + 1 >= self.entries.len() {
            return (Moved::Nowhere, Task::none());
        }
        (Moved::To, self.focus_on(self.focus + 1))
    }

    pub fn up(&mut self) -> (Moved, Task<Message>) {
        let cols = self.columns.get().max(1);
        if self.focus < cols {
            return (Moved::Out, Task::none());
        }
        (Moved::To, self.focus_on(self.focus - cols))
    }

    pub fn down(&mut self) -> (Moved, Task<Message>) {
        let cols = self.columns.get().max(1);
        let last_row = self.entries.len().saturating_sub(1) / cols;
        if self.focus / cols >= last_row {
            return (Moved::Nowhere, Task::none());
        }
        let to = (self.focus + cols).min(self.entries.len() - 1);
        (Moved::To, self.focus_on(to))
    }

    pub fn scrolled(&mut self, viewport: Viewport) {
        self.viewport = Some(viewport);
    }

    /// Whether the grid is scrolled to its top, where a wheel upwards goes
    /// back to the stage.
    pub fn at_top(&self) -> bool {
        self.viewport.is_none_or(|v| v.absolute_offset().y <= 0.5)
    }

    /// Scroll just enough to show the focused disc.
    fn reveal_focus(&self) -> Task<Message> {
        let Some(viewport) = self.viewport else {
            return Task::none();
        };
        let cols = self.columns.get().max(1);
        let pitch = self.pitch.get();
        let top = (self.focus / cols) as f32 * pitch;
        let bottom = top + pitch - GAP_Y;
        let offset = viewport.absolute_offset().y;
        let height = viewport.bounds().height;
        let y = if top < offset {
            top
        } else if bottom > offset + height {
            bottom - height + GRID_BOTTOM.min(height / 4.0)
        } else {
            return Task::none();
        };
        operation::scroll_to(
            Id::from(GRID),
            AbsoluteOffset {
                x: None,
                y: Some(y.max(0.0)),
            },
        )
    }

    pub fn step(&mut self, dt: f32) {
        for motion in self.motions.values_mut() {
            if motion.moving() {
                motion.step(dt);
            }
        }
    }

    pub fn moving(&self) -> bool {
        self.motions.values().any(Motion::moving)
    }

    /// Delete pressed once arms it; again, on the same copy, confirms.
    /// Returns the copy to delete when confirmed.
    pub fn delete(&mut self, index: usize) -> Option<Entry> {
        let entry = self.entries.get(index)?.clone();
        if self.armed.as_deref() == Some(entry.meta.id.as_str()) {
            self.armed = None;
            self.menu = None;
            Some(entry)
        } else {
            self.armed = Some(entry.meta.id.clone());
            None
        }
    }
}

/// What the shelf needs from the rest of the app to draw itself.
pub struct Context<'a> {
    pub style: Style,
    /// The copy on the stage, if one is.
    pub picked: Option<&'a str>,
    /// Whether Back returns to a disc in the drive.
    pub disc_in_drive: bool,
    /// The mouse is in use, not keys or a pad.
    pub pointing: bool,
}

/// Which GPU slot a copy's disc draws into: stable across reloads, so its
/// label stays uploaded, and never the stage's.
fn slot(id: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish() | SLOT_BIT
}

/// "PlayStation · Eidos Interactive · 1996", or an album's artist.
pub fn byline(entry: &Entry) -> String {
    if entry.is_film() {
        return [Some("DVD-Video".into()), entry.meta.year.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("  ·  ");
    }
    if crate::soundtrack::is_entry(entry) {
        let system = entry.meta.system.map_or("Game", GameSystem::name);
        return format!("Soundtrack  ·  {system}");
    }
    if entry.is_album() {
        return [entry.meta.artist.clone(), Some("Music".into())]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("  ·  ");
    }
    [
        Some(
            entry
                .meta
                .system
                .map_or("Disc", GameSystem::name)
                .to_string(),
        ),
        entry.meta.publisher.clone(),
        entry.meta.year.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("  ·  ")
}

impl Shelf {
    pub fn view<'a>(&'a self, size: Size, cx: Context<'a>) -> Element<'a, Message> {
        let pad_x = (size.width * 0.04).max(32.0);
        let glow = image(ui::glow())
            .width(Fill)
            .height(Fill)
            .content_fit(ContentFit::Fill);

        let back = if cx.disc_in_drive || cx.picked.is_some() {
            "Back to the disc"
        } else {
            "Back"
        };
        let back: Element<'_, Message> = match cx.style {
            ui::Style::Pad(_) if !cx.pointing => ui::prompt(
                &format!("{{up}} {}", back.to_uppercase()),
                cx.style,
                13.0,
                Color::from_rgba(1.0, 1.0, 1.0, 0.45),
            ),
            _ => ui::quiet_text(&format!("↑ {back}")),
        };
        let notice: Element<'_, Message> = match &self.notice {
            Some(notice) => text(notice.clone())
                .font(FONT)
                .size(15)
                .color(Color::from_rgb8(0xff, 0x9a, 0x9a))
                .into(),
            None => text(match self.entries.len() {
                0 => "YOUR DISCS".to_string(),
                n => format!("YOUR DISCS  ·  {n}"),
            })
            .font(FONT)
            .size(13)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.35))
            .into(),
        };
        let header = row![
            container(notice).width(Fill),
            ui::quiet(back, Message::CloseLibrary)
        ]
        .align_y(Vertical::Center)
        .padding([0.0, pad_x]);

        let body: Element<'_, Message> = if self.entries.is_empty() {
            container(self.empty(cx.style, cx.pointing))
                .padding([0.0, pad_x])
                .width(Fill)
                .into()
        } else {
            self.grid(size.width - 2.0 * pad_x, pad_x, &cx)
        };

        // Under the grid rather than over it, so a short window never puts
        // the prompt across a disc.
        let footer: Element<'_, Message> = if !cx.pointing && !self.entries.is_empty() {
            container(ui::prompt(
                "{accept} PLAY  ·  {select} DELETE  ·  {back} BACK",
                cx.style,
                13.0,
                Color::from_rgba(1.0, 1.0, 1.0, 0.35),
            ))
            .center_x(Fill)
            .padding([12, 0])
            .into()
        } else {
            space().height(16).into()
        };

        let screen = column![
            space().height(HEADER_TOP),
            header,
            space().height(HEADER_BOTTOM),
            container(body).height(Fill),
            footer,
        ];
        let mut layers = stack![glow, screen];
        if let Some(menu) = self.menu {
            layers = layers.push(self.menu_view(menu, size));
        }
        mouse_area(layers)
            .on_scroll(|delta| Message::Wheel(crate::Screen::Library, delta))
            .on_move(Message::Pointer)
            .into()
    }

    fn grid<'a>(&'a self, width: f32, pad_x: f32, cx: &Context<'a>) -> Element<'a, Message> {
        let cols = (((width + GAP_X) / (CELL_MIN + GAP_X)).floor() as usize).max(1);
        let cell = ((width - GAP_X * (cols - 1) as f32) / cols as f32).max(80.0);
        let disc = cell - 2.0 * CELL_PAD;
        self.columns.set(cols);
        self.pitch.set(disc + STATUS + 4.0 + 2.0 * CELL_PAD + GAP_Y);

        let rows = self.entries.chunks(cols).enumerate().map(|(r, chunk)| {
            let mut line = row![].spacing(GAP_X);
            for (c, entry) in chunk.iter().enumerate() {
                line = line.push(self.cell(r * cols + c, entry, cell, disc, cx));
            }
            // Keep a short last row on the grid, not stretched across it.
            for _ in chunk.len()..cols {
                line = line.push(space().width(cell));
            }
            line.into()
        });
        scrollable(column(rows).spacing(GAP_Y).padding(iced::Padding {
            top: 0.0,
            right: pad_x,
            bottom: GRID_BOTTOM,
            left: pad_x,
        }))
        .id(GRID)
        .on_scroll(Message::ShelfScrolled)
        .direction(Direction::Vertical(Scrollbar::hidden()))
        .into()
    }

    fn cell<'a>(
        &'a self,
        index: usize,
        entry: &'a Entry,
        cell: f32,
        disc: f32,
        cx: &Context<'a>,
    ) -> Element<'a, Message> {
        let id = entry.meta.id.as_str();
        let face = self.faces.get(id);
        let blank;
        let face = match face {
            Some(face) => face.as_ref(),
            None => {
                blank = Face::blank();
                &blank
            }
        };
        let pose = self.motions.get(id).map(Motion::pose).unwrap_or_default();
        let accent = Color::from_rgb(face.accent[0], face.accent[1], face.accent[2]);
        let disc_widget = shader(crate::disc::Disc {
            slot: slot(id),
            pose,
            label: face.label.clone(),
            accent: face.accent,
            pitch: CD_PITCH,
        })
        .width(disc)
        .height(disc);

        let dim = Color::from_rgba(1.0, 1.0, 1.0, 0.35);
        let red = Color::from_rgb8(0xff, 0x8a, 0x8a);
        let status: Element<'_, Message> = if self.armed.as_deref() == Some(id) {
            if cx.pointing {
                text("Delete? Once more to confirm")
                    .font(FONT)
                    .size(13)
                    .color(red)
                    .into()
            } else {
                ui::prompt("{select} again to delete", cx.style, 13.0, red)
            }
        } else if cx.picked == Some(id) {
            text("On the stage")
                .font(FONT)
                .size(13)
                .color(accent)
                .into()
        } else if !face.printed {
            text(entry.meta.title.clone())
                .font(FONT)
                .size(13)
                .color(dim)
                .wrapping(text::Wrapping::None)
                .into()
        } else {
            space().into()
        };

        let focused = index == self.focus;
        // The focus is the disc turning; a ring as well only for keys and
        // pads, which have no pointer to show where they are.
        let ring = focused && !cx.pointing;
        let content = column![
            disc_widget,
            container(status)
                .height(STATUS)
                .width(Fill)
                .align_x(Horizontal::Center)
                .clip(true)
        ]
        .spacing(4)
        .align_x(Horizontal::Center);
        let content = container(content).padding(CELL_PAD).width(cell);
        // The ring is a layer of its own over the cell: drawn in the disc's
        // layer, a bordered quad blacks out the disc's surroundings.
        let boxed: Element<'_, Message> = if ring {
            stack![
                content,
                container(space())
                    .width(Fill)
                    .height(Fill)
                    .style(|_| container::Style {
                        border: Border {
                            color: Color::from_rgba(1.0, 1.0, 1.0, 0.5),
                            width: 2.0,
                            radius: 16.0.into(),
                        },
                        ..Default::default()
                    }),
            ]
            .into()
        } else {
            content.into()
        };
        mouse_area(boxed)
            .on_enter(Message::ShelfFocus(index))
            .on_press(Message::ShelfPick(index))
            .on_right_press(Message::ShelfMenu(index))
            .into()
    }

    fn empty<'a>(&'a self, style: Style, pointing: bool) -> Element<'a, Message> {
        let words: Element<'_, Message> = if pointing {
            text("Nothing kept yet. With a disc in the drive, choose Keep a copy in the bottom corner.")
                .font(FONT)
                .size(15)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.45))
                .align_x(Horizontal::Center)
                .into()
        } else {
            ui::prompt(
                "Nothing kept yet. With a disc in the drive, press {alt} to keep a copy.",
                style,
                15.0,
                Color::from_rgba(1.0, 1.0, 1.0, 0.45),
            )
        };
        let card = stack![
            ui::Dashed {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.15),
                radius: 12.0,
            },
            container(container(words).max_width(448))
                .padding([48, 32])
                .center_x(Fill),
        ];
        container(card).width(Fill).height(160).into()
    }

    fn menu_view<'a>(&'a self, menu: Menu, size: Size) -> Element<'a, Message> {
        let Some(entry) = self.entries.get(menu.index) else {
            return space().into();
        };
        const WIDTH: f32 = 256.0;
        const HEIGHT: f32 = 132.0;
        let armed = self.armed.as_deref() == Some(entry.meta.id.as_str());
        let item = |words: String, color: Color, hover: Color, message: Message| {
            button(text(words).font(FONT).size(14).color(color))
                .width(Fill)
                .padding([6, 10])
                .on_press(message)
                .style(move |_, status| button::Style {
                    background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                        .then_some(Background::Color(hover)),
                    border: Border {
                        radius: 6.0.into(),
                        ..Border::default()
                    },
                    ..button::Style::default()
                })
        };
        let panel = container(
            column![
                container(
                    column![
                        text(entry.meta.title.clone())
                            .font(BOLD)
                            .size(14)
                            .color(Color::WHITE)
                            .wrapping(text::Wrapping::None),
                        text(byline(entry))
                            .font(FONT)
                            .size(12)
                            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.5))
                            .wrapping(text::Wrapping::None),
                    ]
                    .spacing(2)
                )
                .padding([6, 10])
                .clip(true),
                container(space())
                    .height(1)
                    .width(Fill)
                    .style(|_| container::Style {
                        background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.1))),
                        ..Default::default()
                    }),
                item(
                    "▶  Play".into(),
                    Color::WHITE,
                    Color::from_rgba(1.0, 1.0, 1.0, 0.1),
                    Message::ShelfPick(menu.index),
                ),
                item(
                    if armed {
                        "Delete? Choose again to confirm".into()
                    } else {
                        "Delete".into()
                    },
                    Color::from_rgb8(0xff, 0x8a, 0x8a),
                    Color::from_rgba8(0xff, 0x6b, 0x6b, 0.15),
                    Message::ShelfDelete(menu.index),
                ),
            ]
            .spacing(4),
        )
        .width(WIDTH)
        .padding(4)
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgba8(0x0e, 0x0e, 0x16, 0.95))),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.1),
                width: 1.0,
                radius: 8.0.into(),
            },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
                offset: Vector::new(0.0, 8.0),
                blur_radius: 24.0,
            },
            ..Default::default()
        });
        // Kept on screen, whichever corner it was asked for in.
        let x = menu.at.x.min(size.width - WIDTH - 8.0).max(8.0);
        let y = menu.at.y.min(size.height - HEIGHT - 8.0).max(8.0);
        stack![
            // A click anywhere else puts the menu away.
            mouse_area(container(space()).width(Fill).height(Fill))
                .on_press(Message::MenuClose)
                .on_right_press(Message::MenuClose),
            pin(panel).x(x).y(y),
        ]
        .into()
    }
}
