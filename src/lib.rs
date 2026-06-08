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
//! Round 13 (this round) adds the typed structural accessor for the
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
//! The transform / entropy decode pipeline itself still lands in later
//! rounds — [`Error::NotImplemented`] continues to gate the decode
//! path.

#![forbid(unsafe_code)]

pub mod bit_alloc;
pub mod category;
pub mod cookie;
pub mod descramble;
pub mod driver;
pub mod flavor;
pub mod init;
pub mod session;
pub mod subpacket;
pub mod tables;

pub use bit_alloc::{
    bit_alloc_category_for_position, BitAllocAxisPosition, BitAllocCategory, BIT_ALLOC_AXIS_LEN,
    BIT_ALLOC_CATEGORY_COUNT, MAX_BIT_ALLOC_AXIS_POSITION, MAX_BIT_ALLOC_CATEGORY,
};
pub use category::{
    category_parameters, CategoryIndex, CategoryParameters, CATEGORY_COUNT, MAX_CATEGORY_INDEX,
};
pub use cookie::{CookCookie, SelectorFamily, EXTENDED_COOKIE_LEN, SELECTOR_EXTENDED};
pub use descramble::{descramble_packet, xor_descramble, xor_descramble_into, xor_key, CommonMode};
pub use driver::{Driver, PreparedCall};
pub use flavor::{
    flavor_indices_matching_cookie, flavor_record, iter_flavor_records,
    iter_playable_flavor_records, FlavorRecord, FLAVOR_COUNT, RA_GET_NUMBER_OF_FLAVORS2_ADVERTISED,
    RA_GET_NUMBER_OF_FLAVORS_ADVERTISED, SENTINEL_FLAVOR_INDEX,
};
pub use init::{DecodeConfig, Descriptor, PCM_BYTES_PER_SAMPLE, RADECODE_FLAGS_DECODE};
pub use session::CallSession;
pub use subpacket::SubPacketLayout;

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
        }
    }
}

impl std::error::Error for Error {}

/// Crate-local Result alias.
pub type Result<T> = core::result::Result<T, Error>;
