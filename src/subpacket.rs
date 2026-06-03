//! Per-`RADecode` sub-packet split + PCM offset accounting.
//!
//! Source-of-truth: `docs/audio/cook/spec/01-cook-decoder-structure.md` §5
//! (the `RADecode` driver `0x1260` body: per-frame divisor `div [esi+8]`,
//! sub-packet iteration, carry accumulation at context `+0x20`,
//! `memmove`-based leftover handling) and
//! `docs/audio/cook/validation/04-cook-stream-validation.md` §5 (the 144
//! `RADecode` calls × 5 sub-packets × 465-byte input partitioning, the
//! `8 192`-byte first-call overlap-add warm-up, and the steady-state
//! `20 480`-byte per-call output).
//!
//! ## What the binary does
//!
//! `RADecode` (`0x1260`) is **not** the transform itself — it is the
//! per-call driver above the backend. After the optional XOR descramble
//! (`crate::descramble`), it
//!
//! 1. Reads the per-call input length and divides it by the per-frame
//!    unit size held in the decode state (`div dword ptr [esi+8]`, where
//!    `[esi+8]` is the descriptor `+0x0a` value
//!    [`crate::Descriptor::sub_packet_size`]). The quotient is the
//!    number of sub-packets in this call (`sub_packets_per_call`); the
//!    remainder is a hard rejection (spec/01 §5; this crate enforces it
//!    at [`DecodeConfig::from_inputs`] time so the driver can rely on
//!    the invariant).
//! 2. Walks the call's input as `sub_packets_per_call` consecutive
//!    fixed-stride sub-packet slots of `sub_packet_size` bytes each.
//!    Slot `k` occupies bytes `[k × sub_packet_size,
//!    (k + 1) × sub_packet_size)` of the call's input.
//! 3. Invokes the backend's frame-decode method
//!    `[backend_vtable + 0x0c]` exactly **once per `RADecode` call**
//!    (spec/01 §5 audit + validation/04 §4.3); subsequent sub-packets in
//!    the same call take the **carry path** through the de-interleave
//!    buffer at context `+0x20`, which `memmove` retains between calls.
//!    The validator confirmed that this is exactly how the container
//!    packets are consumed: 144 calls × 5 sub-packets × 465 input bytes
//!    are fed straight into the carry buffer, with the backend invoked
//!    once per call (validation/04 §5).
//!
//! ## What this module provides
//!
//! Pure-Rust modelling of stages 1–2 (structural sub-packet split + slot
//! addressing) and the validator-pinned PCM offset accounting that comes
//! out of them:
//!
//! - [`SubPacketLayout`] — derived from a [`DecodeConfig`], giving the
//!   per-call sub-packet count, the per-call sub-packet stride
//!   (= `sub_packet_size`), and the steady-state PCM budget per call.
//!   Also exposes the warm-up vs steady-state asymmetry the validator
//!   measured (validation/04 §5).
//! - [`SubPacketLayout::slot_byte_range(slot_in_call)`] — the byte range
//!   inside one `RADecode` call's input that holds sub-packet `slot`.
//! - [`SubPacketLayout::call_byte_range(call_idx, slot_in_call)`] — the
//!   byte range inside the **whole-stream** input (all `RADecode` calls
//!   concatenated, in container order) that holds the `slot`-th sub-packet
//!   of the `call_idx`-th call. Equivalent to
//!   `call_idx × frame_bytes + slot × sub_packet_size .. + sub_packet_size`.
//! - [`SubPacketLayout::iter_call(input)`] — iterator over the
//!   `sub_packets_per_call` slot slices for one call's input
//!   (`input.len() == frame_bytes`).
//! - [`SubPacketLayout::pcm_offset_for_call(call_idx)`] — the validator-
//!   pinned PCM byte offset at which call `call_idx`'s output starts
//!   inside the concatenated stream PCM (first call emits
//!   [`DecodeConfig::warmup_pcm_bytes`], every subsequent call emits
//!   [`DecodeConfig::pcm_bytes_per_call`]).
//! - [`Error::SlotOutOfRange`] /
//!   [`Error::SubPacketInputLengthMismatch`] for misuse rejections.
//!
//! Stage 3 (the backend frame-decode + carry-buffer state machine) is a
//! `crate::Error::NotImplemented` GAP — modelling it requires the
//! backend's bitstream reader, which lands in a later round.

use crate::{init::DecodeConfig, Error};

/// Derived per-call sub-packet split for a wired [`DecodeConfig`].
///
/// Built by [`SubPacketLayout::from_config`]. Captures the structural
/// invariants the `RADecode` driver `0x1260` enforces every call:
/// `sub_packets_per_call = frame_bytes / sub_packet_size` with zero
/// remainder, every sub-packet slot is `sub_packet_size` bytes,
/// steady-state PCM is `sub_packets_per_call × samples_per_frame ×
/// channels × 2`, and the first call emits two transform-frames of
/// overlap-add warm-up before the steady-state cadence (validation/04
/// §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubPacketLayout {
    /// Per-stream coded frame size in bytes (one `RADecode`-call's
    /// input length).
    pub frame_bytes: u32,
    /// Per-stream sub-packet size in bytes (descriptor `+0x0a`).
    pub sub_packet_size: u16,
    /// Sub-packets one `RADecode` call partitions its input into.
    pub sub_packets_per_call: u32,
    /// Steady-state PCM byte budget per `RADecode` call.
    pub pcm_bytes_per_call: u32,
    /// First-call PCM budget (the two-transform-frame overlap-add
    /// warm-up). Pinned at `8 192` on the validated stream
    /// (validation/04 §5).
    pub warmup_pcm_bytes: u32,
}

impl SubPacketLayout {
    /// Derive the layout from a wired [`DecodeConfig`].
    ///
    /// Cannot fail: [`DecodeConfig::from_inputs`] already enforces
    /// `frame_bytes % sub_packet_size == 0` and the non-zero divisor
    /// invariants on which this layout depends.
    pub fn from_config(cfg: &DecodeConfig) -> Self {
        // `warmup` is a u32 by construction: 2 × samples_per_frame ×
        // channels × PCM_BYTES_PER_SAMPLE; on every well-formed Cook
        // stream this fits in u32 (max 2 × 1024 × 2 × 2 = 8 192).
        let warmup_u64 = cfg.warmup_pcm_bytes();
        debug_assert!(
            warmup_u64 <= u32::MAX as u64,
            "warmup_pcm_bytes overflow — Cook geometry guarantees ≤ 8192"
        );
        SubPacketLayout {
            frame_bytes: cfg.frame_bytes,
            sub_packet_size: cfg.sub_packet_size,
            sub_packets_per_call: cfg.sub_packets_per_call,
            pcm_bytes_per_call: cfg.pcm_bytes_per_call,
            warmup_pcm_bytes: warmup_u64 as u32,
        }
    }

    /// Byte range inside one `RADecode` call's input that holds
    /// sub-packet `slot_in_call`.
    ///
    /// Returns `[slot × sub_packet_size, (slot + 1) × sub_packet_size)`.
    ///
    /// # Errors
    ///
    /// - [`Error::SlotOutOfRange`] if `slot_in_call >=
    ///   sub_packets_per_call`.
    pub fn slot_byte_range(&self, slot_in_call: u32) -> Result<core::ops::Range<u32>, Error> {
        if slot_in_call >= self.sub_packets_per_call {
            return Err(Error::SlotOutOfRange {
                slot: slot_in_call,
                slots_per_call: self.sub_packets_per_call,
            });
        }
        let start = slot_in_call * self.sub_packet_size as u32;
        let end = start + self.sub_packet_size as u32;
        Ok(start..end)
    }

    /// Byte range inside the **whole-stream** input (all `RADecode` call
    /// inputs concatenated in container order) that holds sub-packet
    /// `slot_in_call` of call `call_idx`.
    ///
    /// Equivalent to
    /// `call_idx × frame_bytes + slot_in_call × sub_packet_size ..
    ///  + sub_packet_size`.
    ///
    /// `call_idx` is `u32`; the stream offset arithmetic uses `u64` so
    /// long streams cannot wrap (a 16.6 s validated stream is 67 KB of
    /// coded input; 4-byte-saturation is two-orders-of-magnitude beyond
    /// realistic Cook stream lengths).
    ///
    /// # Errors
    ///
    /// - [`Error::SlotOutOfRange`] if `slot_in_call >=
    ///   sub_packets_per_call`.
    pub fn call_byte_range(
        &self,
        call_idx: u32,
        slot_in_call: u32,
    ) -> Result<core::ops::Range<u64>, Error> {
        let slot = self.slot_byte_range(slot_in_call)?;
        let call_base = call_idx as u64 * self.frame_bytes as u64;
        Ok(call_base + slot.start as u64..call_base + slot.end as u64)
    }

    /// Iterate the `sub_packets_per_call` slot slices of one call's
    /// input.
    ///
    /// `input.len()` must equal [`Self::frame_bytes`]; otherwise an
    /// [`Error::SubPacketInputLengthMismatch`] is returned by the
    /// iterator's *first* call (the iterator yields the error once and
    /// then terminates).
    ///
    /// On success the iterator yields exactly [`Self::sub_packets_per_call`]
    /// slot slices of [`Self::sub_packet_size`] bytes each, in slot order
    /// `0 .. sub_packets_per_call`.
    pub fn iter_call<'a>(
        &self,
        input: &'a [u8],
    ) -> impl Iterator<Item = Result<&'a [u8], Error>> + 'a {
        let frame_bytes = self.frame_bytes as usize;
        let sub_packet_size = self.sub_packet_size as usize;
        let sub_packets_per_call = self.sub_packets_per_call as usize;
        SubPacketIter {
            input,
            frame_bytes,
            sub_packet_size,
            sub_packets_per_call,
            next_slot: 0,
            length_checked: false,
        }
    }

    /// PCM byte offset at which call `call_idx`'s output starts inside
    /// the concatenated stream PCM.
    ///
    /// Validator-pinned (validation/04 §5):
    /// - call 0 starts at offset 0 (and emits [`Self::warmup_pcm_bytes`]
    ///   `= 8 192` on the validated stream),
    /// - call `k` for `k >= 1` starts at
    ///   `warmup_pcm_bytes + (k - 1) × pcm_bytes_per_call`.
    ///
    /// Returns `u64` so a long stream can never wrap; a typical stereo
    /// 44 100 Hz Cook stream emits ~176 KB/s of PCM, so even hour-long
    /// streams stay well inside `u64`.
    pub fn pcm_offset_for_call(&self, call_idx: u32) -> u64 {
        if call_idx == 0 {
            0
        } else {
            self.warmup_pcm_bytes as u64 + (call_idx as u64 - 1) * self.pcm_bytes_per_call as u64
        }
    }

    /// Total PCM bytes emitted across `calls` consecutive `RADecode`
    /// calls (the call after `pcm_offset_for_call(calls - 1)`'s payload).
    ///
    /// Equivalent to `DecodeConfig::total_pcm_bytes(calls)` and pinned
    /// against the validator's `2 936 832` bytes / 144-call figure
    /// (validation/04 §5). Returns `0` for `calls == 0`.
    pub fn total_pcm_bytes(&self, calls: u32) -> u64 {
        if calls == 0 {
            return 0;
        }
        self.warmup_pcm_bytes as u64 + (calls as u64 - 1) * self.pcm_bytes_per_call as u64
    }
}

/// Iterator yielded by [`SubPacketLayout::iter_call`].
struct SubPacketIter<'a> {
    input: &'a [u8],
    frame_bytes: usize,
    sub_packet_size: usize,
    sub_packets_per_call: usize,
    next_slot: usize,
    length_checked: bool,
}

impl<'a> Iterator for SubPacketIter<'a> {
    type Item = Result<&'a [u8], Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.length_checked {
            self.length_checked = true;
            if self.input.len() != self.frame_bytes {
                // Surface the mismatch once, then terminate.
                self.next_slot = self.sub_packets_per_call;
                return Some(Err(Error::SubPacketInputLengthMismatch {
                    got: self.input.len(),
                    expected: self.frame_bytes,
                }));
            }
        }
        if self.next_slot >= self.sub_packets_per_call {
            return None;
        }
        let start = self.next_slot * self.sub_packet_size;
        let end = start + self.sub_packet_size;
        self.next_slot += 1;
        Some(Ok(&self.input[start..end]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cookie::CookCookie, flavor::flavor_record, init::DecodeConfig, init::Descriptor,
        init::PCM_BYTES_PER_SAMPLE,
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

    fn real_layout() -> SubPacketLayout {
        let cookie = CookCookie::parse(&REAL_COOKIE).unwrap();
        let flavor = flavor_record(21).unwrap();
        let cfg = DecodeConfig::from_inputs(&cookie, &REAL_DESCRIPTOR, &flavor, REAL_FRAME_BYTES)
            .unwrap();
        SubPacketLayout::from_config(&cfg)
    }

    #[test]
    fn from_config_matches_decode_config() {
        let layout = real_layout();
        // 465 / 93 = 5 sub-packets per call (validation/04 §5).
        assert_eq!(layout.sub_packets_per_call, 5);
        assert_eq!(layout.sub_packet_size, 93);
        assert_eq!(layout.frame_bytes, 465);
        // 5 × 1024 × 2 × 2 = 20 480 (validation/04 §5).
        assert_eq!(layout.pcm_bytes_per_call, 20_480);
        // 2 × 1024 × 2 × 2 = 8 192 (validation/04 §5).
        assert_eq!(layout.warmup_pcm_bytes, 8_192);
    }

    #[test]
    fn slot_byte_range_covers_every_byte() {
        let layout = real_layout();
        // Slots 0..4: [0,93), [93,186), [186,279), [279,372), [372,465).
        for slot in 0..layout.sub_packets_per_call {
            let r = layout.slot_byte_range(slot).unwrap();
            assert_eq!(r.start, slot * 93);
            assert_eq!(r.end, (slot + 1) * 93);
        }
        // Slots tile the frame with no gap or overlap.
        let mut covered = 0u32;
        for slot in 0..layout.sub_packets_per_call {
            let r = layout.slot_byte_range(slot).unwrap();
            assert_eq!(r.start, covered, "slot {slot} starts after previous ends");
            covered = r.end;
        }
        assert_eq!(covered, layout.frame_bytes);
    }

    #[test]
    fn slot_out_of_range_rejected() {
        let layout = real_layout();
        let err = layout.slot_byte_range(5).unwrap_err();
        assert_eq!(
            err,
            Error::SlotOutOfRange {
                slot: 5,
                slots_per_call: 5
            }
        );
        // u32 boundary still rejects cleanly.
        let err = layout.slot_byte_range(u32::MAX).unwrap_err();
        assert!(matches!(err, Error::SlotOutOfRange { slot, .. } if slot == u32::MAX));
    }

    #[test]
    fn call_byte_range_strides_by_frame_bytes() {
        let layout = real_layout();
        // Call 0, slot 0 == slot range.
        let r0_0 = layout.call_byte_range(0, 0).unwrap();
        assert_eq!(r0_0, 0u64..93);
        // Call 1, slot 0 starts at frame_bytes = 465.
        let r1_0 = layout.call_byte_range(1, 0).unwrap();
        assert_eq!(r1_0, 465u64..465 + 93);
        // Call 143 (last), slot 4 (last) ends at 144 × 465 = 66 960
        // (the validator's 144 packets × 465 bytes).
        let r_last = layout.call_byte_range(143, 4).unwrap();
        assert_eq!(r_last.end, 144u64 * 465);
        // Slot-out-of-range error surfaces from the same path.
        let err = layout.call_byte_range(0, 5).unwrap_err();
        assert!(matches!(err, Error::SlotOutOfRange { .. }));
    }

    #[test]
    fn iter_call_yields_5_slots_each_93_bytes() {
        let layout = real_layout();
        // Build a synthetic 465-byte call input where each byte is its
        // offset, so we can pin the slot contents byte-exactly.
        let input: Vec<u8> = (0..465u32).map(|i| (i & 0xff) as u8).collect();
        let slots: Vec<_> = layout
            .iter_call(&input)
            .map(|r| r.unwrap().to_vec())
            .collect();
        assert_eq!(slots.len(), 5);
        for (k, slot) in slots.iter().enumerate() {
            assert_eq!(slot.len(), 93, "slot {k} is 93 bytes");
            assert_eq!(
                slot[0],
                ((k * 93) & 0xff) as u8,
                "slot {k} first byte is offset {} mod 256",
                k * 93
            );
        }
        // Concatenation equals the original input.
        let recombined: Vec<u8> = slots.into_iter().flatten().collect();
        assert_eq!(recombined, input);
    }

    #[test]
    fn iter_call_rejects_wrong_length() {
        let layout = real_layout();
        let short = vec![0u8; 464];
        let mut it = layout.iter_call(&short);
        match it.next() {
            Some(Err(Error::SubPacketInputLengthMismatch { got, expected })) => {
                assert_eq!(got, 464);
                assert_eq!(expected, 465);
            }
            other => panic!("expected length-mismatch error, got {other:?}"),
        }
        // Iterator terminates after surfacing the error.
        assert!(it.next().is_none());
    }

    #[test]
    fn pcm_offset_for_call_matches_validator() {
        // Validator §5: 144 calls → 2 936 832 bytes.
        // call 0  → offset 0 (emits 8 192 warm-up).
        // call 1  → offset 8 192.
        // call 2  → offset 8 192 + 20 480 = 28 672.
        // call k≥1 → 8 192 + (k - 1) × 20 480.
        let layout = real_layout();
        assert_eq!(layout.pcm_offset_for_call(0), 0);
        assert_eq!(layout.pcm_offset_for_call(1), 8_192);
        assert_eq!(layout.pcm_offset_for_call(2), 8_192 + 20_480);
        assert_eq!(layout.pcm_offset_for_call(143), 8_192 + 142 * 20_480);
        // The offset where call 144 *would* start (one past the last
        // real call) is the validator's total PCM byte count.
        assert_eq!(layout.pcm_offset_for_call(144), 2_936_832);
        assert_eq!(layout.total_pcm_bytes(144), 2_936_832);
        assert_eq!(layout.total_pcm_bytes(0), 0);
        assert_eq!(layout.total_pcm_bytes(1), 8_192);
    }

    #[test]
    fn pcm_offsets_match_decode_config_total() {
        // For every k ∈ [0, 144], pcm_offset_for_call(k) is the byte
        // count accumulated by k completed RADecode calls — the same
        // value DecodeConfig::total_pcm_bytes(k) returns.
        let cookie = CookCookie::parse(&REAL_COOKIE).unwrap();
        let flavor = flavor_record(21).unwrap();
        let cfg = DecodeConfig::from_inputs(&cookie, &REAL_DESCRIPTOR, &flavor, REAL_FRAME_BYTES)
            .unwrap();
        let layout = SubPacketLayout::from_config(&cfg);
        for k in 0..=144u32 {
            assert_eq!(layout.pcm_offset_for_call(k), cfg.total_pcm_bytes(k));
        }
    }

    #[test]
    fn pcm_offset_arithmetic_uses_validated_constants() {
        // The arithmetic this module performs is exactly the validator's:
        // warmup = 2 × spf × ch × 2 and pcm/call = N × spf × ch × 2.
        // Re-derive both from the wired DecodeConfig to confirm there is
        // no hidden hard-coded value sneaking in.
        let layout = real_layout();
        let spf = 1024u32;
        let ch = 2u32;
        let pcm_bps = PCM_BYTES_PER_SAMPLE; // 2
        assert_eq!(layout.warmup_pcm_bytes, 2 * spf * ch * pcm_bps);
        assert_eq!(
            layout.pcm_bytes_per_call,
            layout.sub_packets_per_call * spf * ch * pcm_bps
        );
    }
}
