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
