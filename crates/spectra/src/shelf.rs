//! The desktop, and the folders on it.
//!
//! The desktop holds the drive, three folders - Sounds, Videos and Games -
//! and whatever copies have been put there as favourites. A folder opens as
//! a window of every copy of its kind; a favourite is still in its folder
//! too. Icons go down the left of the desktop and then across, as desktop
//! icons do, and are chosen as they were: a blue cast over the icon and its
//! name picked out in blue.
//!
//! A copy's icon is its disc, flat, drawn once and kept (see `artwork.rs`);
//! the spinning disc is kept for the stage. Nothing here moves, so a desktop
//! left alone draws nothing.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iced::alignment::{Horizontal, Vertical};
use iced::widget::image::FilterMethod;
use iced::widget::scrollable::{AbsoluteOffset, Direction, Scrollbar, Viewport};
use iced::widget::{
    Id, button, column, container, image, mouse_area, operation, pin, row, scrollable, space,
    stack, text,
};
use iced::{Background, Border, Color, Element, Fill, Point, Size, Task};
use spectra_core::GameSystem;
use spectra_core::library::{self, Entry};

use crate::art::{Face, Icon};
use crate::theme::{self, BLUE, INK};
use crate::ui::{self, Style};
use crate::{BOLD, FONT, Message};

/// A desktop icon's cell: the icon, and two lines of its name under it.
const DESK_CELL: Size = Size::new(164.0, 168.0);
const DESK_ICON: f32 = 112.0;
const DESK_PAD: f32 = 16.0;
/// A copy's cell in a folder.
const FOLDER_CELL: Size = Size::new(176.0, 196.0);
const FOLDER_ICON: f32 = 132.0;
const PAD: f32 = 16.0;
/// The name under an icon: two lines of the pixel face.
const NAME: f32 = 40.0;
const GRID: &str = "folder";

/// The folders, in the order they stand on the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Sounds,
    Videos,
    Games,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Sounds, Section::Videos, Section::Games];

    fn of(entry: &Entry) -> Self {
        match kind(entry) {
            ui::Kind::Film => Self::Videos,
            ui::Kind::Game => Self::Games,
            ui::Kind::Music => Self::Sounds,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Sounds => "Sounds",
            Self::Videos => "Videos",
            Self::Games => "Games",
        }
    }

    /// The key that opens it, shown on its badge.
    fn key(self) -> &'static str {
        match self {
            Self::Sounds => "S",
            Self::Videos => "V",
            Self::Games => "G",
        }
    }

    /// Its badge's colour, and the colour of the letter on it.
    fn badge(self) -> (Color, Color) {
        match self {
            Self::Sounds => (Color::from_rgb8(0xf5, 0xd0, 0x1e), INK),
            Self::Videos => (Color::from_rgb8(0xe8, 0x2c, 0x2c), Color::WHITE),
            Self::Games => (Color::from_rgb8(0x24, 0x4c, 0xe0), Color::WHITE),
        }
    }
}

/// A place on the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spot {
    Drive,
    Folder(Section),
    /// A favourite: a copy, by its place in the library.
    Copy(usize),
}

/// A right-click menu on one copy. `at` is where it was asked for, or
/// None for the keys, which have it in the middle.
#[derive(Debug, Clone, Copy)]
pub struct Menu {
    pub index: usize,
    pub at: Option<Point>,
    /// The item the keys are on.
    pub item: usize,
}

/// What a menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Play,
    Pin,
    Unpin,
    Delete,
}

pub struct Shelf {
    pub entries: Vec<Entry>,
    faces: HashMap<String, Arc<Face>>,
    /// The copies on the desktop, by ID, in the order they were put there.
    favourites: Vec<String>,
    /// The folder open in its window, if one is.
    pub open: Option<Section>,
    /// The spot chosen on the desktop.
    desk: usize,
    /// The copy chosen in the open folder: its place in the folder.
    inside: usize,
    /// A copy whose delete is waiting to be confirmed.
    pub armed: Option<String>,
    pub menu: Option<Menu>,
    /// Something that went wrong, said where it was asked for.
    pub notice: Option<String>,
    viewport: Option<Viewport>,
    /// As last laid out: the desktop's icons to a column, and the folder's
    /// to a row - what the arrows move by.
    desk_rows: Cell<usize>,
    folder_columns: Cell<usize>,
}

impl Shelf {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            faces: HashMap::new(),
            favourites: load_favourites(),
            open: None,
            desk: 0,
            inside: 0,
            armed: None,
            menu: None,
            notice: None,
            viewport: None,
            desk_rows: Cell::new(1),
            folder_columns: Cell::new(1),
        }
    }

    /// Bring the desktop and folders into line with the library on disk.
    /// Returns the copies whose faces still have to be made.
    pub fn load(&mut self, entries: Vec<Entry>) -> Vec<Entry> {
        self.entries = entries;
        let ids: Vec<_> = self.entries.iter().map(|e| e.meta.id.clone()).collect();
        self.faces.retain(|id, _| ids.contains(id));
        if self.armed.as_ref().is_some_and(|id| !ids.contains(id)) {
            self.armed = None;
        }
        self.menu = None;
        self.desk = self.desk.min(self.spots().len() - 1);
        if let Some(open) = self.open {
            self.inside = self.inside.min(self.folder(open).len().saturating_sub(1));
        }
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

    /// Every spot on the desktop, in order: the drive, the folders, then
    /// the favourites that are still in the library.
    pub fn spots(&self) -> Vec<Spot> {
        let mut spots = vec![Spot::Drive];
        spots.extend(Section::ALL.map(Spot::Folder));
        spots.extend(
            self.favourites
                .iter()
                .filter_map(|id| self.entries.iter().position(|e| &e.meta.id == id))
                .map(Spot::Copy),
        );
        spots
    }

    /// The copies in a folder, by their places in the library: newest
    /// first, as the library lists them.
    fn folder(&self, section: Section) -> Vec<usize> {
        (0..self.entries.len())
            .filter(|&i| Section::of(&self.entries[i]) == section)
            .collect()
    }

    /// What has the keys: a spot on the desktop, or a copy in the folder.
    pub fn chosen(&self) -> Option<Spot> {
        match self.open {
            Some(open) => self.folder(open).get(self.inside).map(|&i| Spot::Copy(i)),
            None => self.spots().get(self.desk).copied(),
        }
    }

    /// The copy chosen, if what is chosen is a copy.
    pub fn focused_index(&self) -> Option<usize> {
        match self.chosen()? {
            Spot::Copy(index) => Some(index),
            _ => None,
        }
    }

    pub fn focused(&self) -> Option<&Entry> {
        self.focused_index().map(|i| &self.entries[i])
    }

    /// The drive, chosen on the desktop: a disc has gone in.
    pub fn focus_drive(&mut self) {
        self.desk = 0;
    }

    pub fn open_folder(&mut self, section: Section) -> Task<Message> {
        if self.open != Some(section) {
            self.inside = 0;
        }
        self.open = Some(section);
        self.menu = None;
        self.armed = None;
        self.notice = None;
        operation::scroll_to(
            Id::from(GRID),
            AbsoluteOffset {
                x: None,
                y: Some(0.0),
            },
        )
    }

    /// The folder's window closed: the folder is chosen on the desktop.
    pub fn close_folder(&mut self) -> bool {
        let Some(open) = self.open.take() else {
            return false;
        };
        if let Some(at) = self.spots().iter().position(|s| *s == Spot::Folder(open)) {
            self.desk = at;
        }
        self.menu = None;
        self.armed = None;
        true
    }

    /// A copy chosen from outside, as the command line does: chosen in the
    /// folder, if it is in the one open.
    pub fn focus_entry(&mut self, index: usize) {
        if let Some(open) = self.open
            && let Some(at) = self.folder(open).iter().position(|&i| i == index)
        {
            self.inside = at;
        }
    }

    /// A spot on the desktop under the pointer.
    pub fn point_desk(&mut self, at: usize) {
        if at < self.spots().len() && self.desk != at {
            self.desk = at;
            self.armed = None;
        }
    }

    /// A copy in the folder under the pointer. Never scrolls: what the
    /// pointer is on is in view already.
    pub fn point_folder(&mut self, index: usize) {
        self.focus_entry(index);
    }

    /// An arrow pressed: along the desktop's columns, or the folder's rows.
    pub fn step(&mut self, dx: isize, dy: isize) -> Task<Message> {
        self.armed = None;
        match self.open {
            Some(open) => {
                let count = self.folder(open).len();
                if count == 0 {
                    return Task::none();
                }
                let columns = self.folder_columns.get().max(1) as isize;
                let at = self.inside as isize;
                let to = if dy != 0 {
                    at + dy * columns
                } else if (at % columns + dx).clamp(0, columns - 1) == at % columns + dx {
                    at + dx
                } else {
                    at
                };
                // Down from a row with nothing straight below goes to the
                // last copy, rather than nowhere.
                let to = if dy > 0
                    && to >= count as isize
                    && at / columns < (count as isize - 1) / columns
                {
                    count as isize - 1
                } else {
                    to
                };
                if (0..count as isize).contains(&to) {
                    self.inside = to as usize;
                }
                self.reveal_focus()
            }
            None => {
                let count = self.spots().len() as isize;
                let rows = self.desk_rows.get().max(1) as isize;
                let at = self.desk as isize;
                let to = if dx != 0 {
                    at + dx * rows
                } else if (at % rows + dy).clamp(0, rows - 1) == at % rows + dy {
                    at + dy
                } else {
                    at
                };
                if (0..count).contains(&to) {
                    self.desk = to as usize;
                }
                Task::none()
            }
        }
    }

    pub fn scrolled(&mut self, viewport: Viewport) {
        self.viewport = Some(viewport);
    }

    /// Scroll the folder just enough to show the copy chosen, and not at
    /// all if it is in view.
    fn reveal_focus(&self) -> Task<Message> {
        let Some(viewport) = self.viewport else {
            return Task::none();
        };
        let columns = self.folder_columns.get().max(1);
        let top = PAD + (self.inside / columns) as f32 * FOLDER_CELL.height;
        let bottom = top + FOLDER_CELL.height;
        let offset = viewport.absolute_offset().y;
        let height = viewport.bounds().height;
        let y = if top < offset + PAD {
            top - PAD
        } else if bottom > offset + height {
            bottom - height + PAD
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

    pub fn pinned(&self, id: &str) -> bool {
        self.favourites.iter().any(|f| f == id)
    }

    /// Onto the desktop, or off it. Off it, the copy stays in its folder:
    /// only Delete takes a copy away.
    pub fn toggle_pin(&mut self, index: usize) {
        let Some(id) = self.entries.get(index).map(|e| e.meta.id.clone()) else {
            return;
        };
        if self.pinned(&id) {
            self.favourites.retain(|f| *f != id);
        } else {
            self.favourites.push(id);
        }
        self.desk = self.desk.min(self.spots().len() - 1);
        self.menu = None;
        save_favourites(&self.favourites);
    }

    /// What a copy's menu offers.
    pub fn acts(&self, index: usize) -> [Act; 3] {
        let pinned = self
            .entries
            .get(index)
            .is_some_and(|e| self.pinned(&e.meta.id));
        [
            Act::Play,
            if pinned { Act::Unpin } else { Act::Pin },
            Act::Delete,
        ]
    }

    /// Up or down the menu, with the keys.
    pub fn menu_step(&mut self, by: isize) {
        if let Some(menu) = &mut self.menu {
            menu.item = (menu.item as isize + by).clamp(0, 2) as usize;
        }
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

/// Where the desktop's favourites are kept: beside the library.
fn favourites_file() -> Option<PathBuf> {
    Some(library::root()?.parent()?.join("desktop.json"))
}

fn load_favourites() -> Vec<String> {
    favourites_file()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_favourites(favourites: &[String]) {
    let Some(path) = favourites_file() else {
        return;
    };
    if let Ok(json) = serde_json::to_vec_pretty(favourites) {
        // Whole or not at all: written aside, then put in place.
        let part = path.with_extension("part");
        if std::fs::write(&part, json).is_ok() {
            let _ = std::fs::rename(&part, path);
        }
    }
}

/// What the desktop and the folders need from the rest of the app.
pub struct Context<'a> {
    pub style: Style,
    /// The copy on the stage, if one is.
    pub picked: Option<&'a str>,
    /// The mouse is in use, not keys or a pad.
    pub pointing: bool,
    /// Whether the desktop has the keys: no window is in front of it.
    pub desk_active: bool,
    pub drive: Drive,
}

/// The drive, as its icon shows it.
pub struct Drive {
    /// The disc in it, if one is.
    pub icon: Option<Icon>,
    pub title: String,
}

/// What a copy holds, for its folder. A soundtrack is music, though its
/// copy is a game's.
fn kind(entry: &Entry) -> ui::Kind {
    if entry.is_film() {
        ui::Kind::Film
    } else if entry.is_album() || crate::soundtrack::is_entry(entry) {
        ui::Kind::Music
    } else {
        ui::Kind::Game
    }
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

/// An icon, plain or chosen, `side` square.
fn icon_image<'a>(icon: &Icon, chosen: bool, side: f32, pixels: bool) -> Element<'a, Message> {
    icon_faded(icon, chosen, side, pixels, 1.0)
}

fn icon_faded<'a>(
    icon: &Icon,
    chosen: bool,
    side: f32,
    pixels: bool,
    opacity: f32,
) -> Element<'a, Message> {
    let handle = if chosen { &icon.chosen } else { &icon.plain };
    let picture = image(handle.clone())
        .width(side)
        .height(side)
        .opacity(opacity);
    if pixels {
        picture.filter_method(FilterMethod::Nearest).into()
    } else {
        picture.into()
    }
}

/// A name on the desktop: white, standing off the sky by a dark shadow;
/// picked out in blue when chosen.
fn desk_name<'a>(words: String, chosen: bool) -> Element<'a, Message> {
    let line = |color: Color| {
        text(words.clone())
            .font(FONT)
            .size(14)
            .color(color)
            .align_x(Horizontal::Center)
    };
    let words: Element<'_, Message> = if chosen {
        container(line(Color::WHITE))
            .padding([1, 4])
            .style(|_| container::Style {
                background: Some(Background::Color(BLUE)),
                ..Default::default()
            })
            .into()
    } else {
        stack![
            container(line(Color::from_rgba(0.0, 0.0, 0.15, 0.85))).padding(iced::Padding {
                top: 2.0,
                left: 6.0,
                right: 2.0,
                bottom: 0.0,
            }),
            container(line(Color::WHITE)).padding([1, 4]),
        ]
        .into()
    };
    container(words)
        .width(DESK_CELL.width)
        .height(NAME)
        .align_x(Horizontal::Center)
        .clip(true)
        .into()
}

/// A name in a folder: ink on the window's grey; picked out in blue when
/// chosen.
fn name<'a>(words: String, chosen: bool, width: f32) -> Element<'a, Message> {
    let words = text(words)
        .font(FONT)
        .size(12)
        .color(if chosen { Color::WHITE } else { INK })
        .align_x(Horizontal::Center);
    container(
        container(words)
            .padding([1, 4])
            .max_width(width)
            .style(move |_| container::Style {
                background: chosen.then_some(Background::Color(BLUE)),
                ..Default::default()
            }),
    )
    .width(width)
    .height(NAME)
    .align_x(Horizontal::Center)
    .clip(true)
    .into()
}

/// A key, on a round badge at an icon's corner: what opens it.
fn key_badge<'a>(key: &'static str, (back, ink): (Color, Color)) -> Element<'a, Message> {
    container(text(key).font(BOLD).size(16).color(ink).center())
        .width(BADGE)
        .height(BADGE)
        .style(move |_| container::Style {
            background: Some(Background::Color(back)),
            border: Border {
                color: INK,
                width: 2.0,
                radius: (BADGE / 2.0).into(),
            },
            ..Default::default()
        })
        .into()
}

const BADGE: f32 = 32.0;
/// Where a badge sits on its icon: low on the right.
const BADGE_AT: f32 = DESK_ICON - BADGE - 2.0;
/// The drive's badge: green, as its light is.
const DRIVE_BADGE: (Color, Color) = (Color::from_rgb(0.18, 0.62, 0.27), Color::WHITE);

impl Shelf {
    /// The desktop's icons, down the left and then across `size`.
    pub fn desktop<'a>(&'a self, size: Size, cx: &Context<'a>) -> Element<'a, Message> {
        let rows = (((size.height - 2.0 * DESK_PAD) / DESK_CELL.height).floor() as usize).max(1);
        self.desk_rows.set(rows);
        let spots = self.spots();
        let columns = spots.chunks(rows).enumerate().map(|(c, chunk)| {
            column(
                chunk
                    .iter()
                    .enumerate()
                    .map(|(r, spot)| self.desk_icon(c * rows + r, *spot, cx)),
            )
            .into()
        });
        container(row(columns))
            .padding(DESK_PAD)
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn desk_icon<'a>(&'a self, at: usize, spot: Spot, cx: &Context<'a>) -> Element<'a, Message> {
        let chosen = cx.desk_active && self.desk == at;
        let (picture, words): (Element<'_, Message>, String) = match spot {
            Spot::Drive => {
                let picture = match &cx.drive.icon {
                    Some(disc) => icon_image(disc, chosen, DESK_ICON, false),
                    // No disc: the drive itself, faded as a disabled icon
                    // was, its name saying why. It can still be chosen, to
                    // say so.
                    None => icon_faded(&theme::drive_icon(), chosen, DESK_ICON, true, 0.45),
                };
                (
                    stack![
                        picture,
                        pin(key_badge("D", DRIVE_BADGE)).x(BADGE_AT).y(BADGE_AT)
                    ]
                    .into(),
                    cx.drive.title.clone(),
                )
            }
            Spot::Folder(section) => (
                stack![
                    icon_image(&theme::folder_icon(), chosen, DESK_ICON, true),
                    pin(key_badge(section.key(), section.badge()))
                        .x(BADGE_AT)
                        .y(BADGE_AT),
                ]
                .into(),
                section.name().to_string(),
            ),
            Spot::Copy(index) => {
                let entry = &self.entries[index];
                (
                    icon_image(&self.face(entry).icon, chosen, DESK_ICON, false),
                    entry.meta.title.clone(),
                )
            }
        };
        let picture = container(picture).width(DESK_ICON).height(DESK_ICON);
        let cell = column![
            container(picture)
                .width(DESK_CELL.width)
                .align_x(Horizontal::Center)
                .padding(iced::Padding {
                    top: 6.0,
                    ..iced::Padding::ZERO
                }),
            desk_name(words, chosen),
        ]
        .width(DESK_CELL.width)
        .height(DESK_CELL.height)
        .spacing(4);
        let mut area = mouse_area(cell)
            .on_move(move |_| Message::DeskFocus(at))
            .on_press(Message::DeskOpen(at));
        if let Spot::Copy(index) = spot {
            area = area.on_right_press(Message::ShelfMenu(index));
        }
        area.into()
    }

    fn face(&self, entry: &Entry) -> Arc<Face> {
        self.faces
            .get(&entry.meta.id)
            .cloned()
            .unwrap_or_else(|| Arc::new(Face::blank()))
    }

    /// The open folder's window: its title, and what is in it, `size`
    /// being the window's.
    pub fn folder_window<'a>(
        &'a self,
        size: Size,
        cx: &Context<'a>,
    ) -> Option<(String, Element<'a, Message>)> {
        let open = self.open?;
        let copies = self.folder(open);
        let title = format!("{} ({})", open.name(), copies.len());
        let columns =
            (((size.width - 2.0 * PAD - 16.0) / FOLDER_CELL.width).floor() as usize).max(1);
        let notice = self.notice.as_ref().map(|notice| {
            container(
                text(notice.clone())
                    .font(FONT)
                    .size(13)
                    .color(Color::from_rgb8(0xc0, 0x10, 0x10)),
            )
            .padding([8.0, PAD])
        });
        self.folder_columns.set(columns);
        let grid: Element<'_, Message> = if copies.is_empty() {
            container(self.empty(open, cx))
                .padding(PAD)
                .width(Fill)
                .into()
        } else {
            let rows = copies.chunks(columns).enumerate().map(|(r, chunk)| {
                row(chunk
                    .iter()
                    .enumerate()
                    .map(|(c, &index)| self.folder_icon(r * columns + c, index, cx)))
                .into()
            });
            scrollable(column(rows).padding(PAD))
                .id(GRID)
                .on_scroll(Message::ShelfScrolled)
                .direction(Direction::Vertical(
                    Scrollbar::new().width(10).scroller_width(10).margin(2),
                ))
                .style(|look, status| {
                    let mut style = scrollable::default(look, status);
                    style.vertical_rail.background =
                        Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.06)));
                    style.vertical_rail.scroller.background = Background::Color(theme::SILVER);
                    style.vertical_rail.scroller.border = Border {
                        color: INK,
                        width: 2.0,
                        radius: 0.0.into(),
                    };
                    style
                })
                .width(Fill)
                .height(Fill)
                .into()
        };
        let body = column![]
            .push(notice)
            .push(container(grid).height(Fill))
            .push(self.status(cx));
        Some((title, body.into()))
    }

    fn folder_icon<'a>(
        &'a self,
        at: usize,
        index: usize,
        cx: &Context<'a>,
    ) -> Element<'a, Message> {
        let entry = &self.entries[index];
        let chosen = self.inside == at;
        let mut words = entry.meta.title.clone();
        if cx.picked == Some(entry.meta.id.as_str()) {
            words = format!("▶ {words}");
        }
        let cell = column![
            container(icon_image(
                &self.face(entry).icon,
                chosen,
                FOLDER_ICON,
                false
            ))
            .width(FOLDER_CELL.width)
            .align_x(Horizontal::Center)
            .padding(iced::Padding {
                top: 8.0,
                ..iced::Padding::ZERO
            }),
            name(words, chosen, FOLDER_CELL.width - 8.0),
        ]
        .width(FOLDER_CELL.width)
        .height(FOLDER_CELL.height)
        .align_x(Horizontal::Center)
        .spacing(4);
        mouse_area(cell)
            .on_move(move |_| Message::ShelfFocus(index))
            .on_press(Message::ShelfPick(index))
            .on_right_press(Message::ShelfMenu(index))
            .into()
    }

    /// The status bar along the bottom of a folder: what is chosen, and
    /// what the keys do to it.
    fn status<'a>(&'a self, cx: &Context<'a>) -> Element<'a, Message> {
        let (what, about) = match self.focused() {
            Some(entry) => (entry.meta.title.clone(), byline(entry)),
            None => (String::new(), String::new()),
        };
        let red = Color::from_rgb8(0xc0, 0x10, 0x10);
        let armed = self
            .focused()
            .is_some_and(|e| self.armed.as_deref() == Some(e.meta.id.as_str()));
        let keys: Option<Element<'_, Message>> = if armed {
            Some(if cx.pointing {
                text("Delete? Once more to confirm")
                    .font(FONT)
                    .size(12)
                    .color(red)
                    .into()
            } else {
                ui::prompt("{select} again to delete", cx.style, 12.0, red)
            })
        } else if cx.pointing || self.focused().is_none() {
            None
        } else {
            Some(ui::prompt(
                "{accept} Play  {select} Menu  {back} Close",
                cx.style,
                12.0,
                theme::faint(0.7),
            ))
        };
        let line = row![
            theme::sunken(
                row![
                    text(what)
                        .font(BOLD)
                        .size(12)
                        .wrapping(text::Wrapping::None),
                    text(about)
                        .font(FONT)
                        .size(12)
                        .color(theme::faint(0.6))
                        .wrapping(text::Wrapping::None),
                ]
                .spacing(12)
            )
            .width(Fill)
            .clip(true),
        ]
        .push(keys.map(theme::sunken))
        .spacing(4)
        .align_y(Vertical::Center);
        container(line)
            .padding(4)
            .width(Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(theme::SILVER)),
                ..Default::default()
            })
            .into()
    }

    fn empty<'a>(&'a self, open: Section, cx: &Context<'a>) -> Element<'a, Message> {
        let words = format!(
            "No {} kept yet. With a disc in the drive, {}.",
            open.name().to_lowercase(),
            if cx.pointing {
                "choose Keep a copy"
            } else {
                "press {alt} to keep a copy"
            }
        );
        let card = stack![
            ui::Dashed {
                color: theme::faint(0.3),
                radius: 4.0,
            },
            container(
                container(ui::prompt(&words, cx.style, 13.0, theme::faint(0.6))).max_width(448)
            )
            .padding([40, 32])
            .center_x(Fill),
        ];
        container(card).width(Fill).height(140).into()
    }

    /// A copy's menu, over everything: at the pointer, or in the middle of
    /// `size` for the keys.
    pub fn menu_view<'a>(&'a self, menu: Menu, size: Size) -> Element<'a, Message> {
        let Some(entry) = self.entries.get(menu.index) else {
            return space().into();
        };
        const WIDTH: f32 = 260.0;
        const HEIGHT: f32 = 150.0;
        let armed = self.armed.as_deref() == Some(entry.meta.id.as_str());
        let items = self
            .acts(menu.index)
            .into_iter()
            .enumerate()
            .map(|(i, act)| {
                let (words, color, message) = match act {
                    Act::Play => ("▶ Play".to_string(), INK, Message::ShelfPick(menu.index)),
                    Act::Pin => ("Add to desktop".into(), INK, Message::ShelfPin(menu.index)),
                    Act::Unpin => (
                        "Remove from desktop".into(),
                        INK,
                        Message::ShelfPin(menu.index),
                    ),
                    Act::Delete => (
                        if armed {
                            "Delete? Again to confirm".into()
                        } else {
                            "Delete".into()
                        },
                        Color::from_rgb8(0xc0, 0x10, 0x10),
                        Message::ShelfDelete(menu.index),
                    ),
                };
                let keyed = menu.at.is_none() && menu.item == i;
                button(text(words.to_uppercase()).font(FONT).size(13))
                    .width(Fill)
                    .padding([6, 10])
                    .on_press(message)
                    .style(move |_, status| {
                        let hot = keyed
                            || matches!(status, button::Status::Hovered | button::Status::Pressed);
                        button::Style {
                            background: hot.then_some(Background::Color(BLUE)),
                            text_color: if hot { Color::WHITE } else { color },
                            ..button::Style::default()
                        }
                    })
                    .into()
            });
        let panel = container(
            column![
                container(
                    column![
                        text(entry.meta.title.clone())
                            .font(BOLD)
                            .size(13)
                            .color(INK)
                            .wrapping(text::Wrapping::None),
                        text(byline(entry))
                            .font(FONT)
                            .size(11)
                            .color(theme::faint(0.6))
                            .wrapping(text::Wrapping::None),
                    ]
                    .spacing(2)
                )
                .padding([6, 10])
                .clip(true),
                container(space())
                    .height(2)
                    .width(Fill)
                    .style(|_| container::Style {
                        background: Some(Background::Color(INK)),
                        ..Default::default()
                    }),
            ]
            .extend(items)
            .spacing(2),
        )
        .width(WIDTH)
        .padding(2)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::PAPER)),
            border: Border {
                color: INK,
                width: 2.0,
                radius: 0.0.into(),
            },
            shadow: theme::hard_shadow(4.0),
            ..Default::default()
        });
        // Kept on screen, whichever corner it was asked for in.
        let at = menu.at.unwrap_or(Point::new(
            (size.width - WIDTH) / 2.0,
            (size.height - HEIGHT) / 2.0,
        ));
        let x = at.x.min(size.width - WIDTH - 8.0).max(8.0);
        let y = at.y.min(size.height - HEIGHT - 8.0).max(8.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_starts_with_the_drive_and_three_folders() {
        let mut shelf = Shelf::new();
        shelf.favourites.clear();
        assert_eq!(
            shelf.spots(),
            vec![
                Spot::Drive,
                Spot::Folder(Section::Sounds),
                Spot::Folder(Section::Videos),
                Spot::Folder(Section::Games),
            ]
        );
    }

    #[test]
    fn the_arrows_go_down_the_desktop_then_across() {
        let mut shelf = Shelf::new();
        shelf.favourites.clear();
        shelf.desk_rows.set(2);
        let _ = shelf.step(0, 1);
        assert_eq!(shelf.chosen(), Some(Spot::Folder(Section::Sounds)));
        let _ = shelf.step(0, 1);
        assert_eq!(
            shelf.chosen(),
            Some(Spot::Folder(Section::Sounds)),
            "the column ends"
        );
        let _ = shelf.step(1, 0);
        assert_eq!(shelf.chosen(), Some(Spot::Folder(Section::Games)));
    }
}
