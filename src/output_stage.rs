//! Inverse-transform output stage — windowing + overlap-add
//! (frame-syntax §5, spec/01 §5.1).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §5 (the output
//! stage pointer) and `docs/audio/cook/spec/01-cook-decoder-structure.md`
//! §5.1 (the decode-chain stage inventory:
//! *"inverse MDCT at the selected block length with windowing/overlap-add
//! … → PCM out"*), plus `docs/audio/cook/provenance/05-cook-backend.md`
//! evidence #14 (*"`0x2bb0` multiplies by `[0x8c0c]`=0.5 / `[0x8c10]`=0.75"*
//! — the L/R overlap-add mix weights) and the runtime-recovered
//! apodisation window `tables/mdct-window-1024.{csv,meta}`.
//!
//! ## What the trace pins (wired here)
//!
//! `spec/05` §5 pins the output stage at stage level: each channel's
//! spectrum is run through the inverse transform, **windowed with the
//! apodisation window the decoder builds at init for the flavour's
//! transform size** (`cook.dll!0x3290` from the `{2.0, 0.25, π, 0.5}`
//! quad; recovered for N = 1024 as `tables/mdct-window-1024`),
//! gain-scaled by the §1 envelope, and overlap-added into the output
//! (the `0x8c0c`/`0x8c10` mix weights). The pieces wired here:
//!
//! 1. **Windowing.** Each block of time-domain samples (the iMDCT
//!    kernel's output) is multiplied point-wise by the window —
//!    [`apply_window`] performs the point-wise multiply against a
//!    caller-supplied window slice (typically
//!    [`crate::mdct::long_full_window_unit`]).
//! 2. **Overlap-add.** Consecutive MDCT blocks overlap by the TDAC
//!    structure; the output is the sum of the windowed tail of the
//!    previous block and the windowed head of the current block.
//!    [`overlap_add`] combines two windowed contributions of equal length.
//! 3. **Mix weights.** The L/R combine path (`cook.dll!0x2bb0`) scales by
//!    the fixed scalars `0.5` ([`OVERLAP_MIX_WEIGHT_HALF`], RVA `0x8c0c`)
//!    and `0.75` ([`OVERLAP_MIX_WEIGHT_THREE_QUARTER`], RVA `0x8c10`)
//!    (evidence #14). [`overlap_add_weighted`] applies a per-contribution
//!    weight pair.
//!
//! > **Round-10 correction.** Earlier revisions of this module windowed
//! > with the five short `.rdata` tables at `0x8d0c` ("the five
//! > Princen-Bradley windows"). That label is withdrawn: those tables
//! > are the §4.3 joint-stereo **pan-coefficient** tables
//! > ([`crate::coupling::coupling_pan_table`]) — their single consumer
//! > in the image is the §4.2 stereo split, and lengths 3/7/15/31/63
//! > are not transform sizes this codec uses. The transform window is
//! > runtime-built and enters here as a slice.
//!
//! ## What stays a GAP (not wired)
//!
//! - The **iMDCT kernel itself** (`cook.dll!0x5b70`, fed the `0xa1b0`
//!   pre/post rotation table) has **no validated closed form** — the
//!   `0xa1b0` table is not a unit-circle twiddle (`a^2 + d^2 != 1`; audit
//!   #16) and is a recorded GAP. This module takes the kernel's
//!   time-domain output as a **caller input** and wires only the
//!   pinned windowing + overlap-add that surrounds it.
//! - The exact **overlap geometry** (how the mix weights map onto the
//!   previous/current contributions per flavor) past the pinned scalars
//!   is not closed-form in the trace; [`overlap_add_weighted`] applies
//!   caller-chosen weights so no per-flavor routing is fabricated.
//!
//! ## Wall-respect note
//!
//! The window values come from the runtime-recovered
//! `tables/mdct-window-1024.csv` (vendored, byte-validated), the mix
//! weights from `provenance/05` evidence #14, and the
//! windowing/overlap-add structure from `spec/05` §5 / spec/01 §5.1. The
//! iMDCT kernel (the unpinned `0xa1b0` rotation) is a caller input,
//! never guessed.

use crate::Error;

/// The `0.5` overlap-add / combine mix weight at RVA `0x8c0c`
/// (`provenance/05` evidence #14: *"`[0x8c0c]`=0.5"*).
pub const OVERLAP_MIX_WEIGHT_HALF: f32 = 0.5;

/// RVA of the [`OVERLAP_MIX_WEIGHT_HALF`] `0.5` scalar (`0x8c0c`).
pub const OVERLAP_MIX_WEIGHT_HALF_RVA: u32 = 0x8c0c;

/// The `0.75` overlap-add / combine mix weight at RVA `0x8c10`
/// (`provenance/05` evidence #14: *"`[0x8c10]`=0.75"*).
pub const OVERLAP_MIX_WEIGHT_THREE_QUARTER: f32 = 0.75;

/// RVA of the [`OVERLAP_MIX_WEIGHT_THREE_QUARTER`] `0.75` scalar
/// (`0x8c10`).
pub const OVERLAP_MIX_WEIGHT_THREE_QUARTER_RVA: u32 = 0x8c10;

/// Multiply a block of time-domain samples point-wise by an apodisation
/// window (`spec/05` §5: the runtime-built window for the flavour's
/// transform size — [`crate::mdct::long_full_window_unit`] for the
/// recovered N = 1024 flavour).
///
/// `samples.len()` must equal `window.len()`.
///
/// # Errors
///
/// Returns [`Error::OutputWindowLengthMismatch`] when `samples.len()`
/// differs from the window length.
pub fn apply_window(samples: &mut [f32], window: &[f32]) -> Result<(), Error> {
    if samples.len() != window.len() {
        return Err(Error::OutputWindowLengthMismatch {
            got: samples.len(),
            window: window.len(),
        });
    }
    for (s, &wk) in samples.iter_mut().zip(window.iter()) {
        *s *= wk;
    }
    Ok(())
}

/// Apply the window to a copy of `samples`, returning the windowed
/// block (the non-mutating form of [`apply_window`]).
///
/// # Errors
///
/// Returns [`Error::OutputWindowLengthMismatch`] when `samples.len()`
/// differs from the window length.
pub fn windowed(samples: &[f32], window: &[f32]) -> Result<Vec<f32>, Error> {
    let mut out = samples.to_vec();
    apply_window(&mut out, window)?;
    Ok(out)
}

/// Overlap-add two equal-length windowed contributions into one output
/// block (`spec/05` §5: *"overlap-added into the output"*).
///
/// Returns `prev_tail[k] + cur_head[k]` for each `k` — the canonical
/// TDAC overlap-add where the windowed tail of the previous block and the
/// windowed head of the current block sum.
///
/// # Errors
///
/// Returns [`Error::OverlapAddLengthMismatch`] when the two slices differ
/// in length.
pub fn overlap_add(prev_tail: &[f32], cur_head: &[f32]) -> Result<Vec<f32>, Error> {
    if prev_tail.len() != cur_head.len() {
        return Err(Error::OverlapAddLengthMismatch {
            a: prev_tail.len(),
            b: cur_head.len(),
        });
    }
    Ok(prev_tail
        .iter()
        .zip(cur_head.iter())
        .map(|(&a, &b)| a + b)
        .collect())
}

/// Weighted overlap-add — `w_prev * prev[k] + w_cur * cur[k]` per sample
/// (`spec/05` §5 / `provenance/05` evidence #14: the combine path scales
/// by the `0x8c0c`/`0x8c10` mix weights).
///
/// The weights are caller-supplied (typically [`OVERLAP_MIX_WEIGHT_HALF`]
/// and/or [`OVERLAP_MIX_WEIGHT_THREE_QUARTER`]) because the exact
/// per-flavor weight→contribution routing past the two pinned scalars is
/// not closed-form in the trace; this wires the arithmetic the binary
/// performs given the weights.
///
/// # Errors
///
/// Returns [`Error::OverlapAddLengthMismatch`] when the two slices differ
/// in length.
pub fn overlap_add_weighted(
    prev: &[f32],
    cur: &[f32],
    w_prev: f32,
    w_cur: f32,
) -> Result<Vec<f32>, Error> {
    if prev.len() != cur.len() {
        return Err(Error::OverlapAddLengthMismatch {
            a: prev.len(),
            b: cur.len(),
        });
    }
    Ok(prev
        .iter()
        .zip(cur.iter())
        .map(|(&a, &b)| w_prev * a + w_cur * b)
        .collect())
}

/// Window then gain-scale one iMDCT-output block — the post-transform
/// per-block sequence of `spec/05` §5 *before* the overlap-add
/// (windowing → §1 gain scaling).
///
/// This composes the two pinned per-block operations in that order:
/// [`apply_window`] then [`crate::gain::apply_gain_blocks`] (the
/// per-sub-block gain profile). The result is the windowed, gain-scaled
/// block ready for the overlap-add with the neighbouring block.
///
/// - `block` is the iMDCT kernel's time-domain output (a caller input —
///   the kernel itself is the `0xa1b0` GAP).
/// - `window` is the apodisation window; `block.len()` must match its
///   length.
/// - `gain_blocks` is the expanded per-sub-block gain profile; a flat
///   profile is passed as `&[1.0]`.
///
/// # Errors
///
/// - [`Error::OutputWindowLengthMismatch`] when `block.len()` differs from
///   the window length.
/// - [`Error::GainBlockCountZero`] when `gain_blocks` is empty.
pub fn window_and_gain(
    block: &[f32],
    window: &[f32],
    gain_blocks: &[f32],
) -> Result<Vec<f32>, Error> {
    let mut out = block.to_vec();
    apply_window(&mut out, window)?;
    crate::gain::apply_gain_blocks(&mut out, gain_blocks)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdct::long_full_window_unit;

    #[test]
    fn mix_weights_match_evidence_14() {
        assert_eq!(OVERLAP_MIX_WEIGHT_HALF, 0.5);
        assert_eq!(OVERLAP_MIX_WEIGHT_HALF_RVA, 0x8c0c);
        assert_eq!(OVERLAP_MIX_WEIGHT_THREE_QUARTER, 0.75);
        assert_eq!(OVERLAP_MIX_WEIGHT_THREE_QUARTER_RVA, 0x8c10);
    }

    #[test]
    fn apply_window_multiplies_by_the_window() {
        // A unit block becomes exactly the window — checked against the
        // runtime-recovered N=1024 window.
        let w = long_full_window_unit();
        let mut samples = vec![1.0f32; w.len()];
        apply_window(&mut samples, w).unwrap();
        assert_eq!(samples, w);
    }

    #[test]
    fn apply_window_scales_each_sample() {
        let w = [0.5f32, 1.0, 2.0, 0.25];
        let mut samples: Vec<f32> = (0..w.len()).map(|i| (i as f32) + 1.0).collect();
        let original = samples.clone();
        apply_window(&mut samples, &w).unwrap();
        for k in 0..w.len() {
            assert!((samples[k] - original[k] * w[k]).abs() < 1e-6, "k {k}");
        }
    }

    #[test]
    fn apply_window_rejects_wrong_length() {
        let w = [1.0f32; 15];
        let mut samples = vec![1.0f32; 14];
        assert_eq!(
            apply_window(&mut samples, &w).unwrap_err(),
            Error::OutputWindowLengthMismatch {
                got: 14,
                window: 15
            }
        );
    }

    #[test]
    fn windowed_is_non_mutating_apply() {
        let w = [0.25f32, 0.5, 0.75];
        let samples = vec![2.0f32, 3.0, 4.0];
        let out = windowed(&samples, &w).unwrap();
        for k in 0..3 {
            assert!((out[k] - samples[k] * w[k]).abs() < 1e-6);
        }
        // Input untouched.
        assert_eq!(samples, vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn overlap_add_sums_contributions() {
        let prev = [1.0f32, 2.0, 3.0];
        let cur = [10.0f32, 20.0, 30.0];
        assert_eq!(overlap_add(&prev, &cur).unwrap(), vec![11.0, 22.0, 33.0]);
    }

    #[test]
    fn overlap_add_rejects_length_mismatch() {
        assert_eq!(
            overlap_add(&[1.0, 2.0], &[1.0]).unwrap_err(),
            Error::OverlapAddLengthMismatch { a: 2, b: 1 }
        );
    }

    #[test]
    fn weighted_overlap_applies_per_contribution_weight() {
        let prev = [4.0f32, 8.0];
        let cur = [2.0f32, 6.0];
        let out = overlap_add_weighted(
            &prev,
            &cur,
            OVERLAP_MIX_WEIGHT_HALF,
            OVERLAP_MIX_WEIGHT_THREE_QUARTER,
        )
        .unwrap();
        assert_eq!(out[0], 0.5 * 4.0 + 0.75 * 2.0);
        assert_eq!(out[1], 0.5 * 8.0 + 0.75 * 6.0);
    }

    #[test]
    fn weighted_overlap_unit_weights_equals_plain() {
        let prev = [1.0f32, 2.0, 3.0];
        let cur = [4.0f32, 5.0, 6.0];
        assert_eq!(
            overlap_add_weighted(&prev, &cur, 1.0, 1.0).unwrap(),
            overlap_add(&prev, &cur).unwrap()
        );
    }

    #[test]
    fn recovered_long_window_hop_tdac_reconstructs_constant() {
        // The unit-normalised recovered window satisfies the hop TDAC
        // identity W[k]^2 + W[k+512]^2 = 1: overlap-adding a unit block
        // windowed twice across the hop restores a flat envelope.
        let w = long_full_window_unit();
        let hop = w.len() / 2;
        for k in 0..hop {
            let id = w[k] * w[k] + w[k + hop] * w[k + hop];
            assert!((id - 1.0).abs() < 1e-3, "hop TDAC k {k} = {id}");
        }
    }

    #[test]
    fn window_and_gain_composes_window_then_gain() {
        // A unit block windowed then scaled by a flat gain of 2.0 equals
        // 2 * window.
        let w = [0.5f32, 0.25, 0.75, 1.0, 0.1, 0.9, 0.3];
        let block = vec![1.0f32; w.len()];
        let out = window_and_gain(&block, &w, &[2.0]).unwrap();
        for k in 0..w.len() {
            assert!((out[k] - 2.0 * w[k]).abs() < 1e-6, "k {k}");
        }
    }

    #[test]
    fn window_and_gain_unit_gain_equals_window() {
        // Flat unity gain leaves the windowed block unchanged.
        let w = [0.5f32, 0.25, 0.75];
        let block = vec![2.0f32, 3.0, 4.0];
        let out = window_and_gain(&block, &w, &[1.0]).unwrap();
        let direct = windowed(&block, &w).unwrap();
        assert_eq!(out, direct);
    }

    #[test]
    fn window_and_gain_rejects_bad_inputs() {
        let w = [1.0f32; 3];
        // Wrong block length.
        assert_eq!(
            window_and_gain(&[1.0, 2.0], &w, &[1.0]).unwrap_err(),
            Error::OutputWindowLengthMismatch { got: 2, window: 3 }
        );
        // Empty gain profile.
        assert_eq!(
            window_and_gain(&[1.0, 2.0, 3.0], &w, &[]).unwrap_err(),
            Error::GainBlockCountZero
        );
    }
}
