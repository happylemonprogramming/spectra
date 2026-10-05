//! The look: a desktop from around the turn of the century. A pixel sky
//! over a green hill, windows with blue title bars on pale grey, chunky
//! buttons with hard shadows, a grey taskbar, and everything lettered in
//! Silkscreen, a pixel face.
//!
//! All of it is made here or built in - the font is Silkscreen (SIL Open
//! Font License, 62 KB for both weights), the sky is drawn in code - so it
//! looks the same with no network and nothing else installed.

use std::sync::OnceLock;

use iced::alignment::Vertical;
use iced::widget::{button, container, image, row, text};
use iced::{Background, Border, Color, Element, Fill, Shadow, Vector};

use crate::{BOLD, FONT, Message};

/// The two weights of Silkscreen, for iced to load at start.
pub const FONTS: [&[u8]; 2] = [
    include_bytes!("../assets/fonts/Silkscreen-Regular.ttf"),
    include_bytes!("../assets/fonts/Silkscreen-Bold.ttf"),
];

/// Pale grey paper, near-black ink, and pure blue for whatever is chosen.
pub const PAPER: Color = Color::from_rgb(
    0xf0 as f32 / 255.0,
    0xf3 as f32 / 255.0,
    0xf5 as f32 / 255.0,
);
pub const INK: Color = Color::from_rgb(
    0x1a as f32 / 255.0,
    0x1a as f32 / 255.0,
    0x1a as f32 / 255.0,
);
pub const BLUE: Color = Color::from_rgb(0.0, 0.0, 1.0);
/// The taskbar's and the buttons' grey.
pub const SILVER: Color = Color::from_rgb(0.76, 0.78, 0.8);

/// Ink, lighter: for what is said second.
pub fn faint(alpha: f32) -> Color {
    Color { a: alpha, ..INK }
}

/// A hard shadow, offset and unblurred, as pixel art draws one.
pub fn hard_shadow(by: f32) -> Shadow {
    Shadow {
        color: Color::from_rgba(0.0, 0.0, 0.25, 0.35),
        offset: Vector::new(by, by),
        blur_radius: 0.0,
    }
}

/// The desktop: a sky of banded blues with blocky clouds, over a green
/// hill. Drawn small, dithered, and shown with its pixels square, so it
/// reads as pixel art at any size. Made once.
pub fn backdrop() -> image::Handle {
    static SKY: OnceLock<image::Handle> = OnceLock::new();
    SKY.get_or_init(|| {
        const W: u32 = 320;
        const H: u32 = 180;
        // A 4 by 4 ordered dither, for bands of colour without banding.
        const BAYER: [[f32; 4]; 4] = [
            [0.0, 8.0, 2.0, 10.0],
            [12.0, 4.0, 14.0, 6.0],
            [3.0, 11.0, 1.0, 9.0],
            [15.0, 7.0, 13.0, 5.0],
        ];
        let mix = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] {
            std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
        };
        // Steps a smooth ramp into a few bands, dithered at their edges.
        let banded = |t: f32, x: u32, y: u32, bands: f32| -> f32 {
            let d = (BAYER[(y % 4) as usize][(x % 4) as usize] + 0.5) / 16.0;
            ((t * bands + d - 0.5).round() / bands).clamp(0.0, 1.0)
        };
        let clouds: [(f32, f32, f32); 6] = [
            (52.0, 30.0, 1.0),
            (150.0, 18.0, 0.8),
            (236.0, 44.0, 1.2),
            (300.0, 22.0, 0.7),
            (98.0, 66.0, 0.6),
            (196.0, 80.0, 0.9),
        ];
        let cloud = |x: f32, y: f32| -> Option<f32> {
            // Each cloud a few overlapping round puffs, flat underneath.
            for &(cx, cy, s) in &clouds {
                let puffs = [
                    (-14.0, 2.0, 8.0),
                    (-4.0, -3.0, 10.0),
                    (8.0, -1.0, 9.0),
                    (17.0, 3.0, 6.0),
                ];
                if y > cy + 6.0 * s {
                    continue;
                }
                for (dx, dy, r) in puffs {
                    let (px, py, r) = (cx + dx * s, cy + dy * s, r * s);
                    let d = ((x - px).powi(2) + ((y - py) * 1.3).powi(2)).sqrt();
                    if d < r {
                        // Shaded a little towards the flat underside.
                        return Some(((y - (cy - 8.0 * s)) / (14.0 * s)).clamp(0.0, 1.0));
                    }
                }
            }
            None
        };
        let mut pixels = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H {
            for x in 0..W {
                let (fx, fy) = (x as f32, y as f32);
                // The hill: a long low swell, higher on the left.
                let ridge = H as f32 * 0.66 - 20.0 * ((fx / W as f32) * 2.6 + 0.4).sin()
                    + 6.0 * ((fx / W as f32) * 7.0).sin();
                let colour = if fy >= ridge {
                    let t = banded(((fy - ridge) / (H as f32 - ridge + 1.0)).sqrt(), x, y, 5.0);
                    mix([120.0, 205.0, 60.0], [34.0, 120.0, 30.0], t)
                } else if let Some(shade) = cloud(fx, fy) {
                    let t = banded(shade, x, y, 3.0);
                    mix([255.0, 255.0, 255.0], [196.0, 214.0, 240.0], t)
                } else {
                    let t = banded(fy / ridge.max(1.0), x, y, 7.0);
                    mix([22.0, 76.0, 214.0], [140.0, 196.0, 255.0], t)
                };
                pixels.extend(colour.map(|c| c as u8));
                pixels.push(255);
            }
        }
        image::Handle::from_rgba(W, H, pixels)
    })
    .clone()
}

/// Pixel art, drawn into a small grid and made four times the size with
/// its pixels kept square, with a hard shadow down and to the right.
fn pixel_art(draw: impl Fn(&mut dyn FnMut(i32, i32, i32, i32, [u8; 4]))) -> crate::art::Icon {
    const SIDE: u32 = 32;
    const SCALE: u32 = 4;
    let mut grid = vec![[0u8; 4]; (SIDE * SIDE) as usize];
    let mut fill = |x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]| {
        for y in y0.max(0)..y1.min(SIDE as i32) {
            for x in x0.max(0)..x1.min(SIDE as i32) {
                grid[(y as u32 * SIDE + x as u32) as usize] = c;
            }
        }
    };
    draw(&mut fill);
    let mut out = ::image::RgbaImage::new(SIDE * SCALE, SIDE * SCALE);
    for (x, y, px) in out.enumerate_pixels_mut() {
        let (gx, gy) = (x / SCALE, y / SCALE);
        let at = |x: u32, y: u32| grid[(y * SIDE + x) as usize];
        let here = at(gx, gy);
        *px = if here[3] > 0 {
            ::image::Rgba(here)
        } else if gx >= 2 && gy >= 2 && at(gx - 2, gy - 2)[3] > 0 {
            ::image::Rgba([0, 0, 64, 90])
        } else {
            ::image::Rgba([0, 0, 0, 0])
        };
    }
    crate::art::Icon::new(&out)
}

const OUTLINE: [u8; 4] = [26, 26, 26, 255];

/// A folder: a manila one, a tab at its back, its front dithered in two
/// yellows.
pub fn folder_icon() -> crate::art::Icon {
    static ICON: OnceLock<crate::art::Icon> = OnceLock::new();
    ICON.get_or_init(|| {
        pixel_art(|fill| {
            fill(3, 6, 14, 10, OUTLINE);
            fill(4, 7, 13, 10, [214, 176, 60, 255]);
            fill(2, 9, 30, 27, OUTLINE);
            fill(3, 10, 29, 26, [222, 186, 64, 255]);
            fill(2, 13, 30, 14, OUTLINE);
            for y in 14..26 {
                for x in 3..29 {
                    let c = if (x + y) % 2 == 0 {
                        [252, 226, 120, 255]
                    } else {
                        [244, 212, 96, 255]
                    };
                    fill(x, y, x + 1, y + 1, c);
                }
            }
        })
    })
    .clone()
}

/// A disc drive: a grey box with its tray, a button and a light.
pub fn drive_icon() -> crate::art::Icon {
    static ICON: OnceLock<crate::art::Icon> = OnceLock::new();
    ICON.get_or_init(|| {
        pixel_art(|fill| {
            fill(2, 9, 30, 24, OUTLINE);
            fill(3, 10, 29, 23, [200, 204, 210, 255]);
            fill(3, 10, 29, 11, [236, 238, 242, 255]);
            fill(5, 14, 27, 16, OUTLINE);
            fill(6, 14, 26, 15, [110, 114, 122, 255]);
            fill(22, 19, 25, 21, OUTLINE);
            fill(6, 19, 8, 21, [40, 190, 70, 255]);
        })
    })
    .clone()
}

/// A window on the desktop: a blue title bar with its name and a close
/// box, over a pale grey body, in a dark frame with a hard shadow.
pub fn window<'a>(
    title: String,
    body: Element<'a, Message>,
    close: Option<Message>,
) -> Element<'a, Message> {
    let close: Element<'a, Message> = match close {
        Some(message) => button(text("X").font(BOLD).size(12).color(INK).center())
            .width(22)
            .height(20)
            .padding(0)
            .on_press(message)
            .style(|_, status| button::Style {
                background: Some(Background::Color(
                    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                        Color::WHITE
                    } else {
                        SILVER
                    },
                )),
                text_color: INK,
                border: Border {
                    color: INK,
                    width: 2.0,
                    radius: 0.0.into(),
                },
                ..button::Style::default()
            })
            .into(),
        None => iced::widget::space().into(),
    };
    let bar = container(
        row![
            container(
                text(title.to_uppercase())
                    .font(BOLD)
                    .size(14)
                    .color(Color::WHITE)
                    .wrapping(text::Wrapping::None)
            )
            .width(Fill)
            .clip(true),
            close,
        ]
        .spacing(8)
        .align_y(Vertical::Center),
    )
    .padding([5, 6])
    .width(Fill)
    .style(|_| container::Style {
        background: Some(Background::Color(BLUE)),
        ..Default::default()
    });
    container(iced::widget::column![
        bar,
        container(body).width(Fill).height(Fill)
    ])
    .width(Fill)
    .height(Fill)
    .style(|_| container::Style {
        background: Some(Background::Color(PAPER)),
        text_color: Some(INK),
        border: Border {
            color: INK,
            width: 2.0,
            radius: 0.0.into(),
        },
        shadow: hard_shadow(6.0),
        ..Default::default()
    })
    .into()
}

/// A chunky button: square, framed in ink, with a hard shadow; blue with
/// white letters while the pointer is on it, or while it stands for what
/// is showing.
pub fn button_with<'a>(
    content: impl Into<Element<'a, Message>>,
    on_press: Option<Message>,
    lit: bool,
) -> button::Button<'a, Message> {
    button(content)
        .padding([6, 10])
        .on_press_maybe(on_press)
        .style(move |_, status| {
            let hot = lit || matches!(status, button::Status::Hovered | button::Status::Pressed);
            let pressed = matches!(status, button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if hot { BLUE } else { PAPER })),
                text_color: if hot { Color::WHITE } else { INK },
                border: Border {
                    color: INK,
                    width: 2.0,
                    radius: 0.0.into(),
                },
                shadow: if pressed {
                    Shadow::default()
                } else {
                    hard_shadow(3.0)
                },
                ..button::Style::default()
            }
        })
}

/// A button lettered in the pixel face.
pub fn button_text<'a>(words: &str, on_press: Option<Message>, lit: bool) -> Element<'a, Message> {
    button_with(
        text(words.to_uppercase())
            .font(FONT)
            .size(13)
            .wrapping(text::Wrapping::None),
        on_press,
        lit,
    )
    .into()
}

/// A field let into a bar, as a status bar's or a tray's: a little darker
/// than what is round it, with a thin dark edge.
pub fn sunken<'a>(content: impl Into<Element<'a, Message>>) -> container::Container<'a, Message> {
    container(content)
        .padding([4, 8])
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.06))),
            text_color: Some(INK),
            border: Border {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
}
