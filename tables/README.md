# oxideav-cook vendored tables

Numeric parameter tables consumed by the decoder, vendored here so the
crate is self-contained for publication. Each is a pure data fact
(*Feist v. Rural*) — the values are not authored, they describe the
geometry the format defines and are extracted directly from the bytes
of the proprietary RealAudio Cook decoder binary by `extract.py` in the
clean-room workspace at `docs/audio/cook/tables/` (parent workspace).

| File | Records | Fields | Consumed by |
| ---- | ------- | ------ | ----------- |
| `flavor-geometry-table.csv` | 31 (indices 0–30) | 7 u32 per record: `coupling_mode, stereo_mode, samples_per_frame, channels, subband_count, frame_bytes, sample_rate_hz` | `crate::flavor` |

The geometry table is loaded at access time via `include_str!` and
parsed in `src/flavor.rs`; numbers are never retyped into Rust source.

A real-stream cross-check (the `FUN_RM_32.rm` flavor-21 cookie, byte
sequence `01 00 00 03 08 00 00 20 00 00 00 00 00 02 00 04`) lives in
`tests/cookie_realstream.rs` and reproduces the §4 field-by-field
agreement recorded in the clean-room validation chapter.
