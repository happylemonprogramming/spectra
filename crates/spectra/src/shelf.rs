//! My discs: one window of everything, on the desktop.
//!
//! The disc in the drive comes first, then every kept copy, newest first,
//! as icons in a window with a blue title bar. Along its top, Sounds,
//! Videos and Games narrow it to one kind; nothing is ever more than a
//! filter away. Along its bottom, the chosen disc's name and maker, and
//! what the keys do to it. Enter puts the chosen disc on the stage, in a
//! window of its own; the options for a copy - open it, delete it - come
//! up in the bar below, not in a menu of their own.
//!
//! A copy's icon is its disc, flat, drawn once and kept (see `artwork.rs`);
//! the spinning disc is kept for the stage. Nothing here moves, so a window
//! left alone draws nothing.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use iced::alignment::{Horizontal, Vertical};
use iced::widget::image::FilterMethod;
use iced::widget::scrollable::{AbsoluteOffset, Direction, Scrollbar, Viewport};
use iced::widget::{
    Id, column, container, image, mouse_area, operation, row, scrollable, space, stack, text,
    text_input,
};
use iced::{Background, Border, Color, Element, Fill, Size, Task};
use spectra_core::GameSystem;
use spectra_core::library::Entry;

use crate::art::{Face, Icon};
use crate::theme;
use crate::ui::{self, Style};
use crate::{BOLD, FONT, Message};

/// A disc's cell in the window.
const CELL: Size = Size::new(176.0, 214.0);
const ICON: f32 = 132.0;
const PAD: f32 = 16.0;
/// The name under an icon: three lines of the pixel face, which a game's
/// soundtrack needs.
const NAME: f32 = 58.0;
const GRID: &str = "discs";
const SEARCH: &str = "search";

/// A kind of disc, which the window can be narrowed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Sounds,
    Videos,
    Games,
    /// Not a kind: the copies starred, whatever they are.
    Starred,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Sounds, Section::Videos, Section::Games];

    /// Where a copy goes. A game's soundtrack is music, though its copy is
    /// the game's.
    pub fn of(entry: &Entry) -> Self {
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
            Self::Starred => "Favorites",
        }
    }

    /// Its picture, on its button.
    pub fn icon(self) -> iced::widget::image::Handle {
        match self {
            Self::Sounds => theme::notes_icon(),
            Self::Videos => theme::tv_icon(),
            Self::Games => theme::pad_icon(),
            Self::Starred => theme::star_icon(),
        }
    }

    /// The key that narrows the window to it, shown on its button. F stars,
    /// so Shift+F shows what is starred.
    pub fn key(self) -> &'static str {
        match self {
            Self::Sounds => "S",
            Self::Videos => "V",
            Self::Games => "G",
            Self::Starred => "⇧F",
        }
    }
}

/// One icon in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Drive,
    /// A copy, by its place in the library.
    Copy(usize),
}

/// The options for a copy, in the bar under the discs: what M, the pad's
/// select button or a right-click brings up.
#[derive(Debug, Clone, Copy)]
pub struct Menu {
    pub index: usize,
    pub item: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Play,
    Delete,
}

const ACTS: [Act; 2] = [Act::Play, Act::Delete];

pub struct Shelf {
    pub entries: Vec<Entry>,
    faces: HashMap<String, Arc<Face>>,
    /// What the window is narrowed to, if anything.
    pub filter: Option<Section>,
    /// The copies starred, by ID, in the order they were.
    favourites: Vec<String>,
    /// Words to find, as typed: only discs whose names have every one of
    /// them are shown.
    pub query: String,
    /// The chosen icon, by its place in the window as it is now.
    at: usize,
    /// What kind of disc is in the drive, if one is in and read.
    drive_kind: Option<Section>,
    /// A copy whose delete is waiting to be confirmed.
    pub armed: Option<String>,
    pub menu: Option<Menu>,
    /// Something that went wrong, said where it was asked for.
    pub notice: Option<String>,
    viewport: Option<Viewport>,
    /// Icons to a row, as last laid out: what Up and Down move by.
    columns: Cell<usize>,
}

impl Shelf {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            faces: HashMap::new(),
            filter: None,
            favourites: load_favourites(),
            query: String::new(),
            at: 0,
            drive_kind: None,
            armed: None,
            menu: None,
            notice: None,
            viewport: None,
            columns: Cell::new(1),
        }
    }

    /// The window as it is now: the drive first - always, with nothing
    /// narrowing it, or when its disc is the kind narrowed to - then the
    /// copies.
    fn items(&self) -> Vec<Item> {
        let words = words(&self.query);
        // A search is of the copies: the drive has a key of its own.
        let drive = words.is_empty()
            && match self.filter {
                None => true,
                Some(section) => self.drive_kind == Some(section),
            };
        drive
            .then_some(Item::Drive)
            .into_iter()
            .chain(
                (0..self.entries.len())
                    .filter(|&i| {
                        let entry = &self.entries[i];
                        self.filter.is_none_or(|s| self.holds(s, entry)) && matches(entry, &words)
                    })
                    .map(Item::Copy),
            )
            .collect()
    }

    /// How many copies a filter shows.
    pub fn count(&self, section: Section) -> usize {
        self.entries
            .iter()
            .filter(|e| self.holds(section, e))
            .count()
    }

    /// Whether a copy is among what a filter shows.
    fn holds(&self, section: Section, entry: &Entry) -> bool {
        match section {
            Section::Starred => self.starred(&entry.meta.id),
            kind => Section::of(entry) == kind,
        }
    }

    pub fn starred(&self, id: &str) -> bool {
        self.favourites.iter().any(|f| f == id)
    }

    /// Star a copy, or not any more; remembered beside the library. A copy
    /// unstarred while Favorites is showing goes from it, and the one after
    /// it is chosen.
    pub fn toggle_star(&mut self, index: usize) {
        let Some(id) = self.entries.get(index).map(|e| e.meta.id.clone()) else {
            return;
        };
        if self.starred(&id) {
            self.favourites.retain(|f| *f != id);
        } else {
            self.favourites.push(id);
        }
        save_favourites(&self.favourites);
        if self.filter == Some(Section::Starred) && self.favourites.is_empty() {
            self.filter = None;
        }
        self.at = self.at.min(self.items().len().saturating_sub(1));
    }

    /// The filters there are buttons for, in order: everything, each kind
    /// with something in it, and Favorites once anything is starred.
    pub fn filters(&self) -> Vec<Option<Section>> {
        let count = |section: Section| {
            self.entries
                .iter()
                .filter(|e| self.holds(section, e))
                .count()
        };
        std::iter::once(None)
            .chain(
                Section::ALL
                    .into_iter()
                    .filter(|&s| count(s) > 0 || self.drive_kind == Some(s))
                    .map(Some),
            )
            .chain((count(Section::Starred) > 0).then_some(Some(Section::Starred)))
            .collect()
    }

    /// The search changed: the first that fits is chosen, so Enter opens it.
    pub fn search(&mut self, query: String) -> Task<Message> {
        self.query = query;
        self.menu = None;
        self.armed = None;
        self.at = 0;
        operation::scroll_to(
            Id::from(GRID),
            AbsoluteOffset {
                x: None,
                y: Some(0.0),
            },
        )
    }

    /// Into the search field, ready to type, what was typed before kept
    /// and selected so typing replaces it.
    pub fn focus_search(&mut self) -> Task<Message> {
        self.menu = None;
        Task::batch([
            operation::focus(Id::from(SEARCH)),
            operation::select_all(Id::from(SEARCH)),
        ])
    }

    fn chosen(&self) -> Option<Item> {
        self.items().get(self.at).copied()
    }

    /// Bring the window into line with the library on disk, the same copy
    /// chosen where it is still there. Returns the copies whose faces still
    /// have to be made.
    pub fn load(&mut self, entries: Vec<Entry>) -> Vec<Entry> {
        let chosen = self.focused().map(|e| e.meta.id.clone());
        self.entries = entries;
        let items = self.items();
        self.at = chosen
            .and_then(|id| {
                items.iter().position(
                    |item| matches!(item, Item::Copy(i) if self.entries[*i].meta.id == id),
                )
            })
            .unwrap_or(self.at)
            .min(items.len().saturating_sub(1));
        let ids: Vec<_> = self.entries.iter().map(|e| e.meta.id.clone()).collect();
        self.faces.retain(|id, _| ids.contains(id));
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

    /// Learn what is in the drive, which decides whether it is shown when
    /// the window is narrowed.
    pub fn set_drive_kind(&mut self, kind: Option<Section>) {
        if self.drive_kind == kind {
            return;
        }
        let chosen = self.chosen();
        self.drive_kind = kind;
        // The drive coming or going moves everything along by one.
        if let Some(at) = chosen.and_then(|item| self.items().iter().position(|i| *i == item)) {
            self.at = at;
        }
        self.at = self.at.min(self.items().len().saturating_sub(1));
    }

    /// The copy chosen, or None when it is the drive.
    pub fn focused(&self) -> Option<&Entry> {
        match self.chosen()? {
            Item::Copy(i) => self.entries.get(i),
            Item::Drive => None,
        }
    }

    pub fn focused_index(&self) -> Option<usize> {
        match self.chosen()? {
            Item::Copy(i) => Some(i),
            Item::Drive => None,
        }
    }

    pub fn drive_chosen(&self) -> bool {
        self.chosen() == Some(Item::Drive)
    }

    /// Choose an icon, keeping the options and a delete only for the icon
    /// they were for.
    fn choose_at(&mut self, at: usize) {
        if at != self.at {
            self.menu = None;
            self.armed = None;
        }
        self.at = at;
    }

    /// Choose a copy by its place in the library, showing everything if
    /// the window is narrowed to another kind. Never scrolls: from the
    /// pointer, what it is on is in view already.
    pub fn choose(&mut self, index: usize) {
        if !self.items().contains(&Item::Copy(index)) {
            // Nothing to scroll to: the copy is about to be chosen anyway.
            let _ = self.narrow(None);
        }
        if let Some(at) = self.items().iter().position(|i| *i == Item::Copy(index)) {
            self.choose_at(at);
        }
    }

    /// Choose the drive, showing everything if it is not in the window.
    pub fn choose_drive(&mut self) {
        if !self.items().contains(&Item::Drive) {
            // The drive is first: nothing to scroll to.
            let _ = self.narrow(None);
        }
        self.choose_at(0);
    }

    /// An arrow pressed: along a row, or up and down a column. Down onto a
    /// short last row lands on its last icon.
    pub fn step(&mut self, dx: isize, dy: isize) -> Task<Message> {
        let len = self.items().len() as isize;
        if len == 0 {
            return Task::none();
        }
        let columns = self.columns.get().max(1) as isize;
        let at = self.at as isize;
        let to = if dx != 0 {
            (at + dx).clamp(0, len - 1)
        } else {
            let to = at + dy * columns;
            if to < 0 {
                at
            } else if to >= len {
                // Only if there is a row below at all.
                if at / columns < (len - 1) / columns {
                    len - 1
                } else {
                    at
                }
            } else {
                to
            }
        };
        self.choose_at(to as usize);
        self.reveal()
    }

    /// Narrow the window to a kind, or, asked for the kind it is narrowed
    /// to already, show everything again. The same disc stays chosen if it
    /// is still there.
    pub fn toggle(&mut self, section: Section) -> Task<Message> {
        let to = if self.filter == Some(section) {
            None
        } else {
            Some(section)
        };
        self.narrow(to)
    }

    pub fn narrow(&mut self, filter: Option<Section>) -> Task<Message> {
        // Nothing starred: nothing to show, so no Favorites to go to.
        if filter == Some(Section::Starred) && self.favourites.is_empty() {
            return Task::none();
        }
        if filter == self.filter {
            return Task::none();
        }
        let chosen = self.chosen();
        self.filter = filter;
        self.at = chosen
            .and_then(|item| self.items().iter().position(|i| *i == item))
            .unwrap_or(0);
        self.menu = None;
        self.armed = None;
        self.reveal()
    }

    /// The next narrowing along, or the one before: everything, Sounds,
    /// Videos, Games, and round again.
    pub fn cycle(&mut self, forward: bool) -> Task<Message> {
        // Only through the buttons there are: none for an empty kind.
        let all = self.filters();
        let n = all.len();
        let i = all.iter().position(|f| *f == self.filter).unwrap_or(0);
        let to = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        self.narrow(all[to])
    }

    pub fn scrolled(&mut self, viewport: Viewport) {
        self.viewport = Some(viewport);
    }

    /// Scroll just enough to show the chosen icon, and not at all if it is
    /// in view.
    fn reveal(&self) -> Task<Message> {
        let Some(viewport) = self.viewport else {
            return Task::none();
        };
        let columns = self.columns.get().max(1);
        let top = PAD + (self.at / columns) as f32 * CELL.height;
        let bottom = top + CELL.height;
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

    /// The options for the chosen copy, from the first.
    pub fn open_menu(&mut self) {
        if let Some(index) = self.focused_index() {
            self.menu = Some(Menu { index, item: 0 });
        }
    }

    pub fn menu_step(&mut self, forward: bool) {
        if let Some(menu) = &mut self.menu {
            menu.item = if forward {
                (menu.item + 1).min(ACTS.len() - 1)
            } else {
                menu.item.saturating_sub(1)
            };
            if ACTS[menu.item] != Act::Delete {
                self.armed = None;
            }
        }
    }

    pub fn menu_act(&self) -> Option<(usize, Act)> {
        self.menu.map(|menu| (menu.index, ACTS[menu.item]))
    }
}

/// Where the favourites are kept: beside the library.
fn favourites_file() -> Option<std::path::PathBuf> {
    Some(
        spectra_core::library::root()?
            .parent()?
            .join("favorites.json"),
    )
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

/// What the window needs from the rest of the app.
pub struct Context<'a> {
    pub style: Style,
    /// The copy on the stage, if one is.
    pub picked: Option<&'a str>,
    /// The mouse is in use, not keys or a pad.
    pub pointing: bool,
    /// Something is on the stage, for Back to go to.
    pub stage: bool,
    pub drive: Drive,
}

/// The drive, as its icon shows it.
pub struct Drive {
    /// The disc in it, if one is.
    pub icon: Option<Icon>,
    pub title: String,
    /// "1997  ·  14 tracks  ·  52 min", or a game's serial.
    pub details: Option<String>,
}

/// What a copy holds, for its section. A soundtrack is music, though its
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
fn icon_image<'a>(
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

/// A name under an icon: ink on the window's grey; picked out in blue when
/// chosen.
fn name<'a>(words: String, chosen: bool, width: f32) -> Element<'a, Message> {
    let words = text(words)
        .font(FONT)
        .size(12)
        .color(if chosen { Color::WHITE } else { theme::ink() })
        .align_x(Horizontal::Center);
    container(
        container(words)
            .padding([1, 4])
            .max_width(width)
            .style(move |_| container::Style {
                background: chosen.then_some(Background::Color(theme::BLUE)),
                ..Default::default()
            }),
    )
    .width(width)
    .height(NAME)
    .align_x(Horizontal::Center)
    .clip(true)
    .into()
}

impl Shelf {
    /// How big a window the discs want, within `most`: wide enough for the
    /// toolbar, then as many across as there are up to the room, and as
    /// many rows as that takes. A small library makes a small window, with
    /// the desktop showing round it; a large one fills the desktop and
    /// scrolls.
    pub fn wanted(&self, most: Size) -> Size {
        // Sized for the whole library, not what a filter or a search leaves
        // showing: the window keeps still while it is narrowed.
        // The title bar, the toolbar, the status bar and the frame.
        const CHROME_H: f32 = 30.0 + 50.0 + 34.0 + 4.0;
        // Padding either side, the scrollbar, and the frame.
        const CHROME_W: f32 = 2.0 * PAD + 16.0 + 4.0;
        const LEAST_W: f32 = 660.0;
        let n = self.entries.len() + 1;
        let fit = (((most.width - CHROME_W) / CELL.width).floor() as usize).max(1);
        let columns = n.min(fit).max(3);
        let rows = n.div_ceil(columns).max(1);
        Size::new(
            (columns as f32 * CELL.width + CHROME_W)
                .max(LEAST_W)
                .min(most.width),
            (rows as f32 * CELL.height + 2.0 * PAD + CHROME_H).min(most.height),
        )
    }

    /// The window: its title, and what is in it, `size` being the window's.
    pub fn window<'a>(&'a self, size: Size, cx: &Context<'a>) -> (String, Element<'a, Message>) {
        let items = self.items();
        // The counts are on the buttons: the title says what is showing.
        let title = self.filter.map_or("My discs", Section::name).to_string();
        let columns = (((size.width - 2.0 * PAD - 16.0) / CELL.width).floor() as usize).max(1);
        self.columns.set(columns);
        let grid: Element<'_, Message> = if items.is_empty() {
            container(self.empty(cx)).padding(PAD).width(Fill).into()
        } else {
            let rows = items.chunks(columns).enumerate().map(|(r, chunk)| {
                row(chunk
                    .iter()
                    .enumerate()
                    .map(|(c, &item)| self.icon(r * columns + c, item, cx)))
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
                    style.vertical_rail.scroller.background = Background::Color(theme::silver());
                    style.vertical_rail.scroller.border = Border {
                        color: theme::ink(),
                        width: 2.0,
                        radius: 0.0.into(),
                    };
                    style
                })
                .width(Fill)
                .height(Fill)
                .into()
        };
        let body = column![
            self.toolbar(cx),
            container(grid).height(Fill),
            self.status(cx),
        ];
        (title, body.into())
    }

    /// Along the top: everything, then each kind with how many there are,
    /// the one showing pressed in. A badge on each says its key.
    fn toolbar<'a>(&'a self, cx: &Context<'a>) -> Element<'a, Message> {
        let keys = cx.style == Style::Keys && !cx.pointing;
        let pad = matches!(cx.style, Style::Pad(_)) && !cx.pointing;
        let button = |words: &str, count: usize, key: Option<&'static str>, on: bool, to| {
            // White on the blue of the one showing, ink on the rest.
            let ink = if on { Color::WHITE } else { theme::ink() };
            let mut line = row![
                text(format!("{} ({count})", words.to_uppercase()))
                    .font(if on { BOLD } else { FONT })
                    .size(13)
                    .color(ink)
                    .wrapping(text::Wrapping::None),
            ]
            .spacing(8)
            .align_y(Vertical::Center);
            if keys && let Some(key) = key {
                line = line.push(ui::key_cap(key, 12.0, Color { a: 0.8, ..ink }));
            }
            theme::button_with(line, Some(to), on)
        };
        let count = |section: Section| {
            self.entries
                .iter()
                .filter(|e| self.holds(section, e))
                .count()
        };
        let mut filters = row![].spacing(8).align_y(Vertical::Center);
        // The shoulder buttons go along the filters: shown at either end.
        if pad {
            filters = filters.push(ui::prompt("{prev}", cx.style, 12.0, theme::faint(0.8)));
        }
        // A kind is its picture, its count and its key: the name is in its
        // tooltip, so the filters take little room.
        let pictured = |picture: iced::widget::image::Handle,
                        name: &'static str,
                        count: usize,
                        key: Option<&'static str>,
                        on: bool,
                        to: Message|
         -> Element<'a, Message> {
            let ink = if on { Color::WHITE } else { theme::ink() };
            let mut line = row![
                image(picture)
                    .width(18)
                    .height(18)
                    .filter_method(FilterMethod::Nearest),
                text(format!("({count})"))
                    .font(if on { BOLD } else { FONT })
                    .size(13)
                    .color(ink),
            ]
            .spacing(6)
            .align_y(Vertical::Center);
            if keys && let Some(key) = key {
                line = line.push(ui::key_cap(key, 12.0, Color { a: 0.8, ..ink }));
            }
            theme::tip(
                theme::button_with(line, Some(to), on),
                name,
                iced::widget::tooltip::Position::Bottom,
            )
        };
        for filter in self.filters() {
            let on = self.filter == filter;
            filters = filters.push(match filter {
                None => button("All", self.entries.len(), None, on, Message::Narrow(None)).into(),
                Some(section) => pictured(
                    section.icon(),
                    section.name(),
                    count(section),
                    Some(section.key()),
                    on,
                    Message::Narrow(filter),
                ),
            });
        }
        if pad {
            filters = filters.push(ui::prompt("{next}", cx.style, 12.0, theme::faint(0.8)));
        }
        let bar = row![filters, space().width(Fill), self.search_field()]
            .spacing(8)
            .align_y(Vertical::Center);
        container(bar)
            .padding([8, 10])
            .width(Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(theme::silver())),
                shadow: iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
                    offset: iced::Vector::new(0.0, 1.0),
                    blur_radius: 0.0,
                },
                ..Default::default()
            })
            .into()
    }

    /// The search field, let into the toolbar: a sunken box of paper, its
    /// key beside it.
    fn search_field<'a>(&'a self) -> Element<'a, Message> {
        // The key is in the field itself, so it is there to read whatever
        // was last in hand.
        let field = text_input("Press / to search", &self.query)
            .id(SEARCH)
            .on_input(Message::Search)
            .on_submit(Message::SearchSubmit)
            .font(FONT)
            .size(13)
            .padding([6, 8])
            .width(220)
            .style(|_, status| {
                let focused = matches!(status, text_input::Status::Focused { .. });
                text_input::Style {
                    background: Background::Color(theme::paper()),
                    border: Border {
                        color: if focused { theme::BLUE } else { theme::ink() },
                        width: 2.0,
                        radius: 0.0.into(),
                    },
                    icon: theme::faint(0.6),
                    placeholder: theme::faint(0.45),
                    value: theme::ink(),
                    selection: Color {
                        a: 0.35,
                        ..theme::BLUE
                    },
                }
            });
        field.into()
    }

    fn face(&self, entry: &Entry) -> Arc<Face> {
        self.faces
            .get(&entry.meta.id)
            .cloned()
            .unwrap_or_else(|| Arc::new(Face::blank()))
    }

    fn icon<'a>(&'a self, at: usize, item: Item, cx: &Context<'a>) -> Element<'a, Message> {
        let chosen = self.at == at;
        let (picture, mut words): (Element<'_, Message>, String) = match item {
            Item::Drive => (
                match &cx.drive.icon {
                    Some(disc) => icon_image(disc, chosen, ICON, false, 1.0),
                    // No disc: the drive itself, faded as a disabled icon
                    // was, its name saying why. It can still be chosen, to
                    // say so.
                    None => icon_image(&theme::drive_icon(), chosen, ICON, true, 0.45),
                },
                cx.drive.title.clone(),
            ),
            Item::Copy(index) => {
                let entry = &self.entries[index];
                (
                    icon_image(&self.face(entry).icon, chosen, ICON, false, 1.0),
                    entry.meta.title.clone(),
                )
            }
        };
        // A favourite wears a star at its shoulder.
        let picture: Element<'_, Message> = match item {
            Item::Copy(index) if self.starred(&self.entries[index].meta.id) => stack![
                picture,
                container(
                    image(theme::star_icon())
                        .width(30)
                        .height(30)
                        .filter_method(FilterMethod::Nearest)
                )
                .width(ICON)
                .align_right(ICON),
            ]
            .into(),
            _ => picture,
        };
        // On the stage now: marked in its name.
        let on_stage = match item {
            Item::Copy(i) => cx.picked == Some(self.entries[i].meta.id.as_str()),
            Item::Drive => cx.picked.is_none() && cx.stage && cx.drive.icon.is_some(),
        };
        if on_stage {
            words = format!("▶ {words}");
        }
        let cell = column![
            container(picture)
                .width(CELL.width)
                .align_x(Horizontal::Center)
                .padding(iced::Padding {
                    top: 8.0,
                    ..iced::Padding::ZERO
                }),
            name(words, chosen, CELL.width - 8.0),
        ]
        .width(CELL.width)
        .height(CELL.height)
        .align_x(Horizontal::Center)
        .spacing(4);
        let area = mouse_area(cell);
        match item {
            Item::Drive => area
                .on_move(|_| Message::DriveFocus)
                .on_press(Message::DrivePick)
                .into(),
            Item::Copy(index) => area
                .on_move(move |_| Message::ShelfFocus(index))
                .on_press(Message::ShelfPick(index))
                .on_right_press(Message::ShelfMenu(index))
                .into(),
        }
    }

    /// The status bar along the bottom: what is chosen, and what can be
    /// done with it - the options, when they are up, or what the keys do.
    fn status<'a>(&'a self, cx: &Context<'a>) -> Element<'a, Message> {
        let (what, about) = match self.chosen() {
            Some(Item::Drive) => (
                cx.drive.title.clone(),
                [Some("In the drive".to_string()), cx.drive.details.clone()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("  ·  "),
            ),
            Some(Item::Copy(i)) => (self.entries[i].meta.title.clone(), byline(&self.entries[i])),
            None => (String::new(), String::new()),
        };
        let id = self.focused().map(|e| e.meta.id.as_str());
        let right: Option<Element<'_, Message>> = if let Some(notice) = &self.notice {
            Some(theme::sunken(text(notice.clone()).font(FONT).size(12).color(theme::red())).into())
        } else if let Some(menu) = self.menu {
            Some(self.options(menu))
        } else if id.is_some() && self.armed.as_deref() == id {
            Some(
                theme::sunken(if cx.pointing {
                    text("Delete? Choose Delete again to confirm")
                        .font(FONT)
                        .size(12)
                        .color(theme::red())
                        .into()
                } else {
                    ui::prompt("{select} again to delete", cx.style, 12.0, theme::red())
                })
                .into(),
            )
        } else if cx.pointing {
            None
        } else {
            let words = self.keys(cx);
            (!words.is_empty()).then(|| {
                theme::sunken(ui::prompt(&words, cx.style, 12.0, theme::faint(0.7))).into()
            })
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
        .push(right)
        .spacing(4)
        .align_y(Vertical::Center);
        container(line)
            .padding(4)
            .width(Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(theme::silver())),
                ..Default::default()
            })
            .into()
    }

    /// What the keys do here, for the status bar.
    fn keys(&self, cx: &Context<'_>) -> String {
        let mut words = String::new();
        if self.drive_chosen() {
            if cx.drive.icon.is_some() {
                words.push_str("{accept} Open  ");
            }
        } else if self.focused().is_some() {
            words.push_str("{accept} Open  {select} Options  ");
        }
        // A keyboard's filters are its letters, on the buttons above.
        if let Style::Pad(_) = cx.style {
            words.push_str("{prev}{next} Filter  ");
        }
        if self.filter.is_some() {
            words.push_str("{back} All");
        } else if cx.stage {
            words.push_str("{back} Stage");
        }
        words.trim_end().to_string()
    }

    /// The options, side by side in the status bar, the one the keys are
    /// on pressed in.
    fn options<'a>(&'a self, menu: Menu) -> Element<'a, Message> {
        let armed = self
            .entries
            .get(menu.index)
            .is_some_and(|e| self.armed.as_deref() == Some(e.meta.id.as_str()));
        let items = ACTS.iter().enumerate().map(|(i, &act)| {
            let (words, message) = match act {
                Act::Play => ("Open", Message::ShelfPick(menu.index)),
                Act::Delete if armed => {
                    ("Delete? Again to confirm", Message::ShelfDelete(menu.index))
                }
                Act::Delete => ("Delete", Message::ShelfDelete(menu.index)),
            };
            theme::button_text(words, Some(message), i == menu.item)
        });
        row![]
            .extend(items)
            .push(theme::button_text("Close", Some(Message::MenuClose), false))
            .spacing(6)
            .align_y(Vertical::Center)
            .into()
    }

    fn empty<'a>(&'a self, cx: &Context<'a>) -> Element<'a, Message> {
        if !self.query.trim().is_empty() {
            let words = format!(
                "Nothing kept is called \"{}\".{}",
                self.query.trim(),
                if cx.pointing {
                    ""
                } else {
                    " {back} to clear the search."
                }
            );
            return dashed(ui::prompt(&words, cx.style, 13.0, theme::faint(0.6)));
        }
        let what = match self.filter {
            Some(Section::Sounds) => "music",
            Some(Section::Videos) => "films",
            Some(Section::Games) => "games",
            // Favorites only show while something is starred.
            None | Some(Section::Starred) => "discs",
        };
        let words = format!(
            "No {what} kept yet. With a disc in the drive, {}.",
            if cx.pointing {
                "choose Keep a copy"
            } else {
                "press {alt} to keep a copy"
            }
        );
        dashed(ui::prompt(&words, cx.style, 13.0, theme::faint(0.6)))
    }
}

/// Words said where there are no discs to show, in a dashed box.
fn dashed<'a>(words: Element<'a, Message>) -> Element<'a, Message> {
    let card = stack![
        ui::Dashed {
            color: theme::faint(0.3),
            radius: 4.0,
        },
        container(container(words).max_width(448))
            .padding([40, 32])
            .center_x(Fill),
    ];
    container(card).width(Fill).height(140).into()
}

/// A search, as words to look for, in lower case.
fn words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// Whether every word is in a copy's name, its artist or its console:
/// "tomb" finds Tomb Raider, "ps2" and "playstation" the games for it.
fn matches(entry: &Entry, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let mut about = format!("{} {}", entry.meta.title, byline(entry)).to_lowercase();
    if let Some(system) = entry.meta.system {
        about.push(' ');
        about.push_str(&format!("{system:?}").to_lowercase());
    }
    words.iter().all(|w| about.contains(w.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_starts_with_the_drive_then_every_copy() {
        let mut shelf = Shelf::new();
        shelf.entries = vec![game(0), album(1)];
        assert_eq!(
            shelf.items(),
            vec![Item::Drive, Item::Copy(0), Item::Copy(1)]
        );
        assert!(shelf.drive_chosen());
    }

    #[test]
    fn a_filter_narrows_the_window_and_the_same_key_shows_everything() {
        let mut shelf = Shelf::new();
        shelf.entries = vec![game(0), album(1), game(2), soundtrack(0)];
        assert_eq!(shelf.items().len(), 5, "the drive and four copies");
        shelf.choose(2);
        let _ = shelf.toggle(Section::Games);
        assert_eq!(shelf.items(), vec![Item::Copy(0), Item::Copy(2)]);
        assert_eq!(shelf.focused_index(), Some(2), "still chosen");
        let _ = shelf.toggle(Section::Sounds);
        assert_eq!(shelf.items(), vec![Item::Copy(1), Item::Copy(3)]);
        let _ = shelf.toggle(Section::Sounds);
        assert_eq!(shelf.filter, None);
        assert_eq!(shelf.items().len(), 5);
    }

    #[test]
    fn the_drive_stays_in_a_narrowed_window_when_its_disc_is_that_kind() {
        let mut shelf = Shelf::new();
        shelf.entries = vec![game(0), album(1)];
        shelf.set_drive_kind(Some(Section::Games));
        let _ = shelf.narrow(Some(Section::Games));
        assert_eq!(shelf.items(), vec![Item::Drive, Item::Copy(0)]);
        let _ = shelf.narrow(Some(Section::Sounds));
        assert_eq!(shelf.items(), vec![Item::Copy(1)]);
    }

    #[test]
    fn the_arrows_go_along_a_row_and_down_a_column() {
        let mut shelf = Shelf::new();
        shelf.columns.set(4);
        shelf.entries = (0..6).map(game).collect();
        // The drive and six games: two rows of four, the second short.
        let _ = shelf.step(1, 0);
        let _ = shelf.step(1, 0);
        assert_eq!(shelf.focused_index(), Some(1));
        let _ = shelf.step(0, 1);
        assert_eq!(shelf.focused_index(), Some(5));
        let _ = shelf.step(1, 0);
        assert_eq!(shelf.focused_index(), Some(5), "no further than the last");
        let _ = shelf.step(0, 1);
        assert_eq!(shelf.focused_index(), Some(5), "no row below");
        let _ = shelf.step(0, -1);
        assert_eq!(shelf.focused_index(), Some(1), "straight up");
        let _ = shelf.step(-1, 0);
        let _ = shelf.step(-1, 0);
        assert!(shelf.drive_chosen());
        let _ = shelf.step(-1, 0);
        assert!(shelf.drive_chosen(), "no further than the first");
    }

    fn meta(id: String, title: String, system: Option<GameSystem>) -> spectra_core::library::Meta {
        spectra_core::library::Meta {
            id,
            title,
            system,
            serial: None,
            publisher: None,
            year: None,
            region: None,
            disc_art: None,
            artist: None,
            tracks: Vec::new(),
            sectors: 0,
            unreadable: 0,
            created: 0,
        }
    }

    fn game(n: usize) -> Entry {
        Entry {
            dir: std::path::PathBuf::from("/nowhere"),
            meta: meta(
                format!("SLUS-{n:05}"),
                format!("Game {n}"),
                Some(GameSystem::Ps1),
            ),
        }
    }

    fn album(n: usize) -> Entry {
        Entry {
            dir: std::path::PathBuf::from("/nowhere"),
            meta: meta(format!("cd-{n}"), format!("Album {n}"), None),
        }
    }

    fn soundtrack(n: usize) -> Entry {
        crate::soundtrack::entry(&game(n))
    }
}
