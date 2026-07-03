//! Pure-Rust RealAudio Cook audio codec.
//!
//! **Clean-room rebuild.** This is a fresh orphan `master`; the previous
//! implementation was retired alongside the OxideAV docs audit dated
//! 2026-05-06. See `README.md` for the rebuild scope.
//!
//! Round 1 landed the stream-configuration front end: the per-flavor
//! [`flavor::flavor_record`] geometry-table loader and the extradata
//! [`cookie::CookCookie`] parser, with a cross-check that a parsed cookie
//! describes the same configuration as its named flavor record.
//!
//! Round 2 vendors the remaining eight DSP parameter tables extracted
//! from the binary into [`tables`]: the two 127-entry power-of-two
//! ladders, the per-category gain-step and gain-bias ramps, the per-
//! category level-count clip, the 11-entry reciprocal table, the
//! 51-entry monotone category-index LUT, and the five Princen-Bradley
//! MDCT half-windows of lengths 3 / 7 / 15 / 31 / 64. Each loader is
//! `OnceLock`-cached and self-validates against the constraint stated
//! in its `.meta` provenance.
//!
//! Round 3 wires the validator-confirmed open-time geometry: the
//! [`init`] module takes a parsed [`CookCookie`], the two
//! [`init::Descriptor`] scalars (`channels_divisor` = descriptor
//! `+0x06`, `sub_packet_size` = descriptor `+0x0a`), and a named
//! [`FlavorRecord`], cross-checks them against each other, and
//! derives the per-`RADecode`-call geometry the driver `0x1260`
//! computes at runtime (`sub_packets_per_call = frame_bytes /
//! sub_packet_size`, `pcm_bytes_per_call = sub_packets_per_call ×
//! samples_per_frame × channels × 2`). Pinned end-to-end against the
//! real `FUN_RM_32.rm` stream (`tests/realstream_decode_config.rs`).
//!
//! Round 4 vendors that real stream into the crate as
//! `tests/fixtures/FUN_RM_32.rm` and bundles a wire-level integration
//! test (`tests/realstream_fixture.rs`) that parses the RealMedia
//! container directly, walks every audio packet, extracts the cookie
//! from the audio `MDPR`'s type-specific-data, and feeds the result
//! through [`DecodeConfig`] — cross-checking the fixture's SHA-256,
//! every top-level chunk size, the 144-packet × 465-byte payload
//! framing, and the validator's 2 936 832-byte total PCM accounting
//! against the bundled wire bytes byte-for-byte.
//!
//! Round 6 wires the first real decode-pipeline byte stage: the
//! per-buffer XOR [`descramble`]. `RADecode`'s first byte-touching step
//! is a word-wise (32-bit) XOR pass over the input, gated by the
//! common-mode flag at context `+0x30` ([`CommonMode`], toggled on by
//! `RASetComMode`). The default is off, so [`descramble_packet`] returns
//! the packet verbatim and zero-copy — exactly the validated real-stream
//! path (`validation/04` §4.3 / §5). The toolkit grows; the public
//! decode path still returns [`Error::NotImplemented`].
//!
//! Round 8 wires the second structural decode-pipeline stage: the per-
//! `RADecode` sub-packet split + PCM offset accounting in [`subpacket`].
//! After the optional XOR descramble, `RADecode` (`cook.dll!0x1260`)
//! partitions its `frame_bytes`-byte input into `sub_packets_per_call`
//! consecutive fixed-stride slots of `sub_packet_size` bytes each
//! ([`SubPacketLayout::iter_call`], [`SubPacketLayout::slot_byte_range`],
//! [`SubPacketLayout::call_byte_range`]) and emits PCM at the validator-
//! pinned cadence (first-call 8 192-byte overlap-add warm-up,
//! steady-state 20 480 bytes/call —
//! [`SubPacketLayout::pcm_offset_for_call`]). Pinned end-to-end against
//! the 144 real packets of `FUN_RM_32.rm` in
//! `tests/subpacket_realstream.rs`. The backend frame-decode + carry-
//! buffer state machine (stage 3, `[backend_vtable + 0x0c]`) is still
//! [`Error::NotImplemented`].
//!
//! Round 9 wires the third structural decode-pipeline stage: the
//! `RADecode` call-sequence session state in [`session`]. A
//! [`CallSession`] holds a [`SubPacketLayout`] plus the running call
//! counter / PCM cursor and exposes
//! ([`CallSession::next_call_expected_input_len`],
//! [`CallSession::next_call_pcm_bytes`],
//! [`CallSession::next_call_pcm_byte_range`]) for sizing the next
//! call's input/output, and [`CallSession::advance_one_call`] for
//! accounting a completed call (validates both lengths against the
//! validator-pinned per-call budget — warm-up on call 0, steady-state
//! thereafter — and increments the cursor). Pinned end-to-end against
//! the 144-call sequence of `FUN_RM_32.rm` in
//! `tests/session_realstream.rs`: walking the full sequence produces
//! the validator's pinned `2 936 832`-byte total. The backend
//! frame-decode itself (the bitstream + transform pipeline behind
//! `[backend_vtable + 0x0c]`) remains [`Error::NotImplemented`].
//!
//! Round 10 wires the per-call orchestrator: [`Driver`] bundles a
//! [`DecodeConfig`], a [`CommonMode`] toggle, and an embedded
//! [`CallSession`] into the `RADecode`-equivalent entry point spec/01
//! §5 describes. [`Driver::prepare_call`] runs stages 1+2
//! (descramble + sub-packet split), returns a [`PreparedCall`] that
//! exposes the descrambled bytes and the sub-packet iterator, and
//! does *not* advance the cursor; [`Driver::advance_after_decode`]
//! accounts for the completed call once the consumer's backend has
//! filled the per-call PCM budget. [`Driver::decode_call`] is the
//! full-pipeline analog: it validates sizes, runs stages 1+2, and
//! surfaces the backend frame-decode as [`Error::NotImplemented`] —
//! reserving that signal exclusively for the transform GAP so length
//! errors stay distinct.
//!
//! Round 11 wires the per-category gain/quantiser parameter bundle:
//! the [`category`] module gives a typed [`CategoryIndex`] newtype
//! enforcing the `0..=6` range the per-band quantiser worker
//! `cook.dll!0x69f0` guards, plus a [`CategoryParameters`] struct that
//! bundles the three parallel `[cat*4 + base]` lookups (`gain_step` /
//! `gain_bias` / `level_count`) into a single audit-anchored accessor.
//! The structural lookup is the piece; the per-band quantiser
//! algorithm itself remains a DOCS-GAP (only the audit's single
//! `(bias + |sample| * step)` sentence describes its arithmetic — too
//! narrow to wire without a band-loop pin).
//!
//! Round 13 adds the typed structural accessor for the
//! 51-entry bit-allocation category LUT (`cook.dll!0x8c40`, audit point
//! #14): the [`bit_alloc`] module wraps the
//! `tables::category_index_lut()` raw slice in two newtypes
//! ([`BitAllocAxisPosition`] in `0..=50` and [`BitAllocCategory`] in
//! `0..=19`) plus the single lookup
//! [`bit_alloc_category_for_position`]. The LUT itself is byte-validated
//! to the meta's *"51 non-decreasing u32 values spanning 0..19"*
//! invariant, and the typed accessor surfaces an out-of-range axis
//! position as the new [`Error::BitAllocAxisOutOfRange`] typed error.
//! The LUT's runtime consumer inside the backend (plausibly paired with
//! the `0x8fcc` category-expectation table audit point #17 leaves as
//! a tightened-but-still GAP) is *not* wired by this round — only the
//! structural lookup is.
//!
//! Round 12 adds the structural cookie-geometry guard:
//! [`CookCookie::validate_geometry`] checks the three independent
//! field-level invariants spec/02 §1 pins on every well-formed flavor
//! record — `channels ∈ {1, 2}` (line 33 / line 50), `subband_count >=
//! 1` (line 34: present on every record, sentinel record 30 hits the
//! minimum at exactly `1`), and `samples_per_frame_x_channels >= 1`
//! (the `[4..5]` product is at least `256` on any well-formed
//! record). [`CookCookie::parse`] runs the guard automatically so every
//! parser-built cookie is structurally well-formed by construction;
//! [`crate::DecodeConfig::from_inputs`] re-runs the same guard at the
//! head of its input wiring so literal-built cookies (test fixtures,
//! cached wire snapshots) get the same structural rejection before the
//! divisor / flavor checks run. Surfaced as the three typed errors
//! [`Error::CookieInvalidChannels`] / [`Error::CookieZeroSubbandCount`]
//! / [`Error::CookieZeroSamplesProduct`] so consumers can distinguish a
//! structurally-malformed cookie from a cookie that simply names the
//! wrong flavor record ([`Error::CookieFlavorMismatch`]).
//!
//! Round 14 adds the typed exponent-indexed accessors for
//! the two back-to-back 127-entry scale ladders (`cook.dll!0x91fc` =
//! `2^k`, `cook.dll!0x93f8` = `2^(k/2)`, both for `k = -63..+63`): the
//! [`scale`] module wraps the raw `tables::pow2_exponent_table()` /
//! `tables::sqrt2_scale_ladder()` slices behind a [`ScaleExponent`]
//! newtype enforcing the `-63..=63` exponent range (out-of-range raises
//! the new [`Error::ScaleExponentOutOfRange`]), with
//! [`pow2_scale_for_exponent`] / [`sqrt2_scale_for_exponent`] as the
//! two lookups. It also pins audit point #15's sub-pointer
//! reconciliation (`provenance/03`): the Round-1 spec/01 §6 rows at
//! `0x92d4` ("2^-9 … 2^19") and `0x94a8` ("0.00138, 0.00195, 0.00276,
//! …") are sub-pointers **into** these ladders at element offsets 54
//! (first exponent −9) and 44 (first exponent −19) — both derived by
//! RVA subtraction and exported as named constants. The
//! exponent-producing runtime stage (which worker feeds which ladder)
//! remains a spec/01 §6 GAP; only the typed table access is wired.
//!
//! Round 15 wires the `RADecode` `flags` decode/observe
//! gate — the first half of the backend frame-decode slot
//! `[backend_vtable + 0x0c]` the validator pinned. The decode driver
//! `cook.dll!0x1260` forwards `(~flags) & 1` to the backend
//! (`validation/04` §4.3): with `flags` bit 0 = 1 the backend decodes
//! the bitstream; with bit 0 = 0 it emits **zeroed overlap-add output
//! independent of the input** (verified by the validator: all-`0xFF`
//! input produces the same zero output as the real packets). The
//! typed [`DecodeGate`] mirrors that computation
//! ([`DecodeGate::from_flags`] / [`DecodeGate::backend_gate_bit`]),
//! and [`Driver::decode_call_with_flags`] is the six-argument
//! `RADecode` analog: on the observe gate it zero-fills the per-call
//! PCM budget and advances the cursor (the driver's buffer accounting
//! is gate-independent, so the observe path walks the same warm-up /
//! steady-state cadence); on the real-decode gate the
//! bitstream/transform backend remains [`Error::NotImplemented`].
//! [`Driver::decode_call`] is now the
//! `flags = `[`RADECODE_FLAGS_DECODE`] shorthand.
//!
//! Round 16 wires the typed accessor for the
//! windowing / overlap-add side of the inverse-MDCT stage: the
//! [`mdct`] module keys the five vendored Princen-Bradley half-windows
//! (`cook.dll!0x8d0c`, lengths 3 / 7 / 15 / 31 / 64) behind a typed
//! [`MdctWindowLength`] selector — only the five stored lengths are
//! constructible ([`MdctWindowLength::from_len`] raises the typed
//! [`Error::MdctWindowLengthUnsupported`] otherwise) — with
//! [`mdct_half_window`] as the length-keyed lookup and the audit-#14
//! boundary facts (`provenance/03`: window table spans
//! `0x8d0c`..`0x8eec`, abutting the 51-entry category LUT) surfaced as
//! derived RVA constants. The long/short adaptive switching that
//! selects a window at runtime (spec/01 §5.1) and the inverse-MDCT
//! kernel itself (the `0xa1b0` rotation table, audit #16: no validated
//! closed form) remain GAPs — only the typed window-table access is
//! wired.
//!
//! Round 17 (this round) wires the typed accessor for the last
//! vendored DSP table without one: the 11-entry reciprocal
//! averaging-divisor table (`cook.dll!0xa7a8`,
//! `tables/reciprocal-1-over-n.meta`; spec/01 §6 row `0xa7a8`; audit
//! #15's count correction 14 → 11). The [`reciprocal`] module types
//! the table's three structural regions — the consecutive `1/n` run
//! for denominators `1..=9` behind the [`ReciprocalDenominator`]
//! newtype ([`reciprocal_for_denominator`]), the stored `1/20` at
//! element 9 behind its own named accessor
//! ([`reciprocal_one_twentieth`] — its denominator is not adjacent to
//! the run), and the stored trailing `0.0` at element 10 (pinned by
//! constant + test). Out-of-run denominators raise the typed
//! [`Error::ReciprocalDenominatorOutOfRange`]. With this, **every**
//! extracted numeric table in `docs/audio/cook/tables/` is reachable
//! through a typed, range-guarded API; the table's runtime consumer
//! (which worker averages / normalises with it) is not pinned by
//! spec/01 — a recorded GAP, like the frame bitstream syntax itself.
//!
//! Round 18 (this round) wires the first piece of per-band quantiser
//! *arithmetic* — not just a table accessor — that the worker
//! `cook.dll!0x69f0` computes. Two facts are pinned in the table `.meta`
//! files beyond the parallel-table access of round 11: the per-band
//! magnitude form `bias + |sample| * step`
//! (`tables/gain-bias-ramp.meta`, verbatim) and the level-count clip of
//! the per-band quantiser index (`tables/category-level-count.meta`:
//! *"used both to size and to clip the per-band quantiser index"*). The
//! [`quantiser`] module wires both on top of [`CategoryParameters`]:
//! [`band_gain_magnitude`] / [`CategoryParameters::band_gain_magnitude`]
//! evaluate `bias + |sample| * step` against one category bundle, and
//! [`clip_quantiser_index`] / [`CategoryParameters::clip_quantiser_index`]
//! clip a raw index to the `0..=level_count-1` range the category
//! admits. The band loop that drives these primitives (raw-index read,
//! sign restoration, the `0x8fcc` category-expectation combine that
//! audit #17 leaves a GAP, and the feed into the inverse MDCT) is **not**
//! pinned beyond these two `.meta` sentences and stays a recorded GAP.
//!
//! Round 19 (this round) types the RealAudio codec **service-provider
//! interface** (SPI) export surface in [`spi`]: spec/01 §2 pins all 20
//! named exports (ordinals 1–20) of `cook.dll` — their names, front-end
//! RVAs, and roles — plus the `HRESULT` codes the front-ends return
//! (`S_OK`, `E_INVALIDARG` on a NULL handle, the three `E_NOTIMPL`
//! stubs, the `0x80040005` unrecognised-selector code). [`SpiExport`]
//! is an exhaustive ordinal-ordered enum with [`SpiExport::ordinal`] /
//! [`SpiExport::name`] / [`SpiExport::front_end_rva`] /
//! [`SpiExport::notimpl_result`] / [`SpiExport::is_decode_path`] /
//! [`SpiExport::is_encoder`] accessors, anchored to audit point #1
//! (*"all 20 ordinal/name/RVA triples match exactly"*) and the flavor-
//! count immediates of #2 / #3. This is the export-level contract a
//! container demuxer drives the codec through; the worker bodies live in
//! the decode modules.
//!
//! Round 20 (this round) types the `RAGetFlavorProperty` property-ID
//! dispatch surface in [`flavor_property`]: spec/01 §4.2 / spec/02 §1.2
//! pin the export-ordinal-10 worker (`cook.dll!0x17a0`) as an MSVC jump
//! table at RVA `0x1be8` with **21 cases (property IDs 0–20)**, where
//! cases **0, 4, 7** return a NUL-terminated string (length computed via
//! `strlen`) and every other case returns a **32-bit integer** (fixed
//! returned length `4`). The [`FlavorPropertyId`] newtype enforces the
//! `0..=20` range the table bounds (out-of-range raises the typed
//! [`Error::FlavorPropertyIdOutOfRange`]), and [`FlavorPropertyId::kind`]
//! classifies the return shape into the [`FlavorPropertyKind`] enum,
//! anchored to audit point #13 (*"21-entry (0–20) jump table at
//! `0x1be8`; string props (cases 0/4/7) via strlen"*, **CONFIRMED**).
//! The full property-ID → *meaning* enumeration and the
//! property-descriptor structure's stride / layout remain an explicit
//! spec/01 §4.2 / spec/02 §1.2 GAP — only the dispatch surface is wired.
//!
//! This round wires the foundational primitive every backend frame stage
//! consumes: the MSB-first big-endian frame [`bitreader::FrameBitReader`]
//! (`spec/05` §0.1, `provenance/05` evidence #1). It holds the four-field
//! reader state block (`+0x479c` word pointer / `+0x47a0` bit position /
//! `+0x47a4` bit cursor / `+0x47a8` bit limit) and exposes the two pinned
//! reader primitives: [`FrameBitReader::read_bits`] (`read-n-bits`,
//! `cook.dll!0x3f40`, the closed form `word << pos | next >> (32 - pos)`
//! then `>> (32 - n)`) and [`FrameBitReader::read_bit`] /
//! [`FrameBitReader::read_flag`] (`read-1-bit`, `cook.dll!0x3fc0`,
//! unsigned `0`/`1` and the binary's arithmetic-shift `0`/`-1` flag form).
//! Reads at or past the bit limit return `0` and clamp the cursor.
//!
//! A later round wires the **first frame-body stage** the bit reader
//! feeds — the per-sub-packet [`gain`] envelope (`spec/05` §1,
//! `provenance/05` evidence #2 / #3). Two statically-pinned, non-GAP
//! primitives are wired on top of [`FrameBitReader`] and the
//! [`scale`] ladder: [`gain::read_segment_count`] (the leading 6-bit
//! field + the `−6` bias the worker `cook.dll!0x4b50` applies, so the
//! field carries `segment_count + 6`) and [`gain::gain_factor_for_index`]
//! (a per-segment gain index → `2^(index/2)` via the `0x93f8` ladder's
//! centre, `1.0` at element 63 — the `0x94f4` positive-window sub-pointer
//! of evidence #3, surfaced as [`gain::GAIN_POS_WINDOW`]). The
//! per-segment *record reads* (position + gain index) descend the VLC
//! walk `cook.dll!0x3a50`, whose codebook bytes are a §3.2 BSS GAP, and
//! the §1.2 piecewise-constant interpolation/application over the
//! transform sub-blocks both stay recorded GAPs.
//!
//! The latest rounds assemble the **frame-body orchestrator**
//! ([`frame::decode_frame_body`]): it drives the statically-pinned
//! prefix of the §0–§3 walk — the §1.1 gain-segment count, the §2.1
//! subband → coefficient-range geometry ([`subband::SubbandGeometry`])
//! — and stops precisely at the §3 spectral-VLC dequant step, whose
//! seven Huffman codebooks' code/length bytes are runtime-built in
//! `.data` BSS at init and absent from the file image (`spec/05` §3.2,
//! docs-gap #1775). [`Driver::decode_call`] now drives every sub-packet
//! through that orchestrator on the real-decode gate, surfacing the
//! typed [`Error::SpectralCodebookBytesUnavailable`] blocker rather than
//! the opaque [`Error::NotImplemented`].
//!
//! The §1.2 gain *application* (piecewise-constant hold + time-domain
//! multiply) and the §4.2 joint-stereo mirror-index split are also wired
//! ([`gain::apply_gain_envelope`], [`spectral::split_coupled_coefficient`])
//! — pinned closed forms that consume the BSS-gated coefficient inputs.
//! The runtime-built BSS codebook / coupling tables (§3.2 / §4.3) and the
//! iMDCT kernel (§5) remain recorded GAPs the walk stops short of; the
//! entropy + transform pipeline past the §3.2 blocker lands once a
//! dynamic-BSS-dump Validator round provides the populated tables.

#![forbid(unsafe_code)]

pub mod assembler;
pub mod backend;
pub mod bit_alloc;
pub mod bitreader;
pub mod category;
pub mod codebook;
pub mod cookie;
pub mod coupling;
pub mod coupling_control;
pub mod descramble;
pub mod driver;
pub mod flavor;
pub mod flavor_property;
pub mod frame;
pub mod gain;
pub mod imlt;
pub mod index_decomp;
pub mod init;
pub mod mdct;
pub mod output_stage;
pub mod pcm;
pub mod quantiser;
pub mod reciprocal;
pub mod reconstruct;
pub mod scale;
pub mod session;
pub mod spectral;
pub mod spi;
pub mod subband;
pub mod subpacket;
pub mod synthesis;
pub mod tables;

pub use assembler::CallPcmAssembler;
pub use backend::SynthesisBackend;
pub use bit_alloc::{
    bit_alloc_category_for_position, BitAllocAxisPosition, BitAllocCategory, BIT_ALLOC_AXIS_LEN,
    BIT_ALLOC_CATEGORY_COUNT, MAX_BIT_ALLOC_AXIS_POSITION, MAX_BIT_ALLOC_CATEGORY,
};
pub use bitreader::{
    FrameBitReader, CTX_BIT_CURSOR_OFFSET, CTX_BIT_LIMIT_OFFSET, CTX_BIT_POSITION_OFFSET,
    CTX_WORD_POINTER_OFFSET, WORD_BITS, WORD_BYTES,
};
pub use category::{
    category_parameters, gain_step_via_scale_ladder, CategoryIndex, CategoryParameters,
    CATEGORY_COUNT, GAIN_STEP_CENTRE_CATEGORY, MAX_CATEGORY_INDEX,
};
pub use codebook::{spectral_huffman, SpectralHuffman, CODEBOOK_COUNT};
pub use cookie::{CookCookie, SelectorFamily, EXTENDED_COOKIE_LEN, SELECTOR_EXTENDED};
pub use coupling::{CouplingMode, StereoMode, STEREO_MODE_MAX, STEREO_MODE_MIN};
pub use coupling_control::{
    read_coupling_index, read_coupling_mode, read_fixed_coupling_index, CouplingReadMode,
    CTX_COUPLING_INDEX_BITS_OFFSET, CTX_SUBBAND_COUNT_OFFSET,
};
pub use descramble::{descramble_packet, xor_descramble, xor_descramble_into, xor_key, CommonMode};
pub use driver::{DecodeGate, Driver, PreparedCall};
pub use flavor::{
    flavor_indices_matching_cookie, flavor_record, iter_flavor_records,
    iter_playable_flavor_records, FlavorRecord, FLAVOR_COUNT, RA_GET_NUMBER_OF_FLAVORS2_ADVERTISED,
    RA_GET_NUMBER_OF_FLAVORS_ADVERTISED, SENTINEL_FLAVOR_INDEX,
};
pub use flavor_property::{
    FlavorPropertyId, FlavorPropertyKind, FLAVOR_PROPERTY_ID_COUNT, FLAVOR_PROPERTY_INTEGER_LEN,
    FLAVOR_PROPERTY_JUMP_TABLE_RVA, MAX_FLAVOR_PROPERTY_ID, STRING_PROPERTY_IDS,
};
pub use frame::{
    decode_frame_body, frame_body_prefix, reconstruct_frame_spectrum, FrameSpectrum, FrameWalk,
    StereoCoupling,
};
pub use gain::{
    apply_gain_blocks, apply_gain_envelope, expand_gain_envelope, gain_factor_for_index,
    read_segment_count, GainSegment, GAIN_POS_WINDOW, SEGMENT_COUNT_BIAS, SEGMENT_COUNT_FIELD_BITS,
};
pub use imlt::{imlt, imlt_direct, mlt_direct};
pub use index_decomp::{
    decompose_index, index_radix, index_recip, reciprocal_quotient, INDEX_RADIX, INDEX_RECIP,
    INDEX_RECIP_COUNT, INDEX_RECIP_RVA, INDEX_RECIP_SCALE, INDEX_RECIP_SHIFT,
    MAX_INDEX_RECIP_INDEX,
};
pub use init::{DecodeConfig, Descriptor, PCM_BYTES_PER_SAMPLE, RADECODE_FLAGS_DECODE};
pub use mdct::{
    mdct_full_window, mdct_half_window, MdctWindowLength, MDCT_WINDOW_COUNT,
    MDCT_WINDOW_TABLE_END_RVA, MDCT_WINDOW_TABLE_RVA,
};
pub use output_stage::{
    apply_window, overlap_add, overlap_add_weighted, window_and_gain, windowed,
    OVERLAP_MIX_WEIGHT_HALF, OVERLAP_MIX_WEIGHT_HALF_RVA, OVERLAP_MIX_WEIGHT_THREE_QUARTER,
    OVERLAP_MIX_WEIGHT_THREE_QUARTER_RVA,
};
pub use pcm::{f32_to_i16_sample, interleave_stereo, pcm_i16le, write_pcm_i16le};
pub use quantiser::{
    band_gain_magnitude, clip_quantiser_index, quantiser_level, QUANTISER_DIVISOR,
    QUANTISER_DIVISOR_RVA,
};
pub use reciprocal::{
    reciprocal_for_denominator, reciprocal_one_twentieth, ReciprocalDenominator,
    RECIPROCAL_DENOMINATOR_MAX, RECIPROCAL_DENOMINATOR_MIN, RECIPROCAL_ONE_TWENTIETH_INDEX,
    RECIPROCAL_RUN_LEN, RECIPROCAL_TABLE_END_RVA, RECIPROCAL_TABLE_RVA,
    RECIPROCAL_TRAILING_ZERO_INDEX,
};
pub use reconstruct::{
    decouple_stereo, reconstruct_band, reconstruct_spectrum, BandReconstruction, StereoSpectra,
};
pub use scale::{
    pow2_scale_for_exponent, sqrt2_scale_for_exponent, ScaleExponent,
    POW2_SUBPOINTER_ELEMENT_OFFSET, POW2_SUBPOINTER_FIRST_EXPONENT, SCALE_EXPONENT_BIAS,
    SCALE_EXPONENT_MAX, SCALE_EXPONENT_MIN, SQRT2_SUBPOINTER_ELEMENT_OFFSET,
    SQRT2_SUBPOINTER_FIRST_EXPONENT,
};
pub use session::CallSession;
pub use spectral::{
    category_vector_dims, coefficients_for_symbols, coupling_table_len, mirror_partner_index,
    sign_from_bit, spectral_coefficient, split_coupled_coefficient, symbols_for_band,
    CategoryVectorDims, SpectralCodebook, CATEGORY_VECTOR_DIM_HI_RVA, CATEGORY_VECTOR_DIM_LO_RVA,
    DEQUANT_SCALE_NONZERO, DEQUANT_SCALE_RVA, MAX_SPECTRAL_CODEBOOK_INDEX, SIGN_LUT, SIGN_LUT_RVA,
    SPECTRAL_CODEBOOK_COUNT, SPECTRAL_CODEBOOK_DIMS_RVA, SPECTRAL_CODEBOOK_LENGTH_PTRS_RVA,
    SPECTRAL_CODEBOOK_VALUE_PTRS_RVA,
};
pub use spi::{
    SpiExport, E_INVALIDARG, E_NOTIMPL, HR_UNRECOGNISED_SELECTOR, RASETFLAVOR_CONTEXT_OFFSET,
    RA_NUMBER_OF_FLAVORS, RA_NUMBER_OF_FLAVORS2, SPI_EXPORT_COUNT, S_OK,
};
pub use subband::{
    subband_line_range, subband_start_line, SubbandGeometry, SUBBAND_HALF_SCALAR,
    SUBBAND_HALF_SCALAR_RVA, SUBBAND_IDENTITY_RUN, SUBBAND_START_LINE_LUT_RVA,
};
pub use subpacket::SubPacketLayout;
pub use synthesis::Synthesizer;

/// Crate-local error type. Concrete variants land as the rebuild rounds
/// populate the codec pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Reserved placeholder for not-yet-implemented decode paths.
    NotImplemented,
    /// An extradata cookie blob was shorter than the layout requires.
    CookieTooShort {
        /// Number of bytes actually supplied.
        got: usize,
    },
    /// An extradata cookie carried a selector this parser does not handle.
    ///
    /// Returned for any value the backend factory `cook.dll!0x1c60`
    /// would reject with `0x80040005`. The two GAP families
    /// ([`Error::NonExtendedSelectorNotSupported`],
    /// [`Error::MultichannelSelectorNotSupported`]) carry their own
    /// variants so callers can distinguish "binary rejects" from
    /// "parser-scope GAP" without re-parsing the selector.
    UnsupportedSelector {
        /// The leading 32-bit selector value read from the blob.
        selector: u32,
    },
    /// Cookie's leading selector is a recognised mono/stereo Cook
    /// backend selector (`0x01000001` or `0x01000002`) but **not** the
    /// extended-layout selector [`SELECTOR_EXTENDED`].
    ///
    /// The init worker `cook.dll!0x1420` would build the same
    /// mono/stereo backend, but it reads a shorter cookie layout that
    /// `docs/audio/cook/spec/01-cook-decoder-structure.md` §3 does not
    /// pin — a recorded DOCS-GAP. Distinct from
    /// [`Error::UnsupportedSelector`] so consumers can hand this stream
    /// to a future shorter-cookie parser without losing the typing.
    NonExtendedSelectorNotSupported {
        /// The leading 32-bit selector value read from the blob.
        selector: u32,
    },
    /// Cookie's leading selector is the multichannel ("RealAudio 10"
    /// 5.1) backend selector `0x02000000`.
    ///
    /// The backend factory `cook.dll!0x1c60` would dispatch to the
    /// distinct multichannel backend (constructor `0x2260`), but the
    /// per-stream cookie layout it consumes is not pinned by spec/01
    /// §3 or by the validator (the validated `FUN_RM_32.rm` stream is
    /// stereo) — a recorded DOCS-GAP. Distinct from
    /// [`Error::UnsupportedSelector`] so a future multichannel parser
    /// can be added without losing the typed dispatch.
    MultichannelSelectorNotSupported {
        /// The leading 32-bit selector value read from the blob
        /// (always `0x02000000`).
        selector: u32,
    },
    /// `RAInitDecoder` descriptor `+0x06` (`channels_divisor`) was `0`;
    /// the backend init `0x20c0` would divide-by-zero.
    ZeroDivisorChannels,
    /// `RAInitDecoder` descriptor `+0x0a` (`sub_packet_size`) was `0`;
    /// the per-call decode driver `0x1290` would divide-by-zero.
    ZeroDivisorSubPacketSize,
    /// The cookie does not self-describe the named flavor record (one
    /// of channels, subband count, stereo mode, or recovered
    /// samples-per-frame disagrees).
    CookieFlavorMismatch,
    /// `frame_bytes` is not an integer multiple of `sub_packet_size`,
    /// so the per-call sub-packet division would leave a non-zero
    /// remainder.
    FrameNotDivisibleBySubPacket {
        /// Per-stream `frame_bytes` (block-align).
        frame_bytes: u32,
        /// Descriptor `+0x0a`.
        sub_packet_size: u16,
    },
    /// A sub-packet slot index was out of range for the per-call
    /// partition (`slot >= sub_packets_per_call`).
    SlotOutOfRange {
        /// The supplied slot index.
        slot: u32,
        /// The wired sub-packets-per-call count.
        slots_per_call: u32,
    },
    /// An input buffer supplied to [`SubPacketLayout::iter_call`] did
    /// not have length [`SubPacketLayout::frame_bytes`].
    SubPacketInputLengthMismatch {
        /// The buffer length actually supplied.
        got: usize,
        /// The required per-call input length (= `frame_bytes`).
        expected: usize,
    },
    /// A `RADecode` call's input length did not match the per-call
    /// budget the [`session::CallSession`] tracks (= `frame_bytes`).
    CallInputLengthMismatch {
        /// The supplied input length.
        got: usize,
        /// The required per-call input length.
        expected: usize,
    },
    /// A `RADecode` call's output buffer length did not match the
    /// validator-pinned PCM budget the [`session::CallSession`] tracks
    /// (warm-up on the first call, steady-state thereafter).
    CallOutputLengthMismatch {
        /// The supplied output buffer length.
        got: usize,
        /// The required per-call PCM budget for this call index.
        expected: usize,
    },
    /// A gain/quantiser category index was outside the
    /// `0..=[crate::MAX_CATEGORY_INDEX]` range the per-band quantiser
    /// worker `cook.dll!0x69f0` validates (audit note: *"category index
    /// 7 is guarded out by the worker"* —
    /// `docs/audio/cook/tables/category-level-count.meta`).
    CategoryOutOfRange {
        /// The supplied category index.
        got: u8,
    },
    /// A bit-allocation axis position was outside the
    /// `0..=[crate::MAX_BIT_ALLOC_AXIS_POSITION]` range the 51-entry
    /// `cook.dll!0x8c40` LUT covers
    /// (`docs/audio/cook/tables/category-index-lut.meta`,
    /// `element_count: 51`).
    BitAllocAxisOutOfRange {
        /// The supplied axis position.
        got: u8,
    },
    /// A requested MDCT half-window length was not one of the five the
    /// binary stores (3 / 7 / 15 / 31 / 64 —
    /// `docs/audio/cook/tables/mdct-windows.meta`: *"rows:
    /// 3,7,15,31,64"*).
    MdctWindowLengthUnsupported {
        /// The supplied element count.
        got: usize,
    },
    /// A flavor record's `+0x04` stereo-mode selector was outside the set
    /// `docs/audio/cook/spec/02-cook-flavor-and-extradata-layout.md` §1
    /// documents (*"0 for mono; 2–5 for the stereo / surround
    /// families"*). The reserved value `1` and any value `> 5` are
    /// unassigned and absent from the extracted geometry table.
    StereoModeUnsupported {
        /// The supplied raw stereo-mode value.
        got: u32,
    },
    /// A reciprocal-table denominator was outside the consecutive
    /// `[crate::RECIPROCAL_DENOMINATOR_MIN]..=[crate::RECIPROCAL_DENOMINATOR_MAX]`
    /// (= `1..=9`) run the 11-entry `cook.dll!0xa7a8` table stores
    /// (`docs/audio/cook/tables/reciprocal-1-over-n.meta`: *"1/n for
    /// n = 1..9, then 1/20 and 0"*). The stored `1/20` is reachable
    /// through [`crate::reciprocal_one_twentieth`], not through a
    /// denominator value.
    ReciprocalDenominatorOutOfRange {
        /// The supplied denominator.
        got: u8,
    },
    /// A scale-ladder exponent was outside the
    /// `[crate::SCALE_EXPONENT_MIN]..=[crate::SCALE_EXPONENT_MAX]`
    /// (= `-63..=63`) range the two 127-entry ladders at
    /// `cook.dll!0x91fc` / `0x93f8` cover
    /// (`docs/audio/cook/tables/pow2-exponent-table.meta` /
    /// `sqrt2-scale-ladder.meta`: *"k = -63..+63"*).
    ScaleExponentOutOfRange {
        /// The supplied exponent.
        got: i8,
    },
    /// Cookie `[0xc..0xd]` channels field declared a value outside the
    /// `{1, 2}` set spec/02 §1 pins for every well-formed flavor record
    /// (line 33: *"channels: **1** or **2**"*; line 50: *"channels in
    /// {1, 2}"*).
    ///
    /// A value of `0` would also trip a divide-by-zero in
    /// [`crate::CookCookie::samples_per_frame`] (the recovered
    /// samples-per-frame uses `channels` as the divisor); a value of
    /// `>= 3` cannot correspond to any of the 31 well-formed flavor
    /// records and would in any case mis-size every downstream PCM
    /// budget (PCM-bytes-per-call scales linearly with channels).
    /// Surfaced as its own typed variant so callers can distinguish a
    /// structurally-malformed cookie from a cookie that simply names
    /// the wrong flavor record (which stays
    /// [`Error::CookieFlavorMismatch`]).
    CookieInvalidChannels {
        /// The malformed channel count read from cookie `[0xc..0xd]`.
        got: u16,
    },
    /// Cookie `[6..7]` subband count was `0`.
    ///
    /// Spec/02 §1 line 34 documents `subband count` as *"number of
    /// coded subbands (grows with sample rate and bitrate; e.g. 12 at
    /// 8 kHz, 47 at 44.1 kHz)"*; every one of the 31 well-formed
    /// flavor records has `subband_count >= 1` (the sentinel record 30
    /// has the minimum value `1`). A `0` would make the per-band
    /// quantiser loop trivially empty and cannot correspond to any
    /// well-formed flavor record, so the cookie is rejected
    /// structurally rather than allowed to silently mis-match.
    CookieZeroSubbandCount,
    /// Cookie `[4..5]` `samples_per_frame × channels` product was `0`.
    ///
    /// The backend init `0x20c0` divides this value by descriptor
    /// `+0x06` (`channels_divisor`) to recover `samples_per_frame`; a
    /// `0` product cannot correspond to any well-formed flavor record
    /// (all 31 records have `samples_per_frame ∈ {256, 512, 1024}` and
    /// `channels ∈ {1, 2}`, so the product is at least `256`). Rejected
    /// as a structurally-malformed cookie.
    CookieZeroSamplesProduct,
    /// A `RAGetFlavorProperty` property ID was outside the
    /// `0..=[crate::MAX_FLAVOR_PROPERTY_ID]` (= `0..=20`) range the
    /// 21-case jump table at `cook.dll!0x1be8` covers (spec/01 §4.2:
    /// *"a 21-entry jump table (cases 0–20, table at RVA `0x1be8`)"*;
    /// audit point #13).
    FlavorPropertyIdOutOfRange {
        /// The supplied property ID.
        got: u8,
    },
    /// A spectral-codebook index was outside the
    /// `0..=[crate::MAX_SPECTRAL_CODEBOOK_INDEX]` (= `0..=6`) range the
    /// seven spectral Huffman codebooks cover
    /// (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §3.1:
    /// *"There are **seven** spectral codebooks"*;
    /// `tables/spectral-codebook-dims.meta`, `element_count: 7`).
    SpectralCodebookOutOfRange {
        /// The supplied codebook index.
        got: u8,
    },
    /// A spectral-symbol index was outside `0..symbol_count()` for its
    /// codebook (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md`
    /// §3.1 — the seven codebooks have symbol counts
    /// `{196, 100, 49, 625, 256, 243, 32}`).
    SpectralSymbolOutOfRange {
        /// The supplied symbol index.
        got: u32,
        /// The codebook's symbol count.
        count: u32,
    },
    /// The §3.1 spectral VLC walk (`cook.dll!0x3a50`) read
    /// [`max_code_length`](codebook::SpectralHuffman::max_code_length) bits
    /// without matching any codeword — a malformed or exhausted bitstream
    /// (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §3.1).
    SpectralVlcNoMatch {
        /// The codebook (`0..=6`) whose walk found no match.
        codebook: u8,
        /// Bits consumed before giving up (the codebook's max code length).
        bits: u32,
    },
    /// A joint-stereo coupling index `j` was `>=` the coupling table
    /// length `Ncoup` for the §4.2 mirror-index split
    /// (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §4.2:
    /// the partner read `coef[Ncoup - 1 - j]` requires `j < Ncoup`).
    CouplingIndexOutOfRange {
        /// The supplied coupling index.
        got: u32,
        /// The coupling table length `Ncoup = 1 << coupling_bits`.
        ncoup: u32,
    },
    /// A joint-stereo coupling table had length `0`, so the §4.2
    /// mirror-index partner `Ncoup - 1 - j` has no entry to read
    /// (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §4.2).
    CouplingTableEmpty,
    /// A spectral-vector dimension of `0` was passed to the §3.1 symbol /
    /// coefficient grouping arithmetic
    /// (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §3.1: a
    /// real per-category vector dimension is always `2..=10`, so `0`
    /// cannot occur for a looked-up [`CategoryVectorDims`]; surfaced here
    /// rather than dividing by zero).
    SpectralVectorDimZero,
    /// The per-frame gain-envelope segment-count field biased to a
    /// negative count.
    ///
    /// `spec/05` §1.1 (evidence #2): the leading 6-bit field carries
    /// `segment_count + 6` and the worker forms `field − 6`; a
    /// well-formed stream therefore never carries a raw value below `6`.
    /// A raw field `< 6` (or an absent field that reads `0` at end of
    /// frame) would bias negative — surfaced here rather than silently
    /// wrapping.
    GainSegmentCountUnderflow {
        /// The raw 6-bit field value (`< 6`) that biased negative.
        raw: u32,
    },
    /// A gain-envelope application was asked to expand over `0`
    /// sub-blocks.
    ///
    /// `spec/05` §1.2: the gain profile is expanded to **one factor per
    /// sub-block**; a sub-block count of `0` leaves no grid to map the
    /// time-domain samples onto. The sub-block count derives from the
    /// long/short transform-block-length state, which is always `>= 1`
    /// for a real frame — surfaced here rather than producing an empty
    /// profile.
    GainBlockCountZero,
    /// A stereo frame reconstruction (`channels == 2`) was driven without
    /// the §4 coupling inputs.
    ///
    /// `spec/05` §4: the stereo body decodes a single coupled spectrum and
    /// splits it into left/right by the per-coupling-band rotation; that
    /// split needs the coupling-band range, the per-band rotation indices
    /// and the `coef` table (a §4.3 BSS GAP). A stereo route with no
    /// coupling inputs cannot run the decouple — surfaced here.
    StereoCouplingMissing,
    /// A per-band spectral reconstruction was supplied a decoded-value
    /// slice whose length does not match the band's coded line count.
    ///
    /// `spec/05` §3 / §2.1: each subband occupies a half-open
    /// coefficient range `[start_line[band] .. start_line[band+1])`; the
    /// dequantiser fills exactly `line_count` coefficients for the band.
    /// The post-entropy reconstruction takes the band's decoded symbol
    /// values as input (a §3.2 BSS GAP supplies them); this guards a
    /// caller that hands a value slice of the wrong width.
    BandValueCountMismatch {
        /// The band's coded line count (`line_count[band]`).
        line_count: u32,
        /// The number of decoded values the caller supplied.
        got: usize,
    },
    /// A full-spectrum reconstruction was supplied a per-band category
    /// map whose length does not match the stream's subband count.
    ///
    /// `spec/05` §2: the category-assignment pass produces one category
    /// per coded subband; the reconstruction drives the per-band fill over
    /// every subband of the [`crate::subband::SubbandGeometry`]. This
    /// guards a caller whose category map (or per-band value table) is not
    /// `subband_count` long.
    SpectrumBandCountMismatch {
        /// The stream's coded subband count.
        subband_count: u32,
        /// The number of per-band entries the caller supplied.
        got: usize,
    },
    /// A stereo decouple was supplied a coupling-index slice whose length
    /// does not match the number of coupling bands.
    ///
    /// `spec/05` §4.1: one coupling/rotation index `j` is read per
    /// coupling band; the decouple splits each coupling band's coupled
    /// coefficient with its index. This guards a caller whose index slice
    /// width disagrees with the coupling-band count.
    CouplingIndexCountMismatch {
        /// The number of coupling bands the geometry spans.
        coupling_bands: u32,
        /// The number of coupling indices the caller supplied.
        got: usize,
    },
    /// The backend per-frame walk reached the §3 spectral-VLC dequant
    /// step, whose seven Huffman codebooks' per-symbol code/length bytes
    /// are **runtime-built in `.data` BSS at init** and are not present
    /// in the file image (`spec/05` §3.2 — docs-gap #1775).
    ///
    /// Distinct from [`Error::NotImplemented`]: every statically-pinned
    /// frame-body stage before this point ran (the §1.1 gain segment
    /// count, the §2.1 subband geometry); the walk stops precisely at
    /// the sub-step that needs a dynamic BSS dump of the populated
    /// codebook tables (a Validator/Extractor round), rather than
    /// fabricating the codebook contents. The same BSS-build mechanism
    /// gates the §4.3 per-coupling-width rotation coefficient values.
    SpectralCodebookBytesUnavailable,
    /// A reciprocal-table index was outside the seven-entry `0x8fac`
    /// Q-format reciprocal array
    /// (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §2.2:
    /// *"the Q-format constants at RVA `0x8fac`"*; seven entries, indices
    /// `0..=[crate::MAX_INDEX_RECIP_INDEX]`).
    IndexRecipOutOfRange {
        /// The supplied reciprocal-table index.
        got: u8,
    },
    /// A time-domain block passed to the §5 windowing stage did not match
    /// the selected MDCT window length
    /// (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §5: each
    /// block is windowed by one of the five stored Princen-Bradley
    /// windows of length 3/7/15/31/64).
    OutputWindowLengthMismatch {
        /// The supplied block length.
        got: usize,
        /// The selected window length.
        window: usize,
    },
    /// The two contributions passed to the §5 overlap-add differed in
    /// length (`docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §5:
    /// the output is the per-sample sum of two equal-length windowed
    /// blocks).
    OverlapAddLengthMismatch {
        /// Length of the first (previous-block) contribution.
        a: usize,
        /// Length of the second (current-block) contribution.
        b: usize,
    },
    /// The §5 inverse transform was asked for a zero-size block
    /// (`docs/audio/cook/spec/01-cook-decoder-structure.md` §5.1: the
    /// inverse MDCT runs at the selected block length, which is always
    /// `>= 1` for a real frame — a zero-length spectrum has no defined
    /// transform).
    TransformSizeZero,
    /// A forward-MLT input block had odd length — the MDCT consumes
    /// `2N` time samples per `N`-coefficient frame, so the input length
    /// must be even.
    TransformInputLengthOdd {
        /// The supplied input length.
        got: usize,
    },
    /// A spectrum pushed into the §5 [`synthesis::Synthesizer`] did not
    /// carry exactly one hop of coefficients (`spec/01` §5.1: the
    /// inverse MDCT runs at the selected block length — `N` spectral
    /// lines in, `N` finished samples out per frame after the
    /// overlap-add).
    SynthesisSpectrumLengthMismatch {
        /// The supplied spectrum length.
        got: usize,
        /// The engine's hop (spectral lines per frame).
        hop: usize,
    },
    /// A PCM output buffer was not exactly
    /// [`PCM_BYTES_PER_SAMPLE`] (= 2) bytes per sample
    /// (`validation/04` §5: every per-call budget is
    /// `samples × channels × 2` bytes — 16-bit PCM).
    PcmOutputLengthMismatch {
        /// The supplied buffer length.
        got: usize,
        /// The required length (`samples × 2`).
        expected: usize,
    },
    /// The two channel buffers passed to the stereo interleave differed
    /// in length (each sample instant carries one sample per channel).
    InterleaveLengthMismatch {
        /// Channel 0 length.
        ch0: usize,
        /// Channel 1 length.
        ch1: usize,
    },
    /// A synthesized frame's PCM did not carry exactly
    /// `samples_per_frame × channels × 2` bytes
    /// ([`assembler::CallPcmAssembler::frame_pcm_bytes`]).
    FramePcmLengthMismatch {
        /// The supplied frame PCM length.
        got: usize,
        /// The per-frame PCM byte size the config derives.
        expected: usize,
    },
    /// A per-call PCM fill was requested with fewer bytes buffered than
    /// the call budget requires — the caller has not pushed the call's
    /// synthesized frames yet (`validation/04` §5 cadence:
    /// `sub_packets_per_call` frames enter per call).
    PcmAssemblerUnderrun {
        /// Bytes the call budget requires.
        need: usize,
        /// Bytes currently buffered.
        have: usize,
    },
    /// A synthesis window did not carry `2 × samples_per_frame` taps
    /// (`spec/05` §5: the transform emits `2N` samples per `N`-line
    /// frame, each multiplied by one window tap).
    SynthesisWindowLengthMismatch {
        /// The supplied window length.
        got: usize,
        /// The required length (`2 × samples_per_frame`).
        expected: usize,
    },
    /// A [`frame::FrameSpectrum`]'s channel routing disagreed with the
    /// wired config (`spec/05` §0: the mono body decodes one spectrum,
    /// the stereo body a coupled pair).
    FrameSpectrumChannelMismatch {
        /// Channels the spectrum carries (1 = mono, 2 = stereo).
        got: u16,
        /// Channels the config wires.
        expected: u16,
    },
    /// A per-channel §1.2 gain-profile set did not carry exactly one
    /// profile per wired channel.
    GainProfileCountMismatch {
        /// Profiles supplied.
        got: usize,
        /// Profiles required (= channels).
        expected: usize,
    },
    /// A reconstructed spectrum carried more spectral lines than the
    /// transform admits (`spec/01` §5.1: the inverse MDCT runs at the
    /// selected block length — coded lines never exceed it; the
    /// remaining upper lines are uncoded and zero-filled).
    SpectrumExceedsTransformSize {
        /// Coded lines supplied.
        got: usize,
        /// The transform size (`samples_per_frame`).
        hop: usize,
    },
    /// A resume-from-blocker call ([`driver::Driver::synthesized_call`])
    /// was not supplied exactly one post-entropy [`frame::FrameSpectrum`]
    /// per sub-packet (`spec/01` §5: the backend decodes one frame per
    /// sub-packet slot).
    FrameSpectrumCountMismatch {
        /// Spectra supplied.
        got: usize,
        /// Spectra required (= `sub_packets_per_call`).
        expected: usize,
    },
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::NotImplemented => f.write_str(
                "oxideav-cook: clean-room rebuild in progress — see crates/oxideav-cook/README.md",
            ),
            Error::CookieTooShort { got } => {
                write!(f, "oxideav-cook: extradata cookie too short ({got} bytes)")
            }
            Error::UnsupportedSelector { selector } => {
                write!(
                    f,
                    "oxideav-cook: unsupported cookie selector {selector:#010x}"
                )
            }
            Error::NonExtendedSelectorNotSupported { selector } => write!(
                f,
                "oxideav-cook: cookie selector {selector:#010x} is a non-extended mono/stereo \
                 sibling (cookie layout not pinned by spec/01 §3 — DOCS-GAP)"
            ),
            Error::MultichannelSelectorNotSupported { selector } => write!(
                f,
                "oxideav-cook: cookie selector {selector:#010x} is the multichannel backend \
                 (cookie layout not pinned by spec/01 §3 — DOCS-GAP)"
            ),
            Error::ZeroDivisorChannels => f.write_str(
                "oxideav-cook: descriptor channels_divisor (+0x06) is 0 \
                 (would divide-by-zero in backend init 0x20c0)",
            ),
            Error::ZeroDivisorSubPacketSize => f.write_str(
                "oxideav-cook: descriptor sub_packet_size (+0x0a) is 0 \
                 (would divide-by-zero in RADecode 0x1290)",
            ),
            Error::CookieFlavorMismatch => f.write_str(
                "oxideav-cook: extradata cookie does not self-describe the named flavor record",
            ),
            Error::FrameNotDivisibleBySubPacket {
                frame_bytes,
                sub_packet_size,
            } => write!(
                f,
                "oxideav-cook: frame_bytes {frame_bytes} is not an integer multiple of \
                 sub_packet_size {sub_packet_size}"
            ),
            Error::SlotOutOfRange {
                slot,
                slots_per_call,
            } => write!(
                f,
                "oxideav-cook: sub-packet slot {slot} out of range \
                 (slots_per_call = {slots_per_call})"
            ),
            Error::SubPacketInputLengthMismatch { got, expected } => write!(
                f,
                "oxideav-cook: sub-packet input length {got} does not match the per-call \
                 frame_bytes {expected}"
            ),
            Error::CallInputLengthMismatch { got, expected } => write!(
                f,
                "oxideav-cook: RADecode call input length {got} does not match the per-call \
                 frame_bytes {expected}"
            ),
            Error::CallOutputLengthMismatch { got, expected } => write!(
                f,
                "oxideav-cook: RADecode call output buffer length {got} does not match the \
                 validator-pinned PCM budget {expected}"
            ),
            Error::CategoryOutOfRange { got } => write!(
                f,
                "oxideav-cook: gain/quantiser category index {got} is out of range \
                 (max is {})",
                MAX_CATEGORY_INDEX
            ),
            Error::BitAllocAxisOutOfRange { got } => write!(
                f,
                "oxideav-cook: bit-allocation axis position {got} is out of range \
                 (max is {})",
                MAX_BIT_ALLOC_AXIS_POSITION
            ),
            Error::MdctWindowLengthUnsupported { got } => write!(
                f,
                "oxideav-cook: MDCT half-window length {got} is not one of the five stored \
                 lengths (3 / 7 / 15 / 31 / 64)"
            ),
            Error::StereoModeUnsupported { got } => write!(
                f,
                "oxideav-cook: stereo-mode selector {got} is outside the documented set \
                 (0 for mono; 2..=5 for the stereo / surround families; spec/02 §1)"
            ),
            Error::ReciprocalDenominatorOutOfRange { got } => write!(
                f,
                "oxideav-cook: reciprocal-table denominator {got} is outside the consecutive \
                 run ({}..={}; the stored 1/20 has its own accessor)",
                reciprocal::RECIPROCAL_DENOMINATOR_MIN,
                reciprocal::RECIPROCAL_DENOMINATOR_MAX
            ),
            Error::ScaleExponentOutOfRange { got } => write!(
                f,
                "oxideav-cook: scale-ladder exponent {got} is out of range \
                 ({}..={})",
                SCALE_EXPONENT_MIN, SCALE_EXPONENT_MAX
            ),
            Error::CookieInvalidChannels { got } => write!(
                f,
                "oxideav-cook: cookie channels field {got} is outside the well-formed \
                 {{1, 2}} set (spec/02 §1)"
            ),
            Error::CookieZeroSubbandCount => f.write_str(
                "oxideav-cook: cookie subband count is 0 (no well-formed flavor record \
                 has subband_count == 0; spec/02 §1)",
            ),
            Error::CookieZeroSamplesProduct => f.write_str(
                "oxideav-cook: cookie [4..5] samples_per_frame × channels product is 0 \
                 (no well-formed flavor record has samples_per_frame == 0; spec/02 §1)",
            ),
            Error::FlavorPropertyIdOutOfRange { got } => write!(
                f,
                "oxideav-cook: flavor-property ID {got} is out of range \
                 (max is {})",
                flavor_property::MAX_FLAVOR_PROPERTY_ID
            ),
            Error::SpectralCodebookOutOfRange { got } => write!(
                f,
                "oxideav-cook: spectral-codebook index {got} is out of range \
                 (max is {})",
                spectral::MAX_SPECTRAL_CODEBOOK_INDEX
            ),
            Error::SpectralSymbolOutOfRange { got, count } => write!(
                f,
                "oxideav-cook: spectral-symbol index {got} is out of range \
                 (codebook has {count} symbols; spec/05 §3.1)"
            ),
            Error::SpectralVlcNoMatch { codebook, bits } => write!(
                f,
                "oxideav-cook: spectral VLC walk found no codeword in \
                 codebook {codebook} within {bits} bits (spec/05 §3.1)"
            ),
            Error::CouplingIndexOutOfRange { got, ncoup } => write!(
                f,
                "oxideav-cook: coupling index {got} is out of range \
                 (Ncoup = {ncoup})"
            ),
            Error::CouplingTableEmpty => f.write_str(
                "oxideav-cook: joint-stereo coupling table has length 0 \
                 (no mirror-index partner to read; spec/05 §4.2)",
            ),
            Error::SpectralVectorDimZero => f.write_str(
                "oxideav-cook: spectral-vector dimension is 0 \
                 (a real per-category dimension is 2..=10; spec/05 §3.1)",
            ),
            Error::GainSegmentCountUnderflow { raw } => write!(
                f,
                "oxideav-cook: gain-envelope segment-count field {raw} biased \
                 to a negative count (field carries segment_count + 6; spec/05 §1.1)"
            ),
            Error::GainBlockCountZero => f.write_str(
                "oxideav-cook: gain-envelope application asked to expand over 0 \
                 sub-blocks (spec/05 §1.2 expands one factor per sub-block)",
            ),
            Error::StereoCouplingMissing => f.write_str(
                "oxideav-cook: stereo frame reconstruction (channels == 2) requires \
                 the §4 coupling inputs (band range, rotation indices, coef table)",
            ),
            Error::BandValueCountMismatch { line_count, got } => write!(
                f,
                "oxideav-cook: per-band reconstruction value count {got} does not \
                 match the band's coded line count {line_count} (spec/05 §3/§2.1)"
            ),
            Error::SpectrumBandCountMismatch { subband_count, got } => write!(
                f,
                "oxideav-cook: spectrum per-band entry count {got} does not match \
                 the stream's subband count {subband_count} (spec/05 §2)"
            ),
            Error::CouplingIndexCountMismatch {
                coupling_bands,
                got,
            } => write!(
                f,
                "oxideav-cook: coupling index count {got} does not match the \
                 coupling-band count {coupling_bands} (spec/05 §4.1)"
            ),
            Error::SpectralCodebookBytesUnavailable => f.write_str(
                "oxideav-cook: backend frame walk reached the §3 spectral-VLC step; \
                 the seven codebooks' code/length bytes are runtime-built in BSS \
                 and not in the file image (spec/05 §3.2 — docs-gap #1775)",
            ),
            Error::IndexRecipOutOfRange { got } => write!(
                f,
                "oxideav-cook: reciprocal-table index {got} out of range \
                 (0x8fac has 7 entries, indices 0..=6)"
            ),
            Error::OutputWindowLengthMismatch { got, window } => write!(
                f,
                "oxideav-cook: output-stage block length {got} does not match the selected \
                 MDCT window length {window}"
            ),
            Error::OverlapAddLengthMismatch { a, b } => write!(
                f,
                "oxideav-cook: overlap-add contributions differ in length ({a} vs {b})"
            ),
            Error::TransformSizeZero => f.write_str(
                "oxideav-cook: inverse-transform block size is 0 \
                 (spec/01 §5.1 runs the inverse MDCT at a block length >= 1)",
            ),
            Error::TransformInputLengthOdd { got } => write!(
                f,
                "oxideav-cook: forward-MLT input length {got} is odd \
                 (the MDCT consumes 2N time samples per N-coefficient frame)"
            ),
            Error::SynthesisSpectrumLengthMismatch { got, hop } => write!(
                f,
                "oxideav-cook: synthesis spectrum length {got} does not match \
                 the engine hop {hop}"
            ),
            Error::PcmOutputLengthMismatch { got, expected } => write!(
                f,
                "oxideav-cook: PCM output buffer length {got} is not two bytes \
                 per sample (expected {expected})"
            ),
            Error::InterleaveLengthMismatch { ch0, ch1 } => write!(
                f,
                "oxideav-cook: stereo interleave channels differ in length \
                 ({ch0} vs {ch1})"
            ),
            Error::FramePcmLengthMismatch { got, expected } => write!(
                f,
                "oxideav-cook: synthesized frame PCM length {got} does not match \
                 the per-frame budget {expected}"
            ),
            Error::PcmAssemblerUnderrun { need, have } => write!(
                f,
                "oxideav-cook: per-call PCM fill needs {need} bytes but only \
                 {have} are buffered (push the call's frames first)"
            ),
            Error::SynthesisWindowLengthMismatch { got, expected } => write!(
                f,
                "oxideav-cook: synthesis window length {got} does not match \
                 2 x samples_per_frame (expected {expected})"
            ),
            Error::FrameSpectrumChannelMismatch { got, expected } => write!(
                f,
                "oxideav-cook: frame spectrum carries {got} channel(s) but the \
                 config wires {expected}"
            ),
            Error::GainProfileCountMismatch { got, expected } => write!(
                f,
                "oxideav-cook: {got} gain profile(s) supplied for {expected} \
                 channel(s)"
            ),
            Error::SpectrumExceedsTransformSize { got, hop } => write!(
                f,
                "oxideav-cook: spectrum carries {got} coded lines but the \
                 transform admits {hop}"
            ),
            Error::FrameSpectrumCountMismatch { got, expected } => write!(
                f,
                "oxideav-cook: {got} frame spectra supplied for a call of \
                 {expected} sub-packets"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Crate-local Result alias.
pub type Result<T> = core::result::Result<T, Error>;
