//! Bit-budget category bisection for cook (§5.4 of the trace doc).
//!
//! Categories are not transmitted directly — the decoder reconstructs
//! them by:
//!
//! 1. Computing `bits_left = bits_per_subpacket - bits_consumed` (with
//!    the trace-doc interpolation past the budget marker).
//! 2. Binary-searching for a global `bias` such that the summed
//!    `expbits_tab[clip3((bias - sf[i]) / 2)]` equals `bits_left`.
//!    Iteration: start `bias = -32`, step doubling down 32→16→8→4→2→1
//!    (six iterations).
//! 3. Running a two-cursor expand/contract loop `numvector_size - 1`
//!    times that picks one band and either increments `exp_index1`
//!    (finer category) or decrements `exp_index2` (coarser), keeping
//!    cumulative cost closest to `2 * bits_left`. Final
//!    `category[i] = exp_index2[i]`.
//!
//! `expand_category` may bump categories up by one to stay within
//! `dither_tab` bounds (cap = 8).

use crate::tables::EXPBITS_TAB;

/// Clip3 helper used by the categorise: clamp `(bias - sf) / 2` into
/// `[0, 7]` (the index space of `EXPBITS_TAB`).
fn cost_index(bias: i32, sf: i32) -> usize {
    let raw = (bias - sf) / 2;
    raw.clamp(0, (EXPBITS_TAB.len() - 1) as i32) as usize
}

fn cost_for_bias(bias: i32, sf: &[i32]) -> i32 {
    sf.iter().map(|&s| EXPBITS_TAB[cost_index(bias, s)]).sum()
}

/// Find the global bias whose summed cost is as close as possible to
/// `bits_left` without exceeding it. Returns the chosen bias and the
/// cost achieved.
pub fn find_bias(sf: &[i32], bits_left: i32) -> (i32, i32) {
    let mut bias: i32 = 32;
    let mut step: i32 = 64;
    // Six-iteration binary search: start step at 32, halve. Trace doc
    // says 32→16→8→4→2→1.
    for _ in 0..6 {
        step >>= 1;
        let cost = cost_for_bias(bias, sf);
        if cost >= bits_left {
            bias -= step;
        } else {
            bias += step;
        }
    }
    let final_cost = cost_for_bias(bias, sf);
    (bias, final_cost)
}

/// Compute the per-band category vector `category[0..total_subbands]`
/// in `[0..7]` from the scale factors and bit budget.
///
/// `numvector_size` is `1 << log2_numvector_size` (32, 64, 128 for
/// samples_per_channel = 256, 512, 1024).
///
/// Implementation: after find_bias produces an initial assignment via
/// `cost_index`, walk a two-cursor exchange loop `numvector_size - 1`
/// times, picking the band whose increment/decrement keeps cumulative
/// cost closest to `2 * bits_left`. Final category[i] = exp_index2[i].
pub fn categorise(
    sf: &[i32],
    total_subbands: usize,
    bits_left: i32,
    numvector_size: usize,
) -> Vec<u8> {
    let n = total_subbands;
    debug_assert_eq!(sf.len(), n);

    let (bias, _) = find_bias(sf, bits_left);

    // Initial categories from the bias.
    let mut exp_index1: Vec<i32> = sf.iter().map(|&s| cost_index(bias, s) as i32).collect();
    let mut exp_index2: Vec<i32> = exp_index1.clone();

    // Two-cursor expand/contract.
    let target = 2 * bits_left;
    let mut acc: i32 = exp_index1
        .iter()
        .chain(exp_index2.iter())
        .map(|&i| EXPBITS_TAB[i.clamp(0, 7) as usize])
        .sum();

    // We perform `numvector_size - 1` exchange steps. Each step either
    // increments one exp_index1 (cheaper category → adds bits) or
    // decrements one exp_index2 (more expensive category → removes
    // bits). The choice keeps |acc - target| smallest.
    for _ in 0..numvector_size.saturating_sub(1) {
        // Find best "expand" candidate: pick i with the largest
        // weighting (sf-based) where exp_index1 > 0 (can decrement to
        // a finer category, which costs more bits → moves acc up).
        // Find best "contract" candidate: pick i with smallest
        // weighting where exp_index2 < 7 (can increment to coarser,
        // which removes bits → moves acc down).
        let mut best_i1 = -1i32;
        let mut best_w1 = i32::MIN;
        let mut best_i2 = -1i32;
        let mut best_w2 = i32::MAX;
        for i in 0..n {
            if exp_index1[i] > 0 {
                // Decrementing exp_index1 goes from category c to c-1,
                // increasing cost by EXPBITS[c-1] - EXPBITS[c].
                let w = sf[i] + (8 - exp_index1[i]) * 2;
                if w > best_w1 {
                    best_w1 = w;
                    best_i1 = i as i32;
                }
            }
            if exp_index2[i] < 7 {
                // Incrementing exp_index2 goes c -> c+1, decreasing cost.
                let w = sf[i] + (8 - exp_index2[i]) * 2;
                if w < best_w2 {
                    best_w2 = w;
                    best_i2 = i as i32;
                }
            }
        }
        if best_i1 < 0 && best_i2 < 0 {
            break;
        }
        // Compute deltas if we took each move.
        let d1 = if best_i1 >= 0 {
            let i = best_i1 as usize;
            EXPBITS_TAB[(exp_index1[i] - 1).clamp(0, 7) as usize]
                - EXPBITS_TAB[exp_index1[i].clamp(0, 7) as usize]
        } else {
            i32::MIN
        };
        let d2 = if best_i2 >= 0 {
            let i = best_i2 as usize;
            EXPBITS_TAB[(exp_index2[i] + 1).clamp(0, 7) as usize]
                - EXPBITS_TAB[exp_index2[i].clamp(0, 7) as usize]
        } else {
            i32::MAX
        };
        // Keep |acc - target| smallest.  Use saturating arithmetic since
        // d1/d2 may be i32::MIN/MAX sentinels when the corresponding
        // candidate is unavailable.
        let new_acc1 = acc.saturating_add(d1);
        let new_acc2 = acc.saturating_add(d2);
        let pick_first = new_acc1.saturating_sub(target).saturating_abs()
            <= new_acc2.saturating_sub(target).saturating_abs();
        if pick_first && best_i1 >= 0 {
            exp_index1[best_i1 as usize] -= 1;
            acc = new_acc1;
        } else if best_i2 >= 0 {
            exp_index2[best_i2 as usize] += 1;
            acc = new_acc2;
        } else if best_i1 >= 0 {
            exp_index1[best_i1 as usize] -= 1;
            acc = new_acc1;
        }
    }

    // Final categories = exp_index2[].
    exp_index2.iter().map(|&c| c.clamp(0, 7) as u8).collect()
}

/// `expand_category`: bump categories up by one (toward dither/cat-7)
/// where bit-budget exhaustion permits. Cook caps at 7 (the dither_tab
/// has nine entries — index 8 is a guard). For our purposes this is a
/// no-op; categories from `categorise()` are already in `[0..7]`.
pub fn expand_categories(categories: &mut [u8]) {
    for c in categories.iter_mut() {
        if *c > 7 {
            *c = 7;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_bias_does_not_panic_on_uniform_input() {
        let sf = vec![0i32; 32];
        let (_b, cost) = find_bias(&sf, 100);
        assert!(cost >= 0);
    }

    #[test]
    fn categorise_returns_n_categories_in_range() {
        let sf = vec![10i32, 8, 6, 4, 2, 0, -2, -4, 6, 8];
        let cats = categorise(&sf, sf.len(), 50, 32);
        assert_eq!(cats.len(), sf.len());
        for &c in &cats {
            assert!(c <= 7, "category {c} out of [0..7]");
        }
    }

    #[test]
    fn cost_index_clamps() {
        // bias = 0, sf = -100 → (0 - -100)/2 = 50 → clamps to 7.
        assert_eq!(cost_index(0, -100), 7);
        // bias = -100, sf = 0 → -50 → clamps to 0.
        assert_eq!(cost_index(-100, 0), 0);
    }
}
