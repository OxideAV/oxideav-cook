//! Per-band gain/quantiser arithmetic (worker `cook.dll!0x69f0`).
//!
//! Source-of-truth: `docs/audio/cook/spec/05-cook-backend-frame-syntax.md`
//! §2.2 (the per-band quantiser closed form), the two `.meta` files
//! `docs/audio/cook/tables/{gain-bias-ramp,category-level-count}.meta`,
//! and `docs/audio/cook/provenance/05-cook-backend.md` evidence #7
//! (*"Quantiser closed form `clip(round(bias + |q|*step/divisor),
//! level)`; divisor `1.0` at `0xa7d4`"*; the worker `0x69f0` does
//! `fabs`, `*step`, `+bias`, `fdiv [0xa7d4]`, `f→i`, clamp to
//! `[cat*4+0x8f90]`). These pin the per-band quantiser worker
//! `cook.dll!0x69f0` beyond the parallel-table *access* already wired in
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
//! 3. **The full closed form (spec/05 §2.2).** The worker assembles the
//!    two primitives above into a single per-band quantiser level:
//!    `level = clip( round( bias[cat] + |q| * step[cat] / divisor ),
//!    level_count[cat] )`, where the `divisor` is the f32 `1.0` constant
//!    at RVA `0xa7d4` ([`QUANTISER_DIVISOR`], evidence #7's `fdiv
//!    [0xa7d4]`). The order of operations is the binary's: `fabs(q)`,
//!    `* step`, `+ bias`, `/ divisor`, float→int round, then the
//!    `0x8f90` level-count clamp. With the pinned `divisor == 1.0` the
//!    division is the identity, so `level = clip(round(bias + |q|*step),
//!    level_count)` — but the divisor is carried as a named constant
//!    because the binary applies it unconditionally (evidence #7).
//!
//! ## What this module provides
//!
//! - [`band_gain_magnitude`] — evaluates `bias + |sample| * step` for a
//!   [`CategoryParameters`] bundle and one input sample, exactly the
//!   `gain-bias-ramp.meta` form.
//! - [`clip_quantiser_index`] — clips a raw per-band quantiser index to
//!   the `0..=level_count-1` range the category's level-count admits,
//!   the `category-level-count.meta` clip.
//! - [`quantiser_level`] — the full §2.2 closed form
//!   `clip(round(bias + |q|*step / divisor), level_count)`, composing
//!   the magnitude form, the [`QUANTISER_DIVISOR`] divide, the
//!   round-to-nearest float→int, and the level-count clip in the
//!   binary's order.
//! - [`CategoryParameters::band_gain_magnitude`] /
//!   [`CategoryParameters::clip_quantiser_index`] /
//!   [`CategoryParameters::quantiser_level`] — the same operations as
//!   methods on the bundle, since each reads fields of one
//!   already-looked-up [`CategoryParameters`].
//!
//! ## What this module does *not* cover (DOCS-GAP)
//!
//! The pinned facts are the *per-band* arithmetic. The band loop that
//! drives them — how raw quantiser indices `q` are read from the
//! bitstream (the §3.1 VLC walk over the recovered codebooks — see
//! [`crate::spectral_decode`]), how the level combines with the
//! `0x8fcc` category-expectation
//! table (audit #17, *"not statically unambiguous"*), the sign
//! restoration, and where the result feeds the inverse MDCT — is **not**
//! pinned beyond the spec/05 §2.2 closed form and the two `.meta`
//! sentences. This module wires the closed form and its two primitives;
//! the surrounding loop and the `q`-supplying entropy walk remain
//! recorded GAPs.

use crate::category::CategoryParameters;

/// The per-band quantiser divisor — the f32 `1.0` constant at RVA
/// `0xa7d4` the worker `cook.dll!0x69f0` divides the biased magnitude by
/// (`spec/05` §2.2, `provenance/05` evidence #7: *"divisor `1.0` at
/// `0xa7d4`"*, the `fdiv [0xa7d4]`).
///
/// The pinned value is `1.0`, so the division is mathematically the
/// identity; it is carried as a named constant because the binary applies
/// it unconditionally in the closed form
/// `clip(round(bias + |q|*step / divisor), level_count)`.
pub const QUANTISER_DIVISOR: f32 = 1.0;

/// `.rdata` RVA of the [`QUANTISER_DIVISOR`] `1.0` constant
/// (`spec/05` §2.2, `provenance/05` evidence #7).
pub const QUANTISER_DIVISOR_RVA: u32 = 0xa7d4;

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

    /// The full §2.2 per-band quantiser level for a raw VLC magnitude
    /// `q`: `clip(round(bias + |q|*step / divisor), level_count)` against
    /// this category's parameters.
    ///
    /// `q` is the per-band quantisation magnitude the §3.1 VLC walk
    /// decodes (normally the wired §3.1 VLC walk supplies it; this is
    /// the arithmetic the worker applies once `q` is in hand). See
    /// [`quantiser_level`] for the free-function form and the order of
    /// operations.
    pub fn quantiser_level(&self, q: f32) -> u32 {
        quantiser_level(self, q)
    }
}

/// The full §2.2 per-band quantiser closed form
/// `clip(round(bias + |q|*step / divisor), level_count)` (worker
/// `cook.dll!0x69f0`, `spec/05` §2.2, `provenance/05` evidence #7).
///
/// Operations are applied in the binary's order:
///
/// 1. `mag = bias + |q| * step` — the [`band_gain_magnitude`] form
///    (`fabs`, `* step`, `+ bias`).
/// 2. `mag / divisor` — the [`QUANTISER_DIVISOR`] divide (`fdiv
///    [0xa7d4]`); the pinned divisor is `1.0`.
/// 3. round-to-nearest float→int — the `f→i` conversion. A negative
///    rounded value (possible because `bias` is negative and `|q|*step`
///    can be small) is floored to `0` before the clip, since the
///    quantiser index is unsigned and the level-count clamp's lower
///    bound is `0`.
/// 4. `clip(.., level_count)` — the [`clip_quantiser_index`] cap to
///    `0..=level_count-1`.
///
/// `params` is one looked-up [`CategoryParameters`] bundle; `q` is the
/// raw per-band VLC magnitude (normally the wired §3 entropy read's output).
pub fn quantiser_level(params: &CategoryParameters, q: f32) -> u32 {
    let mag = band_gain_magnitude(params, q) / QUANTISER_DIVISOR;
    // Round-to-nearest then floor any negative result to 0 (the index is
    // unsigned; the level-count clamp's lower bound is 0).
    let rounded = mag.round();
    let raw = if rounded <= 0.0 { 0 } else { rounded as u32 };
    clip_quantiser_index(params.level_count, raw)
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

    #[test]
    fn quantiser_divisor_is_pinned_one() {
        // spec/05 §2.2 / evidence #7: divisor is the f32 1.0 at 0xa7d4.
        assert_eq!(QUANTISER_DIVISOR, 1.0);
        assert_eq!(QUANTISER_DIVISOR_RVA, 0xa7d4);
    }

    #[test]
    fn quantiser_level_matches_closed_form() {
        // For every category and a spread of magnitudes, the level equals
        // a hand-evaluation of clip(round(bias + |q|*step / divisor),
        // level_count), and the method agrees with the free function.
        for cat in 0..=crate::MAX_CATEGORY_INDEX {
            let p = params(cat);
            for &q in &[-9.0f32, -2.5, -0.0, 0.0, 0.4, 1.0, 3.7, 50.0] {
                let mag = (p.gain_bias + q.abs() * p.gain_step) / QUANTISER_DIVISOR;
                let rounded = mag.round();
                let raw = if rounded <= 0.0 { 0 } else { rounded as u32 };
                let expected = clip_quantiser_index(p.level_count, raw);
                assert_eq!(quantiser_level(&p, q), expected, "cat {cat} q {q}");
                assert_eq!(p.quantiser_level(q), expected, "cat {cat} q {q}");
            }
        }
    }

    #[test]
    fn quantiser_level_uses_absolute_magnitude() {
        // The closed form takes |q|: +q and -q give the same level.
        for cat in 0..=crate::MAX_CATEGORY_INDEX {
            let p = params(cat);
            for &q in &[0.5f32, 1.0, 7.0, 33.0] {
                assert_eq!(p.quantiser_level(q), p.quantiser_level(-q));
            }
        }
    }

    #[test]
    fn quantiser_level_small_magnitude_floors_to_zero() {
        // bias is negative (-0.20..0.0); a tiny |q| keeps bias + |q|*step
        // <= 0, which floors to index 0 (the unsigned lower bound).
        let p = params(0); // bias = -0.20, step = 0.354 (smallest).
        assert!(p.gain_bias < 0.0);
        assert_eq!(p.quantiser_level(0.0), 0); // bias alone < 0 → 0.
        assert_eq!(p.quantiser_level(0.1), 0); // still <= 0 after round.
    }

    #[test]
    fn quantiser_level_clips_to_top_for_large_magnitude() {
        // A huge magnitude rounds well past every category's level count,
        // so the result is clipped to level_count - 1.
        for cat in 0..=crate::MAX_CATEGORY_INDEX {
            let p = params(cat);
            assert_eq!(p.quantiser_level(1.0e6), p.level_count - 1, "cat {cat}");
        }
    }

    #[test]
    fn quantiser_level_divisor_is_identity() {
        // With the pinned divisor == 1.0 the divide is the identity:
        // quantiser_level equals round-then-clip of band_gain_magnitude.
        for cat in 0..=crate::MAX_CATEGORY_INDEX {
            let p = params(cat);
            for &q in &[2.0f32, 5.5, 11.0] {
                let mag = p.band_gain_magnitude(q);
                let rounded = mag.round();
                let raw = if rounded <= 0.0 { 0 } else { rounded as u32 };
                assert_eq!(
                    p.quantiser_level(q),
                    clip_quantiser_index(p.level_count, raw)
                );
            }
        }
    }
}
