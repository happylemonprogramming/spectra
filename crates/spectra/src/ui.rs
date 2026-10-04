//! Pieces of the screen the stage and the library share: button prompts
//! drawn as the buttons look, quiet buttons, and the library's own light.
//!
//! The prompts are Kenney's Input Prompts (CC0), as Rainbow Player uses
//! them, rasterised once to small PNGs in `assets/prompts` and built in: a
//! few dozen icons, about 60 KB, decoded the first time a style is shown.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use iced::advanced::layout::{self, Layout};
use iced::advanced::mouse;
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Tree, Widget};
use iced::alignment::Vertical;
use iced::widget::{button, image, row, text};
use iced::{Background, Border, Color, Element, Length, Rectangle, Size};

use crate::{FONT, Message};

/// A family of controllers that print the same things on their buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pad {
    Ps5,
    Ps4,
    Ps3,
    Xbox,
    Switch,
    SteamDeck,
    SteamController,
}

impl Pad {
    /// By the USB IDs the pad reports, falling back to its name. Most other
    /// pads are made to pass for an Xbox controller.
    pub fn identify(vendor: Option<u16>, product: Option<u16>, name: &str) -> Self {
        let name = name.to_ascii_lowercase();
        match vendor {
            Some(0x054c) => match product {
                Some(0x0ce6 | 0x0df2) => Self::Ps5,
                Some(0x0268) => Self::Ps3,
                _ if name.contains("dualsense") => Self::Ps5,
                _ => Self::Ps4,
            },
            Some(0x045e) => Self::Xbox,
            Some(0x057e) => Self::Switch,
            // Valve: the Deck's own controls, the Steam Controller, or Steam
            // Input's virtual pad, which on a Deck stands for the Deck.
            Some(0x28de) => match product {
                Some(0x1102 | 0x1142) => Self::SteamController,
                _ => Self::SteamDeck,
            },
            _ if name.contains("dualsense") => Self::Ps5,
            _ if name.contains("dualshock") || name.contains("playstation") => Self::Ps4,
            _ => Self::Xbox,
        }
    }
}

/// Where the last input came from, which decides what prompts look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Keyboard and mouse: prompts are keys.
    Keys,
    Pad(Pad),
}

/// The buttons a prompt can name, by what they do rather than where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Glyph {
    Accept,
    Back,
    Alt,
    Select,
    Start,
    Prev,
    Next,
    Up,
    Down,
    /// The library: L on a keyboard, the select button on a pad.
    Library,
}

impl Glyph {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "accept" => Self::Accept,
            "back" => Self::Back,
            "alt" => Self::Alt,
            "select" => Self::Select,
            "start" => Self::Start,
            "prev" => Self::Prev,
            "next" => Self::Next,
            "up" => Self::Up,
            "down" => Self::Down,
            "library" => Self::Library,
            _ => return None,
        })
    }

    /// The icon's file, without `.png`.
    fn icon(self, style: Style) -> &'static str {
        use Glyph::*;
        let pad = match style {
            Style::Keys => {
                return match self {
                    Accept => "keyboard_enter",
                    Back => "keyboard_backspace",
                    Alt => "keyboard_c",
                    Select => "keyboard_m",
                    Library => "keyboard_l",
                    Start => "keyboard_space",
                    Prev => "keyboard_arrow_left",
                    Next => "keyboard_arrow_right",
                    Up => "keyboard_arrow_up",
                    Down => "keyboard_arrow_down",
                };
            }
            Style::Pad(pad) => pad,
        };
        // Glyphs go by position, as the buttons do: Accept is the bottom face
        // button, which is B on a Nintendo pad. Face buttons are in colour
        // only where the pad's own are.
        match (pad, self) {
            (Pad::Ps5, Accept) => "playstation_button_cross",
            (Pad::Ps5, Back) => "playstation_button_circle",
            (Pad::Ps5, Alt) => "playstation_button_triangle",
            (Pad::Ps5, Select | Library) => "playstation5_button_create",
            (Pad::Ps5, Start) => "playstation5_button_options",
            (Pad::Ps4 | Pad::Ps3, Accept) => "playstation_button_color_cross",
            (Pad::Ps4 | Pad::Ps3, Back) => "playstation_button_color_circle",
            (Pad::Ps4 | Pad::Ps3, Alt) => "playstation_button_color_triangle",
            (Pad::Ps4, Select | Library) => "playstation4_button_share",
            (Pad::Ps4, Start) => "playstation4_button_options",
            (Pad::Ps3, Select | Library) => "playstation3_button_select",
            (Pad::Ps3, Start) => "playstation3_button_start",
            (Pad::Ps5 | Pad::Ps4 | Pad::Ps3, Prev) => "playstation_trigger_l1",
            (Pad::Ps5 | Pad::Ps4 | Pad::Ps3, Next) => "playstation_trigger_r1",
            (Pad::Ps5 | Pad::Ps4 | Pad::Ps3, Up) => "playstation_dpad_up",
            (Pad::Ps5 | Pad::Ps4 | Pad::Ps3, Down) => "playstation_dpad_down",
            (Pad::Xbox, Accept) => "xbox_button_color_a",
            (Pad::Xbox, Back) => "xbox_button_color_b",
            (Pad::Xbox, Alt) => "xbox_button_color_y",
            (Pad::Xbox, Select | Library) => "xbox_button_view",
            (Pad::Xbox, Start) => "xbox_button_menu",
            (Pad::Xbox, Prev) => "xbox_lb",
            (Pad::Xbox, Next) => "xbox_rb",
            (Pad::Xbox, Up) => "xbox_dpad_up",
            (Pad::Xbox, Down) => "xbox_dpad_down",
            (Pad::Switch, Accept) => "switch_button_b",
            (Pad::Switch, Back) => "switch_button_a",
            (Pad::Switch, Alt) => "switch_button_x",
            (Pad::Switch, Select | Library) => "switch_button_minus",
            (Pad::Switch, Start) => "switch_button_plus",
            (Pad::Switch, Prev) => "switch_button_l",
            (Pad::Switch, Next) => "switch_button_r",
            (Pad::Switch, Up) => "switch_dpad_up",
            (Pad::Switch, Down) => "switch_dpad_down",
            (Pad::SteamDeck, Accept) => "steamdeck_button_a",
            (Pad::SteamDeck, Back) => "steamdeck_button_b",
            (Pad::SteamDeck, Alt) => "steamdeck_button_y",
            (Pad::SteamDeck, Select | Library) => "steamdeck_button_view",
            (Pad::SteamDeck, Start) => "steamdeck_button_options",
            (Pad::SteamDeck, Prev) => "steamdeck_button_l1",
            (Pad::SteamDeck, Next) => "steamdeck_button_r1",
            (Pad::SteamDeck, Up) => "steamdeck_dpad_up",
            (Pad::SteamDeck, Down) => "steamdeck_dpad_down",
            (Pad::SteamController, Accept) => "steam_button_a",
            (Pad::SteamController, Back) => "steam_button_b",
            (Pad::SteamController, Alt) => "steam_button_y",
            (Pad::SteamController, Select | Library) => "steam_button_back_icon",
            (Pad::SteamController, Start) => "steam_button_start_icon",
            (Pad::SteamController, Prev) => "steam_lb",
            (Pad::SteamController, Next) => "steam_rb",
            (Pad::SteamController, Up) => "steam_dpad_up",
            (Pad::SteamController, Down) => "steam_dpad_down",
        }
    }
}

/// Every icon a prompt can draw, built in.
const ICONS: &[(&str, &[u8])] = &include!("../assets/prompts/icons.rs");

/// An icon, decoded the first time it is asked for.
fn icon(name: &'static str) -> Option<image::Handle> {
    static DECODED: OnceLock<Mutex<HashMap<&'static str, image::Handle>>> = OnceLock::new();
    let decoded = DECODED.get_or_init(Default::default);
    let mut decoded = decoded.lock().ok()?;
    if let Some(handle) = decoded.get(name) {
        return Some(handle.clone());
    }
    let (_, bytes) = ICONS.iter().find(|(n, _)| *n == name)?;
    let pixels = ::image::load_from_memory(bytes).ok()?.to_rgba8();
    let handle = image::Handle::from_rgba(pixels.width(), pixels.height(), pixels.into_raw());
    decoded.insert(name, handle.clone());
    Some(handle)
}

/// A line with buttons in it: `"{accept} Play"` becomes a drawn button and
/// the word. Templates stay plain strings, so notes can be kept and compared
/// like any other text, and drawn for whichever controller is in hand.
pub fn prompt<'a>(template: &str, style: Style, size: f32, color: Color) -> Element<'a, Message> {
    let mut line = row![].align_y(Vertical::Center);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|c| open + c) else {
            break;
        };
        let words = &rest[..open];
        if !words.is_empty() {
            line = line.push(text(words.to_string()).font(FONT).size(size).color(color));
        }
        let name = &rest[open + 1..close];
        match Glyph::parse(name).and_then(|g| icon(g.icon(style))) {
            // The art has an eighth of padding on each side, so it is drawn
            // larger than the text, as Rainbow Player draws it.
            Some(handle) => {
                line = line.push(
                    image(handle)
                        .width(size * 1.9)
                        .height(size * 1.9)
                        .opacity(color.a.max(0.6)),
                );
            }
            None => {
                line = line.push(
                    text(rest[open..=close].to_string())
                        .font(FONT)
                        .size(size)
                        .color(color),
                );
            }
        }
        rest = &rest[close + 1..];
    }
    if !rest.is_empty() {
        line = line.push(text(rest.to_string()).font(FONT).size(size).color(color));
    }
    line.into()
}

/// A button that stays out of the way until the pointer finds it: small,
/// dim and pill-shaped, as Rainbow Player's quiet buttons are.
pub fn quiet<'a>(
    content: impl Into<Element<'a, Message>>,
    on_press: Message,
) -> Element<'a, Message> {
    button(content)
        .padding([8, 16])
        .on_press(on_press)
        .style(|_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: hovered
                    .then_some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.05))),
                text_color: Color::from_rgba(1.0, 1.0, 1.0, if hovered { 0.8 } else { 0.45 }),
                border: Border {
                    radius: 999.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        })
        .into()
}

/// Text in a quiet button: small and dim, in capitals.
pub fn quiet_text<'a>(words: &str) -> Element<'a, Message> {
    text(words.to_uppercase()).font(FONT).size(13).into()
}

/// What a disc holds, for the small picture on it in the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Game,
    Film,
    Music,
}

impl Kind {
    /// Its picture: a controller, a screen or a pair of notes, white, from
    /// `assets/kinds`. The controller is Kenney's (CC0); the others are drawn
    /// to match it. Decoded the first time each is shown.
    pub fn icon(self) -> image::Handle {
        static DECODED: [OnceLock<image::Handle>; 3] =
            [OnceLock::new(), OnceLock::new(), OnceLock::new()];
        let (slot, bytes): (usize, &[u8]) = match self {
            Self::Game => (0, include_bytes!("../assets/kinds/game.png")),
            Self::Film => (1, include_bytes!("../assets/kinds/film.png")),
            Self::Music => (2, include_bytes!("../assets/kinds/music.png")),
        };
        DECODED[slot]
            .get_or_init(|| {
                let pixels = ::image::load_from_memory(bytes)
                    .expect("built-in icon decodes")
                    .to_rgba8();
                image::Handle::from_rgba(pixels.width(), pixels.height(), pixels.into_raw())
            })
            .clone()
    }
}

/// The library's light: a soft blue glow from the top of the screen, made
/// once as a small image the GPU stretches, like the stage's backdrop.
pub fn glow() -> image::Handle {
    static GLOW: OnceLock<image::Handle> = OnceLock::new();
    GLOW.get_or_init(|| {
        const W: u32 = 64;
        const H: u32 = 40;
        let base = [6.0f32, 6.0, 10.0];
        let tint = [27.0f32, 34.0, 64.0];
        let mut pixels = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H {
            for x in 0..W {
                // An ellipse 90% wide and 60% tall, centred at the top edge.
                let dx = (x as f32 + 0.5) / W as f32 - 0.5;
                let dy = (y as f32 + 0.5) / H as f32;
                let r = ((dx / 0.9).powi(2) + (dy / 0.6).powi(2)).sqrt();
                let t = (1.0 - r / 0.7).clamp(0.0, 1.0) * (0xaa as f32 / 255.0);
                for c in 0..3 {
                    pixels.push((base[c] + (tint[c] - base[c]) * t) as u8);
                }
                pixels.push(255);
            }
        }
        image::Handle::from_rgba(W, H, pixels)
    })
    .clone()
}

/// A rounded rectangle's edge in dashes, behind whatever is stacked on it.
pub struct Dashed {
    pub color: Color,
    pub radius: f32,
}

impl<Theme, Renderer> Widget<Message, Theme, Renderer> for Dashed
where
    Renderer: renderer::Renderer,
{
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        const DASH: f32 = 6.0;
        const GAP: f32 = 4.0;
        const WIDTH: f32 = 1.0;
        let b = layout.bounds();
        let r = self.radius.min(b.width / 2.0).min(b.height / 2.0);
        let mut dash = |x: f32, y: f32, w: f32, h: f32| {
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x,
                        y,
                        width: w,
                        height: h,
                    },
                    ..Quad::default()
                },
                self.color,
            );
        };
        // The straight runs, dashed; the corners are left to the eye.
        let mut x = b.x + r;
        while x < b.x + b.width - r {
            let w = DASH.min(b.x + b.width - r - x);
            dash(x, b.y, w, WIDTH);
            dash(x, b.y + b.height - WIDTH, w, WIDTH);
            x += DASH + GAP;
        }
        let mut y = b.y + r;
        while y < b.y + b.height - r {
            let h = DASH.min(b.y + b.height - r - y);
            dash(b.x, y, WIDTH, h);
            dash(b.x + b.width - WIDTH, y, WIDTH, h);
            y += DASH + GAP;
        }
    }
}

impl<'a> From<Dashed> for Element<'a, Message> {
    fn from(dashed: Dashed) -> Self {
        Element::new(dashed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_has_its_icon_for_every_style() {
        let styles = [
            Style::Keys,
            Style::Pad(Pad::Ps5),
            Style::Pad(Pad::Ps4),
            Style::Pad(Pad::Ps3),
            Style::Pad(Pad::Xbox),
            Style::Pad(Pad::Switch),
            Style::Pad(Pad::SteamDeck),
            Style::Pad(Pad::SteamController),
        ];
        let glyphs = [
            "accept", "back", "alt", "select", "start", "prev", "next", "up", "down", "library",
        ];
        for style in styles {
            for name in glyphs {
                let glyph = Glyph::parse(name).unwrap();
                let file = glyph.icon(style);
                assert!(
                    ICONS.iter().any(|(n, _)| *n == file),
                    "{file} for {name} on {style:?} is not built in"
                );
            }
        }
    }

    #[test]
    fn pads_are_told_apart_by_their_ids() {
        assert_eq!(Pad::identify(Some(0x054c), Some(0x0ce6), ""), Pad::Ps5);
        assert_eq!(Pad::identify(Some(0x054c), Some(0x09cc), ""), Pad::Ps4);
        assert_eq!(Pad::identify(Some(0x045e), Some(0x0b13), ""), Pad::Xbox);
        assert_eq!(
            Pad::identify(Some(0x28de), Some(0x1205), ""),
            Pad::SteamDeck
        );
        assert_eq!(
            Pad::identify(None, None, "DualSense Wireless Controller"),
            Pad::Ps5
        );
        assert_eq!(Pad::identify(Some(0x2dc8), None, "8BitDo"), Pad::Xbox);
    }
}
