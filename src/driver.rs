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
//!    (`validation/04` §4.3 + §5), forwarding `(~flags) & 1` as the
//!    backend's decode/observe gate ([`DecodeGate`]): gate `0`
//!    (`flags` bit 0 = 1) performs the real bitstream decode; gate `1`
//!    (`flags` bit 0 = 0) emits zeroed overlap-add output independent
//!    of the input. Subsequent sub-packets in the same call are
//!    consumed through the carry buffer at context `+0x20`.
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
//! - [`Driver::decode_call_with_flags`] — the full-pipeline per-call
//!   entry point with the raw `RADecode` `flags` argument. Wires
//!   stages 1–5. The observe gate ([`DecodeGate::Observe`], `flags`
//!   bit 0 = 0) is **implemented**: per `validation/04` §4.3 the
//!   backend emits zeroed overlap-add output independent of the
//!   input, so the call zero-fills the per-call PCM budget and
//!   advances the cursor. The real-decode gate
//!   ([`DecodeGate::Decode`]) still surfaces the bitstream/transform
//!   backend as [`crate::Error::NotImplemented`].
//! - [`Driver::decode_call`] — the real-decode shorthand
//!   (`flags = `[`crate::RADECODE_FLAGS_DECODE`]); provided so the
//!   public decode path has a single orchestrated entry point even
//!   before the transform backend lands.
//! - [`DecodeGate`] — the typed `(~flags) & 1` decode/observe gate the
//!   driver forwards to the backend method.
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

/// The backend frame-decode gate derived from `RADecode`'s `flags`
/// argument.
///
/// The decode driver `cook.dll!0x1260` computes `(~flags) & 1` and
/// forwards it to the backend frame-decode method
/// `[backend_vtable + 0x0c]` as its decode/observe gate
/// (`docs/audio/cook/spec/01-cook-decoder-structure.md` §5,
/// `docs/audio/cook/validation/04-cook-stream-validation.md` §4.3):
///
/// - **`flags` bit 0 = 1** → forwarded gate bit `0` → [`DecodeGate::Decode`]:
///   the backend decodes the bitstream and the PCM depends on the
///   input bytes. A real decode of a fresh frame passes
///   [`crate::RADECODE_FLAGS_DECODE`]` = 1`.
/// - **`flags` bit 0 = 0** → forwarded gate bit `1` → [`DecodeGate::Observe`]:
///   the backend emits **zeroed overlap-add output independent of the
///   input** (the validator verified that all-`0xFF` input produces
///   the same zero output as the real packets on this path).
///
/// Only bit 0 of `flags` participates in the gate (`(~flags) & 1`
/// masks everything else away).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeGate {
    /// `flags` bit 0 = 1 — the backend performs the bitstream decode.
    Decode,
    /// `flags` bit 0 = 0 — the backend emits zeroed overlap-add output
    /// independent of the input bytes.
    Observe,
}

impl DecodeGate {
    /// Derive the gate from `RADecode`'s raw `flags` argument.
    ///
    /// Mirrors the driver's `(~flags) & 1` computation: bit 0 set →
    /// [`DecodeGate::Decode`], bit 0 clear → [`DecodeGate::Observe`].
    /// All other `flags` bits are ignored, exactly as the binary's
    /// `& 1` mask ignores them.
    pub const fn from_flags(flags: u32) -> Self {
        if flags & 1 == 1 {
            DecodeGate::Decode
        } else {
            DecodeGate::Observe
        }
    }

    /// The gate bit value the driver forwards to the backend method
    /// `[backend_vtable + 0x0c]` — literally `(~flags) & 1`.
    ///
    /// `0` for [`DecodeGate::Decode`], `1` for [`DecodeGate::Observe`]
    /// (validation/04 §4.3).
    pub const fn backend_gate_bit(self) -> u32 {
        match self {
            DecodeGate::Decode => 0,
            DecodeGate::Observe => 1,
        }
    }

    /// `true` when the backend would perform the real bitstream decode.
    pub const fn is_decode(self) -> bool {
        matches!(self, DecodeGate::Decode)
    }
}

/// Per-call decode driver — the orchestrator above the backend
/// frame-decode.
///
/// Holds a wired [`DecodeConfig`], the [`CommonMode`] toggle that gates
/// the per-buffer XOR descramble (default off — the validated
/// real-stream path), and an embedded [`CallSession`] tracking the
/// per-call cadence. Built by [`Driver::new`].
///
/// The driver drives the backend frame-decode through the
/// [`crate::frame`] orchestrator, which runs the statically-pinned
/// frame-body prefix (§1.1 gain count, §2.1 subband geometry) and stops
/// at the documented §3.2 BSS codebook blocker
/// ([`Error::SpectralCodebookBytesUnavailable`], docs-gap #1775); the
/// inverse-MDCT / coupling-coefficient stages past that blocker are
/// later-round work. [`Driver::prepare_call`] gives consumers the
/// orchestrated stage 1+2 output (descramble + sub-packet split)
/// directly.
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
    /// analog (spec/01 §5) on the real-decode gate.
    ///
    /// Equivalent to [`Driver::decode_call_with_flags`] with
    /// `flags = `[`crate::RADECODE_FLAGS_DECODE`] (= 1, the value a
    /// real decode of a fresh frame passes — validation/04 §4.3). On
    /// this gate the call drives each sub-packet through the
    /// [`crate::frame`] orchestrator, which runs the statically-pinned
    /// frame-body prefix and stops at the documented §3.2 BSS codebook
    /// blocker ([`Error::SpectralCodebookBytesUnavailable`], docs-gap
    /// #1775); see `decode_call_with_flags` for the full per-stage
    /// description and the implemented observe-gate path.
    ///
    /// # Errors
    ///
    /// - [`Error::CallInputLengthMismatch`] / [`Error::CallOutputLengthMismatch`]
    ///   if the buffer sizes disagree with the wired per-call budget.
    /// - [`Error::SpectralCodebookBytesUnavailable`] — the documented
    ///   §3.2 BSS blocker the frame-body walk reaches (docs-gap #1775).
    pub fn decode_call(
        &mut self,
        packet: &[u8],
        output: &mut [u8],
        xor_key: u32,
    ) -> Result<(), Error> {
        self.decode_call_with_flags(packet, output, xor_key, crate::RADECODE_FLAGS_DECODE)
    }

    /// Full-pipeline per-call decode entry point with the raw
    /// `RADecode` `flags` argument — the six-argument `RADecode`
    /// analog (spec/01 §2 / §5).
    ///
    /// Orchestrates stages 1–5:
    ///
    /// 1. Validates the input/output lengths against the wired
    ///    geometry + the validator-pinned per-call PCM budget.
    /// 2. Runs the XOR descramble when [`Driver::common_mode`] is on.
    /// 3. Computes the sub-packet split (validator pin: 5 × 93 = 465).
    /// 4. Invokes the backend frame-decode `[backend_vtable + 0x0c]`
    ///    with the gate bit `(~flags) & 1` ([`DecodeGate`]):
    ///    - **[`DecodeGate::Observe`]** (`flags` bit 0 = 0) — pinned by
    ///      `validation/04` §4.3: the backend emits **zeroed
    ///      overlap-add output independent of the input** (verified
    ///      against the real stream: all-`0xFF` input produces the
    ///      same zero output as the real packets). This path is
    ///      implemented: the output buffer is zero-filled to the
    ///      per-call PCM budget (16-bit PCM silence) and the call
    ///      completes.
    ///    - **[`DecodeGate::Decode`]** (`flags` bit 0 = 1) — the real
    ///      bitstream decode; drives each sub-packet through the
    ///      [`crate::frame`] orchestrator (§1.1 gain count + §2.1
    ///      subband geometry run), which stops at the documented §3.2
    ///      BSS codebook blocker
    ///      ([`Error::SpectralCodebookBytesUnavailable`], docs-gap
    ///      #1775) — the inverse-MDCT / coupling stages past it close in
    ///      a later dynamic-BSS-dump round.
    /// 5. Advances the session cursor on success. The decode-gate
    ///    blocker path returns before the cursor moves; no partial
    ///    state is published on failure.
    ///
    /// The per-call output sizing is gate-independent: the driver
    /// derives its buffer accounting from the wired geometry (spec/01
    /// §5 — `div [esi+8]`, carry/budget bookkeeping at context
    /// `+0x20`), and the gate is merely forwarded to the backend, so
    /// the observe gate walks the same warm-up / steady-state cadence
    /// the validator pinned (first call
    /// [`DecodeConfig::warmup_pcm_bytes`], then
    /// [`DecodeConfig::pcm_bytes_per_call`]).
    ///
    /// # Errors
    ///
    /// - [`Error::CallInputLengthMismatch`] / [`Error::CallOutputLengthMismatch`]
    ///   if the buffer sizes disagree with the wired per-call budget.
    /// - [`Error::SpectralCodebookBytesUnavailable`] from the backend
    ///   frame-body walk on the [`DecodeGate::Decode`] gate (docs-gap
    ///   #1775).
    pub fn decode_call_with_flags(
        &mut self,
        packet: &[u8],
        output: &mut [u8],
        xor_key: u32,
        flags: u32,
    ) -> Result<(), Error> {
        // Stage 1+2: validate, descramble, split.
        let prepared = self.prepare_call(packet, xor_key)?;
        // Pre-validate the output size before invoking the backend —
        // keeps the §3.2 BSS-blocker signal reserved for the frame-body
        // walk itself, distinct from a wrong-sized buffer.
        let expected_out = self.session.next_call_pcm_bytes() as usize;
        if output.len() != expected_out {
            return Err(Error::CallOutputLengthMismatch {
                got: output.len(),
                expected: expected_out,
            });
        }
        // Stage 4: backend frame-decode, gated on `(~flags) & 1`.
        match DecodeGate::from_flags(flags) {
            DecodeGate::Observe => {
                // validation/04 §4.3: zeroed overlap-add output,
                // independent of the input bytes.
                output.fill(0);
                // Stage 5: account for the completed call.
                self.session.advance_one_call(packet.len(), output.len())
            }
            // The real bitstream decode — drive the frame-body
            // orchestrator (spec/05 §0–§3), which runs the
            // statically-pinned prefix (§1.1 gain count, §2.1 subband
            // geometry) and stops precisely at the §3.2 BSS codebook
            // blocker (docs-gap #1775,
            // `Error::SpectralCodebookBytesUnavailable`). Reserving that
            // signal for the documented blocker keeps it distinct from a
            // size mismatch and from the legacy `NotImplemented`.
            //
            // spec/01 §5 pins that the backend frame-decode method is
            // invoked exactly ONCE per call, with subsequent sub-packets
            // consumed through the carry buffer at context `+0x20`; the
            // §0 frame body therefore provably starts at the head of the
            // call's (descrambled) input. Where the remaining
            // `sub_packets_per_call − 1` frame bitstreams sit inside the
            // call — the carry-buffer mechanics — is NOT pinned, and the
            // validated stream shows the 93-byte slot boundaries after
            // slot 0 are not independent frame heads (packet 0's slot 1
            // opens with a §1.1 field of raw 4 < 6, an invalid frame
            // head, while slot 0 opens with the well-formed raw 29).
            // Only the head frame's pinned prefix is walked.
            DecodeGate::Decode => {
                crate::frame::decode_frame_body(
                    prepared.descrambled(),
                    self.config.channels,
                    u32::from(self.config.subband_count),
                )?;
                // Unreachable on a non-trivial stream: `decode_frame_body`
                // returns the BSS blocker for every coded frame. The
                // explicit terminator keeps the match total if a future
                // dynamic-BSS-dump round unblocks the walk.
                Err(Error::SpectralCodebookBytesUnavailable)
            }
        }
    }
}

impl Driver {
    /// Resume-from-blocker per-call decode — the full `RADecode` shape
    /// with the §3.2 entropy step supplied by the caller.
    ///
    /// Runs the real per-call orchestration end-to-end:
    ///
    /// 1. Stage 1+2 ([`Driver::prepare_call`]): input-length validation,
    ///    optional XOR descramble, sub-packet split.
    /// 2. Output-budget validation (the validator-pinned per-call PCM
    ///    budget — warm-up on call 0, steady-state thereafter).
    /// 3. The §5 synthesis of the caller-supplied post-entropy spectra
    ///    (`frames`, one [`crate::frame::FrameSpectrum`] per
    ///    sub-packet — the §3.2 GAP-sourced input a future
    ///    dynamic-BSS-dump round will produce in place of the caller),
    ///    through the [`crate::backend::SynthesisBackend`] into
    ///    `output`.
    /// 4. Session accounting ([`crate::session::CallSession`] cursor
    ///    advance).
    ///
    /// This path deliberately does **not** re-run the frame-body
    /// bitstream walk: the supplied spectra *are* the post-entropy
    /// product, so the entropy-side bitstream consumption they replace
    /// (the walk [`Driver::decode_call`] drives up to the §3.2 blocker)
    /// is theirs to account for. Moreover the §1.1 field reading is
    /// contradicted on real data — 12 of the validated stream's 144
    /// call heads carry a leading 6-bit field `< 6`, which biases
    /// negative under the `spec/05` §1.1 *"field = segment_count + 6"*
    /// reading (a recorded docs-gap; pinned by
    /// `tests/synthesis_realstream.rs`) — so no real-bitstream §1.1
    /// assertion is made on this path.
    ///
    /// The §1 gain profiles are entropy-gated (the per-segment records
    /// descend the §3.2 BSS VLC), so this entry point synthesizes with
    /// the flat unity envelope; callers needing explicit profiles can
    /// drive the backend directly with
    /// [`crate::backend::SynthesisBackend::push_frame_with_gain`].
    ///
    /// On any error the session cursor does not advance; the backend's
    /// overlap/carry state may have consumed a prefix of `frames`
    /// (reset it with [`crate::backend::SynthesisBackend::reset`]
    /// before retrying a failed call).
    ///
    /// # Errors
    ///
    /// - [`Error::CallInputLengthMismatch`] /
    ///   [`Error::CallOutputLengthMismatch`] on buffer-size disagreement.
    /// - [`Error::FrameSpectrumCountMismatch`] when `frames` does not
    ///   carry one spectrum per sub-packet.
    /// - Any synthesis/backend error (channel routing, spectrum width,
    ///   PCM assembly underrun).
    pub fn synthesized_call(
        &mut self,
        packet: &[u8],
        output: &mut [u8],
        xor_key: u32,
        backend: &mut crate::backend::SynthesisBackend,
        frames: &[crate::frame::FrameSpectrum],
    ) -> Result<(), Error> {
        // Stage 1+2: validate, descramble, split.
        let prepared = self.prepare_call(packet, xor_key)?;
        // Output budget validation before any state is touched.
        let expected_out = self.session.next_call_pcm_bytes() as usize;
        if output.len() != expected_out {
            return Err(Error::CallOutputLengthMismatch {
                got: output.len(),
                expected: expected_out,
            });
        }
        let spc = prepared.sub_packets_per_call() as usize;
        if frames.len() != spc {
            return Err(Error::FrameSpectrumCountMismatch {
                got: frames.len(),
                expected: spc,
            });
        }
        // Stage 3: §5 synthesis of the caller-supplied post-entropy
        // spectra, assembled into this call's PCM budget. (No bitstream
        // walk on this path — the spectra replace the entropy stage;
        // see the method docs.)
        for spectrum in frames {
            backend.push_frame(spectrum)?;
        }
        backend.fill_call(output)?;
        // Stage 5: account for the completed call.
        self.session.advance_one_call(packet.len(), output.len())
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

    /// A 465-byte packet whose first sub-packet (93 bytes) carries a
    /// well-formed §1.1 gain header (top 6 bits = `001000` = 8 → 2
    /// segments), so the frame-body walk passes the gain stage and
    /// reaches the §3.2 BSS blocker rather than the underflow guard.
    fn packet_with_valid_gain_header() -> Vec<u8> {
        let mut p = vec![0u8; 465];
        p[0] = 0b0010_0000; // top 6 bits of sub-packet 0 = 8.
        p
    }

    #[test]
    fn decode_call_validates_sizing_before_signalling_backend_gap() {
        // `decode_call` validates input + output sizes before the
        // §3.2 BSS-blocker signal: a wrong-sized buffer should produce a
        // typed length error, not the backend blocker.
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
    fn decode_call_surfaces_bss_blocker_on_wired_sizes() {
        // With both buffer sizes wired correctly and a well-formed gain
        // header, `decode_call` drives the frame-body walk to the
        // documented §3.2 BSS codebook blocker (docs-gap #1775).
        let mut d = real_driver();
        let packet = packet_with_valid_gain_header();
        let mut out = vec![0u8; 8_192];
        let err = d.decode_call(&packet, &mut out, 0).unwrap_err();
        assert_eq!(err, Error::SpectralCodebookBytesUnavailable);
        // The blocker path does NOT advance the session cursor (no
        // partial state on the GAP signal).
        assert_eq!(d.calls_completed(), 0);
        assert_eq!(d.total_pcm_emitted(), 0);
    }

    #[test]
    fn decode_gate_mirrors_inverted_bit_zero() {
        // validation/04 §4.3: the driver forwards `(~flags) & 1`. Only
        // bit 0 participates; all other bits are masked away.
        assert_eq!(DecodeGate::from_flags(1), DecodeGate::Decode);
        assert_eq!(DecodeGate::from_flags(0), DecodeGate::Observe);
        assert_eq!(DecodeGate::from_flags(3), DecodeGate::Decode);
        assert_eq!(DecodeGate::from_flags(2), DecodeGate::Observe);
        assert_eq!(DecodeGate::from_flags(u32::MAX), DecodeGate::Decode);
        assert_eq!(DecodeGate::from_flags(u32::MAX - 1), DecodeGate::Observe);
        // The forwarded gate bit is literally (~flags) & 1.
        for flags in [0u32, 1, 2, 3, 0xFFFF_FFF0, u32::MAX] {
            assert_eq!(
                DecodeGate::from_flags(flags).backend_gate_bit(),
                (!flags) & 1,
                "gate bit for flags {flags:#x}"
            );
        }
        assert!(DecodeGate::Decode.is_decode());
        assert!(!DecodeGate::Observe.is_decode());
        // The decode shorthand constant maps to the decode gate.
        assert_eq!(
            DecodeGate::from_flags(crate::RADECODE_FLAGS_DECODE),
            DecodeGate::Decode
        );
    }

    #[test]
    fn observe_gate_zero_fills_and_advances() {
        // validation/04 §4.3: flags bit 0 = 0 → the backend emits
        // zeroed overlap-add output independent of the input, and the
        // call completes (S_OK at the SPI level → Ok here).
        let mut d = real_driver();
        let packet: Vec<u8> = (0..465u32).map(|i| (i & 0xff) as u8).collect();
        let mut out = vec![0xAAu8; 8_192];
        d.decode_call_with_flags(&packet, &mut out, 0, 0).unwrap();
        assert!(out.iter().all(|&b| b == 0), "observe output is zeroed");
        // The cursor advances — the call completed.
        assert_eq!(d.calls_completed(), 1);
        assert_eq!(d.total_pcm_emitted(), 8_192);
        // Steady-state call next: budget moves to 20 480.
        let mut out2 = vec![0x55u8; 20_480];
        d.decode_call_with_flags(&packet, &mut out2, 0, 0).unwrap();
        assert!(out2.iter().all(|&b| b == 0));
        assert_eq!(d.calls_completed(), 2);
        assert_eq!(d.total_pcm_emitted(), 8_192 + 20_480);
    }

    #[test]
    fn observe_gate_output_is_input_independent() {
        // validation/04 §4.3 verification: all-0xFF input gives the
        // same zero output as real packet bytes on the observe gate.
        let mut d1 = real_driver();
        let mut d2 = real_driver();
        let varied: Vec<u8> = (0..465u32)
            .map(|i| (i.wrapping_mul(31) & 0xff) as u8)
            .collect();
        let all_ff = vec![0xFFu8; 465];
        let mut out1 = vec![0x11u8; 8_192];
        let mut out2 = vec![0x22u8; 8_192];
        d1.decode_call_with_flags(&varied, &mut out1, 0, 0).unwrap();
        d2.decode_call_with_flags(&all_ff, &mut out2, 0, 0).unwrap();
        assert_eq!(out1, out2, "observe output independent of input");
    }

    #[test]
    fn observe_gate_still_validates_buffer_sizes() {
        // Length validation precedes the gate dispatch: wrong-sized
        // buffers produce the typed mismatch on either gate, and the
        // cursor does not move.
        let mut d = real_driver();
        let short = vec![0u8; 464];
        let mut out = vec![0u8; 8_192];
        let err = d
            .decode_call_with_flags(&short, &mut out, 0, 0)
            .unwrap_err();
        assert!(matches!(err, Error::CallInputLengthMismatch { .. }));

        let packet = vec![0u8; 465];
        let mut wrong_out = vec![0u8; 20_480]; // first call expects 8 192
        let err = d
            .decode_call_with_flags(&packet, &mut wrong_out, 0, 0)
            .unwrap_err();
        assert_eq!(
            err,
            Error::CallOutputLengthMismatch {
                got: 20_480,
                expected: 8_192
            }
        );
        assert_eq!(d.calls_completed(), 0);
        assert_eq!(d.total_pcm_emitted(), 0);
    }

    #[test]
    fn decode_gate_via_flags_surfaces_bss_blocker() {
        // decode_call_with_flags on the real-decode gate behaves exactly
        // like decode_call: it drives the frame-body walk to the §3.2
        // BSS blocker, with no cursor motion.
        let mut d = real_driver();
        let packet = packet_with_valid_gain_header();
        let mut out = vec![0u8; 8_192];
        let err = d
            .decode_call_with_flags(&packet, &mut out, 0, crate::RADECODE_FLAGS_DECODE)
            .unwrap_err();
        assert_eq!(err, Error::SpectralCodebookBytesUnavailable);
        assert_eq!(d.calls_completed(), 0);
        assert_eq!(d.total_pcm_emitted(), 0);
    }

    #[test]
    fn decode_gate_surfaces_gain_underflow_before_blocker() {
        // An all-zero first sub-packet biases the §1.1 gain segment-count
        // negative — that stage-1 error fires before the §3 BSS blocker,
        // proving the walk runs the pinned prefix in order.
        let mut d = real_driver();
        let packet = vec![0u8; 465];
        let mut out = vec![0u8; 8_192];
        let err = d.decode_call(&packet, &mut out, 0).unwrap_err();
        assert!(matches!(err, Error::GainSegmentCountUnderflow { raw: 0 }));
        assert_eq!(d.calls_completed(), 0);
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
