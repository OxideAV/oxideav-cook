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
//! | [`coupling_pan_coeffs`]    | 119 | five per-coupling-width §4.3 pan tables, lengths `(1<<w)-1`, `w=2..=6` |
//! | [`frame_read_layout`]      | 9 | observed §0.2 pre-spectral wire field order (widths, reader, call site) |
//! | [`live_frame_params`]      | 3×10 | observed per-frame scalars of three traced real frames |
//! | [`live_frame_allocator_io`] | 3×34 | observed real per-band `v[]` / `cat[]` allocator I/O |
//! | [`spectral_codebook_codes`] | 7 rows | per-symbol Huffman codes for the seven spectral VLC codebooks (BSS-recovered, §3.2) |
//! | [`spectral_codebook_code_lengths`] | 7 rows | per-symbol code lengths, pairs with the codes |
//! | [`category_cost_lut`]      |   7 | per-category expected bit-cost `{52,47,43,37,29,22,16}` (§2.2) |
//! | [`transform_rotation_coeffs`] | 74×5 | iMDCT pre/post rotation coefficients (RVA `0xa1b0`) |
//! | [`mdct_window_builder_consts`] | 4 | f64 const inputs `{2.0,0.25,π,0.5}` to the runtime window builder |
//! | [`mdct_window_1024`]       | 513 | runtime-recovered long-transform (N=1024) apodisation half-window |
//! | [`mdct_twiddle_cos_1024`] / [`mdct_twiddle_sin_1024`] | 512 each | runtime-recovered N=1024 unit-circle rotation twiddles |
//! | [`mdct_sine_1024`]         | 1024 | runtime-recovered sine / pre-rotation table (raw kernel buffer) |
//! | [`coupling_rotation_coeffs`] | 256×2 | runtime-recovered §4.3 joint-stereo `(cos θ, sin θ)` pan pairs |
//! | [`coupling_index_permutation`] | 512 | bit-reversal index into the coupling rotation table |
//! | [`quant_index_reciprocals`] |  7 | Q20 `ceil(2^20/(level_count+1))` digit-split reciprocals (§2.2) |
//! | [`spectral_dequant_scale`] |   8 | dequant magnitude-scale LUT (non-zero idx 5/6/7 = `2^-2.5/2^-2/2^-0.5`) |
//! | [`sign_lut`]               |   2 | spectral sign LUT `{+1, -1}` |
//! | [`category_expectation`]   |  98 | level → reconstructed-magnitude table (7 rows × stride 14) |
//! | [`category_assignment_params`] | 19 | named §2.2 category-assignment algorithm constants + live-frame rows (`cook.dll!0x4800`) |
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
const COUPLING_PAN_COEFFS_CSV: &str = include_str!("../tables/coupling-pan-coeffs.csv");
const FRAME_READ_LAYOUT_CSV: &str = include_str!("../tables/frame-read-layout.csv");
const LIVE_FRAME_PARAMS_CSV: &str = include_str!("../tables/live-frame-params.csv");
const LIVE_FRAME_ALLOCATOR_IO_CSV: &str = include_str!("../tables/live-frame-allocator-io.csv");
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
const MDCT_WINDOW_1024_CSV: &str = include_str!("../tables/mdct-window-1024.csv");
const MDCT_TWIDDLE_COS_1024_CSV: &str = include_str!("../tables/mdct-twiddle-cos-1024.csv");
const MDCT_TWIDDLE_SIN_1024_CSV: &str = include_str!("../tables/mdct-twiddle-sin-1024.csv");
const MDCT_SINE_1024_CSV: &str = include_str!("../tables/mdct-sine-1024.csv");
const COUPLING_ROTATION_COEFFS_CSV: &str = include_str!("../tables/coupling-rotation-coeffs.csv");
const COUPLING_INDEX_PERMUTATION_CSV: &str =
    include_str!("../tables/coupling-index-permutation.csv");
const QUANT_INDEX_RECIPROCALS_CSV: &str = include_str!("../tables/quant-index-reciprocals.csv");
const SPECTRAL_DEQUANT_SCALE_CSV: &str = include_str!("../tables/spectral-dequant-scale.csv");
const SIGN_LUT_CSV: &str = include_str!("../tables/sign-lut.csv");
const CATEGORY_EXPECTATION_CSV: &str = include_str!("../tables/category-expectation.csv");
const CATEGORY_ASSIGNMENT_PARAMS_CSV: &str =
    include_str!("../tables/category-assignment-params.csv");

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

/// `coupling_pan_coeffs` total element count — 119 f32 across five rows
/// of lengths 3, 7, 15, 31, 63 (`tables/coupling-pan-coeffs.meta`; the
/// row extents are read from the `0x8ee8` dispatch pointer array by the
/// extractor, each `(1 << w) - 1` for coupling width `w = 2..=6`).
pub const COUPLING_PAN_TOTAL_LEN: usize = 119;

/// Per-row lengths of the five per-coupling-width pan-coefficient
/// tables — `(1 << w) - 1` for `w = 2..=6`
/// (`tables/coupling-pan-coeffs.meta`).
pub const COUPLING_PAN_ROW_LENS: [usize; 5] = [3, 7, 15, 31, 63];

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

/// `mdct_window_1024` length — 513 f32 = `N/2 + 1` for the long
/// transform `N = 1024` (`tables/mdct-window-1024.meta`, runtime heap
/// buffer at decode-state `+0x16b08`, builder `cook.dll!0x3290`).
pub const MDCT_WINDOW_1024_LEN: usize = 513;

/// `mdct_twiddle_cos_1024` / `mdct_twiddle_sin_1024` length — 512 f32
/// = `N/2` (`tables/mdct-twiddle-{cos,sin}-1024.meta`, state
/// `+0x16b00` / `+0x16b04`).
pub const MDCT_TWIDDLE_1024_LEN: usize = 512;

/// `mdct_sine_1024` length — 1024 f32 = `N`
/// (`tables/mdct-sine-1024.meta`, state `+0x16afc`).
pub const MDCT_SINE_1024_LEN: usize = 1024;

/// `coupling_rotation_coeffs` pair count — 256 `(cos, sin)` pairs
/// (`tables/coupling-rotation-coeffs.meta`, state `+0x47b4`, builder
/// `cook.dll!0x40a0`).
pub const COUPLING_ROTATION_PAIR_COUNT: usize = 256;

/// `coupling_index_permutation` length — 512 u32, matching the
/// recovered coupling width `1 << 9 = 512`
/// (`tables/coupling-index-permutation.meta`, state `+0x47b8`).
pub const COUPLING_INDEX_PERMUTATION_LEN: usize = 512;

/// `quant_index_reciprocals` length — 7 u32
/// (`tables/quant-index-reciprocals.meta`, RVA `0x8fac`).
pub const QUANT_INDEX_RECIPROCALS_LEN: usize = 7;

/// `spectral_dequant_scale` length — 8 f32
/// (`tables/spectral-dequant-scale.meta`, RVA `0x9150`).
pub const SPECTRAL_DEQUANT_SCALE_LEN: usize = 8;

/// `sign_lut` length — 2 f32 `{+1, -1}`
/// (`tables/sign-lut.meta`, RVA `0xa148`).
pub const SIGN_LUT_LEN: usize = 2;

/// `category_expectation` flat length — 98 f32
/// (`tables/category-expectation.meta`, RVA `0x8fc8`, region
/// `0x8fc8..0x9150`).
pub const CATEGORY_EXPECTATION_LEN: usize = 98;

/// `category_assignment_params` data-row count — the 19 named scalar
/// constants of the §2.2 category-assignment algorithm
/// (`tables/category-assignment-params.csv`, algorithm
/// `cook.dll!0x4800`): base `K`, offset start, six bisection steps,
/// divisor, the two clip bounds, the cost-LUT RVA, the category-7 cost,
/// and the refinement-bound context-field offset.
pub const CATEGORY_ASSIGNMENT_PARAMS_ROWS: usize = 19;

/// Row stride of the `category_expectation` table — 14 f32 per
/// category row (= the largest per-category level range,
/// `level_count[0] + 1 = 14`).
///
/// The `.meta` records the 2-D row/column layout as *"not statically
/// unambiguous"*; the stride is pinned **empirically from the staged
/// values themselves**: `98 = 7 × 14`, and under a 14-stride read each
/// row `r` opens with `0.0`, carries exactly `level_count[r]`
/// strictly-increasing non-zero magnitudes (run lengths
/// `{13, 9, 6, 4, 3, 2, 1}` — precisely the seven per-category level
/// counts), and is zero-padded to the stride. The loader asserts that
/// full pattern, so a wrong stride cannot load silently.
pub const CATEGORY_EXPECTATION_STRIDE: usize = 14;

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

/// The runtime-built long-transform (`N = 1024`) MDCT apodisation
/// half-window — 513 f32 (`tables/mdct-window-1024.csv`).
///
/// Built at `RAInitDecoder` by the window builder `cook.dll!0x3290`
/// (x87 `fsin`/`fcos`/`fsqrt` over the `0x8c20` const inputs) into the
/// heap buffer at decode-state `+0x16b08`; recovered by driving the
/// vendor decoder's own init in the univdreams sandbox and dumping the
/// buffer it built (`provenance/06`, ud 0.3.0 `--call` chain). Values
/// are smooth, monotone non-increasing, peak `≈ 1/√512 = 0.04419` (the
/// MDCT `1/√(N/2)` normalisation is folded in), tail → 0 — all
/// asserted at load per the `.meta` validation note.
pub fn mdct_window_1024() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| {
        let t = parse_f32_table_one_per_line(MDCT_WINDOW_1024_CSV, MDCT_WINDOW_1024_LEN);
        // .meta validation: "513 f32, monotone non-increasing, win[-1]=0".
        for w in t.windows(2) {
            assert!(
                w[1] <= w[0],
                "mdct-window-1024 must be monotone non-increasing"
            );
        }
        assert!(
            t[MDCT_WINDOW_1024_LEN - 1].abs() < 1e-12,
            "mdct-window-1024 must end at 0"
        );
        // .meta purpose: peak ~0.0442 = 1/sqrt(512).
        let peak = 1.0f32 / 512f32.sqrt();
        assert!(
            (t[0] - peak).abs() < 1e-6,
            "mdct-window-1024 peak {} must be ~1/sqrt(512) = {peak}",
            t[0]
        );
        t
    })
}

/// The runtime-built `N = 1024` MDCT **cos** rotation twiddles — 512
/// f32 (`tables/mdct-twiddle-cos-1024.csv`, state `+0x16b00`).
///
/// Unit-circle paired with [`mdct_twiddle_sin_1024`]; the loader
/// asserts `cos² + sin² = 1` to the `.meta`'s `< 1e-4` bound over all
/// 512 entries. Their consumption by the fast iMDCT kernel
/// `cook.dll!0x5b70` stays the recorded no-closed-form GAP (audit #16).
pub fn mdct_twiddle_cos_1024() -> &'static [f32] {
    twiddles_1024().0
}

/// The runtime-built `N = 1024` MDCT **sin** rotation twiddles — 512
/// f32 (`tables/mdct-twiddle-sin-1024.csv`, state `+0x16b04`). See
/// [`mdct_twiddle_cos_1024`].
pub fn mdct_twiddle_sin_1024() -> &'static [f32] {
    twiddles_1024().1
}

fn twiddles_1024() -> (&'static [f32], &'static [f32]) {
    static T: OnceLock<(Vec<f32>, Vec<f32>)> = OnceLock::new();
    let (c, s) = T.get_or_init(|| {
        let c = parse_f32_table_one_per_line(MDCT_TWIDDLE_COS_1024_CSV, MDCT_TWIDDLE_1024_LEN);
        let s = parse_f32_table_one_per_line(MDCT_TWIDDLE_SIN_1024_CSV, MDCT_TWIDDLE_1024_LEN);
        // .meta validation: cos^2 + sin^2 == 1 to < 1e-4 over all 512.
        for (i, (&cc, &ss)) in c.iter().zip(s.iter()).enumerate() {
            let e = (cc * cc + ss * ss - 1.0).abs();
            assert!(e < 1e-4, "twiddle {i} off the unit circle by {e}");
        }
        (c, s)
    });
    (c.as_slice(), s.as_slice())
}

/// The runtime-built length-1024 sine / pre-rotation table — 1024 f32
/// (`tables/mdct-sine-1024.csv`, state `+0x16afc`).
///
/// Consumed by the iMDCT kernel `cook.dll!0x5b70`; emitted as the raw
/// runtime buffer (Feist facts). The loader asserts the `.meta` bound:
/// 1024 finite f32 in `[-1, 1]`. The buffer's internal ordering follows
/// the vendor kernel's access pattern, which stays the recorded
/// no-closed-form GAP — no per-element law is asserted.
pub fn mdct_sine_1024() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| {
        let t = parse_f32_table_one_per_line(MDCT_SINE_1024_CSV, MDCT_SINE_1024_LEN);
        for (i, v) in t.iter().enumerate() {
            assert!(
                v.is_finite() && (-1.0..=1.0).contains(v),
                "mdct-sine-1024[{i}] = {v} outside [-1, 1]"
            );
        }
        t
    })
}

/// The runtime-built joint-stereo coupling rotation table — 256
/// unit-circle `(cos θ, sin θ)` pairs
/// (`tables/coupling-rotation-coeffs.csv`, state `+0x47b4`, builder
/// `cook.dll!0x40a0`, addressed at decode via `0x8ee8[width]`).
///
/// This is the §4.3 mirror-index pan table of `spec/05` §4.2
/// (`coef[j]` / `coef[Ncoup-1-j]`), recovered from the vendor
/// decoder's own init (`provenance/06`). The loader asserts the
/// `.meta` validation: every pair on the unit circle to `< 1e-4`.
/// Pairs with [`coupling_index_permutation`], which maps a coupling
/// index onto its slot in this table.
pub fn coupling_rotation_coeffs() -> &'static [[f32; 2]] {
    static T: OnceLock<Vec<[f32; 2]>> = OnceLock::new();
    T.get_or_init(|| {
        let mut out = Vec::with_capacity(COUPLING_ROTATION_PAIR_COUNT);
        for line in COUPLING_ROTATION_COEFFS_CSV.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut it = line.split(',').map(|f| {
                f.trim()
                    .parse::<f32>()
                    .unwrap_or_else(|_| panic!("non-f32 coupling element: {f:?}"))
            });
            let c = it.next().expect("coupling row has a cos column");
            let s = it.next().expect("coupling row has a sin column");
            assert!(it.next().is_none(), "coupling row must hold exactly 2 f32");
            // .meta validation: each cos^2 + sin^2 == 1 to < 1e-4.
            let e = (c * c + s * s - 1.0).abs();
            assert!(e < 1e-4, "coupling pair off the unit circle by {e}");
            out.push([c, s]);
        }
        assert_eq!(
            out.len(),
            COUPLING_ROTATION_PAIR_COUNT,
            "coupling-rotation-coeffs.csv must hold {COUPLING_ROTATION_PAIR_COUNT} pairs"
        );
        out
    })
}

/// The runtime-built coupling index permutation — 512 u32
/// (`tables/coupling-index-permutation.csv`, state `+0x47b8`).
///
/// Maps a coupling index onto its rotation-**element** slot in the
/// flattened [`coupling_rotation_coeffs`] buffer (256 pairs = 512
/// f32). The `.meta` pins it as *"a permutation of 0..511 (bit-reversed
/// order: 0, 256, 128, …)"*; the loader asserts the permutation
/// property.
pub fn coupling_index_permutation() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| {
        let t = parse_u32_table_one_per_line(
            COUPLING_INDEX_PERMUTATION_CSV,
            COUPLING_INDEX_PERMUTATION_LEN,
        );
        // .meta validation: a permutation of 0..511.
        let mut seen = [false; COUPLING_INDEX_PERMUTATION_LEN];
        for &v in &t {
            let v = v as usize;
            assert!(
                v < COUPLING_INDEX_PERMUTATION_LEN && !seen[v],
                "coupling-index-permutation is not a permutation of 0..511"
            );
            seen[v] = true;
        }
        t
    })
}

/// Per-category Q20 quantiser-index reciprocals — 7 u32
/// (`tables/quant-index-reciprocals.csv`, RVA `0x8fac`,
/// `spec/05 §2.2`).
///
/// `.meta`: each entry equals `ceil(2^20 / (level_count[cat] + 1))`
/// exactly for the level counts `{13, 9, 6, 4, 3, 2, 1}` — the
/// reciprocal-multiply constants the category walk `cook.dll!0x44a0`
/// uses to split a VLC vector symbol into mixed-radix magnitude digits
/// without a division. The loader asserts that closed form, and the
/// unit tests cross-check the table against the arithmetic-recovered
/// [`crate::index_decomp::INDEX_RECIP`] constants.
pub fn quant_index_reciprocals() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| {
        let t =
            parse_u32_table_one_per_line(QUANT_INDEX_RECIPROCALS_CSV, QUANT_INDEX_RECIPROCALS_LEN);
        for (cat, &recip) in t.iter().enumerate() {
            let base = category_level_count()[cat] + 1;
            let want = (1u32 << 20).div_ceil(base);
            assert_eq!(
                recip, want,
                "quant-index-reciprocals[{cat}] must be ceil(2^20 / {base})"
            );
        }
        t
    })
}

/// The spectral dequant magnitude-scale LUT — 8 f32
/// (`tables/spectral-dequant-scale.csv`, RVA `0x9150`).
///
/// Read at `[sel*4 + 0x9150]` in the spectral dequantiser
/// `cook.dll!0x4600` and multiplied by the sign LUT and the per-band
/// gain (`provenance/07` item 2). `.meta`: indices 0..4 are `0.0`;
/// indices 5/6/7 are `{2^-2.5, 2^-2, 2^-0.5}` — asserted at load. The
/// runtime scale-**selector** semantics stay a recorded gap.
pub fn spectral_dequant_scale() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| {
        let t =
            parse_f32_table_one_per_line(SPECTRAL_DEQUANT_SCALE_CSV, SPECTRAL_DEQUANT_SCALE_LEN);
        for (i, &v) in t.iter().enumerate() {
            if i < 5 {
                assert_eq!(v, 0.0, "spectral-dequant-scale[{i}] must be 0.0");
            } else {
                // 2^-2.5 / 2^-2 / 2^-0.5 for i = 5 / 6 / 7.
                let k = match i {
                    5 => -2.5f32,
                    6 => -2.0,
                    _ => -0.5,
                };
                // The vendor stores these as 6-decimal-digit rounded
                // constants (0.176777 / 0.25 / 0.707107), so they match
                // the .meta's 2^k identities to ~2e-6 relative, not
                // f32-exactly.
                let want = k.exp2();
                assert!(
                    ((v - want) / want).abs() < 1e-5,
                    "spectral-dequant-scale[{i}] {v} must be 2^{k} to stored precision"
                );
            }
        }
        t
    })
}

/// The spectral sign LUT — 2 f32 `{+1.0, -1.0}`
/// (`tables/sign-lut.csv`, RVA `0xa148`).
///
/// One out-of-band sign bit per non-zero coefficient selects from this
/// LUT (`provenance/07` items 2/3: bit `0` → `+1`, bit `1` → `-1`).
/// Asserted at load; the unit tests cross-check it against the
/// spec-quoted [`crate::spectral::SIGN_LUT`] constant.
pub fn sign_lut() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| {
        let t = parse_f32_table_one_per_line(SIGN_LUT_CSV, SIGN_LUT_LEN);
        assert_eq!(t[0], 1.0, "sign-lut[0] must be +1.0");
        assert_eq!(t[1], -1.0, "sign-lut[1] must be -1.0");
        t
    })
}

/// The category-expectation magnitude table — flat 98 f32
/// (`tables/category-expectation.csv`, RVA `0x8fc8`, region
/// `0x8fc8..0x9150`).
///
/// Read at `[level*4 + row_base]` in the spectral dequantiser
/// `cook.dll!0x4600`'s expectation branch to map a quantised level to a
/// reconstructed magnitude (`provenance/07` item 2). Returned in flat
/// RVA order; the typed 2-D `[category][level]` accessor is
/// [`crate::expectation::expectation_magnitude`] (stride
/// [`CATEGORY_EXPECTATION_STRIDE`], empirically pinned — see that
/// constant's docs).
///
/// The staging CSV stores the region as `0.0`-delimited rows: the
/// extractor closes a row at each `0.0` that follows at least one
/// value, **dropping that delimiter zero**. The loader reconstructs
/// the flat 98-value sequence by re-inserting one `0.0` between
/// consecutive rows, then asserts the `.meta` bound (98 finite f32 in
/// `[0, 8)`) and the full stride-14 zero/run pattern described at
/// [`CATEGORY_EXPECTATION_STRIDE`].
pub fn category_expectation() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| {
        let mut flat: Vec<f32> = Vec::with_capacity(CATEGORY_EXPECTATION_LEN);
        for line in CATEGORY_EXPECTATION_CSV.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if !flat.is_empty() {
                // Re-insert the delimiter zero the extractor dropped
                // between rows.
                flat.push(0.0);
            }
            for f in line.split(',') {
                flat.push(
                    f.trim()
                        .parse::<f32>()
                        .unwrap_or_else(|_| panic!("non-f32 expectation element: {f:?}")),
                );
            }
        }
        assert_eq!(
            flat.len(),
            CATEGORY_EXPECTATION_LEN,
            "category-expectation must reconstruct to {CATEGORY_EXPECTATION_LEN} values"
        );
        // .meta validation: 98 finite f32 in [0, 8).
        for (i, v) in flat.iter().enumerate() {
            assert!(
                v.is_finite() && (0.0..8.0).contains(v),
                "category-expectation[{i}] = {v} outside [0, 8)"
            );
        }
        // Empirical stride pin (see CATEGORY_EXPECTATION_STRIDE): row r
        // = [0.0, m_1 < m_2 < … < m_lc, 0.0 pad…] with lc =
        // level_count[r].
        let lc = category_level_count();
        for (r, row) in flat.chunks(CATEGORY_EXPECTATION_STRIDE).enumerate() {
            let run = lc[r] as usize;
            assert_eq!(row[0], 0.0, "expectation row {r} must open with 0.0");
            for i in 1..=run {
                assert!(
                    row[i] > 0.0 && (i == 1 || row[i] > row[i - 1]),
                    "expectation row {r} must carry {run} increasing magnitudes"
                );
            }
            for (i, &v) in row.iter().enumerate().skip(run + 1) {
                assert_eq!(v, 0.0, "expectation row {r} element {i} must be 0.0 pad");
            }
        }
        flat
    })
}

/// Five per-coupling-width joint-stereo pan-coefficient tables
/// (`tables/coupling-pan-coeffs.csv`, RVA `0x8d0c`, spec/05 §4.3).
///
/// Returns five `&[f32]` slices of lengths 3, 7, 15, 31, 63 — one per
/// coupling width `w = 2..=6`, each of length `(1 << w) - 1`, selected
/// at decode through the dispatch pointer array at `0x8ee8` and read as
/// the mirror-index pair `(t[j], t[n-1-j])` of the §4.2 stereo split.
///
/// The loader self-validates the `.meta` invariants: each row is
/// monotone-decreasing with `1/sqrt2` at its centre, and **all 119**
/// values satisfy the constant-power identity
/// `t[j]^2 + t[n-1-j]^2 = 1` to better than `1e-6`.
///
/// This byte range was previously staged as `mdct-windows` (*"five
/// Princen-Bradley window prototypes"*). That label is **withdrawn**:
/// the range has exactly one consumer in the image — the §4.2 stereo
/// split at `cook.dll!0x3e96` — and zero-filling the
/// `coupling_bits`-selected row moves 3060/4096 PCM bytes while the
/// other four rows are bit-inert (the round-9 ablation). The MDCT
/// apodisation window is a different, runtime-built object
/// ([`mdct_window_1024`]).
pub fn coupling_pan_coeffs() -> [&'static [f32]; 5] {
    static T: OnceLock<[Vec<f32>; 5]> = OnceLock::new();
    let rows = T.get_or_init(|| {
        let mut out: [Vec<f32>; 5] = Default::default();
        let mut row_idx = 0usize;
        for line in COUPLING_PAN_COEFFS_CSV.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            assert!(row_idx < 5, "coupling-pan-coeffs.csv has more than 5 rows");
            let want = COUPLING_PAN_ROW_LENS[row_idx];
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
                "coupling-pan-coeffs.csv row {row_idx} expected {want} f32, got {}",
                row.len()
            );
            // .meta invariants: monotone-decreasing, 1/sqrt2 centre,
            // constant-power mirror pairs to < 1e-6.
            for pair in row.windows(2) {
                assert!(
                    pair[0] > pair[1],
                    "coupling-pan-coeffs row {row_idx} must be strictly decreasing"
                );
            }
            let n = row.len();
            let centre = row[n / 2];
            assert!(
                (centre - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
                "coupling-pan-coeffs row {row_idx} centre must be 1/sqrt2, got {centre}"
            );
            for j in 0..n {
                let p = f64::from(row[j]).mul_add(f64::from(row[j]), 0.0)
                    + f64::from(row[n - 1 - j]) * f64::from(row[n - 1 - j]);
                assert!(
                    (p - 1.0).abs() < 1e-6,
                    "coupling-pan-coeffs row {row_idx} pair {j} constant-power identity: {p}"
                );
            }
            out[row_idx] = row;
            row_idx += 1;
        }
        assert_eq!(
            row_idx, 5,
            "coupling-pan-coeffs.csv must hold exactly 5 rows"
        );
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

/// The §2.2 category-assignment algorithm constants
/// (`tables/category-assignment-params.csv`, algorithm
/// `cook.dll!0x4800`), parsed from the vendored named-scalar table.
///
/// The staged table records the behavioural constants of the recovered
/// bit-allocation pass as `(param, value)` rows (values in decimal or
/// `0x`-prefixed hex). This loader parses them on demand so the numbers
/// are never retyped into source; the typed
/// [`crate::category_assignment`] constants are cross-checked against
/// this table by unit tests.
///
/// Panics at first access if a named row is missing — the same
/// fail-loud contract as every other vendored-table loader.
///
/// Two staged rows are **symbolic**, not numeric (`budget_formula` =
/// `bit_limit-bit_cursor`, `index_list_length` = `M-1` — the round-9
/// live-frame identities); they are carried verbatim as
/// [`CategoryAssignmentParam::Symbolic`] so the table stays complete.
pub fn category_assignment_params() -> &'static [(String, CategoryAssignmentParam)] {
    static T: OnceLock<Vec<(String, CategoryAssignmentParam)>> = OnceLock::new();
    T.get_or_init(|| {
        let mut out = Vec::new();
        for (idx, line) in CATEGORY_ASSIGNMENT_PARAMS_CSV.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || idx == 0 {
                // Header row: `param,value,note`.
                continue;
            }
            // Notes may themselves contain commas — split off the first
            // two fields only.
            let mut fields = line.splitn(3, ',');
            let name = fields
                .next()
                .unwrap_or_else(|| panic!("category-assignment-params row {idx}: missing name"));
            let raw = fields
                .next()
                .unwrap_or_else(|| panic!("category-assignment-params row {idx}: missing value"))
                .trim();
            let numeric = if let Some(hex) = raw.strip_prefix("0x") {
                i64::from_str_radix(hex, 16).ok()
            } else if let Some(hex) = raw.strip_prefix("-0x") {
                i64::from_str_radix(hex, 16).ok().map(|v| -v)
            } else {
                raw.parse::<i64>().ok()
            };
            let value = match numeric {
                Some(v) => CategoryAssignmentParam::Integer(v),
                None => CategoryAssignmentParam::Symbolic(raw.to_owned()),
            };
            out.push((name.to_owned(), value));
        }
        assert_eq!(
            out.len(),
            CATEGORY_ASSIGNMENT_PARAMS_ROWS,
            "category-assignment-params.csv must hold {CATEGORY_ASSIGNMENT_PARAMS_ROWS} data rows"
        );
        out
    })
}

/// One value of the vendored [`category_assignment_params`] table: a
/// numeric constant, or one of the two symbolic live-frame identity
/// rows (`budget_formula`, `index_list_length`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryAssignmentParam {
    /// A numeric row (decimal or `0x`-prefixed in the CSV).
    Integer(i64),
    /// A symbolic identity row, carried verbatim.
    Symbolic(String),
}

/// One named §2.2 category-assignment constant from the vendored
/// [`category_assignment_params`] table.
///
/// Panics if `name` is not a row of the table (a programming error —
/// the callers name compile-time-known rows).
#[must_use]
pub fn category_assignment_param(name: &str) -> i64 {
    match &category_assignment_params()
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("category-assignment-params has no row named {name:?}"))
        .1
    {
        CategoryAssignmentParam::Integer(v) => *v,
        CategoryAssignmentParam::Symbolic(s) => {
            panic!("category-assignment-params row {name:?} is symbolic ({s:?}), not numeric")
        }
    }
}

/// The three traced real frames' per-frame scalars
/// (`tables/live-frame-params.csv`, `provenance/09`): container packet,
/// bit limit / bits consumed, 6-bit envelope seed, 7-bit frame scalar,
/// coupling-control mode flag, and the §2.2 allocator's live inputs
/// (`Nb`, budget, `M`) plus the refinement index-list length.
///
/// The loader self-validates the `.meta` budget identity
/// `alloc_budget == bit_limit − bits-at-allocator-call` cannot be
/// checked here (the cursor at the call is not a column), but the
/// pinned `idx_list_len == M − 1` identity is asserted for every row.
pub fn live_frame_params() -> &'static [LiveFrameParams] {
    static T: OnceLock<Vec<LiveFrameParams>> = OnceLock::new();
    T.get_or_init(|| {
        let mut out = Vec::new();
        for (idx, line) in LIVE_FRAME_PARAMS_CSV.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || idx == 0 {
                continue;
            }
            let f: Vec<i64> = line
                .split(',')
                .map(|x| {
                    x.trim()
                        .parse::<i64>()
                        .unwrap_or_else(|_| panic!("live-frame-params row {idx}: non-i64 {x:?}"))
                })
                .collect();
            assert_eq!(
                f.len(),
                10,
                "live-frame-params row {idx} must have 10 columns"
            );
            let row = LiveFrameParams {
                packet: f[0] as u32,
                bit_limit: f[1] as u32,
                bits_consumed: f[2] as u32,
                envelope_seed: f[3] as u32,
                frame_scalar: f[4] as u32,
                coupling_vlc_flag: f[5] as u32,
                band_count: f[6] as u32,
                alloc_budget: f[7] as i32,
                refinement_bound: f[8] as u32,
                idx_list_len: f[9] as u32,
            };
            assert_eq!(
                row.idx_list_len,
                row.refinement_bound - 1,
                "live-frame-params row {idx}: idx_list_len must be M - 1"
            );
            out.push(row);
        }
        assert_eq!(out.len(), 3, "live-frame-params.csv must hold 3 frames");
        out
    })
}

/// One traced real frame's scalars (`tables/live-frame-params.csv`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveFrameParams {
    /// Container packet index in `FUN_RM_32.rm` (2, 16 or 17).
    pub packet: u32,
    /// Bit-reader limit (`+0x47b8`) — 744 bits = the 93-byte sub-packet.
    pub bit_limit: u32,
    /// Bits actually consumed by the frame decode.
    pub bits_consumed: u32,
    /// The 6-bit envelope seed field as the extractor re-derived it from
    /// the frame buffer (see the crate README's read-layout caveat).
    pub envelope_seed: u32,
    /// The 7-bit frame scalar (semantics NOT established).
    pub frame_scalar: u32,
    /// Coupling-control mode flag (0 = fixed-width indices, 1 = VLC).
    pub coupling_vlc_flag: u32,
    /// Band count `Nb` (decode-state `+0x20`) — 34 on all traced frames.
    pub band_count: u32,
    /// The §2.2 allocator's bit budget (`arg_c`) — the number of
    /// bitstream bits still unread at the call.
    pub alloc_budget: i32,
    /// The refinement bound `M` (decode-state `+0x28`) — 128, pinned by
    /// replay.
    pub refinement_bound: u32,
    /// Length of the refinement index output list (`M − 1`).
    pub idx_list_len: u32,
}

/// The three traced real frames' per-band §2.2 allocator I/O
/// (`tables/live-frame-allocator-io.csv`, `provenance/09`): for each
/// container packet (2, 16, 17), the 34-band envelope value array `v[]`
/// the frame body built from the wire and the 34 categories
/// `cook.dll!0x4800` wrote back — both captured off the caller's stack
/// during a real `RADecode`.
pub fn live_frame_allocator_io() -> &'static [LiveFrameAllocatorIo] {
    static T: OnceLock<Vec<LiveFrameAllocatorIo>> = OnceLock::new();
    T.get_or_init(|| {
        let mut frames: Vec<LiveFrameAllocatorIo> = Vec::new();
        for (idx, line) in LIVE_FRAME_ALLOCATOR_IO_CSV.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || idx == 0 {
                continue;
            }
            let f: Vec<i64> = line
                .split(',')
                .map(|x| {
                    x.trim().parse::<i64>().unwrap_or_else(|_| {
                        panic!("live-frame-allocator-io row {idx}: non-i64 {x:?}")
                    })
                })
                .collect();
            assert_eq!(f.len(), 4, "live-frame-allocator-io row {idx}: 4 columns");
            let (packet, band, v, cat) = (f[0] as u32, f[1] as usize, f[2] as i32, f[3]);
            assert!((0..=7).contains(&cat), "row {idx}: category out of range");
            let frame = match frames.iter_mut().find(|fr| fr.packet == packet) {
                Some(fr) => fr,
                None => {
                    frames.push(LiveFrameAllocatorIo {
                        packet,
                        values: Vec::new(),
                        categories: Vec::new(),
                    });
                    frames.last_mut().expect("just pushed")
                }
            };
            assert_eq!(
                frame.values.len(),
                band,
                "row {idx}: bands must be in order"
            );
            frame.values.push(v);
            frame.categories.push(cat as u8);
        }
        assert_eq!(
            frames.len(),
            3,
            "live-frame-allocator-io.csv must hold 3 frames"
        );
        for fr in &frames {
            assert_eq!(fr.values.len(), 34, "packet {}: 34 bands", fr.packet);
        }
        frames
    })
}

/// One traced frame's §2.2 allocator inputs and outputs
/// (`tables/live-frame-allocator-io.csv`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveFrameAllocatorIo {
    /// Container packet index (2, 16 or 17).
    pub packet: u32,
    /// The 34-band envelope value array `v[]` (allocator input).
    pub values: Vec<i32>,
    /// The 34 per-band categories the allocator wrote back.
    pub categories: Vec<u8>,
}

/// The observed §0.2 pre-spectral wire field order
/// (`tables/frame-read-layout.csv`, `provenance/09`): one row per wire
/// field, in read order, carrying the field name, its width spec
/// (fixed bit count, `coupling_bits` or `VLC`), the reader primitive,
/// the call-site RVA and the repeat count expression.
///
/// The rows are descriptive wire-order facts (the wire order itself is
/// wired as code in [`crate::frame`]); this loader keeps the staged
/// table bit-locked to the crate by parsing and shape-checking it.
pub fn frame_read_layout() -> &'static [FrameReadField] {
    static T: OnceLock<Vec<FrameReadField>> = OnceLock::new();
    T.get_or_init(|| {
        let mut out = Vec::new();
        for (idx, line) in FRAME_READ_LAYOUT_CSV.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || idx == 0 {
                continue;
            }
            let mut fields = line.splitn(7, ',');
            let mut next = |what: &str| -> String {
                fields
                    .next()
                    .unwrap_or_else(|| panic!("frame-read-layout row {idx}: missing {what}"))
                    .trim()
                    .to_owned()
            };
            let row = FrameReadField {
                order: next("order"),
                field: next("field"),
                width: next("width"),
                reader: next("reader"),
                call_site_rva: next("call_site_rva"),
                repeat: next("repeat"),
                semantics: next("semantics"),
            };
            out.push(row);
        }
        assert_eq!(out.len(), 9, "frame-read-layout.csv must hold 9 rows");
        out
    })
}

/// One observed wire field of the §0.2 pre-spectral frame read layout
/// (`tables/frame-read-layout.csv`). All columns are carried verbatim
/// as strings — the staged table mixes numeric widths with symbolic
/// ones (`coupling_bits`, `VLC`) and expression repeats (`Ncoupband`,
/// `Nb-1`, `per band`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameReadField {
    /// Wire-order key (`1`..`8`, with `3a`/`3b` for the two coupling
    /// branches).
    pub order: String,
    /// Field name.
    pub field: String,
    /// Width spec: a bit count, `coupling_bits`, or `VLC`.
    pub width: String,
    /// Reader primitive (`read-n-bits 0x3f40`, `read-1-bit 0x3fc0`,
    /// `VLC walk 0x3a50`).
    pub reader: String,
    /// Call-site RVA of the read.
    pub call_site_rva: String,
    /// Repeat count expression.
    pub repeat: String,
    /// Quoted semantics note.
    pub semantics: String,
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
        let pan = coupling_pan_coeffs();
        for (i, &want) in COUPLING_PAN_ROW_LENS.iter().enumerate() {
            assert_eq!(pan[i].len(), want);
        }
        let total: usize = pan.iter().map(|w| w.len()).sum();
        assert_eq!(total, COUPLING_PAN_TOTAL_LEN);
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
    fn coupling_pan_rows_satisfy_constant_power_to_1e6() {
        // .meta: ALL 119 values satisfy t[j]^2 + t[n-1-j]^2 = 1 to
        // < 1e-6 (the round-10 re-verification tightened the old 1e-3
        // claim), every row length is (1 << w) - 1, and each odd-length
        // row carries exactly 1/sqrt2 at its symmetric centre.
        let pan = coupling_pan_coeffs();
        for (i, row) in pan.iter().enumerate() {
            let n = row.len();
            assert_eq!(n, (1usize << (i + 2)) - 1, "row {i} length");
            for j in 0..n {
                let p = f64::from(row[j]) * f64::from(row[j])
                    + f64::from(row[n - 1 - j]) * f64::from(row[n - 1 - j]);
                assert!(
                    (p - 1.0).abs() < 1e-6,
                    "constant-power fail at row {i}, j={j}: {p}"
                );
            }
            let centre = row[n / 2];
            assert!(
                (centre - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
                "row {i} centre {centre} != 1/sqrt2"
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
    fn coupling_pan_rows_strictly_decreasing() {
        let pan = coupling_pan_coeffs();
        for (i, w) in pan.iter().enumerate() {
            for ww in w.windows(2) {
                assert!(
                    ww[0] > ww[1],
                    "row {i}: expected strictly decreasing, got {ww:?}"
                );
            }
        }
    }

    // ----- round-9 live-frame observation tables --------------------

    #[test]
    fn live_frame_params_pin_the_traced_frames() {
        let rows = live_frame_params();
        assert_eq!(rows.len(), 3);
        let packets: Vec<u32> = rows.iter().map(|r| r.packet).collect();
        assert_eq!(packets, [2, 16, 17]);
        for r in rows {
            // One RADecode call decodes one 93-byte sub-packet.
            assert_eq!(r.bit_limit, 744);
            assert!(r.bits_consumed <= r.bit_limit);
            assert_eq!(r.band_count, 34);
            assert_eq!(r.refinement_bound, 128);
            assert_eq!(r.idx_list_len, 127);
            // The budget is the number of unread bits at the allocator
            // call; the pre-spectral read is well over 100 bits, so the
            // budget is strictly inside the frame.
            assert!(r.alloc_budget > 0 && (r.alloc_budget as u32) < r.bit_limit);
        }
        // The .meta cursor identity: budget = bit_limit - cursor, with
        // the recorded cursors 179 / 175 / 177.
        assert_eq!(rows[0].alloc_budget, 744 - 179);
        assert_eq!(rows[1].alloc_budget, 744 - 175);
        assert_eq!(rows[2].alloc_budget, 744 - 177);
    }

    #[test]
    fn live_frame_allocator_io_matches_params_frames() {
        let io = live_frame_allocator_io();
        let params = live_frame_params();
        assert_eq!(io.len(), 3);
        for (fr, pr) in io.iter().zip(params) {
            assert_eq!(fr.packet, pr.packet);
            assert_eq!(fr.values.len(), pr.band_count as usize);
            assert_eq!(fr.categories.len(), pr.band_count as usize);
            // Every traced frame ends on the category-7 empty band.
            assert_eq!(*fr.categories.last().unwrap(), 7);
            // The envelope values are small positive band indices.
            assert!(fr.values.iter().all(|&v| (0..64).contains(&v)));
        }
    }

    #[test]
    fn frame_read_layout_pins_the_wire_order() {
        let rows = frame_read_layout();
        let fields: Vec<&str> = rows.iter().map(|r| r.field.as_str()).collect();
        assert_eq!(
            fields,
            [
                "subpacket_flag",
                "coupling_vlc_flag",
                "coupling_index_fixed",
                "coupling_index_vlc",
                "envelope_seed",
                "envelope_value",
                "frame_scalar_7",
                "--",
                "spectral",
            ]
        );
        // The fixed-width bit counts of the pinned fields.
        assert_eq!(rows[0].width, "1");
        assert_eq!(rows[1].width, "1");
        assert_eq!(rows[2].width, "coupling_bits");
        assert_eq!(rows[3].width, "VLC");
        assert_eq!(rows[4].width, "6");
        assert_eq!(rows[5].width, "VLC");
        assert_eq!(rows[6].width, "7");
        // The allocator row consumes no bits.
        assert_eq!(rows[7].width, "0");
        assert!(rows[7]
            .semantics
            .contains("budget == bit_limit - bit_cursor"));
        // The envelope VLC selects from the 31-entry tree family.
        assert!(rows[5].semantics.contains("31-entry tree array"));
    }

    // ----- runtime-recovered N=1024 DSP tables (round 8 staging) ----

    #[test]
    fn runtime_dsp_lengths_match_advertised() {
        assert_eq!(mdct_window_1024().len(), MDCT_WINDOW_1024_LEN);
        assert_eq!(mdct_twiddle_cos_1024().len(), MDCT_TWIDDLE_1024_LEN);
        assert_eq!(mdct_twiddle_sin_1024().len(), MDCT_TWIDDLE_1024_LEN);
        assert_eq!(mdct_sine_1024().len(), MDCT_SINE_1024_LEN);
        assert_eq!(
            coupling_rotation_coeffs().len(),
            COUPLING_ROTATION_PAIR_COUNT
        );
        assert_eq!(
            coupling_index_permutation().len(),
            COUPLING_INDEX_PERMUTATION_LEN
        );
        assert_eq!(quant_index_reciprocals().len(), QUANT_INDEX_RECIPROCALS_LEN);
        assert_eq!(spectral_dequant_scale().len(), SPECTRAL_DEQUANT_SCALE_LEN);
        assert_eq!(sign_lut().len(), SIGN_LUT_LEN);
        assert_eq!(category_expectation().len(), CATEGORY_EXPECTATION_LEN);
    }

    #[test]
    fn long_window_hop_tdac_is_constant() {
        // The half-window mirror-completes into a 1024-tap window whose
        // hop-512 TDAC sum W[n]² + W[n+512]² is the constant 1/512 (the
        // folded MDCT normalisation): half[k]² + half[512-k]² = 1/512.
        let w = mdct_window_1024();
        let want = 1.0f64 / 512.0;
        for k in 0..=512usize {
            let a = w[k] as f64;
            let b = w[512 - k] as f64;
            let e = ((a * a + b * b) - want).abs() / want;
            assert!(e < 1e-5, "TDAC sum at k={k} off by rel {e}");
        }
    }

    #[test]
    fn long_twiddles_are_the_mdct_rotation_on_the_unit_circle() {
        // The recovered N=1024 pre/post-rotation twiddles are the
        // standard MDCT rotation sampled at θ_k = π·(k + ¼)/N: the k-th
        // (cos, sin) pair. Pinning the vendored buffers to that closed
        // form both validates the recovery and documents the generating
        // identity — analysis on the extracted numbers only.
        let cos = mdct_twiddle_cos_1024();
        let sin = mdct_twiddle_sin_1024();
        let n = MDCT_TWIDDLE_1024_LEN as f64;
        for (k, (&ck, &sk)) in cos.iter().zip(sin.iter()).enumerate() {
            let theta = std::f64::consts::PI * (k as f64 + 0.25) / (2.0 * n);
            // 2N = 1024; the twiddle spans θ = π(k+¼)/1024.
            let wc = theta.cos() as f32;
            let ws = theta.sin() as f32;
            assert!(
                (ck - wc).abs() < 1e-6,
                "cos twiddle[{k}] = {ck} vs cos(π(k+¼)/1024) = {wc}"
            );
            assert!(
                (sk - ws).abs() < 1e-6,
                "sin twiddle[{k}] = {sk} vs sin(π(k+¼)/1024) = {ws}"
            );
            // Unit-circle: cos² + sin² == 1 (the .meta's own check).
            let r = (ck as f64).hypot(sk as f64);
            assert!((r - 1.0).abs() < 1e-4, "twiddle[{k}] not on unit circle");
        }
    }

    #[test]
    fn long_window_is_the_scaled_half_cosine_apodisation() {
        // The recovered N=1024 apodisation half-window is, to f32
        // precision, the scaled half-cosine w[k] = (1/√512)·cos(π·k/1024)
        // (peak 1/√512 at k=0, falling monotonically to 0 at k=512 —
        // exactly the shape the .meta records). Pin the vendored taps to
        // that generating identity; the folded 1/√(N/2) MDCT
        // normalisation is the 1/√512 scale.
        let w = mdct_window_1024();
        let s = 1.0f64 / (512.0f64).sqrt();
        let n2 = 1024.0f64;
        for (k, &wk) in w.iter().enumerate() {
            let want = (s * (std::f64::consts::PI * k as f64 / n2).cos()) as f32;
            assert!(
                (wk - want).abs() < 5e-6,
                "window[{k}] = {wk} vs (1/√512)·cos(π·{k}/1024) = {want}"
            );
        }
        // Endpoints the .meta pins: peak at 0, zero at 512.
        assert!((w[0] as f64 - s).abs() < 1e-6, "window peak must be 1/√512");
        assert!(w[512].abs() < 1e-6, "window must reach 0 at tap 512");
    }

    #[test]
    fn coupling_permutation_is_a_bit_reversal_involution() {
        let p = coupling_index_permutation();
        // .meta: bit-reversed order 0, 256, 128, … — a 9-bit
        // bit-reversal, which is its own inverse.
        for (j, &s) in p.iter().enumerate() {
            let mut r = 0u32;
            let mut x = j as u32;
            for _ in 0..9 {
                r = (r << 1) | (x & 1);
                x >>= 1;
            }
            assert_eq!(s, r, "perm[{j}] must be the 9-bit reversal of {j}");
            assert_eq!(
                p[s as usize] as usize, j,
                "perm must be an involution at {j}"
            );
        }
        assert_eq!(&p[..3], &[0, 256, 128], ".meta leading order 0, 256, 128");
    }

    #[test]
    fn coupling_pairs_start_with_meta_anchors() {
        // .meta validation: first pairs (1,0), (0,1), (1/√2, 1/√2).
        let t = coupling_rotation_coeffs();
        assert_eq!(t[0][0], 1.0);
        assert!(t[0][1].abs() < 1e-9);
        assert!(t[1][0].abs() < 1e-9);
        assert_eq!(t[1][1], 1.0);
        let r = std::f32::consts::FRAC_1_SQRT_2;
        assert!((t[2][0] - r).abs() < 1e-6 && (t[2][1] - r).abs() < 1e-6);
    }

    #[test]
    fn quant_index_reciprocals_match_arithmetic_constants() {
        // Cross-check the vendored 0x8fac bytes against the constants
        // index_decomp recovered by arithmetic before the table was
        // staged — they must agree exactly.
        assert_eq!(
            quant_index_reciprocals(),
            &crate::index_decomp::INDEX_RECIP[..]
        );
    }

    #[test]
    fn sign_lut_matches_spec_quoted_constant() {
        assert_eq!(sign_lut(), &crate::spectral::SIGN_LUT[..]);
    }

    #[test]
    fn dequant_scale_nonzero_matches_spec_quoted_triple() {
        // spec/05 §3.1 quotes the non-zero triple to 5 printed digits;
        // the vendored bytes must agree to that precision.
        let t = spectral_dequant_scale();
        for (i, &q) in crate::spectral::DEQUANT_SCALE_NONZERO.iter().enumerate() {
            assert!(
                (t[5 + i] - q).abs() < 1e-5,
                "dequant scale idx {} = {} vs spec-quoted {q}",
                5 + i,
                t[5 + i]
            );
        }
    }

    #[test]
    fn category_expectation_rows_track_level_counts() {
        // Row r's non-zero run is exactly level_count[r] long — the
        // empirical basis for the stride-14 [category][level] layout.
        let flat = category_expectation();
        let lc = category_level_count();
        for (r, row) in flat.chunks(CATEGORY_EXPECTATION_STRIDE).enumerate() {
            let nonzero = row.iter().filter(|&&v| v != 0.0).count();
            assert_eq!(
                nonzero, lc[r] as usize,
                "expectation row {r} non-zero run must equal level_count"
            );
        }
        // .meta quotes the first row as "0, 0.392, 0.761, …, 4.724".
        assert!((flat[1] - 0.392).abs() < 1e-4);
        assert!((flat[2] - 0.761).abs() < 1e-4);
        assert!((flat[13] - 4.724).abs() < 1e-4);
    }
}
