# oxideav-cook

Pure-Rust RealAudio Cook audio codec for the
[oxideav](https://github.com/OxideAV/oxideav-workspace) framework.

## Status

**Clean-room rebuild — round 3.** This `master` branch is a fresh
orphan. The previous implementation was retired alongside the docs
audit dated 2026-05-06, which found that the source-of-record trace
document for this codec was authored with a methodology that did not
satisfy clean-room separation. The prior history is preserved on the
`old` branch for archival but is forbidden input for the rebuild.

The rebuild draws only from the strict-isolation clean-room workspace
under `docs/audio/cook/` (binary-derived structural spec + extracted
numeric facts tables + real-stream validation).

## What works

- **Flavor geometry table** — [`flavor_record(index)`](src/flavor.rs)
  loads the 31 well-formed per-flavor geometry records (sample rate,
  channels, samples-per-frame, subband count, frame bytes, coupling /
  stereo mode) from the vendored facts table
  `tables/flavor-geometry-table.csv`, parsed on demand so no numbers are
  retyped into source.
- **Extradata cookie parser** — [`CookCookie::parse`](src/cookie.rs)
  reads the big-endian per-stream extradata cookie for the extended
  (`>= 0x01000003`) selector and cross-checks that it self-describes the
  same configuration (channels, subband count, stereo mode, recovered
  samples-per-frame) as its named flavor record. Pinned against the real
  `FUN_RM_32.rm` stream (flavor 21) in `tests/cookie_realstream.rs`.
- **DSP parameter tables** — [`crate::tables`](src/tables.rs) vendors
  the eight remaining extracted numeric tables (two 127-entry
  power-of-two ladders, per-category gain-step / gain-bias / level-count
  triples, an 11-entry reciprocal table, a 51-entry monotone
  category-index LUT, and the five Princen-Bradley MDCT half-windows
  of lengths 3 / 7 / 15 / 31 / 64). Each loader is `OnceLock`-cached
  and self-validates against the constraint stated in the matching
  `.meta` provenance (e.g. f32-exact equality with `2^k`, TDAC identity
  to better than 1e-3).
- **Decoder open-time geometry** —
  [`DecodeConfig::from_inputs`](src/init.rs) wires a parsed cookie,
  the two `RAInitDecoder` descriptor scalars (`+0x06 channels`,
  `+0x0a sub_packet_size`), and the named flavor record into a single
  derived configuration: `sub_packets_per_call = frame_bytes /
  sub_packet_size`, `pcm_bytes_per_call = sub_packets_per_call ×
  samples_per_frame × channels × 2`, plus first-call overlap-add
  warm-up accounting. Rejects every divide-by-zero (`+0x06 = 0`,
  `+0x0a = 0`) and every cookie/flavor disagreement. Pinned
  end-to-end against `FUN_RM_32.rm` in
  `tests/realstream_decode_config.rs`: 144 `RADecode` calls reproduce
  the validator's 2 936 832-byte total PCM, 20 480-byte steady-state
  per-call budget, and 8 192-byte first-call warm-up exactly. The
  decode-gate constant `RADECODE_FLAGS_DECODE = 1` is the
  validator-pinned value (`(~flags) & 1` is the backend's
  decode/observe gate).
- **Real-stream wire-level cross-check** — a 68 KB RealAudio Cook
  stream (`tests/fixtures/FUN_RM_32.rm`, SHA-256
  `ae7804…45d5c`) is bundled with the crate and parsed end-to-end by
  [`tests/realstream_fixture.rs`](tests/realstream_fixture.rs). The
  test walks the top-level RealMedia chunk sequence
  (`.RMF`(18) / `PROP`(50) / `MDPR`(172) / `MDPR`(627) / `CONT`(26) /
  `DATA`(68706)), recovers the 16-byte Cook cookie from the audio
  `MDPR`'s 94-byte type-specific-data (anchored by the trailing
  lead-in `01 07 00 00 00 00 00 10`), walks all 144 audio packets
  (`[12 B header][465 B payload]`), and feeds the result into
  `DecodeConfig`. Every measurement matches the validator
  (`docs/audio/cook/validation/04-cook-stream-validation.md`)
  byte-for-byte: file SHA-256, every chunk size, the 144-packet ×
  465-byte payload framing, the 5 sub-packets per `RADecode` call,
  the 20 480-byte steady-state PCM budget, the 8 192-byte first-call
  overlap-add warm-up, and the 2 936 832-byte total PCM accounting
  for all 144 calls (16.649 s at stereo 44 100 Hz). A self-contained
  embedded SHA-256 keeps the crate dependency-free.

## Not yet implemented

The transform (MDCT), gain/quantiser, and entropy decode pipeline, the
`oxideav_core` registration glue, and the multichannel (`0x02000000`)
backend selector. The numeric tables the decode path will consume are
all vendored and validated, and the open-time geometry that surrounds
the transform is now derived deterministically; what remains is the
algorithm itself. The public decode path still returns
`Error::NotImplemented`.
