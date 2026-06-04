//! Per-category gain / quantiser parameter bundle.
//!
//! Source-of-truth: `docs/audio/cook/provenance/03-cook-audit.md` audit
//! points #18 and #19 (the two parameter tables newly pinned in round 3)
//! and the three `.meta` files
//! `docs/audio/cook/tables/{gain-step-2pow-half,gain-bias-ramp,
//! category-level-count}.meta`. Each pins that the table is indexed by
//! the **gain/quantiser category index** `0..=6` in the per-band
//! quantiser worker `cook.dll!0x69f0`, in parallel with the other two.
//!
//! ## What the binary does
//!
//! `0x69f0` reads three parallel `.rdata` arrays at `[cat*4 + base]`:
//!
//! - `0x8f58` (`gain-step-2pow-half`, 7 × f32): the per-category
//!   `2^(n/2)` step factor for `n = -3..+3`, centred on `1.0` at
//!   `cat = 3`.
//! - `0x8f74` (`gain-bias-ramp`, 7 × f32): the per-category bias ramp
//!   `-0.20..0.0`, monotone-increasing.
//! - `0x8f90` (`category-level-count`, 7 × u32): the per-category
//!   level-count / clip bound `{13, 9, 6, 4, 3, 2, 1}`,
//!   strictly-decreasing.
//!
//! The audit's `gain-bias-ramp.meta` records the parallel access pattern
//! verbatim: *"indexed by the gain/quantiser category index (0..6) … in
//! parallel with the 2^(n/2) gain-step table … and the per-category
//! level-count LUT: the worker forms (bias + |sample| * step) per
//! band"*. The worker guards out `category == 7` (and any larger value)
//! before doing the lookup.
//!
//! ## What this module provides
//!
//! A typed bundle of the three per-category parameters plus a typed
//! [`CategoryIndex`] newtype that enforces the `0..=6` range at
//! construction time. The bundle is a structural model of the
//! parallel-table access pattern — the full per-band quantiser
//! algorithm (the *"forms (bias + |sample| * step) per band"* part of
//! the meta line, plus the band-loop driving it) is not pinned beyond
//! that single audit sentence and remains a DOCS-GAP; this module only
//! wires the lookup itself.
//!
//! - [`CATEGORY_COUNT`] — the number of categories the worker indexes
//!   (`7`, audit #18/#19).
//! - [`MAX_CATEGORY_INDEX`] — the highest valid category index (`6`,
//!   audit-pinned "category 7 is guarded out" note from
//!   `category-level-count.meta`).
//! - [`CategoryIndex`] — newtype wrapping a category index in
//!   `0..=MAX_CATEGORY_INDEX`. Built by [`CategoryIndex::new`] (returns
//!   [`crate::Error::CategoryOutOfRange`] for `> 6`) or the panicking
//!   [`CategoryIndex::new_const`] for const contexts.
//! - [`CategoryParameters`] — the three parallel values for one
//!   category. Built by [`CategoryParameters::for_index`].
//! - [`category_parameters`] — accessor returning the parameter bundle
//!   for an in-range [`CategoryIndex`]. Cached via the existing
//!   `OnceLock`-backed table loaders in [`crate::tables`].
//!
//! ## Wall-respect note
//!
//! Every fact this module pins is anchored to the three `.meta` files
//! and audit points #18 / #19; no algorithmic content beyond the
//! "parallel index" structural fact is wired here.

use crate::{
    tables::{category_level_count, gain_bias_ramp, gain_step_2pow_half},
    Error,
};

/// Number of gain/quantiser categories the per-band quantiser worker
/// `cook.dll!0x69f0` indexes through the three parallel `.rdata` tables.
///
/// Pinned at `7` by `docs/audio/cook/tables/gain-bias-ramp.meta` /
/// `category-level-count.meta` / `gain-step-2pow-half.meta`
/// (`element_count: 7`, in parallel `[cat*4 + base]` form) and by audit
/// points #18 / #19 in `docs/audio/cook/provenance/03-cook-audit.md`.
pub const CATEGORY_COUNT: u8 = 7;

/// Highest valid category index — `CATEGORY_COUNT - 1`.
///
/// `category-level-count.meta`: *"category index 7 is guarded out by
/// the worker"*; categories above `6` are rejected before the lookup.
pub const MAX_CATEGORY_INDEX: u8 = CATEGORY_COUNT - 1;

/// Typed gain/quantiser category index — in `0..=MAX_CATEGORY_INDEX`.
///
/// Constructed by [`CategoryIndex::new`] (or [`CategoryIndex::new_const`]
/// for const contexts), so consumers never index the per-category tables
/// with an out-of-range value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CategoryIndex(u8);

impl CategoryIndex {
    /// Build a [`CategoryIndex`] from a raw `u8`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CategoryOutOfRange`] when `raw > MAX_CATEGORY_INDEX`.
    pub fn new(raw: u8) -> Result<Self, Error> {
        if raw > MAX_CATEGORY_INDEX {
            return Err(Error::CategoryOutOfRange { got: raw });
        }
        Ok(CategoryIndex(raw))
    }

    /// `const`-context constructor. Panics for out-of-range values.
    ///
    /// Use in `const` contexts where the value is a known-valid
    /// compile-time literal; non-const callers should prefer
    /// [`CategoryIndex::new`] and propagate the typed error.
    ///
    /// # Panics
    ///
    /// Panics when `raw > MAX_CATEGORY_INDEX`.
    pub const fn new_const(raw: u8) -> Self {
        if raw > MAX_CATEGORY_INDEX {
            panic!("CategoryIndex::new_const: value out of range");
        }
        CategoryIndex(raw)
    }

    /// Raw category index as `u8`.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Raw category index as `usize` — for table lookup.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// One category's parallel parameter bundle, exactly as the per-band
/// quantiser worker `cook.dll!0x69f0` reads it.
///
/// All three fields are read at the same `[cat*4 + base]` offset across
/// the three `.rdata` arrays, so they are always queried together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CategoryParameters {
    /// `2^(n/2)` step factor for this category (`n = cat - 3`, so
    /// `cat = 3` gives the centred value `1.0`). From
    /// `tables/gain-step-2pow-half.csv` / `.meta`.
    pub gain_step: f32,
    /// Bias ramp value `-0.20..0.0` for this category, monotone-
    /// increasing across `cat = 0..=6`. From `tables/gain-bias-ramp.csv`
    /// / `.meta`.
    pub gain_bias: f32,
    /// Per-category quantiser level-count / clip bound. Sequence is
    /// `{13, 9, 6, 4, 3, 2, 1}` over `cat = 0..=6`, strictly-decreasing.
    /// From `tables/category-level-count.csv` / `.meta`.
    pub level_count: u32,
}

impl CategoryParameters {
    /// Look up the parameter bundle for one category.
    ///
    /// Each field is read at `[index * 4 + base]` from the matching
    /// vendored table — the same parallel access the binary's `0x69f0`
    /// worker performs. The underlying CSV loads are
    /// `OnceLock`-cached (one parse per process).
    pub fn for_index(index: CategoryIndex) -> Self {
        let i = index.as_usize();
        CategoryParameters {
            gain_step: gain_step_2pow_half()[i],
            gain_bias: gain_bias_ramp()[i],
            level_count: category_level_count()[i],
        }
    }
}

/// Convenience accessor — `CategoryParameters::for_index(index)`.
pub fn category_parameters(index: CategoryIndex) -> CategoryParameters {
    CategoryParameters::for_index(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_count_and_max_match_meta() {
        // `.meta`: `element_count: 7`. `MAX_CATEGORY_INDEX` is the
        // highest valid index, i.e. 6.
        assert_eq!(CATEGORY_COUNT, 7);
        assert_eq!(MAX_CATEGORY_INDEX, 6);
        assert_eq!(CATEGORY_COUNT as usize, gain_step_2pow_half().len());
        assert_eq!(CATEGORY_COUNT as usize, gain_bias_ramp().len());
        assert_eq!(CATEGORY_COUNT as usize, category_level_count().len());
    }

    #[test]
    fn category_index_accepts_valid_range() {
        for cat in 0..=MAX_CATEGORY_INDEX {
            let idx = CategoryIndex::new(cat).unwrap();
            assert_eq!(idx.get(), cat);
            assert_eq!(idx.as_usize(), cat as usize);
        }
    }

    #[test]
    fn category_index_rejects_out_of_range() {
        for cat in (MAX_CATEGORY_INDEX + 1)..=u8::MAX {
            let err = CategoryIndex::new(cat).unwrap_err();
            assert_eq!(err, Error::CategoryOutOfRange { got: cat });
            if cat == u8::MAX {
                break;
            }
        }
        // `7` is the audit-named "guarded out" value — pin it explicitly.
        assert_eq!(
            CategoryIndex::new(7).unwrap_err(),
            Error::CategoryOutOfRange { got: 7 }
        );
    }

    #[test]
    #[should_panic(expected = "CategoryIndex::new_const")]
    fn new_const_panics_on_out_of_range() {
        let _ = CategoryIndex::new_const(7);
    }

    #[test]
    fn new_const_accepts_valid_const_context() {
        const C0: CategoryIndex = CategoryIndex::new_const(0);
        const C6: CategoryIndex = CategoryIndex::new_const(6);
        assert_eq!(C0.get(), 0);
        assert_eq!(C6.get(), 6);
    }

    #[test]
    fn parameters_for_cat_3_centre_on_unity_gain_step() {
        // `gain-step-2pow-half.meta`: midpoint 1.0 at index 3 (n = 0).
        let params = CategoryParameters::for_index(CategoryIndex::new(3).unwrap());
        assert_eq!(params.gain_step.to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn parameters_match_parallel_table_lookup() {
        // Every category's bundle matches a manual lookup against the
        // three parallel tables — the same access `0x69f0` performs.
        for cat in 0..=MAX_CATEGORY_INDEX {
            let idx = CategoryIndex::new(cat).unwrap();
            let params = CategoryParameters::for_index(idx);
            let i = cat as usize;
            assert_eq!(params.gain_step, gain_step_2pow_half()[i]);
            assert_eq!(params.gain_bias, gain_bias_ramp()[i]);
            assert_eq!(params.level_count, category_level_count()[i]);
        }
    }

    #[test]
    fn parameters_walk_the_pinned_invariants() {
        // `category-level-count.meta`: strictly-decreasing
        // `{13, 9, 6, 4, 3, 2, 1}`.
        // `gain-bias-ramp.meta`: monotone-increasing `-0.20..0.0`.
        let mut prev_bias = f32::NEG_INFINITY;
        let mut prev_levels = u32::MAX;
        for cat in 0..=MAX_CATEGORY_INDEX {
            let params = CategoryParameters::for_index(CategoryIndex::new(cat).unwrap());
            assert!(
                params.gain_bias >= prev_bias,
                "gain_bias not monotone at cat={cat}"
            );
            assert!(
                params.level_count < prev_levels,
                "level_count not strictly decreasing at cat={cat}"
            );
            prev_bias = params.gain_bias;
            prev_levels = params.level_count;
        }
        // Cap values pinned by the `.meta`.
        let cat0 = CategoryParameters::for_index(CategoryIndex::new(0).unwrap());
        let cat6 = CategoryParameters::for_index(CategoryIndex::new(6).unwrap());
        assert_eq!(cat0.level_count, 13);
        assert_eq!(cat6.level_count, 1);
        assert!(cat0.gain_bias < -0.19 && cat0.gain_bias > -0.21);
        assert!(cat6.gain_bias.abs() < 1e-6);
    }

    #[test]
    fn module_accessor_matches_struct_method() {
        for cat in 0..=MAX_CATEGORY_INDEX {
            let idx = CategoryIndex::new(cat).unwrap();
            assert_eq!(category_parameters(idx), CategoryParameters::for_index(idx));
        }
    }
}
