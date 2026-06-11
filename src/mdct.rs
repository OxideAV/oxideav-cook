//! MDCT half-window typed accessors (windowing / overlap-add stage).
//!
//! Source-of-truth: `docs/audio/cook/tables/mdct-windows.{csv,meta}`
//! (RVA `0x8d0c`, 120 × f32 LE; *"five concatenated MDCT
//! analysis/synthesis windows of lengths 3,7,15,31,64; each is a
//! monotone-decreasing half-window with 1/sqrt2 at its midpoint"*;
//! validation: *"windows of length 3/7/15/31 satisfy the
//! Princen-Bradley TDAC identity w[k]^2 + w[N-1-k]^2 = 1 to < 1e-3"*),
//! the `tables/README.md` "mdct-windows row structure" note, and audit
//! point #14 in `docs/audio/cook/provenance/03-cook-audit.md`
//! (boundaries re-verified: *"cat-lut ends exactly at window table
//! `0x8d0c`; windows end at `0x8eec`"*).
//!
//! ## What the binary stores
//!
//! `cook.dll`'s `.rdata` holds 120 consecutive f32 at `0x8d0c`: five
//! concatenated half-windows of lengths **3, 7, 15, 31, 64**. Each is
//! monotone-decreasing with `1/sqrt2 ≈ 0.7071` at its midpoint, and the
//! four shorter windows satisfy the Princen-Bradley TDAC
//! perfect-reconstruction identity — they are the windowing /
//! overlap-add side of the inverse-MDCT stage spec/01 §5.1 inventories
//! (`switching to long transform` / `switching to short transform`).
//! Per the `tables/README.md` row-structure note, *"the window lengths
//! correspond to the codec's per-flavor subband geometry"*.
//!
//! ## What this module provides
//!
//! - [`MdctWindowLength`] — a typed selector over exactly the five
//!   stored half-window lengths, built from a raw length by
//!   [`MdctWindowLength::from_len`] (any other length raises the typed
//!   [`crate::Error::MdctWindowLengthUnsupported`]).
//! - [`mdct_half_window`] — the length-keyed lookup over the vendored
//!   [`crate::tables::mdct_windows`] rows.
//! - Derived per-row positioning: [`MdctWindowLength::element_offset`]
//!   (cumulative element offset of the row inside the 120-element
//!   concatenation) and [`MdctWindowLength::rva`] (the row's address in
//!   the binary, pure RVA arithmetic from the `.meta` table head). The
//!   audit-#14 end boundary `0x8eec` is surfaced as
//!   [`MDCT_WINDOW_TABLE_END_RVA`] (derived, not retyped).
//! - [`MdctWindowLength::tdac_pinned`] — whether the `.meta` validation
//!   note pins the Princen-Bradley TDAC identity for this row (it does
//!   for lengths 3 / 7 / 15 / 31; the 64-row is *not* covered by that
//!   sentence and is deliberately reported `false`).
//!
//! ## What this module does *not* cover
//!
//! Which transform block length / flavor consumes which window — the
//! long/short adaptive switching of spec/01 §5.1 and the per-flavor
//! mapping — is not pinned beyond the row-structure note's
//! "corresponds to the per-flavor subband geometry" sentence, and the
//! inverse-MDCT kernel itself (the `0xa1b0` rotation table consumed by
//! `cook.dll!0x5b70`) is a recorded GAP with **no validated closed
//! form** (audit point #16). This module models the typed window-table
//! access only, not the transform.

use crate::{
    tables::{mdct_windows, MDCT_WINDOWS_TOTAL_LEN, MDCT_WINDOW_ROW_LENS},
    Error,
};

/// Number of stored MDCT half-windows
/// (`tables/mdct-windows.meta`: *"rows: 3,7,15,31,64"*).
pub const MDCT_WINDOW_COUNT: usize = 5;

/// RVA of the MDCT half-window table head
/// (`tables/mdct-windows.meta` line `rva: 0x8d0c`).
pub const MDCT_WINDOW_TABLE_RVA: u32 = 0x8d0c;

/// One-past-the-end RVA of the window table — `0x8d0c + 120 × 4 =
/// 0x8eec`, the boundary audit point #14 re-verified (*"windows end at
/// `0x8eec`"*). Derived, not retyped.
pub const MDCT_WINDOW_TABLE_END_RVA: u32 =
    MDCT_WINDOW_TABLE_RVA + (MDCT_WINDOWS_TOTAL_LEN as u32) * 4;

/// Typed selector over the five stored MDCT half-window lengths.
///
/// Only the five lengths the binary stores are constructible, so
/// consumers can never ask the window table for a geometry it does not
/// hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MdctWindowLength {
    /// The 3-element half-window (row 0).
    L3,
    /// The 7-element half-window (row 1).
    L7,
    /// The 15-element half-window (row 2).
    L15,
    /// The 31-element half-window (row 3).
    L31,
    /// The 64-element half-window (row 4).
    L64,
}

impl MdctWindowLength {
    /// All five stored window lengths, in table (row) order.
    pub const ALL: [MdctWindowLength; MDCT_WINDOW_COUNT] = [
        MdctWindowLength::L3,
        MdctWindowLength::L7,
        MdctWindowLength::L15,
        MdctWindowLength::L31,
        MdctWindowLength::L64,
    ];

    /// Build a [`MdctWindowLength`] from a raw element count.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MdctWindowLengthUnsupported`] for any length
    /// other than the five the binary stores (3 / 7 / 15 / 31 / 64).
    pub fn from_len(len: usize) -> Result<Self, Error> {
        for w in Self::ALL {
            if w.window_len() == len {
                return Ok(w);
            }
        }
        Err(Error::MdctWindowLengthUnsupported { got: len })
    }

    /// Row index of this window inside the five-row concatenation
    /// (table order, `0..MDCT_WINDOW_COUNT`).
    pub const fn row_index(self) -> usize {
        match self {
            MdctWindowLength::L3 => 0,
            MdctWindowLength::L7 => 1,
            MdctWindowLength::L15 => 2,
            MdctWindowLength::L31 => 3,
            MdctWindowLength::L64 => 4,
        }
    }

    /// Element count of this half-window, read from the vendored
    /// per-row lengths (never retyped).
    pub const fn window_len(self) -> usize {
        MDCT_WINDOW_ROW_LENS[self.row_index()]
    }

    /// Cumulative element offset of this row inside the 120-element
    /// concatenation at `0x8d0c` (row starts: 0, 3, 10, 25, 56 —
    /// derived by summing the preceding row lengths).
    pub const fn element_offset(self) -> usize {
        let mut off = 0usize;
        let mut i = 0usize;
        while i < self.row_index() {
            off += MDCT_WINDOW_ROW_LENS[i];
            i += 1;
        }
        off
    }

    /// RVA of this row's first element inside the binary's window
    /// table (pure RVA arithmetic from the `.meta` table head).
    pub const fn rva(self) -> u32 {
        MDCT_WINDOW_TABLE_RVA + (self.element_offset() as u32) * 4
    }

    /// Whether the `.meta` validation note pins the Princen-Bradley
    /// TDAC identity `w[k]^2 + w[N-1-k]^2 = 1` (to < 1e-3) for this
    /// row.
    ///
    /// True for the four shorter windows (3 / 7 / 15 / 31); the
    /// `.meta` sentence does **not** cover the 64-row, so it reports
    /// `false` here even though the value pattern looks similar — the
    /// typed API only asserts what the trace pins.
    pub const fn tdac_pinned(self) -> bool {
        !matches!(self, MdctWindowLength::L64)
    }
}

/// Look up one of the five MDCT half-windows by typed length
/// (`cook.dll!0x8d0c`, `tables/mdct-windows.csv`).
///
/// The returned slice has exactly [`MdctWindowLength::window_len`]
/// elements; it is monotone-decreasing with `1/sqrt2` at its midpoint
/// per the `.meta` purpose note. The underlying CSV load is
/// `OnceLock`-cached (one parse per process).
pub fn mdct_half_window(len: MdctWindowLength) -> &'static [f32] {
    mdct_windows()[len.row_index()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_meta_and_audit_14() {
        // `.meta`: rows 3,7,15,31,64 — five windows, 120 elements.
        assert_eq!(MDCT_WINDOW_COUNT, MDCT_WINDOW_ROW_LENS.len());
        let total: usize = MdctWindowLength::ALL.iter().map(|w| w.window_len()).sum();
        assert_eq!(total, MDCT_WINDOWS_TOTAL_LEN);
        // Audit #14: "windows end at 0x8eec" — derived end boundary.
        assert_eq!(MDCT_WINDOW_TABLE_END_RVA, 0x8eec);
        // Audit #14: "cat-lut ends exactly at window table 0x8d0c" —
        // the 51-entry u32 LUT at 0x8c40 spans exactly up to the
        // window-table head.
        assert_eq!(
            0x8c40 + (crate::tables::CATEGORY_INDEX_LUT_LEN as u32) * 4,
            MDCT_WINDOW_TABLE_RVA
        );
    }

    #[test]
    fn row_offsets_and_rvas_are_cumulative() {
        // Row starts derive from summing preceding row lengths:
        // 0, 3, 10, 25, 56 → RVAs 0x8d0c, 0x8d18, 0x8d34, 0x8d70,
        // 0x8dec.
        let mut expected_off = 0usize;
        for w in MdctWindowLength::ALL {
            assert_eq!(w.element_offset(), expected_off);
            assert_eq!(w.rva(), MDCT_WINDOW_TABLE_RVA + (expected_off as u32) * 4);
            expected_off += w.window_len();
        }
        // The last row runs exactly up to the audit-#14 end boundary.
        let last = MdctWindowLength::L64;
        assert_eq!(
            last.rva() + (last.window_len() as u32) * 4,
            MDCT_WINDOW_TABLE_END_RVA
        );
    }

    #[test]
    fn from_len_roundtrips_all_five_lengths() {
        for w in MdctWindowLength::ALL {
            let rebuilt = MdctWindowLength::from_len(w.window_len()).unwrap();
            assert_eq!(rebuilt, w);
            assert_eq!(mdct_half_window(rebuilt).len(), w.window_len());
        }
    }

    #[test]
    fn from_len_rejects_unsupported_lengths() {
        for got in [0usize, 1, 2, 4, 8, 16, 30, 32, 63, 65, 120, 128, 1024] {
            let err = MdctWindowLength::from_len(got).unwrap_err();
            assert_eq!(err, Error::MdctWindowLengthUnsupported { got });
        }
    }

    #[test]
    fn windows_are_monotone_decreasing_per_meta() {
        // `.meta` purpose: "each is a monotone-decreasing half-window".
        for w in MdctWindowLength::ALL {
            let row = mdct_half_window(w);
            for pair in row.windows(2) {
                assert!(
                    pair[0] >= pair[1],
                    "window len {} not monotone-decreasing at {pair:?}",
                    w.window_len()
                );
            }
        }
    }

    #[test]
    fn windows_carry_one_over_sqrt2_at_midpoint() {
        // `.meta` purpose: "1/sqrt2 at its midpoint". The odd-length
        // rows carry it at the symmetric midpoint w[N/2]; the 64-row
        // carries it at w[N/2 - 1] (the half-window straddles the
        // boundary) — same anchoring as the tables-module check.
        let target = (0.5f32).sqrt();
        for w in MdctWindowLength::ALL {
            let row = mdct_half_window(w);
            let n = row.len();
            let candidate = if n % 2 == 1 {
                row[n / 2]
            } else {
                row[n / 2 - 1]
            };
            assert!(
                (candidate - target).abs() < 5e-3,
                "window len {n}: midpoint {candidate} is not 1/sqrt2"
            );
        }
    }

    #[test]
    fn tdac_identity_holds_exactly_where_meta_pins_it() {
        // `.meta` validation: "windows of length 3/7/15/31 satisfy the
        // Princen-Bradley TDAC identity w[k]^2 + w[N-1-k]^2 = 1 to
        // < 1e-3" — pinned through the typed accessor.
        for w in MdctWindowLength::ALL {
            assert_eq!(w.tdac_pinned(), w != MdctWindowLength::L64);
            if !w.tdac_pinned() {
                continue;
            }
            let row = mdct_half_window(w);
            let n = row.len();
            for k in 0..n / 2 {
                let lhs = row[k] * row[k] + row[n - 1 - k] * row[n - 1 - k];
                assert!(
                    (lhs - 1.0).abs() < 1e-3,
                    "TDAC fail at len={n}, k={k}: {lhs}"
                );
            }
        }
    }

    #[test]
    fn typed_lookup_matches_raw_table_rows() {
        let raw = mdct_windows();
        for w in MdctWindowLength::ALL {
            let typed = mdct_half_window(w);
            let raw_row = raw[w.row_index()];
            assert_eq!(typed.len(), raw_row.len());
            for (a, b) in typed.iter().zip(raw_row.iter()) {
                assert_eq!(a.to_bits(), b.to_bits());
            }
        }
    }
}
