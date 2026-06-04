//! `RADecode`-equivalent per-call driver — the orchestrator that wires
//! the structural decode-pipeline stages together.
//!
//! Source-of-truth: `docs/audio/cook/spec/01-cook-decoder-structure.md` §5
//! (the body of the per-call decode driver `0x1260`: per-buffer XOR
//! descramble gated by the `+0x30` common-mode flag, per-frame divisor
//! `div [esi+8]`, sub-packet iteration, backend frame-decode method
//! `[backend_vtable + 0x0c]` invoked once per call, carry-buffer
//! accounting at context `+0x20`) and
//! `docs/audio/cook/validation/04-cook-stream-validation.md` §5 (the
//! validated per-call cadence: 144 container packets are fed in stream
//! order with no external descramble or de-interleave, the first call
//! emits the `8 192`-byte overlap-add warm-up, every subsequent call
//! emits the steady-state `20 480` bytes, and the total accumulates to
//! `2 936 832` bytes at call 144).
//!
//! ## What the binary does
//!
//! `RADecode` (`0x1260`) is the per-call entry point. Each call:
//!
//! 1. Optionally applies the per-buffer XOR descramble
//!    ([`crate::descramble`]); gated on the common-mode flag at context
//!    `+0x30` ([`CommonMode`]). The default is off — the validated
//!    real-stream path.
//! 2. Divides `input_len` by `[esi+8]` (= `sub_packet_size`); the
//!    quotient is `sub_packets_per_call`, the remainder is a hard
//!    rejection (the parent [`DecodeConfig::from_inputs`] enforces the
//!    divisibility invariant at open time so the driver can rely on it).
//! 3. Walks the call's input as fixed-stride sub-packet slots
//!    ([`SubPacketLayout::iter_call`]).
//! 4. Invokes the backend frame-decode method
//!    `[backend_vtable + 0x0c]` exactly **once per call**
//!    (`validation/04` §4.3 + §5); subsequent sub-packets in the same
//!    call are consumed through the carry buffer at context `+0x20`.
//! 5. Emits PCM at the validator-pinned cadence: first call
//!    [`DecodeConfig::warmup_pcm_bytes`] bytes, then
//!    [`DecodeConfig::pcm_bytes_per_call`] bytes per call.
//!
//! ## What this module provides
//!
//! A pure-Rust orchestrator that wires together the four structural
//! decode-pipeline stages already in this crate
//! ([`crate::descramble`], [`crate::subpacket`], [`crate::session`])
//! into a single per-call entry point that mirrors the `RADecode` shape
//! without owning the backend frame-decode (that is still a
//! [`crate::Error::NotImplemented`] GAP — modelling it requires the
//! bitstream reader, MDCT, and gain/quantiser, all of which land in
//! later rounds).
//!
//! - [`Driver`] — the orchestrator. Holds a [`DecodeConfig`], a
//!   [`CommonMode`] toggle, and an embedded [`CallSession`] for the
//!   per-call cadence. Built by [`Driver::new`].
//! - [`Driver::prepare_call`] — the validation + stage 1+2 orchestrator.
//!   Validates the input length, optionally runs the XOR descramble,
//!   and returns a [`PreparedCall`] that exposes the descrambled byte
//!   view and the sub-packet iterator. Does **not** advance the session
//!   cursor (the backend has not been invoked yet); call
//!   [`Driver::advance_after_decode`] once the consumer's backend has
//!   filled the per-call PCM budget.
//! - [`Driver::decode_call`] — the full-pipeline per-call entry point.
//!   Wires stages 1–5; the backend invocation itself still surfaces as
//!   [`crate::Error::NotImplemented`]. Provided so the public decode
//!   path has a single orchestrated entry point even before the backend
//!   lands.
//! - [`PreparedCall`] — the descrambled + length-checked view of one
//!   call's input. Exposes the sub-packet slices via
//!   [`PreparedCall::iter_sub_packets`] and the descrambled bytes via
//!   [`PreparedCall::descrambled`].
//!
//! ## Wall-respect note
//!
//! Every behavioural fact this module pins is anchored to spec/01 §5 or
//! validation/04 §5; the orchestration shape is the binary's own (per
//! the spec) and the carry-buffer mechanics that the binary keeps
//! internal are not modelled here (the validator measured that
//! container packets can be fed directly in stream order — no external
//! carry handling is needed for the validated path).

use std::borrow::Cow;

use crate::{
    descramble::{descramble_packet, CommonMode},
    init::DecodeConfig,
    session::CallSession,
    subpacket::SubPacketLayout,
    Error,
};

/// Per-call decode driver — the orchestrator above the backend
/// frame-decode.
///
/// Holds a wired [`DecodeConfig`], the [`CommonMode`] toggle that gates
/// the per-buffer XOR descramble (default off — the validated
/// real-stream path), and an embedded [`CallSession`] tracking the
/// per-call cadence. Built by [`Driver::new`].
///
/// The driver does **not** own the backend frame-decode (the bitstream
/// reader, MDCT, gain/quantiser are all later-round work);
/// [`Driver::decode_call`] surfaces them as [`Error::NotImplemented`]
/// while [`Driver::prepare_call`] gives consumers the orchestrated
/// stage 1+2 output (descramble + sub-packet split) so they can plug a
/// future backend implementation in without re-deriving the
/// orchestration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Driver {
    config: DecodeConfig,
    common_mode: CommonMode,
    session: CallSession,
}

impl Driver {
    /// Build a fresh driver from a wired [`DecodeConfig`].
    ///
    /// The session starts at call counter `0`; the common-mode flag
    /// starts off (the constructor default — `RASetComMode` sets it,
    /// nothing clears it once set; the validated real-stream path is
    /// the off-path: `validation/04` §4.3 / §5). Use
    /// [`Driver::with_common_mode`] to start with common mode on.
    pub fn new(config: DecodeConfig) -> Self {
        let session = CallSession::from_config(&config);
        Driver {
            config,
            common_mode: CommonMode::off(),
            session,
        }
    }

    /// Builder-style: set the initial common-mode flag.
    ///
    /// Mirrors the binary contract that `RASetComMode` (spec/01 §2,
    /// worker `0x16a0`) sets the context `+0x30` flag; the constructor
    /// default is off, the validated real-stream path is off, and
    /// there is no SPI in this build to clear the flag once set.
    pub const fn with_common_mode(mut self, mode: CommonMode) -> Self {
        self.common_mode = mode;
        self
    }

    /// The wired configuration.
    pub fn config(&self) -> &DecodeConfig {
        &self.config
    }

    /// Current common-mode state (gates the per-buffer XOR descramble).
    pub fn common_mode(&self) -> CommonMode {
        self.common_mode
    }

    /// Set the common-mode flag at runtime (the `RASetComMode` analog).
    ///
    /// Spec/01 §2: `RASetComMode` only sets the flag to `1`; the
    /// binary has no SPI to clear it. This Rust API exposes both
    /// transitions for testability; consumers tracking the binary
    /// contract verbatim can keep [`CommonMode::off`] as the construction
    /// default and call this once with [`CommonMode::on`] if they need
    /// the XOR path.
    pub fn set_common_mode(&mut self, mode: CommonMode) {
        self.common_mode = mode;
    }

    /// The validator-pinned per-call sub-packet split layout.
    pub fn layout(&self) -> SubPacketLayout {
        self.session.layout()
    }

    /// Number of completed `RADecode` calls.
    pub fn calls_completed(&self) -> u32 {
        self.session.calls_completed()
    }

    /// Running PCM cursor — total bytes emitted across completed calls.
    ///
    /// Equal to [`DecodeConfig::total_pcm_bytes`]`(calls_completed)` at
    /// every step (validation/04 §5).
    pub fn total_pcm_emitted(&self) -> u64 {
        self.session.total_pcm_emitted()
    }

    /// Expected input length of the next call (= `frame_bytes`).
    pub fn next_call_expected_input_len(&self) -> u32 {
        self.session.next_call_expected_input_len()
    }

    /// PCM byte budget of the next call alone.
    ///
    /// Validator-pinned (validation/04 §5): warm-up on call `0`,
    /// steady-state thereafter.
    pub fn next_call_pcm_bytes(&self) -> u32 {
        self.session.next_call_pcm_bytes()
    }

    /// Byte range the next call's PCM output occupies inside the
    /// concatenated stream PCM.
    pub fn next_call_pcm_byte_range(&self) -> core::ops::Range<u64> {
        self.session.next_call_pcm_byte_range()
    }

    /// Orchestrate the per-call descramble + sub-packet split (stages
    /// 1+2 of spec/01 §5).
    ///
    /// Validates that `packet.len() == frame_bytes`, runs the XOR
    /// descramble when [`Driver::common_mode`] is on (validated default
    /// is off, so the zero-copy [`Cow::Borrowed`] path is what the
    /// real-stream test exercises), and returns a [`PreparedCall`]
    /// holding the descrambled bytes plus the sub-packet iterator.
    ///
    /// Does **not** advance the session cursor; the backend has not
    /// been invoked yet. Once a consumer's backend has filled the
    /// per-call PCM budget, call [`Driver::advance_after_decode`] to
    /// account for the completed call.
    ///
    /// `xor_key` is the per-call descramble key (compute via
    /// [`crate::xor_key`]`(in_ptr, in_len)`); ignored when
    /// [`Driver::common_mode`] is off.
    ///
    /// # Errors
    ///
    /// - [`Error::CallInputLengthMismatch`] if `packet.len() !=
    ///   frame_bytes`.
    pub fn prepare_call<'a>(
        &self,
        packet: &'a [u8],
        xor_key: u32,
    ) -> Result<PreparedCall<'a>, Error> {
        let expected = self.session.next_call_expected_input_len() as usize;
        if packet.len() != expected {
            return Err(Error::CallInputLengthMismatch {
                got: packet.len(),
                expected,
            });
        }
        let descrambled = descramble_packet(self.common_mode, packet, xor_key);
        Ok(PreparedCall {
            descrambled,
            layout: self.session.layout(),
        })
    }

    /// Account for one completed call without invoking the backend.
    ///
    /// Validates that `output_len == next_call_pcm_bytes()` (the
    /// validator-pinned budget for the next call) and advances the
    /// session cursor on success. Use this after the consumer's
    /// backend has filled the call's PCM into a buffer of the expected
    /// size.
    ///
    /// # Errors
    ///
    /// - [`Error::CallOutputLengthMismatch`] if `output_len` does not
    ///   match the per-call PCM budget.
    pub fn advance_after_decode(&mut self, output_len: usize) -> Result<(), Error> {
        let input_len = self.session.next_call_expected_input_len() as usize;
        self.session.advance_one_call(input_len, output_len)
    }

    /// Full-pipeline per-call decode entry point — the `RADecode`
    /// analog (spec/01 §5).
    ///
    /// Orchestrates stages 1–5:
    ///
    /// 1. Validates the input/output lengths against the wired
    ///    geometry + the validator-pinned per-call PCM budget.
    /// 2. Runs the XOR descramble when [`Driver::common_mode`] is on.
    /// 3. Computes the sub-packet split (validator pin: 5 × 93 = 465).
    /// 4. Invokes the backend frame-decode — surfaced as
    ///    [`Error::NotImplemented`] in this build; this is the GAP that
    ///    later rounds close.
    /// 5. Advances the session cursor on success (the
    ///    [`Error::NotImplemented`] path returns before the cursor
    ///    moves; no partial state is published on failure).
    ///
    /// Provided so the public decode path has a coherent entry point
    /// even while the backend is still a GAP: external callers can
    /// already pre-size their PCM output buffers exactly with
    /// [`Driver::next_call_pcm_bytes`], hand the call to
    /// `decode_call`, and treat the resulting
    /// [`Error::NotImplemented`] as the documented signal that the
    /// transform pipeline has not landed yet.
    ///
    /// # Errors
    ///
    /// - [`Error::CallInputLengthMismatch`] / [`Error::CallOutputLengthMismatch`]
    ///   if the buffer sizes disagree with the wired per-call budget.
    /// - [`Error::NotImplemented`] from the backend frame-decode step.
    pub fn decode_call(
        &mut self,
        packet: &[u8],
        output: &mut [u8],
        xor_key: u32,
    ) -> Result<(), Error> {
        // Stage 1+2: validate, descramble, split.
        let _prepared = self.prepare_call(packet, xor_key)?;
        // Pre-validate the output size before claiming the backend was
        // even invoked — keeps `Error::NotImplemented` reserved for the
        // transform itself.
        let expected_out = self.session.next_call_pcm_bytes() as usize;
        if output.len() != expected_out {
            return Err(Error::CallOutputLengthMismatch {
                got: output.len(),
                expected: expected_out,
            });
        }
        // Stage 4: backend frame-decode — still GAP.
        Err(Error::NotImplemented)
    }
}

/// Output of [`Driver::prepare_call`] — the descrambled, length-checked
/// view of one call's input.
///
/// Exposes the descrambled bytes (a [`Cow::Borrowed`] when common mode
/// is off, [`Cow::Owned`] when it is on — see [`crate::descramble`])
/// and an iterator over the validator-pinned sub-packet slot slices
/// (`5 × 93` bytes on the validated stream).
#[derive(Debug, Clone)]
pub struct PreparedCall<'a> {
    descrambled: Cow<'a, [u8]>,
    layout: SubPacketLayout,
}

impl<'a> PreparedCall<'a> {
    /// The descrambled per-call input.
    ///
    /// Length is [`SubPacketLayout::frame_bytes`] (= `frame_bytes`).
    /// When [`Driver::common_mode`] was off this is the input buffer
    /// verbatim (zero-copy); when on it is a fresh allocation holding
    /// the result of the word-wise XOR pass (see [`crate::descramble`]).
    pub fn descrambled(&self) -> &[u8] {
        &self.descrambled
    }

    /// Sub-packets per call (= [`SubPacketLayout::sub_packets_per_call`]).
    pub fn sub_packets_per_call(&self) -> u32 {
        self.layout.sub_packets_per_call
    }

    /// Sub-packet size in bytes (= [`SubPacketLayout::sub_packet_size`]).
    pub fn sub_packet_size(&self) -> u16 {
        self.layout.sub_packet_size
    }

    /// Iterate the sub-packet slot slices for this call.
    ///
    /// Yields exactly [`Self::sub_packets_per_call`] slices of
    /// [`Self::sub_packet_size`] bytes each, in slot order
    /// `0 .. sub_packets_per_call`. The slices borrow from the
    /// descrambled bytes returned by [`Self::descrambled`].
    pub fn iter_sub_packets(&self) -> impl Iterator<Item = Result<&[u8], Error>> + '_ {
        self.layout.iter_call(&self.descrambled)
    }

    /// The wired sub-packet layout.
    pub fn layout(&self) -> SubPacketLayout {
        self.layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cookie::CookCookie,
        descramble::{xor_descramble, xor_key},
        flavor::flavor_record,
        init::{Descriptor, PCM_BYTES_PER_SAMPLE},
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

    fn real_driver() -> Driver {
        Driver::new(real_config())
    }

    #[test]
    fn fresh_driver_state() {
        let d = real_driver();
        assert_eq!(d.calls_completed(), 0);
        assert_eq!(d.total_pcm_emitted(), 0);
        assert_eq!(d.next_call_expected_input_len(), 465);
        // First call emits the validator-pinned 8 192-byte warm-up.
        assert_eq!(d.next_call_pcm_bytes(), 8_192);
        // Default common-mode is off — matches the validated path.
        assert!(!d.common_mode().is_on());
    }

    #[test]
    fn with_common_mode_sets_initial_state() {
        let d = real_driver().with_common_mode(CommonMode::on());
        assert!(d.common_mode().is_on());
    }

    #[test]
    fn set_common_mode_updates_runtime_state() {
        let mut d = real_driver();
        assert!(!d.common_mode().is_on());
        d.set_common_mode(CommonMode::on());
        assert!(d.common_mode().is_on());
        d.set_common_mode(CommonMode::off());
        assert!(!d.common_mode().is_on());
    }

    #[test]
    fn prepare_call_off_path_is_zero_copy() {
        // Default common-mode-off path: descrambled bytes are the input
        // verbatim with no allocation (validation/04 §4.3 / §5 — the
        // validated path).
        let d = real_driver();
        let packet: Vec<u8> = (0..465u32).map(|i| (i & 0xff) as u8).collect();
        let prepared = d.prepare_call(&packet, 0xDEAD_BEEF).unwrap();
        assert_eq!(prepared.descrambled(), &packet[..]);
        // The iterator yields exactly 5 × 93-byte slot slices.
        let slots: Vec<_> = prepared
            .iter_sub_packets()
            .map(|r| r.unwrap().to_vec())
            .collect();
        assert_eq!(slots.len(), 5);
        for slot in &slots {
            assert_eq!(slot.len(), 93);
        }
        // Concatenation reproduces the descrambled bytes.
        let recombined: Vec<u8> = slots.into_iter().flatten().collect();
        assert_eq!(recombined, packet);
    }

    #[test]
    fn prepare_call_on_path_runs_xor_pass() {
        // Common-mode-on path: prepared bytes are the XOR-descrambled
        // input. Round-trip back through descramble_packet to confirm
        // self-inverse (the on-path has no bit-exact validator ground
        // truth — only algebraic properties — per spec/01 §5 tail GAP).
        let d = real_driver().with_common_mode(CommonMode::on());
        let packet: Vec<u8> = (0..465u32)
            .map(|i| (i.wrapping_mul(7) & 0xff) as u8)
            .collect();
        let key = xor_key(0x6000_0000, packet.len() as u32);
        let prepared = d.prepare_call(&packet, key).unwrap();
        // Descrambled bytes differ from input (on-path actually runs the
        // pass) but a second on-path application restores them.
        assert_ne!(prepared.descrambled(), &packet[..]);
        let restored = xor_descramble(prepared.descrambled(), key);
        assert_eq!(restored, packet);
        // Sub-packet count unchanged by the descramble (geometry pin).
        assert_eq!(prepared.sub_packets_per_call(), 5);
        assert_eq!(prepared.sub_packet_size(), 93);
    }

    #[test]
    fn prepare_call_rejects_wrong_input_length() {
        let d = real_driver();
        let short = vec![0u8; 464];
        let err = d.prepare_call(&short, 0).unwrap_err();
        assert_eq!(
            err,
            Error::CallInputLengthMismatch {
                got: 464,
                expected: 465
            }
        );
    }

    #[test]
    fn prepare_call_does_not_advance_session() {
        let d = real_driver();
        let packet = vec![0u8; 465];
        let _ = d.prepare_call(&packet, 0).unwrap();
        // Counter unchanged — prepare_call orchestrates stages 1+2 only.
        assert_eq!(d.calls_completed(), 0);
        assert_eq!(d.total_pcm_emitted(), 0);
    }

    #[test]
    fn advance_after_decode_walks_validator_cadence() {
        // Walk the full 144-call sequence using the driver's advance
        // analog of the session's `advance_one_call`.
        let mut d = real_driver();
        d.advance_after_decode(8_192).unwrap();
        assert_eq!(d.calls_completed(), 1);
        assert_eq!(d.total_pcm_emitted(), 8_192);
        for _ in 1..144 {
            d.advance_after_decode(20_480).unwrap();
        }
        assert_eq!(d.calls_completed(), 144);
        assert_eq!(d.total_pcm_emitted(), 2_936_832);
    }

    #[test]
    fn advance_after_decode_rejects_wrong_output_size() {
        let mut d = real_driver();
        // The first call expects the 8 192 warm-up, not steady-state.
        let err = d.advance_after_decode(20_480).unwrap_err();
        assert_eq!(
            err,
            Error::CallOutputLengthMismatch {
                got: 20_480,
                expected: 8_192
            }
        );
        // No state mutation on rejection.
        assert_eq!(d.calls_completed(), 0);
        assert_eq!(d.total_pcm_emitted(), 0);
    }

    #[test]
    fn decode_call_validates_sizing_before_signalling_backend_gap() {
        // `decode_call` validates input + output sizes before the
        // `Error::NotImplemented` signal: a wrong-sized buffer should
        // produce a typed length error, not the backend GAP.
        let mut d = real_driver();
        let packet = vec![0u8; 464];
        let mut out = vec![0u8; 8_192];
        let err = d.decode_call(&packet, &mut out, 0).unwrap_err();
        assert!(matches!(err, Error::CallInputLengthMismatch { .. }));

        let packet = vec![0u8; 465];
        let mut out = vec![0u8; 20_480]; // wrong: first call expects 8 192
        let err = d.decode_call(&packet, &mut out, 0).unwrap_err();
        assert_eq!(
            err,
            Error::CallOutputLengthMismatch {
                got: 20_480,
                expected: 8_192
            }
        );
    }

    #[test]
    fn decode_call_surfaces_backend_gap_on_wired_sizes() {
        // With both buffer sizes wired correctly, `decode_call`
        // surfaces the backend GAP as `Error::NotImplemented`.
        let mut d = real_driver();
        let packet = vec![0u8; 465];
        let mut out = vec![0u8; 8_192];
        let err = d.decode_call(&packet, &mut out, 0).unwrap_err();
        assert_eq!(err, Error::NotImplemented);
        // The `NotImplemented` path does NOT advance the session
        // cursor (no partial state on the GAP signal).
        assert_eq!(d.calls_completed(), 0);
        assert_eq!(d.total_pcm_emitted(), 0);
    }

    #[test]
    fn driver_pcm_byte_range_matches_session() {
        // The driver delegates pcm-range queries to the embedded session
        // → both must report identical ranges at every step.
        let cfg = real_config();
        let mut d = Driver::new(cfg);
        for k in 0..144u32 {
            let want_start = if k == 0 {
                0u64
            } else {
                8_192 + (k as u64 - 1) * 20_480
            };
            let want_end = want_start + d.next_call_pcm_bytes() as u64;
            assert_eq!(d.next_call_pcm_byte_range(), want_start..want_end);
            let want_pcm = if k == 0 { 8_192 } else { 20_480 };
            d.advance_after_decode(want_pcm).unwrap();
        }
    }

    #[test]
    fn driver_geometry_derived_from_config() {
        // Sanity: the driver's per-call sizes are derived from the
        // wired DecodeConfig, not hard-coded.
        let d = real_driver();
        let layout = d.layout();
        let spf = 1024u32;
        let ch = 2u32;
        let pcm_bps = PCM_BYTES_PER_SAMPLE;
        assert_eq!(layout.warmup_pcm_bytes, 2 * spf * ch * pcm_bps);
        assert_eq!(
            layout.pcm_bytes_per_call,
            layout.sub_packets_per_call * spf * ch * pcm_bps
        );
    }
}
