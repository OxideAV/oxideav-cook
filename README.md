# oxideav-cook

Pure-Rust RealAudio Cook audio codec for the
[oxideav](https://github.com/OxideAV/oxideav-workspace) framework.

## Status

**Clean-room rebuild, in progress.** This `master` branch is a fresh
orphan. The previous implementation was retired alongside the docs
audit dated 2026-05-06, which found that the source-of-record trace
document for this codec was authored with a methodology that did not
satisfy clean-room separation. The prior history is preserved on the
`old` branch for archival but is forbidden input for the rebuild.

The structural / table / per-call-orchestration layers are wired and
validated against a real RealAudio Cook stream; the backend frame-decode
transform (bitstream reader, gain/quantiser, inverse MDCT, joint-stereo
coupling) is not yet implemented (see "Not yet implemented").

The rebuild draws only from the strict-isolation clean-room workspace
under `docs/audio/cook/` (binary-derived structural spec + extracted
numeric facts tables + real-stream validation).

## What works

- **RealAudio codec SPI export surface** — [`spi`](src/spi.rs) types the
  20 named exports (ordinals 1–20) of `cook.dll` spec/01 §2 pins, behind
  the exhaustive ordinal-ordered [`SpiExport`](src/spi.rs) enum.
  [`SpiExport::ordinal`](src/spi.rs) / [`name`](src/spi.rs) /
  [`front_end_rva`](src/spi.rs) carry each export's PE-export-directory
  triple (`cook.dll!0xaa30`, audit point #1: *"all 20 ordinal/name/RVA
  triples match exactly"*), and the front-end RVAs are tested to lie in
  the `.text` section (`0x1000..0x7c3c`) and to share a body for the two
  `0x1210` GUID stubs (ordinals 8/9). [`notimpl_result`](src/spi.rs)
  surfaces the three `E_NOTIMPL` stubs (`RAGetBackend` /
  `RAGetDecoderBackendGUID` / `RAGetGUID`),
  [`is_decode_path`](src/spi.rs) / [`is_encoder`](src/spi.rs) split the
  decode SPI from the four encoder exports + DRM hook, and the SPI's
  `HRESULT` contract is exported as named constants
  ([`S_OK`](src/spi.rs) = 0, [`E_INVALIDARG`](src/spi.rs) = `0x80070057`
  NULL-handle, [`E_NOTIMPL`](src/spi.rs) = `0x80004001`,
  [`HR_UNRECOGNISED_SELECTOR`](src/spi.rs) = `0x80040005`) alongside the
  hardcoded flavor-count immediates ([`RA_NUMBER_OF_FLAVORS`](src/spi.rs)
  = 15, [`RA_NUMBER_OF_FLAVORS2`](src/spi.rs) = 34, audit #2) and the
  `RASetFlavor` context store offset
  ([`RASETFLAVOR_CONTEXT_OFFSET`](src/spi.rs) = `0x28`, audit #3). This
  is the export-level contract a container demuxer drives the codec
  through; the worker bodies live in the decode modules.
- **`RAGetFlavorProperty` property-ID dispatch** —
  [`flavor_property`](src/flavor_property.rs) types the export-ordinal-10
  worker's (`cook.dll!0x17a0`) MSVC jump table at RVA `0x1be8`
  (spec/01 §4.2, spec/02 §1.2, audit point #13): **21 cases**
  (property IDs 0–20), where cases **0, 4, 7** return a NUL-terminated
  string (length computed by `strlen`) and every other case returns a
  **32-bit integer** (fixed returned length `4`).
  [`FlavorPropertyId::new`](src/flavor_property.rs) is the `0..=20`
  range-checked newtype ([`MAX_FLAVOR_PROPERTY_ID`](src/flavor_property.rs)
  = 20; out-of-range raises the new `Error::FlavorPropertyIdOutOfRange`),
  and [`FlavorPropertyId::kind`](src/flavor_property.rs) classifies the
  return shape into the [`FlavorPropertyKind`](src/flavor_property.rs)
  enum (`String` → `fixed_len() == None`, the run-time `strlen`;
  `Integer` → `fixed_len() == Some(4)` =
  [`FLAVOR_PROPERTY_INTEGER_LEN`](src/flavor_property.rs)). The three
  string IDs are surfaced as
  [`STRING_PROPERTY_IDS`](src/flavor_property.rs) `= [0, 4, 7]`, and the
  table head as [`FLAVOR_PROPERTY_JUMP_TABLE_RVA`](src/flavor_property.rs)
  = `0x1be8`. The full property-ID → meaning enumeration and the
  property-descriptor structure's stride / field layout remain an
  explicit spec/01 §4.2 / spec/02 §1.2 DOCS-GAP — only the pinned
  dispatch surface (ID range + return kind) is wired.
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
- **Joint-stereo / coupling-mode classification** —
  [`coupling`](src/coupling.rs) turns a flavor record's two raw
  joint-coding selectors into checked types. [`StereoMode::from_raw`](src/coupling.rs)
  classifies the `+0x04` stereo-mode field per spec/02 §1
  (*"0 for mono; 2–5 for the stereo / surround families"*):
  `0` → `Mono`, `2..=5` → `Stereo(value)`, and the reserved `1` /
  any `> 5` raise `Error::StereoModeUnsupported` (both unassigned by
  the spec and absent from the extracted geometry table).
  [`CouplingMode::from_raw`](src/coupling.rs) classifies the `+0x00`
  coupling/region field (*"0 for the plain mono/stereo flavors; small
  non-zero values for the coupled stereo and multichannel flavors"*):
  `0` → `None`, any non-zero → `Coupled(value)` (the spec admits the
  region family without a closed set, so the value is preserved
  verbatim). [`FlavorRecord::coupling_mode_class`](src/flavor.rs) /
  [`stereo_mode_class`](src/flavor.rs) and the
  `is_coupled` / `is_stereo` shortcuts read the already-parsed record
  fields. Tests cross-check every vendored record (e.g. record 0 is
  coupled-mono, record 21 is coupled-stereo, the index-30 sentinel is
  coupled `17` / stereo `5`) and pin the table's empirical relationship
  (every stereo-mode record is coupled, but not conversely). The
  per-value coupling **algorithm** (which DSP each `2/3/4/5` stereo
  family and each `1/2/5/6/8/17/19` region selects) is a recorded
  DOCS-GAP — spec pins the value ranges, not the coupling arithmetic.
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
- **Bit-allocation category-index LUT** —
  [`bit_alloc`](src/bit_alloc.rs) wires the typed structural accessor
  for the 51-entry `cook.dll!0x8c40` table (audit point #14 in
  `docs/audio/cook/provenance/03-cook-audit.md`; `.meta`: *"monotone-
  nondecreasing small-integer index LUT (0..19); maps a 51-position
  axis onto 20 categories"*). Two newtypes
  ([`BitAllocAxisPosition`](src/bit_alloc.rs) in `0..=50` enforced by
  [`BitAllocAxisPosition::new`](src/bit_alloc.rs), and
  [`BitAllocCategory`](src/bit_alloc.rs) in `0..=19` — the latter only
  constructible via the lookup or the panicking const-context
  constructor) wrap
  [`bit_alloc_category_for_position`](src/bit_alloc.rs) over the
  vendored 51-entry `category-index-lut.csv` slice. Three audit-anchored
  constants surface the LUT bounds:
  [`BIT_ALLOC_AXIS_LEN`](src/bit_alloc.rs) = 51,
  [`BIT_ALLOC_CATEGORY_COUNT`](src/bit_alloc.rs) = 20, and
  [`MAX_BIT_ALLOC_CATEGORY`](src/bit_alloc.rs) = 19. Out-of-range axis
  positions raise the new `Error::BitAllocAxisOutOfRange { got: u8 }`
  typed error, and the unit tests pin the meta's three structural
  invariants end-to-end (every position maps to a category in `0..=19`,
  the mapping is monotone-nondecreasing across all 51 positions, every
  category in the full `0..=19` range is reached by some position). The
  LUT's runtime consumer inside the decoder backend (plausibly paired
  with the `0x8fcc` category-expectation table — audit point #17 is
  *"tightened but still GAP: 2D row/column layout not statically
  unambiguous"*) is not pinned by this round; the structural lookup is
  the piece this module wires.
- **Subband → coefficient-range geometry** —
  [`subband`](src/subband.rs) wires frame-syntax §2.1
  (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §2.1,
  `provenance/05` evidence #4): the same `cook.dll!0x8c40` LUT that
  [`bit_alloc`](src/bit_alloc.rs) reads as a category map is *also*,
  read as `[band*4 + 0x8c40]`, the **start spectral line of each
  subband** (identity `0..11` over the first twelve subbands, then
  compresses). [`subband_start_line`](src/subband.rs) reads `lut[band]`,
  [`subband_line_range`](src/subband.rs) is the half-open
  `[start_line[band] .. start_line[band+1])` coefficient range a band
  occupies, and [`SubbandGeometry::new`](src/subband.rs) caches the
  `subband_count + 1` boundary lines for a fixed-`subband_count` stream
  and answers per-band [`line_range`](src/subband.rs) /
  [`line_count`](src/subband.rs) / [`total_coded_lines`](src/subband.rs)
  queries — the band → line mapping both the §2.2 dequant walk and the
  §4 joint-stereo coupling split drive off. The companion `0.5` scalar
  at `0x8c3c` is surfaced as [`SUBBAND_HALF_SCALAR`](src/subband.rs). The
  §2.2 category-*assignment* bit-allocation loop (keyed off the `0x8f38`
  per-category expected-cost LUT, which is **not** among the extracted
  tables) is a recorded DOCS-GAP; only the static band → line geometry
  is wired here. Ten unit tests pin the identity run, the post-12
  compression, the gap-free boundary tiling, and the range bounds.
- **Exponent-indexed scale-ladder accessors** —
  [`scale`](src/scale.rs) wires the typed lookups for the two
  back-to-back 127-entry f32 ladders (`cook.dll!0x91fc` = `2^k`,
  `cook.dll!0x93f8` = `2^(k/2)`, both for `k = -63..+63` per
  `tables/pow2-exponent-table.meta` / `sqrt2-scale-ladder.meta`).
  [`ScaleExponent::new`](src/scale.rs) enforces the `-63..=63` exponent
  range (out-of-range raises the typed
  `Error::ScaleExponentOutOfRange { got: i8 }`), and
  [`pow2_scale_for_exponent`](src/scale.rs) /
  [`sqrt2_scale_for_exponent`](src/scale.rs) are the two lookups over
  the vendored CSVs (element `i ↔ k = i - 63`,
  `SCALE_EXPONENT_BIAS = 63` at the shared `2^0 = 1.0` midpoint). The
  module also pins audit point #15's sub-pointer reconciliation
  (`docs/audio/cook/provenance/03-cook-audit.md`): the
  spec/01 §6 rows at `0x92d4` ("29 f32, 2^-9 … 2^19") and `0x94a8`
  ("59 f32, 0.00138, 0.00195, 0.00276, …") are sub-pointers **into**
  these ladders, surfaced as derived (RVA-subtraction) constants
  `POW2_SUBPOINTER_ELEMENT_OFFSET = 54` /
  `POW2_SUBPOINTER_FIRST_EXPONENT = -9` and
  `SQRT2_SUBPOINTER_ELEMENT_OFFSET = 44` /
  `SQRT2_SUBPOINTER_FIRST_EXPONENT = -19`; the tests pin the spec-quoted
  values at those positions (`2^-9` / `2^19` f32-exact, and the three
  `0.00138 / 0.00195 / 0.00276` leading values to printed precision).
  The exponent-producing runtime stage (which worker feeds which
  ladder) remains a spec/01 §6 GAP — only the typed table access is
  wired.
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
  triple in a single call. This module models the structural
  parallel-table lookup; the two per-band *arithmetic* primitives the
  `.meta` files pin are wired in [`quantiser`](src/quantiser.rs) (below),
  and the band loop that drives them remains a DOCS-GAP. The module also
  exposes the **cross-table bridge** to the shared `2^(k/2)` scale ladder:
  [`CategoryIndex::gain_step_exponent`](src/category.rs) maps a category
  onto the half-octave exponent `k = cat - 3` (centre
  [`GAIN_STEP_CENTRE_CATEGORY`](src/category.rs) = 3 → `2^0 = 1.0`), and
  [`gain_step_via_scale_ladder`](src/category.rs) resolves the step
  through [`scale`](src/scale.rs)'s 127-entry `0x93f8` ladder instead of
  the 7-entry `0x8f58` table. The two `.meta` files pin both as the same
  `2^(k/2)` family (`gain-step-2pow-half.meta`: *"2^(n/2) … n = -3..+3 …
  centred on 1.0"*; `sqrt2-scale-ladder.meta`: *"2^(k/2), k = -63..+63"*),
  so the ladder reading reproduces the per-category-table value
  bit-for-bit — the gain-step table is a 7-element slice of the ladder.
  The exponent-producing runtime stage stays a spec/01 §6 DOCS-GAP.
- **Per-band quantiser arithmetic** — [`quantiser`](src/quantiser.rs)
  wires the two per-band primitives the worker `cook.dll!0x69f0`
  computes, pinned verbatim in the table `.meta` files (the first decode
  *arithmetic* beyond table access). The magnitude form
  `bias + |sample| * step` — `gain-bias-ramp.meta`: *"the worker forms
  `(bias + |sample| * step)` per band"* — is
  [`band_gain_magnitude(&params, sample)`](src/quantiser.rs) /
  [`CategoryParameters::band_gain_magnitude`](src/quantiser.rs),
  evaluating `gain_bias + |sample| * gain_step` against one
  [`CategoryParameters`](src/category.rs) bundle. The level-count clip —
  `category-level-count.meta`: the `{13, 9, 6, 4, 3, 2, 1}` LUT is
  *"used both to size and to clip the per-band quantiser index"* — is
  [`clip_quantiser_index(level_count, raw_index)`](src/quantiser.rs) /
  [`CategoryParameters::clip_quantiser_index`](src/quantiser.rs),
  capping a raw index to `0..=level_count-1` (an index `>= L` clips to
  `L - 1`). Eight unit tests pin both: the magnitude is `|sample|`-
  symmetric, collapses to the category bias at `sample = 0`, and reduces
  to `bias + |sample|` for category 3 (`gain_step == 1.0`); the clip
  passes through every in-range index and caps at the top valid index
  per category (cat 0 → 12, cat 6 single-level → always 0). The band
  loop driving these (raw-index read, sign restoration, the `0x8fcc`
  category-expectation combine that audit #17 leaves a GAP, and the feed
  into the inverse MDCT) is not pinned beyond these two `.meta`
  sentences and stays a recorded DOCS-GAP.
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
- **Typed MDCT half-window accessors** — [`mdct`](src/mdct.rs) keys
  the five vendored Princen-Bradley half-windows (`cook.dll!0x8d0c`,
  lengths 3 / 7 / 15 / 31 / 64 — the windowing / overlap-add side of
  the inverse-MDCT stage spec/01 §5.1 inventories) behind the typed
  [`MdctWindowLength`](src/mdct.rs) selector: only the five stored
  lengths are constructible ([`MdctWindowLength::from_len`](src/mdct.rs)
  raises the typed `Error::MdctWindowLengthUnsupported` otherwise), and
  [`mdct_half_window`](src/mdct.rs) is the length-keyed lookup over the
  vendored rows. Per-row positioning is derived, never retyped:
  [`element_offset`](src/mdct.rs) sums the preceding row lengths
  (row starts 0 / 3 / 10 / 25 / 56) and [`rva`](src/mdct.rs) is pure
  RVA arithmetic from the `.meta` table head, with the audit-#14
  boundary facts (`docs/audio/cook/provenance/03-cook-audit.md`:
  *"cat-lut ends exactly at window table `0x8d0c`; windows end at
  `0x8eec`"*) pinned by tests through the derived
  [`MDCT_WINDOW_TABLE_END_RVA`](src/mdct.rs) constant.
  [`tdac_pinned`](src/mdct.rs) reports exactly which rows the `.meta`
  validation note covers with the Princen-Bradley TDAC identity
  (3 / 7 / 15 / 31 — the 64-row is not covered by that sentence and
  deliberately reports `false`). The long/short adaptive switching
  that selects a window at runtime and the inverse-MDCT kernel itself
  (the `0xa1b0` rotation table, audit #16: no validated closed form)
  remain GAPs — only the typed window-table access is wired.
- **Typed reciprocal-divisor accessors** — [`reciprocal`](src/reciprocal.rs)
  types the 11-entry averaging-divisor table (`cook.dll!0xa7a8`,
  `tables/reciprocal-1-over-n.meta`; spec/01 §6 row `0xa7a8`; audit
  #15's element-count correction 14 → 11) by its three structural
  regions: the consecutive `1/n` run for denominators `1..=9` behind
  the [`ReciprocalDenominator`](src/reciprocal.rs) newtype
  ([`reciprocal_for_denominator`](src/reciprocal.rs); out-of-run
  values raise the typed `Error::ReciprocalDenominatorOutOfRange`),
  the stored `1/20` at element 9 behind its own named accessor
  [`reciprocal_one_twentieth`](src/reciprocal.rs) (its denominator is
  not adjacent to the run — `new(20)` deliberately rejects), and the
  stored trailing `0.0` at element 10 (constant + test). The table-end
  RVA is derived (`0xa7a8 + 11 × 4 = 0xa7d4`), never retyped, and the
  tests pin the `.meta` validation note bit-exactly (each run element
  equals the correctly-rounded f32 `1/n`; element 9 equals `1/20`).
  With this module every extracted numeric table in
  `docs/audio/cook/tables/` is reachable through a typed,
  range-guarded API; the table's runtime consumer (which backend
  worker averages / normalises with it) is a recorded GAP.
- **`RADecode` flags decode/observe gate** — [`DecodeGate`](src/driver.rs)
  types the `(~flags) & 1` computation the decode driver
  `cook.dll!0x1260` forwards to the backend frame-decode method
  `[backend_vtable + 0x0c]`
  (`docs/audio/cook/validation/04-cook-stream-validation.md` §4.3:
  `flags` bit 0 = 1 → gate `0` → real bitstream decode; bit 0 = 0 →
  gate `1` → zeroed overlap-add output **independent of the input** —
  the validator verified all-`0xFF` input produces the same zero
  output as the real packets). All other `flags` bits are masked away,
  exactly as the binary's `& 1` does.
  [`Driver::decode_call_with_flags`](src/driver.rs) is the
  six-argument `RADecode` analog: the **observe gate is implemented**
  (zero-fills the per-call PCM budget — the driver's buffer accounting
  is geometry-derived and gate-independent, so the observe path walks
  the validator's warm-up / steady-state cadence — and advances the
  cursor), while the real-decode gate still surfaces the transform GAP
  as `Error::NotImplemented` without moving the cursor.
  `Driver::decode_call` is now the
  `flags = RADECODE_FLAGS_DECODE` shorthand. Pinned against all 144
  real packets of `FUN_RM_32.rm` in `tests/driver_realstream.rs`:
  the observe-gate walk completes 144/144 calls with all-zero PCM
  summing to the pinned `2 936 832`-byte total, and a real packet vs
  an all-`0xFF` packet produce byte-identical observe output.
- **Spectral-VLC codebook geometry + joint-stereo rotation closed form** —
  [`spectral`](src/spectral.rs) wires the statically-pinned wire-format
  facts of the backend frame syntax (`docs/audio/cook/spec/05-cook-
  backend-frame-syntax.md` §2.2 / §3.1 / §4.2). The per-category spectral
  -vector dimensions are typed as
  [`CategoryVectorDims::for_category`](src/spectral.rs) over the two new
  parallel tables (`0x9170` low `{2,2,2,4,4,5,5}` /
  `0x918c` high `{10,10,10,5,5,4,4}`, indexed by the `0..=6` category) —
  the count of spectral lines grouped per VLC symbol. The seven spectral
  Huffman codebooks are typed behind the range-checked
  [`SpectralCodebook`](src/spectral.rs) newtype
  ([`symbol_count`](src/spectral.rs) reads the `0x91e0` counts
  `{196,100,49,625,256,243,32}`; out-of-range raises the new
  `Error::SpectralCodebookOutOfRange`), and the embedded-sign-bit
  dequant LUT `{+1.0,-1.0}` at `0xa148` is
  [`SIGN_LUT`](src/spectral.rs) / [`sign_from_bit`](src/spectral.rs).
  The joint-stereo §4.2 reconstruction is the pinned mirror-index closed
  form: [`coupling_table_len`](src/spectral.rs) is `Ncoup = 1 <<
  coupling_bits`, [`mirror_partner_index`](src/spectral.rs) is the
  `Ncoup-1-j` partner read (self-inverse; the centre index of an
  odd-length table is its own partner — the 45° pan point), and
  [`split_coupled_coefficient`](src/spectral.rs) reproduces
  `(out0, out1) = c * (coef[j], coef[Ncoup-1-j])` given a
  caller-supplied coefficient table. The per-symbol codebook code/length
  **bytes** (§3.2) and the per-coupling-width rotation **coefficient
  values** (§4.3) are built in the decoder's `.data` BSS at init and are
  not in the file image — explicit recorded GAPs surfaced as RVA
  constants ([`SPECTRAL_CODEBOOK_VALUE_PTRS_RVA`](src/spectral.rs) /
  [`SPECTRAL_CODEBOOK_LENGTH_PTRS_RVA`](src/spectral.rs)) but with no
  retyped numbers, pending a dynamic-BSS-dump Validator round. 16 unit
  tests pin the codebook counts, vector-dimension sequences, sign LUT,
  and the mirror-index self-inverse / energy-pan invariants.
- **MSB-first frame bit reader** — [`bitreader`](src/bitreader.rs) wires
  the foundational primitive every backend per-frame stage reads through
  (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §0.1,
  `docs/audio/cook/provenance/05-cook-backend.md` evidence #1).
  [`FrameBitReader`](src/bitreader.rs) holds the four-field reader state
  block (`+0x479c` word pointer / `+0x47a0` bit position / `+0x47a4` bit
  cursor / `+0x47a8` bit limit — surfaced as the named `CTX_*_OFFSET`
  constants) and exposes the two pinned reader primitives:
  [`read_bits(n)`](src/bitreader.rs) (`read-n-bits`, `cook.dll!0x3f40`)
  assembles `n` bits MSB-first across the word boundary by the pinned
  closed form `word << pos | next >> (32 - pos)` then `>> (32 - n)`, and
  [`read_bit`](src/bitreader.rs) / [`read_flag`](src/bitreader.rs)
  (`read-1-bit`, `cook.dll!0x3fc0`) return the next single bit as an
  unsigned `0`/`1` and as the binary's arithmetic-shifted `0`/`-1` signed
  flag respectively. Reads at or past the bit limit (`+0x47a8`, the frame
  size in bits) return `0` and clamp the cursor at the limit, exactly the
  binary's *"reads past it return 0"* end-of-frame behaviour;
  [`with_bit_limit`](src/bitreader.rs) sets an explicit frame bit limit
  and [`new`](src/bitreader.rs) defaults it to the full byte length. The
  input byte slice is viewed as a sequence of big-endian 32-bit words
  ([`word_index`](src/bitreader.rs) / [`bit_position`](src/bitreader.rs)
  decompose the running cursor the same way the binary maintains it
  incrementally). 13 unit tests pin MSB-first extraction, the cross-word
  straddle, the limit clamp, single-bit/multi-bit composition, and
  cursor/word/position lockstep. Only the reader primitives are wired;
  the frame body that drives them — the gain envelope (§1), the
  category/quant walk (§2), the spectral VLC descent (§3, the bit-by-bit
  walk `cook.dll!0x3a50` over the BSS-built codebooks of §3.2) and the
  inverse transform (§5) — and the runtime-built BSS codebook / coupling
  tables (§3.2 / §4.3) remain recorded DOCS-GAPs.
- **Gain-control envelope (frame-syntax part 1)** —
  [`gain`](src/gain.rs) wires the first frame-body stage the bit reader
  feeds: the per-sub-packet gain envelope
  (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §1,
  `provenance/05` evidence #2 / #3). Two statically-pinned, non-GAP
  primitives sit on top of [`FrameBitReader`](src/bitreader.rs) and the
  existing [`scale`](src/scale.rs) ladder.
  [`read_segment_count`](src/gain.rs) reads the leading 6-bit field
  (`read-n-bits` with `n = 6`, the worker `cook.dll!0x4b50`'s `push 6`)
  and applies the `−6` bias the worker forms (`count + 0xfffffffa`), so
  the wire field carries `segment_count + 6`; a raw value `< 6` surfaces
  the typed [`Error::GainSegmentCountUnderflow`](src/lib.rs).
  [`gain_factor_for_index`](src/gain.rs) resolves a per-segment gain
  index to `2^(index/2)` via the `0x93f8` `sqrt(2)` ladder indexed at its
  centre (`1.0` at element 63 — the `0x94f4` positive-window sub-pointer
  of evidence #3, `(0x94f4 − 0x93f8)/4 = 63`), with the
  `{1.0, √2, 2.0, 2√2, 4.0}` positive window exposed as
  [`GAIN_POS_WINDOW`](src/gain.rs). 10 unit tests pin the count-bias
  endpoints (raw 6 → 0 flat, raw 63 → 57), the mid-range bias, the 6-bit
  consume, the `< 6` / empty-frame underflow guard, the unity centre, the
  positive-window match to f32 tolerance, the symmetric negative branch
  (`f(-2)·f(2) == 1`) and the ladder range endpoints. The per-segment
  *record reads* (position + gain index, via the §3.2 BSS-gated VLC walk
  `cook.dll!0x3a50`) and the §1.2 piecewise-constant
  interpolation/application over the transform sub-blocks stay recorded
  DOCS-GAPs.

## Not yet implemented

The real-decode half of the backend frame-decode — the bitstream
reader, gain/quantiser, inverse MDCT, optional LPC/temporal prediction,
post-filter, and joint-stereo coupling sitting behind the
`[backend_vtable + 0x0c]` slot when the forwarded gate bit is `0` —
remains a `crate::Error::NotImplemented` GAP (the observe half, gate
bit `1`, is implemented: zeroed overlap-add output per validation/04
§4.3). The full-pipeline `Driver::decode_call` /
`Driver::decode_call_with_flags` validate buffer sizes and run stages
1+2, then surface that GAP signal explicitly on the real-decode gate so
consumers can already wire the crate into a real container demuxer and
treat the backend signal as the single documented gating value while
the transform pipeline lands in later rounds. The `oxideav_core` registration glue and the cookie
layouts of the non-extended `0x01000001` / `0x01000002` mono/stereo
siblings and the multichannel (`0x02000000`) backend family are also
DOCS-GAPs — typed in `CookCookie::parse` so callers can triage
GAP-typed selectors separately from values the binary genuinely
rejects. The numeric tables the decode path will consume are all
vendored, validated, **and now individually reachable through typed,
range-guarded accessors**, and the per-call orchestration that
surrounds the transform is wired deterministically through `Driver`.

The round-5 backend frame-syntax trace
(`docs/audio/cook/spec/05-cook-backend-frame-syntax.md`) pins the
**wire-format structure** of the transform — the MSB-first bit reader
(§0.1), the gain envelope's `read6 → count − 6` segment layout and its
`sqrt(2)^index` gain ladder (§1), the category/quant walk and its five
per-category tables + quantiser closed form (§2), the seven spectral
codebook dimensions + sign/scale LUTs (§3.1), and the joint-stereo
mirror-index rotation closed form (§4.2). The statically-pinned pieces of
that surface — the per-category vector dimensions, the seven codebook
symbol counts, the sign LUT, and the §4.2 mirror-index rotation — are now
wired in [`spectral`](src/spectral.rs). **What is not yet assembled into a
running real-decode path** is the bit-level walk that consumes them
(the bit-reader state machine, the gain-envelope + category-walk
sequencing, the spectral VLC descent, the iMDCT kernel) plus three
**runtime-built-in-BSS** GAPs the trace explicitly leaves open
(`docs/audio/cook/spec/05` §6): the per-symbol spectral codebook
code/length **bytes** (§3.2), the per-coupling-width rotation
**coefficient values** (§4.3), and the iMDCT `0x8fcc` / `0xa1b0`
rotation-table 2-D layouts (carried over from spec/01 §6). Each is
addressed through a relocated `.data` BSS pointer not present in the file
image, so it needs a dynamic-BSS-dump Validator/Extractor round before
the entropy + transform walk can be wired bit-exactly. Until then the
real-decode gate stays `crate::Error::NotImplemented` (the observe
half — gate bit `1`, zeroed overlap-add output per validation/04 §4.3 —
is implemented).
