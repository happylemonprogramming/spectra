//! The start menu: what the start button opens, as it was in its day.
//!
//! A blue band across the top with Spectra's name; under it, two columns.
//! The white one on the left is where to go - your discs, each kind, search,
//! the drive - and, at its foot, every key. The pale blue one on the right
//! is the few things to choose - night or day, whether to look things up
//! online - and what Spectra has to work with: the players it hands discs
//! to, the drive, how much is kept. Along the bottom, another blue band,
//! with the version and the way out.
//!
//! The keys and the pad move one highlight through it, top to bottom and
//! column to column, and the pointer moves the same highlight, so there is
//! only ever one thing lit.

use iced::alignment::Vertical;
use iced::gradient::Linear;
use iced::widget::image::FilterMethod;
use iced::widget::{column, container, image, mouse_area, row, scrollable, space, text, tooltip};
use iced::{Background, Border, Color, Element, Fill, Radians, Size, Task};

use crate::shelf::Section;
use crate::{BOLD, FONT, Message, READING, Remote, Spectra, settings, theme, ui};

/// Something in the start menu that can be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Discs,
    Kind(Section),
    Search,
    Drive,
    /// Every key, in place of the places; or the places again.
    Keys,
    Night,
    Online,
    ClearCache,
    Quit,
}

impl Item {
    /// Which column it is in: the left, the right, or the band along the
    /// bottom. Left and Right go between them.
    fn column(self) -> u8 {
        match self {
            Item::Discs | Item::Kind(_) | Item::Search | Item::Drive | Item::Keys => 0,
            Item::Night | Item::Online | Item::ClearCache => 1,
            Item::Quit => 2,
        }
    }
}

#[derive(Debug, Default)]
pub struct Menu {
    pub open: bool,
    /// The item lit, by its place in `items`.
    pub at: usize,
    /// Every key showing in the left column, in place of the places.
    keys: bool,
    /// Clear cache chosen once, waiting for the second.
    clearing: bool,
}

const WIDTH: f32 = 620.0;
/// The XP blue of a lit item, and the pale blue of the right column.
const LIT: Color = Color::from_rgb(
    0x31 as f32 / 255.0,
    0x6a as f32 / 255.0,
    0xc5 as f32 / 255.0,
);

/// The blue of the bands, darker towards their foot.
fn band() -> Background {
    Background::Gradient(
        Linear::new(Radians(std::f32::consts::PI))
            .add_stop(0.0, Color::from_rgb8(0x3a, 0x80, 0xf0))
            .add_stop(0.45, Color::from_rgb8(0x24, 0x5e, 0xdb))
            .add_stop(1.0, Color::from_rgb8(0x1a, 0x48, 0xb8))
            .into(),
    )
}

fn right_column() -> Color {
    if theme::night() {
        Color::from_rgb8(0x2c, 0x34, 0x4c)
    } else {
        Color::from_rgb8(0xd3, 0xe5, 0xfa)
    }
}

/// The right column's words: XP's navy, or pale by night.
fn navy() -> Color {
    if theme::night() {
        Color::from_rgb8(0xd6, 0xe2, 0xf8)
    } else {
        Color::from_rgb8(0x0a, 0x24, 0x6a)
    }
}

impl Spectra {
    /// What can be chosen, top to bottom, column by column.
    fn start_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        if self.start.keys {
            items.push(Item::Keys);
        } else {
            items.push(Item::Discs);
            items.extend(self.shelf.filters().into_iter().flatten().map(Item::Kind));
            items.extend([Item::Search, Item::Drive, Item::Keys]);
        }
        items.extend([Item::Night, Item::Online, Item::ClearCache, Item::Quit]);
        items
    }

    /// Up from the start button, looking round as it opens; or away.
    pub(crate) fn start_toggle(&mut self) -> Task<Message> {
        if self.start.open {
            self.start.open = false;
            return Task::none();
        }
        self.start = Menu {
            open: true,
            ..Menu::default()
        };
        self.look_round()
    }

    fn look_round(&self) -> Task<Message> {
        Task::perform(
            crate::blocking(|| Box::new(settings::facts())),
            Message::Facts,
        )
    }

    /// A press of the remote while the menu is up. None for one it has no
    /// use for: the menu goes, and the press does what it does elsewhere.
    pub(crate) fn start_remote(&mut self, remote: Remote) -> Option<Task<Message>> {
        let items = self.start_items();
        let n = items.len();
        let at = self.start.at.min(n - 1);
        match remote {
            // No further than the ends: from the top, Up must never come
            // round to Quit at the bottom.
            Remote::Up => self.start.at = at.saturating_sub(1),
            Remote::Down => self.start.at = (at + 1).min(n - 1),
            // To the first item in the next column, or the one before.
            Remote::Right | Remote::Left => {
                let here = items[at].column();
                let to = if matches!(remote, Remote::Right) {
                    (here + 1).min(2)
                } else {
                    here.saturating_sub(1)
                };
                if let Some(i) = items.iter().position(|item| item.column() == to) {
                    self.start.at = i;
                }
            }
            Remote::Select | Remote::PlayPause => return Some(self.start_pick(items[at])),
            Remote::Back if self.start.keys => {
                self.start.keys = false;
                self.start.at = self.start_items().iter().position(|i| *i == Item::Keys)?;
            }
            Remote::Back | Remote::Menu | Remote::Library => self.start.open = false,
            _ => return None,
        }
        Some(Task::none())
    }

    /// Do what an item says. Going somewhere puts the menu away; a choice
    /// leaves it up, to see it made.
    pub(crate) fn start_pick(&mut self, item: Item) -> Task<Message> {
        if let Some(at) = self.start_items().iter().position(|i| *i == item) {
            self.start.at = at;
        }
        if item != Item::ClearCache {
            self.start.clearing = false;
        }
        match item {
            Item::Discs => {
                self.start.open = false;
                Task::batch([self.shelf.narrow(None), self.open_library()])
            }
            Item::Kind(section) => {
                self.start.open = false;
                Task::batch([self.shelf.narrow(Some(section)), self.open_library()])
            }
            Item::Search => {
                self.start.open = false;
                Task::batch([self.open_library(), self.shelf.focus_search()])
            }
            Item::Drive => {
                self.start.open = false;
                self.open_drive()
            }
            Item::Keys => {
                self.start.keys = !self.start.keys;
                self.start.at = self
                    .start_items()
                    .iter()
                    .position(|i| *i == Item::Keys)
                    .unwrap_or(0);
                Task::none()
            }
            Item::Night => {
                theme::toggle_night();
                Task::none()
            }
            Item::Online => {
                settings::set_online(!settings::online());
                Task::none()
            }
            Item::ClearCache => {
                if !self.start.clearing {
                    self.start.clearing = true;
                    return Task::none();
                }
                self.start.clearing = false;
                let _ = settings::clear_cache();
                self.look_round()
            }
            // As Ctrl+Q does: off the TV first, if it is there.
            Item::Quit => {
                let _ = std::fs::remove_file(crate::windows::socket(std::process::id()));
                self.quit()
            }
        }
    }

    /// The menu, up from the start button: in the desktop's bottom left
    /// corner, against the taskbar.
    pub(crate) fn start_menu(&self, desk: Size) -> Element<'_, Message> {
        let items = self.start_items();
        let lit = |item: Item| items.get(self.start.at) == Some(&item);
        let keys = self.style == ui::Style::Keys && !self.pointing;
        let index = |item: Item| items.iter().position(|i| *i == item).unwrap_or(0);

        // An item: lit in XP's blue while the keys or the pointer are on it.
        let item = |item: Item, content: Element<'static, Message>| -> Element<'static, Message> {
            let on = lit(item);
            mouse_area(
                container(content)
                    .width(Fill)
                    .padding([6, 10])
                    .style(move |_| container::Style {
                        background: on.then_some(Background::Color(LIT)),
                        ..Default::default()
                    }),
            )
            .on_enter(Message::StartHover(index(item)))
            .on_press(Message::StartPick(item))
            .into()
        };
        let picture = |handle: image::Handle, side: f32| {
            image(handle)
                .width(side)
                .height(side)
                .filter_method(FilterMethod::Nearest)
        };
        let cap = |key: &'static str, on: bool| -> Option<Element<'static, Message>> {
            keys.then(|| ui::key_cap(key, 11.0, if on { Color::WHITE } else { theme::faint(0.6) }))
        };

        // The left column's places: a big picture, the name bold, a line
        // under it.
        let place = |what: Item,
                     icon: image::Handle,
                     name: &'static str,
                     about: String,
                     key: Option<&'static str>| {
            let on = lit(what);
            let ink = if on { Color::WHITE } else { theme::ink() };
            let line = row![
                picture(icon, 30.0),
                column![
                    text(name.to_uppercase()).font(BOLD).size(13).color(ink),
                    text(about)
                        .font(READING)
                        .size(12)
                        .color(Color { a: 0.7, ..ink }),
                ]
                .spacing(2)
                .width(Fill),
            ]
            .push(key.and_then(|k| cap(k, on)))
            .spacing(10)
            .align_y(Vertical::Center);
            item(what, line.into())
        };
        let rule = |colour: Color| {
            container(space())
                .height(1)
                .width(Fill)
                .style(move |_| container::Style {
                    background: Some(Background::Color(colour)),
                    ..Default::default()
                })
        };

        let left: Element<'_, Message> = if self.start.keys {
            let shortcuts = [
                ("ENTER", "Open, or play"),
                ("SPACE", "Play or pause"),
                ("S  V  G", "Sounds, Videos, Games"),
                ("⇧F", "Favorites"),
                ("/", "Search"),
                ("F", "Favorite, or not"),
                ("C", "Keep a copy"),
                ("M", "Options"),
                ("DEL", "Remove a copy"),
                ("D", "The drive"),
                ("N", "Night or day"),
                (",", "This menu"),
                ("PG UP  PG DN", "Previous, next"),
                ("BKSP", "Back"),
            ];
            let rows = shortcuts.iter().map(|&(key, does)| {
                row![
                    container(ui::key_cap(key, 10.0, theme::faint(0.75))).width(96),
                    text(does).font(READING).size(12).color(theme::faint(0.85)),
                ]
                .align_y(Vertical::Center)
                .into()
            });
            let on = lit(Item::Keys);
            let ink = if on { Color::WHITE } else { theme::ink() };
            column![
                // The keys and the credits scroll; Back stays at the foot.
                scrollable(
                    column![
                        column(rows).spacing(4),
                        text(
                            "Silkscreen by Jason Kottke (SIL OFL). Button prompts by \
                             Kenney (CC0). Film and game details from Wikidata and \
                             Wikipedia (CC BY-SA)."
                        )
                        .font(READING)
                        .size(10)
                        .color(theme::faint(0.5)),
                    ]
                    .spacing(12)
                    .padding([6, 14]),
                )
                .style(theme::scroll_style)
                .height(Fill),
                rule(theme::faint(0.15)),
                item(
                    Item::Keys,
                    row![
                        text("◀").font(READING).size(12).color(ink),
                        text("BACK").font(BOLD).size(13).color(ink),
                    ]
                    .spacing(8)
                    .align_y(Vertical::Center)
                    .into(),
                ),
            ]
            .spacing(8)
            .padding([6, 0])
            .into()
        } else {
            let mut places = column![place(
                Item::Discs,
                theme::mark_icon(),
                "My discs",
                format!("Everything kept  ({})", self.shelf.entries.len()),
                Some("L"),
            )]
            .spacing(2);
            for section in self.shelf.filters().into_iter().flatten() {
                let count = self.shelf.count(section);
                let about = match section {
                    Section::Sounds => "Albums and soundtracks",
                    Section::Videos => "Films",
                    Section::Games => "Games",
                    Section::Starred => "Starred",
                };
                places = places.push(place(
                    Item::Kind(section),
                    section.icon(),
                    section.name(),
                    format!("{about}  ({count})"),
                    Some(section.key()),
                ));
            }
            places = places.push(rule(theme::faint(0.15)));
            places = places.push(place(
                Item::Search,
                theme::magnifier_icon(),
                "Search",
                "Find a disc by its name".into(),
                Some("/"),
            ));
            places = places.push(place(
                Item::Drive,
                theme::drive_icon().plain,
                "The drive",
                self.desk_context().drive.title,
                Some("D"),
            ));
            places = places.push(space().height(Fill));
            places = places.push(rule(theme::faint(0.15)));
            let on = lit(Item::Keys);
            let ink = if on { Color::WHITE } else { theme::ink() };
            places = places.push(item(
                Item::Keys,
                row![
                    picture(theme::keyboard_icon(), 22.0),
                    text("ALL KEYS").font(BOLD).size(13).color(ink).width(Fill),
                    text("▶").font(READING).size(12).color(ink),
                ]
                .spacing(10)
                .align_y(Vertical::Center)
                .into(),
            ));
            places.padding([6, 0]).into()
        };

        // The right column: choices with their state, then what there is.
        let choice = |what: Item,
                      icon: image::Handle,
                      name: &'static str,
                      state: String,
                      key: Option<&'static str>| {
            let on = lit(what);
            let ink = if on { Color::WHITE } else { navy() };
            item(
                what,
                row![
                    picture(icon, 20.0),
                    text(name.to_uppercase())
                        .font(BOLD)
                        .size(12)
                        .color(ink)
                        .width(Fill),
                    text(state)
                        .font(FONT)
                        .size(11)
                        .color(Color { a: 0.8, ..ink }),
                ]
                .push(key.and_then(|k| cap(k, on)))
                .spacing(8)
                .align_y(Vertical::Center)
                .into(),
            )
        };
        let heading = |words: &'static str| {
            container(
                text(words)
                    .font(BOLD)
                    .size(10)
                    .color(Color { a: 0.55, ..navy() }),
            )
            .padding([4, 10])
        };
        let fact = |words: String, alpha: f32| {
            container(
                text(words)
                    .font(READING)
                    .size(13)
                    .color(Color { a: alpha, ..navy() }),
            )
            .padding([0, 10])
        };
        let night = theme::night();
        let mut right = column![
            choice(
                Item::Night,
                if night {
                    theme::sun_icon()
                } else {
                    theme::moon_icon()
                },
                if night { "Day" } else { "Night" },
                String::new(),
                Some("N"),
            ),
            choice(
                Item::Online,
                theme::globe_icon(),
                "Look things up",
                if settings::online() { "ON" } else { "OFF" }.into(),
                None,
            ),
        ]
        .spacing(2);
        match &self.facts {
            None => right = right.push(fact("Looking round…".into(), 0.6)),
            Some(facts) => {
                let cache = if self.start.clearing {
                    "AGAIN TO CLEAR".to_string()
                } else {
                    settings::bytes(facts.cache_bytes)
                };
                right = right.push(choice(
                    Item::ClearCache,
                    theme::bin_icon(),
                    "Clear cache",
                    cache,
                    None,
                ));
                right = right.push(space().height(6));
                right = right.push(rule(Color { a: 0.3, ..navy() }));
                right = right.push(heading("PLAYERS"));
                for part in &facts.parts {
                    let (mark, colour) = match part.found {
                        Some(_) => ("✓", Color::from_rgb8(0x2e, 0x9e, 0x44)),
                        None => ("✗", theme::red()),
                    };
                    let line = row![
                        text(mark).font(READING).size(13).color(colour).width(16),
                        text(part.name)
                            .font(READING)
                            .size(13)
                            .color(navy())
                            .width(110),
                        text(part.does)
                            .font(READING)
                            .size(12)
                            .color(Color { a: 0.6, ..navy() }),
                    ]
                    .align_y(Vertical::Center);
                    // Where it was found, for the pointer that asks.
                    let said = part.found.clone().unwrap_or_else(|| "Not installed".into());
                    right = right.push(
                        tooltip(
                            container(line).padding([1, 10]),
                            container(text(said).font(READING).size(12))
                                .padding([4, 8])
                                .style(|_| container::Style {
                                    background: Some(Background::Color(theme::paper())),
                                    text_color: Some(theme::ink()),
                                    border: Border {
                                        color: theme::ink(),
                                        width: 1.0,
                                        radius: 0.0.into(),
                                    },
                                    ..Default::default()
                                }),
                            tooltip::Position::Left,
                        )
                        .gap(4),
                    );
                }
                right = right.push(heading("DRIVE"));
                right = right.push(fact(
                    facts
                        .drives
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "No drive connected".into()),
                    0.85,
                ));
                right = right.push(heading("KEPT"));
                right = right.push(fact(
                    format!(
                        "{} {}  ·  {}",
                        facts.copies,
                        if facts.copies == 1 { "copy" } else { "copies" },
                        settings::bytes(facts.library_bytes)
                    ),
                    0.85,
                ));
            }
        }

        let header = container(
            row![
                container(picture(theme::mark_icon(), 40.0))
                    .padding(3)
                    .style(|_| container::Style {
                        background: Some(Background::Color(Color::WHITE)),
                        border: Border {
                            color: Color::from_rgb8(0xd8, 0xe4, 0xf8),
                            width: 2.0,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    }),
                column![
                    text("SPECTRA").font(BOLD).size(18).color(Color::WHITE),
                    text("Put a disc in, and it plays.")
                        .font(READING)
                        .size(13)
                        .color(Color::from_rgba(1.0, 1.0, 1.0, 0.85)),
                ]
                .spacing(2),
            ]
            .spacing(12)
            .align_y(Vertical::Center),
        )
        .padding([10, 12])
        .width(Fill)
        .style(|_| container::Style {
            background: Some(band()),
            border: Border {
                radius: iced::border::Radius::default().top_left(8.0).top_right(8.0),
                ..Border::default()
            },
            ..Default::default()
        });
        // XP's orange line under the band, fading out to the right.
        let orange = container(space())
            .height(3)
            .width(Fill)
            .style(|_| container::Style {
                background: Some(Background::Gradient(
                    Linear::new(Radians(std::f32::consts::FRAC_PI_2))
                        .add_stop(0.0, Color::from_rgba8(0xf0, 0x8a, 0x24, 0.0))
                        .add_stop(0.5, Color::from_rgb8(0xf0, 0x8a, 0x24))
                        .add_stop(1.0, Color::from_rgba8(0xf0, 0x8a, 0x24, 0.0))
                        .into(),
                )),
                ..Default::default()
            });
        let quit = item(
            Item::Quit,
            row![
                picture(theme::power_icon(), 22.0),
                text("QUIT SPECTRA").font(BOLD).size(12).color(Color::WHITE),
            ]
            .spacing(8)
            .align_y(Vertical::Center)
            .into(),
        );
        let quit = container(quit).width(190);
        let footer = container(
            row![
                text(format!(
                    "Version {}  ·  GPL-3.0-or-later",
                    env!("CARGO_PKG_VERSION")
                ))
                .font(READING)
                .size(12)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.8))
                .width(Fill),
                quit,
            ]
            .align_y(Vertical::Center),
        )
        .padding([4, 10])
        .width(Fill)
        .style(|_| container::Style {
            background: Some(band()),
            ..Default::default()
        });

        let body = row![
            container(left)
                .width(WIDTH / 2.0)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Background::Color(theme::paper())),
                    ..Default::default()
                }),
            container(right.padding([6, 0]))
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Background::Color(right_column())),
                    border: Border {
                        color: Color::from_rgb8(0x95, 0xbd, 0xe7),
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                }),
        ];
        let height = (desk.height - 8.0).min(560.0);
        let menu =
            container(column![header, orange, container(body).height(Fill), footer].width(WIDTH))
                .height(height)
                .padding(2)
                .style(|_| container::Style {
                    background: Some(Background::Color(Color::from_rgb8(0x1a, 0x48, 0xb8))),
                    border: Border {
                        color: Color::from_rgb8(0x1a, 0x48, 0xb8),
                        width: 2.0,
                        radius: iced::border::Radius::default()
                            .top_left(10.0)
                            .top_right(10.0),
                    },
                    shadow: theme::hard_shadow(5.0),
                    ..Default::default()
                });
        // Swallows clicks on itself, so only a click off it puts it away.
        let menu = mouse_area(menu).on_press(Message::StartHover(self.start.at));
        container(menu)
            .width(Fill)
            .height(Fill)
            .align_bottom(Fill)
            .align_left(Fill)
            .into()
    }
}
