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
    tables::{
        mdct_window_builder_consts, mdct_windows, MDCT_WINDOWS_TOTAL_LEN, MDCT_WINDOW_ROW_LENS,
    },
    Error,
};

/// `.rdata` base RVA of the runtime MDCT window/twiddle builder's four f64
/// const inputs (`0x8c20`, `provenance/06` Ask 2).
pub const MDCT_WINDOW_BUILDER_CONSTS_RVA: u32 = 0x8c20;

/// The four f64 const inputs to the runtime window/twiddle builder
/// `cook.dll!0x3290`, in stored order — `{2.0, 0.25, π, 0.5}`
/// (`tables/mdct-window-builder-consts.csv`, RVA `0x8c20`).
///
/// `provenance/06` Ask 2 pins these as the constants the builder
/// multiplies/divides by (`2.0` normalisation denominator at `0x8c20`,
/// `0.25` phase constant at `0x8c28`, `π` angle base at `0x8c30`, `0.5`
/// half-sample bias at `0x8c38`) when it computes — at **decode** time, for
/// the per-frame block length `N` — the length-`N` sine table, the
/// length-`N/2` cos/sin rotation twiddles, and the length-`N/2+1`
/// sqrt-weighted cosine window into the per-channel heap state.
///
/// The **runtime window/twiddle values themselves stay a GAP**: they are
/// built lazily at decode time and are never in the file image
/// (`provenance/06`: only the short `N = 8` twiddles are built at init,
/// verified as `cos(π/8) = 0.9239`, `sin(π/8) = 0.3827`; the long
/// `N = 1024` window/twiddles need a `RADecode` drive the extractor could
/// not orchestrate). This accessor exposes only the pinned const inputs and
/// the documented build structure — it does **not** reconstruct the
/// builder's (unpinned) formula.
#[must_use]
pub fn window_builder_consts() -> [f64; 4] {
    mdct_window_builder_consts()
}

/// The window-builder normalisation denominator `2.0` (RVA `0x8c20`).
#[must_use]
pub fn window_builder_denominator() -> f64 {
    window_builder_consts()[0]
}

/// The window-builder phase constant `0.25` (RVA `0x8c28`).
#[must_use]
pub fn window_builder_phase() -> f64 {
    window_builder_consts()[1]
}

/// The window-builder angle base `π` (RVA `0x8c30`).
#[must_use]
pub fn window_builder_pi() -> f64 {
    window_builder_consts()[2]
}

/// The window-builder half-sample bias `0.5` (RVA `0x8c38`).
#[must_use]
pub fn window_builder_half_bias() -> f64 {
    window_builder_consts()[3]
}

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

/// The full `2L`-tap Princen-Bradley window — the mirror completion of
/// the stored half-window.
///
/// Derivation (no numbers are invented):
///
/// - `tables/mdct-windows.meta` names each stored row an *"MDCT
///   analysis/synthesis window"* that is a **monotone-decreasing
///   half-window** — i.e. the *falling* half of the full window (a full
///   Princen-Bradley window rises, peaks, then falls; a decreasing run
///   of length `L` is its second half).
/// - The Princen-Bradley family the `.meta` names is **symmetric**
///   (`W[k] = W[2L-1-k]`), so the rising half is the stored half read in
///   reverse — the unique symmetric completion:
///   `W[k] = w[L-1-k]` for `k < L` and `W[L+k] = w[k]` for `k < L`.
/// - The overlap-add perfect-reconstruction identity across the hop,
///   `W[k]^2 + W[k+L]^2 = 1`, then follows **directly** from the pinned
///   in-row identity `w[k]^2 + w[L-1-k]^2 = 1` (the `.meta` validation
///   note, pinned for rows 3/7/15/31) — no new numeric fact is assumed.
///
/// The result is the window the §5 output stage multiplies each
/// inverse-transform block by before the overlap-add (`spec/05` §5:
/// *"windowed with one of the five Princen-Bradley windows …, and
/// overlap-added into the output"*). Cached per row (one build per
/// process); elements are bit-identical to the vendored half-window
/// values, only re-ordered.
pub fn mdct_full_window(len: MdctWindowLength) -> &'static [f32] {
    use std::sync::OnceLock;
    static FULL: OnceLock<[Vec<f32>; MDCT_WINDOW_COUNT]> = OnceLock::new();
    let rows = FULL.get_or_init(|| {
        let build = |w: MdctWindowLength| -> Vec<f32> {
            let half = mdct_half_window(w);
            // Rising half = mirror of the stored falling half, then the
            // stored falling half verbatim.
            half.iter().rev().chain(half.iter()).copied().collect()
        };
        [
            build(MdctWindowLength::L3),
            build(MdctWindowLength::L7),
            build(MdctWindowLength::L15),
            build(MdctWindowLength::L31),
            build(MdctWindowLength::L64),
        ]
    });
    &rows[len.row_index()]
}

// ---- runtime-recovered long-transform (N = 1024) window --------------

/// The long transform size `N = 1024` the runtime recovery drove the
/// window builder `cook.dll!0x3290` with (decode-state `+0x47bc`;
/// `provenance/06`: *"`+0x06 = 2` drives `N = 1024`"* — the descriptor
/// field is the channel count and `N = 2048 / channels`).
pub const LONG_TRANSFORM_N: usize = 1024;

/// The recovered long-transform apodisation **half-window** — 513 taps
/// (`N/2 + 1`), monotone non-increasing from `≈ 1/√512` to `0`
/// ([`crate::tables::mdct_window_1024`], state `+0x16b08`).
pub fn long_half_window() -> &'static [f32] {
    crate::tables::mdct_window_1024()
}

/// The full 1024-tap long-transform window — the mirror completion of
/// the recovered half-window about its peak:
/// `W[n] = half[|n − 512|]`.
///
/// Derivation (no numbers are invented): the recovered buffer is the
/// **falling half** of the apodisation (monotone from peak to zero,
/// per its `.meta`), so the rising half is the same values read in
/// reverse. Under this completion the hop-512 TDAC sum
/// `W[n]² + W[n+512]² = half[512−n]² + half[n]²` is the **constant
/// `1/512`** to float precision (pinned by a `tables` test) — the
/// Princen-Bradley perfect-reconstruction identity with the vendor's
/// `1/√(N/2)` MDCT normalisation folded into the window.
///
/// Two properties are worth noting for consumers (both observed from
/// the recovered values, not assumed): the window's sample grid is
/// **integer** (its TDAC mirror is about tap 512, not 511.5, so
/// `W[0] = 0` exactly while `W[1023]` is small-but-non-zero), and the
/// folded `1/√512` scale means overlap-adding two windowed blocks
/// reproduces `1/512 ×` the source — use [`long_full_window_unit`]
/// with this crate's unit-normalised [`crate::imlt`] convention.
pub fn long_full_window() -> &'static [f32] {
    use std::sync::OnceLock;
    static FULL: OnceLock<Vec<f32>> = OnceLock::new();
    FULL.get_or_init(|| {
        let half = long_half_window();
        (0..LONG_TRANSFORM_N)
            .map(|n| half[(n as isize - 512).unsigned_abs()])
            .collect()
    })
}

/// The recovered long window rescaled to the **unit TDAC** convention
/// (`W[n]² + W[n+512]² = 1`) of the five stored short windows — the
/// form [`crate::synthesis::Synthesizer`] consumes.
///
/// This is [`long_full_window`] `× √512`: the rescale removes the
/// vendor's folded `1/√(N/2)` MDCT normalisation (this crate's
/// [`crate::imlt`] carries its normalisation inside the transform
/// instead — see the `imlt` module's convention note). The shape is
/// bit-derived from the recovered values; only the documented scalar
/// differs.
pub fn long_full_window_unit() -> &'static [f32] {
    use std::sync::OnceLock;
    static FULL: OnceLock<Vec<f32>> = OnceLock::new();
    FULL.get_or_init(|| {
        let s = (512.0f64).sqrt() as f32;
        long_full_window().iter().map(|&w| w * s).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_builder_consts_named_accessors() {
        // provenance/06 Ask 2 / .meta: {2.0, 0.25, π, 0.5} at 0x8c20..0x8c38.
        assert_eq!(MDCT_WINDOW_BUILDER_CONSTS_RVA, 0x8c20);
        assert_eq!(window_builder_denominator(), 2.0);
        assert_eq!(window_builder_phase(), 0.25);
        assert_eq!(window_builder_pi(), std::f64::consts::PI);
        assert_eq!(window_builder_half_bias(), 0.5);
        assert_eq!(
            window_builder_consts(),
            [
                window_builder_denominator(),
                window_builder_phase(),
                window_builder_pi(),
                window_builder_half_bias()
            ]
        );
    }

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
    fn full_window_is_mirror_completion_of_stored_half() {
        // W[k] = w[L-1-k] (rising), W[L+k] = w[k] (stored falling half
        // verbatim) — bit-identical values, only re-ordered.
        for w in MdctWindowLength::ALL {
            let half = mdct_half_window(w);
            let full = mdct_full_window(w);
            let l = half.len();
            assert_eq!(full.len(), 2 * l);
            for k in 0..l {
                assert_eq!(full[k].to_bits(), half[l - 1 - k].to_bits());
                assert_eq!(full[l + k].to_bits(), half[k].to_bits());
            }
        }
    }

    #[test]
    fn full_window_is_symmetric() {
        // The Princen-Bradley completion is symmetric: W[k] = W[2L-1-k].
        for w in MdctWindowLength::ALL {
            let full = mdct_full_window(w);
            let n = full.len();
            for k in 0..n / 2 {
                assert_eq!(full[k].to_bits(), full[n - 1 - k].to_bits());
            }
        }
    }

    #[test]
    fn full_window_rises_then_falls() {
        // The mirror completion of a monotone-decreasing half rises over
        // the first half and falls over the second.
        for w in MdctWindowLength::ALL {
            let full = mdct_full_window(w);
            let l = full.len() / 2;
            for pair in full[..l].windows(2) {
                assert!(pair[0] <= pair[1], "rising half not monotone");
            }
            for pair in full[l..].windows(2) {
                assert!(pair[0] >= pair[1], "falling half not monotone");
            }
        }
    }

    #[test]
    fn full_window_hop_tdac_follows_from_pinned_in_row_identity() {
        // W[k]^2 + W[k+L]^2 = w[L-1-k]^2 + w[k]^2 = 1 — the hop-shift
        // overlap-add identity is exactly the pinned in-row identity,
        // re-indexed. Assert it through the full-window accessor for the
        // rows whose in-row identity the `.meta` pins (3/7/15/31).
        for w in MdctWindowLength::ALL {
            if !w.tdac_pinned() {
                continue;
            }
            let full = mdct_full_window(w);
            let l = full.len() / 2;
            for k in 0..l {
                let id = full[k] * full[k] + full[k + l] * full[k + l];
                assert!(
                    (id - 1.0).abs() < 1e-3,
                    "hop TDAC fail len={l}, k={k}: {id}"
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

    // ---- recovered N=1024 long window -------------------------------

    #[test]
    fn long_full_window_is_the_mirror_completion() {
        let half = long_half_window();
        let full = long_full_window();
        assert_eq!(full.len(), LONG_TRANSFORM_N);
        // Peak at tap 512; both halves bit-identical to the recovered
        // buffer, only re-ordered.
        for (n, &w) in full.iter().enumerate() {
            let k = (n as isize - 512).unsigned_abs();
            assert_eq!(w.to_bits(), half[k].to_bits(), "tap {n}");
        }
        // half[512] is the vendor's x87 cos(π/2) residue (~2.7e-18),
        // zero to float precision.
        assert!(full[0].abs() < 1e-12, "integer grid: W[0] = half[512] ≈ 0");
        assert_eq!(full[512].to_bits(), half[0].to_bits(), "peak at 512");
    }

    #[test]
    fn long_full_window_unit_has_unit_hop_tdac() {
        // × √512 rescale → W[n]² + W[n+512]² = 1, the same convention
        // as the five stored short windows.
        let w = long_full_window_unit();
        for n in 0..512usize {
            let id = (w[n] as f64).powi(2) + (w[n + 512] as f64).powi(2);
            assert!((id - 1.0).abs() < 1e-5, "unit TDAC fail at {n}: {id}");
        }
        // Peak ≈ 1 (the folded 1/√512 removed).
        assert!((w[512] - 1.0).abs() < 1e-6);
    }
}
