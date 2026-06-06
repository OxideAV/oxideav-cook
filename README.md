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
  retyped into source. [`iter_flavor_records()`](src/flavor.rs) walks
  every `(index, record)` pair in table order;
  [`flavor_indices_matching_cookie(cookie)`](src/flavor.rs) returns
  every record whose four cookie-checkable fields (channels, subband
  count, stereo mode, recovered samples-per-frame) agree with a parsed
  cookie — a cookie does not carry `frame_bytes` / `sample_rate_hz` /
  `coupling_mode`, so the real `FUN_RM_32.rm` cookie legitimately
  matches both records 21 and 22 (they differ only in `frame_bytes`,
  744 vs 1024) and the container-supplied `flavor` index is what
  disambiguates at open time. [`FlavorRecord::is_sentinel`](src/flavor.rs)
  discriminates the closing single-subband sentinel record at index 30
  (`SENTINEL_FLAVOR_INDEX`) the spec/02 §1.1 audit-resolved block pins
  (`(17, 5, 1024, 1, 1, 256, 44100)` — `subband_count = 1` is the
  identifying field), and
  [`iter_playable_flavor_records()`](src/flavor.rs) walks exactly the
  30 playable presets at indices 0..=29. Two audit-anchored constants
  surface the hardcoded immediates the binary's ordinal-7 / ordinal-9
  exports return:
  [`RA_GET_NUMBER_OF_FLAVORS_ADVERTISED`](src/flavor.rs) = 15
  (`cook.dll!0x1620`, `mov ax, 0x0f`) and
  [`RA_GET_NUMBER_OF_FLAVORS2_ADVERTISED`](src/flavor.rs) = 34
  (`cook.dll!0x1630`, `mov ax, 0x22`, also the `RASetFlavor` upper
  bound at `cook.dll!0x1640`), distinct from the table-derived
  [`FLAVOR_COUNT`](src/flavor.rs) = 31 — anchored to
  `docs/audio/cook/provenance/03-cook-audit.md` audit point #2 / #12.
- **Extradata cookie parser** — [`CookCookie::parse`](src/cookie.rs)
  reads the big-endian per-stream extradata cookie for the extended
  (`0x01000003`) selector and cross-checks that it self-describes the
  same configuration (channels, subband count, stereo mode, recovered
  samples-per-frame) as its named flavor record. Pinned against the real
  `FUN_RM_32.rm` stream (flavor 21) in `tests/cookie_realstream.rs`.
  [`CookCookie::validate_geometry`](src/cookie.rs) enforces the three
  field-level invariants spec/02 §1 pins on every well-formed flavor
  record (`channels ∈ {1, 2}`, `subband_count >= 1`,
  `samples_per_frame × channels >= 1`); `parse` runs it automatically
  so every returned cookie is structurally well-formed by
  construction, and `DecodeConfig::from_inputs` re-runs the guard at
  the head of its wiring so literal-built cookies (test fixtures,
  cached wire snapshots) get the same structural rejection. Surfaced
  as `Error::CookieInvalidChannels` / `Error::CookieZeroSubbandCount`
  / `Error::CookieZeroSamplesProduct` so callers can distinguish a
  malformed cookie body from a cookie that simply names the wrong
  flavor record.
- **Backend-family selector classification** —
  [`SelectorFamily`](src/cookie.rs) classifies any 32-bit selector by the
  backend family the proprietary decoder's factory `0x1c60` would
  dispatch to (spec/01 §3.1): the three mono/stereo Cook selectors
  (`0x01000001` / `0x01000002` / `0x01000003`) share `MonoStereo`,
  `0x02000000` is `Multichannel`, and anything else lands as
  `Unsupported`. `CookCookie::parse` now returns the family-specific
  `Error::NonExtendedSelectorNotSupported` for the two MonoStereo
  siblings (same backend, shorter cookie layout that spec/01 §3 does
  not pin — DOCS-GAP) and `Error::MultichannelSelectorNotSupported`
  for the `0x02000000` family (distinct backend, cookie layout
  GAP-only), letting downstream callers triage GAP-typed selectors
  separately from values the binary genuinely rejects with the
  `0x80040005` path.
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
  `+0x0a = 0`) and every cookie/flavor disagreement. The
  descriptor-side spf-recovery step is also exposed on its own as
  [`Descriptor::recover_samples_per_frame`](src/init.rs) so a stream
  sniffer can reproduce the backend `idiv` at `cook.dll!0x21c2`
  (`validation/04` §4.2: `2048 / 2 = 1024`) without committing to a
  full flavor cross-check yet — the typed `Error::ZeroDivisorChannels`
  surfaces the divide-by-zero path the same way the full pipeline
  does. Pinned
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
- **Per-`RADecode` sub-packet split + PCM offset accounting** —
  [`SubPacketLayout`](src/subpacket.rs) is the second byte-touching
  structural stage of the `RADecode` decode driver
  (`docs/audio/cook/spec/01-cook-decoder-structure.md` §5,
  `docs/audio/cook/validation/04-cook-stream-validation.md` §5). Derived
  from a wired [`DecodeConfig`], it partitions one `RADecode`-call input
  into `sub_packets_per_call` consecutive fixed-stride slots of
  `sub_packet_size` bytes ([`slot_byte_range`](src/subpacket.rs),
  [`call_byte_range`](src/subpacket.rs),
  [`iter_call`](src/subpacket.rs)). On the validated `FUN_RM_32.rm`
  stream that is 144 `RADecode` calls × 5 sub-packets × 93 bytes; the
  whole-stream ranges tile the concatenated 144 × 465 = 66 960-byte
  audio input with no gap or overlap, and
  [`pcm_offset_for_call`](src/subpacket.rs) reproduces the validator's
  first-call 8 192-byte overlap-add warm-up + steady-state 20 480
  bytes/call cadence, summing to the pinned 2 936 832-byte total at
  call 144. Pinned end-to-end against all 144 real packets in
  `tests/subpacket_realstream.rs`.
- **`RADecode` call-sequence session state** —
  [`CallSession`](src/session.rs) is the third structural decode-pipeline
  stage above the backend (`docs/audio/cook/spec/01-cook-decoder-
  structure.md` §5 driver-state semantics + `docs/audio/cook/validation/
  04-cook-stream-validation.md` §5 cadence). Built from a [`DecodeConfig`]
  or a [`SubPacketLayout`], it holds the running `RADecode` call counter
  and the running PCM cursor and exposes
  [`next_call_expected_input_len`](src/session.rs) (= `frame_bytes`),
  [`next_call_pcm_bytes`](src/session.rs) (warm-up on call 0, steady-state
  thereafter), and [`next_call_pcm_byte_range`](src/session.rs) for
  sizing the next call's input and output deterministically;
  [`advance_one_call`](src/session.rs) validates both buffer lengths
  against the validator-pinned per-call budget and increments the
  cursor. Pinned end-to-end against the 144-call sequence of
  `FUN_RM_32.rm` in `tests/session_realstream.rs`: walking the full
  sequence produces the validator's pinned 2 936 832-byte total. The
  backend frame-decode + carry-buffer state machine itself (the
  `[backend_vtable + 0x0c]` body — bitstream reader, gain/quantiser,
  MDCT, overlap-add) is still typed as `Error::NotImplemented` — it
  lands in a later round.
- **Per-call `Driver` orchestrator** — [`Driver`](src/driver.rs) bundles
  a [`DecodeConfig`], a [`CommonMode`] toggle, and an embedded
  [`CallSession`] into the `RADecode`-equivalent per-call entry point
  spec/01 §5 describes for the decode driver `cook.dll!0x1260`.
  [`Driver::prepare_call`](src/driver.rs) validates the input length,
  runs the per-buffer XOR descramble when common mode is on (the
  constructor default is off — matching the validated real-stream
  path), and returns a [`PreparedCall`](src/driver.rs) exposing the
  descrambled bytes and the sub-packet iterator;
  [`Driver::advance_after_decode`](src/driver.rs) accounts for one
  completed call against the validator-pinned per-call PCM budget
  (warm-up on call 0, steady-state thereafter). The full-pipeline
  [`Driver::decode_call`](src/driver.rs) validates buffer sizes,
  orchestrates stages 1+2, and surfaces the backend frame-decode
  (`[backend_vtable + 0x0c]`) as `Error::NotImplemented` — reserving
  that signal exclusively for the transform GAP so length errors stay
  distinct. Pinned end-to-end against all 144 real packets of
  `FUN_RM_32.rm` in `tests/driver_realstream.rs`: every packet passes
  `prepare_call` verbatim on the default off-path with the 5 × 93
  sub-packet split, walking the 144-call cadence with
  `advance_after_decode` reproduces the validator's `2 936 832`-byte
  total, and `decode_call` on a wired-correct packet/output pair
  signals the backend GAP without advancing the cursor.
- **Per-category gain/quantiser parameter bundle** —
  [`category`](src/category.rs) wires the typed accessor for the three
  parallel `[cat*4 + base]` arrays the per-band quantiser worker
  `cook.dll!0x69f0` reads (audit points #18 / #19 in
  `docs/audio/cook/provenance/03-cook-audit.md`).
  [`CategoryIndex::new`](src/category.rs) enforces the `0..=6` range
  the worker validates (the `.meta` note *"category index 7 is guarded
  out by the worker"* is now a typed `Error::CategoryOutOfRange`
  rejection), and [`CategoryParameters::for_index`](src/category.rs)
  returns the matching `gain_step` (`0x8f58`, `2^(n/2)` centred on 1.0)
  + `gain_bias` (`0x8f74`, monotone-increasing `-0.20..0.0`) +
  `level_count` (`0x8f90`, strictly-decreasing `{13, 9, 6, 4, 3, 2, 1}`)
  triple in a single call. The per-band quantiser algorithm itself
  (the meta's *"forms (bias + |sample| * step) per band"* sentence +
  the band-loop driving it) remains a DOCS-GAP — this module models
  the structural parallel-table lookup only.
- **Per-buffer XOR descramble** — [`descramble`](src/descramble.rs) is
  the first byte-touching stage of the `RADecode` decode driver: a
  word-wise (32-bit, little-endian) XOR pass over the input, keyed by
  `xor_key(in_ptr, in_len) = input_ptr ^ input_len`. The pass is gated
  by the [`CommonMode`](src/descramble.rs) flag (context `+0x30`,
  set by `RASetComMode`); the constructor default is **off**, so
  `descramble_packet(CommonMode::off(), pkt, key)` returns the packet
  verbatim as a zero-copy `Cow::Borrowed` — exactly the validated
  real-stream path
  (`docs/audio/cook/validation/04-cook-stream-validation.md` §4.3 / §5,
  which fed the 144 packets straight from the container, 144/144 S_OK).
  The on-path (XOR enabled) has no bit-exact validator ground truth, so
  its tests pin only the algebraic properties (self-inverse, byte-count
  preservation) on the real `FUN_RM_32.rm` packets in
  `tests/descramble_realstream.rs`. The trailing partial word
  (`len % 4 != 0`) is copied verbatim — a recorded tail-handling
  DOCS-GAP.

## Not yet implemented

The backend frame-decode itself — the bitstream reader, gain/quantiser,
inverse MDCT, optional LPC/temporal prediction, post-filter, and joint-
stereo coupling sitting behind the `[backend_vtable + 0x0c]` slot —
remains a `crate::Error::NotImplemented` GAP. The full-pipeline
`Driver::decode_call` validates buffer sizes and runs stages 1+2, then
surfaces that GAP signal explicitly so consumers can already wire the
crate into a real container demuxer and treat the backend signal as the
single documented gating value while the transform pipeline lands in
later rounds. The `oxideav_core` registration glue and the cookie
layouts of the non-extended `0x01000001` / `0x01000002` mono/stereo
siblings and the multichannel (`0x02000000`) backend family are also
DOCS-GAPs — typed in `CookCookie::parse` so callers can triage
GAP-typed selectors separately from values the binary genuinely
rejects. The numeric tables the decode path will consume are all
vendored and validated, and the per-call orchestration that surrounds
the transform is now wired deterministically through `Driver` — what
remains is the algorithm itself.
