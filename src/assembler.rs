//! Frame → per-call PCM assembly — the carry-buffer cadence between the
//! per-frame synthesis output and the per-call `RADecode` PCM budget.
//!
//! Source-of-truth:
//! `docs/audio/cook/validation/04-cook-stream-validation.md` §5 (the
//! validated cadence: every call consumes `sub_packets_per_call` coded
//! frames, the **first** call emits the `warmup` budget — two frames'
//! worth on the validated stream — and every subsequent call emits the
//! steady-state `sub_packets_per_call`-frame budget, accumulating to the
//! pinned `2 936 832` bytes at call 144) and
//! `docs/audio/cook/spec/01-cook-decoder-structure.md` §5 (the decode
//! driver's carry-buffer accounting at context `+0x20`).
//!
//! ## The pinned arithmetic
//!
//! Per call, `sub_packets_per_call` frames of
//! `samples_per_frame × channels × 2` PCM bytes enter and the
//! validator-pinned per-call budget leaves. Because the first call
//! emits less than it consumes (`warmup < pcm_bytes_per_call`), a
//! constant backlog of `pcm_bytes_per_call − warmup` bytes (three
//! frames on the validated stream) rides in the carry buffer from call
//! 0 onward — exactly the driver's `+0x20` carry accounting. The
//! [`CallPcmAssembler`] is that queue: [`CallPcmAssembler::push_frame_pcm`]
//! enqueues one synthesized frame's PCM,
//! [`CallPcmAssembler::fill_call`] dequeues one call's budget.
//!
//! ## What is a model choice (recorded)
//!
//! The queue is **FIFO** — the arithmetic-consistent model in which
//! call `k`'s output is the oldest not-yet-emitted synthesized PCM.
//! The trace pins the byte *accounting* (sizes and totals), not which
//! physical frame lands in which call's output; the FIFO mapping is the
//! natural overlap-add pipeline reading and is documented here rather
//! than silently assumed.

use std::collections::VecDeque;

use crate::{init::DecodeConfig, Error};

/// FIFO carry buffer between per-frame synthesis PCM and the per-call
/// `RADecode` output budget (spec/01 §5 `+0x20` carry accounting,
/// validation/04 §5 cadence).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallPcmAssembler {
    frame_pcm_bytes: u32,
    queue: VecDeque<u8>,
}

impl CallPcmAssembler {
    /// Build the assembler for a wired [`DecodeConfig`].
    ///
    /// The per-frame PCM size derives from the config's own accounting:
    /// `pcm_bytes_per_call / sub_packets_per_call` (=
    /// `samples_per_frame × channels × 2`; `4 096` bytes on the
    /// validated stream).
    ///
    /// # Errors
    ///
    /// Returns [`Error::FrameNotDivisibleBySubPacket`] when the config
    /// carries zero sub-packets per call (a `frame_bytes = 0` config —
    /// no frame geometry to assemble).
    pub fn from_config(config: &DecodeConfig) -> Result<Self, Error> {
        if config.sub_packets_per_call == 0 {
            return Err(Error::FrameNotDivisibleBySubPacket {
                frame_bytes: config.frame_bytes,
                sub_packet_size: config.sub_packet_size,
            });
        }
        Ok(CallPcmAssembler {
            frame_pcm_bytes: config.pcm_bytes_per_call / config.sub_packets_per_call,
            queue: VecDeque::new(),
        })
    }

    /// PCM bytes one synthesized frame contributes
    /// (`samples_per_frame × channels × 2`).
    pub fn frame_pcm_bytes(&self) -> u32 {
        self.frame_pcm_bytes
    }

    /// Bytes currently riding in the carry buffer.
    pub fn buffered(&self) -> usize {
        self.queue.len()
    }

    /// Enqueue one synthesized frame's PCM bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::FramePcmLengthMismatch`] when `frame` is not
    /// exactly [`CallPcmAssembler::frame_pcm_bytes`] long.
    pub fn push_frame_pcm(&mut self, frame: &[u8]) -> Result<(), Error> {
        if frame.len() != self.frame_pcm_bytes as usize {
            return Err(Error::FramePcmLengthMismatch {
                got: frame.len(),
                expected: self.frame_pcm_bytes as usize,
            });
        }
        self.queue.extend(frame.iter().copied());
        Ok(())
    }

    /// Dequeue one call's PCM budget into `out` (size the buffer with
    /// [`crate::session::CallSession::next_call_pcm_bytes`] /
    /// [`crate::driver::Driver::next_call_pcm_bytes`] — warm-up on call
    /// 0, steady-state thereafter).
    ///
    /// # Errors
    ///
    /// Returns [`Error::PcmAssemblerUnderrun`] (and leaves the queue
    /// untouched) when fewer bytes are buffered than `out` requires —
    /// the caller has not pushed the call's frames yet.
    pub fn fill_call(&mut self, out: &mut [u8]) -> Result<(), Error> {
        if self.queue.len() < out.len() {
            return Err(Error::PcmAssemblerUnderrun {
                need: out.len(),
                have: self.queue.len(),
            });
        }
        for b in out.iter_mut() {
            *b = self.queue.pop_front().expect("length checked above");
        }
        Ok(())
    }

    /// Drop any buffered bytes (stream reset).
    pub fn reset(&mut self) {
        self.queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cookie::CookCookie,
        flavor::flavor_record,
        init::{DecodeConfig, Descriptor},
    };

    // Validated stream pins (validation/04 §2.1 / §4 / §5).
    const REAL_COOKIE: [u8; 16] = [
        0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00,
        0x04,
    ];

    fn real_config() -> DecodeConfig {
        let cookie = CookCookie::parse(&REAL_COOKIE).unwrap();
        let descriptor = Descriptor {
            channels_divisor: 2,
            sub_packet_size: 93,
        };
        let flavor = flavor_record(21).unwrap();
        DecodeConfig::from_inputs(&cookie, &descriptor, &flavor, 465).unwrap()
    }

    #[test]
    fn frame_pcm_size_derives_from_config() {
        // 20 480 bytes/call ÷ 5 frames/call = 4 096 bytes/frame
        // (= 1024 samples × 2 ch × 2 bytes).
        let a = CallPcmAssembler::from_config(&real_config()).unwrap();
        assert_eq!(a.frame_pcm_bytes(), 4_096);
        assert_eq!(a.buffered(), 0);
    }

    #[test]
    fn push_rejects_wrong_frame_size() {
        let mut a = CallPcmAssembler::from_config(&real_config()).unwrap();
        let err = a.push_frame_pcm(&[0u8; 4_095]).unwrap_err();
        assert_eq!(
            err,
            Error::FramePcmLengthMismatch {
                got: 4_095,
                expected: 4_096
            }
        );
        assert_eq!(a.buffered(), 0);
    }

    #[test]
    fn underrun_is_typed_and_non_destructive() {
        let mut a = CallPcmAssembler::from_config(&real_config()).unwrap();
        a.push_frame_pcm(&[1u8; 4_096]).unwrap();
        let mut out = vec![0u8; 8_192];
        let err = a.fill_call(&mut out).unwrap_err();
        assert_eq!(
            err,
            Error::PcmAssemblerUnderrun {
                need: 8_192,
                have: 4_096
            }
        );
        // Queue untouched on the typed rejection.
        assert_eq!(a.buffered(), 4_096);
    }

    #[test]
    fn fifo_order_is_preserved() {
        let mut a = CallPcmAssembler::from_config(&real_config()).unwrap();
        a.push_frame_pcm(&vec![0x11u8; 4_096]).unwrap();
        a.push_frame_pcm(&vec![0x22u8; 4_096]).unwrap();
        a.push_frame_pcm(&vec![0x33u8; 4_096]).unwrap();
        let mut out = vec![0u8; 8_192];
        a.fill_call(&mut out).unwrap();
        assert!(out[..4_096].iter().all(|&b| b == 0x11));
        assert!(out[4_096..].iter().all(|&b| b == 0x22));
        assert_eq!(a.buffered(), 4_096);
        a.reset();
        assert_eq!(a.buffered(), 0);
    }

    #[test]
    fn validated_144_call_cadence_reproduces_pinned_totals() {
        // validation/04 §5: per call 5 frames in; 8 192 bytes out on
        // call 0 and 20 480 thereafter; total 2 936 832 at call 144.
        // The constant carry backlog is pcm_bytes_per_call − warmup =
        // 12 288 bytes (three frames) from call 0 onward.
        let cfg = real_config();
        let mut a = CallPcmAssembler::from_config(&cfg).unwrap();
        let frame = vec![0u8; 4_096];
        let mut total_out = 0u64;
        for call in 0..144u32 {
            for _ in 0..cfg.sub_packets_per_call {
                a.push_frame_pcm(&frame).unwrap();
            }
            let budget = if call == 0 { 8_192usize } else { 20_480 };
            let mut out = vec![0xFFu8; budget];
            a.fill_call(&mut out).unwrap();
            total_out += budget as u64;
            assert_eq!(a.buffered(), 12_288, "constant 3-frame backlog");
            assert_eq!(total_out, cfg.total_pcm_bytes(call + 1));
        }
        assert_eq!(total_out, 2_936_832);
    }

    #[test]
    fn from_config_rejects_zero_sub_packets() {
        let mut cfg = real_config();
        cfg.sub_packets_per_call = 0;
        cfg.frame_bytes = 0;
        assert!(matches!(
            CallPcmAssembler::from_config(&cfg),
            Err(Error::FrameNotDivisibleBySubPacket { .. })
        ));
    }
}
