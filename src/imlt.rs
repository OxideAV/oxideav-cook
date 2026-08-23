//! Inverse MLT (inverse MDCT) — the §5 inverse-transform closed form.
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/01-cook-decoder-structure.md` §5.1 (the decode
//! chain runs an *"inverse MDCT at the selected block length with
//! windowing/overlap-add … → PCM out"*) and
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §5 (*"each
//! channel's spectrum is run through the inverse transform (the iMDCT
//! kernel …), windowed with one of the five Princen-Bradley windows …,
//! gain-scaled by the §1 envelope, and overlap-added into the output"*).
//!
//! ## What is pinned, and what this module wires
//!
//! The trace pins **what the transform is** — an inverse MDCT — and pins
//! the surrounding windows as Princen-Bradley TDAC windows
//! (`tables/mdct-windows.meta`). The inverse MDCT is a mathematical
//! object with a single defining closed form (up to normalisation /
//! reflection conventions):
//!
//! ```text
//! y[n] = (2/N) * Σ_{k=0}^{N-1} X[k] · cos( (π/N)·(n + ½ + N/2)·(k + ½) )
//! ```
//!
//! for `n = 0 .. 2N-1` — the unique member of the MDCT family whose
//! windowed overlap-add with symmetric TDAC windows (`W[k]² + W[k+N]² =
//! 1`, exactly the identity `mdct-windows.meta` pins) reconstructs the
//! input perfectly. [`imlt_direct`] evaluates that definition directly
//! (f64 accumulation, f32 output); [`mlt_direct`] is the forward
//! companion (`X[k] = Σ_n x[n]·cos(…)`, same kernel) used by the
//! perfect-reconstruction property tests.
//!
//! Two structural facts of this closed form are pinned by the definition
//! itself and asserted by the unit tests:
//!
//! - **Time-domain alias symmetry** — the output's first half is
//!   antisymmetric (`y[n] = −y[N−1−n]`) and its second half symmetric
//!   (`y[N+n] = y[2N−1−n]`); this is what makes the overlap-add cancel
//!   the aliasing (TDAC).
//! - **Tight-frame composition** — `mlt_direct(imlt_direct(X)) = 2·X`
//!   (the analysis basis covers each spectral vector twice across the
//!   alias structure).
//!
//! ## What stays a GAP (not wired)
//!
//! The binary's **fast kernel** (`cook.dll!0x5b70`, fed the pre/post
//! rotation table at RVA `0xa1b0`) is a recorded GAP with **no validated
//! closed form** (audit #16: the `0xa1b0` table is not a unit-circle
//! twiddle). This module wires the transform the stage is *pinned as* —
//! the definition-level inverse MDCT — not that kernel's factorisation.
//! Whether the binary's kernel normalisation / sign / reflection
//! convention matches this canonical one **cannot be verified until the
//! §3.2 entropy GAP lands** (there is no bit-exact spectral input to
//! compare against); the convention wired here is the one the stored
//! TDAC windows reconstruct perfectly with, and that caveat is recorded
//! here rather than silently assumed away.
//!
//! ## Wall-respect note
//!
//! No implementation was consulted: the closed form is the textbook
//! definition of the inverse MDCT, selected because `spec/01` §5.1 /
//! `spec/05` §5 pin the stage *as* an inverse MDCT and the vendored
//! windows pin the TDAC family. All tests validate against mathematical
//! identities and the vendored window tables only.

use crate::Error;

/// Inverse MLT / inverse MDCT — `N` spectral coefficients to `2N`
/// time-domain samples (`spec/01` §5.1, `spec/05` §5).
///
/// Evaluates the defining closed form
/// `y[n] = (2/N)·Σ_k X[k]·cos((π/N)(n + ½ + N/2)(k + ½))` directly with
/// f64 accumulation. The output block feeds the §5 windowing /
/// gain-scale / overlap-add stages ([`crate::output_stage`],
/// [`crate::synthesis`]).
///
/// # Errors
///
/// Returns [`Error::TransformSizeZero`] when `spectrum` is empty.
pub fn imlt_direct(spectrum: &[f32]) -> Result<Vec<f32>, Error> {
    let n = spectrum.len();
    if n == 0 {
        return Err(Error::TransformSizeZero);
    }
    let mut out = vec![0f32; 2 * n];
    // Exact-by-linearity fast path: the transform of the zero spectrum
    // is the zero block (the observe-gate / silent-frame case).
    if spectrum.iter().all(|&c| c == 0.0) {
        return Ok(out);
    }
    let nf = n as f64;
    let scale = 2.0 / nf;
    for (m, sample) in out.iter_mut().enumerate() {
        let phase = core::f64::consts::PI / nf * (m as f64 + 0.5 + nf / 2.0);
        let mut acc = 0f64;
        for (k, &coef) in spectrum.iter().enumerate() {
            acc += f64::from(coef) * (phase * (k as f64 + 0.5)).cos();
        }
        *sample = (scale * acc) as f32;
    }
    Ok(out)
}

/// Forward MLT / MDCT — `2N` time-domain samples to `N` spectral
/// coefficients (the analysis companion of [`imlt_direct`], same kernel,
/// unscaled).
///
/// Provided so consumers and tests can drive the §5 synthesis chain with
/// known spectra: an analysis-windowed signal pushed through
/// `mlt_direct` → [`imlt_direct`] → synthesis-window → overlap-add
/// reconstructs perfectly (the TDAC property the stored windows pin).
///
/// # Errors
///
/// - [`Error::TransformSizeZero`] when `block` is empty.
/// - [`Error::TransformInputLengthOdd`] when `block.len()` is odd (the
///   input must be `2N` samples).
pub fn mlt_direct(block: &[f32]) -> Result<Vec<f32>, Error> {
    if block.is_empty() {
        return Err(Error::TransformSizeZero);
    }
    if block.len() % 2 != 0 {
        return Err(Error::TransformInputLengthOdd { got: block.len() });
    }
    let n = block.len() / 2;
    let nf = n as f64;
    let mut out = vec![0f32; n];
    for (k, coef) in out.iter_mut().enumerate() {
        let kf = k as f64 + 0.5;
        let mut acc = 0f64;
        for (m, &sample) in block.iter().enumerate() {
            let phase = core::f64::consts::PI / nf * (m as f64 + 0.5 + nf / 2.0);
            acc += f64::from(sample) * (phase * kf).cos();
        }
        *coef = acc as f32;
    }
    Ok(out)
}

/// Inverse MLT — same closed form as [`imlt_direct`], computed in
/// `O(N log N)` for power-of-two sizes (the codec's frame sizes are
/// 256 / 512 / 1024 — spec/02 §1) and falling back to the direct
/// evaluation otherwise.
///
/// Mathematically identical to [`imlt_direct`] (the unit tests pin the
/// equality across sizes): the definition is folded onto a DCT-IV
/// (`y[n] = (2/N)·g(n + N/2)` with `g` extended by the kernel's own
/// `g(t) = −g(2N−1−t)` / `g(t) = −g(t−2N)` symmetries), and the DCT-IV
/// is read off the odd bins of a zero-padded `4N`-point FFT with a
/// half-bin twiddle — pure algebra on the same cosine kernel, no new
/// numeric facts.
///
/// # Errors
///
/// Returns [`Error::TransformSizeZero`] when `spectrum` is empty.
pub fn imlt(spectrum: &[f32]) -> Result<Vec<f32>, Error> {
    let n = spectrum.len();
    if n == 0 {
        return Err(Error::TransformSizeZero);
    }
    if !n.is_power_of_two() {
        return imlt_direct(spectrum);
    }
    let mut out = vec![0f32; 2 * n];
    if spectrum.iter().all(|&c| c == 0.0) {
        return Ok(out);
    }

    // DCT-IV via the odd bins of a 4N-point FFT:
    //   C[k] = Σ_n X[n]·cos((π/N)(n+½)(k+½))
    //        = Re( e^{−iπ(k+½)/(2N)} · Σ_n X[n]·e^{−i2πn(2k+1)/(4N)} ).
    let m = 4 * n;
    let mut re = vec![0f64; m];
    let mut im = vec![0f64; m];
    for (i, &x) in spectrum.iter().enumerate() {
        re[i] = f64::from(x);
    }
    fft_in_place(&mut re, &mut im);
    let nf = n as f64;
    let scale = 2.0 / nf;
    let mut c = vec![0f64; n];
    for (k, ck) in c.iter_mut().enumerate() {
        let ang = -core::f64::consts::PI * (k as f64 + 0.5) / (2.0 * nf);
        let (s, co) = ang.sin_cos();
        let bin = 2 * k + 1;
        *ck = co * re[bin] - s * im[bin];
    }
    // Fold the DCT-IV onto the 2N-sample IMLT output:
    //   y[n] = (2/N)·C[n+N/2]           for n ∈ [0, N/2)
    //   y[n] = −(2/N)·C[3N/2−1−n]       for n ∈ [N/2, 3N/2)
    //   y[n] = −(2/N)·C[n−3N/2]         for n ∈ [3N/2, 2N)
    let half = n / 2;
    for (i, sample) in out.iter_mut().enumerate() {
        let v = if i < half {
            c[i + half]
        } else if i < n + half {
            -c[n + half - 1 - i]
        } else {
            -c[i - n - half]
        };
        *sample = (scale * v) as f32;
    }
    Ok(out)
}

/// Iterative radix-2 decimation-in-time FFT (forward,
/// `e^{−i2πjk/len}` kernel) over split re/im f64 buffers whose length
/// is a power of two.
fn fft_in_place(re: &mut [f64], im: &mut [f64]) {
    let len = re.len();
    debug_assert!(len.is_power_of_two());
    // Bit-reversal permutation.
    let bits = len.trailing_zeros();
    for i in 0..len {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // Butterfly passes.
    let mut width = 2usize;
    while width <= len {
        let step = -2.0 * core::f64::consts::PI / width as f64;
        for start in (0..len).step_by(width) {
            for k in 0..width / 2 {
                let ang = step * k as f64;
                let (ws, wc) = ang.sin_cos();
                let a = start + k;
                let b = a + width / 2;
                let tr = wc * re[b] - ws * im[b];
                let ti = wc * im[b] + ws * re[b];
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        width *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random f32 in [-1, 1] (no dev-deps).
    fn prng(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((*seed >> 8) as f32 / (1 << 24) as f32) * 2.0 - 1.0
    }

    #[test]
    fn imlt_rejects_empty_spectrum() {
        assert_eq!(imlt_direct(&[]).unwrap_err(), Error::TransformSizeZero);
    }

    #[test]
    fn mlt_rejects_empty_and_odd_input() {
        assert_eq!(mlt_direct(&[]).unwrap_err(), Error::TransformSizeZero);
        assert_eq!(
            mlt_direct(&[1.0, 2.0, 3.0]).unwrap_err(),
            Error::TransformInputLengthOdd { got: 3 }
        );
    }

    #[test]
    fn imlt_of_zero_spectrum_is_zero() {
        let y = imlt_direct(&[0.0; 31]).unwrap();
        assert_eq!(y.len(), 62);
        assert!(y.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn imlt_output_has_tdac_alias_symmetry() {
        // First half antisymmetric (y[n] = -y[N-1-n]), second half
        // symmetric (y[N+n] = y[2N-1-n]) — the structure that makes the
        // overlap-add cancel the time-domain aliasing.
        let mut seed = 7u32;
        for n in [3usize, 7, 15, 31, 64] {
            let spectrum: Vec<f32> = (0..n).map(|_| prng(&mut seed)).collect();
            let y = imlt_direct(&spectrum).unwrap();
            for i in 0..n / 2 {
                assert!(
                    (y[i] + y[n - 1 - i]).abs() < 1e-5,
                    "first half not antisymmetric at n={n}, i={i}"
                );
                assert!(
                    (y[n + i] - y[2 * n - 1 - i]).abs() < 1e-5,
                    "second half not symmetric at n={n}, i={i}"
                );
            }
        }
    }

    #[test]
    fn imlt_is_linear() {
        let mut seed = 11u32;
        let a: Vec<f32> = (0..15).map(|_| prng(&mut seed)).collect();
        let b: Vec<f32> = (0..15).map(|_| prng(&mut seed)).collect();
        let combo: Vec<f32> = a
            .iter()
            .zip(b.iter())
            .map(|(&x, &y)| 2.5 * x - 0.5 * y)
            .collect();
        let ya = imlt_direct(&a).unwrap();
        let yb = imlt_direct(&b).unwrap();
        let yc = imlt_direct(&combo).unwrap();
        for i in 0..yc.len() {
            assert!(
                (yc[i] - (2.5 * ya[i] - 0.5 * yb[i])).abs() < 1e-4,
                "linearity fail at {i}"
            );
        }
    }

    #[test]
    fn forward_after_inverse_is_twice_identity() {
        // MLT ∘ IMLT = 2·id — the tight-frame composition of the
        // canonical normalisation (forward unscaled, inverse 2/N).
        let mut seed = 23u32;
        for n in [3usize, 7, 31] {
            let spectrum: Vec<f32> = (0..n).map(|_| prng(&mut seed)).collect();
            let round = mlt_direct(&imlt_direct(&spectrum).unwrap()).unwrap();
            for k in 0..n {
                assert!(
                    (round[k] - 2.0 * spectrum[k]).abs() < 1e-4,
                    "composition fail n={n}, k={k}: {} vs {}",
                    round[k],
                    2.0 * spectrum[k]
                );
            }
        }
    }

    #[test]
    fn fast_imlt_matches_direct_on_power_of_two_sizes() {
        // The O(N log N) path is the same closed form, algebraically
        // refactored — pin the numeric equality against the definition
        // across the codec's frame sizes and smaller powers of two.
        let mut seed = 0xFA57u32;
        for n in [4usize, 8, 64, 256, 512, 1024] {
            let spectrum: Vec<f32> = (0..n).map(|_| prng(&mut seed) * 100.0).collect();
            let fast = imlt(&spectrum).unwrap();
            let direct = imlt_direct(&spectrum).unwrap();
            assert_eq!(fast.len(), direct.len());
            let peak = direct.iter().fold(0f32, |a, &v| a.max(v.abs())).max(1.0);
            for i in 0..fast.len() {
                assert!(
                    (fast[i] - direct[i]).abs() <= 1e-4 * peak,
                    "fast/direct mismatch n={n}, i={i}: {} vs {}",
                    fast[i],
                    direct[i]
                );
            }
        }
    }

    #[test]
    fn fast_imlt_falls_back_to_direct_on_other_sizes() {
        // Non-power-of-two sizes (the vendored 3/7/15/31 window hops)
        // take the direct path — bit-identical results.
        let mut seed = 0xFA11u32;
        for n in [3usize, 7, 15, 31, 100] {
            let spectrum: Vec<f32> = (0..n).map(|_| prng(&mut seed)).collect();
            assert_eq!(
                imlt(&spectrum).unwrap(),
                imlt_direct(&spectrum).unwrap(),
                "fallback mismatch at n={n}"
            );
        }
    }

    #[test]
    fn fast_imlt_zero_and_empty_match_direct_contract() {
        assert_eq!(imlt(&[]).unwrap_err(), Error::TransformSizeZero);
        let y = imlt(&[0.0; 64]).unwrap();
        assert_eq!(y.len(), 128);
        assert!(y.iter().all(|&v| v == 0.0));
    }

    /// An exact-TDAC test window (`sin(π(k+½)/2L)` satisfies
    /// `W[k]² + W[k+L]² = 1` exactly) — a pure test fixture, not the
    /// codec's window.
    fn synthetic_tdac_window(hop: usize) -> Vec<f32> {
        (0..2 * hop)
            .map(|k| ((k as f64 + 0.5) * core::f64::consts::PI / (2.0 * hop as f64)).sin() as f32)
            .collect()
    }

    #[test]
    fn windowed_overlap_add_reconstructs_signal() {
        // The full perfect-reconstruction chain across several hop
        // sizes: analysis-window → MLT → IMLT → synthesis-window →
        // overlap-add reproduces the input, frame after frame.
        for hop in [3usize, 7, 15, 31] {
            let w = synthetic_tdac_window(hop);
            let frames = 6usize;
            let mut seed = 0x5EED_0000u32 ^ hop as u32;
            let signal: Vec<f32> = (0..hop * (frames + 2)).map(|_| prng(&mut seed)).collect();

            // Per-frame: analysis chunk at hop stride, windowed, MLT.
            let spectra: Vec<Vec<f32>> = (0..frames + 1)
                .map(|f| {
                    let chunk: Vec<f32> = signal[f * hop..f * hop + 2 * hop]
                        .iter()
                        .zip(w.iter())
                        .map(|(&s, &wk)| s * wk)
                        .collect();
                    mlt_direct(&chunk).unwrap()
                })
                .collect();

            // Synthesis: IMLT, synthesis window, overlap-add adjacent
            // frames — output block f covers signal[(f+1)*hop ..].
            for f in 0..frames {
                let ya = imlt_direct(&spectra[f]).unwrap();
                let yb = imlt_direct(&spectra[f + 1]).unwrap();
                for i in 0..hop {
                    let out = ya[hop + i] * w[hop + i] + yb[i] * w[i];
                    let want = signal[(f + 1) * hop + i];
                    assert!(
                        (out - want).abs() < 2e-3,
                        "PR fail hop={hop}, frame={f}, i={i}: {out} vs {want}"
                    );
                }
            }
        }
    }
}
