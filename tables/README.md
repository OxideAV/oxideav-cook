# oxideav-cook vendored tables

Numeric parameter tables consumed by the decoder, vendored here so the
crate is self-contained for publication. Each is a pure data fact
(*Feist v. Rural*) — the values are not authored; they describe the
geometry / DSP the format defines and are extracted directly from the
bytes of the proprietary RealAudio Cook decoder binary by `extract.py`
in the clean-room workspace at `docs/audio/cook/tables/` (parent
workspace).

| File | Records | Type | Consumed by |
| ---- | ------- | ---- | ----------- |
| `flavor-geometry-table.csv` | 31 records × 7 u32 | u32 | `crate::flavor` |
| `pow2-exponent-table.csv`   | 127 | f32 | `crate::tables::pow2_exponent_table` |
| `sqrt2-scale-ladder.csv`    | 127 | f32 | `crate::tables::sqrt2_scale_ladder` |
| `gain-step-2pow-half.csv`   | 7   | f32 | `crate::tables::gain_step_2pow_half` |
| `gain-bias-ramp.csv`        | 7   | f32 | `crate::tables::gain_bias_ramp` |
| `category-level-count.csv`  | 7   | u32 | `crate::tables::category_level_count` |
| `reciprocal-1-over-n.csv`   | 11  | f32 | `crate::tables::reciprocal_1_over_n` |
| `category-index-lut.csv`    | 51  | u32 | `crate::tables::category_index_lut` |
| `mdct-windows.csv`          | 5 rows (3, 7, 15, 31, 64) | f32 | `crate::tables::mdct_windows` |
| `category-vector-dim-lo.csv` | 7  | u32 | `crate::tables::category_vector_dim_lo` |
| `category-vector-dim-hi.csv` | 7  | u32 | `crate::tables::category_vector_dim_hi` |
| `spectral-codebook-dims.csv` | 7  | u32 | `crate::tables::spectral_codebook_dims` |
| `spectral-codebook-codes.csv` | 7 ragged rows | u32 | `crate::tables::spectral_codebook_codes` |
| `spectral-codebook-code-lengths.csv` | 7 ragged rows | u32 | `crate::tables::spectral_codebook_code_lengths` |
| `category-cost-lut.csv`     | 7   | u32 | `crate::tables::category_cost_lut` |
| `transform-rotation-coeffs.csv` | 74 rows × 5 | f32 | `crate::tables::transform_rotation_coeffs` |
| `mdct-window-builder-consts.csv` | 4 | f64 | `crate::tables::mdct_window_builder_consts` |
| `mdct-window-1024.csv`      | 513 | f32 | `crate::tables::mdct_window_1024` |
| `mdct-twiddle-cos-1024.csv` | 512 | f32 | `crate::tables::mdct_twiddle_cos_1024` |
| `mdct-twiddle-sin-1024.csv` | 512 | f32 | `crate::tables::mdct_twiddle_sin_1024` |
| `mdct-sine-1024.csv`        | 1024 | f32 | `crate::tables::mdct_sine_1024` |
| `coupling-rotation-coeffs.csv` | 256 pairs | f32 | `crate::tables::coupling_rotation_coeffs` |
| `coupling-index-permutation.csv` | 512 | u32 | `crate::tables::coupling_index_permutation` |
| `quant-index-reciprocals.csv` | 7 | u32 | `crate::tables::quant_index_reciprocals` |
| `spectral-dequant-scale.csv` | 8  | f32 | `crate::tables::spectral_dequant_scale` |
| `sign-lut.csv`              | 2   | f32 | `crate::tables::sign_lut` |
| `category-expectation.csv`  | 98 (0.0-delimited rows) | f32 | `crate::tables::category_expectation` |

The `mdct-*-1024` and `coupling-*` tables are **runtime-recovered**
facts: they are built by the vendor decoder's own `RAInitDecoder` into
heap/BSS buffers (never present in the file image) and were dumped from
the guest memory the vendor DLL populated in the univdreams sandbox
(`docs/audio/cook/provenance/06-cook-univdreams-extraction.md`,
`extract_runtime_dsp.py`) — data facts read from the decoder's memory
image, not an algorithmic derivation.

Tables are loaded at access time via `include_str!` and parsed on
demand; numbers are never retyped into Rust source. Per-table loaders
cache the parse via `std::sync::OnceLock` and self-validate against the
constraint stated in the matching `.meta` file in the clean-room
workspace (e.g. f32-exact equality with `2^k`, Princen-Bradley TDAC,
monotone non-decreasing). The validation tests are in
`src/tables.rs::tests`.

A real-stream cross-check (the `FUN_RM_32.rm` flavor-21 cookie, byte
sequence `01 00 00 03 08 00 00 20 00 00 00 00 00 02 00 04`) lives in
`tests/cookie_realstream.rs` and reproduces the §4 field-by-field
agreement recorded in the clean-room validation chapter.
