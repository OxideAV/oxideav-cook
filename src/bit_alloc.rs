//! Bit-allocation category-index LUT (structural typed accessor).
//!
//! Source-of-truth: `docs/audio/cook/tables/category-index-lut.csv` /
//! `.meta` (RVA `0x8c40`, 51 × u32 LE; *"monotone-nondecreasing
//! small-integer index LUT (0..19); maps a 51-position axis onto 20
//! categories (category / quantiser-index map)"*) and audit point #14 in
//! `docs/audio/cook/provenance/03-cook-audit.md` (the table is
//! byte-confirmed and bounded — its `.rdata` region ends exactly at the
//! window table `0x8d0c`).
//!
//! ## What the binary stores
//!
//! `cook.dll`'s `.rdata` at RVA `0x8c40` holds a 51-entry `u32 LE` table
//! that increases monotone-nondecreasingly from `0` to `19`. Reading
//! the bytes (extracted byte-for-byte to `tables/category-index-lut.csv`)
//! shows the run-length pattern:
//!
//! ```text
//! 0 1 2 3 4 5 6 7 8 9 10 11 | 11 12 | 12 13 13 | 14 14 14 |
//! 15 15 15 15 | 16 16 16 16 16 | 17 17 17 17 17 17 |
//! 18 18 18 18 18 18 18 | 19 19 19 19 19 19 19 19 19
//! ```
//!
//! — i.e. every output category in `0..=19` is reachable, and the LUT
//! grows fastest at the low end (one position per category) and slowest
//! at the top (nine positions all mapping to category `19`). The "from-
//! `0x8c40`" placement plus the matching `cook.dll!0x8d0c` window-table
//! boundary make this a well-isolated table whose **structural** shape
//! (length, range, monotonicity) is pinned without needing the runtime
//! consumer.
//!
//! ## What this module provides
//!
//! A typed pair of newtypes plus a single lookup:
//!
//! - [`BIT_ALLOC_AXIS_LEN`] — the LUT's input axis size (`51`).
//! - [`BIT_ALLOC_CATEGORY_COUNT`] — the number of distinct output
//!   categories (`20`, exactly the `0..=19` range of the
//!   monotone-nondecreasing range).
//! - [`MAX_BIT_ALLOC_CATEGORY`] — the highest category value (`19`).
//! - [`BitAllocAxisPosition`] — newtype wrapping a `0..=50` axis index.
//!   Built by [`BitAllocAxisPosition::new`] (returns
//!   [`crate::Error::BitAllocAxisOutOfRange`] for `> 50`) or
//!   [`BitAllocAxisPosition::new_const`] for const contexts.
//! - [`BitAllocCategory`] — newtype wrapping a `0..=19` output category.
//!   Cannot be constructed directly by external callers other than via
//!   the LUT lookup (and the panicking [`BitAllocCategory::new_const`]
//!   for const tests) — that way the type carries the invariant that any
//!   value of it is in-range for the table.
//! - [`bit_alloc_category_for_position`] — the LUT lookup itself.
//!
//! ## What this module does *not* cover
//!
//! The exact runtime consumer of the LUT inside the decoder backend is
//! still a `spec/01 §6` GAP — audit point #17 (the `0x8fcc`
//! category-expectation / bit-allocation table the dequant worker
//! `0x4600` reads) plausibly pairs with this LUT to drive a per-band
//! category lookup, but the pairing is *not* statically pinned. This
//! module models the **table** only, not the band-loop or the runtime
//! index source.
//!
//! ## Wall-respect note
//!
//! Every fact this module pins is anchored to the `.meta` and to the
//! corresponding audit point. No algorithmic content beyond the
//! monotone-nondecreasing structural fact is wired here.

use crate::{tables::category_index_lut, Error};

/// Length of the LUT's input axis — `51` positions (audit-pinned at
/// `tables/category-index-lut.meta` line `element_count: 51`).
pub const BIT_ALLOC_AXIS_LEN: usize = 51;

/// Number of distinct output categories — `20` (every value in the
/// `0..=19` range named by the meta's *"maps a 51-position axis onto 20
/// categories"* sentence is reached by the LUT — verified by the
/// [`bit_alloc_lut_covers_full_category_range`](#test) test).
pub const BIT_ALLOC_CATEGORY_COUNT: u8 = 20;

/// Highest valid category value — `BIT_ALLOC_CATEGORY_COUNT - 1`.
pub const MAX_BIT_ALLOC_CATEGORY: u8 = BIT_ALLOC_CATEGORY_COUNT - 1;

/// Highest valid axis position — `BIT_ALLOC_AXIS_LEN as u8 - 1`.
pub const MAX_BIT_ALLOC_AXIS_POSITION: u8 = (BIT_ALLOC_AXIS_LEN as u8) - 1;

/// Typed input position into the bit-allocation category LUT, in
/// `0..=MAX_BIT_ALLOC_AXIS_POSITION`.
///
/// Built by [`BitAllocAxisPosition::new`] (or
/// [`BitAllocAxisPosition::new_const`] for const contexts), so consumers
/// never index the 51-entry LUT with an out-of-range value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitAllocAxisPosition(u8);

impl BitAllocAxisPosition {
    /// Build a [`BitAllocAxisPosition`] from a raw `u8`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::BitAllocAxisOutOfRange`] when
    /// `raw > MAX_BIT_ALLOC_AXIS_POSITION` (i.e. `raw >= 51`).
    pub fn new(raw: u8) -> Result<Self, Error> {
        if raw > MAX_BIT_ALLOC_AXIS_POSITION {
            return Err(Error::BitAllocAxisOutOfRange { got: raw });
        }
        Ok(BitAllocAxisPosition(raw))
    }

    /// `const`-context constructor. Panics for out-of-range values.
    ///
    /// Use in `const` contexts where the value is a known-valid
    /// compile-time literal; non-const callers should prefer
    /// [`BitAllocAxisPosition::new`] and propagate the typed error.
    ///
    /// # Panics
    ///
    /// Panics when `raw > MAX_BIT_ALLOC_AXIS_POSITION`.
    pub const fn new_const(raw: u8) -> Self {
        if raw > MAX_BIT_ALLOC_AXIS_POSITION {
            panic!("BitAllocAxisPosition::new_const: value out of range");
        }
        BitAllocAxisPosition(raw)
    }

    /// Raw axis position as `u8`.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Raw axis position as `usize` — for table lookup.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Typed bit-allocation category value, in `0..=MAX_BIT_ALLOC_CATEGORY`.
///
/// Constructed by [`bit_alloc_category_for_position`] (the LUT lookup)
/// or, in `const` contexts, by [`BitAllocCategory::new_const`]. Carries
/// the invariant that any value of this type is in-range for the LUT's
/// monotone-nondecreasing `0..=19` output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitAllocCategory(u8);

impl BitAllocCategory {
    /// `const`-context constructor. Panics for out-of-range values.
    ///
    /// # Panics
    ///
    /// Panics when `raw > MAX_BIT_ALLOC_CATEGORY`.
    pub const fn new_const(raw: u8) -> Self {
        if raw > MAX_BIT_ALLOC_CATEGORY {
            panic!("BitAllocCategory::new_const: value out of range");
        }
        BitAllocCategory(raw)
    }

    /// Raw category value as `u8`.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Raw category value as `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Look up the bit-allocation category for an axis position.
///
/// Reads `category_index_lut()[pos]` from the vendored 51-entry table
/// (`tables/category-index-lut.csv`) and wraps the result as a typed
/// [`BitAllocCategory`]. The underlying CSV load is `OnceLock`-cached
/// (one parse per process).
pub fn bit_alloc_category_for_position(pos: BitAllocAxisPosition) -> BitAllocCategory {
    let raw = category_index_lut()[pos.as_usize()];
    // The LUT is byte-validated to fit in `0..=19` (extractor's
    // `validation` line + audit point #14); the `as u8` cannot truncate
    // a meaningful bit. Defensive assert keeps the invariant local.
    debug_assert!(
        raw <= MAX_BIT_ALLOC_CATEGORY as u32,
        "category_index_lut[{}] = {raw} out of LUT range",
        pos.as_usize()
    );
    BitAllocCategory(raw as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_meta() {
        // `.meta`: `element_count: 51`, range `0..19` ⇒ 20 categories.
        assert_eq!(BIT_ALLOC_AXIS_LEN, 51);
        assert_eq!(BIT_ALLOC_CATEGORY_COUNT, 20);
        assert_eq!(MAX_BIT_ALLOC_CATEGORY, 19);
        assert_eq!(MAX_BIT_ALLOC_AXIS_POSITION, 50);
        assert_eq!(BIT_ALLOC_AXIS_LEN, category_index_lut().len());
    }

    #[test]
    fn axis_position_accepts_full_range() {
        for p in 0..=MAX_BIT_ALLOC_AXIS_POSITION {
            let pos = BitAllocAxisPosition::new(p).unwrap();
            assert_eq!(pos.get(), p);
            assert_eq!(pos.as_usize(), p as usize);
        }
    }

    #[test]
    fn axis_position_rejects_out_of_range() {
        for p in (MAX_BIT_ALLOC_AXIS_POSITION + 1)..=u8::MAX {
            let err = BitAllocAxisPosition::new(p).unwrap_err();
            assert_eq!(err, Error::BitAllocAxisOutOfRange { got: p });
            if p == u8::MAX {
                break;
            }
        }
        // `51` is the first out-of-range value — pin it explicitly.
        assert_eq!(
            BitAllocAxisPosition::new(51).unwrap_err(),
            Error::BitAllocAxisOutOfRange { got: 51 }
        );
    }

    #[test]
    #[should_panic(expected = "BitAllocAxisPosition::new_const")]
    fn axis_position_new_const_panics_on_out_of_range() {
        let _ = BitAllocAxisPosition::new_const(51);
    }

    #[test]
    fn axis_position_new_const_accepts_valid_const_context() {
        const P0: BitAllocAxisPosition = BitAllocAxisPosition::new_const(0);
        const P50: BitAllocAxisPosition = BitAllocAxisPosition::new_const(50);
        assert_eq!(P0.get(), 0);
        assert_eq!(P50.get(), 50);
    }

    #[test]
    #[should_panic(expected = "BitAllocCategory::new_const")]
    fn category_new_const_panics_on_out_of_range() {
        let _ = BitAllocCategory::new_const(20);
    }

    #[test]
    fn category_new_const_accepts_valid_const_context() {
        const C0: BitAllocCategory = BitAllocCategory::new_const(0);
        const C19: BitAllocCategory = BitAllocCategory::new_const(19);
        assert_eq!(C0.get(), 0);
        assert_eq!(C19.get(), 19);
    }

    #[test]
    fn lookup_matches_raw_table_at_every_position() {
        let raw = category_index_lut();
        for p in 0..=MAX_BIT_ALLOC_AXIS_POSITION {
            let pos = BitAllocAxisPosition::new(p).unwrap();
            let got = bit_alloc_category_for_position(pos);
            assert_eq!(
                got.get() as u32,
                raw[p as usize],
                "LUT lookup at position {p} disagrees with raw table"
            );
            assert!(got.get() <= MAX_BIT_ALLOC_CATEGORY);
        }
    }

    #[test]
    fn lookup_endpoints_match_meta_bounds() {
        // `.meta`: monotone 0..19 over 51 positions; position 0 ⇒ 0,
        // position 50 ⇒ 19 (the table starts at the low bound and is
        // saturated at the top bound by position 50; the
        // `bit_alloc_lut_covers_full_category_range` test below pins
        // that every category in between is reached).
        let p0 = BitAllocAxisPosition::new(0).unwrap();
        let p50 = BitAllocAxisPosition::new(50).unwrap();
        assert_eq!(bit_alloc_category_for_position(p0).get(), 0);
        assert_eq!(bit_alloc_category_for_position(p50).get(), 19);
    }

    #[test]
    fn lookup_is_monotone_non_decreasing_across_full_axis() {
        // `.meta` validation line: *"51 non-decreasing u32 values
        // spanning 0..19"*. Reproduce that property end-to-end through
        // the typed lookup API.
        let mut prev: u8 = 0;
        for p in 0..=MAX_BIT_ALLOC_AXIS_POSITION {
            let cat = bit_alloc_category_for_position(BitAllocAxisPosition::new(p).unwrap()).get();
            assert!(
                cat >= prev,
                "LUT not monotone at position {p}: prev={prev}, got={cat}"
            );
            prev = cat;
        }
    }

    #[test]
    fn bit_alloc_lut_covers_full_category_range() {
        // Every category in `0..=MAX_BIT_ALLOC_CATEGORY` must be reached
        // by some axis position — that is what justifies the
        // `BIT_ALLOC_CATEGORY_COUNT = 20` constant (audit-pinned range
        // is the full `0..19`, not a sub-range). If a future
        // table-extraction round changes the LUT in a way that drops a
        // category, this test catches it.
        let mut seen = [false; BIT_ALLOC_CATEGORY_COUNT as usize];
        for p in 0..=MAX_BIT_ALLOC_AXIS_POSITION {
            let cat = bit_alloc_category_for_position(BitAllocAxisPosition::new(p).unwrap()).get();
            seen[cat as usize] = true;
        }
        for (i, &reached) in seen.iter().enumerate() {
            assert!(reached, "category {i} not reached by any axis position");
        }
    }
}
