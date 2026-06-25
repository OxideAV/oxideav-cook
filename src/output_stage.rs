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
//! — the L/R overlap-add mix weights) and the
//! `docs/audio/cook/tables/mdct-windows.meta` validation note (*"windows
//! of length 3/7/15/31 satisfy the Princen-Bradley TDAC identity
//! `w[k]^2 + w[N-1-k]^2 = 1`"*).
//!
//! ## What the trace pins (wired here)
//!
//! `spec/05` §5 pins the output stage at stage level: *"each channel's
//! spectrum is run through the inverse transform … windowed with one of
//! the five Princen-Bradley windows (`tables/mdct-windows.csv`),
//! gain-scaled by the §1 envelope, and overlap-added into the output (the
//! `0x8c0c`/`0x8c10` mix weights)."* The two pieces that are
//! **statically pinned** (and wired here) are:
//!
//! 1. **Windowing.** Each block of `N` time-domain samples (the iMDCT
//!    kernel's output) is multiplied point-wise by the stored
//!    [`crate::mdct::mdct_half_window`] of the matching length — the
//!    Princen-Bradley window whose `w[k]^2 + w[N-1-k]^2 = 1` TDAC identity
//!    `mdct-windows.meta` validates. [`apply_window`] performs the
//!    point-wise multiply.
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
//! ## What stays a GAP (not wired)
//!
//! - The **iMDCT kernel itself** (`cook.dll!0x5b70`, fed the `0xa1b0`
//!   pre/post rotation table) has **no validated closed form** — the
//!   `0xa1b0` table is not a unit-circle twiddle (`a^2 + d^2 != 1`; audit
//!   #16) and is a recorded GAP. This module takes the kernel's
//!   time-domain output as a **caller input** and wires only the
//!   pinned windowing + overlap-add that surrounds it.
//! - The **long/short block-length switching** (which window a given
//!   frame uses) is a recorded spec/01 §5.1 GAP — the caller supplies the
//!   [`crate::mdct::MdctWindowLength`].
//! - The exact **overlap geometry** (how the mix weights map onto the
//!   previous/current contributions per flavor) past the pinned scalars
//!   is not closed-form in the trace; [`overlap_add_weighted`] applies
//!   caller-chosen weights so no per-flavor routing is fabricated.
//!
//! ## Wall-respect note
//!
//! The window values come from `tables/mdct-windows.csv` (vendored,
//! byte-validated), the mix weights from `provenance/05` evidence #14,
//! and the windowing/overlap-add structure from `spec/05` §5 / spec/01
//! §5.1. The iMDCT kernel (the unpinned `0xa1b0` rotation) is a caller
//! input, never guessed.

use crate::{mdct::mdct_half_window, mdct::MdctWindowLength, Error};

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

/// Multiply a block of time-domain samples point-wise by the stored MDCT
/// window of the matching length (`spec/05` §5: *"windowed with one of
/// the five Princen-Bradley windows"*).
///
/// `samples.len()` must equal the window length; the window is the
/// vendored [`mdct_half_window`] row whose `w[k]^2 + w[N-1-k]^2 = 1` TDAC
/// identity `mdct-windows.meta` validates.
///
/// # Errors
///
/// Returns [`Error::OutputWindowLengthMismatch`] when `samples.len()`
/// differs from the window length.
pub fn apply_window(samples: &mut [f32], window: MdctWindowLength) -> Result<(), Error> {
    let w = mdct_half_window(window);
    if samples.len() != w.len() {
        return Err(Error::OutputWindowLengthMismatch {
            got: samples.len(),
            window: w.len(),
        });
    }
    for (s, &wk) in samples.iter_mut().zip(w.iter()) {
        *s *= wk;
    }
    Ok(())
}

/// Apply the stored MDCT window to a copy of `samples`, returning the
/// windowed block (the non-mutating form of [`apply_window`]).
///
/// # Errors
///
/// Returns [`Error::OutputWindowLengthMismatch`] when `samples.len()`
/// differs from the window length.
pub fn windowed(samples: &[f32], window: MdctWindowLength) -> Result<Vec<f32>, Error> {
    let mut out = samples.to_vec();
    apply_window(&mut out, window)?;
    Ok(out)
}

/// Overlap-add two equal-length windowed contributions into one output
/// block (`spec/05` §5: *"overlap-added into the output"*).
///
/// Returns `prev_tail[k] + cur_head[k]` for each `k` — the canonical
/// TDAC overlap-add where the windowed tail of the previous block and the
/// windowed head of the current block sum (the windows being
/// perfect-reconstruction by the `mdct-windows.meta` TDAC identity).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_weights_match_evidence_14() {
        assert_eq!(OVERLAP_MIX_WEIGHT_HALF, 0.5);
        assert_eq!(OVERLAP_MIX_WEIGHT_HALF_RVA, 0x8c0c);
        assert_eq!(OVERLAP_MIX_WEIGHT_THREE_QUARTER, 0.75);
        assert_eq!(OVERLAP_MIX_WEIGHT_THREE_QUARTER_RVA, 0x8c10);
    }

    #[test]
    fn apply_window_multiplies_by_stored_row() {
        // A length-3 unit block becomes exactly the stored window.
        let win = MdctWindowLength::L3;
        let w = mdct_half_window(win).to_vec();
        let mut samples = vec![1.0f32; w.len()];
        apply_window(&mut samples, win).unwrap();
        assert_eq!(samples, w);
    }

    #[test]
    fn apply_window_scales_each_sample() {
        let win = MdctWindowLength::L7;
        let w = mdct_half_window(win).to_vec();
        let mut samples: Vec<f32> = (0..w.len()).map(|i| (i as f32) + 1.0).collect();
        let original = samples.clone();
        apply_window(&mut samples, win).unwrap();
        for k in 0..w.len() {
            assert!((samples[k] - original[k] * w[k]).abs() < 1e-6, "k {k}");
        }
    }

    #[test]
    fn apply_window_rejects_wrong_length() {
        let win = MdctWindowLength::L15;
        let mut samples = vec![1.0f32; 14]; // window is 15.
        assert_eq!(
            apply_window(&mut samples, win).unwrap_err(),
            Error::OutputWindowLengthMismatch {
                got: 14,
                window: 15
            }
        );
    }

    #[test]
    fn windowed_is_non_mutating_apply() {
        let win = MdctWindowLength::L3;
        let samples = vec![2.0f32, 3.0, 4.0];
        let out = windowed(&samples, win).unwrap();
        let w = mdct_half_window(win);
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
    fn windowed_block_tdac_overlap_reconstructs_constant() {
        // The Princen-Bradley TDAC identity w[k]^2 + w[N-1-k]^2 = 1 means
        // overlap-adding a block windowed twice with its mirror restores
        // a flat envelope: for a unit input, w[k]*w[k] + w[N-1-k]*w[N-1-k]
        // summed across the mirror axis equals 1. Verify the identity the
        // .meta pins holds for the wired window row.
        for win in [
            MdctWindowLength::L3,
            MdctWindowLength::L7,
            MdctWindowLength::L15,
            MdctWindowLength::L31,
        ] {
            let w = mdct_half_window(win);
            let n = w.len();
            for k in 0..n {
                let id = w[k] * w[k] + w[n - 1 - k] * w[n - 1 - k];
                assert!((id - 1.0).abs() < 1e-3, "TDAC {win:?} k {k} = {id}");
            }
        }
    }
}
