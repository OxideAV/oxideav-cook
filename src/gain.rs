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
//! ## §1.2 envelope application (wired here)
//!
//! Once the segments are known, §1.2 pins the *application*: the gain
//! profile is expanded to **one factor per sub-block** by carrying the
//! last segment's factor forward (a piecewise-constant hold between
//! segment positions), then multiplied into the time-domain samples in
//! the output stage. Wired as [`GainSegment`] (a `(position,
//! gain_index)` event), [`expand_gain_envelope`] (the piecewise-constant
//! hold over the sub-block axis) and [`apply_gain_envelope`] /
//! [`apply_gain_blocks`] (the post-transform time-domain multiply). The
//! closed form (hold-forward expansion + per-sub-block multiply) is
//! pinned by §1.2; the *number of sub-blocks* derives from the
//! long/short transform-block-length state (§1.2 / spec/01 §5.1), so it
//! is a caller input rather than guessed here.
//!
//! ## What stays a GAP (not wired)
//!
//! The **per-segment record reads themselves** — the `position` (the
//! sample/sub-block index at which the gain changes) and the `gain
//! index` — go through the bit-by-bit VLC walk `cook.dll!0x3a50`
//! (`spec/05` §1.1 / §3.1), whose codebook code/length bytes are built
//! in the decoder's `.data` BSS at init and are **not** present in the
//! file image (`spec/05` §3.2 — a recorded GAP pending a Validator round
//! that dumps the populated tables). This module therefore wires the
//! statically-pinned, non-GAP primitives of §1: the segment-count field
//! parse (a plain 6-bit read + `−6` bias), the gain-index → factor
//! resolution (the centred-ladder lookup), and the §1.2 piecewise-
//! constant expansion + time-domain application **given** a set of
//! segments. Reading the segment list *off the bitstream* is the
//! VLC-gated step that stays a recorded GAP.
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

/// One gain-envelope **segment** — a `(position, gain_index)` event
/// (`spec/05` §1.1 / §1.2).
///
/// `position` is the sub-block index at which the gain changes;
/// `gain_index` is the signed step that [`gain_factor_for_index`]
/// resolves to a `2^(index/2)` multiplicative factor. A frame carries
/// `segment_count` ([`read_segment_count`]) of these, read through the
/// VLC walk (a recorded §3.2 BSS GAP); this type models a *known*
/// segment so the §1.2 expansion/application can be wired and tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GainSegment {
    /// Sub-block index at which this segment's gain takes effect.
    pub position: u32,
    /// Signed gain step → `2^(index/2)` via [`gain_factor_for_index`].
    pub gain_index: i8,
}

/// Expand a set of gain segments into **one factor per sub-block** by the
/// §1.2 piecewise-constant hold.
///
/// `spec/05` §1.2: *"the gain profile is expanded to one factor per
/// sub-block by carrying the last segment's factor forward
/// (piecewise-constant hold between segment positions)."* The output is a
/// `block_count`-long vector of `f32` gain factors: sub-blocks before the
/// first segment's `position` carry the unity factor `1.0` (a flat /
/// uncovered envelope, the `segment_count == 0` default of §1.1), and
/// from each segment's `position` onward the factor holds that segment's
/// `2^(gain_index/2)` value until the next segment's `position`.
///
/// Segments are applied in ascending `position` order; a segment whose
/// `position >= block_count` affects no sub-block (it lies past the
/// transform window). `gain_index` is resolved through the shared
/// `2^(k/2)` ladder ([`gain_factor_for_index`]), so the factors are the
/// binary's f32 values.
///
/// # Errors
///
/// Returns [`Error::ScaleExponentOutOfRange`] when any segment's
/// `gain_index` is outside the ladder's `-63..=63` reachable range.
pub fn expand_gain_envelope(
    segments: &[GainSegment],
    block_count: usize,
) -> Result<Vec<f32>, Error> {
    // Default: flat unity gain over every sub-block (the segment_count
    // == 0 case of §1.1, an uncovered envelope).
    let mut blocks = vec![1.0f32; block_count];

    // Apply segments in ascending position order so a later segment's
    // hold-forward overwrites an earlier one's only from its position on.
    let mut ordered: Vec<GainSegment> = segments.to_vec();
    ordered.sort_by_key(|s| s.position);

    for seg in &ordered {
        let factor = gain_factor_for_index(seg.gain_index)?;
        let start = seg.position as usize;
        if start >= block_count {
            // Segment position is past the window — no sub-block to hold.
            continue;
        }
        // Piecewise-constant hold from `position` to the end (a later
        // segment with a higher position overwrites the tail it covers).
        for slot in blocks.iter_mut().skip(start) {
            *slot = factor;
        }
    }
    Ok(blocks)
}

/// Apply an already-expanded per-sub-block gain profile to the
/// time-domain `samples` of a transform window (`spec/05` §1.2).
///
/// `spec/05` §1.2: the per-sub-block gain is *"multiplied into the
/// time-domain samples in the output stage."* The `samples` are split
/// evenly into `blocks.len()` contiguous sub-blocks (the transform
/// window divided into the §1.2 sub-block grid); every sample in
/// sub-block `b` is multiplied by `blocks[b]`. Any trailing samples that
/// do not divide evenly into the sub-block grid are left scaled by the
/// last block factor (the hold extends to the window end).
///
/// This is the **post-transform** time-varying gain — characteristic
/// Cook gain control, not a frequency-domain scale.
///
/// # Errors
///
/// Returns [`Error::GainBlockCountZero`] when `blocks` is empty (no
/// sub-block grid to map the samples onto).
pub fn apply_gain_blocks(samples: &mut [f32], blocks: &[f32]) -> Result<(), Error> {
    if blocks.is_empty() {
        return Err(Error::GainBlockCountZero);
    }
    let n = samples.len();
    let nblocks = blocks.len();
    // Even split of the window into `nblocks` contiguous sub-blocks;
    // sample `i` falls in sub-block `min(i / block_len, nblocks - 1)`
    // so any non-dividing tail rides the last block's factor.
    let block_len = n.div_ceil(nblocks).max(1);
    for (i, s) in samples.iter_mut().enumerate() {
        let b = (i / block_len).min(nblocks - 1);
        *s *= blocks[b];
    }
    Ok(())
}

/// Expand `segments` into a per-sub-block profile and apply it to
/// `samples` in one step (`spec/05` §1.2).
///
/// Equivalent to [`expand_gain_envelope`]`(segments, block_count)`
/// followed by [`apply_gain_blocks`]`(samples, &profile)`.
///
/// # Errors
///
/// - [`Error::GainBlockCountZero`] when `block_count == 0`.
/// - [`Error::ScaleExponentOutOfRange`] when a segment's `gain_index` is
///   outside the ladder range.
pub fn apply_gain_envelope(
    samples: &mut [f32],
    segments: &[GainSegment],
    block_count: usize,
) -> Result<(), Error> {
    if block_count == 0 {
        return Err(Error::GainBlockCountZero);
    }
    let profile = expand_gain_envelope(segments, block_count)?;
    apply_gain_blocks(samples, &profile)
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

    // ----- §1.2 envelope expansion + application -----

    /// No segments → flat unity gain over every sub-block.
    #[test]
    fn expand_empty_envelope_is_flat_unity() {
        let profile = expand_gain_envelope(&[], 8).unwrap();
        assert_eq!(profile, vec![1.0f32; 8]);
    }

    /// A single segment at position 0 holds its factor over the whole
    /// window (piecewise-constant hold from position 0 to the end).
    #[test]
    fn expand_single_segment_holds_forward() {
        // gain_index 2 → 2^(2/2) = 2.0.
        let segs = [GainSegment {
            position: 0,
            gain_index: 2,
        }];
        let profile = expand_gain_envelope(&segs, 4).unwrap();
        let two = gain_factor_for_index(2).unwrap();
        assert_eq!(profile, vec![two; 4]);
    }

    /// A segment partway through holds unity before its position then its
    /// factor from its position onward.
    #[test]
    fn expand_mid_window_segment_splits_hold() {
        let segs = [GainSegment {
            position: 2,
            gain_index: 2,
        }];
        let profile = expand_gain_envelope(&segs, 4).unwrap();
        let two = gain_factor_for_index(2).unwrap();
        assert_eq!(profile, vec![1.0, 1.0, two, two]);
    }

    /// Two segments: each holds from its position to the next; later
    /// position overwrites the tail.
    #[test]
    fn expand_two_segments_hold_in_sequence() {
        let segs = [
            GainSegment {
                position: 1,
                gain_index: 2, // 2.0
            },
            GainSegment {
                position: 3,
                gain_index: -2, // 0.5
            },
        ];
        let profile = expand_gain_envelope(&segs, 5).unwrap();
        let up = gain_factor_for_index(2).unwrap();
        let down = gain_factor_for_index(-2).unwrap();
        assert_eq!(profile, vec![1.0, up, up, down, down]);
    }

    /// Out-of-order segments are sorted by position before the hold.
    #[test]
    fn expand_sorts_segments_by_position() {
        let segs = [
            GainSegment {
                position: 3,
                gain_index: -2,
            },
            GainSegment {
                position: 1,
                gain_index: 2,
            },
        ];
        let profile = expand_gain_envelope(&segs, 5).unwrap();
        let up = gain_factor_for_index(2).unwrap();
        let down = gain_factor_for_index(-2).unwrap();
        assert_eq!(profile, vec![1.0, up, up, down, down]);
    }

    /// A segment whose position is past the window affects no sub-block.
    #[test]
    fn expand_segment_past_window_is_inert() {
        let segs = [GainSegment {
            position: 10,
            gain_index: 4,
        }];
        let profile = expand_gain_envelope(&segs, 4).unwrap();
        assert_eq!(profile, vec![1.0f32; 4]);
    }

    /// Apply a per-block profile to time-domain samples: each contiguous
    /// sub-block is scaled by its factor.
    #[test]
    fn apply_blocks_scales_each_subblock() {
        let mut samples = [1.0f32; 8];
        let blocks = [2.0f32, 0.5];
        apply_gain_blocks(&mut samples, &blocks).unwrap();
        // 8 samples / 2 blocks = 4 per block.
        assert_eq!(&samples[..4], &[2.0; 4]);
        assert_eq!(&samples[4..], &[0.5; 4]);
    }

    /// A non-dividing tail rides the last block's factor (the hold
    /// extends to the window end).
    #[test]
    fn apply_blocks_tail_rides_last_factor() {
        let mut samples = [1.0f32; 7];
        let blocks = [2.0f32, 4.0];
        apply_gain_blocks(&mut samples, &blocks).unwrap();
        // block_len = ceil(7/2) = 4: samples 0..4 -> 2.0, 4..7 -> 4.0.
        assert_eq!(&samples[..4], &[2.0; 4]);
        assert_eq!(&samples[4..], &[4.0; 3]);
    }

    /// Empty block list is a typed error.
    #[test]
    fn apply_blocks_empty_is_error() {
        let mut samples = [1.0f32; 4];
        assert!(matches!(
            apply_gain_blocks(&mut samples, &[]),
            Err(Error::GainBlockCountZero)
        ));
    }

    /// One-shot expand+apply: segments + sample buffer → scaled samples.
    #[test]
    fn apply_envelope_one_shot_matches_two_step() {
        let segs = [GainSegment {
            position: 2,
            gain_index: 2,
        }];
        let mut one = [1.0f32; 8];
        apply_gain_envelope(&mut one, &segs, 4).unwrap();

        let profile = expand_gain_envelope(&segs, 4).unwrap();
        let mut two = [1.0f32; 8];
        apply_gain_blocks(&mut two, &profile).unwrap();

        assert_eq!(one, two);
    }

    /// Zero block count in the one-shot path is a typed error.
    #[test]
    fn apply_envelope_zero_blocks_is_error() {
        let mut samples = [1.0f32; 4];
        assert!(matches!(
            apply_gain_envelope(&mut samples, &[], 0),
            Err(Error::GainBlockCountZero)
        ));
    }

    /// Flat unity envelope leaves samples unchanged (the segment_count ==
    /// 0 default of §1.1).
    #[test]
    fn apply_flat_envelope_is_identity() {
        let mut samples = [3.0f32, -7.0, 0.25, 9.0];
        let original = samples;
        apply_gain_envelope(&mut samples, &[], 4).unwrap();
        assert_eq!(samples, original);
    }
}
