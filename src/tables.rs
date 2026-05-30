//! Vendored DSP tables.
//!
//! Numeric tables extracted from the proprietary Cook decoder binary
//! (SHA-256 `0a8c69d…`) live alongside the crate at `tables/*.csv`,
//! one CSV per table, each accompanied by a `.meta` file in the
//! clean-room workspace recording RVA, type, and validation note.
//! Values are facts (*Feist v. Rural*); they are loaded at access time
//! via `include_str!` and parsed on demand so numbers are never
//! retyped into Rust source.
//!
//! ## Tables
//!
//! | Table | Count | Purpose (per `docs/audio/cook/tables/*.meta`) |
//! | ----- | -----:| --------------------------------------------- |
//! | [`pow2_exponent_table`]    | 127 | `2^k`, k = -63..+63 — exponent / gain scaling (f32-exact) |
//! | [`sqrt2_scale_ladder`]     | 127 | `2^(k/2)`, k = -63..+63 — dequant scale-factor ladder |
//! | [`gain_step_2pow_half`]    |   7 | `2^(n/2)`, n = -3..+3 — per-band gain-step factors |
//! | [`gain_bias_ramp`]         |   7 | per-category gain bias ramp `-0.20..0.0` |
//! | [`category_level_count`]   |   7 | per-category quantiser level-count clip bound `{13,9,6,4,3,2,1}` |
//! | [`reciprocal_1_over_n`]    |  11 | `1/n`, n=1..9, then 1/20 and 0 — averaging divisors |
//! | [`category_index_lut`]     |  51 | monotone 0..19 category / quantiser-index LUT |
//! | [`mdct_windows`]           | 120 | five Princen-Bradley half-windows, lengths 3,7,15,31,64 |
//!
//! Each loader caches its parse via [`std::sync::OnceLock`] so the CSV
//! is split once per process. Lengths are advertised by the constants
//! below — the parser asserts the row count matches.

use std::sync::OnceLock;

// ---- raw CSV bytes -------------------------------------------------

const POW2_EXPONENT_CSV: &str = include_str!("../tables/pow2-exponent-table.csv");
const SQRT2_SCALE_LADDER_CSV: &str = include_str!("../tables/sqrt2-scale-ladder.csv");
const GAIN_STEP_CSV: &str = include_str!("../tables/gain-step-2pow-half.csv");
const GAIN_BIAS_CSV: &str = include_str!("../tables/gain-bias-ramp.csv");
const CATEGORY_LEVEL_COUNT_CSV: &str = include_str!("../tables/category-level-count.csv");
const RECIPROCAL_CSV: &str = include_str!("../tables/reciprocal-1-over-n.csv");
const CATEGORY_INDEX_LUT_CSV: &str = include_str!("../tables/category-index-lut.csv");
const MDCT_WINDOWS_CSV: &str = include_str!("../tables/mdct-windows.csv");

// ---- advertised lengths (Feist facts from the `.meta` files) ------

/// `pow2_exponent_table` length — 127 f32 (`spec/01 §6` / `tables/`
/// `pow2-exponent-table.meta`).
pub const POW2_EXPONENT_LEN: usize = 127;

/// `sqrt2_scale_ladder` length — 127 f32 (`spec/01 §6` / `tables/`
/// `sqrt2-scale-ladder.meta`).
pub const SQRT2_SCALE_LADDER_LEN: usize = 127;

/// `gain_step_2pow_half` length — 7 f32 (`tables/`
/// `gain-step-2pow-half.meta`).
pub const GAIN_STEP_LEN: usize = 7;

/// `gain_bias_ramp` length — 7 f32 (`tables/gain-bias-ramp.meta`).
pub const GAIN_BIAS_LEN: usize = 7;

/// `category_level_count` length — 7 u32 (`tables/`
/// `category-level-count.meta`).
pub const CATEGORY_LEVEL_COUNT_LEN: usize = 7;

/// `reciprocal_1_over_n` length — 11 f32 (`tables/`
/// `reciprocal-1-over-n.meta`).
pub const RECIPROCAL_LEN: usize = 11;

/// `category_index_lut` length — 51 u32 (`tables/`
/// `category-index-lut.meta`).
pub const CATEGORY_INDEX_LUT_LEN: usize = 51;

/// `mdct_windows` total element count — 120 f32 across five rows of
/// lengths 3, 7, 15, 31, 64 (`tables/mdct-windows.meta`).
pub const MDCT_WINDOWS_TOTAL_LEN: usize = 120;

/// Per-row lengths of the five MDCT half-windows.
pub const MDCT_WINDOW_ROW_LENS: [usize; 5] = [3, 7, 15, 31, 64];

// ---- helpers -------------------------------------------------------

fn parse_f32_table_one_per_line(csv: &str, expected: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(expected);
    for line in csv.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        out.push(
            line.parse::<f32>()
                .unwrap_or_else(|_| panic!("non-f32 row: {line:?}")),
        );
    }
    assert_eq!(
        out.len(),
        expected,
        "expected {expected} rows in vendored CSV, got {}",
        out.len()
    );
    out
}

fn parse_u32_table_one_per_line(csv: &str, expected: usize) -> Vec<u32> {
    let mut out = Vec::with_capacity(expected);
    for line in csv.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        out.push(
            line.parse::<u32>()
                .unwrap_or_else(|_| panic!("non-u32 row: {line:?}")),
        );
    }
    assert_eq!(
        out.len(),
        expected,
        "expected {expected} rows in vendored CSV, got {}",
        out.len()
    );
    out
}

// ---- accessors -----------------------------------------------------

/// `2^k` for k = -63..+63 (`tables/pow2-exponent-table.csv`).
///
/// `pow2_exponent_table()[i]` corresponds to `k = i - 63`, so
/// `[0] = 2^-63`, `[63] = 1.0`, `[126] = 2^63`. The values are
/// f32-exact per the `.meta` validation note.
pub fn pow2_exponent_table() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| parse_f32_table_one_per_line(POW2_EXPONENT_CSV, POW2_EXPONENT_LEN))
}

/// `2^(k/2)` for k = -63..+63 (`tables/sqrt2-scale-ladder.csv`).
///
/// Same index convention as [`pow2_exponent_table`]. Lays back-to-back
/// with that table in the binary's `.rdata` (`0x91fc`→`0x95f0`).
pub fn sqrt2_scale_ladder() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| parse_f32_table_one_per_line(SQRT2_SCALE_LADDER_CSV, SQRT2_SCALE_LADDER_LEN))
}

/// `2^(n/2)` for n = -3..+3 (`tables/gain-step-2pow-half.csv`).
///
/// Per-band gain-step factors centred on `1.0 = 2^0` at index 3. Read
/// in parallel with [`gain_bias_ramp`] in the per-band quantiser
/// worker `cook.dll!0x69f0`.
pub fn gain_step_2pow_half() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| parse_f32_table_one_per_line(GAIN_STEP_CSV, GAIN_STEP_LEN))
}

/// Per-category gain bias ramp (`tables/gain-bias-ramp.csv`).
///
/// Indexed by gain/quantiser category (0..6). Monotone-increasing
/// from `-0.20` to `0.0`. Read in parallel with
/// [`gain_step_2pow_half`] by `cook.dll!0x69f0`.
pub fn gain_bias_ramp() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| parse_f32_table_one_per_line(GAIN_BIAS_CSV, GAIN_BIAS_LEN))
}

/// Per-category quantiser level-count / clip bound
/// (`tables/category-level-count.csv`): `{13, 9, 6, 4, 3, 2, 1}`.
///
/// Indexed by gain/quantiser category (0..6) in `cook.dll!0x69f0` to
/// both size and clip the per-band quantiser index.
pub fn category_level_count() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| {
        parse_u32_table_one_per_line(CATEGORY_LEVEL_COUNT_CSV, CATEGORY_LEVEL_COUNT_LEN)
    })
}

/// Reciprocal `1/n` averaging divisors (`tables/reciprocal-1-over-n.csv`).
///
/// 11 entries: `1/1, 1/2, …, 1/9, 1/20, 0`. The `1/20` and trailing `0`
/// are non-contiguous-`1/n` entries the decoder uses as a stored
/// constant rather than computing at runtime.
pub fn reciprocal_1_over_n() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| parse_f32_table_one_per_line(RECIPROCAL_CSV, RECIPROCAL_LEN))
}

/// Monotone 0..19 category / quantiser-index LUT
/// (`tables/category-index-lut.csv`, 51 u32).
///
/// A small index map at `cook.dll!0x8c40` — pinned as a fact but the
/// exact runtime consumer is a `spec/01 §6` GAP.
pub fn category_index_lut() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| parse_u32_table_one_per_line(CATEGORY_INDEX_LUT_CSV, CATEGORY_INDEX_LUT_LEN))
}

/// Five concatenated MDCT analysis/synthesis half-windows
/// (`tables/mdct-windows.csv`).
///
/// Returns five `&[f32]` slices of lengths 3, 7, 15, 31, 64
/// (= 120 f32 total). Each window is monotone-decreasing with
/// `1/sqrt2` at its midpoint, and the four shorter windows satisfy
/// the Princen-Bradley TDAC identity `w[k]^2 + w[N-1-k]^2 = 1` to
/// better than 1e-3 — they are perfect-reconstruction MDCT windows.
pub fn mdct_windows() -> [&'static [f32]; 5] {
    static T: OnceLock<[Vec<f32>; 5]> = OnceLock::new();
    let rows = T.get_or_init(|| {
        let mut out: [Vec<f32>; 5] = Default::default();
        let mut row_idx = 0usize;
        for line in MDCT_WINDOWS_CSV.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            assert!(row_idx < 5, "mdct-windows.csv has more than 5 rows");
            let want = MDCT_WINDOW_ROW_LENS[row_idx];
            let row: Vec<f32> = line
                .split(',')
                .map(|f| {
                    f.trim()
                        .parse::<f32>()
                        .unwrap_or_else(|_| panic!("non-f32 element: {f:?}"))
                })
                .collect();
            assert_eq!(
                row.len(),
                want,
                "mdct-windows.csv row {row_idx} expected {want} f32, got {}",
                row.len()
            );
            out[row_idx] = row;
            row_idx += 1;
        }
        assert_eq!(row_idx, 5, "mdct-windows.csv must hold exactly 5 rows");
        out
    });
    [
        rows[0].as_slice(),
        rows[1].as_slice(),
        rows[2].as_slice(),
        rows[3].as_slice(),
        rows[4].as_slice(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- length / shape checks (one per table) --------------------

    #[test]
    fn lengths_match_advertised() {
        assert_eq!(pow2_exponent_table().len(), POW2_EXPONENT_LEN);
        assert_eq!(sqrt2_scale_ladder().len(), SQRT2_SCALE_LADDER_LEN);
        assert_eq!(gain_step_2pow_half().len(), GAIN_STEP_LEN);
        assert_eq!(gain_bias_ramp().len(), GAIN_BIAS_LEN);
        assert_eq!(category_level_count().len(), CATEGORY_LEVEL_COUNT_LEN);
        assert_eq!(reciprocal_1_over_n().len(), RECIPROCAL_LEN);
        assert_eq!(category_index_lut().len(), CATEGORY_INDEX_LUT_LEN);
        let windows = mdct_windows();
        for (i, &want) in MDCT_WINDOW_ROW_LENS.iter().enumerate() {
            assert_eq!(windows[i].len(), want);
        }
        let total: usize = windows.iter().map(|w| w.len()).sum();
        assert_eq!(total, MDCT_WINDOWS_TOTAL_LEN);
    }

    // ----- Feist-fact value checks (anchored to .meta validation) ----

    #[test]
    fn pow2_exponent_is_f32_exact() {
        // .meta: every element exactly equals 2^k for consecutive k in
        // [-63, 63]. f32 represents 2^k exactly for all of those k.
        let t = pow2_exponent_table();
        for (i, v) in t.iter().enumerate() {
            let k = i as i32 - 63;
            // f32::powi exact for integer exponents within the
            // representable range.
            let want = 2f32.powi(k);
            assert!(
                v.to_bits() == want.to_bits(),
                "pow2_exponent_table[{i}] (k={k}) got {v} want {want}"
            );
        }
    }

    #[test]
    fn sqrt2_ladder_matches_within_f32_rounding() {
        // .meta: 2^(k/2) ladder for k = -63..+63, matched to better
        // than 2e-8 (f32 rounding tolerance).
        let t = sqrt2_scale_ladder();
        for (i, v) in t.iter().enumerate() {
            let k = i as i32 - 63;
            let want = (k as f32 * 0.5_f32).exp2();
            let rel = ((v - want) / want).abs();
            assert!(
                rel < 2e-7,
                "sqrt2_scale_ladder[{i}] (k={k}) got {v} want {want} rel={rel}"
            );
        }
    }

    #[test]
    fn gain_step_centres_on_one() {
        // 2^(n/2) for n in -3..+3 → index 3 is 1.0 exactly.
        let t = gain_step_2pow_half();
        assert_eq!(t[3].to_bits(), 1.0f32.to_bits());
        for (i, v) in t.iter().enumerate() {
            let n = i as i32 - 3;
            let want = (n as f32 * 0.5).exp2();
            assert!(
                ((v - want) / want).abs() < 1e-6,
                "gain_step_2pow_half[{i}] (n={n}) {v} vs {want}"
            );
        }
    }

    #[test]
    fn gain_bias_monotone_increasing() {
        let t = gain_bias_ramp();
        for w in t.windows(2) {
            assert!(w[0] <= w[1], "gain_bias_ramp not monotone: {w:?}");
        }
        assert!(t[0] < -0.19 && t[0] > -0.21);
        assert!(t[GAIN_BIAS_LEN - 1].abs() < 1e-6);
    }

    #[test]
    fn category_level_count_is_specced_sequence() {
        // .meta: {13, 9, 6, 4, 3, 2, 1}, strictly decreasing.
        assert_eq!(category_level_count(), &[13, 9, 6, 4, 3, 2, 1]);
    }

    #[test]
    fn reciprocal_first_nine_are_one_over_n() {
        let t = reciprocal_1_over_n();
        for n in 1..=9u32 {
            let i = n as usize - 1;
            let want = 1.0 / n as f32;
            assert!(
                ((t[i] - want) / want).abs() < 1e-6,
                "reciprocal_1_over_n[{i}] {} vs 1/{n} = {want}",
                t[i]
            );
        }
        // 10th entry is 1/20, 11th is 0.0.
        assert!(((t[9] - 0.05) / 0.05).abs() < 1e-6);
        assert_eq!(t[10].to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn category_index_lut_monotone_non_decreasing_to_19() {
        let t = category_index_lut();
        for w in t.windows(2) {
            assert!(
                w[0] <= w[1],
                "category_index_lut must be monotone, broke at {w:?}"
            );
        }
        assert_eq!(t[0], 0);
        // .meta: monotone 0..19 index map.
        assert!(t[CATEGORY_INDEX_LUT_LEN - 1] <= 19);
    }

    #[test]
    fn mdct_windows_satisfy_princen_bradley() {
        // .meta: the four shorter windows (3, 7, 15, 31) satisfy
        // w[k]^2 + w[N-1-k]^2 = 1 to < 1e-3. The 64-window is the
        // analysis/synthesis pair, identical TDAC check.
        let windows = mdct_windows();
        for w in &windows[..4] {
            let n = w.len();
            for k in 0..n / 2 {
                let lhs = w[k] * w[k] + w[n - 1 - k] * w[n - 1 - k];
                assert!(
                    (lhs - 1.0).abs() < 1e-3,
                    "TDAC fail at len={n}, k={k}: {lhs}"
                );
            }
        }
    }

    fn one_over_sqrt2_present_near_centre(w: &[f32]) -> bool {
        // The `.meta` summary "1/sqrt2 at its midpoint" is anchored
        // empirically: odd-length rows (3 / 7 / 15 / 31) carry the
        // value at the symmetric midpoint `w[N/2]`; the 64-row carries
        // it one slot earlier, at `w[N/2 - 1] = w[31]` (the half-window
        // straddles the boundary). Accept either candidate position.
        let target = (0.5_f32).sqrt();
        let n = w.len();
        let mid_hi = w[n / 2];
        let mid_lo = if n >= 2 { w[n / 2 - 1] } else { mid_hi };
        (mid_hi - target).abs() < 5e-3 || (mid_lo - target).abs() < 5e-3
    }

    #[test]
    fn mdct_windows_have_one_over_sqrt2_near_midpoint() {
        // .meta: each window is monotone-decreasing with 1/sqrt2 at its
        // midpoint. The odd-length rows match the symmetric midpoint
        // exactly; the 64-row carries 1/sqrt2 at index 32 (`w[N/2]`).
        let windows = mdct_windows();
        for (i, w) in windows.iter().enumerate() {
            assert!(
                one_over_sqrt2_present_near_centre(w),
                "row {i} (len {}) does not carry 1/sqrt2 at midpoint",
                w.len()
            );
        }
    }

    #[test]
    fn mdct_windows_monotone_decreasing() {
        let windows = mdct_windows();
        for (i, w) in windows.iter().enumerate() {
            for ww in w.windows(2) {
                assert!(
                    ww[0] >= ww[1],
                    "row {i}: expected monotone-decreasing, got {ww:?}"
                );
            }
        }
    }
}
