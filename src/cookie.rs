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
//!
//! ## Selector families (spec/01 §3.1)
//!
//! The decoder's backend factory `cook.dll!0x1c60` switches on the
//! leading 32-bit selector to allocate one of several polymorphic decoder
//! objects. The published mapping is:
//!
//! | Selector value(s)                        | Backend                                     |
//! | ---------------------------------------- | ------------------------------------------- |
//! | `0x01000001`, `0x01000002`, `0x01000003` | mono/stereo Cook transform decoder (`0x1d20` / vtable `0x60bd8bb0`) |
//! | `0x02000000`                             | multichannel ("RealAudio 10" 5.1 family) — distinct backend class (`0x2260`) |
//! | other values in the recognised range     | additional distinct backend class (`0x2070`) |
//! | otherwise                                | rejected (`0x80040005`)                      |
//!
//! The init worker `cook.dll!0x1420` performs an *orthogonal* comparison
//! against `0x01000003` to decide whether to read **additional** bytes
//! (the [`SELECTOR_EXTENDED`] threshold of this module): values
//! `>= SELECTOR_EXTENDED` cause the extended-path field reads at
//! `[8..12]`, `[12..14]`, `[14..16]`. Values `< SELECTOR_EXTENDED` take
//! the shorter single-selector path; that layout is not pinned by spec
//! or validator and is a recorded **DOCS-GAP**.
//!
//! Only the `0x01000003` selector — the one validated end-to-end against
//! `FUN_RM_32.rm` (`validation/04`) — is decoded by [`CookCookie::parse`]
//! here. The two sibling MonoStereo selectors (`0x01000001` /
//! `0x01000002`) and the multichannel `0x02000000` family are surfaced
//! by [`SelectorFamily::classify`] for callers that need to triage a
//! stream's selector before parsing, but [`CookCookie::parse`] returns
//! a typed error rather than silently building a stereo cookie out of
//! their bytes.

use crate::{flavor::FlavorRecord, Error};

/// Selector boundary. Values `>= SELECTOR_EXTENDED` select the
/// **extended cookie layout** (the 16-byte grid this parser consumes).
///
/// The init worker `cook.dll!0x1420` reads the additional `[8..12]`,
/// `[12..14]`, `[14..16]` fields only when the selector is at least
/// this value. Values below take a shorter single-selector layout that
/// spec/01 §3 does not pin and the validator did not exercise — a
/// recorded DOCS-GAP. Distinct from [`SelectorFamily`], which classifies
/// the selector by *backend class* rather than by *layout length*.
pub const SELECTOR_EXTENDED: u32 = 0x0100_0003;

/// Number of cookie bytes consumed by the extended layout.
pub const EXTENDED_COOKIE_LEN: usize = 16;

/// Backend-factory classification of a cookie's leading 32-bit selector
/// (spec/01 §3.1).
///
/// Independent of [`SELECTOR_EXTENDED`] (which gates cookie *layout
/// length*, not backend class). Returned by [`SelectorFamily::classify`]
/// for any 32-bit value; callers can pre-triage a stream's selector
/// before deciding whether [`CookCookie::parse`] is the right entry
/// point or — for the GAP families — whether to stop and report a
/// docs gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorFamily {
    /// `0x01000001`, `0x01000002`, `0x01000003` — the mono/stereo Cook
    /// transform decoder (backend constructor `0x1d20`, vtable
    /// `0x60bd8bb0`). [`CookCookie::parse`] decodes only the
    /// `0x01000003` member of this family, which is the one the
    /// validator confirmed against a real stream (`validation/04` §4).
    /// The `0x01000001` / `0x01000002` siblings share the backend
    /// class but consume a shorter cookie layout that spec/01 §3 does
    /// not pin (DOCS-GAP).
    MonoStereo,
    /// `0x02000000` — the multichannel ("RealAudio 10" 5.1) backend
    /// (constructor `0x2260`). Cookie byte layout is not pinned by
    /// spec/01 §3 or by the validator (the validated stream is stereo)
    /// — DOCS-GAP.
    Multichannel,
    /// Other recognised-range value — distinct backend class
    /// (constructor `0x2070`). The exact in-range set is not
    /// enumerated by spec/01 §3.1 beyond the three families above —
    /// DOCS-GAP. Surfaced here so a caller can distinguish "the
    /// factory would have accepted it" from a genuinely unsupported
    /// value.
    Other,
    /// Selector value the backend factory does not accept (the
    /// `0x80040005` return path from `cook.dll!0x1c60`).
    Unsupported,
}

impl SelectorFamily {
    /// Classify a 32-bit selector value by backend family (spec/01
    /// §3.1).
    ///
    /// The three enumerated MonoStereo selectors and the
    /// Multichannel `0x02000000` are documented exactly; every other
    /// value is reported as [`SelectorFamily::Unsupported`] since the
    /// "other (in range)" set of spec/01 §3.1 is not enumerated and
    /// safely conflating it with [`SelectorFamily::Other`] would risk
    /// surfacing values the binary rejects.
    pub const fn classify(selector: u32) -> SelectorFamily {
        match selector {
            0x0100_0001..=0x0100_0003 => SelectorFamily::MonoStereo,
            0x0200_0000 => SelectorFamily::Multichannel,
            _ => SelectorFamily::Unsupported,
        }
    }

    /// Whether [`CookCookie::parse`] can decode a cookie with this
    /// family's leading selector.
    ///
    /// Currently true only for the exact `0x01000003` selector inside
    /// the [`SelectorFamily::MonoStereo`] family (the validator-pinned
    /// path). The other MonoStereo selectors and every other family
    /// require a non-extended or distinct cookie layout that spec/01
    /// §3 does not yet pin.
    pub const fn is_parser_supported(selector: u32) -> bool {
        selector == SELECTOR_EXTENDED
    }
}

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
    /// - [`Error::NonExtendedSelectorNotSupported`] if the leading
    ///   selector is a documented MonoStereo sibling (`0x01000001` /
    ///   `0x01000002`) — same backend family but a shorter cookie
    ///   layout that spec/01 §3 does not pin (DOCS-GAP).
    /// - [`Error::MultichannelSelectorNotSupported`] if the leading
    ///   selector is `0x02000000` — the distinct multichannel backend
    ///   family (spec/01 §3.1); cookie layout not pinned (DOCS-GAP).
    /// - [`Error::UnsupportedSelector`] for any other selector value
    ///   (the `0x80040005` rejection path of `cook.dll!0x1c60`).
    pub fn parse(blob: &[u8]) -> Result<Self, Error> {
        if blob.len() < EXTENDED_COOKIE_LEN {
            return Err(Error::CookieTooShort { got: blob.len() });
        }
        let be16 = |o: usize| u16::from_be_bytes([blob[o], blob[o + 1]]);
        let be32 = |o: usize| u32::from_be_bytes([blob[o], blob[o + 1], blob[o + 2], blob[o + 3]]);

        let selector = be32(0);
        // Branch on the spec/01 §3.1 family before reading any
        // extended-layout bytes. The non-extended MonoStereo siblings
        // and the multichannel family share the leading selector field
        // but spec/01 §3 does not pin their cookie body — surface them
        // with their own typed errors rather than silently constructing
        // a stereo cookie out of bytes the binary would not read here.
        if selector == SELECTOR_EXTENDED {
            Ok(CookCookie {
                selector,
                samples_per_frame_x_channels: be16(4),
                subband_count: be16(6),
                reserved: be32(8),
                channels: be16(12),
                stereo_mode: be16(14),
            })
        } else {
            match SelectorFamily::classify(selector) {
                SelectorFamily::MonoStereo => {
                    // 0x01000001 / 0x01000002 — same backend family,
                    // non-extended cookie layout (DOCS-GAP).
                    Err(Error::NonExtendedSelectorNotSupported { selector })
                }
                SelectorFamily::Multichannel => {
                    Err(Error::MultichannelSelectorNotSupported { selector })
                }
                SelectorFamily::Other | SelectorFamily::Unsupported => {
                    Err(Error::UnsupportedSelector { selector })
                }
            }
        }
    }

    /// Classify this cookie's selector by backend family
    /// (spec/01 §3.1).
    ///
    /// Always returns [`SelectorFamily::MonoStereo`] for a successfully
    /// parsed cookie (only [`SELECTOR_EXTENDED`] reaches construction
    /// today), but exists so consumers can use the family-classification
    /// API uniformly across pre- and post-parse code paths.
    pub fn family(&self) -> SelectorFamily {
        SelectorFamily::classify(self.selector)
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
    fn non_extended_monostereo_sibling_rejected_with_typed_error() {
        // Selector 0x01000002 is a documented MonoStereo sibling
        // (spec/01 §3.1) but takes the non-extended cookie layout
        // (DOCS-GAP). Surface it with the family-specific error so
        // callers can triage GAP-typed vs binary-rejected separately.
        let mut blob = REAL_COOKIE;
        blob[3] = 0x02;
        assert_eq!(
            CookCookie::parse(&blob),
            Err(Error::NonExtendedSelectorNotSupported {
                selector: 0x0100_0002
            })
        );
        // 0x01000001 is the same family.
        blob[3] = 0x01;
        assert_eq!(
            CookCookie::parse(&blob),
            Err(Error::NonExtendedSelectorNotSupported {
                selector: 0x0100_0001
            })
        );
    }

    #[test]
    fn multichannel_selector_rejected_with_typed_error() {
        // Selector 0x02000000 is the multichannel ("RealAudio 10")
        // backend (spec/01 §3.1); cookie layout not pinned.
        let mut blob = REAL_COOKIE;
        blob[0] = 0x02;
        blob[1] = 0x00;
        blob[2] = 0x00;
        blob[3] = 0x00;
        assert_eq!(
            CookCookie::parse(&blob),
            Err(Error::MultichannelSelectorNotSupported {
                selector: 0x0200_0000
            })
        );
    }

    #[test]
    fn truly_unrecognised_selector_rejected() {
        // A value the backend factory `0x1c60` would return
        // `0x80040005` for: outside the three documented families.
        let mut blob = REAL_COOKIE;
        blob[0] = 0xff;
        blob[1] = 0xff;
        blob[2] = 0xff;
        blob[3] = 0xff;
        assert_eq!(
            CookCookie::parse(&blob),
            Err(Error::UnsupportedSelector {
                selector: 0xffff_ffff
            })
        );
    }

    #[test]
    fn selector_family_classifies_documented_values() {
        // spec/01 §3.1 documented mappings.
        assert_eq!(
            SelectorFamily::classify(0x0100_0001),
            SelectorFamily::MonoStereo
        );
        assert_eq!(
            SelectorFamily::classify(0x0100_0002),
            SelectorFamily::MonoStereo
        );
        assert_eq!(
            SelectorFamily::classify(SELECTOR_EXTENDED),
            SelectorFamily::MonoStereo
        );
        assert_eq!(
            SelectorFamily::classify(0x0200_0000),
            SelectorFamily::Multichannel
        );
        // Off-by-one above the multichannel value: not part of the
        // enumerated set, surfaced as Unsupported.
        assert_eq!(
            SelectorFamily::classify(0x0200_0001),
            SelectorFamily::Unsupported
        );
        // The validator-confirmed value is the only one this parser
        // decodes today.
        assert!(SelectorFamily::is_parser_supported(SELECTOR_EXTENDED));
        assert!(!SelectorFamily::is_parser_supported(0x0100_0001));
        assert!(!SelectorFamily::is_parser_supported(0x0200_0000));
    }

    #[test]
    fn parsed_cookie_family_is_monostereo() {
        let c = CookCookie::parse(&REAL_COOKIE).unwrap();
        assert_eq!(c.family(), SelectorFamily::MonoStereo);
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
