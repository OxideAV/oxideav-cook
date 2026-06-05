//! Decoder open configuration — the wired init layer.
//!
//! `RAInitDecoder` consumes a small descriptor (two `u16` scalars at
//! `+0x06` and `+0x0a` plus a pointer/length pair to the extradata
//! blob) and a per-stream extradata cookie. Their interaction is now
//! pinned by a real-stream cross-check (`docs/audio/cook/validation/
//! 04-cook-stream-validation.md`):
//!
//! - Descriptor `+0x06` (`u16`) is the **`channels` divisor** that
//!   recovers `samples_per_frame` from the cookie's
//!   `samples_per_frame × channels` product at blob bytes `[4..6]`
//!   (`backend init 0x20c0` `idiv` at `0x21c2`: `2048 / 2 = 1024`).
//!   Passing `0` is a hard divide-by-zero.
//! - Descriptor `+0x0a` (`u16`) is the **`sub_packet_size`**: the
//!   number of bytes of coded payload per sub-packet. The decode
//!   driver `RADecode` (`0x1260`) computes
//!   `sub_packets_per_call = frame_bytes / sub_packet_size` (the
//!   `div [esi+8]` at `0x1290`). On the validated stream
//!   `frame_bytes = 465` and `sub_packet_size = 93`, so
//!   `sub_packets_per_call = 5`. Passing `0` is a hard divide-by-zero.
//! - `RADecode`'s `flags` argument is forwarded as `(~flags) & 1` to
//!   the backend frame-decode method `[backend_vtable + 0x0c]`. A
//!   real decode of a fresh frame therefore passes `flags = 1`
//!   (gate `0`); `flags = 0` (gate `1`) is the "observe / emit zeros"
//!   path.
//!
//! This module builds a [`DecodeConfig`] from a parsed
//! [`CookCookie`], a [`Descriptor`] scalar pair, and a
//! [`FlavorRecord`]. The config records the exact per-call PCM budget
//! the validator measured (`20 480` bytes in steady state on the
//! validated stream) so consumers can size their output buffers
//! deterministically.

use crate::{cookie::CookCookie, flavor::FlavorRecord, Error};

/// Bit 0 of `RADecode`'s `flags` argument, inverted, is the backend
/// frame-decode gate.
///
/// A real decode of a fresh frame passes this value (`= 1`); the
/// backend's gate then sees `(~flags) & 1 = 0` and performs the
/// bitstream decode.
///
/// Pinned by `docs/audio/cook/validation/04-cook-stream-validation.md`
/// §4.3.
pub const RADECODE_FLAGS_DECODE: u32 = 1;

/// Output sample format: 16-bit signed little-endian.
///
/// Pinned by the validator (`validation/04` §5): the decoder emits
/// `2 936 832` bytes for a `16.649 s` stereo 44 100 Hz stream
/// (`2 936 832 / 4 / 44 100 = 16.649 s`), i.e. 4 bytes per stereo
/// frame = 2 bytes × 2 channels.
pub const PCM_BYTES_PER_SAMPLE: u32 = 2;

/// `RAInitDecoder` descriptor scalars relevant to the decode-state
/// geometry.
///
/// The full descriptor structure carries other fields (extradata
/// pointer at `+0x16`, length at `+0x12`); this struct holds only the
/// two `u16` scalars the backend uses for geometry, named by their
/// validated runtime role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    /// Descriptor `+0x06`: the `channels` divisor that recovers
    /// `samples_per_frame` from the cookie's `[4..6]` product.
    pub channels_divisor: u16,
    /// Descriptor `+0x0a`: the sub-packet size in bytes; divisor of
    /// `frame_bytes` to derive sub-packets-per-call.
    pub sub_packet_size: u16,
}

/// A fully-wired decoder configuration.
///
/// Built by [`DecodeConfig::from_inputs`] from the three orthogonal
/// inputs the decoder receives at open time (cookie, descriptor,
/// flavor record) and the per-stream `frame_bytes`. Every field
/// agrees with the validated stream when fed with that stream's
/// inputs (see `tests/realstream.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeConfig {
    /// Channel count.
    pub channels: u16,
    /// Sample rate in Hz (from the flavor record).
    pub sample_rate_hz: u32,
    /// Samples per transform frame (one of `256 / 512 / 1024`).
    pub samples_per_frame: u32,
    /// Number of coded subbands.
    pub subband_count: u16,
    /// Stereo / coupling mode.
    pub stereo_mode: u16,
    /// Per-stream coded frame size in bytes (the per-call input
    /// length consumed by `RADecode`).
    pub frame_bytes: u32,
    /// Per-stream sub-packet size in bytes (from descriptor `+0x0a`).
    pub sub_packet_size: u16,
    /// Sub-packets consumed by one `RADecode` call:
    /// `frame_bytes / sub_packet_size`.
    pub sub_packets_per_call: u32,
    /// Steady-state PCM byte budget per `RADecode` call:
    /// `sub_packets_per_call × samples_per_frame × channels ×
    /// PCM_BYTES_PER_SAMPLE`.
    pub pcm_bytes_per_call: u32,
}

impl DecodeConfig {
    /// Wire cookie + descriptor + flavor into a derived config.
    ///
    /// `frame_bytes` is the per-stream coded frame size carried by the
    /// container header (block-align / `coded_frame_size` in the
    /// RealAudio v5 header — `465` on the validated stream).
    ///
    /// # Errors
    ///
    /// - [`Error::CookieInvalidChannels`] /
    ///   [`Error::CookieZeroSubbandCount`] /
    ///   [`Error::CookieZeroSamplesProduct`] if the cookie is
    ///   structurally malformed (see
    ///   [`CookCookie::validate_geometry`]). A cookie obtained from
    ///   [`CookCookie::parse`] is already validated; the explicit
    ///   re-check here catches literal-constructed cookies that bypass
    ///   the parser.
    /// - [`Error::ZeroDivisorChannels`] if the descriptor's
    ///   `channels_divisor` is `0` (would divide-by-zero in
    ///   `backend init 0x20c0`).
    /// - [`Error::ZeroDivisorSubPacketSize`] if the descriptor's
    ///   `sub_packet_size` is `0` (would divide-by-zero in
    ///   `RADecode 0x1290`).
    /// - [`Error::CookieFlavorMismatch`] if the cookie does not
    ///   self-describe the named flavor record (channels, subband
    ///   count, stereo mode, or recovered samples-per-frame disagree).
    /// - [`Error::FrameNotDivisibleBySubPacket`] if `frame_bytes` is
    ///   not an integer multiple of `sub_packet_size` (`RADecode`'s
    ///   `div [esi+8]` would leave a non-zero remainder, leaving
    ///   bytes in the carry buffer that the next call cannot align).
    pub fn from_inputs(
        cookie: &CookCookie,
        descriptor: &Descriptor,
        flavor: &FlavorRecord,
        frame_bytes: u32,
    ) -> Result<Self, Error> {
        // Structural well-formedness of the cookie body (channels
        // in {1, 2}, non-zero subband count, non-zero spf×ch product).
        // A parser-built cookie has already passed this; the re-check
        // closes the gap for callers building a `CookCookie` literal
        // (e.g. test fixtures or cached wire snapshots).
        cookie.validate_geometry()?;
        if descriptor.channels_divisor == 0 {
            return Err(Error::ZeroDivisorChannels);
        }
        if descriptor.sub_packet_size == 0 {
            return Err(Error::ZeroDivisorSubPacketSize);
        }
        if !cookie.matches_flavor(flavor) {
            return Err(Error::CookieFlavorMismatch);
        }

        // Independent recovery: the cookie's product divided by the
        // descriptor's channel scalar should agree with the flavor's
        // samples_per_frame. Anything else would mean the descriptor
        // came from a different stream than the cookie.
        let recovered_spf =
            cookie.samples_per_frame_x_channels as u32 / descriptor.channels_divisor as u32;
        if recovered_spf != flavor.samples_per_frame {
            return Err(Error::CookieFlavorMismatch);
        }

        if frame_bytes % descriptor.sub_packet_size as u32 != 0 {
            return Err(Error::FrameNotDivisibleBySubPacket {
                frame_bytes,
                sub_packet_size: descriptor.sub_packet_size,
            });
        }
        let sub_packets_per_call = frame_bytes / descriptor.sub_packet_size as u32;
        let pcm_bytes_per_call = sub_packets_per_call
            * flavor.samples_per_frame
            * flavor.channels
            * PCM_BYTES_PER_SAMPLE;

        Ok(DecodeConfig {
            channels: cookie.channels,
            sample_rate_hz: flavor.sample_rate_hz,
            samples_per_frame: flavor.samples_per_frame,
            subband_count: cookie.subband_count,
            stereo_mode: cookie.stereo_mode,
            frame_bytes,
            sub_packet_size: descriptor.sub_packet_size,
            sub_packets_per_call,
            pcm_bytes_per_call,
        })
    }

    /// PCM bytes the decoder would emit across `n` `RADecode` calls
    /// in steady state.
    ///
    /// Excludes the first-call overlap-add warm-up (the validator
    /// observed `8 192` bytes on the first call vs `20 480` in steady
    /// state — see `validation/04` §5); use [`DecodeConfig::
    /// total_pcm_bytes`] for an exact match against the validator's
    /// `2 936 832` bytes figure.
    pub fn steady_state_pcm_bytes(&self, calls: u32) -> u64 {
        calls as u64 * self.pcm_bytes_per_call as u64
    }

    /// PCM bytes emitted by the first `RADecode` call (the overlap-add
    /// warm-up).
    ///
    /// Pinned by the validator (`validation/04` §5): the first call on
    /// the real stream emitted `8 192` bytes vs `20 480` in steady
    /// state. `8 192 = 2 × samples_per_frame × channels ×
    /// PCM_BYTES_PER_SAMPLE` on the validated stream (`2 × 1024 × 2 ×
    /// 2`): the decoder primes the overlap-add chain by emitting two
    /// transform-frames' worth of PCM on the first call, then settles
    /// into the steady-state `sub_packets_per_call`-frames-per-call
    /// cadence on every subsequent call.
    pub fn warmup_pcm_bytes(&self) -> u64 {
        2 * self.samples_per_frame as u64 * self.channels as u64 * PCM_BYTES_PER_SAMPLE as u64
    }

    /// Total PCM bytes the decoder emits across `calls` `RADecode`
    /// calls, accounting for the first-call overlap-add warm-up.
    ///
    /// The first call emits [`Self::warmup_pcm_bytes`]; every
    /// subsequent call emits [`Self::pcm_bytes_per_call`].
    ///
    /// Returns `0` if `calls == 0`.
    pub fn total_pcm_bytes(&self, calls: u32) -> u64 {
        if calls == 0 {
            return 0;
        }
        let warmup = self.warmup_pcm_bytes();
        let steady = (calls as u64 - 1) * self.pcm_bytes_per_call as u64;
        warmup + steady
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flavor::flavor_record;

    /// Cookie / descriptor / frame_bytes triple from the validated
    /// `FUN_RM_32.rm` stream (`docs/audio/cook/validation/04-cook-
    /// stream-validation.md`).
    const REAL_COOKIE: [u8; 16] = [
        0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00,
        0x04,
    ];
    const REAL_DESCRIPTOR: Descriptor = Descriptor {
        channels_divisor: 2,
        sub_packet_size: 93,
    };
    const REAL_FRAME_BYTES: u32 = 465;

    fn real_cookie() -> CookCookie {
        CookCookie::parse(&REAL_COOKIE).unwrap()
    }

    fn real_flavor() -> FlavorRecord {
        flavor_record(21).unwrap()
    }

    #[test]
    fn real_stream_derives_validated_geometry() {
        let cfg = DecodeConfig::from_inputs(
            &real_cookie(),
            &REAL_DESCRIPTOR,
            &real_flavor(),
            REAL_FRAME_BYTES,
        )
        .unwrap();
        // Direct cookie/flavor passthrough.
        assert_eq!(cfg.channels, 2);
        assert_eq!(cfg.sample_rate_hz, 44100);
        assert_eq!(cfg.samples_per_frame, 1024);
        assert_eq!(cfg.subband_count, 32);
        assert_eq!(cfg.stereo_mode, 4);
        assert_eq!(cfg.frame_bytes, 465);
        assert_eq!(cfg.sub_packet_size, 93);
        // Validator §5: `465 / 93 = 5` sub-packets per call.
        assert_eq!(cfg.sub_packets_per_call, 5);
        // Validator §5: per-call output 20480 bytes in steady state =
        // 5 × 1024 × 2 ch × 2 bytes.
        assert_eq!(cfg.pcm_bytes_per_call, 20480);
    }

    #[test]
    fn zero_channels_divisor_rejected() {
        let bad = Descriptor {
            channels_divisor: 0,
            ..REAL_DESCRIPTOR
        };
        assert_eq!(
            DecodeConfig::from_inputs(&real_cookie(), &bad, &real_flavor(), REAL_FRAME_BYTES),
            Err(Error::ZeroDivisorChannels),
        );
    }

    #[test]
    fn zero_sub_packet_size_rejected() {
        let bad = Descriptor {
            channels_divisor: 2,
            sub_packet_size: 0,
        };
        assert_eq!(
            DecodeConfig::from_inputs(&real_cookie(), &bad, &real_flavor(), REAL_FRAME_BYTES),
            Err(Error::ZeroDivisorSubPacketSize),
        );
    }

    #[test]
    fn mismatched_flavor_rejected() {
        // Flavor 4 is mono 1024 spf 44.1 kHz — channels disagree with
        // the stereo cookie.
        let other = flavor_record(4).unwrap();
        assert_eq!(
            DecodeConfig::from_inputs(&real_cookie(), &REAL_DESCRIPTOR, &other, REAL_FRAME_BYTES),
            Err(Error::CookieFlavorMismatch),
        );
    }

    #[test]
    fn non_integer_sub_packet_division_rejected() {
        // 466 / 93 has remainder.
        let err = DecodeConfig::from_inputs(&real_cookie(), &REAL_DESCRIPTOR, &real_flavor(), 466)
            .unwrap_err();
        assert_eq!(
            err,
            Error::FrameNotDivisibleBySubPacket {
                frame_bytes: 466,
                sub_packet_size: 93
            }
        );
    }

    #[test]
    fn cross_check_with_wrong_descriptor_channel_count() {
        // Pretend the descriptor came from a mono build: divisor=1
        // would recover samples_per_frame=2048, disagreeing with the
        // flavor's 1024.
        let mono_div = Descriptor {
            channels_divisor: 1,
            sub_packet_size: 93,
        };
        assert_eq!(
            DecodeConfig::from_inputs(&real_cookie(), &mono_div, &real_flavor(), REAL_FRAME_BYTES),
            Err(Error::CookieFlavorMismatch),
        );
    }

    #[test]
    fn total_pcm_bytes_matches_validator_for_144_calls() {
        // Validator §5: 144 RADecode calls → 2 936 832 PCM bytes
        // (first call 8 192 = 2 frames warm-up; 143 steady-state ×
        // 20 480).
        let cfg = DecodeConfig::from_inputs(
            &real_cookie(),
            &REAL_DESCRIPTOR,
            &real_flavor(),
            REAL_FRAME_BYTES,
        )
        .unwrap();
        let warmup_bytes = 2u64 * 1024 * 2 * 2; // 8 192
        let steady_bytes = 143u64 * 20_480; // 2 928 640
        assert_eq!(warmup_bytes, 8_192);
        assert_eq!(steady_bytes + warmup_bytes, 2_936_832);
        assert_eq!(cfg.warmup_pcm_bytes(), warmup_bytes);
        assert_eq!(cfg.total_pcm_bytes(144), 2_936_832);
        // Steady-state-only accounting is documented to exclude the
        // warm-up: 144 × 20 480 = 2 949 120.
        assert_eq!(cfg.steady_state_pcm_bytes(144), 2_949_120);
        assert_eq!(cfg.total_pcm_bytes(0), 0);
        assert_eq!(cfg.total_pcm_bytes(1), warmup_bytes);
    }

    #[test]
    fn radecode_flags_constant_is_validated_value() {
        // `validation/04` §4.3: flags=1 enables backend decode.
        assert_eq!(RADECODE_FLAGS_DECODE, 1);
        // Backend's gate sees the inverted bit 0.
        assert_eq!((!RADECODE_FLAGS_DECODE) & 1, 0);
    }

    // --- structural cookie-geometry guards (spec/02 §1) ---

    #[test]
    fn literal_cookie_with_invalid_channels_rejected_before_divisor_checks() {
        // A consumer constructing a `CookCookie` literal directly
        // (bypassing `parse`) gets the same structural rejection.
        // Pick a cookie with channels=3: validate_geometry catches it
        // before the cookie/flavor cross-check would have surfaced
        // CookieFlavorMismatch.
        let bad = crate::cookie::CookCookie {
            selector: crate::cookie::SELECTOR_EXTENDED,
            samples_per_frame_x_channels: 2048,
            subband_count: 32,
            reserved: 0,
            channels: 3,
            stereo_mode: 4,
        };
        let err =
            DecodeConfig::from_inputs(&bad, &REAL_DESCRIPTOR, &real_flavor(), REAL_FRAME_BYTES)
                .unwrap_err();
        assert_eq!(err, Error::CookieInvalidChannels { got: 3 });
    }

    #[test]
    fn literal_cookie_with_zero_subband_count_rejected_before_divisor_checks() {
        let bad = crate::cookie::CookCookie {
            selector: crate::cookie::SELECTOR_EXTENDED,
            samples_per_frame_x_channels: 2048,
            subband_count: 0,
            reserved: 0,
            channels: 2,
            stereo_mode: 4,
        };
        let err =
            DecodeConfig::from_inputs(&bad, &REAL_DESCRIPTOR, &real_flavor(), REAL_FRAME_BYTES)
                .unwrap_err();
        assert_eq!(err, Error::CookieZeroSubbandCount);
    }

    #[test]
    fn literal_cookie_with_zero_samples_product_rejected_before_divisor_checks() {
        let bad = crate::cookie::CookCookie {
            selector: crate::cookie::SELECTOR_EXTENDED,
            samples_per_frame_x_channels: 0,
            subband_count: 32,
            reserved: 0,
            channels: 2,
            stereo_mode: 4,
        };
        let err =
            DecodeConfig::from_inputs(&bad, &REAL_DESCRIPTOR, &real_flavor(), REAL_FRAME_BYTES)
                .unwrap_err();
        assert_eq!(err, Error::CookieZeroSamplesProduct);
    }

    #[test]
    fn cookie_validation_runs_before_descriptor_divisor_checks() {
        // A bad cookie + a zero descriptor divisor: the cookie check
        // surfaces first (cookie validation runs at the top of
        // `from_inputs`).
        let bad = crate::cookie::CookCookie {
            selector: crate::cookie::SELECTOR_EXTENDED,
            samples_per_frame_x_channels: 2048,
            subband_count: 32,
            reserved: 0,
            channels: 5,
            stereo_mode: 4,
        };
        let bad_desc = Descriptor {
            channels_divisor: 0,
            sub_packet_size: 0,
        };
        let err = DecodeConfig::from_inputs(&bad, &bad_desc, &real_flavor(), REAL_FRAME_BYTES)
            .unwrap_err();
        assert_eq!(err, Error::CookieInvalidChannels { got: 5 });
    }
}
