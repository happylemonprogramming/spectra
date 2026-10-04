//! Reading and identifying optical discs.
//!
//! A [`Disc`] is anything sectors can be read from: a drive spoken to over
//! SG_IO ([`drive`]), or an image of one ([`image`]). [`identify`] reads as
//! little of it as it can and says what it is - an audio CD, a film, a game
//! and for which console, or a PC disc.
//!
//! Much of this is ported from Rainbow Player (GPL-3.0-or-later), which did
//! the same over WebUSB.

pub mod catalog;
pub mod copy;
pub mod cue;
pub mod disc;
pub mod discid;
pub mod drive;
pub mod dvd;
pub mod dvdcss;
mod error;
pub mod identify;
pub mod image;
pub mod iso9660;
pub mod library;
pub mod soundtrack;
pub mod udf;

#[cfg(test)]
mod testdisc;

pub use disc::{Disc, Media, SECTOR, Toc, Track};
pub use error::{Error, Result, ScsiError};
pub use identify::{DiscKind, GameIdentity, GameSystem, Report, identify};
