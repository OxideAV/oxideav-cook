//! Reciprocal averaging-divisor table (typed accessors).
//!
//! Source-of-truth: `docs/audio/cook/tables/reciprocal-1-over-n.{csv,meta}`
//! (RVA `0xa7a8`, 11 × f32 LE; purpose: *"reciprocal table 1/n for
//! n = 1..9, then 1/20 and 0 (averaging / normalisation divisors)"*;
//! validation: *"elements 0..8 equal 1/(n+1) for n in [0,8]; element 9 =
//! 1/20 = 0.05; element 10 = 0.0"*), the spec/01 §6 row at `0xa7a8`
//! (*"`1, 1/2 … 1/9, 1/20, 0`"*; *"the bytes after are separate scalar
//! FP constants"*), and audit point #15 in
//! `docs/audio/cook/provenance/03-cook-audit.md` (Round-1's `0xa7a8`
//! element-count estimate of 14 corrected to **11** reciprocals, the
//! "tail" being separate scalar constants).
//!
//! ## What the binary stores
//!
//! `cook.dll`'s `.rdata` holds an 11-entry f32 table at `0xa7a8` with
//! three structural regions:
//!
//! 1. **Elements 0..=8** — a consecutive `1/n` run for denominators
//!    `n = 1..=9` (element `i` stores `1/(i+1)`).
//! 2. **Element 9** — `1/20`, a non-consecutive stored divisor (the
//!    denominator jumps from 9 to 20).
//! 3. **Element 10** — `0.0`, a stored zero closing the table.
//!
//! The table abuts the standalone FP scalar constants that follow it in
//! `.rdata` (spec/01 §6); its end RVA is derived here, never retyped.
//!
//! ## What this module provides
//!
//! - [`ReciprocalDenominator`] — newtype wrapping a denominator `n` in
//!   `1..=9` (the consecutive run), built by
//!   [`ReciprocalDenominator::new`] (returns
//!   [`crate::Error::ReciprocalDenominatorOutOfRange`] otherwise) or
//!   the panicking [`ReciprocalDenominator::new_const`] for const
//!   contexts.
//! - [`reciprocal_for_denominator`] — the `1/n` lookup over
//!   [`crate::tables::reciprocal_1_over_n`] for the consecutive run.
//! - [`reciprocal_one_twentieth`] — the element-9 `1/20` stored
//!   divisor (its denominator is not adjacent to the run, so it gets a
//!   named accessor instead of a [`ReciprocalDenominator`]).
//! - Named structural constants for the three regions and the derived
//!   table-end RVA.
//!
//! ## What this module does *not* cover
//!
//! The table's runtime consumer is not pinned: the `.meta` purpose line
//! stops at *"averaging / normalisation divisors"* and no spec/01
//! worker is traced to it — a recorded GAP. This module wires the typed
//! table access only.

use crate::{
    tables::{reciprocal_1_over_n, RECIPROCAL_LEN},
    Error,
};

/// RVA of the reciprocal table head
/// (`tables/reciprocal-1-over-n.meta` line `rva: 0xa7a8`).
pub const RECIPROCAL_TABLE_RVA: u32 = 0xa7a8;

/// First RVA past the reciprocal table — derived
/// `0xa7a8 + 11 × 4 = 0xa7d4`, never retyped. The bytes from here on
/// are the separate standalone FP scalar constants spec/01 §6 notes
/// after the `0xa7a8` row.
pub const RECIPROCAL_TABLE_END_RVA: u32 = RECIPROCAL_TABLE_RVA + (RECIPROCAL_LEN as u32) * 4;

/// Length of the consecutive `1/n` run at the head of the table
/// (elements 0..=8 store `1/1 .. 1/9` per the `.meta` validation note).
pub const RECIPROCAL_RUN_LEN: usize = 9;

/// Smallest denominator in the consecutive run (`n = 1`, element 0).
pub const RECIPROCAL_DENOMINATOR_MIN: u8 = 1;

/// Largest denominator in the consecutive run (`n = 9`, element 8).
pub const RECIPROCAL_DENOMINATOR_MAX: u8 = 9;

/// Element index of the stored `1/20` divisor (`.meta`: *"element 9 =
/// 1/20 = 0.05"*) — the denominator jumps from 9 to 20 here, so this
/// entry is outside the consecutive run.
pub const RECIPROCAL_ONE_TWENTIETH_INDEX: usize = 9;

/// Denominator of the element-9 stored divisor (`1/20`).
pub const RECIPROCAL_ONE_TWENTIETH_DENOMINATOR: u32 = 20;

/// Element index of the stored trailing `0.0` (`.meta`: *"element 10 =
/// 0.0"*) — the closing entry of the 11-element table.
pub const RECIPROCAL_TRAILING_ZERO_INDEX: usize = 10;

/// Typed denominator into the consecutive `1/n` run of the reciprocal
/// table, in `RECIPROCAL_DENOMINATOR_MIN..=RECIPROCAL_DENOMINATOR_MAX`
/// (= `1..=9`).
///
/// Built by [`ReciprocalDenominator::new`] (or
/// [`ReciprocalDenominator::new_const`] for const contexts), so
/// consumers never index the run with a denominator the table does not
/// store consecutively (`1/20` lives behind its own named accessor,
/// [`reciprocal_one_twentieth`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReciprocalDenominator(u8);

impl ReciprocalDenominator {
    /// Build a [`ReciprocalDenominator`] from a raw `u8`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ReciprocalDenominatorOutOfRange`] when `raw` is
    /// outside `RECIPROCAL_DENOMINATOR_MIN..=RECIPROCAL_DENOMINATOR_MAX`
    /// (= `1..=9`). Note `20` is also rejected here: the stored `1/20`
    /// is not part of the consecutive run — use
    /// [`reciprocal_one_twentieth`] for it.
    pub fn new(raw: u8) -> Result<Self, Error> {
        if !(RECIPROCAL_DENOMINATOR_MIN..=RECIPROCAL_DENOMINATOR_MAX).contains(&raw) {
            return Err(Error::ReciprocalDenominatorOutOfRange { got: raw });
        }
        Ok(ReciprocalDenominator(raw))
    }

    /// `const`-context constructor. Panics for out-of-range values.
    ///
    /// Use in `const` contexts where the value is a known-valid
    /// compile-time literal; non-const callers should prefer
    /// [`ReciprocalDenominator::new`] and propagate the typed error.
    ///
    /// # Panics
    ///
    /// Panics when `raw` is outside
    /// `RECIPROCAL_DENOMINATOR_MIN..=RECIPROCAL_DENOMINATOR_MAX`.
    pub const fn new_const(raw: u8) -> Self {
        if raw < RECIPROCAL_DENOMINATOR_MIN || raw > RECIPROCAL_DENOMINATOR_MAX {
            panic!("ReciprocalDenominator::new_const: value out of range");
        }
        ReciprocalDenominator(raw)
    }

    /// Raw denominator `n` as `u8`.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Element index into the 11-entry table (`i = n - 1`, always in
    /// `0..RECIPROCAL_RUN_LEN` — element `i` stores `1/(i+1)` per the
    /// `.meta` validation note).
    pub const fn table_index(self) -> usize {
        (self.0 - 1) as usize
    }
}

/// Look up `1/n` in the consecutive run of the 11-entry reciprocal
/// table (`cook.dll!0xa7a8`, `tables/reciprocal-1-over-n.csv`).
///
/// The stored values are the f32-rounded reciprocals (`.meta`
/// validation: *"elements 0..8 equal 1/(n+1) for n in [0,8]"*). The
/// underlying CSV load is `OnceLock`-cached (one parse per process).
pub fn reciprocal_for_denominator(n: ReciprocalDenominator) -> f32 {
    reciprocal_1_over_n()[n.table_index()]
}

/// The stored `1/20` divisor at element
/// [`RECIPROCAL_ONE_TWENTIETH_INDEX`] (`.meta`: *"element 9 = 1/20 =
/// 0.05"*).
///
/// Its denominator (20) is not adjacent to the `1..=9` run, so it is
/// surfaced as a named accessor rather than through
/// [`ReciprocalDenominator`].
pub fn reciprocal_one_twentieth() -> f32 {
    reciprocal_1_over_n()[RECIPROCAL_ONE_TWENTIETH_INDEX]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_meta_and_spec01_row() {
        // `.meta`: 11 elements; the three regions tile the table.
        assert_eq!(
            RECIPROCAL_RUN_LEN + /* 1/20 */ 1 + /* 0.0 */ 1,
            RECIPROCAL_LEN
        );
        assert_eq!(RECIPROCAL_ONE_TWENTIETH_INDEX, RECIPROCAL_RUN_LEN);
        assert_eq!(RECIPROCAL_TRAILING_ZERO_INDEX, RECIPROCAL_LEN - 1);
        // Derived end RVA: 0xa7a8 + 11 × 4 = 0xa7d4 (pure arithmetic).
        assert_eq!(RECIPROCAL_TABLE_END_RVA, 0xa7d4);
        // The run covers exactly denominators 1..=9.
        assert_eq!(
            (RECIPROCAL_DENOMINATOR_MAX - RECIPROCAL_DENOMINATOR_MIN + 1) as usize,
            RECIPROCAL_RUN_LEN
        );
    }

    #[test]
    fn denominator_accepts_full_run() {
        for n in RECIPROCAL_DENOMINATOR_MIN..=RECIPROCAL_DENOMINATOR_MAX {
            let d = ReciprocalDenominator::new(n).unwrap();
            assert_eq!(d.get(), n);
            assert_eq!(d.table_index(), (n - 1) as usize);
            assert!(d.table_index() < RECIPROCAL_RUN_LEN);
        }
    }

    #[test]
    fn denominator_rejects_out_of_run_values() {
        // 0 (would divide-by-zero), 10..=19 (not stored), 20 (stored
        // but NOT part of the consecutive run — named accessor only),
        // and beyond.
        for raw in [0u8, 10, 19, 20, 21, u8::MAX] {
            let err = ReciprocalDenominator::new(raw).unwrap_err();
            assert_eq!(err, Error::ReciprocalDenominatorOutOfRange { got: raw });
        }
    }

    #[test]
    #[should_panic(expected = "ReciprocalDenominator::new_const")]
    fn denominator_new_const_panics_below_range() {
        let _ = ReciprocalDenominator::new_const(0);
    }

    #[test]
    #[should_panic(expected = "ReciprocalDenominator::new_const")]
    fn denominator_new_const_panics_above_range() {
        let _ = ReciprocalDenominator::new_const(10);
    }

    #[test]
    fn denominator_new_const_accepts_valid_const_context() {
        const N1: ReciprocalDenominator = ReciprocalDenominator::new_const(1);
        const N9: ReciprocalDenominator = ReciprocalDenominator::new_const(9);
        assert_eq!(N1.table_index(), 0);
        assert_eq!(N9.table_index(), RECIPROCAL_RUN_LEN - 1);
    }

    #[test]
    fn run_lookup_is_f32_exact_reciprocal_at_every_denominator() {
        // `.meta` validation: elements 0..8 equal 1/(n+1) for n in
        // [0,8] — i.e. element n-1 is the f32-rounded 1/n. f32 division
        // is correctly rounded, so `1.0f32 / n` is bit-identical to the
        // stored value.
        for n in RECIPROCAL_DENOMINATOR_MIN..=RECIPROCAL_DENOMINATOR_MAX {
            let got = reciprocal_for_denominator(ReciprocalDenominator::new(n).unwrap());
            let want = 1.0f32 / (n as f32);
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "reciprocal_for_denominator({n}) got {got} want {want}"
            );
        }
    }

    #[test]
    fn one_twentieth_is_f32_exact() {
        // `.meta` validation: element 9 = 1/20 = 0.05.
        let got = reciprocal_one_twentieth();
        assert_eq!(got.to_bits(), (1.0f32 / 20.0f32).to_bits());
        assert_eq!(got.to_bits(), 0.05f32.to_bits());
    }

    #[test]
    fn trailing_element_is_exactly_zero() {
        // `.meta` validation: element 10 = 0.0.
        let raw = reciprocal_1_over_n();
        assert_eq!(
            raw[RECIPROCAL_TRAILING_ZERO_INDEX].to_bits(),
            0.0f32.to_bits()
        );
    }

    #[test]
    fn lookups_match_raw_table_at_every_position() {
        let raw = reciprocal_1_over_n();
        for n in RECIPROCAL_DENOMINATOR_MIN..=RECIPROCAL_DENOMINATOR_MAX {
            let d = ReciprocalDenominator::new(n).unwrap();
            assert_eq!(
                reciprocal_for_denominator(d).to_bits(),
                raw[d.table_index()].to_bits()
            );
        }
        assert_eq!(
            reciprocal_one_twentieth().to_bits(),
            raw[RECIPROCAL_ONE_TWENTIETH_INDEX].to_bits()
        );
    }
}
