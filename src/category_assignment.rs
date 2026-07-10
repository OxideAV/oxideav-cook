//! §2.2 category-assignment / bit-allocation pass (`cook.dll!0x4800`).
//!
//! Source-of-truth:
//! `docs/audio/cook/provenance/08-cook-category-assignment.md` and
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
//! ## Stage 2 — per-band ±1 refinement (`provenance/08` §"Stage 2")
//!
//! After the base pass leaves a residual budget, a refinement loop
//! **bounded by the decode-state field `+0x28`** (`M`, the
//! [`refinement_bound`](assign_categories) argument) adjusts individual
//! bands by ±1 category to spend/reclaim the residual, recording the
//! order of adjusted bands in an index list. For a **uniform** base under
//! budget the loop is fully characterised: it upgrades the lowest-index
//! bands one category finer, one band per pass, `M − 1` bands in total
//! (the validator: `Nb=8`, `v=0`, `B=200`, base uniform category 5 →
//! `M=2` upgrades band 0, `M=4` upgrades bands 0/1/2, …). That regime is
//! implemented and validated here ([`refine_uniform`]).
//!
//! The **non-flat priority interleave** and the **over-budget reclaim**
//! direction (which also emit a mixed upgrade/reclaim index list) are, as
//! `provenance/08` itself records, only *partially* characterised. This
//! module therefore applies refinement only in the validated
//! uniform-under-budget regime and otherwise returns the base assignment
//! unchanged, never fabricating the uncharacterised order. See the
//! module-level followup in `README.md`.
//!
//! ## Wall-respect note
//!
//! Every constant is either the documented `category-assignment-params`
//! table (`K = 32`, offset start `−32`, steps, clip `0..=7`, `cost[7]=0`)
//! or a value read from the opaque validator's **own output** for a known
//! input. No decoder source was read; the budget-slack rule is the
//! documented `K` under a strict comparison, cross-checked against the
//! validator across flat and non-flat sweeps.

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

/// Whether a base assignment is uniform (all bands the same category).
fn is_uniform(cats: &[u8]) -> Option<u8> {
    let first = *cats.first()?;
    if cats.iter().all(|&c| c == first) {
        Some(first)
    } else {
        None
    }
}

/// Apply the Stage-2 refinement to a **uniform** base assignment under
/// budget: upgrade the lowest-index bands one category finer, one per
/// pass, `M − 1` bands total, stopping at the finest category (0).
///
/// Returns the refined categories and the upgraded band indices in the
/// validator's descending order (`[m−2, m−3, …, 0]`). This is the
/// validated regime of `provenance/08` Stage 2; callers reach it through
/// [`assign_categories`].
#[must_use]
pub fn refine_uniform(base: &[u8], refinement_bound: u32) -> CategoryAssignment {
    let mut categories = base.to_vec();
    let mut adjusted_asc: Vec<u32> = Vec::new();
    // `M - 1` upgrades (the base assignment consumes the first pass).
    let upgrades = refinement_bound.saturating_sub(1) as usize;
    let mut done = 0usize;
    let mut band = 0usize;
    while done < upgrades && band < categories.len() {
        if categories[band] > CATEGORY_CLIP_LO as u8 {
            categories[band] -= 1;
            adjusted_asc.push(band as u32);
            done += 1;
        }
        band += 1;
    }
    // The validator's index list is most-recent-first (descending band).
    adjusted_asc.reverse();
    CategoryAssignment {
        categories,
        adjusted: adjusted_asc,
    }
}

/// Assign per-band categories from `(v, budget)` with an optional Stage-2
/// refinement bounded by `refinement_bound` (`M`, decode-state `+0x28`).
///
/// Runs the base pass ([`assign_base_categories`]); when the base is
/// **uniform and under budget** it applies the validated
/// [`refine_uniform`] Stage-2 upgrade. Otherwise — a non-uniform base or
/// an over-budget base, whose refinement priority order `provenance/08`
/// leaves only partially characterised — it returns the base assignment
/// unchanged rather than fabricate the uncharacterised order.
///
/// `refinement_bound == 0` or `1` applies no refinement (the base pass
/// is the first `M`).
#[must_use]
pub fn assign_categories(v: &[i32], budget: i32, refinement_bound: u32) -> CategoryAssignment {
    let base = assign_base_categories(v, budget);
    let base_cost: u32 = base.iter().map(|&c| category_cost(c)).sum();
    let under_budget = (base_cost as i32) < budget;
    match (is_uniform(&base), under_budget) {
        (Some(c), true) if c > CATEGORY_CLIP_LO as u8 && refinement_bound > 1 => {
            refine_uniform(&base, refinement_bound)
        }
        _ => CategoryAssignment {
            categories: base,
            adjusted: Vec::new(),
        },
    }
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
        // Nb=8, v=0, B=60: base marginally exceeds (uniform cat6, cost
        // 128 > 60). The reclaim-direction refinement index list is only
        // partially characterised, so we leave the base unchanged.
        let v = [0i32; 8];
        let a = assign_categories(&v, 60, 4);
        assert_eq!(a.categories, vec![6; 8]);
        assert!(a.adjusted.is_empty());
    }

    #[test]
    fn nonuniform_base_is_left_unrefined() {
        // A non-flat base: refinement priority order is only partially
        // characterised, so assign_categories returns the base unchanged.
        let v = [0, 4, 8, 12, 2, 6, 10, 14];
        let a = assign_categories(&v, 250, 5);
        assert_eq!(a.categories, assign_base_categories(&v, 250));
        assert!(a.adjusted.is_empty());
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
