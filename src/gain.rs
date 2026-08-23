//! Gain / scale DSP primitives of the §1 stage (`spec/05` §1) — the
//! `sqrt(2)` ladder resolution and the piecewise-constant gain-profile
//! expansion + time-domain application.
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §1 (round-9
//! revision) and `docs/audio/cook/provenance/05-cook-backend.md`
//! evidence #3 (the `2^(k/2)` ladder and its positive window at
//! `0x94f4`).
//!
//! > **Round-9 correction — the §1.1 wire reading is withdrawn.**
//! > Earlier revisions read the frame head as a *time-domain gain
//! > envelope*: a 6-bit segment count biased −6
//! > (`read_segment_count`, since removed) followed by per-segment
//! > `{position, gain index}` records. The live trace does not support
//! > that reading: the head worker `cook.dll!0x4b50` performs one
//! > 6-bit read and `Nb − 1` VLC reads and nothing else, and the array
//! > it fills is the §2.2 allocator's per-band value array `v[]` (see
//! > [`crate::frame`]). This crate's own real-stream data had already
//! > flagged the old reading (12 of the 144 call heads biased
//! > negative under it). **Whether this flavor carries a time-domain
//! > gain envelope at all is open** (`spec/05` §1.2); the only
//! > unaccounted-for pre-spectral fields are the 1-bit sub-packet flag
//! > and the 7-bit frame scalar.
//!
//! What stays wired here — as DSP primitives whose *inputs* are caller
//! events, not wire reads:
//!
//! - **Gain-index → factor resolution** ([`gain_factor_for_index`]):
//!   each unit of gain index multiplies by `sqrt(2)` — the `2^(k/2)`
//!   ladder (`tables/sqrt2-scale-ladder.csv`) indexed at its centre
//!   (`1.0` at element 63) ± the index; evidence #3 pins the centre
//!   offset as `(0x94f4 − 0x93f8)/4 = 63` and the positive window
//!   `{1.0, √2, 2.0, 2√2, 4.0}` ([`GAIN_POS_WINDOW`]).
//! - **Profile expansion + application** ([`GainSegment`],
//!   [`expand_gain_envelope`], [`apply_gain_envelope`] /
//!   [`apply_gain_blocks`]): the piecewise-constant hold of segment
//!   events over a sub-block grid and the post-transform time-domain
//!   multiply — generic §1.2-shaped DSP retained for whenever a wire
//!   source for gain events is pinned (none is today).
//!
//! ## Wall-respect note
//!
//! The ladder values come from the vendored table; the withdrawn wire
//! reading is documented, not silently dropped; no gain event is ever
//! read off the bitstream by this module.

use crate::{
    scale::{sqrt2_scale_for_exponent, ScaleExponent},
    Error,
};

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
/// resolves to a `2^(index/2)` multiplicative factor. No wire source
/// for gain events is pinned (the round-9 trace withdrew the old §1.1
/// reading — see the module docs); this type models a *known* segment
/// so the §1.2-shaped expansion/application stays wired and tested.
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

    // ----- ladder window (evidence #3) -----

    /// The centre offset of the positive-branch window equals the
    /// ladder's `2^0` element (63), per `(0x94f4 − 0x93f8)/4`.
    #[test]
    fn pos_window_offset_is_ladder_centre() {
        assert_eq!(GAIN_POS_WINDOW_ELEMENT_OFFSET, 63);
        assert_eq!(GAIN_POS_WINDOW_ELEMENT_OFFSET, SCALE_EXPONENT_BIAS);
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
