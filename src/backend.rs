//! Synthesis backend — the assembled post-entropy half of the backend
//! frame decode: reconstructed spectra in, per-call 16-bit PCM out.
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §5 (the
//! per-channel stage order: inverse transform → window → §1 gain →
//! overlap-add), `docs/audio/cook/spec/01-cook-decoder-structure.md`
//! §5 / §5.1 (the decode chain ends *"… → PCM out"* and the driver's
//! carry-buffer accounting), and
//! `docs/audio/cook/validation/04-cook-stream-validation.md` §5 (the
//! per-call PCM cadence the assembly reproduces).
//!
//! ## Position in the pipeline
//!
//! This is the stage that consumes the [`crate::frame::FrameSpectrum`]
//! the post-entropy reconstruction produces
//! ([`crate::frame::reconstruct_frame_spectrum`]) and finishes the
//! decode:
//!
//! ```text
//! FrameSpectrum ──per channel──▶ Synthesizer (§5 iMLT→window→gain→OLA)
//!               ──interleave──▶ pcm (16-bit LE) ──▶ CallPcmAssembler
//!               ──fill_call──▶ one RADecode call's PCM budget
//! ```
//!
//! With this module every stage between the §3.2 entropy blocker and
//! the emitted PCM bytes is wired; the entropy-decoded spectra remain
//! the caller's GAP-sourced input (docs-gap #1775), and the
//! **frame-length synthesis window** is a caller input too (the stored
//! table holds only the 3/7/15/31/64 rows — the `2 × samples_per_frame`
//! window is runtime-built like the codebooks, a recorded GAP).
//!
//! ## Uncoded upper lines
//!
//! The §2.1 subband geometry codes only
//! [`crate::subband::SubbandGeometry::total_coded_lines`] of the
//! transform's `samples_per_frame` spectral lines; the remaining upper
//! lines carry no bitstream data and are zero-filled before the
//! transform — the only non-fabricating completion.

use crate::{
    assembler::CallPcmAssembler,
    frame::FrameSpectrum,
    init::DecodeConfig,
    pcm::{interleave_stereo, pcm_i16le},
    synthesis::Synthesizer,
    Error,
};

/// Post-entropy synthesis backend: per-channel §5 synthesis engines +
/// the per-call PCM carry buffer, wired from one [`DecodeConfig`].
#[derive(Debug, Clone, PartialEq)]
pub struct SynthesisBackend {
    channels: u16,
    synths: Vec<Synthesizer>,
    assembler: CallPcmAssembler,
}

impl SynthesisBackend {
    /// Build the backend for a wired config and a caller-supplied full
    /// synthesis window.
    ///
    /// `window.len()` must equal `2 × samples_per_frame` (the §5
    /// transform emits `2N` samples per `N`-line frame). The
    /// frame-length window is **not** among the five extracted rows —
    /// like the §3.2 codebooks it would be built at runtime, so it is a
    /// recorded GAP input, never fabricated here.
    ///
    /// # Errors
    ///
    /// - [`Error::SynthesisWindowLengthMismatch`] when the window is not
    ///   `2 × samples_per_frame` taps.
    /// - [`Error::CookieInvalidChannels`] when the config's channel
    ///   count is neither 1 nor 2.
    /// - [`Error::FrameNotDivisibleBySubPacket`] for a zero-sub-packet
    ///   config (see [`CallPcmAssembler::from_config`]).
    pub fn new(config: &DecodeConfig, window: &[f32]) -> Result<Self, Error> {
        let expected = 2 * config.samples_per_frame as usize;
        if window.len() != expected {
            return Err(Error::SynthesisWindowLengthMismatch {
                got: window.len(),
                expected,
            });
        }
        if config.channels != 1 && config.channels != 2 {
            return Err(Error::CookieInvalidChannels {
                got: config.channels,
            });
        }
        let assembler = CallPcmAssembler::from_config(config)?;
        let synths = (0..config.channels)
            .map(|_| Synthesizer::with_window(window))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SynthesisBackend {
            channels: config.channels,
            synths,
            assembler,
        })
    }

    /// Spectral lines per frame (= `samples_per_frame`).
    pub fn hop(&self) -> usize {
        self.synths[0].hop()
    }

    /// Channel count the backend routes.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// PCM bytes currently riding in the per-call carry buffer.
    pub fn buffered(&self) -> usize {
        self.assembler.buffered()
    }

    /// Push one frame's reconstructed spectra through the §5 chain with
    /// the flat unity §1 envelope on every channel.
    ///
    /// # Errors
    ///
    /// See [`SynthesisBackend::push_frame_with_gain`].
    pub fn push_frame(&mut self, spectrum: &FrameSpectrum) -> Result<(), Error> {
        match self.channels {
            1 => self.push_frame_with_gain(spectrum, &[&[1.0]]),
            _ => self.push_frame_with_gain(spectrum, &[&[1.0], &[1.0]]),
        }
    }

    /// Push one frame's reconstructed spectra through the §5 chain —
    /// per-channel inverse transform, window, §1.2 gain profile,
    /// overlap-add — interleave, convert to 16-bit LE PCM, and enqueue
    /// the frame into the per-call carry buffer.
    ///
    /// `gain_blocks` carries one expanded §1.2 factor profile per
    /// channel ([`crate::gain::expand_gain_envelope`]; `&[1.0]` for a
    /// flat envelope). Spectra shorter than the transform size have
    /// their uncoded upper lines zero-filled (see the module docs).
    ///
    /// # Errors
    ///
    /// - [`Error::FrameSpectrumChannelMismatch`] when the spectrum's
    ///   channel routing disagrees with the config.
    /// - [`Error::GainProfileCountMismatch`] when `gain_blocks` does not
    ///   carry one profile per channel.
    /// - [`Error::SpectrumExceedsTransformSize`] when a channel carries
    ///   more spectral lines than the transform admits.
    /// - [`Error::GainBlockCountZero`] when a profile is empty.
    pub fn push_frame_with_gain(
        &mut self,
        spectrum: &FrameSpectrum,
        gain_blocks: &[&[f32]],
    ) -> Result<(), Error> {
        if gain_blocks.len() != self.channels as usize {
            return Err(Error::GainProfileCountMismatch {
                got: gain_blocks.len(),
                expected: self.channels as usize,
            });
        }
        let samples = match (spectrum, self.channels) {
            (FrameSpectrum::Mono(spec), 1) => {
                let padded = self.pad_spectrum(spec)?;
                self.synths[0].push_spectrum_with_gain(&padded, gain_blocks[0])?
            }
            (FrameSpectrum::Stereo(s), 2) => {
                let p0 = self.pad_spectrum(&s.ch0)?;
                let p1 = self.pad_spectrum(&s.ch1)?;
                let ch0 = self.synths[0].push_spectrum_with_gain(&p0, gain_blocks[0])?;
                let ch1 = self.synths[1].push_spectrum_with_gain(&p1, gain_blocks[1])?;
                interleave_stereo(&ch0, &ch1)?
            }
            (FrameSpectrum::Mono(_), other) => {
                return Err(Error::FrameSpectrumChannelMismatch {
                    got: 1,
                    expected: other,
                })
            }
            (FrameSpectrum::Stereo(_), other) => {
                return Err(Error::FrameSpectrumChannelMismatch {
                    got: 2,
                    expected: other,
                })
            }
        };
        self.assembler.push_frame_pcm(&pcm_i16le(&samples))
    }

    /// Dequeue one call's PCM budget (size the buffer with
    /// [`crate::driver::Driver::next_call_pcm_bytes`], then account the
    /// call with [`crate::driver::Driver::advance_after_decode`]).
    ///
    /// # Errors
    ///
    /// Returns [`Error::PcmAssemblerUnderrun`] when fewer bytes are
    /// buffered than `out` requires.
    pub fn fill_call(&mut self, out: &mut [u8]) -> Result<(), Error> {
        self.assembler.fill_call(out)
    }

    /// Reset every per-channel overlap tail and drop buffered PCM
    /// (stream reset).
    pub fn reset(&mut self) {
        for s in &mut self.synths {
            s.reset();
        }
        self.assembler.reset();
    }

    /// Zero-fill a coded spectrum's uncoded upper lines up to the
    /// transform size.
    fn pad_spectrum(&self, coded: &[f32]) -> Result<Vec<f32>, Error> {
        let hop = self.hop();
        if coded.len() > hop {
            return Err(Error::SpectrumExceedsTransformSize {
                got: coded.len(),
                hop,
            });
        }
        let mut full = vec![0.0f32; hop];
        full[..coded.len()].copy_from_slice(coded);
        Ok(full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cookie::CookCookie, driver::Driver, flavor::flavor_record, init::Descriptor,
        reconstruct::StereoSpectra,
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

    /// Exact-TDAC synthesis window test fixture (`sin(π(k+½)/2L)`), for
    /// hops with no vendored row — it does not claim to be the codec's
    /// frame-length window (a recorded GAP input).
    fn synthetic_tdac_window(hop: usize) -> Vec<f32> {
        (0..2 * hop)
            .map(|k| ((k as f64 + 0.5) * core::f64::consts::PI / (2.0 * hop as f64)).sin() as f32)
            .collect()
    }

    fn zero_stereo_frame(lines: usize) -> FrameSpectrum {
        FrameSpectrum::Stereo(StereoSpectra {
            ch0: vec![0.0; lines],
            ch1: vec![0.0; lines],
        })
    }

    #[test]
    fn new_validates_window_length_against_config() {
        let cfg = real_config();
        // spf = 1024 → the window must be 2 048 taps.
        let err = SynthesisBackend::new(&cfg, &[1.0; 128]).unwrap_err();
        assert_eq!(
            err,
            Error::SynthesisWindowLengthMismatch {
                got: 128,
                expected: 2_048
            }
        );
        let b = SynthesisBackend::new(&cfg, &synthetic_tdac_window(1_024)).unwrap();
        assert_eq!(b.hop(), 1_024);
        assert_eq!(b.channels(), 2);
        assert_eq!(b.buffered(), 0);
    }

    #[test]
    fn push_rejects_channel_and_arity_mismatches() {
        let cfg = real_config();
        let mut b = SynthesisBackend::new(&cfg, &synthetic_tdac_window(1_024)).unwrap();
        // Mono spectrum on a stereo config.
        let err = b
            .push_frame(&FrameSpectrum::Mono(vec![0.0; 16]))
            .unwrap_err();
        assert_eq!(
            err,
            Error::FrameSpectrumChannelMismatch {
                got: 1,
                expected: 2
            }
        );
        // One gain profile for two channels.
        let err = b
            .push_frame_with_gain(&zero_stereo_frame(16), &[&[1.0]])
            .unwrap_err();
        assert_eq!(
            err,
            Error::GainProfileCountMismatch {
                got: 1,
                expected: 2
            }
        );
        // A spectrum wider than the transform.
        let err = b.push_frame(&zero_stereo_frame(1_025)).unwrap_err();
        assert_eq!(
            err,
            Error::SpectrumExceedsTransformSize {
                got: 1_025,
                hop: 1_024
            }
        );
        assert_eq!(b.buffered(), 0, "rejections buffer nothing");
    }

    #[test]
    fn zero_spectra_walk_reproduces_observe_gate_output_and_cadence() {
        // Feed the validated 144-call × 5-frame cadence with silent
        // spectra: the emitted PCM must be byte-identical to the
        // observe-gate output (all-zero — validation/04 §4.3) and the
        // totals must land on the pinned 2 936 832 bytes, in lockstep
        // with the Driver's session accounting.
        let cfg = real_config();
        let mut b = SynthesisBackend::new(&cfg, &synthetic_tdac_window(1_024)).unwrap();
        let mut d = Driver::new(cfg);
        for _call in 0..144u32 {
            for _ in 0..cfg.sub_packets_per_call {
                b.push_frame(&zero_stereo_frame(50)).unwrap();
            }
            let budget = d.next_call_pcm_bytes() as usize;
            let mut out = vec![0xAAu8; budget];
            b.fill_call(&mut out).unwrap();
            assert!(out.iter().all(|&v| v == 0), "silent PCM");
            d.advance_after_decode(out.len()).unwrap();
        }
        assert_eq!(d.calls_completed(), 144);
        assert_eq!(d.total_pcm_emitted(), 2_936_832);
        // The constant three-frame carry backlog rides at end of stream.
        assert_eq!(b.buffered(), 12_288);
    }

    #[test]
    fn mono_roundtrip_reconstructs_signal_through_pcm() {
        // End-to-end at a literal mono hop-64 geometry: MLT-analysed
        // frames pushed through the backend come back out of the PCM
        // stage as the source signal (16-bit quantisation + PR
        // tolerance). Uses the exact-TDAC fixture window.
        let hop = 64usize;
        let cfg = DecodeConfig {
            channels: 1,
            sample_rate_hz: 8_000,
            samples_per_frame: hop as u32,
            subband_count: 12,
            stereo_mode: 0,
            frame_bytes: 96,
            sub_packet_size: 48,
            sub_packets_per_call: 2,
            pcm_bytes_per_call: 2 * hop as u32 * 2,
        };
        let window = synthetic_tdac_window(hop);
        let mut b = SynthesisBackend::new(&cfg, &window).unwrap();

        // A deterministic smooth signal at ~half full scale.
        let frames = 8usize;
        let signal: Vec<f32> = (0..hop * (frames + 2))
            .map(|i| 12_000.0 * ((i as f32) * 0.05).sin() + 5_000.0 * ((i as f32) * 0.013).cos())
            .collect();

        let mut emitted: Vec<u8> = Vec::new();
        for f in 0..frames {
            let chunk: Vec<f32> = signal[f * hop..f * hop + 2 * hop]
                .iter()
                .zip(window.iter())
                .map(|(&x, &w)| x * w)
                .collect();
            let spectrum = crate::mlt_direct(&chunk).unwrap();
            b.push_frame(&FrameSpectrum::Mono(spectrum)).unwrap();
            let mut out = vec![0u8; cfg.pcm_bytes_per_call as usize / 2];
            b.fill_call(&mut out).unwrap();
            emitted.extend_from_slice(&out);
        }
        // Decode the PCM back to samples; skip the warm-up frame
        // (zero previous tail) and compare against the source.
        for i in hop..frames * hop {
            let lo = emitted[2 * i] as u16;
            let hi = emitted[2 * i + 1] as u16;
            let got = (lo | (hi << 8)) as i16 as f32;
            let want = signal[i];
            assert!(
                (got - want).abs() < 64.0,
                "PCM roundtrip fail at {i}: {got} vs {want}"
            );
        }
    }

    #[test]
    fn reset_clears_tails_and_buffer() {
        let cfg = real_config();
        let mut b = SynthesisBackend::new(&cfg, &synthetic_tdac_window(1_024)).unwrap();
        // A non-zero frame leaves a tail + buffered bytes.
        let spec = FrameSpectrum::Stereo(StereoSpectra {
            ch0: vec![100.0; 32],
            ch1: vec![-100.0; 32],
        });
        b.push_frame(&spec).unwrap();
        assert_eq!(b.buffered(), 4_096);
        b.reset();
        assert_eq!(b.buffered(), 0);
        // Post-reset the backend behaves like a fresh one.
        let mut fresh = SynthesisBackend::new(&cfg, &synthetic_tdac_window(1_024)).unwrap();
        b.push_frame(&spec).unwrap();
        fresh.push_frame(&spec).unwrap();
        let mut a_out = vec![0u8; 4_096];
        let mut f_out = vec![0u8; 4_096];
        b.fill_call(&mut a_out).unwrap();
        fresh.fill_call(&mut f_out).unwrap();
        assert_eq!(a_out, f_out);
    }
}
