//! Inverse Modified Discrete Cosine Transform for cook.
//!
//! Cook uses an iMDCT of length `N = samples_per_channel` (256 / 512 /
//! 1024). The transform takes N coefficients and produces 2N output
//! samples; cook keeps only the second half as new PCM, with the first
//! half consumed by the overlap-add (`prev * window[N-1-i]`) step.
//!
//! Window (§5.8): half-sine, scaled by `√(2/N)`:
//!
//! ```text
//! window[i] = sin((i + 0.5) * π / (2N)) * √(2 / N)   for i ∈ [0, N-1]
//! ```
//!
//! MDCT scale (§9.1): `1/32768`.
//!
//! This is a clean-room reference implementation: O(N²) direct
//! evaluation. Cook frames are at most 1024 samples (one cook frame
//! every ~23 ms at 44.1 kHz) so the cost is acceptable; a future
//! optimisation pass could swap in a 2N-point FFT-based transform.

use std::f32::consts::PI;

use crate::tables::pow2tab;

/// MDCT scale factor used by cook (§9.1).
pub const MDCT_SCALE: f32 = 1.0 / 32_768.0;

/// Build the cook window for length N. Returns a vector of N entries.
pub fn build_window(n: usize) -> Vec<f32> {
    let scale = (2.0_f32 / n as f32).sqrt();
    let inv_two_n = PI / (2.0 * n as f32);
    (0..n)
        .map(|i| ((i as f32 + 0.5) * inv_two_n).sin() * scale)
        .collect()
}

/// Run an inverse MDCT of length N on `coeffs[0..N]`. Writes 2N output
/// samples into `out[0..2N]`. `out` is overwritten.
///
/// Mathematical definition (DCT-IV like):
///
/// ```text
/// out[k] = Σ_{n=0..N-1} coeffs[n] * cos(π * (2k + 1 + N) * (2n + 1) / (4N))
///                                              for k ∈ [0, 2N - 1]
/// ```
pub fn imdct_naive(coeffs: &[f32], out: &mut [f32]) {
    let n = coeffs.len();
    debug_assert_eq!(out.len(), 2 * n);
    let inv_4n = PI / (4.0 * n as f32);
    let n_offset = n as f32;
    for k in 0..(2 * n) {
        let kf = k as f32;
        let mut acc = 0.0f32;
        for nn in 0..n {
            let nf = nn as f32;
            let angle = ((2.0 * kf + 1.0 + n_offset) * (2.0 * nf + 1.0)) * inv_4n;
            acc += coeffs[nn] * angle.cos();
        }
        out[k] = acc * MDCT_SCALE;
    }
}

/// Per-channel state carried across cook frames: the first half of the
/// previous iMDCT output (used as `prev[]` for the lapping) and the
/// gain index of the previous frame's last slot.
#[derive(Clone)]
pub struct ChannelState {
    pub prev: Vec<f32>,
    pub prev_gain: i32,
    pub window: std::sync::Arc<Vec<f32>>,
    pub gain_table: std::sync::Arc<[f32; 31]>,
    pub samples_per_channel: usize,
}

impl ChannelState {
    pub fn new(
        samples_per_channel: usize,
        window: std::sync::Arc<Vec<f32>>,
        gain_table: std::sync::Arc<[f32; 31]>,
    ) -> Self {
        Self {
            prev: vec![0.0; samples_per_channel],
            prev_gain: 0,
            window,
            gain_table,
            samples_per_channel,
        }
    }

    /// Run iMDCT on `coeffs`, apply the per-slot gain ramp, lap with the
    /// previous half-block, and produce N PCM samples (`samples_per_channel`).
    /// `gains[0..8]` are the 8 slot gain indices for this frame; `gains[8]`
    /// (if present) is the carry-over gain for next frame's `prev_gain` —
    /// in cook this is `gains[7]`, the last slot's index. Output is
    /// written to `out[0..N]`. Internal scratch is a 2N buffer that's
    /// freshly built each call.
    pub fn process(&mut self, coeffs: &[f32], gains: &[i32; 8], out: &mut [f32]) {
        let n = self.samples_per_channel;
        debug_assert_eq!(coeffs.len(), n);
        debug_assert_eq!(out.len(), n);
        let mut tmp = vec![0.0f32; 2 * n];
        imdct_naive(coeffs, &mut tmp);

        // Per-slot gain interpolation. Each slot is gain_size_factor =
        // n/8 samples wide. Constant gain: scale all slot samples by
        // 2^gain_index. Different gains: ratio gain_table[16 + (next-cur)]
        // applied gsf times per slot to ramp exponentially.
        let gsf = n / 8;
        let p2 = pow2tab();
        // Combine the iMDCT output with the lapping window AND the gain
        // ramp for the second half (the "new PCM" half). The first half
        // of `tmp` is the lapping contribution and isn't gained until
        // it becomes `prev` for the next frame — but cook applies the
        // gain after the lap, on the final output. Behavioural sequence
        // per §5.8:
        //   out[i] = imdct_out[i] * fc * window[i] - prev[i] * window[N-1-i]
        // where `fc = 2^prev_gain` (the previous frame's last gain).
        //
        // Then the per-slot gain ramp is applied to `out[0..N]` using
        // gains[0..8] of the *current* frame. The carry-over for next
        // frame's `prev_gain` is gains[7] (the last slot's index).
        let fc = if (-63..=63).contains(&self.prev_gain) {
            p2[(self.prev_gain + 63) as usize]
        } else {
            // Shouldn't happen on valid streams — gain idx is constrained
            // to [-7..+8], but we guard defensively.
            1.0
        };

        // First, build the lapped output (length N). `tmp[0..N]` holds the
        // first half of the new iMDCT, `prev[0..N]` holds the first half
        // of the previous iMDCT (= the lapping memory). `out[i]` is the
        // windowed sum.
        for i in 0..n {
            out[i] = tmp[i] * fc * self.window[i] - self.prev[i] * self.window[n - 1 - i];
        }

        // Save the second half as `prev` for the next frame.
        self.prev.copy_from_slice(&tmp[n..2 * n]);

        // Apply the per-slot gain ramp.
        // Slot s spans samples [s*gsf .. (s+1)*gsf).
        // - If gains[s] == gains[s-1] (or s == 0 with no carry), apply
        //   constant 2^gains[s] (here we use 2^gains[s] directly since
        //   the prev_gain has already multiplied the lap).
        // - Otherwise, build a geometric ramp from current to next gain.
        // Cook's gain semantics: each slot has its own gain index; the
        // *intra-slot* ramp interpolates from previous slot's gain to
        // current slot's gain over gsf samples.
        let mut prev_g = self.prev_gain;
        for s in 0..8 {
            let cur_g = gains[s];
            let slot_start = s * gsf;
            let cur_factor = if (-63..=63).contains(&cur_g) {
                p2[(cur_g + 63) as usize]
            } else {
                1.0
            };
            if cur_g == prev_g {
                // Constant gain across the slot — but we already scaled
                // the lap by fc=2^prev_gain. Apply (cur_factor / fc) as
                // an additional scale so each slot's output ends up at
                // 2^cur_g.
                let extra = cur_factor / fc;
                if (extra - 1.0).abs() > 1e-6 {
                    for i in 0..gsf {
                        out[slot_start + i] *= extra;
                    }
                }
            } else {
                // Build an exponential ramp from 2^prev_g to 2^cur_g over
                // gsf samples; combine it with /fc (= /2^prev_gain).
                let prev_factor = if (-63..=63).contains(&prev_g) {
                    p2[(prev_g + 63) as usize]
                } else {
                    1.0
                };
                let ratio = (cur_factor / prev_factor).powf(1.0 / gsf as f32);
                let mut r = prev_factor / fc;
                for i in 0..gsf {
                    out[slot_start + i] *= r;
                    r *= ratio;
                }
            }
            prev_g = cur_g;
        }

        // Carry over the last slot's gain index for next frame.
        self.prev_gain = gains[7];
    }

    pub fn reset(&mut self) {
        for v in self.prev.iter_mut() {
            *v = 0.0;
        }
        self.prev_gain = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_unity_overlap_property() {
        // Half-sine with √(2/N) scaling: window[i]^2 + window[N-1-i]^2 = 2/N.
        // Sum of sin^2((i+0.5)π/(2N)) + sin^2((N-1-i+0.5)π/(2N))
        //   = sin^2(θ) + cos^2(θ) = 1 (since the second equals
        //   sin((N-i-0.5)π/(2N)) = cos((i+0.5)π/(2N))).
        let n = 64;
        let w = build_window(n);
        for i in 0..n {
            let s = w[i] * w[i] + w[n - 1 - i] * w[n - 1 - i];
            assert!((s - 2.0 / n as f32).abs() < 1e-5, "i={i} sum={s}");
        }
    }

    #[test]
    fn imdct_zero_in_zero_out() {
        let n = 64;
        let coeffs = vec![0.0f32; n];
        let mut out = vec![0.0f32; 2 * n];
        imdct_naive(&coeffs, &mut out);
        for &v in &out {
            assert!(v.abs() < 1e-9);
        }
    }

    #[test]
    fn imdct_dc_produces_smooth_output() {
        // DC input (one non-zero coeff) should produce a smooth
        // sinusoidal output; just sanity-check that something happens.
        let n = 64;
        let mut coeffs = vec![0.0f32; n];
        coeffs[0] = 1.0;
        let mut out = vec![0.0f32; 2 * n];
        imdct_naive(&coeffs, &mut out);
        let max = out.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(max > 0.0);
    }
}
