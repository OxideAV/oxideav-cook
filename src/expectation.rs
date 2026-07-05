//! §3.1 level → reconstructed-magnitude mapping — the `0x8fc8`
//! category-expectation table behind a typed `[category][level]`
//! accessor, plus the pinned signed-value assembly.
//!
//! Source-of-truth:
//! `docs/audio/cook/provenance/07-cook-spectral-decode.md` item 2 (*"the
//! quantised level → reconstructed SIGNED value convention"*) and
//! `docs/audio/cook/tables/category-expectation.meta`:
//!
//! - each decoded digit is an **unsigned magnitude level** in
//!   `[0, level_count[cat]]` — *"there is no `-level_count/2` centering;
//!   the level is a pure magnitude"*;
//! - for each **non-zero** magnitude one sign bit is read out-of-band
//!   and applied through the `0xa148` sign LUT (*"bit 0 → +1, bit 1 →
//!   −1"*);
//! - the dequantiser `cook.dll!0x4600` maps the level to a
//!   reconstructed magnitude by reading `[level*4 + row_base]` in the
//!   `0x8fc8` category-expectation table, and forms
//!   `value = sign × magnitude × per_band_gain`.
//!
//! ## The 2-D layout pin
//!
//! The `.meta` records the table's 2-D row/column layout as *"not
//! statically unambiguous"*. It is pinned here **empirically from the
//! staged values themselves**: the flat region is `98 = 7 × 14` f32,
//! and under a stride-14 read each row `r` opens with `0.0` (level 0
//! reconstructs to silence), carries exactly `level_count[r]`
//! strictly-increasing non-zero magnitudes — the run lengths
//! `{13, 9, 6, 4, 3, 2, 1}` are precisely the seven per-category level
//! counts — and is zero-padded to the stride. That correspondence
//! determines the row axis to be the category. The
//! [`crate::tables::category_expectation`] loader asserts the full
//! pattern at parse time, so a wrong stride cannot load silently.
//!
//! ## What stays open
//!
//! The `0x9150` dequant magnitude-scale LUT (non-zero entries
//! `{2^-2.5, 2^-2, 2^-0.5}` at index 5/6/7) rides in the same
//! dequantiser: `provenance/07` pins it to the **codebook branch** of
//! `cook.dll!0x4600` while the expectation table serves the
//! *"expectation / non-codebook branch"*, but the runtime
//! scale-**selector** (which of the eight entries a given coefficient
//! multiplies by, and how the two branches are chosen) is not pinned —
//! a recorded gap. This module wires the branch whose indexing *is*
//! pinned (`[level*4 + row_base]`, level-keyed).

use crate::{category::CategoryIndex, spectral::sign_from_bit, tables, Error};

/// RVA of the category-expectation magnitude table
/// (`tables/category-expectation.meta`).
pub const CATEGORY_EXPECTATION_RVA: u32 = 0x8fc8;

/// Row stride of the `[category][level]` read — re-exported from
/// [`tables::CATEGORY_EXPECTATION_STRIDE`] (empirically pinned; see the
/// module docs).
pub const CATEGORY_EXPECTATION_STRIDE: usize = tables::CATEGORY_EXPECTATION_STRIDE;

/// The reconstructed magnitude for a quantised level of a category —
/// the `[level*4 + row_base]` read of `cook.dll!0x4600`'s expectation
/// branch (`provenance/07` item 2 step 3).
///
/// `level` is the unsigned magnitude digit the §3.1 vector decode
/// produced, in `0..=level_count[category]`. Level `0` reconstructs to
/// exactly `0.0` (the table row opens with `0.0`), so zero-magnitude
/// coefficients stay silent.
///
/// # Errors
///
/// [`Error::ExpectationLevelOutOfRange`] when
/// `level > level_count[category]` — beyond the row's pinned magnitude
/// run (the encoder clips levels to `level_count`, `cook.dll!0x69f0`,
/// so a larger level cannot be produced by a well-formed stream).
pub fn expectation_magnitude(category: CategoryIndex, level: u32) -> Result<f32, Error> {
    let level_count = tables::category_level_count()[category.as_usize()];
    if level > level_count {
        return Err(Error::ExpectationLevelOutOfRange {
            category: category.get(),
            got: level,
        });
    }
    let flat = tables::category_expectation();
    Ok(flat[category.as_usize() * CATEGORY_EXPECTATION_STRIDE + level as usize])
}

/// Assemble one reconstructed spectral coefficient from a decoded
/// `(level, sign bit)` pair and the per-band gain — the
/// `provenance/07` item-2 closed form
/// `value = sign × magnitude × per_band_gain` with
/// `magnitude = expectation[category][level]` and the sign from the
/// `0xa148` LUT (bit `0` → `+1`, bit `1` → `−1`).
///
/// `sign_bit` is ignored for a zero level (a zero magnitude carries no
/// sign bit on the wire — pass `0`).
///
/// # Errors
///
/// [`Error::ExpectationLevelOutOfRange`] — see
/// [`expectation_magnitude`].
pub fn dequantise_level(
    category: CategoryIndex,
    level: u32,
    sign_bit: u32,
    band_gain: f32,
) -> Result<f32, Error> {
    let magnitude = expectation_magnitude(category, level)?;
    Ok(sign_from_bit(sign_bit) * magnitude * band_gain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::category_level_count;

    fn cat(c: u8) -> CategoryIndex {
        CategoryIndex::new(c).unwrap()
    }

    #[test]
    fn level_zero_is_silence_for_every_category() {
        for c in 0..7u8 {
            assert_eq!(expectation_magnitude(cat(c), 0).unwrap(), 0.0);
        }
    }

    #[test]
    fn magnitudes_are_positive_and_strictly_increasing() {
        for c in 0..7u8 {
            let lc = category_level_count()[c as usize];
            let mut prev = 0.0f32;
            for level in 1..=lc {
                let m = expectation_magnitude(cat(c), level).unwrap();
                assert!(m > prev, "cat {c} level {level}: {m} must exceed {prev}");
                prev = m;
            }
        }
    }

    #[test]
    fn levels_beyond_the_clip_bound_are_rejected() {
        for c in 0..7u8 {
            let lc = category_level_count()[c as usize];
            assert!(expectation_magnitude(cat(c), lc).is_ok());
            assert_eq!(
                expectation_magnitude(cat(c), lc + 1).unwrap_err(),
                Error::ExpectationLevelOutOfRange {
                    category: c,
                    got: lc + 1
                }
            );
        }
    }

    #[test]
    fn meta_quoted_first_row_values() {
        // .meta quotes row 0 as "0, 0.392, 0.761, …, 4.724" (printed
        // precision).
        assert!((expectation_magnitude(cat(0), 1).unwrap() - 0.392).abs() < 1e-4);
        assert!((expectation_magnitude(cat(0), 2).unwrap() - 0.761).abs() < 1e-4);
        assert!((expectation_magnitude(cat(0), 13).unwrap() - 4.724).abs() < 1e-4);
    }

    #[test]
    fn dequantise_applies_sign_and_gain() {
        let m = expectation_magnitude(cat(3), 2).unwrap();
        // bit 0 → +1, bit 1 → −1 (0xa148), gain multiplies linearly.
        assert_eq!(dequantise_level(cat(3), 2, 0, 1.0).unwrap(), m);
        assert_eq!(dequantise_level(cat(3), 2, 1, 1.0).unwrap(), -m);
        assert_eq!(dequantise_level(cat(3), 2, 0, 2.0).unwrap(), 2.0 * m);
        assert_eq!(dequantise_level(cat(3), 2, 1, 0.5).unwrap(), -0.5 * m);
    }

    #[test]
    fn dequantise_zero_level_is_silent_regardless_of_sign_and_gain() {
        for c in 0..7u8 {
            assert_eq!(dequantise_level(cat(c), 0, 0, 8.0).unwrap(), 0.0);
            assert_eq!(dequantise_level(cat(c), 0, 1, 8.0).unwrap(), 0.0);
        }
    }
}
