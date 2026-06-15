# Changelog

All notable changes to this crate are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `category` cross-table bridge — `CategoryIndex::gain_step_exponent()`
  and the free `gain_step_via_scale_ladder()` tie the per-category gain
  index to the shared `2^(k/2)` half-octave scale ladder. The two
  `docs/audio/cook/tables/*.meta` files pin the 7-entry per-category step
  table (`gain-step-2pow-half`, `0x8f58`: *"2^(n/2) … n = -3..+3 …
  centred on 1.0"*) and the 127-entry ladder (`sqrt2-scale-ladder`,
  `0x93f8`: *"2^(k/2), k = -63..+63"*) as the **same** `2^(k/2)` family,
  so category `cat` is the ladder element at exponent `k = cat - 3` (the
  new `GAIN_STEP_CENTRE_CATEGORY` = 3 maps to `2^0 = 1.0`). Resolving the
  step through the ladder reproduces the per-category-table value
  bit-for-bit (the gain-step table is a 7-element slice of the ladder), so
  a decode-side consumer can index either `.rdata` table interchangeably
  from a single category index. 3 unit tests pin the exponent mapping
  (`-3..=3`), the centre-at-unity, and the bit-identical equivalence
  across all 7 categories. The exponent-producing runtime stage (which
  worker feeds which ladder) remains a spec/01 §6 DOCS-GAP.
- `flavor_property` module — typed model of the `RAGetFlavorProperty`
  property-ID dispatch surface, grounded in
  `docs/audio/cook/spec/01-cook-decoder-structure.md` §4.2,
  `docs/audio/cook/spec/02-cook-flavor-and-extradata-layout.md` §1.2 and
  `docs/audio/cook/provenance/03-cook-audit.md` audit point #13. The
  export-ordinal-10 worker (`cook.dll!0x17a0`) dispatches `property_id`
  through an MSVC jump table at RVA `0x1be8` with 21 cases (IDs 0–20):
  cases 0, 4, 7 return a NUL-terminated string (length via `strlen`) and
  every other case returns a 32-bit integer (fixed returned length 4).
  `FlavorPropertyId` is a `0..=20`-range-checked newtype (out-of-range →
  the new `Error::FlavorPropertyIdOutOfRange`) with `kind()` /
  `is_string()` / `is_integer()` / `fixed_len()`, plus the
  `FlavorPropertyKind` enum and the `FLAVOR_PROPERTY_JUMP_TABLE_RVA` /
  `FLAVOR_PROPERTY_ID_COUNT` / `MAX_FLAVOR_PROPERTY_ID` /
  `FLAVOR_PROPERTY_INTEGER_LEN` / `STRING_PROPERTY_IDS` constants. The
  property-ID → meaning enumeration and the descriptor structure's
  stride / layout remain an explicit spec GAP. 9 unit tests.
- `spi` module — typed model of the RealAudio codec service-provider
  interface export surface, grounded in
  `docs/audio/cook/spec/01-cook-decoder-structure.md` §2 and
  `docs/audio/cook/provenance/03-cook-audit.md` audit points #1/#2/#3.
  `SpiExport` is an exhaustive ordinal-ordered enum over the 20 named
  exports (ordinals 1–20) with `ordinal()` / `name()` /
  `front_end_rva()` / `notimpl_result()` / `is_decode_path()` /
  `is_encoder()` accessors; the front-end RVAs are pinned to the spec/01
  §2 export table (and tested to lie in the `.text` section, with the two
  GUID stubs at `0x1210` sharing a body). The SPI `HRESULT` contract is
  exported as named constants (`S_OK`, `E_INVALIDARG` = `0x80070057`,
  `E_NOTIMPL` = `0x80004001`, `HR_UNRECOGNISED_SELECTOR` = `0x80040005`),
  alongside the hardcoded flavor-count immediates (`RA_NUMBER_OF_FLAVORS`
  = 15, `RA_NUMBER_OF_FLAVORS2` = 34) and the `RASetFlavor` context store
  offset (`RASETFLAVOR_CONTEXT_OFFSET` = `0x28`). 14 unit tests.
- `coupling` module — typed joint-stereo / coupling-mode classification
  of a flavor record's two leading selectors, grounded in
  `docs/audio/cook/spec/02-cook-flavor-and-extradata-layout.md` §1 and
  the extracted `tables/flavor-geometry-table.csv`. `StereoMode::from_raw`
  classifies the `+0x04` stereo-mode field (*"0 for mono; 2–5 for the
  stereo / surround families"*): `0` → `Mono`, `2..=5` → `Stereo(value)`,
  with the reserved `1` and any `> 5` raising the new
  `Error::StereoModeUnsupported`. `CouplingMode::from_raw` classifies the
  `+0x00` coupling/region field (*"0 for the plain mono/stereo flavors;
  small non-zero values for the coupled stereo and multichannel
  flavors"*): `0` → `None`, any non-zero → `Coupled(value)` (total — the
  spec admits the region family without a closed set, so the raw value
  is preserved). `FlavorRecord::coupling_mode_class` /
  `stereo_mode_class` and the `is_coupled` / `is_stereo` shortcuts read
  the already-parsed record fields. Eleven unit tests cross-check every
  vendored record and pin the table's empirical relationship (every
  stereo-mode record is also coupled, but the converse fails — record 0
  is coupled-mono). The per-value coupling *algorithm* remains a recorded
  DOCS-GAP. New exports: `CouplingMode`, `StereoMode`, `STEREO_MODE_MIN`,
  `STEREO_MODE_MAX`, `Error::StereoModeUnsupported`.
- `quantiser` module — the first per-band quantiser *arithmetic* (not a
  table accessor) the worker `cook.dll!0x69f0` computes. Two facts pinned
  in the table `.meta` files beyond round 11's parallel-table access:
  the per-band magnitude form `bias + |sample| * step`
  (`docs/audio/cook/tables/gain-bias-ramp.meta`, verbatim: *"the worker
  forms `(bias + |sample| * step)` per band"*) and the level-count clip
  (`docs/audio/cook/tables/category-level-count.meta`: the
  `{13, 9, 6, 4, 3, 2, 1}` LUT is *"used both to size and to clip the
  per-band quantiser index"*). `band_gain_magnitude(&params, sample)`
  and `CategoryParameters::band_gain_magnitude(sample)` evaluate
  `gain_bias + |sample| * gain_step` against one category bundle;
  `clip_quantiser_index(level_count, raw_index)` and
  `CategoryParameters::clip_quantiser_index(raw_index)` clip a raw index
  to `0..=level_count-1` (an index `>= L` clips to `L - 1`; the
  zero-count branch keeps the helper total). Eight unit tests pin both
  primitives: the magnitude form is `|sample|`-symmetric, collapses to
  the category bias at `sample = 0`, and reduces to `bias + |sample|`
  for category 3 (`gain_step == 1.0`); the clip passes through every
  in-range index and caps at the top valid index for each category,
  spot-checked against the `{13, …, 1}` LUT (cat 0 top index 12, cat 6
  single-level → always 0). The band loop driving these primitives
  (raw-index read, sign restoration, the `0x8fcc` category-expectation
  combine that audit #17 leaves a GAP, and the feed into the inverse
  MDCT) is not pinned beyond these two `.meta` sentences and remains a
  recorded DOCS-GAP; only the two stated primitives are wired.

- `reciprocal` module — typed accessors for the 11-entry reciprocal
  averaging-divisor table at `cook.dll!0xa7a8`
  (`docs/audio/cook/tables/reciprocal-1-over-n.meta`; the spec/01 §6
  row at `0xa7a8`: *"`1, 1/2 … 1/9, 1/20, 0`"*, *"the bytes after are
  separate scalar FP constants"*; audit point #15 in
  `docs/audio/cook/provenance/03-cook-audit.md`: Round-1's
  element-count estimate of 14 corrected to **11**). The table's three
  structural regions get typed coverage: the consecutive `1/n` run for
  denominators `1..=9` behind the `ReciprocalDenominator` newtype
  (`new(u8) -> Result<_, Error>` range guard, `new_const` panicking
  const-context constructor, `get` / `table_index` with `i = n - 1`)
  and the `reciprocal_for_denominator` lookup; the stored `1/20` at
  element 9 behind its own named accessor `reciprocal_one_twentieth`
  (its denominator is not adjacent to the run, so it is deliberately
  NOT constructible as a `ReciprocalDenominator` — `new(20)` rejects);
  and the stored trailing `0.0` at element 10, pinned by the
  `RECIPROCAL_TRAILING_ZERO_INDEX` constant + test. Named constants:
  `RECIPROCAL_TABLE_RVA = 0xa7a8`, `RECIPROCAL_TABLE_END_RVA` (derived
  `0xa7a8 + 11 × 4 = 0xa7d4`, never retyped), `RECIPROCAL_RUN_LEN = 9`,
  `RECIPROCAL_DENOMINATOR_MIN/MAX = 1/9`,
  `RECIPROCAL_ONE_TWENTIETH_INDEX = 9`,
  `RECIPROCAL_ONE_TWENTIETH_DENOMINATOR = 20`,
  `RECIPROCAL_TRAILING_ZERO_INDEX = 10`. Unit tests pin the `.meta`
  validation note end-to-end through the typed API: every run element
  is bit-identical to the correctly-rounded f32 `1/n`, element 9 is
  bit-identical to `1/20` (= `0.05f32`), element 10 is exactly `0.0`,
  and the three regions tile the 11-element table. With this module,
  **every** extracted numeric table in `docs/audio/cook/tables/` is
  reachable through a typed, range-guarded crate API. The table's
  runtime consumer (which backend worker averages / normalises with
  it) is not pinned by spec/01 — a recorded GAP; only the typed table
  access is wired.
- `Error::ReciprocalDenominatorOutOfRange { got: u8 }` variant for
  out-of-run denominator rejections.

- `mdct` module — typed accessors for the five vendored
  Princen-Bradley MDCT half-windows at `cook.dll!0x8d0c` (lengths
  3 / 7 / 15 / 31 / 64; `docs/audio/cook/tables/mdct-windows.meta`,
  the `tables/README.md` row-structure note, and audit point #14 in
  `docs/audio/cook/provenance/03-cook-audit.md`). API:
  `MdctWindowLength` (enum over exactly the five stored lengths, with
  `from_len(usize) -> Result<_, Error>` as the fallible constructor,
  `ALL`, `row_index`, `window_len`, the derived `element_offset` /
  `rva` row positioning, and `tdac_pinned`), `mdct_half_window
  (MdctWindowLength) -> &'static [f32]` as the length-keyed lookup,
  and the constants `MDCT_WINDOW_COUNT = 5`, `MDCT_WINDOW_TABLE_RVA =
  0x8d0c`, `MDCT_WINDOW_TABLE_END_RVA` (derived `0x8d0c + 120 × 4 =
  0x8eec` — the audit-#14 re-verified end boundary). Unit tests pin
  the `.meta` invariants through the typed API (monotone-decreasing
  rows, `1/sqrt2` at each midpoint, the Princen-Bradley TDAC identity
  for exactly the four rows the `.meta` covers) plus both audit-#14
  boundary facts (the 51-entry category LUT at `0x8c40` ends exactly
  at the window-table head; the windows end at `0x8eec`).
  `tdac_pinned` deliberately reports `false` for the 64-row — the
  `.meta` validation sentence covers only lengths 3/7/15/31, and the
  typed API asserts only what the trace pins. The long/short adaptive
  window switching (spec/01 §5.1) and the inverse-MDCT kernel (the
  `0xa1b0` rotation table — audit #16: no validated closed form)
  remain GAPs; only the typed window-table access is wired.
- `Error::MdctWindowLengthUnsupported { got: usize }` variant for
  window-length rejections.

- `DecodeGate` — typed `(~flags) & 1` decode/observe gate the decode
  driver `cook.dll!0x1260` forwards to the backend frame-decode method
  `[backend_vtable + 0x0c]`
  (`docs/audio/cook/validation/04-cook-stream-validation.md` §4.3 /
  `docs/audio/cook/spec/01-cook-decoder-structure.md` §5): `flags`
  bit 0 = 1 → `DecodeGate::Decode` (gate bit `0`, real bitstream
  decode); bit 0 = 0 → `DecodeGate::Observe` (gate bit `1`, zeroed
  overlap-add output independent of the input — the validator
  verified all-`0xFF` input produces the same zero output as the real
  packets). API: `DecodeGate::from_flags(u32)`,
  `DecodeGate::backend_gate_bit()` (= literally `(~flags) & 1`),
  `DecodeGate::is_decode()`.
- `Driver::decode_call_with_flags` — the six-argument `RADecode`
  analog taking the raw `flags` argument. The observe gate is
  **implemented**: the per-call PCM budget is zero-filled (the
  driver's buffer accounting is geometry-derived and
  gate-independent, so the observe path walks the validator-pinned
  warm-up / steady-state cadence) and the session cursor advances.
  The real-decode gate still surfaces the bitstream/transform backend
  as `Error::NotImplemented` without moving the cursor — that signal
  stays reserved exclusively for the transform GAP.
  `Driver::decode_call` is now the `flags = RADECODE_FLAGS_DECODE`
  shorthand (behaviour unchanged). Pinned against all 144 real
  packets of `FUN_RM_32.rm` in `tests/driver_realstream.rs`: the
  observe-gate walk completes 144/144 calls with all-zero PCM summing
  to the validator's `2 936 832`-byte total, and a real packet vs an
  all-`0xFF` packet produce byte-identical observe output (the §4.3
  verification, reproduced).

- `scale` module — typed exponent-indexed accessors for the two
  back-to-back 127-entry f32 scale ladders at `cook.dll!0x91fc`
  (`2^k`) and `cook.dll!0x93f8` (`2^(k/2)`), both spanning
  `k = -63..+63` (`docs/audio/cook/tables/pow2-exponent-table.meta` /
  `sqrt2-scale-ladder.meta`). API: `ScaleExponent::new(i8) ->
  Result<_, Error>` (typed range guard), `ScaleExponent::new_const`
  (panicking, const-context), `ScaleExponent::{get, table_index}`,
  the two lookups `pow2_scale_for_exponent` /
  `sqrt2_scale_for_exponent` over the vendored CSV slices, and the
  named constants `SCALE_EXPONENT_MIN = -63` / `SCALE_EXPONENT_MAX =
  63` / `SCALE_EXPONENT_BIAS = 63` (element `i ↔ k = i - 63`; element
  63 is the shared `2^0 = 1.0` midpoint). Also pins audit point #15's
  sub-pointer reconciliation
  (`docs/audio/cook/provenance/03-cook-audit.md`): the Round-1
  spec/01 §6 survey rows at `0x92d4` ("exact powers of two, 2^-9 …
  2^19") and `0x94a8` ("0.00138, 0.00195, 0.00276, … ≈2^(n/2)
  ladder") are **sub-pointers into** the two 127-entry ladders, not
  separate tables — surfaced as RVA-derived constants
  `POW2_SUBPOINTER_ELEMENT_OFFSET = (0x92d4 - 0x91fc) / 4 = 54` (first
  exponent `-9`) and `SQRT2_SUBPOINTER_ELEMENT_OFFSET = (0x94a8 -
  0x93f8) / 4 = 44` (first exponent `-19`), with tests pinning the
  spec-quoted values at those ladder positions (`2^-9` and `2^19`
  f32-exact for the pow2 row; the `0.00138 / 0.00195 / 0.00276`
  leading triple to printed precision for the sqrt2 row). The Round-1
  element counts (29 / 59) are recorded by the audit as superseded
  scan-extent estimates and are deliberately not exported. The
  exponent-producing runtime stage (which worker indexes which ladder
  with what exponent source) remains a spec/01 §6 GAP — this module
  wires the typed table access only.
- `Error::ScaleExponentOutOfRange { got: i8 }` variant for
  scale-ladder exponent rejections.
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
