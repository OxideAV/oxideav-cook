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
//! The transform / entropy decode pipeline itself still lands in later
//! rounds — [`Error::NotImplemented`] continues to gate the decode
//! path.

#![forbid(unsafe_code)]

pub mod cookie;
pub mod flavor;
pub mod init;
pub mod tables;

pub use cookie::CookCookie;
pub use flavor::{flavor_record, FlavorRecord, FLAVOR_COUNT};
pub use init::{DecodeConfig, Descriptor, PCM_BYTES_PER_SAMPLE, RADECODE_FLAGS_DECODE};

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
    UnsupportedSelector {
        /// The leading 32-bit selector value read from the blob.
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
        }
    }
}

impl std::error::Error for Error {}

/// Crate-local Result alias.
pub type Result<T> = core::result::Result<T, Error>;
