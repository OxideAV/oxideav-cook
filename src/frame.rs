//! Backend per-frame body orchestrator (frame-syntax §0–§5).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §0 (the frame
//! driver + bit reader), §1 (gain envelope), §2 (category/quant walk),
//! §3 (spectral VLC), §4 (joint-stereo coupling) and §5 (output stage),
//! plus `docs/audio/cook/provenance/05-cook-backend.md`.
//!
//! ## What this module does
//!
//! `spec/05` §0 pins the backend per-frame body as four sub-stages run
//! in a fixed order over one sub-packet's bitstream:
//!
//! > *"**gain control → category/quant walk → spectral VLC dequant →
//! > inverse transform**; … the stereo body additionally runs the
//! > coupling split."*
//!
//! This module assembles the **statically-pinned** prefix of that walk
//! into a single orchestrated entry point that mirrors the binary's
//! frame body, driving each stage off the modules already in this crate:
//!
//! 1. **Gain envelope (§1).** Reads the leading 6-bit segment-count
//!    field ([`crate::gain::read_segment_count`]) from the frame bit
//!    reader. The per-segment record reads (position + gain index) then
//!    descend the VLC walk — a §3.2 BSS GAP (see below).
//! 2. **Subband geometry (§2.1).** Builds the band → coefficient-range
//!    map ([`crate::subband::SubbandGeometry`]) the dequant walk and the
//!    coupling split both consume.
//! 3. **Spectral VLC dequant (§3).** Each coded band reads vector VLC
//!    symbols from one of the seven codebooks; the codebook *contents*
//!    (per-symbol code/length bytes) are built in the decoder's `.data`
//!    BSS at init and are **not** in the file image (`spec/05` §3.2).
//!    This is the **hard blocker** — the walk stops here precisely,
//!    surfacing [`crate::Error::SpectralCodebookBytesUnavailable`]
//!    (tracked as docs-gap #1775).
//!
//! The orchestrator is therefore a *driver to the documented blocker*:
//! it performs every stage the trace pins statically and reports exactly
//! the sub-step where the BSS-built tables are required, rather than
//! guessing the codebook bytes.
//!
//! ## What stays a GAP (not wired)
//!
//! - The per-symbol spectral codebook **code/length bytes** (§3.2) and
//!   the per-coupling-width rotation **coefficient values** (§4.3) are
//!   runtime-built in BSS — the walk reaches the §3 point that needs
//!   them and stops (docs-gap #1775).
//! - The §2.2 category-*assignment* bit-allocation loop (keyed off the
//!   `0x8f38` per-category expected-cost LUT, not among the extracted
//!   tables) is a separate recorded DOCS-GAP.
//! - The iMDCT kernel (§5, the `0xa1b0` rotation table) is a recorded
//!   spec/01 §6 GAP.
//!
//! ## Wall-respect note
//!
//! Every behavioural fact here is anchored to `spec/05` §0–§4; the stage
//! order and the band → line geometry are the trace's own. No codebook /
//! coupling-coefficient bytes are guessed — the walk stops at the
//! documented BSS blocker.

use crate::{bitreader::FrameBitReader, gain::read_segment_count, subband::SubbandGeometry, Error};

/// The decode progress one frame-body walk reached before the documented
/// BSS blocker.
///
/// Returned by [`decode_frame_body`] on the real-decode path: it carries
/// the statically-pinned state assembled up to the §3 spectral-VLC
/// blocker so a consumer (or a future Validator round that dumps the BSS
/// codebooks) can resume exactly where the file-image facts run out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameWalk {
    /// Gain-envelope segment count read at the top of the frame (§1.1).
    /// The per-segment records themselves are VLC-gated (§3.2 GAP), so
    /// only the count is recovered before the blocker.
    pub gain_segment_count: u32,
    /// Per-stream subband geometry (§2.1) — the band → coefficient-range
    /// map the dequant walk and coupling split consume.
    pub subband_geometry: SubbandGeometry,
    /// Total coded spectral lines across all subbands
    /// (`subband_geometry.total_coded_lines()`), the number of
    /// coefficients the §3 dequant walk would fill.
    pub total_coded_lines: u32,
    /// Bits consumed from the frame bitstream up to the blocker.
    pub bits_consumed: u32,
}

/// Walk the backend per-frame body as far as the statically-pinned
/// frame-syntax allows, stopping at the documented §3.2 BSS blocker.
///
/// `frame` is one sub-packet's bitstream (the §0 frame body); `channels`
/// and `subband_count` come from the wired [`crate::DecodeConfig`].
///
/// The walk:
///
/// 1. Reads the §1.1 gain-envelope segment count from `frame`.
/// 2. Builds the §2.1 subband geometry for `subband_count`.
/// 3. Reaches the §3 spectral-VLC dequant step and stops:
///    [`Error::SpectralCodebookBytesUnavailable`] — the seven codebooks'
///    per-symbol code/length bytes are built in `.data` BSS at init and
///    are not present in the file image (`spec/05` §3.2, docs-gap
///    #1775). The walk does **not** guess them.
///
/// This is the faithful "drive to the documented blocker" entry point:
/// every pinned stage runs, and the precise sub-step needing the
/// dynamic BSS dump is surfaced as a typed error rather than fabricated.
///
/// # Errors
///
/// - [`Error::GainSegmentCountUnderflow`] if the §1.1 segment-count
///   field biases negative.
/// - [`Error::CookieZeroSubbandCount`] if `subband_count == 0`.
/// - [`Error::BitAllocAxisOutOfRange`] if `subband_count` exceeds the
///   51-entry subband LUT.
/// - [`Error::SpectralCodebookBytesUnavailable`] — the documented §3.2
///   BSS blocker (docs-gap #1775); always returned for a non-trivial
///   stream, after the pinned prefix has run.
pub fn decode_frame_body(
    frame: &[u8],
    channels: u16,
    subband_count: u32,
) -> Result<FrameWalk, Error> {
    // Run the statically-pinned prefix (§1.1 gain count + §2.1 subband
    // geometry); any stage-1/2 error surfaces here. If the prefix
    // succeeds the walk has reached the §3 spectral-VLC dequant step,
    // whose codebook code/length bytes are runtime-built in BSS
    // (spec/05 §3.2) and not in the file image — stop precisely here
    // (docs-gap #1775) rather than guess the codebook contents.
    let _prefix = frame_body_prefix(frame, channels, subband_count)?;
    Err(Error::SpectralCodebookBytesUnavailable)
}

/// Walk the pinned prefix and return the assembled [`FrameWalk`] state up
/// to (but not raising) the §3.2 BSS blocker.
///
/// Identical to [`decode_frame_body`] except it returns the
/// [`FrameWalk`] instead of the [`Error::SpectralCodebookBytesUnavailable`]
/// terminator, so callers (and tests) can inspect the statically-pinned
/// state the walk recovered before the documented blocker.
///
/// # Errors
///
/// Same as [`decode_frame_body`] minus the terminal
/// [`Error::SpectralCodebookBytesUnavailable`].
pub fn frame_body_prefix(
    frame: &[u8],
    channels: u16,
    subband_count: u32,
) -> Result<FrameWalk, Error> {
    let _ = channels;
    let mut reader = FrameBitReader::new(frame);
    let gain_segment_count = read_segment_count(&mut reader)?;
    let subband_geometry = SubbandGeometry::new(subband_count)?;
    let total_coded_lines = subband_geometry.total_coded_lines();
    Ok(FrameWalk {
        gain_segment_count,
        subband_geometry,
        total_coded_lines,
        bits_consumed: reader.bit_cursor(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A frame whose leading 6-bit field is `001000` = 8 → 8 − 6 = 2
    // gain segments (a well-formed, non-flat envelope head).
    fn frame_two_segments() -> [u8; 8] {
        // top 6 bits = 001000 = 8.
        [0b0010_0000, 0, 0, 0, 0, 0, 0, 0]
    }

    #[test]
    fn prefix_recovers_gain_count_and_geometry() {
        let frame = frame_two_segments();
        let walk = frame_body_prefix(&frame, 2, 20).unwrap();
        assert_eq!(walk.gain_segment_count, 2);
        assert_eq!(walk.subband_geometry.subband_count(), 20);
        assert_eq!(
            walk.total_coded_lines,
            walk.subband_geometry.total_coded_lines()
        );
        // The gain segment-count read consumed exactly 6 bits.
        assert_eq!(walk.bits_consumed, 6);
    }

    #[test]
    fn decode_stops_at_documented_bss_blocker() {
        // The full walk runs the pinned prefix then surfaces the §3.2
        // BSS blocker (docs-gap #1775) — not NotImplemented, not a guess.
        let frame = frame_two_segments();
        assert_eq!(
            decode_frame_body(&frame, 2, 20).unwrap_err(),
            Error::SpectralCodebookBytesUnavailable
        );
    }

    #[test]
    fn decode_surfaces_gain_underflow_before_blocker() {
        // A raw segment-count field < 6 biases negative — that §1.1 error
        // fires before the §3 blocker is reached.
        let frame = [0u8; 8]; // top 6 bits = 0 → bias -6 → underflow.
        assert!(matches!(
            decode_frame_body(&frame, 2, 20),
            Err(Error::GainSegmentCountUnderflow { raw: 0 })
        ));
    }

    #[test]
    fn decode_rejects_zero_subband_count() {
        let frame = frame_two_segments();
        assert_eq!(
            decode_frame_body(&frame, 2, 0).unwrap_err(),
            Error::CookieZeroSubbandCount
        );
    }

    #[test]
    fn prefix_band_geometry_tiles() {
        // The recovered geometry tiles its coefficient range gap-free —
        // the band → line map the §3 dequant walk would consume.
        let frame = frame_two_segments();
        let walk = frame_body_prefix(&frame, 1, 12).unwrap();
        let geom = &walk.subband_geometry;
        let mut expected = geom.start_line(0).unwrap();
        for band in 0..geom.subband_count() {
            let r = geom.line_range(band).unwrap();
            assert_eq!(r.start, expected);
            expected = r.end;
        }
    }

    #[test]
    fn mono_and_stereo_reach_same_blocker() {
        // The §4 coupling split is past the §3 blocker, so mono and
        // stereo both stop at the same documented sub-step.
        let frame = frame_two_segments();
        assert_eq!(
            decode_frame_body(&frame, 1, 20).unwrap_err(),
            Error::SpectralCodebookBytesUnavailable
        );
        assert_eq!(
            decode_frame_body(&frame, 2, 20).unwrap_err(),
            Error::SpectralCodebookBytesUnavailable
        );
    }
}
