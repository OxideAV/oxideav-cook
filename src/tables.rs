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
//! | [`spectral_codebook_codes`] | 7 rows | per-symbol Huffman codes for the seven spectral VLC codebooks (BSS-recovered, §3.2) |
//! | [`spectral_codebook_code_lengths`] | 7 rows | per-symbol code lengths, pairs with the codes |
//! | [`category_cost_lut`]      |   7 | per-category expected bit-cost `{52,47,43,37,29,22,16}` (§2.2) |
//! | [`transform_rotation_coeffs`] | 74×5 | iMDCT pre/post rotation coefficients (RVA `0xa1b0`) |
//! | [`mdct_window_builder_consts`] | 4 | f64 const inputs `{2.0,0.25,π,0.5}` to the runtime window builder |
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
const CATEGORY_VECTOR_DIM_LO_CSV: &str = include_str!("../tables/category-vector-dim-lo.csv");
const CATEGORY_VECTOR_DIM_HI_CSV: &str = include_str!("../tables/category-vector-dim-hi.csv");
const SPECTRAL_CODEBOOK_DIMS_CSV: &str = include_str!("../tables/spectral-codebook-dims.csv");
const SPECTRAL_CODEBOOK_CODES_CSV: &str = include_str!("../tables/spectral-codebook-codes.csv");
const SPECTRAL_CODEBOOK_CODE_LENGTHS_CSV: &str =
    include_str!("../tables/spectral-codebook-code-lengths.csv");
const CATEGORY_COST_LUT_CSV: &str = include_str!("../tables/category-cost-lut.csv");
const TRANSFORM_ROTATION_COEFFS_CSV: &str = include_str!("../tables/transform-rotation-coeffs.csv");
const MDCT_WINDOW_BUILDER_CONSTS_CSV: &str =
    include_str!("../tables/mdct-window-builder-consts.csv");

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

/// `category_vector_dim_lo` length — 7 u32 (`spec/05 §2.2` /
/// `tables/category-vector-dim-lo.meta`, `element_count: 7`).
pub const CATEGORY_VECTOR_DIM_LEN: usize = 7;

/// `spectral_codebook_dims` length — 7 u32 (`spec/05 §3.1` /
/// `tables/spectral-codebook-dims.meta`, `element_count: 7`).
pub const SPECTRAL_CODEBOOK_DIMS_LEN: usize = 7;

/// Per-codebook symbol counts of the seven spectral Huffman codebooks —
/// `{196, 100, 49, 625, 256, 243, 32}` (`spec/05 §3.1` /
/// `tables/spectral-codebook-dims.meta`). This is the row-length of each
/// codebook in [`spectral_codebook_codes`] / [`spectral_codebook_code_lengths`].
pub const SPECTRAL_CODEBOOK_SYMBOL_COUNTS: [usize; 7] = [196, 100, 49, 625, 256, 243, 32];

/// `category_cost_lut` length — 7 u32 (`spec/05 §2.2` /
/// `tables/category-cost-lut.meta`).
pub const CATEGORY_COST_LUT_LEN: usize = 7;

/// `transform_rotation_coeffs` row count — 74 groups of 5 f32
/// (`tables/transform-rotation-coeffs.meta`, RVA `0xa1b0`).
pub const TRANSFORM_ROTATION_ROW_COUNT: usize = 74;

/// Elements per `transform_rotation_coeffs` row — 5 f32, consumed
/// 5-at-a-time (stride `0x14`) by the iMDCT kernel `cook.dll!0x5b70`.
pub const TRANSFORM_ROTATION_ROW_LEN: usize = 5;

/// `mdct_window_builder_consts` length — 4 f64
/// (`tables/mdct-window-builder-consts.meta`, RVA `0x8c20`).
pub const MDCT_WINDOW_BUILDER_CONSTS_LEN: usize = 4;

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

/// Parse a CSV where each non-empty line is one comma-separated `u32`
/// row, into a fixed-size array of `Vec<u32>` whose row lengths are
/// asserted against `row_lens`.
fn parse_ragged_u32_rows<const N: usize>(csv: &str, row_lens: &[usize; N]) -> [Vec<u32>; N] {
    let mut out: [Vec<u32>; N] = std::array::from_fn(|_| Vec::new());
    let mut row_idx = 0usize;
    for line in csv.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        assert!(row_idx < N, "ragged CSV has more than {N} rows");
        let row: Vec<u32> = line
            .split(',')
            .map(|f| {
                f.trim()
                    .parse::<u32>()
                    .unwrap_or_else(|_| panic!("non-u32 element: {f:?}"))
            })
            .collect();
        assert_eq!(
            row.len(),
            row_lens[row_idx],
            "ragged CSV row {row_idx} expected {} elements, got {}",
            row_lens[row_idx],
            row.len()
        );
        out[row_idx] = row;
        row_idx += 1;
    }
    assert_eq!(row_idx, N, "ragged CSV must hold exactly {N} rows");
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

/// Per-category spectral-vector dimension (low), `{2, 2, 2, 4, 4, 5, 5}`
/// (`tables/category-vector-dim-lo.csv`, 7 u32).
///
/// Read at `[category*4 + 0x9170]` (`spec/05 §2.2`) in the category walk;
/// indexed by gain/quantiser category (0..6). Sets the number of spectral
/// coefficients grouped per VLC symbol on the low branch.
pub fn category_vector_dim_lo() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| {
        parse_u32_table_one_per_line(CATEGORY_VECTOR_DIM_LO_CSV, CATEGORY_VECTOR_DIM_LEN)
    })
}

/// Per-category spectral-vector dimension (high), `{10, 10, 10, 5, 5, 4, 4}`
/// (`tables/category-vector-dim-hi.csv`, 7 u32).
///
/// Read at `[category*4 + 0x918c]` (`spec/05 §2.2`), in parallel with the
/// low-dimension table at `0x9170`; indexed by gain/quantiser category
/// (0..6).
pub fn category_vector_dim_hi() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| {
        parse_u32_table_one_per_line(CATEGORY_VECTOR_DIM_HI_CSV, CATEGORY_VECTOR_DIM_LEN)
    })
}

/// Spectral-VLC codebook symbol counts, `{196, 100, 49, 625, 256, 243, 32}`
/// (`tables/spectral-codebook-dims.csv`, 7 u32).
///
/// One per spectral Huffman codebook (0..6). The third of three parallel
/// arrays at `0x91a8` (value-table pointers) / `0x91c4` (length-table
/// pointers) / `0x91e0` (these counts); the two pointer arrays are
/// relocated into BSS and built at init, so the per-symbol code/length
/// bytes are not in the file image (`spec/05 §3.2` GAP). Only the counts
/// are statically pinnable.
pub fn spectral_codebook_dims() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| {
        parse_u32_table_one_per_line(SPECTRAL_CODEBOOK_DIMS_CSV, SPECTRAL_CODEBOOK_DIMS_LEN)
    })
}

/// Per-symbol Huffman **codes** (bit patterns, right-aligned, read
/// MSB-first) for the seven spectral VLC codebooks
/// (`tables/spectral-codebook-codes.csv`, `spec/05 §3.1` / `§3.2`).
///
/// Returns seven `&[u32]` rows, one per codebook, of lengths
/// [`SPECTRAL_CODEBOOK_SYMBOL_COUNTS`] `{196, 100, 49, 625, 256, 243,
/// 32}`. Pairs with [`spectral_codebook_code_lengths`]: symbol `s` of
/// codebook `b` has code `codes[b][s]` of `lengths[b][s]` bits.
///
/// These are the tables `spec/05 §3.2` recorded as a runtime-built-in-BSS
/// **GAP** (docs-gap #1775) — recovered by dumping the guest BSS the
/// vendor decoder's own `RAInitDecoder` populated
/// (`docs/audio/cook/provenance/06-cook-univdreams-extraction.md`), so the
/// numbers are Feist-clean facts read from the decoder's memory image, not
/// an algorithmic derivation.
pub fn spectral_codebook_codes() -> &'static [Vec<u32>; 7] {
    static T: OnceLock<[Vec<u32>; 7]> = OnceLock::new();
    T.get_or_init(|| {
        parse_ragged_u32_rows(
            SPECTRAL_CODEBOOK_CODES_CSV,
            &SPECTRAL_CODEBOOK_SYMBOL_COUNTS,
        )
    })
}

/// Per-symbol Huffman **code lengths** (bits) for the seven spectral VLC
/// codebooks (`tables/spectral-codebook-code-lengths.csv`, `spec/05 §3.1`
/// / `§3.2`).
///
/// Returns seven `&[u32]` rows, one per codebook, of the same lengths as
/// [`spectral_codebook_codes`]. The per-codebook Kraft sum
/// `Σ 2^-len` is `[1.000427, 1.000275, 1.000031, 1.001938, 1.002167,
/// 1.308594, 1.0]` (`.meta`): codebook 6 is a strict prefix code and
/// codebooks 0–5 carry the Cook escape-style duplicate max-length
/// codewords, so the distinct codewords still form a proper prefix code
/// (verified by the unit tests) while the duplicates are the escape
/// mechanism.
pub fn spectral_codebook_code_lengths() -> &'static [Vec<u32>; 7] {
    static T: OnceLock<[Vec<u32>; 7]> = OnceLock::new();
    T.get_or_init(|| {
        parse_ragged_u32_rows(
            SPECTRAL_CODEBOOK_CODE_LENGTHS_CSV,
            &SPECTRAL_CODEBOOK_SYMBOL_COUNTS,
        )
    })
}

/// Per-category expected bit-cost LUT — `{52, 47, 43, 37, 29, 22, 16}`
/// (`tables/category-cost-lut.csv`, RVA `0x8f38`, `spec/05 §2.2`).
///
/// Read as `[category*4 + 0x8f38]` in the category-assignment /
/// bit-allocation pass `cook.dll!0x4800`: the running frame bit budget is
/// reduced by the assigned category's cost each refinement round until the
/// budget is met. Strictly decreasing across the seven categories.
pub fn category_cost_lut() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| parse_u32_table_one_per_line(CATEGORY_COST_LUT_CSV, CATEGORY_COST_LUT_LEN))
}

/// The iMDCT pre/post rotation-coefficient table
/// (`tables/transform-rotation-coeffs.csv`, RVA `0xa1b0`).
///
/// Returns 74 rows of 5 f32 each (370 total), consumed 5-at-a-time
/// (stride `0x14`) by the iMDCT butterfly kernel `cook.dll!0x5b70`, the
/// group base selected by the block-length class in `cook.dll!0x5b10`.
/// The `.meta` records it as pure read-only `.rdata` const data with **no
/// validated closed form** (columns 0 and 2 are equal in 71/74 groups;
/// values lie in `(-1.575, 1.999)`; not a unit-circle twiddle), so it is
/// vendored as flat Feist facts — the typed table access is wired here,
/// not the kernel's use of it.
pub fn transform_rotation_coeffs() -> &'static [[f32; 5]] {
    static T: OnceLock<Vec<[f32; 5]>> = OnceLock::new();
    T.get_or_init(|| {
        let mut out = Vec::with_capacity(TRANSFORM_ROTATION_ROW_COUNT);
        for line in TRANSFORM_ROTATION_COEFFS_CSV.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let row: Vec<f32> = line
                .split(',')
                .map(|f| {
                    f.trim()
                        .parse::<f32>()
                        .unwrap_or_else(|_| panic!("non-f32 rotation element: {f:?}"))
                })
                .collect();
            assert_eq!(
                row.len(),
                TRANSFORM_ROTATION_ROW_LEN,
                "transform-rotation row must hold {TRANSFORM_ROTATION_ROW_LEN} f32, got {}",
                row.len()
            );
            out.push([row[0], row[1], row[2], row[3], row[4]]);
        }
        assert_eq!(
            out.len(),
            TRANSFORM_ROTATION_ROW_COUNT,
            "transform-rotation-coeffs.csv must hold {TRANSFORM_ROTATION_ROW_COUNT} rows, got {}",
            out.len()
        );
        out
    })
}

/// The four f64 const inputs to the runtime MDCT window/twiddle builder
/// `cook.dll!0x3290` — `{2.0, 0.25, π, 0.5}` (`tables/`
/// `mdct-window-builder-consts.csv`, RVA `0x8c20`).
///
/// These are the constants the builder multiplies/divides by when it
/// computes the full-length sine table, cos/sin rotation twiddles and the
/// sqrt-weighted cosine window at decode time (`provenance/06` Ask 2). The
/// **runtime window/twiddle values themselves stay a GAP** (built lazily
/// at decode time for the per-frame block length, never in the file
/// image); only the const inputs are pinned here.
///
/// The CSV carries a `rva,value,role` header and one row per constant; the
/// loader reads the `value` column (index 1).
pub fn mdct_window_builder_consts() -> [f64; 4] {
    static T: OnceLock<[f64; 4]> = OnceLock::new();
    *T.get_or_init(|| {
        let mut out = Vec::with_capacity(MDCT_WINDOW_BUILDER_CONSTS_LEN);
        for line in MDCT_WINDOW_BUILDER_CONSTS_CSV.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("rva,") {
                continue;
            }
            let value = line
                .split(',')
                .nth(1)
                .expect("window-builder-consts row has a value column");
            out.push(
                value
                    .trim()
                    .parse::<f64>()
                    .unwrap_or_else(|_| panic!("non-f64 window-builder const: {value:?}")),
            );
        }
        assert_eq!(
            out.len(),
            MDCT_WINDOW_BUILDER_CONSTS_LEN,
            "mdct-window-builder-consts.csv must hold {MDCT_WINDOW_BUILDER_CONSTS_LEN} values, got {}",
            out.len()
        );
        [out[0], out[1], out[2], out[3]]
    })
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
        assert_eq!(category_vector_dim_lo().len(), CATEGORY_VECTOR_DIM_LEN);
        assert_eq!(category_vector_dim_hi().len(), CATEGORY_VECTOR_DIM_LEN);
        assert_eq!(spectral_codebook_dims().len(), SPECTRAL_CODEBOOK_DIMS_LEN);
    }

    #[test]
    fn spectral_codebook_dims_are_specced_sequence() {
        // spec/05 §3.1 / .meta: {196, 100, 49, 625, 256, 243, 32}.
        assert_eq!(spectral_codebook_dims(), &[196, 100, 49, 625, 256, 243, 32]);
    }

    #[test]
    fn category_vector_dims_are_specced_sequences() {
        // spec/05 §2.2 / .meta: lo {2,2,2,4,4,5,5}, hi {10,10,10,5,5,4,4}.
        assert_eq!(category_vector_dim_lo(), &[2, 2, 2, 4, 4, 5, 5]);
        assert_eq!(category_vector_dim_hi(), &[10, 10, 10, 5, 5, 4, 4]);
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
    fn spectral_codebook_rows_match_symbol_counts() {
        let codes = spectral_codebook_codes();
        let lens = spectral_codebook_code_lengths();
        for b in 0..7 {
            assert_eq!(
                codes[b].len(),
                SPECTRAL_CODEBOOK_SYMBOL_COUNTS[b],
                "codes cb{b}"
            );
            assert_eq!(
                lens[b].len(),
                SPECTRAL_CODEBOOK_SYMBOL_COUNTS[b],
                "lens cb{b}"
            );
            assert_eq!(codes[b].len() as u32, spectral_codebook_dims()[b]);
        }
    }

    #[test]
    fn spectral_codebook_every_code_fits_its_length() {
        // .meta: "each code < 2^(its code-length)".
        let codes = spectral_codebook_codes();
        let lens = spectral_codebook_code_lengths();
        for b in 0..7 {
            for (s, (&c, &l)) in codes[b].iter().zip(lens[b].iter()).enumerate() {
                assert!(
                    (1..=32).contains(&l),
                    "cb{b} sym{s} length {l} out of range"
                );
                if l < 32 {
                    assert!(c < (1u32 << l), "cb{b} sym{s} code {c} exceeds {l} bits");
                }
            }
        }
    }

    #[test]
    fn spectral_codebook_kraft_sums_match_meta() {
        // .meta Kraft = Σ 2^-len per codebook.
        let want = [
            1.000427f64,
            1.000275,
            1.000031,
            1.001938,
            1.002167,
            1.308594,
            1.0,
        ];
        let lens = spectral_codebook_code_lengths();
        for b in 0..7 {
            let kraft: f64 = lens[b].iter().map(|&l| 2f64.powi(-(l as i32))).sum();
            assert!(
                (kraft - want[b]).abs() < 5e-4,
                "cb{b} Kraft {kraft} vs meta {}",
                want[b]
            );
        }
    }

    #[test]
    fn spectral_codebook_distinct_codewords_form_prefix_code() {
        // .meta: distinct codewords form a proper prefix code; only the
        // max-length escape codewords are duplicated. Verify no distinct
        // codeword is a proper prefix of another (a prefix-free set).
        let codes = spectral_codebook_codes();
        let lens = spectral_codebook_code_lengths();
        for b in 0..7 {
            // Collect distinct (len, code) codewords.
            let mut words: Vec<(u32, u32)> = codes[b]
                .iter()
                .zip(lens[b].iter())
                .map(|(&c, &l)| (l, c))
                .collect();
            words.sort_unstable();
            words.dedup();
            for &(la, ca) in &words {
                for &(lb, cb) in &words {
                    if la < lb {
                        // Is `ca` (la bits) a prefix of `cb` (lb bits)?
                        if (cb >> (lb - la)) == ca {
                            panic!("cb{b}: codeword ({la},{ca}) prefixes ({lb},{cb})");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn category_cost_lut_is_specced_sequence() {
        // spec/05 §2.2 / .meta: {52, 47, 43, 37, 29, 22, 16}, strictly decreasing.
        assert_eq!(category_cost_lut(), &[52, 47, 43, 37, 29, 22, 16]);
        assert_eq!(category_cost_lut().len(), CATEGORY_COST_LUT_LEN);
        for w in category_cost_lut().windows(2) {
            assert!(w[0] > w[1], "cost LUT must be strictly decreasing: {w:?}");
        }
    }

    #[test]
    fn transform_rotation_shape_and_range() {
        let rows = transform_rotation_coeffs();
        assert_eq!(rows.len(), TRANSFORM_ROTATION_ROW_COUNT);
        // .meta: values lie in (-1.575, 1.999).
        for r in rows {
            for &v in r {
                assert!(
                    v > -1.575 && v < 1.999,
                    "rotation value {v} out of documented range"
                );
            }
        }
        // .meta: columns 0 and 2 are equal in 71/74 groups.
        let equal_cols = rows.iter().filter(|r| r[0] == r[2]).count();
        assert_eq!(equal_cols, 71, "expected 71 groups with col0 == col2");
    }

    #[test]
    fn window_builder_consts_match_spec() {
        // .meta: {2.0, 0.25, π, 0.5}.
        let c = mdct_window_builder_consts();
        assert_eq!(c[0], 2.0);
        assert_eq!(c[1], 0.25);
        assert_eq!(c[2], std::f64::consts::PI);
        assert_eq!(c[3], 0.5);
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
