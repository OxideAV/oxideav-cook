//! `RADecode` call-sequence session state — the third structural
//! decode-pipeline stage above the backend frame-decode.
//!
//! Source-of-truth: `docs/audio/cook/spec/01-cook-decoder-structure.md` §5
//! (the `RADecode` driver `0x1260` tracks a residual / carry count at
//! context `+0x20` and uses `memmove` to retain leftover bytes between
//! calls — the de-interleave carry buffer) and
//! `docs/audio/cook/validation/04-cook-stream-validation.md` §5 (the
//! validated operating mode where 144 container packets are fed in stream
//! order with no external descramble or de-interleave, the first call
//! emits the `8 192`-byte overlap-add warm-up, every subsequent call
//! emits the steady-state `20 480` bytes, and the total accumulates to
//! `2 936 832` bytes at call 144).
//!
//! ## What the binary does
//!
//! `RADecode` (`0x1260`) is a per-call entry point. Each call:
//!
//! 1. Reads `input_len` and divides by the per-frame divisor
//!    `[esi+8]` (= `sub_packet_size`); the quotient is
//!    `sub_packets_per_call`, the remainder is a hard rejection
//!    (modelled by [`crate::DecodeConfig::from_inputs`]).
//! 2. Walks the call's input as fixed-stride sub-packet slots
//!    (modelled by [`crate::SubPacketLayout`]).
//! 3. Invokes the backend frame-decode method
//!    `[backend_vtable + 0x0c]` exactly **once per call**
//!    (validation/04 §4.3 + §5); subsequent sub-packets in the same call
//!    are consumed through the carry buffer at context `+0x20`.
//! 4. Emits PCM at the validator-pinned cadence: first call
//!    [`crate::DecodeConfig::warmup_pcm_bytes`] bytes, then
//!    [`crate::DecodeConfig::pcm_bytes_per_call`] bytes per call.
//!
//! Across a session of `N` calls, the running total PCM is exactly
//! [`crate::DecodeConfig::total_pcm_bytes`]`(N)` (= `2 936 832` at
//! `N = 144` on the validated stream).
//!
//! ## What this module provides
//!
//! Pure-Rust modelling of the per-call **driver state** the structural
//! stages do not capture on their own: the call counter, the running
//! PCM cursor, the expected per-call input length, and the
//! validator-pinned output budget for the next call. The session never
//! invokes the backend frame-decode (that is still
//! [`crate::Error::NotImplemented`] in this crate); it gives consumers
//! a deterministic, allocation-free way to walk a `RADecode` call
//! sequence and reason about input/output sizing without owning the
//! transform pipeline.
//!
//! - [`CallSession::new`] builds a session from a [`SubPacketLayout`].
//! - [`CallSession::calls_completed`] returns the running count.
//! - [`CallSession::next_call_pcm_byte_range`] returns the byte range
//!   the next call's PCM output occupies inside the concatenated
//!   stream PCM (validator-pinned: `[0, 8 192)` for the first call,
//!   then strides of `20 480` on the validated stream).
//! - [`CallSession::next_call_expected_input_len`] returns the
//!   per-call input length (= [`SubPacketLayout::frame_bytes`]).
//! - [`CallSession::next_call_pcm_bytes`] returns the PCM budget of
//!   the next call alone (warm-up on call 0, steady-state thereafter).
//! - [`CallSession::advance_one_call`] consumes one call: validates
//!   that the input length matches and that the output buffer length
//!   matches the call's PCM budget, then increments the counter.
//! - [`CallSession::total_pcm_emitted`] returns the running PCM cursor
//!   (= [`crate::DecodeConfig::total_pcm_bytes`]`(calls_completed)`).
//! - [`crate::Error::CallInputLengthMismatch`] /
//!   [`crate::Error::CallOutputLengthMismatch`] surface misuse.

use crate::{init::DecodeConfig, subpacket::SubPacketLayout, Error};

/// Stateful walker over a `RADecode` call sequence.
///
/// Built by [`CallSession::new`] from a [`SubPacketLayout`]. Each
/// successful [`CallSession::advance_one_call`] increments the call
/// counter and the running PCM cursor by the validator-pinned per-call
/// budget (warm-up on the first call, steady-state thereafter). The
/// session never decodes — it walks the driver-level cadence the
/// validator measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallSession {
    layout: SubPacketLayout,
    calls_completed: u32,
    total_pcm_emitted: u64,
}

impl CallSession {
    /// Build a fresh session at call counter `0`.
    ///
    /// The session captures the [`SubPacketLayout`] by value (it is
    /// `Copy`) and starts at `calls_completed = 0`,
    /// `total_pcm_emitted = 0`.
    pub fn new(layout: SubPacketLayout) -> Self {
        CallSession {
            layout,
            calls_completed: 0,
            total_pcm_emitted: 0,
        }
    }

    /// Build a session directly from a wired [`DecodeConfig`].
    ///
    /// Equivalent to
    /// `CallSession::new(SubPacketLayout::from_config(cfg))`.
    pub fn from_config(cfg: &DecodeConfig) -> Self {
        Self::new(SubPacketLayout::from_config(cfg))
    }

    /// The wired layout the session walks.
    pub fn layout(&self) -> SubPacketLayout {
        self.layout
    }

    /// Number of `RADecode` calls completed so far.
    pub fn calls_completed(&self) -> u32 {
        self.calls_completed
    }

    /// Running total of PCM bytes the session has accounted for.
    ///
    /// Equal to [`DecodeConfig::total_pcm_bytes`]`(calls_completed)`
    /// at every step (validation/04 §5).
    pub fn total_pcm_emitted(&self) -> u64 {
        self.total_pcm_emitted
    }

    /// Per-call input length the next call expects.
    ///
    /// Equal to [`SubPacketLayout::frame_bytes`] (the validated stream's
    /// `465`-byte container packet).
    pub fn next_call_expected_input_len(&self) -> u32 {
        self.layout.frame_bytes
    }

    /// PCM byte budget the next call alone will emit.
    ///
    /// Validator-pinned (validation/04 §5):
    /// - on call `0` (the first), [`SubPacketLayout::warmup_pcm_bytes`]
    ///   (= `8 192` on the validated stream);
    /// - on every call thereafter, [`SubPacketLayout::pcm_bytes_per_call`]
    ///   (= `20 480` on the validated stream).
    pub fn next_call_pcm_bytes(&self) -> u32 {
        if self.calls_completed == 0 {
            self.layout.warmup_pcm_bytes
        } else {
            self.layout.pcm_bytes_per_call
        }
    }

    /// Byte range the next call's PCM output occupies inside the
    /// concatenated stream PCM.
    ///
    /// Returns `[total_pcm_emitted, total_pcm_emitted +
    /// next_call_pcm_bytes)`. Equivalent to
    /// `[SubPacketLayout::pcm_offset_for_call(n),
    /// SubPacketLayout::pcm_offset_for_call(n + 1))` for
    /// `n = calls_completed`.
    pub fn next_call_pcm_byte_range(&self) -> core::ops::Range<u64> {
        let start = self.total_pcm_emitted;
        let end = start + self.next_call_pcm_bytes() as u64;
        start..end
    }

    /// Account for one completed `RADecode` call.
    ///
    /// Validates that
    /// - `input_len == next_call_expected_input_len()`
    ///   (= [`SubPacketLayout::frame_bytes`]) — `RADecode`'s `div
    ///   [esi+8]` invariant; a mismatch is an
    ///   [`Error::CallInputLengthMismatch`].
    /// - `output_len == next_call_pcm_bytes()` — the validator-pinned
    ///   per-call PCM budget; a mismatch is an
    ///   [`Error::CallOutputLengthMismatch`].
    ///
    /// On success increments the call counter and adds
    /// `next_call_pcm_bytes()` to the running PCM cursor.
    ///
    /// This method does **not** invoke the backend frame-decode (that
    /// is still [`Error::NotImplemented`]); it walks the driver-level
    /// cadence so consumers can pre-size their output buffers exactly
    /// and pin their accounting against the validator without
    /// duplicating the offset arithmetic.
    ///
    /// # Errors
    ///
    /// - [`Error::CallInputLengthMismatch`] if `input_len` disagrees
    ///   with the per-call input length.
    /// - [`Error::CallOutputLengthMismatch`] if `output_len` disagrees
    ///   with the per-call PCM budget.
    pub fn advance_one_call(&mut self, input_len: usize, output_len: usize) -> Result<(), Error> {
        let expected_in = self.next_call_expected_input_len() as usize;
        if input_len != expected_in {
            return Err(Error::CallInputLengthMismatch {
                got: input_len,
                expected: expected_in,
            });
        }
        let expected_out = self.next_call_pcm_bytes() as usize;
        if output_len != expected_out {
            return Err(Error::CallOutputLengthMismatch {
                got: output_len,
                expected: expected_out,
            });
        }
        // u32 + u32 fits in u64 by construction (Cook geometry: max
        // 20 480 bytes/call × 2^32 calls < 2^48).
        self.total_pcm_emitted += expected_out as u64;
        self.calls_completed += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cookie::CookCookie, flavor::flavor_record, init::Descriptor, init::PCM_BYTES_PER_SAMPLE,
    };

    // Validated stream pins (validation/04 §2.1 / §4 / §5).
    const REAL_COOKIE: [u8; 16] = [
        0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00,
        0x04,
    ];
    const REAL_DESCRIPTOR: Descriptor = Descriptor {
        channels_divisor: 2,
        sub_packet_size: 93,
    };
    const REAL_FRAME_BYTES: u32 = 465;

    fn real_config() -> DecodeConfig {
        let cookie = CookCookie::parse(&REAL_COOKIE).unwrap();
        let flavor = flavor_record(21).unwrap();
        DecodeConfig::from_inputs(&cookie, &REAL_DESCRIPTOR, &flavor, REAL_FRAME_BYTES).unwrap()
    }

    fn real_session() -> CallSession {
        CallSession::from_config(&real_config())
    }

    #[test]
    fn fresh_session_at_zero() {
        let s = real_session();
        assert_eq!(s.calls_completed(), 0);
        assert_eq!(s.total_pcm_emitted(), 0);
        // First call emits the warm-up.
        assert_eq!(s.next_call_pcm_bytes(), 8_192);
        assert_eq!(s.next_call_expected_input_len(), 465);
        assert_eq!(s.next_call_pcm_byte_range(), 0u64..8_192);
    }

    #[test]
    fn next_call_after_one_advance_is_steady_state() {
        let mut s = real_session();
        s.advance_one_call(465, 8_192).unwrap();
        assert_eq!(s.calls_completed(), 1);
        assert_eq!(s.total_pcm_emitted(), 8_192);
        // Call 1 emits steady-state 20_480.
        assert_eq!(s.next_call_pcm_bytes(), 20_480);
        assert_eq!(s.next_call_pcm_byte_range(), 8_192u64..8_192 + 20_480);
    }

    #[test]
    fn full_144_call_sequence_matches_validator_total() {
        // Validator §5: 144 calls × 465-byte input → 2 936 832 bytes
        // PCM (8 192 warm-up + 143 × 20 480 steady-state).
        let mut s = real_session();
        // Call 0: warm-up.
        s.advance_one_call(465, 8_192).unwrap();
        // Calls 1..144: steady-state.
        for _ in 1..144 {
            s.advance_one_call(465, 20_480).unwrap();
        }
        assert_eq!(s.calls_completed(), 144);
        assert_eq!(s.total_pcm_emitted(), 2_936_832);
        // The next call's range starts at the pinned total.
        assert_eq!(s.next_call_pcm_byte_range().start, 2_936_832);
    }

    #[test]
    fn input_length_mismatch_rejected() {
        let mut s = real_session();
        // 464 != 465 (the per-call input length).
        let err = s.advance_one_call(464, 8_192).unwrap_err();
        assert_eq!(
            err,
            Error::CallInputLengthMismatch {
                got: 464,
                expected: 465
            }
        );
        // State unchanged after the rejection.
        assert_eq!(s.calls_completed(), 0);
        assert_eq!(s.total_pcm_emitted(), 0);
    }

    #[test]
    fn output_length_mismatch_rejected_on_warmup() {
        let mut s = real_session();
        // 20_480 is the steady-state budget but the first call expects
        // the 8_192 warm-up.
        let err = s.advance_one_call(465, 20_480).unwrap_err();
        assert_eq!(
            err,
            Error::CallOutputLengthMismatch {
                got: 20_480,
                expected: 8_192
            }
        );
        // State unchanged.
        assert_eq!(s.calls_completed(), 0);
        assert_eq!(s.total_pcm_emitted(), 0);
    }

    #[test]
    fn output_length_mismatch_rejected_on_steady_state() {
        let mut s = real_session();
        // Walk the warm-up correctly.
        s.advance_one_call(465, 8_192).unwrap();
        // 8_192 is the warm-up budget but call 1 expects steady-state.
        let err = s.advance_one_call(465, 8_192).unwrap_err();
        assert_eq!(
            err,
            Error::CallOutputLengthMismatch {
                got: 8_192,
                expected: 20_480
            }
        );
        // State unchanged after the rejection.
        assert_eq!(s.calls_completed(), 1);
        assert_eq!(s.total_pcm_emitted(), 8_192);
    }

    #[test]
    fn total_pcm_emitted_tracks_decode_config_total() {
        // For every k ∈ [0, 144], the running PCM cursor equals
        // DecodeConfig::total_pcm_bytes(k).
        let cfg = real_config();
        let mut s = CallSession::from_config(&cfg);
        for k in 0..144u32 {
            assert_eq!(s.calls_completed(), k);
            assert_eq!(s.total_pcm_emitted(), cfg.total_pcm_bytes(k));
            let want_pcm = if k == 0 { 8_192 } else { 20_480 };
            s.advance_one_call(465, want_pcm).unwrap();
        }
        assert_eq!(s.calls_completed(), 144);
        assert_eq!(s.total_pcm_emitted(), cfg.total_pcm_bytes(144));
        assert_eq!(s.total_pcm_emitted(), 2_936_832);
    }

    #[test]
    fn pcm_byte_range_chains_with_subpacket_layout() {
        // CallSession::next_call_pcm_byte_range() at call k matches
        // [SubPacketLayout::pcm_offset_for_call(k),
        //  SubPacketLayout::pcm_offset_for_call(k+1)).
        let cfg = real_config();
        let layout = SubPacketLayout::from_config(&cfg);
        let mut s = CallSession::new(layout);
        for k in 0..144u32 {
            let want_start = layout.pcm_offset_for_call(k);
            let want_end = layout.pcm_offset_for_call(k + 1);
            let got = s.next_call_pcm_byte_range();
            assert_eq!(got, want_start..want_end, "call {k}");
            let want_pcm = if k == 0 { 8_192 } else { 20_480 };
            s.advance_one_call(465, want_pcm).unwrap();
        }
    }

    #[test]
    fn from_config_matches_new_from_layout() {
        let cfg = real_config();
        let a = CallSession::from_config(&cfg);
        let b = CallSession::new(SubPacketLayout::from_config(&cfg));
        assert_eq!(a, b);
    }

    #[test]
    fn warmup_constant_derived_from_geometry() {
        // The session's warm-up budget is 2 × spf × ch × pcm_bps —
        // derived from the wired DecodeConfig, not hard-coded.
        let cfg = real_config();
        let s = CallSession::from_config(&cfg);
        let spf = 1024u32;
        let ch = 2u32;
        let pcm_bps = PCM_BYTES_PER_SAMPLE;
        assert_eq!(s.next_call_pcm_bytes(), 2 * spf * ch * pcm_bps);
    }
}
