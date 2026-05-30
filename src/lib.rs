//! Pure-Rust RealAudio Cook audio codec.
//!
//! **Clean-room rebuild.** This is a fresh orphan `master`; the previous
//! implementation was retired alongside the OxideAV docs audit dated
//! 2026-05-06. See `README.md` for the rebuild scope.
//!
//! Round 1 lands the stream-configuration front end: the per-flavor
//! [`flavor::flavor_record`] geometry-table loader and the extradata
//! [`cookie::CookCookie`] parser, with a cross-check that a parsed cookie
//! describes the same configuration as its named flavor record. The
//! transform / entropy decode pipeline lands in later rounds.

#![forbid(unsafe_code)]

pub mod cookie;
pub mod flavor;

pub use cookie::CookCookie;
pub use flavor::{flavor_record, FlavorRecord, FLAVOR_COUNT};

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
        }
    }
}

impl std::error::Error for Error {}

/// Crate-local Result alias.
pub type Result<T> = core::result::Result<T, Error>;
