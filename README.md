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
transform is assembled end-to-end** around the round-9 §0.2 wire order:
the fixed-width frame head (sub-packet flag, coupling control, envelope
seed), the §2.2 category-assignment / bit-allocation loop — now
reproducing the vendor decoder's own categories **bit-exactly on all
three traced real frames** (34/34 bands each) — the §3 spectral entropy
read over the recovered codebooks, the §4 joint-stereo pan split over
the round-10 `.rdata` pan tables, and the §5 windowing / overlap-add
into audible 16-bit PCM. What still gates a raw-bytes real-stream
decode: the **envelope VLC tree family** (the 31 trees at
`backend+0x44c8` that carry the `Nb − 1` per-band envelope values) and
the **coupling-index VLC tree** are not among the staged tables; the
iMDCT kernel's exact block cadence / normalisation and the per-band
`v[b]` → reconstruction-gain law are likewise open (see "Not yet
implemented"). Frames whose envelope was captured live (three are
staged) walk end-to-end through an injection API.

The rebuild draws only from the strict-isolation clean-room workspace
under `docs/audio/cook/` (binary-derived structural spec + extracted
numeric facts tables + real-stream validation).

## What works

- **§2.2 category-assignment / bit-allocation loop (`cook.dll!0x4800`,
  the last routing GAP)** — Cook does not transmit per-band spectral
  categories; they are **computed** in-decoder from a per-band value
  array `v[]` and the frame bit budget.
  [`category_assignment`](src/category_assignment.rs) is that loop, from
  `docs/audio/cook/provenance/08-cook-category-assignment.md`,
  `provenance/09` §3 (the live-frame captures) and
  `tables/category-assignment-params.csv` / `live-frame-*.csv`. The
  **base pass** `cat[b] = clip((32 + off − v[b]) >> 1, 0, 7)` picks the
  global offset by the documented `K = 32` slack rule (`refine one
  category finer while total_cost + K < budget`), reproducing the
  vendor's own `cook.dll!0x4800` output across the provenance/08 budget
  sweeps **and** landing on the traced `off = −3` for live packet 2.
  The **Stage-2 refinement** is the round-9 parity-sweep walk
  ([`refine_categories`](src/category_assignment.rs)): unit offset
  steps below the base, per step the even-`t` class
  (`t = K + off − v[b]`) moving one category finer in ascending band
  order, one `M − 1` step per candidate, applying only while
  `Σcost + Δ ≤ 2 × budget − 6` — the `0x8f38` cost LUT is denominated
  in **half-bits** (the slack constant is fitted; its `{5, 6}` window
  is recorded and pinned by a test). Validated against the vendor's own
  real-frame output: all three traced frames reproduce **34/34
  categories bit-exactly** (packets 16/17 stop mid-sweep, so this pins
  the step rule, not just an offset), the landing totals are the
  documented 1124/1130/1126 against `2 × budget`, and packet 2's five
  sweep sizes and first two sweep memberships match `provenance/09` §3
  exactly. [`decode_spectrum_assigned`](src/frame_decode.rs) computes
  the per-band [`BandCategory`] list from `(values, budget,
  refinement_bound)` and runs it straight through the
  codebook-by-category §3 band decode.

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
  [`reconstruct_band`](src/reconstruct.rs) `values` input. Of the gaps
  this bullet once listed, the level → signed-value mapping, the
  codebook selection (the category), the §4.3 coupling coefficients and
  the N = 1024 window are all since recovered/wired; the **envelope and
  coupling-index VLC tree contents** remain the recorded gaps (see "Not
  yet implemented"). Companion typed accessors:
  [`bit_alloc::category_bit_cost`](src/bit_alloc.rs) (the `0x8f38` §2.2
  cost LUT — half-bits per band), [`transform`](src/transform.rs) (the
  `0xa1b0` 74×5 iMDCT rotation table, kernel use still a
  no-closed-form GAP), and
  [`mdct::window_builder_consts`](src/mdct.rs) (the `0x8c20`
  `{2.0,0.25,π,0.5}` runtime-window-builder inputs).
- **Backend per-frame-body orchestrator — the §0.2 wire walk** —
  [`frame`](src/frame.rs) rebuilds the frame body around the wire order
  round 9 pinned by behavioural trace
  (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §0.2,
  `tables/frame-read-layout.csv`): [`read_frame_head`](src/frame.rs)
  consumes fields 1–4 — the 1-bit sub-packet flag, the coupling mode
  flag, `Ncoupband` fixed-width coupling indices (the VLC branch
  surfaces the typed `Error::CouplingIndexTreeUnavailable`), the 6-bit
  envelope seed — and [`decode_frame_body`](src/frame.rs) continues
  through the field-5 envelope values (the unstaged 31-entry VLC tree
  family; a captured `v[]` + cursor resumes the walk through
  [`EnvelopeInjection`](src/frame.rs) — three real frames' captures are
  vendored), the 7-bit frame scalar, the §2.2 allocator with the
  round-9 budget rule `budget = bit_limit − cursor`, the computed
  categories, the §3 codebook-by-category spectral read over the
  20-line band geometry, and the §4 pan split.
  [`FrameLayout`](src/frame.rs) carries the traced per-flavor
  parameters ([`validated_stereo`](src/frame.rs): `Nb = 34`,
  `coupling_bits = 4`, `Ncoupband = 16`, `M = 128`;
  [`for_flavor_geometry`](src/frame.rs) refuses untraced flavors) and
  [`CouplingMap`](src/frame.rs) makes the unpinned §4.1 coupling band
  range an explicit input (default hypothesis: start band 2, two
  subbands per index — `2 + 16 × 2 = 34` fits every traced number;
  flagged, not fact). Validated by a synthetic 93-byte stereo frame
  shaped exactly like traced packet 2 (scalar at bits 172..179, budget
  565, categories == the live capture, bit-exact total consumption) and
  a five-frame streaming test into non-silent PCM
  (`tests/frame_walk_stream.rs`).
  [`Driver::decode_call`](src/driver.rs) /
  [`decode_call_with_flags`](src/driver.rs) drive every call through
  the walk on the real-decode gate, surfacing the typed
  `Error::EnvelopeValueTreeUnavailable` /
  `Error::CouplingIndexTreeUnavailable` gaps (the retired
  `SpectralCodebookBytesUnavailable` blocker's narrowed successors) —
  pinned on the real packets in `tests/driver_realstream.rs`.
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
  category-index LUT, and the five per-coupling-width §4.3 pan tables
  of lengths 3 / 7 / 15 / 31 / 63). Each loader is `OnceLock`-cached
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
  [`subband`](src/subband.rs) wires the band → line map the §2.2 dequant
  walk, the §3 spectral read and the §4 coupling split drive off. The
  band width is **derived from the staged category tables**: `dim_lo`
  (`{2,2,2,4,4,5,5}`, the codebook vector dimension) times `dim_hi`
  (`{10,10,10,5,5,4,4}`, the per-band symbol count) is **20 for every
  category**, so a coded band is 20 spectral lines
  ([`LINES_PER_BAND`](src/subband.rs)): band `b` occupies
  `[20·b, 20·b + 20)`, the live `Nb = 34` bands of the validated flavor
  cover 680 of its 1024 transform lines, and the 51-entry `0x8c40` LUT
  spans exactly `floor(1024 / 20)` whole subbands
  ([`MAX_SUBBANDS`](src/subband.rs)).
  [`SubbandGeometry::new`](src/subband.rs) answers per-band
  [`line_range`](src/subband.rs) / [`line_count`](src/subband.rs) /
  [`total_coded_lines`](src/subband.rs) and
  [`band_symbol_count`](src/subband.rs) `= ceil(20 / dim_lo) = dim_hi`
  (pinned by a test for every category). An earlier reading took
  spec/05 §2.1's "read as `[band*4 + 0x8c40]` … start spectral line"
  sentence literally (bands 0..11 one line wide, 20 subbands covering
  15 lines) — ruled out by the 20-line product; the LUT stays typed in
  [`bit_alloc`](src/bit_alloc.rs) under its pinned category/position
  role and what it indexes per subband at decode time is a recorded
  docs question (see "Not yet implemented"). The companion `0.5`
  scalar at `0x8c3c` is surfaced as
  [`SUBBAND_HALF_SCALAR`](src/subband.rs).
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
  divisor-identity). The `0x8fcc` category-expectation combine (audit
  #17 GAP) and the feed into the inverse MDCT stay recorded DOCS-GAPs.
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
- **§4.3 per-coupling-width pan-coefficient tables (round-10
  relabel)** — the five short `.rdata` tables at `cook.dll!0x8d0c`
  this crate once keyed as "MDCT half-windows" are, per
  `docs/audio/cook/provenance/10-cook-coupling-pan-label.md`, the
  joint-stereo **pan-coefficient** tables of spec/05 §4.3: one per
  coupling width `w = 2..=6`, each of length `(1 << w) − 1` (extents
  read from the `0x8ee8` dispatch pointer array, which the five tables
  end exactly at), all 119 values satisfying the constant-power identity
  `t[j]² + t[n−1−j]² = 1` to `< 1e-6`, every row strictly decreasing
  with `1/√2` at its centre. The range has exactly one consumer in the
  image — the §4.2 stereo split at `cook.dll!0x3e96` — and the round-9
  ablation moved 3060/4096 PCM bytes by zero-filling the
  `coupling_bits`-selected row while the other four were bit-inert.
  [`coupling::CouplingPanWidth`](src/coupling.rs)
  ([`from_bits`](src/coupling.rs) over the per-flavor `coupling_bits`,
  [`table_len`](src/coupling.rs) `= (1 << w) − 1`,
  [`rva`](src/coupling.rs) derived from the table head),
  [`coupling_pan_table`](src/coupling.rs),
  [`coupling_pan_pair`](src/coupling.rs) `(t[j], t[Ncoup−1−j])` and
  [`split_coupled_recovered`](src/coupling.rs) wire the §4.2 split over
  the vendored `tables/coupling-pan-coeffs.csv`; the old window-role
  API and the superseded de-permuted 512-entry "§4.3 table" built from
  the round-8 init rotation buffers are removed (those two buffers stay
  vendored as recovered facts whose consuming stage is unpinned).
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
  form: [`coupling_table_len`](src/spectral.rs) is
  `Ncoup = (1 << coupling_bits) − 1` (the round-9/10 table extents),
  [`mirror_partner_index`](src/spectral.rs) is the
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
  cursor/word/position lockstep. The reader's context offsets follow the
  round-9 correction (`+0x47ac..+0x47b8` — observed as the stores a
  live `RADecode` makes); [`skip_bits`](src/bitreader.rs) supports the
  envelope-injection walk. The frame body that drives the reader is the
  §0.2 walk of [`frame`](src/frame.rs).
- **Gain / scale DSP primitives (§1)** — [`gain`](src/gain.rs) keeps
  the ladder resolution and profile primitives after the round-9
  withdrawal of the old §1.1 wire reading (the head worker
  `cook.dll!0x4b50` fills the allocator's `v[]`; there is no
  segment-count field — this crate's own 12-of-144 real-stream
  underflow finding foreshadowed exactly that).
  [`gain_factor_for_index`](src/gain.rs) resolves a gain index to
  `2^(index/2)` via the `0x93f8` `sqrt(2)` ladder indexed at its centre
  (`1.0` at element 63 — the `0x94f4` positive-window sub-pointer of
  evidence #3), with the `{1.0, √2, 2.0, 2√2, 4.0}` window exposed as
  [`GAIN_POS_WINDOW`](src/gain.rs); [`GainSegment`](src/gain.rs) /
  [`expand_gain_envelope`](src/gain.rs) /
  [`apply_gain_blocks`](src/gain.rs) /
  [`apply_gain_envelope`](src/gain.rs) keep the §1.2-shaped
  piecewise-constant expansion + post-transform time-domain multiply as
  caller-input DSP (no wire source for gain events is pinned; whether
  this flavor carries a time-domain gain envelope at all is open,
  spec/05 §1.2).
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
  (`cook.dll!0x3a50` over an unstaged coupling-index tree),
  **clear** → a fixed-width `read-n-bits(coupling_bits)` field with
  `n =` context `+0x1c`. The fixed-width branch
  ([`read_fixed_coupling_index`](src/coupling_control.rs)) is fully
  implemented from the [`FrameBitReader`](src/bitreader.rs) and yields one
  rotation index `j` (the walk rejects the one value past the pan
  table's `Ncoup = (1 << coupling_bits) − 1` entries — the traced
  width-4 indices stay `0..=14`);
  [`read_coupling_index`](src/coupling_control.rs) surfaces
  `Error::CouplingIndexTreeUnavailable` for the VLC branch rather than
  guessing. The context offsets `+0x1c` (coupling bit width) and `+0x18`
  (per-channel subband count, [`CTX_SUBBAND_COUNT_OFFSET`](src/coupling_control.rs))
  are surfaced as named constants. The **coupling-band boundary
  derivation** (§4.1 gives no closed form) stays a recorded DOCS-GAP —
  [`CouplingMap`](src/frame.rs) makes it an explicit input.
- **Inverse-transform output stage (§5)** —
  [`output_stage`](src/output_stage.rs) wires the windowing + overlap-add
  that surrounds the iMDCT kernel
  (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §5, spec/01
  §5.1, `provenance/05` evidence #14).
  [`apply_window`](src/output_stage.rs) /
  [`windowed`](src/output_stage.rs) multiply a time-domain block
  point-wise by the apodisation window (a caller-supplied slice — the
  runtime-built window, [`long_full_window_unit`](src/mdct.rs) for the
  recovered N = 1024 flavour);
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
  per-flavor weight routing also stays a GAP. Typed errors
  `Error::OutputWindowLengthMismatch` / `Error::OverlapAddLengthMismatch`.
- **§5 MLT/IMLT synthesis back end (spectra → PCM bytes)** — the entire
  post-entropy half of the backend frame decode is now assembled,
  milestone by milestone.
  [`imlt_direct`](src/imlt.rs) / [`mlt_direct`](src/imlt.rs)
  wire the definition-level inverse/forward MLT the stage is pinned *as*
  (spec/01 §5.1 *"inverse MDCT"*): the TDAC alias symmetry, linearity,
  the tight-frame composition `MLT∘IMLT = 2·id`, and windowed
  overlap-add **perfect reconstruction** across several hop sizes
  (exact-TDAC test windows) and the recovered N = 1024 window are
  pinned by tests (the binary's fast kernel — `0x5b70` +
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
  post-entropy spectra in, per-call PCM out,
  session cursor advanced. Pinned end-to-end in
  `tests/synthesis_realstream.rs`: the 144-real-packet silent-spectra
  walk is **byte-identical to the observe-gate output** call-by-call,
  and a mono hop-64 roundtrip reconstructs a source signal through the
  full spectra → PCM-bytes path. The vendor's block cadence between a
  frame's 680 coded lines and the recovered hop-512 window stays tied
  to the recorded kernel GAP (see "Not yet implemented").
- **Real call-head statistics fit the §0.2 flag reading** — the
  12-of-144 negative-bias finding this crate recorded against the old
  §1.1 reading is resolved by the round-9 layout: re-read under §0.2
  the head bits are the 1-bit sub-packet flag (0 on 139 of 144 call
  heads — the traced frames all carried 0) and the coupling mode flag
  (the VLC branch on 107 of 144, matching the trace observing both
  branches live). Pinned in `tests/synthesis_realstream.rs`, with the
  historical 12-of-144 statistic kept as the pointer that foreshadowed
  the withdrawal.

## Not yet implemented

The real-decode gate drives the assembled **§0.2 frame walk**
([`frame`](src/frame.rs)) on every call: the fixed-width head (sub-packet
flag, coupling control, envelope seed) parses on real packets, and the
walk stops at the first of two **unstaged VLC tree families**:

- the **envelope value trees** — field 5's `Nb − 1` per-band values are
  read through a separate 31-entry tree family at `backend+0x44c8`
  (tree `max(0, k − 3)` for symbol `k`; spec/05 §1.1) whose per-symbol
  code/length contents are not among the staged tables
  (`Error::EnvelopeValueTreeUnavailable`);
- the **coupling-index tree** — field 3's VLC branch (mode flag `1`,
  taken by 107 of the validated stream's 144 call heads)
  (`Error::CouplingIndexTreeUnavailable`).

Both are data gaps of exactly the kind the docs Extractor's runtime
dumps have closed before (the seven spectral codebooks came back the
same way). Frames whose envelope was captured live resume through
[`EnvelopeInjection`](src/frame.rs) — the three staged captures walk
end-to-end — and everything downstream of field 5 is assembled: scalar,
budget rule, allocator (vendor-exact on the live frames), spectral read,
pan split, synthesis to PCM.

Recorded open questions that ride alongside (typed as caller inputs, not
guessed): the **per-band reconstruction gain law** (how `v[b]` maps to
the §1–§2 "per-band gain" the dequant applies — `band_gains` is a
caller input); the **§4.1 coupling band range** (which subbands each
coupling index covers — [`CouplingMap`](src/frame.rs), with the
`2 + 16 × 2 = 34` hypothesis flagged) and the **uncoupled low bands'
stereo routing**; the **iMDCT kernel** (`cook.dll!0x5b70` + the `0xa1b0`
rotation table, no validated closed form) and its **block cadence**
between a frame's 680 coded lines and the recovered hop-512 window —
the wired transform is the canonical TDAC closed form with a documented
normalisation caveat; the **1-bit sub-packet flag and 7-bit frame
scalar semantics** (positions pinned, meaning open); the **wire ↔
frame-buffer correspondence** for the traced frames (see the caveat
below); the **`+0x20` carry-buffer mechanics** (where the four
remaining frame bitstreams sit inside a 465-byte call); and the
`oxideav_core` registration glue plus the non-extended
(`0x01000001`/`0x01000002`) and multichannel (`0x02000000`) cookie
layouts (typed in `CookCookie::parse`).

### A recorded caveat on the staged field values

The round-9 extractor re-derived the traced frames' field *values*
(envelope seed 17/55/27, frame scalar 109/89/103, coupling indices)
from a decoder-memory frame-buffer dump at the recorded bit offsets.
Two observations this crate made while consuming the staging suggest
that derivation needs re-verification: the staged seed values differ
from the stack-captured `v[0]` (17 vs 25, 55 vs 29, 27 vs 28) with no
pinned transform between them, and no 93-byte sub-packet of the wire
stream parses under the pinned layout with the staged values (nor does
any wire alignment reproduce the staged spectral consumption under the
recovered codebooks). The field *widths and order* (bit-cursor deltas)
and the stack-captured `v[]`/`cat[]`/budgets are unaffected — the
category-assignment validation rests only on those.
