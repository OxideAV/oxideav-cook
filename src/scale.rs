//! Exponent-indexed gain/dequant scale ladders (typed accessors).
//!
//! Source-of-truth: `docs/audio/cook/tables/pow2-exponent-table.{csv,meta}`
//! (RVA `0x91fc`, 127 × f32 LE; *"power-of-two exponent table 2^k,
//! k = -63..+63"*; validation: *"every element exactly equals 2^k for
//! consecutive integer k in [-63,63] (f32-exact)"*) and
//! `docs/audio/cook/tables/sqrt2-scale-ladder.{csv,meta}` (RVA `0x93f8`,
//! 127 × f32 LE; *"square-root-of-two ladder 2^(k/2), k = -63..+63"*;
//! validation: *"each element = 2^(k/2) … within f32 precision (max
//! rel-err < 2e-8)"*), plus audit point #15 in
//! `docs/audio/cook/provenance/03-cook-audit.md`.
//!
//! ## What the binary stores
//!
//! `cook.dll`'s `.rdata` holds two back-to-back 127-entry f32 ladders
//! sharing one index convention: element `i` corresponds to exponent
//! `k = i - 63`, so element 63 is `2^0 = 1.0` in both tables.
//!
//! - `0x91fc`: `2^k` — decode-side gain / exponent scaling (the
//!   imported `_CIpow` CRT intrinsic in spec/01 §1.1 is consistent
//!   with exponential dequant scaling, and this ladder is the stored
//!   integer-exponent form).
//! - `0x93f8`: `2^(k/2)` — the dequantisation scale-factor / gain
//!   ladder (half-integer exponent steps).
//!
//! ## The audit-#15 sub-pointer reconciliation
//!
//! Round 1's spec/01 §6 survey reported two *additional* tables at
//! `0x92d4` ("29 f32, exact powers of two, 2^-9 … 2^19") and `0x94a8`
//! ("59 f32, 0.00138, 0.00195, 0.00276, … ≈2^(n/2) ladder"). Audit
//! point #15 (`provenance/03`) corrected both rows: they are
//! **sub-pointers into** the two 127-entry ladders, not separate
//! tables. The element arithmetic is pure RVA subtraction:
//!
//! - `(0x92d4 - 0x91fc) / 4 = 54` → first exponent `54 - 63 = -9`,
//!   exactly the `2^-9` leading value the Round-1 row quotes (and 29
//!   elements onward ends at index 82 → `2^19`, the row's quoted
//!   endpoint).
//! - `(0x94a8 - 0x93f8) / 4 = 44` → first exponent `44 - 63 = -19`,
//!   and `2^(-19/2) ≈ 0.00138` / `2^(-18/2) ≈ 0.00195` /
//!   `2^(-17/2) ≈ 0.00276` are exactly the three leading values the
//!   Round-1 row quotes.
//!
//! This module surfaces both element offsets and both first-exponents
//! as named constants so a later round wiring the in-binary consumers
//! (which may load via the sub-pointer bases rather than the table
//! heads) can anchor its index math to audit point #15 without redoing
//! the RVA arithmetic. The Round-1 element *counts* (29 / 59) are
//! recorded by the audit as superseded estimates of how far the survey
//! scanned, so they are deliberately **not** exported as constants —
//! only the quoted values, which remain byte-true, are pinned by the
//! tests below.
//!
//! ## What this module provides
//!
//! - [`ScaleExponent`] — newtype wrapping an exponent `k` in
//!   `-63..=63`, built by [`ScaleExponent::new`] (returns
//!   [`crate::Error::ScaleExponentOutOfRange`] otherwise) or the
//!   panicking [`ScaleExponent::new_const`] for const contexts.
//! - [`pow2_scale_for_exponent`] — the `2^k` lookup over
//!   [`crate::tables::pow2_exponent_table`].
//! - [`sqrt2_scale_for_exponent`] — the `2^(k/2)` lookup over
//!   [`crate::tables::sqrt2_scale_ladder`].
//! - The four sub-pointer reconciliation constants of audit #15.
//!
//! ## What this module does *not* cover
//!
//! The runtime consumers — which worker indexes which ladder with what
//! exponent source (gain-control words, category values, …) — are not
//! statically pinned by spec/01 (§6 marks the dequant-scale row as a
//! GAP beyond the sub-pointer fact). This module models the typed table
//! access only, not the exponent-producing stage.

use crate::{
    tables::{pow2_exponent_table, sqrt2_scale_ladder},
    Error,
};

/// Lowest exponent both 127-entry ladders store (`k = -63`; both
/// `.meta` files: *"k = -63..+63"*).
pub const SCALE_EXPONENT_MIN: i8 = -63;

/// Highest exponent both 127-entry ladders store (`k = +63`).
pub const SCALE_EXPONENT_MAX: i8 = 63;

/// Index of exponent `0` in both ladders — element 63 is `2^0 = 1.0`
/// (`element i ↔ k = i - 63` per the `.meta` index convention).
pub const SCALE_EXPONENT_BIAS: usize = 63;

/// RVA of the `2^k` exponent table head
/// (`tables/pow2-exponent-table.meta` line `rva: 0x91fc`).
pub const POW2_TABLE_RVA: u32 = 0x91fc;

/// RVA of the `2^(k/2)` scale ladder head
/// (`tables/sqrt2-scale-ladder.meta` line `rva: 0x93f8`).
pub const SQRT2_TABLE_RVA: u32 = 0x93f8;

/// RVA of the Round-1 spec/01 §6 "`2^-9 … 2^19`" row — audit point #15
/// (`provenance/03`) pins it as a **sub-pointer into** the `0x91fc`
/// ladder, not a separate table.
pub const POW2_SUBPOINTER_RVA: u32 = 0x92d4;

/// RVA of the Round-1 spec/01 §6 "`0.00138, 0.00195, 0.00276, …`" row —
/// audit point #15 (`provenance/03`) pins it as a **sub-pointer into**
/// the `0x93f8` ladder, not a separate table.
pub const SQRT2_SUBPOINTER_RVA: u32 = 0x94a8;

/// Element offset of [`POW2_SUBPOINTER_RVA`] inside the `2^k` ladder —
/// `(0x92d4 - 0x91fc) / 4 = 54` (derived, not retyped).
pub const POW2_SUBPOINTER_ELEMENT_OFFSET: usize =
    ((POW2_SUBPOINTER_RVA - POW2_TABLE_RVA) / 4) as usize;

/// Element offset of [`SQRT2_SUBPOINTER_RVA`] inside the `2^(k/2)`
/// ladder — `(0x94a8 - 0x93f8) / 4 = 44` (derived, not retyped).
pub const SQRT2_SUBPOINTER_ELEMENT_OFFSET: usize =
    ((SQRT2_SUBPOINTER_RVA - SQRT2_TABLE_RVA) / 4) as usize;

/// First exponent reachable through the `0x92d4` sub-pointer:
/// `54 - 63 = -9`, the `2^-9` leading value the Round-1 spec/01 §6 row
/// quotes.
pub const POW2_SUBPOINTER_FIRST_EXPONENT: ScaleExponent =
    ScaleExponent::new_const(POW2_SUBPOINTER_ELEMENT_OFFSET as i8 - SCALE_EXPONENT_BIAS as i8);

/// First exponent reachable through the `0x94a8` sub-pointer:
/// `44 - 63 = -19`, whose `2^(-19/2) ≈ 0.00138` is the leading value
/// the Round-1 spec/01 §6 row quotes.
pub const SQRT2_SUBPOINTER_FIRST_EXPONENT: ScaleExponent =
    ScaleExponent::new_const(SQRT2_SUBPOINTER_ELEMENT_OFFSET as i8 - SCALE_EXPONENT_BIAS as i8);

/// Typed exponent into the two 127-entry scale ladders, in
/// `SCALE_EXPONENT_MIN..=SCALE_EXPONENT_MAX` (= `-63..=63`).
///
/// Built by [`ScaleExponent::new`] (or [`ScaleExponent::new_const`] for
/// const contexts), so consumers never index either 127-entry ladder
/// with an out-of-range exponent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScaleExponent(i8);

impl ScaleExponent {
    /// Build a [`ScaleExponent`] from a raw `i8`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ScaleExponentOutOfRange`] when `raw` is outside
    /// `SCALE_EXPONENT_MIN..=SCALE_EXPONENT_MAX`.
    pub fn new(raw: i8) -> Result<Self, Error> {
        if !(SCALE_EXPONENT_MIN..=SCALE_EXPONENT_MAX).contains(&raw) {
            return Err(Error::ScaleExponentOutOfRange { got: raw });
        }
        Ok(ScaleExponent(raw))
    }

    /// `const`-context constructor. Panics for out-of-range values.
    ///
    /// Use in `const` contexts where the value is a known-valid
    /// compile-time literal; non-const callers should prefer
    /// [`ScaleExponent::new`] and propagate the typed error.
    ///
    /// # Panics
    ///
    /// Panics when `raw` is outside
    /// `SCALE_EXPONENT_MIN..=SCALE_EXPONENT_MAX`.
    pub const fn new_const(raw: i8) -> Self {
        if raw < SCALE_EXPONENT_MIN || raw > SCALE_EXPONENT_MAX {
            panic!("ScaleExponent::new_const: value out of range");
        }
        ScaleExponent(raw)
    }

    /// Raw exponent `k` as `i8`.
    pub const fn get(self) -> i8 {
        self.0
    }

    /// Element index into either 127-entry ladder
    /// (`i = k + SCALE_EXPONENT_BIAS`, always in `0..127`).
    pub const fn table_index(self) -> usize {
        (self.0 as i32 + SCALE_EXPONENT_BIAS as i32) as usize
    }
}

/// Look up `2^k` in the 127-entry exponent ladder (`cook.dll!0x91fc`,
/// `tables/pow2-exponent-table.csv`).
///
/// The values are f32-exact per the `.meta` validation note. The
/// underlying CSV load is `OnceLock`-cached (one parse per process).
pub fn pow2_scale_for_exponent(k: ScaleExponent) -> f32 {
    pow2_exponent_table()[k.table_index()]
}

/// Look up `2^(k/2)` in the 127-entry scale ladder (`cook.dll!0x93f8`,
/// `tables/sqrt2-scale-ladder.csv`).
///
/// Matches `2^(k/2)` to better than `2e-8` relative error per the
/// `.meta` validation note. The underlying CSV load is
/// `OnceLock`-cached (one parse per process).
pub fn sqrt2_scale_for_exponent(k: ScaleExponent) -> f32 {
    sqrt2_scale_ladder()[k.table_index()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::{POW2_EXPONENT_LEN, SQRT2_SCALE_LADDER_LEN};

    #[test]
    fn constants_match_meta_and_audit_15() {
        // `.meta`: k = -63..+63 over 127 elements; element 63 is 2^0.
        assert_eq!(SCALE_EXPONENT_MIN, -63);
        assert_eq!(SCALE_EXPONENT_MAX, 63);
        assert_eq!(SCALE_EXPONENT_BIAS, 63);
        assert_eq!(
            (SCALE_EXPONENT_MAX as i32 - SCALE_EXPONENT_MIN as i32 + 1) as usize,
            POW2_EXPONENT_LEN
        );
        assert_eq!(POW2_EXPONENT_LEN, SQRT2_SCALE_LADDER_LEN);
        // Audit #15 sub-pointer arithmetic — pure RVA subtraction.
        assert_eq!(POW2_SUBPOINTER_ELEMENT_OFFSET, 54);
        assert_eq!(SQRT2_SUBPOINTER_ELEMENT_OFFSET, 44);
        assert_eq!(POW2_SUBPOINTER_FIRST_EXPONENT.get(), -9);
        assert_eq!(SQRT2_SUBPOINTER_FIRST_EXPONENT.get(), -19);
        // The first-exponent constants and the element offsets name the
        // same ladder position.
        assert_eq!(
            POW2_SUBPOINTER_FIRST_EXPONENT.table_index(),
            POW2_SUBPOINTER_ELEMENT_OFFSET
        );
        assert_eq!(
            SQRT2_SUBPOINTER_FIRST_EXPONENT.table_index(),
            SQRT2_SUBPOINTER_ELEMENT_OFFSET
        );
        // The two ladders lay back-to-back in `.rdata` (`0x91fc` +
        // 127 × 4 = `0x93f8` — the tables.rs doc's "0x91fc→0x95f0"
        // span note).
        assert_eq!(
            POW2_TABLE_RVA + (POW2_EXPONENT_LEN as u32) * 4,
            SQRT2_TABLE_RVA
        );
    }

    #[test]
    fn exponent_accepts_full_range() {
        for k in SCALE_EXPONENT_MIN..=SCALE_EXPONENT_MAX {
            let e = ScaleExponent::new(k).unwrap();
            assert_eq!(e.get(), k);
            assert_eq!(e.table_index(), (k as i32 + 63) as usize);
            assert!(e.table_index() < POW2_EXPONENT_LEN);
        }
    }

    #[test]
    fn exponent_rejects_out_of_range_both_sides() {
        for raw in [-64i8, i8::MIN, 64, i8::MAX] {
            let err = ScaleExponent::new(raw).unwrap_err();
            assert_eq!(err, Error::ScaleExponentOutOfRange { got: raw });
        }
    }

    #[test]
    #[should_panic(expected = "ScaleExponent::new_const")]
    fn exponent_new_const_panics_above_range() {
        let _ = ScaleExponent::new_const(64);
    }

    #[test]
    #[should_panic(expected = "ScaleExponent::new_const")]
    fn exponent_new_const_panics_below_range() {
        let _ = ScaleExponent::new_const(-64);
    }

    #[test]
    fn exponent_new_const_accepts_valid_const_context() {
        const KMIN: ScaleExponent = ScaleExponent::new_const(-63);
        const K0: ScaleExponent = ScaleExponent::new_const(0);
        const KMAX: ScaleExponent = ScaleExponent::new_const(63);
        assert_eq!(KMIN.table_index(), 0);
        assert_eq!(K0.table_index(), SCALE_EXPONENT_BIAS);
        assert_eq!(KMAX.table_index(), POW2_EXPONENT_LEN - 1);
    }

    #[test]
    fn pow2_lookup_is_f32_exact_at_every_exponent() {
        // `.meta` validation: every element exactly equals 2^k.
        for k in SCALE_EXPONENT_MIN..=SCALE_EXPONENT_MAX {
            let got = pow2_scale_for_exponent(ScaleExponent::new(k).unwrap());
            let want = 2f32.powi(k as i32);
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "pow2_scale_for_exponent({k}) got {got} want {want}"
            );
        }
    }

    #[test]
    fn sqrt2_lookup_matches_half_exponent_within_f32_rounding() {
        // `.meta` validation: 2^(k/2) within f32 precision.
        for k in SCALE_EXPONENT_MIN..=SCALE_EXPONENT_MAX {
            let got = sqrt2_scale_for_exponent(ScaleExponent::new(k).unwrap());
            let want = (k as f32 * 0.5).exp2();
            let rel = ((got - want) / want).abs();
            assert!(
                rel < 2e-7,
                "sqrt2_scale_for_exponent({k}) got {got} want {want} rel={rel}"
            );
        }
    }

    #[test]
    fn exponent_zero_is_exactly_one_in_both_ladders() {
        // Element 63 (k = 0) is the 1.0 midpoint of both ladders.
        const K0: ScaleExponent = ScaleExponent::new_const(0);
        assert_eq!(pow2_scale_for_exponent(K0).to_bits(), 1.0f32.to_bits());
        assert_eq!(sqrt2_scale_for_exponent(K0).to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn pow2_subpointer_span_carries_spec01_quoted_endpoints() {
        // spec/01 §6 Round-1 row at `0x92d4`: "exact powers of two,
        // 2^-9 … 2^19". Audit #15 re-bases the row as ladder element 54
        // onward; the quoted endpoint values stay byte-true.
        let first = pow2_scale_for_exponent(POW2_SUBPOINTER_FIRST_EXPONENT);
        assert_eq!(first.to_bits(), 2f32.powi(-9).to_bits());
        let last = pow2_scale_for_exponent(ScaleExponent::new(19).unwrap());
        assert_eq!(last.to_bits(), 2f32.powi(19).to_bits());
    }

    #[test]
    fn sqrt2_subpointer_leading_values_match_spec01_quotes() {
        // spec/01 §6 Round-1 row at `0x94a8`: "0.00138, 0.00195,
        // 0.00276, …". Those are 2^(-19/2), 2^(-18/2), 2^(-17/2) — the
        // ladder from element 44 (audit #15). Pin to the row's printed
        // 3-significant-figure precision.
        let k0 = SQRT2_SUBPOINTER_FIRST_EXPONENT;
        let k1 = ScaleExponent::new(k0.get() + 1).unwrap();
        let k2 = ScaleExponent::new(k0.get() + 2).unwrap();
        let quotes = [
            (sqrt2_scale_for_exponent(k0), 0.00138f32),
            (sqrt2_scale_for_exponent(k1), 0.00195f32),
            (sqrt2_scale_for_exponent(k2), 0.00276f32),
        ];
        for (i, (got, quoted)) in quotes.iter().enumerate() {
            assert!(
                ((got - quoted) / quoted).abs() < 5e-3,
                "sub-pointer element {i}: got {got}, spec/01 §6 quotes {quoted}"
            );
        }
    }

    #[test]
    fn lookups_match_raw_tables_at_every_exponent() {
        let pow2 = pow2_exponent_table();
        let sqrt2 = sqrt2_scale_ladder();
        for k in SCALE_EXPONENT_MIN..=SCALE_EXPONENT_MAX {
            let e = ScaleExponent::new(k).unwrap();
            assert_eq!(
                pow2_scale_for_exponent(e).to_bits(),
                pow2[e.table_index()].to_bits()
            );
            assert_eq!(
                sqrt2_scale_for_exponent(e).to_bits(),
                sqrt2[e.table_index()].to_bits()
            );
        }
    }
}
