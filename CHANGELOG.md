# Changelog

All notable changes to this crate are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `bit_alloc` module — typed structural accessor for the 51-entry
  bit-allocation category-index LUT at `cook.dll!0x8c40` (audit point
  #14 in `docs/audio/cook/provenance/03-cook-audit.md`;
  `docs/audio/cook/tables/category-index-lut.meta`: *"monotone-
  nondecreasing small-integer index LUT (0..19); maps a 51-position
  axis onto 20 categories"*). Two newtypes — `BitAllocAxisPosition` in
  `0..=50` (built by the fallible `BitAllocAxisPosition::new` returning
  `Error::BitAllocAxisOutOfRange { got: u8 }` for out-of-range values,
  or by the panicking `BitAllocAxisPosition::new_const` for const
  contexts) and `BitAllocCategory` in `0..=19` (constructible only via
  the LUT lookup or the panicking const-context constructor, so any
  value of the type carries the in-range invariant) — wrap the single
  lookup `bit_alloc_category_for_position(BitAllocAxisPosition) ->
  BitAllocCategory` over the vendored
  `tables::category_index_lut()` slice. Three audit-anchored constants
  surface the LUT bounds: `BIT_ALLOC_AXIS_LEN = 51`,
  `BIT_ALLOC_CATEGORY_COUNT = 20`, `MAX_BIT_ALLOC_CATEGORY = 19`. Unit
  tests pin the `.meta`'s three structural invariants end-to-end
  through the typed API (every position maps to a category in `0..=19`;
  the mapping is monotone-nondecreasing across all 51 positions; every
  category in the full `0..=19` range is reached by some axis position
  — the last property is what justifies the `BIT_ALLOC_CATEGORY_COUNT =
  20` constant). The LUT's runtime consumer inside the decoder backend
  (plausibly paired with the `0x8fcc` category-expectation table the
  dequant worker `0x4600` reads — audit point #17 is left as
  *"tightened but still GAP: 2D row/column layout not statically
  unambiguous"*) is not pinned by this round; the structural lookup is
  the piece this module wires.
- `FlavorRecord::is_sentinel() -> bool`, `iter_playable_flavor_records()`,
  and three named constants
  (`RA_GET_NUMBER_OF_FLAVORS_ADVERTISED = 15`,
  `RA_GET_NUMBER_OF_FLAVORS2_ADVERTISED = 34`,
  `SENTINEL_FLAVOR_INDEX = 30`): a typed structural discriminator
  separating the 30 playable flavor presets (indices 0..=29) from the
  closing single-subband sentinel record at index 30 the
  `docs/audio/cook/spec/02-cook-flavor-and-extradata-layout.md` §1.1
  audit-resolved block pins. The two `RAGetNumberOfFlavors*` constants
  surface the hardcoded immediates the binary's ordinal-7 / ordinal-9
  exports return (`docs/audio/cook/provenance/03-cook-audit.md` audit
  point #2: `mov ax, 0x0f` and `mov ax, 0x22`) as named, audit-anchored
  values distinct from the table-derived `FLAVOR_COUNT = 31`. The
  `is_sentinel()` predicate discriminates on `subband_count == 1` (the
  sentinel hits the minimum value; every playable record carries
  `subband_count >= 9`), and `iter_playable_flavor_records()` is the
  walker that visits exactly the 30 non-sentinel `(index, record)`
  pairs in table order for callers that want only the decodable music
  presets.
- `Descriptor::recover_samples_per_frame(&CookCookie) -> Result<u32, Error>`:
  typed accessor for the descriptor-side spf-recovery step. Reproduces
  the `idiv` at `cook.dll!0x21c2` inside the backend init `0x20c0` that
  `docs/audio/cook/validation/04-cook-stream-validation.md` §4.2 pins
  (on the validated stream: cookie `[4..6] = 2048`, descriptor
  `+0x06 = 2` ⇒ recovered `samples_per_frame = 1024`). Returns the
  typed `Error::ZeroDivisorChannels` when the descriptor's
  `channels_divisor` is `0` (would divide-by-zero in the backend
  init). `DecodeConfig::from_inputs` now delegates its inline
  divisor-then-cross-check step to this method; the recover-only
  accessor lets a stream sniffer fetch the transform frame length from
  cookie + descriptor without committing to a full flavor cross-check
  yet.
- `CookCookie::validate_geometry() -> Result<(), Error>`: structural
  well-formedness guard for the cookie body, anchored to the three
  field-level invariants `docs/audio/cook/spec/02-cook-flavor-and-
  extradata-layout.md` §1 pins on every well-formed flavor record
  (table at lines 26–36 and the "channels in {1, 2}" sentence at line
  50): `channels ∈ {1, 2}`, `subband_count >= 1`, and
  `samples_per_frame_x_channels >= 1`. Three new typed errors surface
  the failures: `Error::CookieInvalidChannels { got: u16 }` for a
  channels field outside `{1, 2}`, `Error::CookieZeroSubbandCount` for
  a zero subband count, and `Error::CookieZeroSamplesProduct` for a
  zero `samples_per_frame × channels` product. `CookCookie::parse` now
  runs the guard automatically, so every parser-built cookie is
  structurally well-formed by construction;
  `DecodeConfig::from_inputs` re-runs the same guard at the head of
  its wiring so literal-built cookies (test fixtures, cached wire
  snapshots) get the same structural rejection before the existing
  divisor / flavor checks. Lets callers distinguish a malformed
  cookie body from a cookie that simply names the wrong flavor record
  (still `Error::CookieFlavorMismatch`).
- `category` module: typed gain/quantiser category index newtype +
  per-category parameter bundle for the per-band quantiser worker
  `cook.dll!0x69f0`. API: `CATEGORY_COUNT = 7`,
  `MAX_CATEGORY_INDEX = 6`, `CategoryIndex::new(u8) -> Result<_, Error>`,
  `CategoryIndex::new_const(u8) -> CategoryIndex` (panicking,
  const-context),  `CategoryIndex::{get, as_usize}`,
  `CategoryParameters { gain_step, gain_bias, level_count }`,
  `CategoryParameters::for_index(CategoryIndex)`, and the
  `category_parameters(CategoryIndex)` free accessor. Wires the parallel
  `[cat*4 + base]` access pattern audit points #18 / #19 in
  `docs/audio/cook/provenance/03-cook-audit.md` pin (gain-step
  `0x8f58`, gain-bias `0x8f74`, level-count `0x8f90`), with the
  `0..=6` range enforced by typed construction — the
  `category-level-count.meta` note *"category index 7 is guarded out
  by the worker"* is now a typed `Error::CategoryOutOfRange` rejection
  rather than a runtime panic. The per-band quantiser arithmetic
  itself (the meta's *"(bias + |sample| * step)"* sentence and the
  band-loop driving it) remains a DOCS-GAP; this module wires only
  the structural lookup.
- `Error::CategoryOutOfRange { got: u8 }` variant for category-index
  rejections.
- `driver` module: `Driver` is the `RADecode`-equivalent per-call
  orchestrator — bundles a `DecodeConfig`, a `CommonMode` toggle, and
  an embedded `CallSession` into the single entry point spec/01 §5
  describes for the per-call decode driver `cook.dll!0x1260`. API:
  `Driver::new(DecodeConfig)`, `Driver::with_common_mode(CommonMode)`
  (builder), `Driver::set_common_mode(CommonMode)`,
  `Driver::common_mode()`, `Driver::config()`, `Driver::layout()`,
  `Driver::calls_completed()`, `Driver::total_pcm_emitted()`,
  `Driver::next_call_expected_input_len()`,
  `Driver::next_call_pcm_bytes()`,
  `Driver::next_call_pcm_byte_range()`, plus the two orchestration
  methods below. Pinned by `docs/audio/cook/spec/01-cook-decoder-
  structure.md` §5 (the body of the per-call decode driver) and
  `docs/audio/cook/validation/04-cook-stream-validation.md` §4.3 / §5
  (the validated per-call cadence).
- `Driver::prepare_call(packet, xor_key) -> Result<PreparedCall, Error>`:
  validates the input length, runs the per-buffer XOR descramble when
  `common_mode` is on (the constructor default is off, matching the
  validated real-stream path), and returns a `PreparedCall` exposing
  the descrambled bytes and the sub-packet iterator. Does **not**
  advance the session cursor — call `Driver::advance_after_decode`
  once the consumer's backend has filled the per-call PCM budget.
- `Driver::advance_after_decode(output_len) -> Result<(), Error>`:
  accounts for one completed `RADecode` call without invoking the
  backend. Validates `output_len` against the validator-pinned per-call
  budget (warm-up on call 0, steady-state thereafter) and advances the
  session cursor on success.
- `Driver::decode_call(packet, output, xor_key) -> Result<(), Error>`:
  the full-pipeline analog. Validates input + output sizes against
  the wired per-call budget, runs stages 1+2 (descramble + sub-packet
  split), and surfaces the backend frame-decode (`[backend_vtable +
  0x0c]`) as `Error::NotImplemented` — reserving that signal
  exclusively for the transform GAP so length mismatches stay
  distinct. On the GAP signal the cursor does NOT advance (no partial
  state).
- `PreparedCall<'a>`: descrambled, length-checked view of one call's
  input. API: `descrambled()`, `sub_packets_per_call()`,
  `sub_packet_size()`, `iter_sub_packets()`, `layout()`. Bytes are a
  zero-copy `Cow::Borrowed` on the off-path and a `Cow::Owned` on the
  on-path (matching `crate::descramble`).
- `tests/driver_realstream.rs`: walks the bundled `FUN_RM_32.rm` to
  its 144 validator-pinned 465-byte audio payloads and drives them
  through the `Driver`. Confirms `prepare_call` accepts every packet
  verbatim on the default off-path and partitions each into 5 × 93-byte
  sub-packet slots; that walking the 144 calls with `advance_after_decode`
  using the validator-pinned per-call budgets reproduces the
  `2 936 832`-byte total exactly; that `decode_call` on a real packet
  with correctly-sized buffers surfaces the backend GAP as
  `Error::NotImplemented` without advancing the cursor; that a
  wrong-sized input rejects with `CallInputLengthMismatch` (never the
  GAP signal); and that the on-path is self-inverse on a real packet.
- `session` module: `CallSession::new(SubPacketLayout)` /
  `CallSession::from_config(&DecodeConfig)` builds a stateful walker
  over a `RADecode` call sequence (the third structural decode-pipeline
  stage above the backend frame-decode). The session captures the
  running call counter and PCM cursor and exposes
  `calls_completed()`, `total_pcm_emitted()`,
  `next_call_expected_input_len()` (= `frame_bytes`),
  `next_call_pcm_bytes()` (validator-pinned: warm-up on call 0,
  steady-state thereafter), `next_call_pcm_byte_range()` for sizing
  the next call's output slice, and
  `advance_one_call(input_len, output_len)` to account for a completed
  call — validates both lengths against the validator-pinned per-call
  budget and steps the cursor. Pinned by
  `docs/audio/cook/spec/01-cook-decoder-structure.md` §5 (the per-call
  `RADecode` driver) and `docs/audio/cook/validation/04-cook-stream-
  validation.md` §5 (the 144-call cadence: 8 192-byte warm-up + 143 ×
  20 480-byte steady-state = 2 936 832-byte total).
- `Error::CallInputLengthMismatch { got, expected }` /
  `Error::CallOutputLengthMismatch { got, expected }` for the new
  session-level rejections.
- `tests/session_realstream.rs`: walks the bundled `FUN_RM_32.rm` to
  its 144 validated 465-byte payloads and runs `CallSession` over the
  full sequence — confirms the running cursor matches
  `DecodeConfig::total_pcm_bytes` at every step, reproduces the
  validator's 2 936 832-byte total exactly after 144 calls, and that
  mis-sized input/output buffers surface as typed mismatches without
  advancing the session state.
- `subpacket` module: `SubPacketLayout::from_config(&DecodeConfig)`
  derives the per-`RADecode` sub-packet split the driver `cook.dll!
  0x1260` enforces — `sub_packets_per_call` fixed-stride slots of
  `sub_packet_size` bytes each within one call's `frame_bytes`-byte
  input — and the validator-pinned PCM offset accounting (first-call
  warm-up + steady-state cadence). API: `slot_byte_range(slot)`,
  `call_byte_range(call_idx, slot)`, `iter_call(input)`,
  `pcm_offset_for_call(call_idx)`, `total_pcm_bytes(calls)`. Pinned by
  `docs/audio/cook/spec/01-cook-decoder-structure.md` §5 + `docs/audio/
  cook/validation/04-cook-stream-validation.md` §5.
- `Error::SlotOutOfRange { slot, slots_per_call }` /
  `Error::SubPacketInputLengthMismatch { got, expected }` variants for
  the new `subpacket` misuse rejections.
- `tests/subpacket_realstream.rs`: walks the bundled `FUN_RM_32.rm` to
  its 144 validated 465-byte payloads and confirms
  `SubPacketLayout::iter_call(packet)` partitions each into exactly 5 ×
  93-byte sub-packets that round-trip to the input; that the whole-
  stream `call_byte_range` values tile the 144 × 465 = 66 960-byte
  audio input with no gap or overlap; and that
  `pcm_offset_for_call(144) = 2 936 832` reproduces the validator's
  total-PCM figure exactly, with every adjacent pair after the warm-up
  advancing by the steady-state 20 480-byte budget.
- `SelectorFamily` enum + `SelectorFamily::classify(selector)` /
  `::is_parser_supported(selector)` API in `cookie`, classifying the
  cookie's leading 32-bit selector by the backend family the
  proprietary decoder's factory `cook.dll!0x1c60` would dispatch to
  (spec/01 §3.1): `MonoStereo` covers `0x01000001` / `0x01000002` /
  `0x01000003`, `Multichannel` covers `0x02000000`, and every other
  value is reported as `Unsupported`. `CookCookie::family()` returns
  the family of a parsed cookie (always `MonoStereo` today).
- `Error::NonExtendedSelectorNotSupported { selector }` returned by
  `CookCookie::parse` for the documented MonoStereo siblings
  `0x01000001` / `0x01000002` — same backend family as the validated
  `0x01000003` stream but a shorter cookie layout that spec/01 §3
  does not pin (DOCS-GAP). Distinct from `UnsupportedSelector` so a
  future shorter-cookie parser can be added without typed downstream
  callers losing their dispatch.
- `Error::MultichannelSelectorNotSupported { selector }` returned by
  `CookCookie::parse` for the multichannel `0x02000000` family
  (spec/01 §3.1). The backend factory would dispatch to the distinct
  multichannel backend (constructor `0x2260`); cookie body layout is
  not pinned by spec or the validator (the validated `FUN_RM_32.rm`
  stream is stereo) — DOCS-GAP.

### Changed

- `CookCookie::parse` no longer surfaces every `< 0x01000003` selector
  as `Error::UnsupportedSelector`. The two documented MonoStereo
  siblings (`0x01000001` / `0x01000002`) and the multichannel
  `0x02000000` selector now produce their family-specific GAP errors;
  `UnsupportedSelector` is reserved for values the backend factory
  `cook.dll!0x1c60` would reject with `0x80040005`.

### Earlier additions

- `descramble` module: the per-buffer XOR descramble — the first
  byte-touching stage of the `RADecode` decode driver. `xor_descramble`
  / `xor_descramble_into` run the word-wise (32-bit, little-endian) XOR
  pass; `xor_key(in_ptr, in_len)` computes the binary's per-call key
  `input_ptr ^ input_len` from explicit `u32` factors (no unsafe
  pointer-to-int). A trailing partial word (`len % 4 != 0`) is copied
  verbatim — a recorded tail-handling DOCS-GAP, the conservative
  self-inverse-preserving choice. Pinned by `docs/audio/cook/spec/
  01-cook-decoder-structure.md` §5 (the `0x1283` loop + Round-3 audit
  clarification) and `docs/audio/cook/validation/04-cook-stream-
  validation.md` §4.3.
- `CommonMode` toggle (`off()` / `on()` / `is_on()`) mirroring the
  context `+0x30` flag that gates the XOR pass: zero-initialised (off)
  by the constructor, set to `1` by `RASetComMode` (spec/01 §2). With
  `descramble_packet(common_mode, packet, key)` the off-path returns the
  input verbatim as a zero-copy `Cow::Borrowed` — the validated
  real-stream path (validation/04 §4.3 / §5) — and the on-path runs the
  XOR pass into a `Cow::Owned`.
- `tests/descramble_realstream.rs`: walks the bundled `FUN_RM_32.rm` to
  its 144 validated 465-byte audio payloads and confirms
  `descramble_packet(CommonMode::off(), pkt, 0)` is a byte-identical
  zero-copy `Cow::Borrowed`, and that the on-path is self-inverse and
  byte-count-preserving on the first packet and mid-stream packet 100
  for two arbitrary keys (the on-path has no bit-exact validator ground
  truth, so only its algebraic properties are pinned).
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
