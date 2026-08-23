//! MDCT window-builder constants and the runtime-recovered N = 1024
//! long-transform window/twiddle tables (§5 output stage).
//!
//! Source-of-truth: `docs/audio/cook/tables/mdct-window-builder-consts.
//! {csv,meta}` (the `0x8c20` f64 quad the runtime builder
//! `cook.dll!0x3290` consumes), `tables/mdct-window-1024.{csv,meta}` /
//! `tables/mdct-twiddle-{cos,sin}-1024.{csv,meta}` /
//! `tables/mdct-sine-1024.{csv,meta}` (the runtime dump of the built
//! N = 1024 window and rotation tables), and `provenance/06` Ask 2.
//!
//! The decoder builds its apodisation window **at runtime**: nothing in
//! the file image holds the long-transform window values. The five
//! short `.rdata` tables at `0x8d0c` this module once exposed as "MDCT
//! half-windows" are **not windows at all** — round 10
//! (`provenance/10`) pinned them as the §4.3 joint-stereo
//! pan-coefficient tables (exactly one consumer in the image: the §4.2
//! stereo split at `cook.dll!0x3e96`), and they now live in
//! [`crate::coupling`] as [`crate::coupling::coupling_pan_table`]. The
//! transform window is a different object entirely: length `N/2 + 1`
//! (513 for N = 1024), built by `0x3290` from the `{2.0, 0.25, π, 0.5}`
//! quad and the per-flavour transform size.
use crate::tables::mdct_window_builder_consts;

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
    fn category_lut_ends_at_the_pan_table_head() {
        // Audit #14 (re-read under the round-10 relabel): "cat-lut ends
        // exactly at [0x8d0c]" — the 51-entry u32 LUT at 0x8c40 spans
        // exactly up to the §4.3 pan-table head.
        assert_eq!(
            0x8c40 + (crate::tables::CATEGORY_INDEX_LUT_LEN as u32) * 4,
            crate::coupling::COUPLING_PAN_TABLE_RVA
        );
    }

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
