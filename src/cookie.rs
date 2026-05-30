//! Extradata "cookie" parser.
//!
//! When the decoder is opened it consumes a small per-stream extradata
//! blob carried by the `.ra`/`.rm` container. The blob is read
//! **big-endian**. Its leading 32-bit value is a *selector* compared
//! against [`SELECTOR_EXTENDED`]: values `>= SELECTOR_EXTENDED` take an
//! extended path that reads a channel and stereo-mode pair from the tail
//! of the blob.
//!
//! Extended-selector byte grid (offsets into the blob):
//!
//! | Bytes      | Field                              |
//! | ---------- | ---------------------------------- |
//! | `[0..4]`   | u32 selector                       |
//! | `[4..6]`   | u16 samples_per_frame × channels   |
//! | `[6..8]`   | u16 subband count                  |
//! | `[8..12]`  | u32 reserved / sub-selector        |
//! | `[12..14]` | u16 channels                       |
//! | `[14..16]` | u16 stereo / coupling mode         |

use crate::{flavor::FlavorRecord, Error};

/// Selector boundary. Values `>= SELECTOR_EXTENDED` select the
/// mono/stereo backend and the extended (16-byte) cookie layout.
pub const SELECTOR_EXTENDED: u32 = 0x0100_0003;

/// Number of cookie bytes consumed by the extended layout.
pub const EXTENDED_COOKIE_LEN: usize = 16;

/// Parsed extended-selector extradata cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CookCookie {
    /// 32-bit selector (`>= SELECTOR_EXTENDED`).
    pub selector: u32,
    /// `samples_per_frame × channels` (numerator; divide by `channels`
    /// to recover samples-per-frame).
    pub samples_per_frame_x_channels: u16,
    /// Number of coded subbands.
    pub subband_count: u16,
    /// Reserved / sub-selector field (observed 0 on validated streams).
    pub reserved: u32,
    /// Channel count.
    pub channels: u16,
    /// Stereo / coupling mode.
    pub stereo_mode: u16,
}

impl CookCookie {
    /// Parse an extended-selector cookie from `blob`.
    ///
    /// # Errors
    /// - [`Error::CookieTooShort`] if `blob` is shorter than
    ///   [`EXTENDED_COOKIE_LEN`].
    /// - [`Error::UnsupportedSelector`] if the leading selector is
    ///   `< SELECTOR_EXTENDED` (the single-selector / multichannel paths
    ///   are not handled by this parser).
    pub fn parse(blob: &[u8]) -> Result<Self, Error> {
        if blob.len() < EXTENDED_COOKIE_LEN {
            return Err(Error::CookieTooShort { got: blob.len() });
        }
        let be16 = |o: usize| u16::from_be_bytes([blob[o], blob[o + 1]]);
        let be32 = |o: usize| u32::from_be_bytes([blob[o], blob[o + 1], blob[o + 2], blob[o + 3]]);

        let selector = be32(0);
        if selector < SELECTOR_EXTENDED {
            return Err(Error::UnsupportedSelector { selector });
        }
        Ok(CookCookie {
            selector,
            samples_per_frame_x_channels: be16(4),
            subband_count: be16(6),
            reserved: be32(8),
            channels: be16(12),
            stereo_mode: be16(14),
        })
    }

    /// Recover samples-per-frame from the cookie: the `[4..6]` product
    /// divided by the channel count.
    ///
    /// Returns `None` if the cookie declares zero channels (the product
    /// could not have been formed).
    pub fn samples_per_frame(&self) -> Option<u32> {
        if self.channels == 0 {
            return None;
        }
        Some(self.samples_per_frame_x_channels as u32 / self.channels as u32)
    }

    /// Check that this cookie describes the same configuration as a named
    /// flavor geometry `record`.
    ///
    /// Compares channels, subband count, stereo mode, and the recovered
    /// samples-per-frame. `frame_bytes`, `coupling_mode`, and sample rate
    /// are not carried by the cookie and are not compared here.
    pub fn matches_flavor(&self, record: &FlavorRecord) -> bool {
        self.channels as u32 == record.channels
            && self.subband_count as u32 == record.subband_count
            && self.stereo_mode as u32 == record.stereo_mode
            && self.samples_per_frame() == Some(record.samples_per_frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flavor::flavor_record;

    /// The real `FUN_RM_32.rm` extradata cookie (flavor 21), verbatim.
    const REAL_COOKIE: [u8; 16] = [
        0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00,
        0x04,
    ];

    #[test]
    fn parses_real_cookie_fields() {
        let c = CookCookie::parse(&REAL_COOKIE).unwrap();
        assert_eq!(c.selector, SELECTOR_EXTENDED);
        assert_eq!(c.samples_per_frame_x_channels, 2048);
        assert_eq!(c.subband_count, 32);
        assert_eq!(c.reserved, 0);
        assert_eq!(c.channels, 2);
        assert_eq!(c.stereo_mode, 4);
        assert_eq!(c.samples_per_frame(), Some(1024));
    }

    #[test]
    fn real_cookie_matches_flavor_record_21() {
        let c = CookCookie::parse(&REAL_COOKIE).unwrap();
        let r = flavor_record(21).unwrap();
        assert!(c.matches_flavor(&r), "cookie {c:?} vs record {r:?}");
    }

    #[test]
    fn short_blob_is_rejected() {
        assert_eq!(
            CookCookie::parse(&REAL_COOKIE[..15]),
            Err(Error::CookieTooShort { got: 15 })
        );
    }

    #[test]
    fn low_selector_is_rejected() {
        let mut blob = REAL_COOKIE;
        blob[3] = 0x02; // selector 0x01000002 < SELECTOR_EXTENDED
        assert_eq!(
            CookCookie::parse(&blob),
            Err(Error::UnsupportedSelector {
                selector: 0x0100_0002
            })
        );
    }

    #[test]
    fn samples_per_frame_zero_channels_is_none() {
        let c = CookCookie {
            selector: SELECTOR_EXTENDED,
            samples_per_frame_x_channels: 2048,
            subband_count: 32,
            reserved: 0,
            channels: 0,
            stereo_mode: 4,
        };
        assert_eq!(c.samples_per_frame(), None);
    }
}
