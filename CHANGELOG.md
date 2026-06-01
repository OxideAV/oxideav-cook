# Changelog

All notable changes to this crate are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `flavor::iter_flavor_records()`: iterator over every well-formed
  `(index, FlavorRecord)` pair in the vendored geometry table (exactly
  `FLAVOR_COUNT` = 31 pairs, in table order, ending on the
  single-subband sentinel at index 30 called out by `docs/audio/cook/
  spec/02-cook-flavor-and-extradata-layout.md` §1.1).
- `flavor::flavor_indices_matching_cookie(cookie)`: returns every
  flavor index whose record agrees with the four-tuple a cookie itself
  carries (channels, subband count, stereo mode, recovered
  samples-per-frame). On the validated `FUN_RM_32.rm` stream this
  returns both **21 and 22** because the cookie does not carry
  `frame_bytes` — they share `(channels=2, subband_count=32,
  stereo_mode=4, samples_per_frame=1024)` and differ only in
  `frame_bytes` (744 vs 1024). Pinned by `docs/audio/cook/validation/
  04-cook-stream-validation.md` §4.1 (cookie field-set) and §4.4
  (record-21 cross-check).
- `tests/realstream_decode_config.rs`:
  `fun_rm_32_cookie_matches_records_21_and_22` exercises the new
  multi-match API on the real-stream cookie and pins the four shared
  fields across every matching record.
- `tests/realstream_fixture.rs`: wire-level cross-check of the bundled
  `tests/fixtures/FUN_RM_32.rm` RealMedia file against
  `docs/audio/cook/validation/04-cook-stream-validation.md`. The test
  pins the fixture's SHA-256
  (`ae7804ce179f7d8d907f67ac3e17c0da560e05c7730e1c45a04c1d19a2e45d5c`),
  walks the top-level chunk sequence
  (`.RMF`(18) / `PROP`(50) / `MDPR`(172) / `MDPR`(627) / `CONT`(26) /
  `DATA`(68706)), extracts the 16-byte Cook cookie from the audio
  `MDPR`'s 94-byte type-specific-data (anchored at the trailing
  `01 07 00 00 00 00 00 10` lead-in), walks all 144 audio packets in
  `DATA` (each `[12-byte packet header][465-byte payload]`), and feeds
  the result through `DecodeConfig::from_inputs` to confirm the
  validator's 5 sub-packets/call, 20 480 PCM bytes/call steady-state,
  8 192-byte first-call warm-up, and 2 936 832-byte total PCM
  accounting hold bit-for-bit on the bundled bitstream. The test
  carries its own embedded SHA-256 so the crate stays dependency-free.
- `tests/fixtures/FUN_RM_32.rm` (69 765 B): real RealAudio Cook stream
  (flavor 21, stereo 44 100 Hz, 1024-sample frames, 32 subbands) used
  by the new integration test. Mirrors the validator's fixture under
  `docs/audio/cook/fixtures/`; the bundled copy keeps the published
  crate self-contained.
- `EXTENDED_COOKIE_LEN` and `SELECTOR_EXTENDED` re-exports at the crate
  root for downstream consumers building their own RealMedia demuxers.
- `init` module: `DecodeConfig::from_inputs(cookie, descriptor, flavor,
  frame_bytes)` wires the per-stream open-time geometry pinned by
  `docs/audio/cook/validation/04-cook-stream-validation.md`. The
  derived config records `sub_packets_per_call = frame_bytes /
  sub_packet_size` and the steady-state PCM budget
  `pcm_bytes_per_call = sub_packets_per_call × samples_per_frame ×
  channels × 2`, plus `warmup_pcm_bytes` (two-frame first-call
  overlap-add) and `total_pcm_bytes(calls)` accounting. Rejects every
  divide-by-zero (`+0x06 = 0`, `+0x0a = 0`), cookie/flavor mismatch,
  and non-integer `frame_bytes / sub_packet_size` ratio.
- `Descriptor` struct mirroring the two `RAInitDecoder` `u16` scalars
  (`channels_divisor = +0x06`, `sub_packet_size = +0x0a`) named by
  their validated runtime role.
- `RADECODE_FLAGS_DECODE = 1` public constant: the validator-pinned
  `RADecode` `flags` value that enables backend frame-decode
  (`(~flags) & 1 = 0` reaches the backend's decode/observe gate).
- `PCM_BYTES_PER_SAMPLE = 2` public constant pinning the 16-bit
  LE sample format the decoder emits.
- `Error::ZeroDivisorChannels`, `Error::ZeroDivisorSubPacketSize`,
  `Error::CookieFlavorMismatch`,
  `Error::FrameNotDivisibleBySubPacket { frame_bytes, sub_packet_size }`
  for the new init-time rejections.
- `tests/realstream_decode_config.rs`: end-to-end cross-check of the
  derived `DecodeConfig` against the `FUN_RM_32.rm` numbers (144
  `RADecode` calls → 2 936 832 PCM bytes total; 20 480 bytes per
  steady-state call; 8 192 first-call warm-up; 16.649 s wall-clock).

## [0.0.2](https://github.com/OxideAV/oxideav-cook/releases/tag/v0.0.2) - 2026-05-30

### Other

- Round 2: vendor 8 extracted DSP tables with self-validating loaders
- Round 1: flavor geometry table loader + extradata cookie parser
- Round 0 — clean-room rebuild scaffold (orphan master)

### Added

- Flavor geometry table loader: `flavor_record(index) -> Option<FlavorRecord>`
  reads the 31 well-formed per-flavor records from the vendored facts
  table `tables/flavor-geometry-table.csv` (parsed on demand, never
  retyped into source).
- Extradata cookie parser: `CookCookie::parse` reads the big-endian
  per-stream cookie for the extended (`>= 0x01000003`) selector, recovers
  samples-per-frame, and cross-checks the cookie against its named flavor
  record (`matches_flavor`). Pinned against the real `FUN_RM_32.rm`
  stream (flavor 21) in `tests/cookie_realstream.rs`.
- `Error::CookieTooShort` and `Error::UnsupportedSelector` variants.
- `tables` module vendoring the remaining eight extracted DSP parameter
  tables (two 127-entry power-of-two ladders, per-category gain-step /
  gain-bias / level-count triples, the 11-entry reciprocal averaging
  table, the 51-entry monotone category-index LUT, and the five
  Princen-Bradley MDCT half-windows of lengths 3 / 7 / 15 / 31 / 64),
  each `OnceLock`-cached and self-validating against the constraint
  stated in its clean-room `.meta` (f32-exact equality with `2^k`, TDAC
  identity, monotonicity).

### Changed

- Clean-room rebuild from a fresh orphan `master`. The previous
  implementation was retired by the OxideAV docs audit dated
  2026-05-06; the prior history is preserved on the `old` branch.
  See `README.md` for the rebuild scope and the strict-isolation
  workspace the Implementer rounds will draw from.
