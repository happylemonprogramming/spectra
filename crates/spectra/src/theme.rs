//! The look: a desktop from around the turn of the century. A pixel sky
//! over a green hill, windows with blue title bars on pale grey, chunky
//! buttons with hard shadows, a grey taskbar, and everything lettered in
//! Silkscreen, a pixel face.
//!
//! By day or by night: night turns the sky dark and starry and the windows
//! dark grey with pale ink; the blue stays blue. The choice is kept with
//! the other settings (see `settings.rs`).
//!
//! All of it is made here or built in - the font is Silkscreen (SIL Open
//! Font License, 62 KB for both weights), the sky is drawn in code - so it
//! looks the same with no network and nothing else installed.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::alignment::Vertical;
use iced::widget::{button, container, image, row, text};
use iced::{Background, Border, Color, Element, Fill, Shadow, Vector};

use crate::{BOLD, FONT, Message};

/// The two weights of Silkscreen, for iced to load at start.
pub const FONTS: [&[u8]; 2] = [
    include_bytes!("../assets/fonts/Silkscreen-Regular.ttf"),
    include_bytes!("../assets/fonts/Silkscreen-Bold.ttf"),
];

/// Whether it is night. Read wherever something is drawn; set by the one
/// switch, so every window agrees.
static NIGHT: AtomicBool = AtomicBool::new(false);

pub fn night() -> bool {
    NIGHT.load(Ordering::Relaxed)
}

/// Day to night, or back, and remembered.
pub fn toggle_night() {
    set_night(!night());
    crate::settings::save();
}

/// As the settings say, at start.
pub fn set_night(night: bool) {
    NIGHT.store(night, Ordering::Relaxed);
}

fn rgb8(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

/// Pale grey paper by day, dark grey by night.
pub fn paper() -> Color {
    if night() {
        rgb8(0x26, 0x29, 0x31)
    } else {
        rgb8(0xf0, 0xf3, 0xf5)
    }
}

/// Near-black ink by day, pale by night.
pub fn ink() -> Color {
    if night() {
        rgb8(0xe4, 0xe7, 0xec)
    } else {
        rgb8(0x1a, 0x1a, 0x1a)
    }
}

/// Pure blue for whatever is chosen, day or night.
pub const BLUE: Color = Color::from_rgb(0.0, 0.0, 1.0);

/// The taskbar's and the buttons' grey.
pub fn silver() -> Color {
    if night() {
        rgb8(0x44, 0x48, 0x52)
    } else {
        Color::from_rgb(0.76, 0.78, 0.8)
    }
}

/// For what went wrong: bright enough to read on either paper.
pub fn red() -> Color {
    if night() {
        rgb8(0xff, 0x70, 0x70)
    } else {
        rgb8(0xc0, 0x10, 0x10)
    }
}

/// Ink, lighter: for what is said second.
pub fn faint(alpha: f32) -> Color {
    Color { a: alpha, ..ink() }
}

/// A hard shadow, offset and unblurred, as pixel art draws one.
pub fn hard_shadow(by: f32) -> Shadow {
    Shadow {
        color: if night() {
            Color::from_rgba(0.0, 0.0, 0.0, 0.5)
        } else {
            Color::from_rgba(0.0, 0.0, 0.25, 0.35)
        },
        offset: Vector::new(by, by),
        blur_radius: 0.0,
    }
}

/// The desktop: a sky of banded blues with blocky clouds, over a green
/// hill - or, by night, a dark sky with stars and a moon over the same
/// hill, asleep. Drawn small, dithered, and shown with its pixels square,
/// so it reads as pixel art at any size. Each made once.
pub fn backdrop() -> image::Handle {
    static DAY: OnceLock<image::Handle> = OnceLock::new();
    static NIGHT_SKY: OnceLock<image::Handle> = OnceLock::new();
    if night() {
        NIGHT_SKY.get_or_init(|| sky(true)).clone()
    } else {
        DAY.get_or_init(|| sky(false)).clone()
    }
}

fn sky(night: bool) -> image::Handle {
    {
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
        // A star here and there, the same ones every time: a small hash of
        // the pixel's place, and only the sparsest of its values.
        let star = |x: u32, y: u32| -> Option<f32> {
            let h = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263);
            let h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
            match h % 211 {
                0 => Some(1.0),
                1 | 2 => Some(0.55),
                _ => None,
            }
        };
        // The moon: high on the right, a little gibbous.
        let moon = |fx: f32, fy: f32| -> Option<bool> {
            let (cx, cy, r) = (262.0, 34.0, 11.0);
            let d = (fx - cx).hypot(fy - cy);
            if d > r {
                return None;
            }
            let shade = (fx - cx + 3.0).hypot(fy - cy - 1.0) > r - 2.0;
            Some(shade)
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
                    if night {
                        mix([38.0, 92.0, 48.0], [10.0, 36.0, 22.0], t)
                    } else {
                        mix([120.0, 205.0, 60.0], [34.0, 120.0, 30.0], t)
                    }
                } else if night && let Some(edge) = moon(fx, fy) {
                    if edge {
                        [214.0, 216.0, 196.0]
                    } else {
                        [246.0, 246.0, 226.0]
                    }
                } else if let Some(shade) = cloud(fx, fy) {
                    let t = banded(shade, x, y, 3.0);
                    if night {
                        mix([72.0, 80.0, 112.0], [40.0, 46.0, 76.0], t)
                    } else {
                        mix([255.0, 255.0, 255.0], [196.0, 214.0, 240.0], t)
                    }
                } else if night
                    && fy < ridge - 6.0
                    && let Some(bright) = star(x, y)
                {
                    mix([120.0, 130.0, 170.0], [236.0, 238.0, 255.0], bright)
                } else {
                    let t = banded(fy / ridge.max(1.0), x, y, 7.0);
                    if night {
                        mix([6.0, 10.0, 32.0], [34.0, 46.0, 98.0], t)
                    } else {
                        mix([22.0, 76.0, 214.0], [140.0, 196.0, 255.0], t)
                    }
                };
                pixels.extend(colour.map(|c| c as u8));
                pixels.push(255);
            }
        }
        image::Handle::from_rgba(W, H, pixels)
    }
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

/// Fills every cell of the grid inside a circle, given its colour there.
fn disc_cells(
    fill: &mut dyn FnMut(i32, i32, i32, i32, [u8; 4]),
    (cx, cy, r): (f32, f32, f32),
    colour: impl Fn(f32, f32) -> Option<[u8; 4]>,
) {
    for y in 0..32 {
        for x in 0..32 {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            if dx.hypot(dy) <= r
                && let Some(c) = colour(dx, dy)
            {
                fill(x, y, x + 1, y + 1, c);
            }
        }
    }
}

/// The sun, for the switch to day: a round yellow face and eight short
/// rays, outlined.
pub fn sun_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        pixel_art(|fill| {
            for (x0, y0, x1, y1) in [
                (15, 1, 17, 6),
                (15, 26, 17, 31),
                (1, 15, 6, 17),
                (26, 15, 31, 17),
            ] {
                fill(x0, y0, x1, y1, [250, 196, 40, 255]);
            }
            for (x, y) in [(5, 5), (24, 5), (5, 24), (24, 24)] {
                fill(x, y, x + 3, y + 3, [250, 196, 40, 255]);
            }
            disc_cells(fill, (16.0, 16.0, 9.5), |_, _| Some(OUTLINE));
            disc_cells(fill, (16.0, 16.0, 8.0), |dx, dy| {
                Some(if dx + dy < -4.0 {
                    [255, 238, 140, 255]
                } else {
                    [255, 214, 60, 255]
                })
            });
        })
        .plain
    })
    .clone()
}

/// The moon, for the switch to night: a navy crescent with a spark of a
/// star beside it, dark enough to read on the light taskbar.
pub fn moon_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        pixel_art(|fill| {
            let bite = |dx: f32, dy: f32| (dx - 7.0).hypot(dy + 6.0) < 9.0;
            disc_cells(fill, (15.0, 17.0, 12.5), |dx, dy| {
                (!bite(dx + 1.5, dy - 1.5)).then_some(OUTLINE)
            });
            disc_cells(fill, (15.0, 17.0, 11.0), |dx, dy| {
                (!bite(dx, dy)).then_some(if dx + dy < 2.0 {
                    [70, 86, 190, 255]
                } else {
                    [44, 56, 150, 255]
                })
            });
            let spark = [255, 214, 60, 255];
            fill(24, 6, 27, 9, spark);
            fill(25, 4, 26, 11, spark);
            fill(22, 7, 29, 8, spark);
        })
        .plain
    })
    .clone()
}

/// Sounds: a pair of notes, beamed.
pub fn notes_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        let violet = [110, 90, 230, 255];
        pixel_art(|fill| {
            fill(11, 6, 27, 10, OUTLINE);
            fill(12, 7, 26, 9, violet);
            fill(11, 6, 13, 24, OUTLINE);
            fill(25, 6, 27, 21, OUTLINE);
            for (cx, cy) in [(9.0, 24.0), (23.0, 21.0)] {
                disc_cells(fill, (cx, cy, 4.5), |_, _| Some(OUTLINE));
                disc_cells(fill, (cx, cy, 3.0), |_, _| Some(violet));
            }
        })
        .plain
    })
    .clone()
}

/// Videos: a television of the time, in a wooden cabinet, rabbit ears on
/// top.
pub fn tv_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        pixel_art(|fill| {
            for i in 0..6 {
                fill(13 - i, 2 + i, 15 - i, 3 + i, OUTLINE);
                fill(17 + i, 2 + i, 19 + i, 3 + i, OUTLINE);
            }
            fill(14, 7, 18, 9, OUTLINE);
            fill(3, 9, 29, 28, OUTLINE);
            fill(4, 10, 28, 27, [196, 120, 60, 255]);
            fill(4, 10, 28, 12, [214, 146, 82, 255]);
            fill(6, 12, 22, 25, OUTLINE);
            fill(7, 13, 21, 24, [90, 170, 220, 255]);
            fill(7, 13, 12, 17, [190, 230, 250, 255]);
            for cy in [15.5, 21.5] {
                disc_cells(fill, (25.0, cy, 1.8), |_, _| Some(OUTLINE));
            }
            fill(5, 27, 9, 30, OUTLINE);
            fill(23, 27, 27, 30, OUTLINE);
        })
        .plain
    })
    .clone()
}

/// Search: a magnifying glass.
pub fn magnifier_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        pixel_art(|fill| {
            for i in 0..9 {
                fill(18 + i, 18 + i, 22 + i, 21 + i, OUTLINE);
            }
            for i in 1..8 {
                fill(19 + i, 20 + i, 21 + i, 21 + i, [150, 100, 60, 255]);
            }
            disc_cells(fill, (13.0, 13.0, 10.5), |_, _| Some(OUTLINE));
            disc_cells(fill, (13.0, 13.0, 8.0), |dx, dy| {
                Some(if dx + dy < -3.0 {
                    [190, 230, 250, 255]
                } else {
                    [120, 190, 235, 255]
                })
            });
        })
        .plain
    })
    .clone()
}

/// Online: the world.
pub fn globe_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        pixel_art(|fill| {
            disc_cells(fill, (16.0, 16.0, 13.5), |_, _| Some(OUTLINE));
            disc_cells(fill, (16.0, 16.0, 12.0), |dx, dy| {
                let land = (dx < -2.0 && -6.0 < dy && dy < 5.0)
                    || (dx > 3.0 && dy < -4.0)
                    || (2.0 < dx && dx < 9.0 && 3.0 < dy && dy < 9.0);
                Some(if land {
                    [80, 180, 90, 255]
                } else {
                    [60, 130, 230, 255]
                })
            });
        })
        .plain
    })
    .clone()
}

/// Quit: the red power button.
pub fn power_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        let red = [220, 70, 50, 255];
        let white = [255, 255, 255, 255];
        pixel_art(|fill| {
            fill(3, 3, 29, 29, OUTLINE);
            fill(4, 4, 28, 28, red);
            fill(4, 4, 28, 7, [240, 110, 80, 255]);
            let gap = |dx: f32, dy: f32| dx.abs() < 3.0 && dy < 0.0;
            disc_cells(fill, (16.0, 17.0, 8.5), |dx, dy| {
                (!gap(dx, dy)).then_some(white)
            });
            disc_cells(fill, (16.0, 17.0, 6.0), |dx, dy| {
                (!gap(dx, dy)).then_some(red)
            });
            fill(15, 7, 17, 16, white);
        })
        .plain
    })
    .clone()
}

/// Clearing the cache: a wastebasket, its lid on.
pub fn bin_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        let (grey, light) = ([150, 160, 176, 255], [196, 204, 216, 255]);
        pixel_art(|fill| {
            fill(12, 3, 20, 7, OUTLINE);
            fill(13, 4, 19, 6, grey);
            fill(4, 6, 28, 11, OUTLINE);
            fill(5, 7, 27, 10, light);
            fill(6, 11, 26, 30, OUTLINE);
            fill(7, 11, 25, 29, grey);
            for x in [10, 15, 20] {
                fill(x, 13, x + 2, 27, light);
            }
        })
        .plain
    })
    .clone()
}

/// The keys: a keyboard.
pub fn keyboard_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        let (cap, edge) = ([250, 250, 252, 255], [150, 156, 170, 255]);
        pixel_art(|fill| {
            fill(2, 9, 30, 25, OUTLINE);
            fill(3, 10, 29, 24, [214, 218, 226, 255]);
            for y in [12, 16] {
                for x in (5..27).step_by(4) {
                    fill(x, y, x + 3, y + 3, cap);
                    fill(x, y + 3, x + 3, y + 4, edge);
                }
            }
            fill(9, 20, 23, 23, cap);
            fill(9, 23, 23, 24, edge);
        })
        .plain
    })
    .clone()
}

/// Games: a controller, round at its grips, a d-pad and two buttons.
pub fn pad_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        let (blue, shade) = ([60, 110, 230, 255], [40, 80, 190, 255]);
        pixel_art(|fill| {
            for cx in [9.0, 23.0] {
                disc_cells(fill, (cx, 21.0, 7.5), |_, _| Some(OUTLINE));
            }
            fill(8, 10, 24, 22, OUTLINE);
            for cx in [9.0, 23.0] {
                disc_cells(fill, (cx, 21.0, 6.0), |_, dy| {
                    Some(if dy < 2.0 { blue } else { shade })
                });
            }
            fill(9, 11, 23, 21, blue);
            fill(6, 19, 12, 21, [240, 240, 250, 255]);
            fill(8, 17, 10, 23, [240, 240, 250, 255]);
            fill(21, 17, 23, 19, [250, 210, 60, 255]);
            fill(24, 20, 26, 22, [240, 80, 80, 255]);
        })
        .plain
    })
    .clone()
}

/// A star, for a favourite: five points, gold, outlined.
pub fn star_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        // Whether a point is inside a five-pointed star of the given size.
        let inside = |dx: f32, dy: f32, outer: f32| {
            let r = dx.hypot(dy);
            let angle = dy.atan2(dx) + std::f32::consts::FRAC_PI_2;
            let step = std::f32::consts::TAU / 10.0;
            let t = (angle.rem_euclid(2.0 * step) / step - 1.0).abs();
            r <= outer * (0.45 + 0.55 * t)
        };
        pixel_art(|fill| {
            for y in 0..32 {
                for x in 0..32 {
                    let (dx, dy) = (x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 17.0);
                    let c = if inside(dx, dy, 12.0) {
                        if dx + dy < -3.0 {
                            [255, 236, 120, 255]
                        } else {
                            [250, 196, 40, 255]
                        }
                    } else if inside(dx, dy, 15.0) {
                        OUTLINE
                    } else {
                        continue;
                    };
                    fill(x, y, x + 1, y + 1, c);
                }
            }
        })
        .plain
    })
    .clone()
}

/// Spectra's own mark, for the start button: a disc, a rainbow catching
/// across its face.
pub fn mark_icon() -> image::Handle {
    static ICON: OnceLock<image::Handle> = OnceLock::new();
    ICON.get_or_init(|| {
        const BANDS: [[u8; 4]; 6] = [
            [236, 64, 64, 255],
            [246, 150, 40, 255],
            [246, 222, 60, 255],
            [70, 196, 90, 255],
            [60, 130, 240, 255],
            [150, 90, 220, 255],
        ];
        pixel_art(|fill| {
            disc_cells(fill, (16.0, 16.0, 14.5), |_, _| Some(OUTLINE));
            disc_cells(fill, (16.0, 16.0, 13.0), |dx, dy| {
                // Silver, with the rainbow in a band across the middle.
                let along = (dx - dy) / 2.0;
                Some(if along.abs() < 7.0 {
                    BANDS[(((along + 7.0) / 14.0) * 6.0).clamp(0.0, 5.0) as usize]
                } else {
                    [214, 220, 230, 255]
                })
            });
            disc_cells(fill, (16.0, 16.0, 5.0), |_, _| Some([232, 236, 242, 255]));
            disc_cells(fill, (16.0, 16.0, 2.5), |_, _| Some(OUTLINE));
        })
        .plain
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

/// The width of a window's frame.
const FRAME: f32 = 2.0;

/// A window on the desktop: a blue title bar with its name and a close
/// box, over a pale grey body, in a dark frame with a hard shadow.
pub fn window<'a>(
    title: String,
    body: Element<'a, Message>,
    close: Option<Message>,
) -> Element<'a, Message> {
    let close: Element<'a, Message> = match close {
        Some(message) => button(text("X").font(BOLD).size(12).color(ink()).center())
            .width(22)
            .height(20)
            .padding(0)
            .on_press(message)
            .style(|_, status| button::Style {
                background: Some(Background::Color(
                    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                        Color::WHITE
                    } else {
                        silver()
                    },
                )),
                text_color: ink(),
                border: Border {
                    color: ink(),
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
    // Inside the frame, so the title bar and the bars along the edges do
    // not paint over it: one unbroken line all the way round.
    .padding(FRAME)
    .width(Fill)
    .height(Fill)
    .style(|_| container::Style {
        background: Some(Background::Color(paper())),
        text_color: Some(ink()),
        border: Border {
            color: ink(),
            width: FRAME,
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
                background: Some(Background::Color(if hot { BLUE } else { paper() })),
                text_color: if hot { Color::WHITE } else { ink() },
                border: Border {
                    color: ink(),
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

/// A button that is only its content until the pointer finds it: for the
/// tray, where a frame round every icon would be clutter.
pub fn ghost<'a>(
    content: impl Into<Element<'a, Message>>,
    on_press: Message,
) -> button::Button<'a, Message> {
    button(content)
        .padding([4, 6])
        .on_press(on_press)
        .style(|_, status| {
            let hot = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: hot.then_some(Background::Color(Color { a: 0.12, ..ink() })),
                text_color: ink(),
                ..button::Style::default()
            }
        })
}

/// Words that come up over something the pointer rests on: a small window
/// of paper, framed in ink.
pub fn tip<'a>(
    content: impl Into<Element<'a, Message>>,
    words: &'static str,
    position: iced::widget::tooltip::Position,
) -> Element<'a, Message> {
    iced::widget::tooltip(
        content,
        container(text(words.to_uppercase()).font(FONT).size(12))
            .padding([4, 8])
            .style(|_| container::Style {
                background: Some(Background::Color(paper())),
                text_color: Some(ink()),
                border: Border {
                    color: ink(),
                    width: 2.0,
                    radius: 0.0.into(),
                },
                shadow: hard_shadow(3.0),
                ..Default::default()
            }),
        position,
    )
    .gap(6)
    .into()
}

/// A scrollbar as the windows draw one: a grey thumb framed in ink, on a
/// faint rail.
pub fn scroll_style(
    look: &iced::Theme,
    status: iced::widget::scrollable::Status,
) -> iced::widget::scrollable::Style {
    let mut style = iced::widget::scrollable::default(look, status);
    style.vertical_rail.background = Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.06)));
    style.vertical_rail.scroller.background = Background::Color(silver());
    style.vertical_rail.scroller.border = Border {
        color: ink(),
        width: 2.0,
        radius: 0.0.into(),
    };
    style
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
            text_color: Some(ink()),
            border: Border {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
}
