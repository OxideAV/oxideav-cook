//! Streaming §5 synthesis engine — inverse transform → window → gain →
//! overlap-add, one frame at a time.
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §5 (*"each
//! channel's spectrum is run through the inverse transform …, windowed
//! with one of the five Princen-Bradley windows …, gain-scaled by the §1
//! envelope, and overlap-added into the output"*) and
//! `docs/audio/cook/spec/01-cook-decoder-structure.md` §5.1 (the
//! per-frame decode chain and its carry/overlap state).
//!
//! ## What this module wires
//!
//! [`Synthesizer`] is the per-channel streaming state machine of the §5
//! stage order. Each [`Synthesizer::push_spectrum`] call:
//!
//! 1. runs the [`crate::imlt::imlt`] inverse transform (`N`
//!    spectral lines → `2N` time samples),
//! 2. multiplies by the full `2N`-tap Princen-Bradley window,
//! 3. optionally applies the §1.2 per-sub-block gain profile
//!    ([`crate::gain::apply_gain_blocks`]) — the *"windowed …,
//!    gain-scaled …, and overlap-added"* order of `spec/05` §5,
//! 4. overlap-adds the block's first half against the carried tail of
//!    the previous block and emits `N` finished samples,
//! 5. carries the block's second half as the next call's tail.
//!
//! The tail starts **zeroed**, so the first emitted block carries only
//! the current frame's contribution — consistent with the *"zeroed
//! overlap-add output"* the observe gate emits and the validator's
//! first-call warm-up accounting (`validation/04` §4.3 / §5).
//!
//! ## Window inputs
//!
//! [`Synthesizer::from_stored`] builds the engine over one of the five
//! vendored windows ([`crate::mdct::mdct_full_window`], hops 3 / 7 / 15
//! / 31 / 64). [`Synthesizer::with_window`] accepts a caller-supplied
//! full window for other hop sizes: the **frame-length** window (e.g.
//! `2 × 1024` taps for the validated stream's 1024-sample frames) is not
//! among the extracted tables — like the codebook and coupling tables it
//! would be built at runtime — so it stays a caller input (a recorded
//! GAP), never fabricated here. The engine's arithmetic is
//! window-agnostic; its perfect-reconstruction property is pinned by the
//! tests over the vendored windows.
//!
//! ## Wall-respect note
//!
//! The stage order is `spec/05` §5's own; the transform is the
//! definition-level inverse MDCT ([`crate::imlt`]); the windows come
//! from the vendored table or from the caller. Nothing numeric is
//! invented.

use crate::{gain::apply_gain_blocks, imlt::imlt, mdct, Error};

/// Streaming per-channel §5 synthesis state — inverse transform →
/// window → gain → overlap-add, with the previous block's tail carried
/// across calls.
#[derive(Debug, Clone, PartialEq)]
pub struct Synthesizer {
    /// Full `2·hop`-tap synthesis window.
    window: Vec<f32>,
    /// Carried second half of the previous windowed block (`hop`
    /// samples; zeroed at start — the warm-up state).
    tail: Vec<f32>,
}

impl Synthesizer {
    /// Build the engine over a caller-supplied full window.
    ///
    /// `window.len()` must be even and non-zero; the hop (spectral
    /// lines per frame, samples emitted per frame) is `window.len() / 2`.
    /// For hop sizes beyond the five vendored rows the window is a
    /// caller input (see the module docs — the frame-length window is a
    /// recorded GAP).
    ///
    /// # Errors
    ///
    /// - [`Error::TransformSizeZero`] when `window` is empty.
    /// - [`Error::TransformInputLengthOdd`] when `window.len()` is odd.
    pub fn with_window(window: &[f32]) -> Result<Self, Error> {
        if window.is_empty() {
            return Err(Error::TransformSizeZero);
        }
        if window.len() % 2 != 0 {
            return Err(Error::TransformInputLengthOdd { got: window.len() });
        }
        Ok(Synthesizer {
            window: window.to_vec(),
            tail: vec![0.0; window.len() / 2],
        })
    }

    /// Build the engine over one of the five vendored Princen-Bradley
    /// windows ([`mdct::mdct_full_window`]); the hop equals the stored
    /// half-window length (3 / 7 / 15 / 31 / 64).
    pub fn from_stored(len: mdct::MdctWindowLength) -> Self {
        // The vendored full window is even-length and non-empty by
        // construction, so `with_window` cannot fail here.
        Self::with_window(mdct::mdct_full_window(len))
            .expect("vendored full windows are non-empty and even-length")
    }

    /// Build the engine over the **recovered long-transform window**
    /// ([`mdct::long_full_window_unit`], the runtime-recovered N=1024
    /// apodisation mirror-completed and rescaled to this engine's unit
    /// TDAC convention) — hop 512.
    ///
    /// This replaces the former frame-length-window GAP input for the
    /// long transform: the taps are the vendor decoder's own values
    /// (heap-recovered at `RAInitDecoder`, `provenance/06`), not a
    /// fabricated window. How the per-frame spectrum is arranged across
    /// hop-512 blocks by the vendor's fast kernel (`cook.dll!0x5b70`)
    /// stays tied to the recorded kernel GAP; the engine itself is
    /// block-cadence-agnostic.
    pub fn with_recovered_long_window() -> Self {
        Self::with_window(mdct::long_full_window_unit())
            .expect("recovered long window is non-empty and even-length")
    }

    /// Spectral lines consumed / samples emitted per pushed frame.
    pub fn hop(&self) -> usize {
        self.window.len() / 2
    }

    /// The full synthesis window driving this engine.
    pub fn window(&self) -> &[f32] {
        &self.window
    }

    /// The carried overlap tail (the next call's previous-block
    /// contribution). `hop()` samples; all-zero before the first push
    /// and after [`Synthesizer::reset`].
    pub fn tail(&self) -> &[f32] {
        &self.tail
    }

    /// Clear the carried tail back to the zeroed warm-up state.
    pub fn reset(&mut self) {
        self.tail.fill(0.0);
    }

    /// Push one frame's spectrum through the §5 chain and emit `hop()`
    /// finished time-domain samples.
    ///
    /// Shorthand for [`Synthesizer::push_spectrum_with_gain`] with the
    /// flat unity §1 envelope (`spec/05` §1.1: zero gain segments — the
    /// flat-envelope frame).
    ///
    /// # Errors
    ///
    /// See [`Synthesizer::push_spectrum_with_gain`].
    pub fn push_spectrum(&mut self, spectrum: &[f32]) -> Result<Vec<f32>, Error> {
        self.push_spectrum_with_gain(spectrum, &[1.0])
    }

    /// Push one frame's spectrum through the §5 chain — inverse
    /// transform, window, §1.2 gain profile, overlap-add — and emit
    /// `hop()` finished samples.
    ///
    /// `spectrum` must carry exactly `hop()` coefficients (zero-fill
    /// uncoded upper lines before calling — see
    /// [`crate::subband::SubbandGeometry::total_coded_lines`]).
    /// `gain_blocks` is the expanded §1.2 per-sub-block factor profile
    /// ([`crate::gain::expand_gain_envelope`]); pass `&[1.0]` for the
    /// flat envelope. The profile is applied across the full `2N`
    /// windowed block, in the `spec/05` §5 order (*"windowed …,
    /// gain-scaled …, and overlap-added"*).
    ///
    /// # Errors
    ///
    /// - [`Error::SynthesisSpectrumLengthMismatch`] when
    ///   `spectrum.len() != hop()`.
    /// - [`Error::GainBlockCountZero`] when `gain_blocks` is empty.
    pub fn push_spectrum_with_gain(
        &mut self,
        spectrum: &[f32],
        gain_blocks: &[f32],
    ) -> Result<Vec<f32>, Error> {
        let hop = self.hop();
        if spectrum.len() != hop {
            return Err(Error::SynthesisSpectrumLengthMismatch {
                got: spectrum.len(),
                hop,
            });
        }
        // §5 stage 1: inverse transform (N → 2N; the fast path for
        // power-of-two hops, definition-equal — see crate::imlt).
        let mut block = imlt(spectrum)?;
        // §5 stage 2: synthesis window.
        for (s, &w) in block.iter_mut().zip(self.window.iter()) {
            *s *= w;
        }
        // §5 stage 3: §1.2 per-sub-block gain profile.
        apply_gain_blocks(&mut block, gain_blocks)?;
        // §5 stage 4: overlap-add first half against the carried tail.
        let out: Vec<f32> = self
            .tail
            .iter()
            .zip(block[..hop].iter())
            .map(|(&t, &c)| t + c)
            .collect();
        // Stage 5: carry the second half as the next call's tail.
        self.tail.copy_from_slice(&block[hop..]);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdct::MdctWindowLength;
    use crate::mlt_direct;

    fn prng(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((*seed >> 8) as f32 / (1 << 24) as f32) * 2.0 - 1.0
    }

    /// An exact-TDAC test window for arbitrary even hop sizes
    /// (`sin(π(k+½)/2L)` satisfies `W[k]² + W[k+L]² = 1` exactly): a
    /// pure test fixture for exercising the engine at hops without a
    /// vendored window — it does not claim to be the codec's window.
    fn synthetic_tdac_window(hop: usize) -> Vec<f32> {
        (0..2 * hop)
            .map(|k| ((k as f64 + 0.5) * core::f64::consts::PI / (2.0 * hop as f64)).sin() as f32)
            .collect()
    }

    #[test]
    fn with_window_rejects_empty_and_odd() {
        assert_eq!(
            Synthesizer::with_window(&[]).unwrap_err(),
            Error::TransformSizeZero
        );
        assert_eq!(
            Synthesizer::with_window(&[1.0, 1.0, 1.0]).unwrap_err(),
            Error::TransformInputLengthOdd { got: 3 }
        );
    }

    #[test]
    fn from_stored_wires_the_vendored_window() {
        for wl in MdctWindowLength::ALL {
            let s = Synthesizer::from_stored(wl);
            assert_eq!(s.hop(), wl.window_len());
            assert_eq!(s.window(), crate::mdct::mdct_full_window(wl));
            assert!(s.tail().iter().all(|&v| v == 0.0), "fresh tail zeroed");
        }
    }

    #[test]
    fn push_rejects_wrong_spectrum_length() {
        let mut s = Synthesizer::from_stored(MdctWindowLength::L7);
        assert_eq!(
            s.push_spectrum(&[0.0; 6]).unwrap_err(),
            Error::SynthesisSpectrumLengthMismatch { got: 6, hop: 7 }
        );
    }

    #[test]
    fn push_rejects_empty_gain_profile() {
        let mut s = Synthesizer::from_stored(MdctWindowLength::L3);
        assert_eq!(
            s.push_spectrum_with_gain(&[0.0; 3], &[]).unwrap_err(),
            Error::GainBlockCountZero
        );
    }

    #[test]
    fn zero_spectra_stream_emits_zero_pcm() {
        // The all-zero spectral stream synthesizes to all-zero output —
        // the observe-gate / warm-up consistency property
        // (validation/04 §4.3: zeroed overlap-add output).
        let mut s = Synthesizer::from_stored(MdctWindowLength::L31);
        for _ in 0..4 {
            let out = s.push_spectrum(&[0.0; 31]).unwrap();
            assert_eq!(out.len(), 31);
            assert!(out.iter().all(|&v| v == 0.0));
            assert!(s.tail().iter().all(|&v| v == 0.0));
        }
    }

    #[test]
    fn streaming_reconstruction_over_stored_windows() {
        // End-to-end streaming PR over the vendored windows whose TDAC
        // identity the .meta pins: analysis-window+MLT per frame, push
        // through the engine, and every emitted block from the second
        // on reproduces the source signal.
        for wl in [
            MdctWindowLength::L3,
            MdctWindowLength::L7,
            MdctWindowLength::L15,
            MdctWindowLength::L31,
        ] {
            let mut s = Synthesizer::from_stored(wl);
            let hop = s.hop();
            let w = s.window().to_vec();
            let frames = 6usize;
            let mut seed = 0xC00C_0000u32 ^ hop as u32;
            let signal: Vec<f32> = (0..hop * (frames + 2)).map(|_| prng(&mut seed)).collect();

            for f in 0..frames {
                let chunk: Vec<f32> = signal[f * hop..f * hop + 2 * hop]
                    .iter()
                    .zip(w.iter())
                    .map(|(&x, &wk)| x * wk)
                    .collect();
                let spectrum = mlt_direct(&chunk).unwrap();
                let out = s.push_spectrum(&spectrum).unwrap();
                if f == 0 {
                    continue; // warm-up block: previous tail was zero.
                }
                for i in 0..hop {
                    let want = signal[f * hop + i];
                    assert!(
                        (out[i] - want).abs() < 2e-3,
                        "PR fail hop={hop}, frame={f}, i={i}: {} vs {want}",
                        out[i]
                    );
                }
            }
        }
    }

    #[test]
    fn streaming_reconstruction_with_synthetic_window_at_larger_hop() {
        // The engine is window-agnostic: an exact-TDAC synthetic window
        // (test fixture) at hop 128 reconstructs the same way.
        let window = synthetic_tdac_window(128);
        let mut s = Synthesizer::with_window(&window).unwrap();
        let hop = s.hop();
        assert_eq!(hop, 128);
        let frames = 4usize;
        let mut seed = 0xABCD_1234u32;
        let signal: Vec<f32> = (0..hop * (frames + 2)).map(|_| prng(&mut seed)).collect();
        for f in 0..frames {
            let chunk: Vec<f32> = signal[f * hop..f * hop + 2 * hop]
                .iter()
                .zip(window.iter())
                .map(|(&x, &w)| x * w)
                .collect();
            let spectrum = mlt_direct(&chunk).unwrap();
            let out = s.push_spectrum(&spectrum).unwrap();
            if f == 0 {
                continue;
            }
            for i in 0..hop {
                let want = signal[f * hop + i];
                assert!(
                    (out[i] - want).abs() < 2e-3,
                    "PR fail frame={f}, i={i}: {} vs {want}",
                    out[i]
                );
            }
        }
    }

    #[test]
    fn streaming_reconstruction_with_the_recovered_long_window() {
        // The recovered N=1024 window (unit-TDAC form) drives the
        // engine at hop 512 and reconstructs a random source through
        // the full analysis → synthesis chain. The recovered taps sit
        // on an integer grid (mirror about tap 512, not 511.5 — see
        // mdct::long_full_window), so alias cancellation carries a
        // half-sample residual of ~2e-3 relative; the tolerance covers
        // it and the reconstruction is otherwise exact.
        let mut s = Synthesizer::with_recovered_long_window();
        let hop = s.hop();
        assert_eq!(hop, 512);
        let w = s.window().to_vec();
        let frames = 4usize;
        let mut seed = 0x1024_5121u32;
        let signal: Vec<f32> = (0..hop * (frames + 2)).map(|_| prng(&mut seed)).collect();
        for f in 0..frames {
            let chunk: Vec<f32> = signal[f * hop..f * hop + 2 * hop]
                .iter()
                .zip(w.iter())
                .map(|(&x, &wk)| x * wk)
                .collect();
            let spectrum = mlt_direct(&chunk).unwrap();
            let out = s.push_spectrum(&spectrum).unwrap();
            if f == 0 {
                continue; // warm-up block.
            }
            let mut max_err = 0.0f32;
            for i in 0..hop {
                let want = signal[f * hop + i];
                max_err = max_err.max((out[i] - want).abs());
            }
            assert!(
                max_err < 8e-3,
                "recovered-window PR: frame {f} max err {max_err}"
            );
        }
    }

    #[test]
    fn flat_gain_two_scales_the_current_contribution() {
        // On a fresh engine (zero tail) a flat gain profile of 2.0
        // doubles the emitted block relative to the unity profile.
        let spectrum: Vec<f32> = (0..15).map(|k| (k as f32) - 7.0).collect();
        let mut a = Synthesizer::from_stored(MdctWindowLength::L15);
        let mut b = Synthesizer::from_stored(MdctWindowLength::L15);
        let unity = a.push_spectrum(&spectrum).unwrap();
        let doubled = b.push_spectrum_with_gain(&spectrum, &[2.0]).unwrap();
        for i in 0..15 {
            assert!(
                (doubled[i] - 2.0 * unity[i]).abs() < 1e-5,
                "gain scale fail at {i}"
            );
        }
        // The carried tail is scaled too (gain applies to the full
        // windowed block before the overlap split).
        for i in 0..15 {
            assert!((b.tail()[i] - 2.0 * a.tail()[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn reset_restores_warmup_state() {
        let mut s = Synthesizer::from_stored(MdctWindowLength::L7);
        let spectrum = [1.0f32; 7];
        s.push_spectrum(&spectrum).unwrap();
        assert!(s.tail().iter().any(|&v| v != 0.0));
        s.reset();
        assert!(s.tail().iter().all(|&v| v == 0.0));
        // Post-reset behaviour matches a fresh engine.
        let fresh = Synthesizer::from_stored(MdctWindowLength::L7)
            .push_spectrum(&spectrum)
            .unwrap();
        let after = s.push_spectrum(&spectrum).unwrap();
        assert_eq!(fresh, after);
    }
}
