//! Per-sub-packet gain-control envelope — frame-syntax part 1 (`spec/05`
//! §1).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §1 (the
//! gain-envelope bitstream fields and the `sqrt(2)` gain ladder) and
//! `docs/audio/cook/provenance/05-cook-backend.md` evidence #2 and #3:
//!
//! - **#2** — *"Gain envelope read first: `read6 → count`, biased −6";
//!   `0x4b50` calls `0x3f40` with immediate `6` (`push 6`), then
//!   `count + 0xfffffffa` (= −6) stored."*
//! - **#3** — *"Per-segment gain index → `sqrt(2)^index` from `2^(k/2)`
//!   ladder; positive branch `{1,√2,2,2√2,4}` at `0x94f4`"; `0x4b20`
//!   reads `[idx*4 + 0x60bd94f4]`; `(0x94f4−0x93f8)/4 = 63` → centre
//!   `1.0` of the 127-entry sqrt2 ladder (`tables/sqrt2-scale-ladder`)."*
//!
//! ## What the trace pins (wired here)
//!
//! At the top of the backend frame body, **before** the spectral data,
//! the gain-control worker (`cook.dll!0x4b50`) reads the per-sub-packet
//! gain envelope (`spec/05` §1.1):
//!
//! 1. **Envelope segment count.** A fixed 6-bit field is read first
//!    (`read-n-bits` with `n = 6`, evidence #2's `push 6`); a bias of
//!    `−6` is then applied (the field carries `count + 6`; the worker
//!    forms `count − 6`). This is the number of gain segments for the
//!    frame — `0` when the envelope is flat. Wired as
//!    [`read_segment_count`].
//! 2. **Per-segment gain factor.** Each segment's *gain index* (a signed
//!    step) selects a multiplicative factor from the `2^(k/2)` ladder
//!    (`tables/sqrt2-scale-ladder.csv`, RVA `0x93f8`): the applier
//!    (`cook.dll!0x4b20`) indexes that ladder at its **centre**
//!    (`1.0` at element 63) ± the gain index, i.e. each unit of gain
//!    index multiplies by `sqrt(2)`. Evidence #3 pins the centre offset
//!    as `(0x94f4 − 0x93f8)/4 = 63`. Wired as [`gain_factor_for_index`];
//!    the small positive window `{1.0, √2, 2.0, 2√2, 4.0}` at the
//!    sub-pointer RVA `0x94f4` (indices `0..=4`) is [`GAIN_POS_WINDOW`].
//!
//! ## What stays a GAP (not wired)
//!
//! The **per-segment record reads themselves** — the `position` (the
//! sample/sub-block index at which the gain changes) and the `gain
//! index` — go through the bit-by-bit VLC walk `cook.dll!0x3a50`
//! (`spec/05` §1.1 / §3.1), whose codebook code/length bytes are built
//! in the decoder's `.data` BSS at init and are **not** present in the
//! file image (`spec/05` §3.2 — a recorded GAP pending a Validator round
//! that dumps the populated tables). This module therefore wires the two
//! statically-pinned, non-GAP primitives of §1: the segment-count field
//! parse (a plain 6-bit read + `−6` bias) and the gain-index → factor
//! resolution (the centred-ladder lookup). The full per-segment record
//! walk and the §1.2 piecewise-constant interpolation/application over
//! the (long/short) transform sub-blocks are left to a later round that
//! also unblocks the VLC walk.
//!
//! ## Wall-respect note
//!
//! Every fact here is anchored to `spec/05` §1 and provenance evidence
//! #2 / #3 plus the existing [`crate::scale`] ladder accessor
//! ([`crate::scale::sqrt2_scale_for_exponent`], itself sourced from
//! `tables/sqrt2-scale-ladder.{csv,meta}`). No algorithmic content beyond
//! the two pinned primitives is wired; the VLC-gated per-segment reads
//! are flagged as a GAP, not guessed.

use crate::{
    bitreader::FrameBitReader,
    scale::{sqrt2_scale_for_exponent, ScaleExponent},
    Error,
};

/// Bit width of the envelope segment-count field read first by
/// `cook.dll!0x4b50` (`spec/05` §1.1; evidence #2's `push 6`).
pub const SEGMENT_COUNT_FIELD_BITS: u32 = 6;

/// The `−6` bias applied to the raw 6-bit segment-count field
/// (`spec/05` §1.1; evidence #2: the worker forms `count + 0xfffffffa`,
/// i.e. `count − 6`). The wire field carries `segment_count + 6`.
pub const SEGMENT_COUNT_BIAS: i32 = -6;

/// `.rdata` head RVA of the `2^(k/2)` gain/dequant ladder
/// (`tables/sqrt2-scale-ladder.meta` `rva: 0x93f8`), shared with
/// [`crate::scale`]. The gain applier indexes this ladder relative to
/// its centre.
pub const SQRT2_LADDER_RVA: u32 = 0x93f8;

/// `.rdata` RVA of the gain applier's positive-branch sub-pointer
/// (`cook.dll!0x4b20` reads `[idx*4 + 0x94f4]`; evidence #3). It points
/// at the **centre** (`1.0`) of the `0x93f8` ladder:
/// `(0x94f4 − 0x93f8)/4 = 63` (the ladder's `2^0` element).
pub const GAIN_POS_WINDOW_RVA: u32 = 0x94f4;

/// Element offset of [`GAIN_POS_WINDOW_RVA`] inside the `0x93f8` ladder —
/// `(0x94f4 − 0x93f8)/4 = 63`, the `2^0 = 1.0` centre (derived, not
/// retyped). Matches [`crate::scale::SCALE_EXPONENT_BIAS`].
pub const GAIN_POS_WINDOW_ELEMENT_OFFSET: usize =
    ((GAIN_POS_WINDOW_RVA - SQRT2_LADDER_RVA) / 4) as usize;

/// The small positive-branch window `{1.0, √2, 2.0, 2√2, 4.0}` the
/// per-segment applier (`cook.dll!0x4b20`) reads directly at
/// `[0..=4 + 0x94f4]` (`spec/05` §1.1; evidence #3). These are the
/// gain factors for gain indices `0..=4` — i.e. `2^(index/2)` for
/// `index = 0..4` — and are exactly the first five entries reachable
/// through [`gain_factor_for_index`] from `0`. Held as the closed-form
/// `sqrt(2)^index` literals the trace names; the bit-exact values come
/// from the ladder via [`gain_factor_for_index`].
pub const GAIN_POS_WINDOW: [f32; 5] = [
    1.0,
    core::f32::consts::SQRT_2,
    2.0,
    2.0 * core::f32::consts::SQRT_2,
    4.0,
];

/// Read and bias the per-frame gain-envelope **segment count**
/// (`spec/05` §1.1, evidence #2).
///
/// Reads the leading 6-bit field via the frame bit reader and applies
/// the `−6` bias (the wire field carries `segment_count + 6`). The
/// result is the number of gain segments for the frame (`0` = flat
/// envelope).
///
/// # Errors
///
/// Returns [`Error::GainSegmentCountUnderflow`] when the raw 6-bit field
/// is `< 6`, which would bias to a negative count (the wire field is
/// defined as `segment_count + 6`, so a well-formed stream never carries
/// a value below `6`). Carries the offending raw field for diagnostics.
pub fn read_segment_count(reader: &mut FrameBitReader<'_>) -> Result<u32, Error> {
    let raw = reader.read_bits(SEGMENT_COUNT_FIELD_BITS);
    let biased = raw as i32 + SEGMENT_COUNT_BIAS;
    if biased < 0 {
        return Err(Error::GainSegmentCountUnderflow { raw });
    }
    Ok(biased as u32)
}

/// Resolve a per-segment **gain index** to its multiplicative gain
/// factor `2^(index/2)` (`spec/05` §1.1, evidence #3).
///
/// The applier indexes the `2^(k/2)` ladder at its centre
/// (`1.0` at element 63) `± index`, so `index` maps directly to the
/// ladder's exponent `k`: `gain_factor(index) = 2^(index/2)`. The
/// underlying value comes from [`crate::scale::sqrt2_scale_for_exponent`]
/// over `tables/sqrt2-scale-ladder.csv`, so it is the binary's f32 value,
/// not a recomputed power.
///
/// # Errors
///
/// Returns [`Error::ScaleExponentOutOfRange`] (via [`ScaleExponent::new`])
/// when `index` is outside the ladder's `-63..=63` reachable range.
pub fn gain_factor_for_index(index: i8) -> Result<f32, Error> {
    let k = ScaleExponent::new(index)?;
    Ok(sqrt2_scale_for_exponent(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scale::{SCALE_EXPONENT_BIAS, SCALE_EXPONENT_MAX, SCALE_EXPONENT_MIN};

    // ----- segment-count field (§1.1, evidence #2) -----

    /// The centre offset of the positive-branch window equals the
    /// ladder's `2^0` element (63), per `(0x94f4 − 0x93f8)/4`.
    #[test]
    fn pos_window_offset_is_ladder_centre() {
        assert_eq!(GAIN_POS_WINDOW_ELEMENT_OFFSET, 63);
        assert_eq!(GAIN_POS_WINDOW_ELEMENT_OFFSET, SCALE_EXPONENT_BIAS);
    }

    /// `read6 → count` with the `−6` bias: a raw field of `6` yields a
    /// flat envelope (count `0`); raw `63` (max 6-bit) yields `57`.
    #[test]
    fn segment_count_bias_endpoints() {
        // raw = 6 (lowest well-formed) → 0 segments (flat).
        let data = [0b0001_1000u8, 0, 0, 0]; // top 6 bits = 000110 = 6
        let mut r = FrameBitReader::new(&data);
        assert_eq!(read_segment_count(&mut r).unwrap(), 0);

        // raw = 63 (all six bits set) → 57 segments.
        let data = [0b1111_1100u8, 0, 0, 0]; // top 6 bits = 111111 = 63
        let mut r = FrameBitReader::new(&data);
        assert_eq!(read_segment_count(&mut r).unwrap(), 57);
    }

    /// A mid-range raw field biases correctly: raw = 10 → 4 segments.
    #[test]
    fn segment_count_mid_range() {
        // top 6 bits = 001010 = 10 → 10 − 6 = 4.
        let data = [0b0010_1000u8, 0, 0, 0];
        let mut r = FrameBitReader::new(&data);
        assert_eq!(read_segment_count(&mut r).unwrap(), 4);
    }

    /// The segment-count read consumes exactly 6 bits from the frame.
    #[test]
    fn segment_count_consumes_six_bits() {
        let data = [0b0010_1000u8, 0, 0, 0];
        let mut r = FrameBitReader::new(&data);
        let _ = read_segment_count(&mut r).unwrap();
        assert_eq!(r.bit_cursor(), SEGMENT_COUNT_FIELD_BITS);
    }

    /// A raw field `< 6` would bias negative — the typed underflow guard
    /// fires and carries the offending raw value.
    #[test]
    fn segment_count_underflow_below_six() {
        for raw in 0u32..6 {
            // place `raw` in the top 6 bits.
            let top = (raw << 2) as u8;
            let data = [top, 0, 0, 0];
            let mut r = FrameBitReader::new(&data);
            match read_segment_count(&mut r) {
                Err(Error::GainSegmentCountUnderflow { raw: got }) => {
                    assert_eq!(got, raw);
                }
                other => panic!("raw {raw}: expected underflow, got {other:?}"),
            }
        }
    }

    /// An empty frame reads `0` for the field → bias `−6` → underflow
    /// (the field is never absent in a well-formed gain header).
    #[test]
    fn segment_count_empty_frame_underflows() {
        let mut r = FrameBitReader::new(&[]);
        assert!(matches!(
            read_segment_count(&mut r),
            Err(Error::GainSegmentCountUnderflow { raw: 0 })
        ));
    }

    // ----- gain-index → factor (§1.1, evidence #3) -----

    /// Gain index `0` is the ladder centre `1.0` (flat / unity gain).
    #[test]
    fn gain_index_zero_is_unity() {
        assert_eq!(gain_factor_for_index(0).unwrap(), 1.0);
    }

    /// Each positive unit of gain index multiplies by `sqrt(2)`; the
    /// first five entries reproduce the `{1,√2,2,2√2,4}` positive window
    /// the applier reads at `0x94f4` (evidence #3) to f32 tolerance.
    #[test]
    fn gain_index_positive_window_matches() {
        for (idx, &expected) in GAIN_POS_WINDOW.iter().enumerate() {
            let got = gain_factor_for_index(idx as i8).unwrap();
            let rel = ((got - expected) / expected).abs();
            assert!(
                rel < 2e-7,
                "gain index {idx}: got {got}, want {expected} (rel {rel})"
            );
        }
    }

    /// Symmetric ladder: negative indices are the reciprocal half-octave
    /// steps — index `-2` is `0.5`, index `2` is `2.0`.
    #[test]
    fn gain_index_negative_branch() {
        let down2 = gain_factor_for_index(-2).unwrap();
        let up2 = gain_factor_for_index(2).unwrap();
        assert!((down2 - 0.5).abs() < 1e-6, "index -2 = {down2}");
        assert!((up2 - 2.0).abs() < 1e-6, "index +2 = {up2}");
        // 2^(k/2) is multiplicative: f(-2) * f(2) == 1.0.
        assert!((down2 * up2 - 1.0).abs() < 1e-5);
    }

    /// The endpoints of the ladder's reachable range resolve; one past
    /// either end is a typed out-of-range error.
    #[test]
    fn gain_index_range_endpoints() {
        assert!(gain_factor_for_index(SCALE_EXPONENT_MIN).is_ok());
        assert!(gain_factor_for_index(SCALE_EXPONENT_MAX).is_ok());
    }
}
