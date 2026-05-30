# oxideav-cook

Pure-Rust RealAudio Cook audio codec for the
[oxideav](https://github.com/OxideAV/oxideav-workspace) framework.

## Status

**Clean-room rebuild — round 1.** This `master` branch is a fresh
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

## Not yet implemented

The transform (MDCT), gain/quantiser, and entropy decode pipeline, the
`oxideav_core` registration glue, and the multichannel (`0x02000000`)
backend selector. The public decode path still returns
`Error::NotImplemented`.
