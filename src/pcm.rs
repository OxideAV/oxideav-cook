//! PCM emission — float samples to the validator-pinned 16-bit LE
//! output format.
//!
//! Source-of-truth:
//! `docs/audio/cook/validation/04-cook-stream-validation.md` §5 (every
//! per-call PCM budget is `samples × channels × 2` bytes — the 16-bit
//! sample format behind [`crate::PCM_BYTES_PER_SAMPLE`]` = 2`, and the
//! observe gate's zeroed output is 16-bit PCM silence) and
//! `docs/audio/cook/spec/01-cook-decoder-structure.md` §5.1 (the decode
//! chain ends *"… → PCM out"*).
//!
//! ## What is pinned vs chosen
//!
//! **Pinned:** the output is 16-bit PCM, two bytes per sample,
//! little-endian words in the output buffer, `channels` interleaved
//! samples per sample instant (the per-call byte budgets only account
//! exactly under sample-interleaved `samples × channels × 2` packing).
//!
//! **Not pinned (recorded here):** the binary's float→int conversion
//! convention (rounding mode, clamp behaviour at full scale). This
//! module wires round-to-nearest (ties away from zero, the `f32::round`
//! semantics) with saturation to `i16::MIN..=i16::MAX` — properties the
//! unit tests pin so the choice is explicit and revisitable once the
//! §3.2 entropy GAP lands and bit-exact comparison becomes possible.
//!
//! ## Wall-respect note
//!
//! Byte width, endianness and interleave arise from the validator's
//! byte accounting; the rounding convention is documented as a choice,
//! not claimed as a binary fact.

use crate::Error;

/// Convert one float sample to a 16-bit PCM sample: round to nearest
/// (ties away from zero) and saturate to the `i16` range.
///
/// The float is interpreted at PCM scale (a full-scale sample is
/// `±32767.0`); values beyond the range clamp.
pub fn f32_to_i16_sample(x: f32) -> i16 {
    let r = x.round();
    if r <= f32::from(i16::MIN) {
        i16::MIN
    } else if r >= f32::from(i16::MAX) {
        i16::MAX
    } else {
        r as i16
    }
}

/// Write float samples as 16-bit little-endian PCM into `out`.
///
/// `out.len()` must equal `samples.len() × `[`crate::PCM_BYTES_PER_SAMPLE`]
/// (= `2` — the validator-pinned byte width).
///
/// # Errors
///
/// Returns [`Error::PcmOutputLengthMismatch`] when the output buffer is
/// not exactly two bytes per sample.
pub fn write_pcm_i16le(samples: &[f32], out: &mut [u8]) -> Result<(), Error> {
    let expected = samples.len() * crate::PCM_BYTES_PER_SAMPLE as usize;
    if out.len() != expected {
        return Err(Error::PcmOutputLengthMismatch {
            got: out.len(),
            expected,
        });
    }
    for (chunk, &s) in out.chunks_exact_mut(2).zip(samples.iter()) {
        chunk.copy_from_slice(&f32_to_i16_sample(s).to_le_bytes());
    }
    Ok(())
}

/// Convert float samples to a fresh 16-bit little-endian PCM byte
/// vector (the allocating form of [`write_pcm_i16le`]).
pub fn pcm_i16le(samples: &[f32]) -> Vec<u8> {
    let mut out = vec![0u8; samples.len() * crate::PCM_BYTES_PER_SAMPLE as usize];
    // Cannot fail: the buffer is sized to the exact budget above.
    write_pcm_i16le(samples, &mut out).expect("buffer sized to samples * 2");
    out
}

/// Interleave two equal-length channel buffers sample-by-sample
/// (`L R L R …`) — the packing under which the validator's per-call
/// `samples × channels × 2` byte budgets account exactly.
///
/// # Errors
///
/// Returns [`Error::InterleaveLengthMismatch`] when the two channels
/// differ in length.
pub fn interleave_stereo(ch0: &[f32], ch1: &[f32]) -> Result<Vec<f32>, Error> {
    if ch0.len() != ch1.len() {
        return Err(Error::InterleaveLengthMismatch {
            ch0: ch0.len(),
            ch1: ch1.len(),
        });
    }
    let mut out = Vec::with_capacity(ch0.len() * 2);
    for (&a, &b) in ch0.iter().zip(ch1.iter()) {
        out.push(a);
        out.push(b);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_conversion_rounds_to_nearest() {
        assert_eq!(f32_to_i16_sample(0.0), 0);
        assert_eq!(f32_to_i16_sample(0.4), 0);
        assert_eq!(f32_to_i16_sample(0.5), 1); // ties away from zero
        assert_eq!(f32_to_i16_sample(-0.5), -1);
        assert_eq!(f32_to_i16_sample(1000.2), 1000);
        assert_eq!(f32_to_i16_sample(-1000.7), -1001);
    }

    #[test]
    fn sample_conversion_saturates() {
        assert_eq!(f32_to_i16_sample(32767.0), 32767);
        assert_eq!(f32_to_i16_sample(32768.0), 32767);
        assert_eq!(f32_to_i16_sample(1.0e9), 32767);
        assert_eq!(f32_to_i16_sample(-32768.0), -32768);
        assert_eq!(f32_to_i16_sample(-32769.0), -32768);
        assert_eq!(f32_to_i16_sample(-1.0e9), -32768);
    }

    #[test]
    fn pcm_bytes_are_little_endian() {
        // 0x0102 = 258.0 → bytes [0x02, 0x01]; -2 → [0xFE, 0xFF].
        let bytes = pcm_i16le(&[258.0, -2.0]);
        assert_eq!(bytes, vec![0x02, 0x01, 0xFE, 0xFF]);
    }

    #[test]
    fn write_validates_two_bytes_per_sample() {
        let samples = [0.0f32; 4];
        let mut short = vec![0u8; 7];
        assert_eq!(
            write_pcm_i16le(&samples, &mut short).unwrap_err(),
            Error::PcmOutputLengthMismatch {
                got: 7,
                expected: 8
            }
        );
        let mut exact = vec![0xAAu8; 8];
        write_pcm_i16le(&samples, &mut exact).unwrap();
        assert!(exact.iter().all(|&b| b == 0), "silence is all-zero bytes");
    }

    #[test]
    fn zero_samples_produce_pcm_silence() {
        // The observe-gate consistency property: float silence converts
        // to the same all-zero bytes the validator observed
        // (validation/04 §4.3).
        let bytes = pcm_i16le(&[0.0f32; 128]);
        assert_eq!(bytes.len(), 256);
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn interleave_alternates_channels() {
        let l = [1.0f32, 3.0, 5.0];
        let r = [2.0f32, 4.0, 6.0];
        assert_eq!(
            interleave_stereo(&l, &r).unwrap(),
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
        );
    }

    #[test]
    fn interleave_rejects_length_mismatch() {
        assert_eq!(
            interleave_stereo(&[1.0], &[1.0, 2.0]).unwrap_err(),
            Error::InterleaveLengthMismatch { ch0: 1, ch1: 2 }
        );
    }

    #[test]
    fn frame_pcm_budget_matches_validated_geometry() {
        // One 1024-sample stereo frame (the validated stream's geometry)
        // packs to 1024 × 2 × 2 = 4096 bytes — the per-frame slice of
        // the validator's 20 480-byte five-frame call budget.
        let l = vec![0.25f32; 1024];
        let r = vec![-0.25f32; 1024];
        let inter = interleave_stereo(&l, &r).unwrap();
        let bytes = pcm_i16le(&inter);
        assert_eq!(bytes.len(), 4096);
        assert_eq!(bytes.len() as u32 * 5, 20_480);
    }
}
