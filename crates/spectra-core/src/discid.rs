//! The MusicBrainz disc ID.
//!
//! It is SHA-1 over a fixed-width 100-slot table rather than over any natural
//! serialisation of the table of contents. Slot 0 holds the lead-out, slot N
//! the offset of track N, and every unused slot is eight ASCII zeros. The
//! digest is then base64'd with `+/=` rewritten to `._-` so it survives in a
//! URL. Reproducing it exactly is what makes a disc identifiable at all, so
//! none of the padding is decorative.
//!
//! Ported from Rainbow Player's `src/lib/cd/discid.ts`, plus the enhanced CD
//! rule from libdiscid.

use base64::Engine;
use serde::Serialize;
use sha1::{Digest, Sha1};

use crate::disc::{PREGAP, Toc};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MusicBrainzId {
    pub disc_id: String,
    /// The `toc=` query form, for MusicBrainz's fuzzy fallback lookup.
    pub toc: String,
}

/// The disc ID of a disc's audio session, or None if it has no audio.
///
/// An enhanced CD's trailing data track is left out and its lead-out moved to
/// where the audio session ended, which is how libdiscid - and so every
/// MusicBrainz submission - computes it.
pub fn musicbrainz(toc: &Toc) -> Option<MusicBrainzId> {
    let session = toc.audio_session()?;
    let (tracks, leadout) = (&session.tracks[..], session.leadout);
    if !tracks.iter().any(|t| !t.data) {
        return None;
    }
    let first = tracks.first()?.number;
    let last = tracks.last()?.number;
    let leadout = leadout + PREGAP;
    let offsets: Vec<u32> = tracks.iter().map(|t| t.lba + PREGAP).collect();

    let mut slots = [0u32; 100];
    slots[0] = leadout;
    for (track, &offset) in tracks.iter().zip(&offsets) {
        slots[usize::from(track.number)] = offset;
    }
    let mut payload = format!("{first:02X}{last:02X}");
    for slot in slots {
        payload.push_str(&format!("{slot:08X}"));
    }
    let digest = Sha1::digest(payload.as_bytes());
    let disc_id = base64::engine::general_purpose::STANDARD
        .encode(digest)
        .replace('+', ".")
        .replace('/', "_")
        .replace('=', "-");

    let toc = [u32::from(first), u32::from(last), leadout]
        .into_iter()
        .chain(offsets)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("+");
    Some(MusicBrainzId { disc_id, toc })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disc::Track;

    fn toc(tracks: &[(u32, bool)], leadout: u32) -> Toc {
        Toc {
            first: 1,
            last: tracks.len() as u8,
            tracks: tracks
                .iter()
                .enumerate()
                .map(|(i, &(lba, data))| Track {
                    number: i as u8 + 1,
                    lba,
                    data,
                })
                .collect(),
            leadout,
        }
    }

    /// Checked against Rainbow Player's TypeScript implementation.
    #[test]
    fn matches_rainbow_player() {
        let id = musicbrainz(&toc(
            &[(0, false), (15_000, false), (32_150, false)],
            51_000,
        ))
        .unwrap();
        assert_eq!(id.toc, "1+3+51150+150+15150+32300");
        assert_eq!(
            id.disc_id,
            include_str!("../tests/discid-expected.txt").trim()
        );
    }

    #[test]
    fn enhanced_cd_leaves_out_its_data_track() {
        let plain = musicbrainz(&toc(&[(0, false), (15_000, false)], 30_000)).unwrap();
        let enhanced =
            musicbrainz(&toc(&[(0, false), (15_000, false), (41_400, true)], 60_000)).unwrap();
        assert_eq!(plain, enhanced);
    }

    #[test]
    fn data_only_disc_has_no_id() {
        assert!(musicbrainz(&toc(&[(0, true)], 300_000)).is_none());
    }
}
