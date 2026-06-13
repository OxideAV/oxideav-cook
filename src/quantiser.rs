//! Per-band gain/quantiser arithmetic (worker `cook.dll!0x69f0`).
//!
//! Source-of-truth: `docs/audio/cook/tables/gain-bias-ramp.meta` and
//! `docs/audio/cook/tables/category-level-count.meta`, plus audit points
//! #18 / #19 in `docs/audio/cook/provenance/03-cook-audit.md`. These pin
//! two facts about the per-band quantiser worker `cook.dll!0x69f0`
//! beyond the parallel-table *access* already wired in
//! [`crate::category`]:
//!
//! 1. **The dequant magnitude form.** `gain-bias-ramp.meta` records the
//!    worker's per-band arithmetic verbatim: *"the worker forms
//!    `(bias + |sample| * step)` per band"*, where `bias` is the
//!    per-category [`CategoryParameters::gain_bias`] ramp value
//!    (`0x8f74`, `-0.20..0.0`) and `step` is the per-category
//!    [`CategoryParameters::gain_step`] `2^(n/2)` factor (`0x8f58`,
//!    centred on `1.0` at category 3). Both are indexed by the same
//!    `[cat*4 + base]` category index, so the form is evaluated against
//!    one [`CategoryParameters`] bundle.
//!
//! 2. **The level-count clip.** `category-level-count.meta` records the
//!    `{13, 9, 6, 4, 3, 2, 1}` LUT (`0x8f90`) as *"used both to size and
//!    to clip the per-band quantiser index"*: a category with
//!    `level_count = L` admits quantiser indices `0..=L-1`, and an index
//!    at or above `L` is clipped down to the last valid index `L-1`.
//!
//! ## What this module provides
//!
//! - [`band_gain_magnitude`] — evaluates `bias + |sample| * step` for a
//!   [`CategoryParameters`] bundle and one input sample, exactly the
//!   `gain-bias-ramp.meta` form.
//! - [`clip_quantiser_index`] — clips a raw per-band quantiser index to
//!   the `0..=level_count-1` range the category's level-count admits,
//!   the `category-level-count.meta` clip.
//! - [`CategoryParameters::band_gain_magnitude`] /
//!   [`CategoryParameters::clip_quantiser_index`] — the same two
//!   operations as methods on the bundle, since both read fields of one
//!   already-looked-up [`CategoryParameters`].
//!
//! ## What this module does *not* cover (DOCS-GAP)
//!
//! The two pinned facts are the *per-band* arithmetic primitives. The
//! band loop that drives them — how raw quantiser indices are read from
//! the bitstream, how `band_gain_magnitude` is combined with the
//! spectral coefficient (the `0x8fcc` category-expectation table, audit
//! #17, is *"not statically unambiguous"* and left a GAP), the sign
//! restoration, and where the result feeds the inverse MDCT — is **not**
//! pinned by spec/01 or the audit beyond the single `(bias + |sample| *
//! step)` sentence. This module wires only the two primitives the
//! `.meta` files state explicitly; the surrounding loop remains a
//! recorded GAP.

use crate::category::CategoryParameters;

impl CategoryParameters {
    /// Evaluate the per-band gain magnitude `bias + |sample| * step` for
    /// this category, exactly the form `gain-bias-ramp.meta` pins for the
    /// worker `cook.dll!0x69f0`.
    ///
    /// `step` is this bundle's [`CategoryParameters::gain_step`]
    /// (`2^(n/2)`), `bias` is its [`CategoryParameters::gain_bias`]
    /// ramp value, and `sample` is the per-band input the worker takes
    /// the magnitude of.
    pub fn band_gain_magnitude(&self, sample: f32) -> f32 {
        band_gain_magnitude(self, sample)
    }

    /// Clip a raw per-band quantiser index to the `0..=level_count-1`
    /// range this category admits.
    ///
    /// `category-level-count.meta` records the `{13, 9, 6, 4, 3, 2, 1}`
    /// LUT as *"used both to size and to clip the per-band quantiser
    /// index"*: a category with `level_count = L` has `L` distinct
    /// levels (indices `0..=L-1`), so an index `>= L` is clipped to the
    /// last valid index `L - 1`.
    pub fn clip_quantiser_index(&self, raw_index: u32) -> u32 {
        clip_quantiser_index(self.level_count, raw_index)
    }
}

/// Evaluate `bias + |sample| * step` for a category's parameter bundle.
///
/// This is the per-band magnitude the worker `cook.dll!0x69f0` forms,
/// recorded verbatim in `docs/audio/cook/tables/gain-bias-ramp.meta`:
/// *"the worker forms `(bias + |sample| * step)` per band"*. `bias` is
/// [`CategoryParameters::gain_bias`], `step` is
/// [`CategoryParameters::gain_step`].
pub fn band_gain_magnitude(params: &CategoryParameters, sample: f32) -> f32 {
    params.gain_bias + sample.abs() * params.gain_step
}

/// Clip a raw per-band quantiser index against a category's level count.
///
/// `category-level-count.meta`: the `{13, 9, 6, 4, 3, 2, 1}` LUT is
/// *"used both to size and to clip the per-band quantiser index"*. A
/// category with `level_count = L` has `L` distinct quantiser levels, so
/// valid indices are `0..=L-1`; any `raw_index >= L` is clipped to the
/// top valid index `L - 1`.
///
/// `level_count` is always `>= 1` for the seven categories
/// (`{13, 9, 6, 4, 3, 2, 1}`), so `L - 1` never underflows for any
/// looked-up [`CategoryParameters`]; the guard below keeps the helper
/// total for a hypothetical zero count.
pub fn clip_quantiser_index(level_count: u32, raw_index: u32) -> u32 {
    if level_count == 0 {
        return 0;
    }
    raw_index.min(level_count - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::category::CategoryIndex;

    fn params(cat: u8) -> CategoryParameters {
        CategoryParameters::for_index(CategoryIndex::new(cat).unwrap())
    }

    #[test]
    fn gain_magnitude_matches_pinned_form() {
        // `bias + |sample| * step` for every category and a spread of
        // signed inputs — the free-function and method agree, and both
        // equal a hand-evaluation of the `gain-bias-ramp.meta` form.
        for cat in 0..=crate::MAX_CATEGORY_INDEX {
            let p = params(cat);
            for &sample in &[-3.5f32, -1.0, -0.0, 0.0, 0.25, 2.0, 17.0] {
                let expected = p.gain_bias + sample.abs() * p.gain_step;
                assert_eq!(band_gain_magnitude(&p, sample), expected);
                assert_eq!(p.band_gain_magnitude(sample), expected);
            }
        }
    }

    #[test]
    fn gain_magnitude_uses_absolute_value() {
        // The form takes |sample|: +x and -x give the same magnitude.
        let p = params(3); // gain_step = 1.0, centred category.
        for &x in &[0.5f32, 1.0, 9.0, 123.5] {
            assert_eq!(p.band_gain_magnitude(x), p.band_gain_magnitude(-x));
        }
    }

    #[test]
    fn gain_magnitude_cat3_is_bias_plus_abs() {
        // Category 3 has gain_step == 1.0 (meta midpoint), so the form
        // reduces to bias + |sample|.
        let p = params(3);
        assert_eq!(p.gain_step.to_bits(), 1.0f32.to_bits());
        assert_eq!(p.band_gain_magnitude(4.0), p.gain_bias + 4.0);
        assert_eq!(p.band_gain_magnitude(-4.0), p.gain_bias + 4.0);
    }

    #[test]
    fn zero_sample_yields_bias() {
        // |0| * step == 0, so the form collapses to the category bias.
        for cat in 0..=crate::MAX_CATEGORY_INDEX {
            let p = params(cat);
            assert_eq!(p.band_gain_magnitude(0.0), p.gain_bias);
        }
    }

    #[test]
    fn clip_passes_through_in_range_indices() {
        // For each category, indices 0..level_count are unchanged.
        for cat in 0..=crate::MAX_CATEGORY_INDEX {
            let p = params(cat);
            for idx in 0..p.level_count {
                assert_eq!(p.clip_quantiser_index(idx), idx);
            }
        }
    }

    #[test]
    fn clip_caps_at_top_valid_index() {
        // An index at or above level_count clips to level_count - 1.
        for cat in 0..=crate::MAX_CATEGORY_INDEX {
            let p = params(cat);
            let top = p.level_count - 1;
            assert_eq!(p.clip_quantiser_index(p.level_count), top);
            assert_eq!(p.clip_quantiser_index(p.level_count + 100), top);
            assert_eq!(p.clip_quantiser_index(u32::MAX), top);
        }
    }

    #[test]
    fn clip_matches_known_level_counts() {
        // Spot-check against the pinned {13, 9, 6, 4, 3, 2, 1} LUT.
        // cat 0 → 13 levels (top index 12); cat 6 → 1 level (top index 0).
        let cat0 = params(0);
        assert_eq!(cat0.level_count, 13);
        assert_eq!(clip_quantiser_index(cat0.level_count, 12), 12);
        assert_eq!(clip_quantiser_index(cat0.level_count, 13), 12);

        let cat6 = params(6);
        assert_eq!(cat6.level_count, 1);
        // Single-level category: every index collapses to 0.
        assert_eq!(clip_quantiser_index(cat6.level_count, 0), 0);
        assert_eq!(clip_quantiser_index(cat6.level_count, 7), 0);
    }

    #[test]
    fn clip_zero_level_count_is_total() {
        // Defensive: a hypothetical zero count never underflows.
        assert_eq!(clip_quantiser_index(0, 0), 0);
        assert_eq!(clip_quantiser_index(0, u32::MAX), 0);
    }
}
