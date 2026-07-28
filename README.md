# oxideav-cook

[![CI](https://github.com/OxideAV/oxideav-cook/actions/workflows/ci.yml/badge.svg)](https://github.com/OxideAV/oxideav-cook/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/oxideav-cook.svg)](https://crates.io/crates/oxideav-cook) [![docs.rs](https://docs.rs/oxideav-cook/badge.svg)](https://docs.rs/oxideav-cook) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

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
validated against a real RealAudio Cook stream, and the **decode
transform is assembled end-to-end**: the §3 spectral entropy read (over
the recovered codebooks), the §2.2 category-assignment / bit-allocation
loop, the §4 joint-stereo coupling split, and the §5 iMDCT / windowing /
overlap-add all wire into audible 16-bit PCM. What is **not** yet pinned
is the per-frame **pre-spectral bitstream read layout** that reaches the
§3 data from the frame head on a real stream — which recovered codebook
the §1.2 gain-index / §2.2 quant-index VLC reads select, and how the
category-assignment value array `v[]` is formed — plus the iMDCT kernel's
exact normalisation/sign convention (see "Not yet implemented").

The rebuild draws only from the strict-isolation clean-room workspace
under `docs/audio/cook/` (binary-derived structural spec + extracted
numeric facts tables + real-stream validation).

## What works

- **§2.2 category-assignment / bit-allocation loop (`cook.dll!0x4800`,
  the last routing GAP)** — Cook does not transmit per-band spectral
  categories; they are **computed** in-decoder from a per-band value
  array `v[]` and the frame bit budget.
  [`category_assignment`](src/category_assignment.rs) is that loop, from
  `docs/audio/cook/provenance/08-cook-category-assignment.md` +
  `tables/category-assignment-params.csv`. The **base pass**
  `cat[b] = clip((32 + off − v[b]) >> 1, 0, 7)` picks the global offset
  so the total `0x8f38`-cost-LUT best matches the budget; the exact
  landing is the documented `K = 32` under a strict slack (`refine one
  category finer while total_cost + K < budget`), which reproduces the
  reference decoder's own `cook.dll!0x4800` output across a fine budget
  sweep and flat / non-flat / `Nb`-varied inputs (every test expectation
  is the validator's output, captured by driving the opaque validator
  binary — no decoder source read). The **Stage-2 ±1 refinement** is
  wired for the validated uniform-under-budget regime (`refine_uniform`);
  the non-flat priority interleave and over-budget reclaim order that
  `provenance/08` records as only partially characterised are left
  unrefined, not fabricated. [`decode_spectrum_assigned`](src/frame_decode.rs)
  computes the per-band [`BandCategory`] list from `(values, budget,
  refinement_bound)` and runs it straight through the codebook-by-category
  §3 band decode — the bridge between the quantiser indices and the
  spectral entropy read.

- **§3.2 spectral codebooks + the §3.1 VLC walk (docs-gap #1775 data
  recovered)** — the docs Extractor round 6 dumped the runtime-built-in-BSS
  spectral codebook code/length bytes by driving the vendor decoder's own
  `RAInitDecoder` in the univdreams sandbox
  (`docs/audio/cook/provenance/06-cook-univdreams-extraction.md`); they are
  vendored as `tables/spectral-codebook-{codes,code-lengths}.csv` and read
  through [`tables`](src/tables.rs) `OnceLock` loaders (Kraft-sum /
  code-fits-length / proper-prefix-code tests). [`codebook`](src/codebook.rs)
  turns each codebook's `(code, length)` pairs into a decode-ready
  read-until-match lookup: [`SpectralHuffman::decode_symbol`](src/codebook.rs)
  is the §3.1 VLC walk `cook.dll!0x3a50`, reading MSB-first through
  [`FrameBitReader`](src/bitreader.rs) and emitting a symbol on the first
  codeword match (the distinct codewords are a proper prefix code; the
  escape-style duplicate max-length codewords resolve to their first symbol,
  with [`is_escape_symbol`](src/codebook.rs) exposing the multiplicity — the
  escape follow-on read stays a GAP). All 1301 symbols across the seven
  codebooks round-trip through a real bit reader in tests.
  [`spectral_decode`](src/spectral_decode.rs) bridges the walk to
  per-coefficient quantised **digits**: [`decompose_symbol`](src/spectral_decode.rs)
  peels a decoded packed symbol into its `dim` base-`radix` digits via the
  already-wired §2.2 [`index_decomp`](src/index_decomp.rs) reciprocal-multiply
  (`radix = level_count + 1`), [`decode_band_digits`](src/spectral_decode.rs)
  runs a whole band, and [`natural_codebook_for`](src/spectral_decode.rs)
  confirms `radix^dim_lo` equals each codebook's symbol count
  `{196,100,49,625,256,243,32}` — the digits feed the existing
  [`reconstruct_band`](src/reconstruct.rs) `values` input. The **level →
  signed-value mapping, the per-band codebook selection, the gain-segment
  and coupling-index VLC reads' codebook, the §4.3 coupling coefficients,
  and the decode-time full-length (N=1024) window** remain recorded gaps
  that gate a real-stream decode to PCM. Companion typed accessors landed
  for the three sibling tables the same round recovered:
  [`bit_alloc::category_bit_cost`](src/bit_alloc.rs) (the `0x8f38` §2.2
  cost LUT), [`transform`](src/transform.rs) (the `0xa1b0` 74×5 iMDCT
  rotation table, kernel use still a no-closed-form GAP), and
  [`mdct::window_builder_consts`](src/mdct.rs) (the `0x8c20`
  `{2.0,0.25,π,0.5}` runtime-window-builder inputs).
- **Backend per-frame-body orchestrator** — [`frame`](src/frame.rs)
  assembles the statically-pinned prefix of the §0 backend frame-body
  stage order (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md`
  §0–§3: gain control → category/quant → spectral VLC dequant) into a
  single walk. [`decode_frame_body`](src/frame.rs) reads the §1.1
  gain-envelope segment count from the [`FrameBitReader`](src/bitreader.rs),
  builds the §2.1 [`SubbandGeometry`](src/subband.rs), then stops
  **precisely** at the §3 spectral-VLC dequant step — whose seven Huffman
  codebooks' per-symbol code/length bytes are runtime-built in `.data`
  BSS at init and absent from the file image (§3.2) — surfacing the typed
  `Error::SpectralCodebookBytesUnavailable` (docs-gap #1775) rather than
  guessing the codebook contents. [`frame_body_prefix`](src/frame.rs)
  returns the recovered [`FrameWalk`](src/frame.rs) (gain count, subband
  geometry, total coded lines, bits consumed) up to the blocker.
  [`Driver::decode_call`](src/driver.rs) /
  [`decode_call_with_flags`](src/driver.rs) now drive every sub-packet
  through the orchestrator on the real-decode gate, replacing the opaque
  `Error::NotImplemented` with the precise §3.2 blocker. The real first
  packet of `FUN_RM_32.rm` carries a well-formed §1.1 gain header (top 6
  bits = 29 → 23 segments), so the walk reaches the §3.2 blocker on real
  data — pinned in `tests/driver_realstream.rs`. The §2.2 category-
  *assignment* loop (`0x8f38` LUT, not extracted), the §3.2 codebook
  bytes, the §4.3 coupling coefficients, and the §5 iMDCT kernel stay
  recorded DOCS-GAPs past the blocker.
- **Post-entropy spectral reconstruction → iMDCT input** —
  [`reconstruct`](src/reconstruct.rs) wires the trace's pinned dequant
  arithmetic *downstream* of the §3.2 entropy blocker (the codebook bytes
  stay the caller's GAP-sourced input).
  [`reconstruct_band`](src/reconstruct.rs) fills one subband's §2.1
  coefficient range with the §3.1 assembly
  `value * sign * dequant_scale * band_gain`;
  [`reconstruct_spectrum`](src/reconstruct.rs) drives that gap-free over
  every subband of a [`SubbandGeometry`](src/subband.rs) to build one
  channel's iMDCT-input spectrum. [`decouple_stereo`](src/reconstruct.rs)
  splits a single coupled spectrum into a
  [`StereoSpectra`](src/reconstruct.rs) pair over a contiguous
  coupling-band range by the §4.2 mirror rotation
  `(out0, out1) = c * (coef[j], coef[Ncoup-1-j])` — one rotation index per
  coupling band (§4.1). [`reconstruct_frame_spectrum`](src/frame.rs) ties
  these into one channel-routed [`FrameSpectrum`](src/frame.rs) (mono
  spectrum or stereo pair) — the iMDCT feed — given the entropy-decoded
  per-band inputs and (for stereo) the §4 [`StereoCoupling`](src/frame.rs)
  inputs. Every BSS-resident input (§3.2 codebook values/signs, §4.3
  coupling `coef`) is caller-supplied; no bytes are guessed.
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
- **Per-band quantiser arithmetic (full §2.2 closed form)** —
  [`quantiser`](src/quantiser.rs) wires the per-band quantiser the worker
  `cook.dll!0x69f0` computes, now assembled into the **complete spec/05
  §2.2 closed form** `level = clip(round(bias[cat] + |q|*step[cat] /
  divisor), level_count[cat])` (`provenance/05` evidence #7: the worker
  does `fabs`, `*step`, `+bias`, `fdiv [0xa7d4]`, `f→i`, clamp to
  `[cat*4+0x8f90]`). [`quantiser_level(&params, q)`](src/quantiser.rs) /
  [`CategoryParameters::quantiser_level`](src/quantiser.rs) compose the
  magnitude form, the [`QUANTISER_DIVISOR`](src/quantiser.rs) divide (the
  f32 `1.0` at RVA `0xa7d4` — carried as a named constant since the binary
  applies it unconditionally), round-to-nearest float→int (negative
  results floored to the unsigned `0` lower bound), and the level-count
  clip in the binary's order. The two underlying primitives remain
  exposed: the magnitude form `bias + |sample| * step` is
  [`band_gain_magnitude`](src/quantiser.rs) (`gain-bias-ramp.meta`: *"the
  worker forms `(bias + |sample| * step)` per band"*) and the level-count
  clip to `0..=level_count-1` is
  [`clip_quantiser_index`](src/quantiser.rs) (`category-level-count.meta`:
  *"used both to size and to clip the per-band quantiser index"*). 14 unit
  tests pin the magnitude symmetry / bias collapse, the clip pass-through
  and top-cap per category, and the full closed form (closed-form match,
  `|q|` symmetry, small-magnitude floor-to-0, large-magnitude top clip,
  divisor-identity). The `q`-supplying §3.1 VLC walk (whose codebook bytes
  are a §3.2 BSS GAP), the `0x8fcc` category-expectation combine (audit
  #17 GAP), and the feed into the inverse MDCT stay recorded DOCS-GAPs.
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
  caller-supplied coefficient table. The **§3.1 dequant tail** is also
  pinned: the three non-zero dequant-scale magnitudes `{0.17678, 0.25,
  0.70711}` at RVA `0x9150` (`provenance/05` evidence #10) are
  [`DEQUANT_SCALE_NONZERO`](src/spectral.rs), and
  [`spectral_coefficient(value, sign_bit, scale, gain)`](src/spectral.rs)
  composes the full pinned reconstruction `coef = value * sign *
  dequant_scale * band_gain` once the codebook `value` is in hand. The
  **§3.1 grouping arithmetic** ties the band geometry to the symbol read:
  [`symbols_for_band(line_count, dim)`](src/spectral.rs) = `ceil(line /
  dim)` and [`coefficients_for_symbols`](src/spectral.rs) = `symbols *
  dim` (each VLC symbol expands to `dim` coefficients), and
  [`SubbandGeometry::band_symbol_count`](src/subband.rs) sizes a band's
  symbol read from its §2.1 line count. The per-symbol codebook
  code/length **bytes** (§3.2), the per-coupling-width rotation
  **coefficient values** (§4.3), and the dim→codebook + lo/hi-branch
  *selection* (not statically unambiguous — the symbol counts do not
  factor as a unique `base^dim`) are recorded GAPs, surfaced as RVA
  constants but with no retyped numbers, pending a dynamic-BSS-dump
  Validator round. 27 unit tests pin the codebook counts,
  vector-dimension sequences, sign LUT, dequant-scale triple, coefficient
  assembly, grouping coverage, and the mirror-index self-inverse /
  energy-pan invariants.
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
  [`GAIN_POS_WINDOW`](src/gain.rs). The **§1.2 application** is also
  wired: [`GainSegment`](src/gain.rs) models a `(position, gain_index)`
  event, [`expand_gain_envelope`](src/gain.rs) expands a segment set into
  one factor per sub-block by the piecewise-constant hold-forward (unity
  before the first segment, each segment's `2^(index/2)` factor held from
  its position to the next), and [`apply_gain_blocks`](src/gain.rs) /
  [`apply_gain_envelope`](src/gain.rs) multiply the per-sub-block profile
  into the time-domain samples (the characteristic Cook **post-transform**
  time-varying gain; a zero sub-block count surfaces the new
  `Error::GainBlockCountZero`). 23 unit tests pin the count-bias
  endpoints, the gain-index → factor resolution, and the §1.2
  expansion/application (flat unity default, single/multi-segment hold,
  position sorting, past-window inertness, sub-block scaling, the
  non-dividing tail). The per-segment *record reads* themselves (position
  + gain index, via the §3.2 BSS-gated VLC walk `cook.dll!0x3a50`) stay a
  recorded DOCS-GAP — the application closed form is pinned, reading the
  segment list off the bitstream is not.
- **Division-free quantiser-index decomposition (§2.2)** —
  [`index_decomp`](src/index_decomp.rs) wires the per-band dequant worker
  `cook.dll!0x44a0` reciprocal-multiply that decomposes a packed quantiser
  index into its per-coefficient digits without a hardware division
  (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §2.2,
  `provenance/05` evidence #7, `tables/README.md` row `0x8fac`). The seven
  Q-format constants
  `{0x12493, 0x1999a, 0x24925, 0x33334, 0x40000, 0x55556, 0x80000}`
  ([`INDEX_RECIP`](src/index_decomp.rs)) are the `ceil(2^20 / n)`
  reciprocals of the seven per-category radices `{14, 10, 7, 5, 4, 3, 2}`
  ([`INDEX_RADIX`](src/index_decomp.rs), recovered by arithmetic and
  verified against the constants). [`reciprocal_quotient`](src/index_decomp.rs)
  applies the pinned `(idx * recip) >> 0x14` multiply-shift and
  [`decompose_index`](src/index_decomp.rs) returns the
  `(quotient, remainder) = (idx / n, idx mod n)` pair — the
  `(codebook-symbol carry, in-symbol-position digit)` halves §2.2 names —
  range-checked behind [`Error::IndexRecipOutOfRange`](src/lib.rs). The
  exact field-decomposition *role* stays the recorded `tables/README.md`
  GAP and the §3.2 BSS codebook bytes the digits index remain GAP; the
  arithmetic primitive and the radix recovery are pinned.
- **Joint-stereo coupling-control read (§4.1)** —
  [`coupling_control`](src/coupling_control.rs) wires the stereo
  coupling-control read `cook.dll!0x3d10`
  (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §4.1,
  `provenance/05` evidence #12 / #13). The leading flag bit selects the
  per-band coupling-index read mode ([`read_coupling_mode`](src/coupling_control.rs)
  → [`CouplingReadMode`](src/coupling_control.rs)): **set** → VLC
  (`cook.dll!0x3a50` over the §3.2 BSS codebooks, the recorded blocker),
  **clear** → a fixed-width `read-n-bits(coupling_bits)` field with
  `n =` context `+0x1c`. The fixed-width branch
  ([`read_fixed_coupling_index`](src/coupling_control.rs)) is fully
  implemented from the [`FrameBitReader`](src/bitreader.rs) and yields one
  rotation index `j` in `0..Ncoup` (`Ncoup = 1 << coupling_bits`), the
  angle quantiser the §4.2 mirror split consumes;
  [`read_coupling_index`](src/coupling_control.rs) surfaces
  `Error::SpectralCodebookBytesUnavailable` for the VLC branch rather than
  guessing. The context offsets `+0x1c` (coupling bit width) and `+0x18`
  (per-channel subband count, [`CTX_SUBBAND_COUNT_OFFSET`](src/coupling_control.rs))
  are surfaced as named constants. The **coupling-band boundary
  derivation** (§4.1 gives no closed form) stays a recorded DOCS-GAP — the
  caller supplies the contiguous coupling-band range.
- **Inverse-transform output stage (§5)** —
  [`output_stage`](src/output_stage.rs) wires the windowing + overlap-add
  that surrounds the iMDCT kernel
  (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §5, spec/01
  §5.1, `provenance/05` evidence #14).
  [`apply_window`](src/output_stage.rs) /
  [`windowed`](src/output_stage.rs) multiply a time-domain block
  point-wise by the stored Princen-Bradley window
  ([`mdct_half_window`](src/mdct.rs), whose `w[k]² + w[N-1-k]² = 1` TDAC
  identity the `mdct-windows.meta` validates);
  [`overlap_add`](src/output_stage.rs) sums two equal-length windowed
  contributions and [`overlap_add_weighted`](src/output_stage.rs) applies
  the L/R combine mix weights `0.5`
  ([`OVERLAP_MIX_WEIGHT_HALF`](src/output_stage.rs), RVA `0x8c0c`) and
  `0.75` ([`OVERLAP_MIX_WEIGHT_THREE_QUARTER`](src/output_stage.rs), RVA
  `0x8c10`). [`window_and_gain`](src/output_stage.rs) composes the §5
  per-block sequence (window then §1 [`apply_gain_blocks`](src/gain.rs)
  gain-scale) in the binary's order. The **iMDCT kernel itself**
  (`cook.dll!0x5b70`, the `0xa1b0` rotation table with no validated closed
  form — audit #16) stays the recorded GAP and is a caller input; the
  long/short window selection and per-flavor weight routing also stay
  GAPs. New typed errors `Error::OutputWindowLengthMismatch` /
  `Error::OverlapAddLengthMismatch`.
- **§5 MLT/IMLT synthesis back end (spectra → PCM bytes)** — the entire
  post-entropy half of the backend frame decode is now assembled,
  milestone by milestone.
  [`mdct_full_window`](src/mdct.rs) mirror-completes each stored
  monotone-decreasing half-window into its full `2L`-tap symmetric
  Princen-Bradley window (the hop-TDAC identity `W[k]² + W[k+L]² = 1`
  follows from the pinned in-row identity — values bit-identical, only
  re-ordered). [`imlt_direct`](src/imlt.rs) / [`mlt_direct`](src/imlt.rs)
  wire the definition-level inverse/forward MLT the stage is pinned *as*
  (spec/01 §5.1 *"inverse MDCT"*): the TDAC alias symmetry, linearity,
  the tight-frame composition `MLT∘IMLT = 2·id`, and windowed
  overlap-add **perfect reconstruction** over the stored 3/7/15/31
  windows are pinned by tests (the binary's fast kernel — `0x5b70` +
  `0xa1b0`, audit #16 — stays the recorded GAP; the
  normalisation-convention caveat is documented, unverifiable until the
  §3.2 entropy GAP lands). [`Synthesizer`](src/synthesis.rs) is the
  streaming per-channel §5 state machine (iMLT → window → §1.2 gain →
  overlap-add, zeroed warm-up tail); [`pcm`](src/pcm.rs) converts to the
  validator-pinned 16-bit LE interleaved format;
  [`CallPcmAssembler`](src/assembler.rs) is the `+0x20` carry-buffer
  cadence queue (the 144-call walk reproduces the pinned 8 192 + 143 ×
  20 480 = 2 936 832-byte accounting with the constant 12 288-byte
  three-frame backlog); [`SynthesisBackend`](src/backend.rs) +
  [`Driver::synthesized_call`](src/driver.rs) assemble it all into the
  **resume-from-blocker `RADecode` analog** — caller-supplied
  post-entropy spectra (the §3.2 GAP input) in, per-call PCM out,
  session cursor advanced. Pinned end-to-end in
  `tests/synthesis_realstream.rs`: the 144-real-packet silent-spectra
  walk is **byte-identical to the observe-gate output** call-by-call,
  and a mono hop-64 roundtrip reconstructs a source signal through the
  full spectra → PCM-bytes path. The frame-length
  (`2 × samples_per_frame`) synthesis window is not among the five
  extracted rows and stays a caller-supplied GAP input.
- **§1.1 real-data finding (recorded docs-gap)** — 12 of the validated
  stream's 144 call heads carry a leading 6-bit field `< 6`, which
  biases negative under the spec/05 §1.1 *"field = segment_count + 6"*
  reading (packet 0 opens with the well-formed raw 29 → 23 segments,
  but packets 4/5 open with raw 4); additionally packet 0's slot-1
  boundary is not a well-formed frame head, consistent with the spec/01
  §5 pin that the backend is invoked **once per call** with
  carry-buffer consumption for the remaining sub-packets. Both pinned
  by `tests/synthesis_realstream.rs`; `decode_call`'s real-decode gate
  now walks the frame body once per call at the call head, and the
  negative-bias semantics await a docs clarification.

## Not yet implemented

The real-decode half of the backend frame-decode **drives the
frame-body orchestrator** ([`frame`](src/frame.rs)) on the real-decode
gate: each sub-packet runs the statically-pinned prefix (the §1.1
gain-envelope segment count and the §2.1 subband → coefficient-range
geometry) and stops precisely where the trace runs out — the
**pre-spectral bitstream read layout**. The seven spectral codebooks
were **recovered** and are now vendored + wired
(`tables/spectral-codebook-{codes,code-lengths}.csv`,
[`codebook`](src/codebook.rs) / [`spectral_decode`](src/spectral_decode.rs)),
so given a per-band category list the §3 read runs end-to-end
([`decode_spectrum`](src/frame_decode.rs)) through the §4 coupling split
and §5 synthesis to PCM. What `spec/05` does **not** pin is how the frame
head reaches that §3 read on a real stream: which recovered codebook the
§1.2 gain-index / §2.2 quant-index VLC reads select, and how the
category-assignment value array `v[]` is formed from the bitstream (the
input the recovered [`category_assignment`](src/category_assignment.rs)
loop consumes). The walk therefore **stops at** that read-layout gap,
surfacing the typed `Error::SpectralCodebookBytesUnavailable` (the
variant name kept for compatibility; it now denotes the read-layout gap,
not a missing-bytes gap) rather than guessing. The observe half (gate
bit `1`) is implemented: zeroed overlap-add output per validation/04
§4.3. `Driver::decode_call` / `Driver::decode_call_with_flags` validate
buffer sizes, run stages 1+2, then drive the orchestrator on the
real-decode gate. Consumers that already hold the post-gain reader
position and a category list run the whole §3→§5 chain via
[`decode_frame_spectrum`](src/frame_decode.rs) /
[`decode_frame_spectrum_assigned`](src/frame_decode.rs) (the latter
computes the category list from `(values, budget, refinement_bound)` and
routes mono/stereo in one call). The `oxideav_core` registration glue and
the cookie
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
wired in [`spectral`](src/spectral.rs), and the §0–§3 prefix is now
**assembled into the running real-decode walk** by
[`frame`](src/frame.rs) (the bit-reader state machine, the
gain-envelope segment-count read, and the §2.1 subband-geometry build).
The entire **post-entropy reconstruction arithmetic** — the §3.1 dequant
band fill, the gap-free spectrum assembly over the §2.1 geometry, the §4.2
stereo decouple, and their channel-routed integration into a `FrameSpectrum`
iMDCT feed — is wired in [`reconstruct`](src/reconstruct.rs) /
[`frame`](src/frame.rs). The **§5 synthesis back end past the iMDCT feed
is fully wired** ([`imlt`](src/imlt.rs), [`synthesis`](src/synthesis.rs),
[`pcm`](src/pcm.rs), [`assembler`](src/assembler.rs),
[`backend`](src/backend.rs), `Driver::synthesized_call`): given the
entropy-decoded spectra, the crate produces the per-call 16-bit PCM
bytes at the validator-pinned cadence. The recovered runtime N=1024 MDCT
window/twiddles and the §4.3 coupling rotation table are vendored and
**cross-checked against their closed forms** (the twiddles are the MDCT
rotation `(cos, sin)(π(k+¼)/1024)`, the long window is
`(1/√512)·cos(πk/1024)`, and the coupling table is the quarter-turn sweep
`cos(jπ/256)` / `sin(rπ/256)`), so the entire §3→§5 chain is assembled;
the codebook bytes are no longer the blocker.

**What is not yet wired** is the frame's **pre-spectral bitstream read
layout** — the step that positions the reader at the §3 data on a real
stream and *produces* the decoded values. `spec/05` does not pin which
recovered codebook the §1.2 gain-index / §2.2 quant-index VLC reads
select, nor how the category-assignment value array `v[]` is formed from
the bitstream; the real-decode gate stops there, typed as
`Error::SpectralCodebookBytesUnavailable` (name kept for compatibility;
it now denotes the read-layout gap, not a missing-bytes gap). The observe
half (gate bit `1`, zeroed overlap-add output per validation/04 §4.3) is
implemented. Further recorded gaps ride alongside: the iMDCT kernel's
`0x8fcc`/`0xa1b0` rotation-table 2-D layout (spec/01 §6; the wired
transform is the canonical TDAC-perfect-reconstruction closed form, its
**normalisation/sign convention** unverifiable until the read layout
lands — caveat in [`imlt`](src/imlt.rs)); the **frame-length synthesis
window** (`2 × samples_per_frame` taps — only the 3/7/15/31/64 short rows
and the N=1024 long row are recovered); the **§1.1 negative-bias
semantics** (12 of 144 real call heads carry a leading 6-bit field `< 6`,
contradicting the `segment_count + 6` reading — see the real-data finding
above); and the **`+0x20` carry-buffer mechanics** (where the four
remaining frame bitstreams sit inside a call; the validated stream shows
the 93-byte slot boundaries after slot 0 are not frame heads).
