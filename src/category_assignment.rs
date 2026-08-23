//! §2.2 category-assignment / bit-allocation pass (`cook.dll!0x4800`).
//!
//! Source-of-truth:
//! `docs/audio/cook/provenance/08-cook-category-assignment.md` (the
//! Stage-1 base pass and the synthetic probes),
//! `docs/audio/cook/provenance/09-cook-frame-read-layout.md` §3 (the
//! live-frame captures, the half-bit cost identity and the Stage-2
//! sweep mechanism; tables `live-frame-params` /
//! `live-frame-allocator-io`) and
//! `docs/audio/cook/tables/category-assignment-params.csv` / `.meta`,
//! with the exact budget-slack constant and the refinement behaviour
//! pinned by **black-box observation** of the reference decoder (the
//! project's opaque validator binary) driving `cook.dll!0x4800` with
//! controlled per-band value arrays and bit budgets and reading the
//! per-band category output back — the same instrumented method the
//! extractor used (`tables/probe_category_assignment.js`). No decoder
//! source was read.
//!
//! ## The routing piece
//!
//! Cook does **not** transmit the per-band spectral category on the
//! wire — it is **computed** by an in-decoder bit-allocation loop from a
//! per-band value array `v[]` (a band priority / envelope index, read
//! through the §2 quant walk) and a running **bit budget**. This module
//! is that loop: it turns `(v[], budget)` into the per-band
//! [`BandCategory`] list that
//! [`crate::frame_decode::decode_spectrum`] then routes through the
//! codebook-by-category §3 band decode. It is the bridge between the
//! quantiser indices and the spectral entropy read.
//!
//! ## Stage 1 — base assignment (`provenance/08` §"Stage 1")
//!
//! For a global **offset** `off`, every band's category is
//!
//! ```text
//! cat[b] = clip( (K + off - v[b]) >> 1 , 0, 7 )
//! ```
//!
//! with `K = 32` ([`BASE_CONSTANT_K`]), an **arithmetic** shift (floor),
//! clipped to `0..=7`. Category `7` is the empty band
//! ([`crate::spectral_decode::EMPTY_BAND_CATEGORY`]): its cost is `0`, so
//! it is the coarsest, free category. `off` is chosen so the total cost
//! `Σ_b cost[cat[b]]` (the `0x8f38` LUT `{52,47,43,37,29,22,16}`, `0` for
//! category 7 — [`crate::bit_alloc::category_bit_cost`]) **best matches**
//! the budget. `provenance/08` records the selection as a six-round
//! bisection (`off` start `-32`, steps `32,16,8,4,2,1`) whose landing the
//! prose summarises as *"nearest; it may marginally exceed `B` when the
//! next step down would undershoot much further"*.
//!
//! Driving `cook.dll!0x4800` across a fine budget sweep pins that landing
//! to a single closed rule keyed on the **documented** `K = 32`: the pass
//! commits one category finer (more bits, higher cost) exactly while
//!
//! ```text
//! current_total_cost + K < budget
//! ```
//!
//! i.e. it refines while at least `K + 1 = 33` budget units sit above the
//! current cost — otherwise it stops (accepting a marginal over-budget at
//! the empty-band cliff, matching the prose). The measured category
//! transitions for a flat input (`Nb=8`, uniform category) land at budget
//! `C_low + 33` for every step (`0→33`, `128→161`, `176→209`, `232→265`,
//! `296→329`, `344→377`, `376→409`) — exactly `C_low + K + 1`. The same
//! rule reproduces every probed **non-flat** vector (ramps, alternating,
//! flat-non-zero) and both `Nb=4` and `Nb=8` — see the unit tests, whose
//! expectations are the validator's own output.
//!
//! ## Stage 2 — per-band ±1 refinement (`provenance/09` §3, `provenance/08` §"Stage 2")
//!
//! After the base pass, a refinement walk **bounded by the decode-state
//! field `+0x28`** (`M`, the [`refinement_bound`](assign_categories)
//! argument; `M = 128` on the validated stream, pinned by replay) spends
//! `M − 1` candidate steps adjusting individual bands one category finer.
//! Round 9 of the docs workspace traced the walk on three real frames and
//! identified its mechanism; [`refine_categories`] is that mechanism:
//!
//! - The walk enumerates **unit offset steps** below the Stage-1 offset.
//!   With `t[b] = K + off − v[b]`, dropping the offset by one changes the
//!   category of exactly the bands with **even** `t` (the parity class),
//!   visited in **ascending band order**; bands whose change the `[0, 7]`
//!   clip absorbs are not candidates. On the traced frame 2 this yields
//!   the documented per-sweep change sets (14 / 16 / 15 / 17 / 13 bands
//!   from offset −3 down to −8, the first two sweeps being exactly the
//!   even- then odd-`t` bands of the base) — pinned by a test against
//!   the staged membership lists.
//! - The `0x8f38` cost LUT is denominated in **half-bits**
//!   ([`REFINEMENT_TARGET_FACTOR`]): the walk fills toward `2 × budget`.
//!   A candidate applies only while `Σcost + Δ <= 2 × budget −`
//!   [`REFINEMENT_CAP_SLACK`]; later candidates are recorded no-ops. The
//!   slack constant is **fitted** to the three live frames (window
//!   `{5, 6}`, `6` used — see the constant's docs); everything else is
//!   the documented mechanism.
//! - Every candidate (applied or not) spends one of the `M − 1` steps, so
//!   a small `M` binds the walk — the `provenance/08` flat probes (`M = 2`
//!   upgrades band 0, `M = 4` upgrades bands 0/1/2, …) are this regime,
//!   and an over-budget base (`B = 60`, uniform cat 6) stays unchanged
//!   because its first candidate already overshoots the half-bit cap.
//!
//! The walk reproduces the vendor decoder's own category output on all
//! three staged live frames (`tables/live-frame-allocator-io.csv`: 34/34
//! bands each; frame 2 stops on a sweep boundary, frames 16/17 stop
//! mid-sweep) **and** every synthetic `provenance/08` expectation; the
//! landing totals satisfy the `provenance/09` §3a identity (Σcost just
//! under `2 × budget`: 1124 / 1130 / 1126 against 1130 / 1138 / 1134).
//! Not reproduced (and not needed for `cat[]`): the vendor's `arg_14`
//! index list, which records the candidate sweeps from the *other* end
//! and keeps enumerating no-ops after the last change until all `M − 1`
//! steps are spent — the returned [`CategoryAssignment::adjusted`] is
//! the applied changes most-recent-first (which equals the validator's
//! index list in the flat M-bound regime).
//!
//! ## Wall-respect note
//!
//! Every constant is either the documented `category-assignment-params`
//! table (`K = 32`, offset start `−32`, steps, clip `0..=7`, `cost[7]=0`,
//! `budget = bit_limit − bit_cursor`, `Nb = 34`, `M = 128`) or a value
//! read from the opaque validator's **own output** for a known input
//! (the synthetic probes of `provenance/08` and the three live frames of
//! `provenance/09`). No decoder source was read; the Stage-1 budget-slack
//! rule is the documented `K` under a strict comparison, the Stage-2
//! mechanism is the documented parity-sweep walk, and the one fitted
//! constant ([`REFINEMENT_CAP_SLACK`]) is recorded with its window.

use crate::{
    bit_alloc::category_bit_cost,
    category::CategoryIndex,
    spectral_decode::{BandCategory, EMPTY_BAND_CATEGORY},
    Error,
};

/// The base additive constant `K = 32` in the per-band category
/// expression `clip((K + off − v[b]) >> 1, 0, 7)`
/// (`category-assignment-params.csv`, `base_constant_K`; `0x20` added in
/// `cook.dll!0x4800`). Also the budget slack: the base pass refines while
/// `total_cost + K < budget`.
pub const BASE_CONSTANT_K: i32 = 32;

/// The initial global bit-allocation offset `−32`
/// (`category-assignment-params.csv`, `offset_start`).
pub const OFFSET_START: i32 = -32;

/// The six halving bisection steps `32,16,8,4,2,1`
/// (`category-assignment-params.csv`, `bisection_step_0..5`).
pub const BISECTION_STEPS: [i32; 6] = [32, 16, 8, 4, 2, 1];

/// The category clip bounds `0..=7`
/// (`category-assignment-params.csv`, `category_clip_lo` / `_hi`).
pub const CATEGORY_CLIP_LO: i32 = 0;
/// See [`CATEGORY_CLIP_LO`].
pub const CATEGORY_CLIP_HI: i32 = EMPTY_BAND_CATEGORY as i32;

/// The per-category bit cost, `0` for the empty band (category 7) and the
/// `0x8f38` LUT value otherwise (`{52,47,43,37,29,22,16}` for `0..=6`).
///
/// `raw_category` must be `0..=7`; any other value is treated as the
/// empty band (cost `0`) — the base formula only ever produces `0..=7`.
#[must_use]
pub fn category_cost(raw_category: u8) -> u32 {
    if raw_category >= EMPTY_BAND_CATEGORY {
        0
    } else {
        // 0..=6 is exactly the CategoryIndex range.
        category_bit_cost(CategoryIndex::new(raw_category).expect("0..=6 is a valid category"))
    }
}

/// The base per-band category `clip((K + off − v) >> 1, 0, 7)` — the
/// `provenance/08` Stage-1 formula, an **arithmetic** (floor) shift.
#[must_use]
pub fn base_category(offset: i32, v: i32) -> u8 {
    let raw = (BASE_CONSTANT_K + offset - v) >> 1;
    raw.clamp(CATEGORY_CLIP_LO, CATEGORY_CLIP_HI) as u8
}

/// The base categories for every band at a fixed `offset`.
fn base_categories_at(offset: i32, v: &[i32]) -> Vec<u8> {
    v.iter().map(|&vb| base_category(offset, vb)).collect()
}

/// The total bit cost of a base assignment at `offset`.
fn total_cost_at(offset: i32, v: &[i32]) -> u32 {
    v.iter()
        .map(|&vb| category_cost(base_category(offset, vb)))
        .sum()
}

/// The coarsest offset at which **every** band is the empty band
/// (category 7) — the base pass's starting point (all bands free).
///
/// Category 7 needs `(K + off − v) >> 1 >= 7`, i.e. `off >= v − 18`; the
/// tightest band is `max(v)`, so `off = max(v) − 18` makes all bands
/// category 7. Returns [`OFFSET_START`]'s neighbourhood for an empty
/// input.
fn coarsest_all_empty_offset(v: &[i32]) -> i32 {
    // (K + off - v) >> 1 >= 7  <=>  K + off - v >= 14  <=>  off >= v - (K - 14) = v - 18.
    let max_v = v.iter().copied().max().unwrap_or(0);
    max_v - (BASE_CONSTANT_K - 14)
}

/// The offset the base bit-allocation pass lands on for `(v, budget)`.
///
/// Starts at the coarsest all-empty offset and steps one category finer
/// (decreasing the offset to the next cost-increasing value) while
/// `total_cost + K < budget` (`provenance/08` Stage 1, the validated
/// budget-slack rule). Stops when refining further is unaffordable or
/// when every band has reached the finest category (0).
#[must_use]
pub fn assign_base_offset(v: &[i32], budget: i32) -> i32 {
    if v.is_empty() {
        return coarsest_all_empty_offset(v);
    }
    let mut offset = coarsest_all_empty_offset(v);
    loop {
        let cost = total_cost_at(offset, v) as i32;
        if cost + BASE_CONSTANT_K >= budget {
            break;
        }
        // Find the next finer offset (lower value) whose total cost
        // strictly increases; if none exists every band is already at
        // the finest category.
        let mut next = offset - 1;
        let mut found = false;
        // The finest reachable offset makes every band category 0, whose
        // cost is bounded; the floor of `v.min() - (K + 2*7)` is a safe
        // stop (below it no category can change).
        let floor = v.iter().copied().min().unwrap_or(0) - (BASE_CONSTANT_K + 16);
        while next >= floor {
            if total_cost_at(next, v) as i32 != cost {
                found = true;
                break;
            }
            next -= 1;
        }
        if !found {
            break;
        }
        offset = next;
    }
    offset
}

/// The base per-band categories (`0..=7`) for `(v, budget)` — Stage 1
/// only, no refinement.
#[must_use]
pub fn assign_base_categories(v: &[i32], budget: i32) -> Vec<u8> {
    base_categories_at(assign_base_offset(v, budget), v)
}

/// A refined category assignment: the per-band categories plus the order
/// of bands the Stage-2 refinement adjusted (`arg_14`, most-recent
/// first — the validator emits the upgraded band indices in descending
/// application order for the uniform-under-budget regime).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryAssignment {
    /// Per-band categories (`0..=7`) after base + applied refinement.
    pub categories: Vec<u8>,
    /// Bands adjusted by the refinement, in the validator's index order.
    pub adjusted: Vec<u32>,
}

impl CategoryAssignment {
    /// The per-band [`BandCategory`] view (empty-band sentinel vs coded).
    ///
    /// # Errors
    ///
    /// [`Error::CategoryOutOfRange`] if any category exceeds `7` — the
    /// base formula never produces one, so this cannot fail for an
    /// assignment this module built.
    pub fn band_categories(&self) -> Result<Vec<BandCategory>, Error> {
        self.categories
            .iter()
            .map(|&c| BandCategory::from_raw(c))
            .collect()
    }
}

/// The factor relating the `0x8f38` cost LUT to the allocator's bit
/// budget: the LUT is denominated in **half-bits**, so the Stage-2
/// refinement fills toward `2 × budget` (`spec/05` §2.2 / `provenance/09`
/// §3a: the three live frames land at Σcost = 1124 / 1130 / 1126
/// against `2 × budget` = 1130 / 1138 / 1134, and `Σcost / 2` matches
/// the bits the spectral stage then consumes to within a few bits).
pub const REFINEMENT_TARGET_FACTOR: i32 = 2;

/// The slack below `2 × budget` the Stage-2 refinement keeps: a
/// candidate change applies only while
/// `Σcost + Δ <= REFINEMENT_TARGET_FACTOR × budget − REFINEMENT_CAP_SLACK`.
///
/// **Fitted against the three staged live frames** (`provenance/09`,
/// `tables/live-frame-allocator-io.csv`): the last applied candidates
/// land at slack 6 / 8 / 8 and the first rejected candidates would land
/// at slack 0 / 2 / 1, while every later (smaller-delta, min 4)
/// candidate must also be rejected — which pins the constant to the
/// window `{5, 6}`; `6` is used. Both values reproduce all 102 live
/// categories; the window is recorded honestly rather than narrowed by
/// guesswork.
pub const REFINEMENT_CAP_SLACK: i32 = 6;

/// Stage-2 per-band ±1 refinement walk (`provenance/09` §3, the
/// front/back sweep; `provenance/08` Stage 2 for the M-bound flat
/// regime) from a Stage-1 base assignment at `base_offset`.
///
/// The walk enumerates **unit offset steps** below the base offset. For
/// the current offset, with `t[b] = K + off − v[b]`, exactly the bands
/// with **even** `t` change category when the offset drops by one
/// (`(t − 1) >> 1 != t >> 1` iff `t` is even); the candidates are
/// visited in **ascending band order** (the adjusting cursor — the
/// vendor records them from the other end, descending), bands whose
/// change is absorbed by the `[0, 7]` clip are not candidates, and every
/// candidate spends one of the `M − 1` refinement steps. A candidate
/// **applies** only while the running total stays under the half-bit
/// target (`Σcost + Δ <= 2 × budget − `[`REFINEMENT_CAP_SLACK`]);
/// otherwise it is a recorded no-op. The walk stops when the step budget
/// is spent or when a whole sweep applied nothing (every later candidate
/// only costs more).
///
/// This reproduces the vendor decoder's own output on all three staged
/// live frames (34/34 bands each, two of them stopping mid-sweep) and on
/// every `provenance/08` synthetic probe (the flat M-bound upgrades, the
/// over-budget uniform base left unchanged).
///
/// Returns the refined categories and the applied band indices
/// most-recent-first.
#[must_use]
pub fn refine_categories(
    v: &[i32],
    budget: i32,
    base_offset: i32,
    refinement_bound: u32,
) -> CategoryAssignment {
    let mut categories = base_categories_at(base_offset, v);
    let mut total: i32 = categories.iter().map(|&c| category_cost(c) as i32).sum();
    let cap = REFINEMENT_TARGET_FACTOR * budget - REFINEMENT_CAP_SLACK;
    let max_steps = refinement_bound.saturating_sub(1);
    let mut steps = 0u32;
    let mut offset = base_offset;
    let mut adjusted_asc: Vec<u32> = Vec::new();
    // Below this offset no band can change any more (every `(t-1)>>1`
    // is negative): a termination guard for degenerate inputs.
    let floor = v.iter().copied().min().unwrap_or(0) - (BASE_CONSTANT_K + 2 * 7 + 2);
    while steps < max_steps && offset >= floor {
        let mut any_candidate = false;
        let mut applied = false;
        for (band, &vb) in v.iter().enumerate() {
            let t = BASE_CONSTANT_K + offset - vb;
            if t.rem_euclid(2) != 0 {
                continue;
            }
            let new = (t - 1) >> 1;
            if !(CATEGORY_CLIP_LO..=CATEGORY_CLIP_HI).contains(&new) {
                continue;
            }
            let current = (t >> 1).clamp(CATEGORY_CLIP_LO, CATEGORY_CLIP_HI);
            if new == current {
                continue;
            }
            any_candidate = true;
            if steps >= max_steps {
                break;
            }
            steps += 1;
            let delta = category_cost(new as u8) as i32 - category_cost(categories[band]) as i32;
            if total + delta <= cap {
                total += delta;
                categories[band] = new as u8;
                adjusted_asc.push(band as u32);
                applied = true;
            }
        }
        offset -= 1;
        if any_candidate && !applied {
            break;
        }
    }
    adjusted_asc.reverse();
    CategoryAssignment {
        categories,
        adjusted: adjusted_asc,
    }
}

/// Assign per-band categories from `(v, budget)` with the Stage-2
/// refinement bounded by `refinement_bound` (`M`, decode-state `+0x28`).
///
/// Runs the base pass ([`assign_base_offset`]) and then the
/// [`refine_categories`] walk from the base offset. `refinement_bound ==
/// 0` or `1` applies no refinement (the base pass is the first `M`).
#[must_use]
pub fn assign_categories(v: &[i32], budget: i32, refinement_bound: u32) -> CategoryAssignment {
    let base_offset = assign_base_offset(v, budget);
    refine_categories(v, budget, base_offset, refinement_bound)
}

/// The total half-bit cost `Σ_b cost[cat[b]]` of an assignment.
#[must_use]
pub fn total_cost(categories: &[u8]) -> u32 {
    categories.iter().map(|&c| category_cost(c)).sum()
}

/// The per-band [`BandCategory`] list from `(v, budget, refinement_bound)`
/// — the value [`crate::frame_decode::decode_spectrum`] consumes.
///
/// # Errors
///
/// [`Error::CategoryOutOfRange`] never fires for an assignment this
/// module builds (the base formula clips to `0..=7`); surfaced for API
/// symmetry with [`BandCategory::from_raw`].
pub fn assign_band_categories(
    v: &[i32],
    budget: i32,
    refinement_bound: u32,
) -> Result<Vec<BandCategory>, Error> {
    assign_categories(v, budget, refinement_bound).band_categories()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every expectation below is the reference validator's own output for
    // the given input (driving cook.dll!0x4800 through the opaque probe),
    // transcribed from provenance/08 and the fine sweeps run this round.

    #[test]
    fn params_match_the_table() {
        assert_eq!(BASE_CONSTANT_K, 32);
        assert_eq!(OFFSET_START, -32);
        assert_eq!(BISECTION_STEPS, [32, 16, 8, 4, 2, 1]);
        assert_eq!(CATEGORY_CLIP_LO, 0);
        assert_eq!(CATEGORY_CLIP_HI, 7);
        assert_eq!(category_cost(7), 0);
        assert_eq!(category_cost(0), 52);
        assert_eq!(category_cost(6), 16);
    }

    #[test]
    fn constants_cross_check_the_vendored_params_table() {
        // Every module constant equals the vendored named-scalar row of
        // `tables/category-assignment-params.csv` — the staged recovery
        // of the `cook.dll!0x4800` algorithm constants. The typed
        // constants exist for const-context use; this pin keeps them
        // bit-locked to the staged table.
        use crate::tables::category_assignment_param as p;
        assert_eq!(i64::from(BASE_CONSTANT_K), p("base_constant_K"));
        assert_eq!(i64::from(OFFSET_START), p("offset_start"));
        for (i, &step) in BISECTION_STEPS.iter().enumerate() {
            assert_eq!(i64::from(step), p(&format!("bisection_step_{i}")));
        }
        // The `>> 1` arithmetic shift in `base_category` is the table's
        // divisor-2 row.
        assert_eq!(p("category_divisor"), 2);
        assert_eq!(i64::from(CATEGORY_CLIP_LO), p("category_clip_lo"));
        assert_eq!(i64::from(CATEGORY_CLIP_HI), p("category_clip_hi"));
        // cost[7] = 0 (the empty band spends no bits).
        assert_eq!(i64::from(category_cost(7)), p("cost_cat7"));
        // The cost LUT the pass reads is the 0x8f38 table this crate
        // vendors as category-cost-lut.csv.
        assert_eq!(p("cost_lut_rva"), 0x8f38);
        // The Stage-2 refinement bound is the decode-state field +0x28.
        assert_eq!(p("refinement_bound_field"), 0x28);
    }

    #[test]
    fn params_table_has_exactly_the_named_rows() {
        let rows = crate::tables::category_assignment_params();
        assert_eq!(rows.len(), crate::tables::CATEGORY_ASSIGNMENT_PARAMS_ROWS);
        let names: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
        for want in [
            "base_constant_K",
            "offset_start",
            "bisection_step_0",
            "bisection_step_5",
            "category_divisor",
            "category_clip_lo",
            "category_clip_hi",
            "cost_lut_rva",
            "cost_cat7",
            "refinement_bound_field",
        ] {
            assert!(names.contains(&want), "missing param row {want:?}");
        }
    }

    #[test]
    fn base_formula_reproduces_the_pinned_vector() {
        // provenance/08: v=[0,4,8,12,2,6,10,14], budget 250 -> offset -17
        // -> cats [7,5,3,1,6,4,2,0].
        let v = [0, 4, 8, 12, 2, 6, 10, 14];
        let off = assign_base_offset(&v, 250);
        let cats = base_categories_at(off, &v);
        assert_eq!(cats, [7, 5, 3, 1, 6, 4, 2, 0]);
        // The formula at the back-solved offset -17 reproduces it too.
        assert_eq!(
            base_categories_at(-17, &v),
            [7, 5, 3, 1, 6, 4, 2, 0],
            "clip((15 - v)>>1, 0, 7)"
        );
    }

    #[test]
    fn flat_budget_sweep_matches_validator() {
        // provenance/08 budget sweep, Nb=8, v=0.
        let v = [0i32; 8];
        for (b, want) in [
            (0i32, 7u8),
            (20, 7),
            (50, 6),
            (176, 5),
            (232, 4),
            (300, 3),
            (400, 1),
            (500, 0),
        ] {
            let cats = assign_base_categories(&v, b);
            assert!(
                cats.iter().all(|&c| c == want),
                "budget {b}: got {cats:?}, want uniform {want}"
            );
        }
    }

    #[test]
    fn flat_category_transitions_land_at_c_low_plus_k_plus_one() {
        // The validator's fine sweep: uniform category drops at budgets
        // {33,161,209,265,329,377,409} for Nb=8 — each is C_low + K + 1.
        let v = [0i32; 8];
        let transitions = [
            (33, 6u8),
            (161, 5),
            (209, 4),
            (265, 3),
            (329, 2),
            (377, 1),
            (409, 0),
        ];
        for (b, want) in transitions {
            let at = assign_base_categories(&v, b);
            let before = assign_base_categories(&v, b - 1);
            assert!(at.iter().all(|&c| c == want), "budget {b}: {at:?}");
            assert!(
                before.iter().all(|&c| c == want + 1),
                "budget {}: {before:?}",
                b - 1
            );
        }
    }

    #[test]
    fn nonflat_base_vectors_match_validator() {
        // Full category vectors captured from the validator this round.
        let a = [0, 4, 8, 12, 2, 6, 10, 14];
        for (b, want) in [
            (80, vec![7, 7, 7, 5, 7, 7, 6, 4]),
            (100, vec![7, 7, 6, 4, 7, 7, 5, 3]),
            (150, vec![7, 7, 5, 3, 7, 6, 4, 2]),
            (180, vec![7, 6, 4, 2, 7, 5, 3, 1]),
            (227, vec![7, 5, 3, 1, 6, 4, 2, 0]),
            (279, vec![6, 4, 2, 0, 5, 3, 1, 0]),
            (340, vec![5, 3, 1, 0, 4, 2, 0, 0]),
            (380, vec![4, 2, 0, 0, 3, 1, 0, 0]),
        ] {
            assert_eq!(assign_base_categories(&a, b), want, "vec A budget {b}");
        }

        let c = [10, 0, 10, 0, 10, 0, 10, 0];
        for (b, want) in [
            (100, vec![5, 7, 5, 7, 5, 7, 5, 7]),
            (150, vec![3, 7, 3, 7, 3, 7, 3, 7]),
            (200, vec![2, 7, 2, 7, 2, 7, 2, 7]),
            (250, vec![1, 6, 1, 6, 1, 6, 1, 6]),
            (300, vec![0, 5, 0, 5, 0, 5, 0, 5]),
        ] {
            assert_eq!(assign_base_categories(&c, b), want, "vec C budget {b}");
        }

        let d = [0, 1, 2, 3, 4, 5];
        for (b, want) in [
            (60, vec![7, 7, 7, 7, 6, 6]),
            (100, vec![7, 7, 6, 6, 5, 5]),
            (140, vec![6, 6, 5, 5, 4, 4]),
            (180, vec![6, 5, 5, 4, 4, 3]),
            (220, vec![5, 4, 4, 3, 3, 2]),
        ] {
            assert_eq!(assign_base_categories(&d, b), want, "vec D budget {b}");
        }
    }

    #[test]
    fn flat_nonzero_and_nb4_match_validator() {
        // v=[3;6], Nb=6.
        let b = [3i32; 6];
        assert_eq!(assign_base_categories(&b, 0), vec![7; 6]);
        assert_eq!(assign_base_categories(&b, 50), vec![6; 6]);
        assert_eq!(assign_base_categories(&b, 120), vec![6; 6]);
        assert_eq!(assign_base_categories(&b, 200), vec![4; 6]);
        assert_eq!(assign_base_categories(&b, 260), vec![2; 6]);
        // Nb=4 flat transitions: {33,97,121,149,181,205,221}.
        let f = [0i32; 4];
        for (bud, want) in [
            (32, 7u8),
            (33, 6),
            (96, 6),
            (97, 5),
            (120, 5),
            (121, 4),
            (148, 4),
            (149, 3),
            (204, 2),
            (205, 1),
            (220, 1),
            (221, 0),
        ] {
            assert!(
                assign_base_categories(&f, bud).iter().all(|&c| c == want),
                "Nb=4 budget {bud} want {want}"
            );
        }
    }

    #[test]
    fn uniform_refinement_matches_validator() {
        // Nb=8, v=0, B=200: base uniform cat5 (cost 176). M upgrades
        // bands 0.. one per pass, M-1 total; idx descending.
        let v = [0i32; 8];
        let cases = [
            (1u32, vec![5, 5, 5, 5, 5, 5, 5, 5], vec![]),
            (2, vec![4, 5, 5, 5, 5, 5, 5, 5], vec![0u32]),
            (3, vec![4, 4, 5, 5, 5, 5, 5, 5], vec![1, 0]),
            (4, vec![4, 4, 4, 5, 5, 5, 5, 5], vec![2, 1, 0]),
            (5, vec![4, 4, 4, 4, 5, 5, 5, 5], vec![3, 2, 1, 0]),
            (6, vec![4, 4, 4, 4, 4, 5, 5, 5], vec![4, 3, 2, 1, 0]),
        ];
        for (m, cats, idx) in cases {
            let a = assign_categories(&v, 200, m);
            assert_eq!(a.categories, cats, "M={m}");
            assert_eq!(a.adjusted, idx, "M={m} idx");
        }
        // B=260: base uniform cat4 (cost 232); same shape one finer.
        let a = assign_categories(&v, 260, 4);
        assert_eq!(a.categories, vec![3, 3, 3, 4, 4, 4, 4, 4]);
        assert_eq!(a.adjusted, vec![2, 1, 0]);
    }

    #[test]
    fn over_budget_uniform_base_is_left_unrefined() {
        // Nb=8, v=0, B=60: base uniform cat6 (cost 128, already past the
        // raw budget). The first refinement candidate would land at
        // 128 + 6 = 134 > 2*60 - 6, so every candidate is a recorded
        // no-op (the validator's "reclaim branch records candidate bands"
        // observation) and the categories stay the base.
        let v = [0i32; 8];
        let a = assign_categories(&v, 60, 4);
        assert_eq!(a.categories, vec![6; 8]);
        assert!(a.adjusted.is_empty());
    }

    #[test]
    fn nonflat_refinement_walks_the_parity_sweep_ascending() {
        // v=[0,4,8,12,2,6,10,14], B=250: base off=-17 gives t = 15 - v,
        // all odd, so the first unit step has no candidates and the
        // second (t = 14 - v, all even) visits bands 0..=6 ascending
        // (band 7 is already category 0 and is absorbed by the clip).
        // M binds: M-1 candidates apply (the half-bit cap 2*250-6 = 494
        // is far above the base cost 246).
        let v = [0, 4, 8, 12, 2, 6, 10, 14];
        let base = assign_base_categories(&v, 250);
        assert_eq!(base, vec![7, 5, 3, 1, 6, 4, 2, 0]);
        let cases: [(u32, Vec<u8>, Vec<u32>); 5] = [
            (1, vec![7, 5, 3, 1, 6, 4, 2, 0], vec![]),
            (2, vec![6, 5, 3, 1, 6, 4, 2, 0], vec![0]),
            (3, vec![6, 4, 3, 1, 6, 4, 2, 0], vec![1, 0]),
            (5, vec![6, 4, 2, 0, 6, 4, 2, 0], vec![3, 2, 1, 0]),
            (8, vec![6, 4, 2, 0, 5, 3, 1, 0], vec![6, 5, 4, 3, 2, 1, 0]),
        ];
        for (m, cats, idx) in cases {
            let a = assign_categories(&v, 250, m);
            assert_eq!(a.categories, cats, "M={m}");
            assert_eq!(a.adjusted, idx, "M={m} idx");
        }
        // Past the first full sweep the walk continues into the next
        // parity class (now t = 13 - v, the same bands again, one finer).
        let a = assign_categories(&v, 250, 16);
        assert_eq!(a.categories, vec![4, 2, 1, 0, 4, 2, 0, 0]);
        assert_eq!(a.adjusted.len(), 15);
    }

    // ---- round-9 live frames: the vendor's own real-frame output ----

    fn live_frames() -> Vec<(u32, Vec<i32>, i32, Vec<u8>)> {
        let io = crate::tables::live_frame_allocator_io();
        let params = crate::tables::live_frame_params();
        io.iter()
            .zip(params)
            .map(|(fr, pr)| {
                assert_eq!(fr.packet, pr.packet);
                (
                    fr.packet,
                    fr.values.clone(),
                    pr.alloc_budget,
                    fr.categories.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn live_frames_reproduce_the_vendor_categories_exactly() {
        // provenance/09 §3 / tables/live-frame-allocator-io.csv: replaying
        // each captured (v[], budget) at Nb=34, M=128 must reproduce the
        // live category array bit-exactly — 34/34 bands on all three
        // frames. Frame 2 stops on a sweep boundary; frames 16 and 17
        // stop mid-sweep (33/34 and 31/34 against any single offset), so
        // this pins the step rule, not just the final offset.
        for (packet, v, budget, want) in live_frames() {
            assert_eq!(v.len(), 34);
            let a = assign_categories(&v, budget, 128);
            assert_eq!(
                a.categories, want,
                "packet {packet}: categories differ from the live capture"
            );
        }
    }

    #[test]
    fn live_frames_land_just_under_the_half_bit_target() {
        // provenance/09 §3a: Σcost[cat[b]] = 1124 / 1130 / 1126 against
        // 2 × budget = 1130 / 1138 / 1134 (cost in half-bits).
        let want_total = [1124u32, 1130, 1126];
        for ((packet, v, budget, _), want) in live_frames().into_iter().zip(want_total) {
            let a = assign_categories(&v, budget, 128);
            let total = total_cost(&a.categories);
            assert_eq!(total, want, "packet {packet} total cost");
            let target = REFINEMENT_TARGET_FACTOR * budget;
            assert!(
                (total as i32) < target,
                "packet {packet}: {total} >= {target}"
            );
            assert!(
                target - (total as i32) <= 10,
                "packet {packet}: slack {} too loose",
                target - total as i32
            );
        }
    }

    #[test]
    fn live_frame_2_stage1_lands_at_minus_three_and_sweeps_as_documented() {
        // provenance/09 §3: packet 2 (budget 565) — stage-1 offset −3,
        // then five unit sweeps of 14 / 16 / 15 / 17 / 13 changes whose
        // first two are the even-t bands {0,1,2,3,4,5,7,11,13,15,18,25,
        // 26,27} and the odd-t bands {6,8,9,10,12,14,16,17,19,20,21,22,
        // 23,24,28,29} of the base, landing exactly on base(−8).
        let (_, v, budget, want) = live_frames().into_iter().next().unwrap();
        assert_eq!(assign_base_offset(&v, budget), -3);
        // Replay sweep by sweep through the refinement bound: each extra
        // sweep's worth of steps applies that sweep's change set.
        let sweep_sizes = [14u32, 16, 15, 17, 13];
        let mut steps = 0u32;
        let mut prev = assign_base_categories(&v, budget);
        let sweeps_doc: [&[u32]; 2] = [
            &[0, 1, 2, 3, 4, 5, 7, 11, 13, 15, 18, 25, 26, 27],
            &[6, 8, 9, 10, 12, 14, 16, 17, 19, 20, 21, 22, 23, 24, 28, 29],
        ];
        for (i, size) in sweep_sizes.iter().enumerate() {
            steps += size;
            let a = assign_categories(&v, budget, steps + 1);
            let changed: Vec<u32> = (0..34u32)
                .filter(|&b| a.categories[b as usize] != prev[b as usize])
                .collect();
            assert_eq!(changed.len(), *size as usize, "sweep {i} size");
            if i < 2 {
                assert_eq!(changed, sweeps_doc[i], "sweep {i} membership");
            }
            // Every change is one category finer.
            for &b in &changed {
                assert_eq!(a.categories[b as usize] + 1, prev[b as usize]);
            }
            prev = a.categories;
        }
        // After exactly 75 changes the walk is at base(−8) == the live array.
        assert_eq!(prev, base_categories_at(-8, &v));
        assert_eq!(prev, want);
        // And the full M=128 walk stops there (the next candidate would
        // land at 2 × budget exactly, which the cap rejects).
        assert_eq!(assign_categories(&v, budget, 128).categories, want);
    }

    #[test]
    fn refinement_cap_window_is_exactly_five_to_six() {
        // Document the fitted constant: slack values 5 and 6 both
        // reproduce all three live frames; 4 and 7 do not. (The walk is
        // re-run with a locally substituted cap to pin the window.)
        fn walk_with_cap(v: &[i32], budget: i32, m: u32, slack: i32) -> Vec<u8> {
            let base_offset = assign_base_offset(v, budget);
            let mut cats = base_categories_at(base_offset, v);
            let mut total: i32 = cats.iter().map(|&c| category_cost(c) as i32).sum();
            let cap = REFINEMENT_TARGET_FACTOR * budget - slack;
            let (mut steps, mut off) = (0u32, base_offset);
            while steps < m - 1 {
                let (mut any, mut applied) = (false, false);
                for (b, &vb) in v.iter().enumerate() {
                    let t = BASE_CONSTANT_K + off - vb;
                    if t.rem_euclid(2) != 0 {
                        continue;
                    }
                    let new = (t - 1) >> 1;
                    if !(0..=7).contains(&new) || new == (t >> 1).clamp(0, 7) {
                        continue;
                    }
                    any = true;
                    if steps >= m - 1 {
                        break;
                    }
                    steps += 1;
                    let d = category_cost(new as u8) as i32 - category_cost(cats[b]) as i32;
                    if total + d <= cap {
                        total += d;
                        cats[b] = new as u8;
                        applied = true;
                    }
                }
                off -= 1;
                if any && !applied {
                    break;
                }
            }
            cats
        }
        let frames = live_frames();
        for slack in [5i32, 6] {
            for (packet, v, budget, want) in &frames {
                assert_eq!(
                    walk_with_cap(v, *budget, 128, slack),
                    *want,
                    "slack {slack} must reproduce packet {packet}"
                );
            }
        }
        for slack in [4i32, 7] {
            let fails = frames
                .iter()
                .filter(|(_, v, budget, want)| walk_with_cap(v, *budget, 128, slack) != *want)
                .count();
            assert!(
                fails > 0,
                "slack {slack} unexpectedly reproduces every live frame"
            );
        }
        assert_eq!(REFINEMENT_CAP_SLACK, 6);
    }

    #[test]
    fn band_categories_view_maps_empty_sentinel() {
        let v = [0, 4, 8, 12, 2, 6, 10, 14];
        let bands = assign_band_categories(&v, 250, 1).unwrap();
        assert_eq!(bands[0], BandCategory::Empty); // category 7
        assert_eq!(
            bands[1],
            BandCategory::Coded(CategoryIndex::new(5).unwrap())
        );
        assert_eq!(
            bands[7],
            BandCategory::Coded(CategoryIndex::new(0).unwrap())
        );
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert!(assign_base_categories(&[], 100).is_empty());
        let a = assign_categories(&[], 100, 4);
        assert!(a.categories.is_empty());
        assert!(a.adjusted.is_empty());
    }

    #[test]
    fn tiny_budget_is_all_empty_band() {
        // Budget <= K makes every band the free empty band (cost 0):
        // 0 + K < B is false for B <= 32.
        let v = [0i32; 5];
        assert_eq!(assign_base_categories(&v, 0), vec![7; 5]);
        assert_eq!(assign_base_categories(&v, 32), vec![7; 5]);
        assert_eq!(assign_base_categories(&v, 33), vec![6; 5]);
    }
}
