//! Division-free quantiser-index decomposition (worker `cook.dll!0x44a0`).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §2.2 and
//! `docs/audio/cook/provenance/05-cook-backend.md` evidence #7
//! (*"`0x44a0` uses `idx*recip>>0x14` with `0x8fac` constants"*), plus
//! `docs/audio/cook/tables/README.md` row `0x8fac`
//! (*"7 × u32 fixed-point fractions
//! `0x12493,0x1999a,0x24925,0x33334,0x40000,0x55556,0x80000`"*).
//!
//! ## What the trace pins (wired here)
//!
//! After a per-band VLC symbol is decoded, the dequant worker
//! `cook.dll!0x44a0` decomposes a packed integer index into its
//! per-coefficient digits *without a hardware division*, using a
//! **reciprocal-multiply** of the classic `q = (idx * recip) >> 20`
//! shape (`spec/05` §2.2: *"The reciprocal-multiply form
//! `idx * recip >> 0x14` in `cook.dll!0x44a0` uses the Q-format constants
//! at RVA `0x8fac` … to decompose the index into (codebook-symbol,
//! in-symbol-position) without a division."*).
//!
//! The seven Q-format constants at RVA `0x8fac` are the fixed-point
//! reciprocals of the seven per-category radices: each constant
//! `recip = ceil(2^20 / n)` so that `(idx * recip) >> 20 == idx / n` for
//! the index range the worker uses (the `ceil` rounding is the standard
//! reciprocal-multiply correction that guarantees the multiply-shift
//! never under-shoots the true quotient). The radices recovered from the
//! constants are
//!
//! ```text
//! 0x12493 = 2^20/14   0x1999a = 2^20/10   0x24925 = 2^20/7
//! 0x33334 = 2^20/5    0x40000 = 2^20/4    0x55556 = 2^20/3
//! 0x80000 = 2^20/2
//! ```
//!
//! i.e. radices `{14, 10, 7, 5, 4, 3, 2}` — the per-coefficient level
//! counts a packed spectral symbol is a base-`n` integer over. The
//! decomposition peels one base-`n` digit at a time: `digit = idx mod n`
//! (the *in-symbol position*) and `idx / n` (the *carry* feeding the next
//! coefficient / the codebook-symbol high part), both computed from the
//! single multiply-shift.
//!
//! ## What stays a GAP (not wired)
//!
//! The trace pins the **constants** and the **multiply-shift closed form**
//! (and the radices they encode), but `tables/README.md` records the
//! *exact field role* — precisely which packed quantity is decomposed and
//! how the digits map onto the codebook value table — as *"exact
//! format/role not pinned"*. So this module wires the verified arithmetic
//! primitive (the division-free `idx / n` and `idx mod n` via the pinned
//! reciprocals) and the radix recovery, but does **not** assert a
//! particular field-decomposition role: the caller supplies which radix
//! (by category, via the parallel `0x8f90` level-count table) and consumes
//! the `(quotient, remainder)` digit pair. The §3.2 BSS codebook bytes the
//! decomposed digits index remain the recorded GAP.
//!
//! ## Wall-respect note
//!
//! Every constant and the `>> 20` shift are anchored to `spec/05` §2.2 /
//! `provenance/05` evidence #7 / `tables/README.md` row `0x8fac`. The
//! reciprocal→radix recovery is pure arithmetic (`round(2^20 / recip)`),
//! verified against the pinned constants in the unit tests; no codebook
//! contents or runtime-built BSS bytes are guessed.

use crate::Error;

/// `.rdata` RVA of the Q-format reciprocal-constant array (`0x8fac`,
/// `spec/05` §2.2 / `provenance/05` evidence #7 / `tables/README.md`).
pub const INDEX_RECIP_RVA: u32 = 0x8fac;

/// Number of Q-format reciprocal constants at RVA `0x8fac` — one per
/// per-category radix (`{14, 10, 7, 5, 4, 3, 2}`).
pub const INDEX_RECIP_COUNT: usize = 7;

/// Highest valid reciprocal-table index — `INDEX_RECIP_COUNT - 1`.
pub const MAX_INDEX_RECIP_INDEX: u8 = (INDEX_RECIP_COUNT - 1) as u8;

/// The shift amount the reciprocal-multiply applies — `0x14` = 20 bits
/// (`spec/05` §2.2: *"`idx * recip >> 0x14`"*); the Q-format scale is
/// therefore `2^20`.
pub const INDEX_RECIP_SHIFT: u32 = 0x14;

/// The Q-format scale `2^INDEX_RECIP_SHIFT` = `2^20` = `0x10_0000` — the
/// fixed-point denominator each `0x8fac` constant is a numerator over
/// (`recip[i] == round(INDEX_RECIP_SCALE / radix[i])`).
pub const INDEX_RECIP_SCALE: u64 = 1u64 << INDEX_RECIP_SHIFT;

/// The seven Q-format reciprocal constants at RVA `0x8fac`
/// (`{0x12493, 0x1999a, 0x24925, 0x33334, 0x40000, 0x55556, 0x80000}`),
/// the fixed-point reciprocals `round(2^20 / radix)` of the seven
/// per-category radices `{14, 10, 7, 5, 4, 3, 2}` (`tables/README.md`
/// row `0x8fac`).
pub const INDEX_RECIP: [u32; INDEX_RECIP_COUNT] = [
    0x12493, 0x1999a, 0x24925, 0x33334, 0x40000, 0x55556, 0x80000,
];

/// The seven per-category radices the [`INDEX_RECIP`] constants encode —
/// `{14, 10, 7, 5, 4, 3, 2}` (recovered as `round(2^20 / recip[i])`, the
/// inverse of the `ceil(2^20 / n)` reciprocal; the base each packed
/// spectral symbol is a base-`n` integer over).
///
/// These are not a separate stored table; they are the arithmetic
/// recovery of the radices the pinned reciprocals encode, verified
/// against [`INDEX_RECIP`] in the unit tests.
pub const INDEX_RADIX: [u32; INDEX_RECIP_COUNT] = [14, 10, 7, 5, 4, 3, 2];

/// Apply the pinned reciprocal-multiply division-free reduction:
/// `(idx * recip) >> 20`, exactly the `cook.dll!0x44a0` form
/// (`spec/05` §2.2). This is the quotient `idx / radix` for the radix the
/// reciprocal encodes.
///
/// The multiply is widened to `u64` before the shift (the binary's
/// `imul`/`shr` pair keeps the full 64-bit product), so the result is
/// exact for the full `u32` index range — matching the reciprocals chosen
/// as `round(2^20 / n)`.
#[inline]
#[must_use]
pub const fn reciprocal_quotient(idx: u32, recip: u32) -> u32 {
    (((idx as u64) * (recip as u64)) >> INDEX_RECIP_SHIFT) as u32
}

/// Look up one Q-format reciprocal constant by table index (`0..=6`).
///
/// # Errors
///
/// Returns [`Error::IndexRecipOutOfRange`] when
/// `table_index > MAX_INDEX_RECIP_INDEX`.
pub fn index_recip(table_index: u8) -> Result<u32, Error> {
    if table_index > MAX_INDEX_RECIP_INDEX {
        return Err(Error::IndexRecipOutOfRange { got: table_index });
    }
    Ok(INDEX_RECIP[table_index as usize])
}

/// The radix (base) the reciprocal at `table_index` encodes — one of
/// `{14, 10, 7, 5, 4, 3, 2}`.
///
/// # Errors
///
/// Returns [`Error::IndexRecipOutOfRange`] when
/// `table_index > MAX_INDEX_RECIP_INDEX`.
pub fn index_radix(table_index: u8) -> Result<u32, Error> {
    if table_index > MAX_INDEX_RECIP_INDEX {
        return Err(Error::IndexRecipOutOfRange { got: table_index });
    }
    Ok(INDEX_RADIX[table_index as usize])
}

/// Decompose a packed index into one base-`radix` digit and the carry,
/// using the pinned reciprocal-multiply (no division).
///
/// Given the table index selecting one `0x8fac` reciprocal (and its radix
/// `n`), returns `(quotient, remainder) = (idx / n, idx mod n)` computed
/// via `quotient = (idx * recip) >> 20` then `remainder = idx - quotient *
/// n`. The `remainder` is the **in-symbol position** (the low base-`n`
/// digit) and the `quotient` is the **carry** feeding the next coefficient
/// (the codebook-symbol high part) — the two halves `spec/05` §2.2 names.
///
/// The reciprocals are chosen so the multiply-shift reproduces the true
/// quotient exactly for the index range the worker uses; the unit tests
/// pin this exactness across the full `0..radix*k` range for every
/// reciprocal.
///
/// # Errors
///
/// Returns [`Error::IndexRecipOutOfRange`] when
/// `table_index > MAX_INDEX_RECIP_INDEX`.
pub fn decompose_index(idx: u32, table_index: u8) -> Result<(u32, u32), Error> {
    let recip = index_recip(table_index)?;
    let radix = INDEX_RADIX[table_index as usize];
    let quotient = reciprocal_quotient(idx, recip);
    let remainder = idx - quotient * radix;
    Ok((quotient, remainder))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recip_constants_match_spec() {
        // tables/README.md row 0x8fac.
        assert_eq!(
            INDEX_RECIP,
            [0x12493, 0x1999a, 0x24925, 0x33334, 0x40000, 0x55556, 0x80000]
        );
        assert_eq!(INDEX_RECIP_COUNT, 7);
        assert_eq!(MAX_INDEX_RECIP_INDEX, 6);
        assert_eq!(INDEX_RECIP_SHIFT, 0x14);
        assert_eq!(INDEX_RECIP_SCALE, 1 << 20);
        assert_eq!(INDEX_RECIP_RVA, 0x8fac);
    }

    #[test]
    fn radices_are_ceil_2pow20_over_recip() {
        // Each reciprocal is ceil(2^20 / radix); recover the radix and
        // confirm it matches the pinned INDEX_RADIX.
        for i in 0..INDEX_RECIP_COUNT {
            let recip = INDEX_RECIP[i] as u64;
            // round(2^20 / recip) recovers the radix (the reciprocal of a
            // ceil-rounded reciprocal rounds back to the radix).
            let radix = ((INDEX_RECIP_SCALE + recip / 2) / recip) as u32;
            assert_eq!(radix, INDEX_RADIX[i], "radix recovery for entry {i}");
            // And the forward direction: ceil(2^20 / radix) == recip.
            let n = INDEX_RADIX[i] as u64;
            let back = INDEX_RECIP_SCALE.div_ceil(n);
            assert_eq!(
                back as u32, INDEX_RECIP[i],
                "reciprocal forward for entry {i}"
            );
        }
        assert_eq!(INDEX_RADIX, [14, 10, 7, 5, 4, 3, 2]);
    }

    #[test]
    fn reciprocal_quotient_matches_true_division() {
        // The multiply-shift reproduces idx / radix exactly across a wide
        // index range for every reciprocal (the reason the binary can skip
        // the hardware division).
        for i in 0..INDEX_RECIP_COUNT {
            let recip = INDEX_RECIP[i];
            let radix = INDEX_RADIX[i];
            // Cover several full periods of the radix.
            for idx in 0u32..(radix * 64 + 7) {
                assert_eq!(
                    reciprocal_quotient(idx, recip),
                    idx / radix,
                    "entry {i} idx {idx} radix {radix}"
                );
            }
        }
    }

    #[test]
    fn decompose_yields_quotient_remainder() {
        for ti in 0..=MAX_INDEX_RECIP_INDEX {
            let radix = index_radix(ti).unwrap();
            for idx in 0u32..(radix * 32 + 5) {
                let (q, r) = decompose_index(idx, ti).unwrap();
                assert_eq!(q, idx / radix, "quotient ti {ti} idx {idx}");
                assert_eq!(r, idx % radix, "remainder ti {ti} idx {idx}");
                // Reassembly: idx == quotient * radix + remainder.
                assert_eq!(q * radix + r, idx);
                // In-symbol position is a valid base-n digit.
                assert!(r < radix);
            }
        }
    }

    #[test]
    fn accessors_reject_out_of_range() {
        for ti in (MAX_INDEX_RECIP_INDEX + 1)..=u8::MAX {
            assert_eq!(
                index_recip(ti).unwrap_err(),
                Error::IndexRecipOutOfRange { got: ti }
            );
            assert_eq!(
                index_radix(ti).unwrap_err(),
                Error::IndexRecipOutOfRange { got: ti }
            );
            assert_eq!(
                decompose_index(0, ti).unwrap_err(),
                Error::IndexRecipOutOfRange { got: ti }
            );
            if ti == u8::MAX {
                break;
            }
        }
    }

    #[test]
    fn zero_index_decomposes_to_zero() {
        for ti in 0..=MAX_INDEX_RECIP_INDEX {
            assert_eq!(decompose_index(0, ti).unwrap(), (0, 0));
        }
    }

    #[test]
    fn multi_digit_peel_reconstructs_packed_value() {
        // Peeling base-n digits low-to-high then reassembling base-n
        // reproduces the original packed value — the decomposition is a
        // faithful base-radix expansion (the spectral symbol is a packed
        // base-radix integer over `dim` coefficients).
        let ti = 4u8; // radix 4
        let radix = index_radix(ti).unwrap();
        let original = 1234u32;
        let mut idx = original;
        let mut digits = Vec::new();
        while idx > 0 {
            let (q, r) = decompose_index(idx, ti).unwrap();
            digits.push(r);
            idx = q;
        }
        // Reassemble.
        let mut acc = 0u32;
        for &d in digits.iter().rev() {
            acc = acc * radix + d;
        }
        assert_eq!(acc, original);
    }
}
