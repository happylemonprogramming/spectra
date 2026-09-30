//! Everything derived from a cover, computed once when the album arrives.
//!
//! None of this happens per frame. The blurred backdrop in particular is a
//! tiny image blurred on the CPU and stretched by the GPU's texture filter:
//! a full-screen blur shader would look the same and cost fill rate on every
//! frame, which an old laptop would feel.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};

/// Label texture size. The disc never covers more than about a thousand
/// pixels, and mipmaps take care of it when smaller.
const LABEL_SIZE: u32 = 1024;
/// Inside this fraction of the radius a pressed disc is bare, clear hub.
pub const HUB_RATIO: f32 = 0.31;

/// A label texture with its full mip chain, ready to upload.
pub struct Label {
    /// Changes whenever the artwork does, so the GPU copy is replaced once.
    pub id: u64,
    pub levels: Vec<RgbaImage>,
}

impl std::fmt::Debug for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Label({}, {} levels)", self.id, self.levels.len())
    }
}

pub struct Art {
    pub label: Arc<Label>,
    pub backdrop: iced::widget::image::Handle,
    /// The cover, small, for layouts with no room for the disc.
    pub thumbnail: iced::widget::image::Handle,
    /// The cover's characteristic colour, for tinting the room and the UI.
    pub accent: [f32; 3],
}

impl Art {
    pub fn new(cover: &RgbaImage) -> Self {
        let square = square(cover);
        Self {
            label: Arc::new(label(&square)),
            backdrop: backdrop(&square),
            thumbnail: thumbnail(&square),
            accent: accent(&square),
        }
    }
}

/// The largest centred square: covers are nearly square, scans sometimes not.
fn square(cover: &RgbaImage) -> RgbaImage {
    let side = cover.width().min(cover.height());
    let (x, y) = ((cover.width() - side) / 2, (cover.height() - side) / 2);
    imageops::crop_imm(cover, x, y, side, side).to_image()
}

/// The cover printed on a disc: the hub left clear, so the read side's
/// polycarbonate shows through as it does on a real pressing.
fn label(square: &RgbaImage) -> Label {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut base = imageops::resize(square, LABEL_SIZE, LABEL_SIZE, FilterType::CatmullRom);
    let centre = LABEL_SIZE as f32 / 2.0;
    let feather = 2.0 / centre;
    for (x, y, px) in base.enumerate_pixels_mut() {
        let r = ((x as f32 + 0.5 - centre).hypot(y as f32 + 0.5 - centre)) / centre;
        let printed = ((r - HUB_RATIO) / feather).clamp(0.0, 1.0);
        px[3] = (f32::from(px[3]) * printed) as u8;
    }

    let mut levels = vec![base];
    while levels.last().is_some_and(|l| l.width() > 1) {
        let prev = levels.last().expect("non-empty");
        let size = (prev.width() / 2).max(1);
        levels.push(imageops::resize(prev, size, size, FilterType::Triangle));
    }
    Label {
        id: NEXT.fetch_add(1, Ordering::Relaxed),
        levels,
    }
}

/// The cover, tiny and heavily blurred. Stretched to the window it becomes a
/// soft wash of the album's colours.
fn backdrop(square: &RgbaImage) -> iced::widget::image::Handle {
    let small = imageops::resize(square, 48, 48, FilterType::Triangle);
    let blurred = imageops::blur(&small, 5.0);
    iced::widget::image::Handle::from_rgba(blurred.width(), blurred.height(), blurred.into_raw())
}

/// Enough pixels for a thumbnail at twice its largest size.
fn thumbnail(square: &RgbaImage) -> iced::widget::image::Handle {
    let small = imageops::resize(square, 192, 192, FilterType::CatmullRom);
    iced::widget::image::Handle::from_rgba(small.width(), small.height(), small.into_raw())
}

/// The most characterful colour: an average weighted towards saturated,
/// bright pixels, so a mostly black cover with a red logo reads as red.
fn accent(square: &RgbaImage) -> [f32; 3] {
    let small = imageops::resize(square, 24, 24, FilterType::Triangle);
    let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
    for &Rgba([r, g, b, _]) in small.pixels() {
        let c = [r, g, b].map(|v| f32::from(v) / 255.0);
        let max = c[0].max(c[1]).max(c[2]);
        let min = c[0].min(c[1]).min(c[2]);
        let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
        let w = 0.05 + saturation * saturation * max;
        for i in 0..3 {
            sum[i] += c[i] * w;
        }
        weight += w;
    }
    let c = sum.map(|v| v / weight);
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    // A greyscale cover has no hue to borrow; glow in Spectra's own violet.
    if max - min < 0.04 {
        return [0.62, 0.52, 1.0];
    }
    // Enough saturation to read as a colour, and bright enough to work as
    // light rather than just paint.
    let saturation = ((max - min) / max).max(0.45);
    let grey = max * (1.0 - saturation);
    let lift = 0.9 / max;
    c.map(|v| {
        let stretched = grey + (v - min) / (max - min) * (max - grey);
        (stretched * lift).min(1.0)
    })
}
