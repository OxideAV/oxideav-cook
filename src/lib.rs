// Parallel-array index loops are idiomatic in codec code.
#![allow(clippy::needless_range_loop)]
// The cplscale ladders are transcribed verbatim from the clean-room VLC
// tables doc — keep their full decimal precision so the matrix decouple
// matches the documented values exactly.
#![allow(clippy::excessive_precision)]

//! Pure-Rust **RealAudio Cook** (RealNetworks G2 / "Cooker") audio
//! decoder for the [oxideav](https://github.com/OxideAV/oxideav-workspace)
//! framework.
//!
//! Cook is the audio codec used inside RealMedia (`*.rm` / `*.ra` /
//! `*.rmvb`) containers. It is a sub-band MDCT-based lossy codec with
//! seven Huffman scalar-quantised vector tables, a per-band
//! differentially-coded scale-factor envelope, joint-stereo coupling
//! via a unit-norm matrix, and a per-slot exponential gain ramp
//! applied after the lapped iMDCT.
//!
//! This crate implements the full per-subpacket decode pipeline
//! documented in [`docs/audio/cook/cook-trace-reverse-engineering.md`]
//! (../../docs/audio/cook/cook-trace-reverse-engineering.md):
//!
//! 1. XOR descrambler (constant `0x37C511F2`, byte-rotated by
//!    word-alignment).
//! 2. Gain-profile RLE bits (8 slots × 4-bit signed gain).
//! 3. Differential scale-factor envelope (13 envelope Huffman tables).
//! 4. Bit-budget category bisection (8-element `expbits_tab`).
//! 5. Per-band SQVH residual decode (7 Huffman tables, base-(kmax+1)
//!    digit packing).
//! 6. Scalar dequantisation with category-specific dither.
//! 7. (joint stereo) Unit-norm matrix decoupling (5 cplscale ladders,
//!    51-entry cplband map).
//! 8. Per-channel sine-windowed iMDCT + overlap-add lapping.
//! 9. Saturate to [-1.0, +1.0].
//!
//! ## Modes
//!
//! | cookversion  | mode          | extradata size | channels |
//! |--------------|---------------|---------------:|---------:|
//! | `0x01000001` | MONO          | 8 B            | 1        |
//! | `0x01000002` | STEREO        | 8 B            | 2        |
//! | `0x01000003` | JOINT_STEREO  | 16 B           | 2        |
//! | `0x02000000` | MULTI_CHANNEL | 80 B (4×20 B)  | 6 / 8    |
//!
//! Sample rates observed in the wild: 8 kHz / 22.05 kHz / 44.1 kHz.
//! Per-channel iMDCT lengths: 256, 512, 1024.

pub mod categorise;
pub mod codec;
pub mod decoder;
pub mod extradata;
pub mod lfg;
pub mod mdct;
pub mod tables;
pub mod vlc;
pub mod vlc_tables;
pub mod xor;

use oxideav_core::CodecRegistry;

/// Codec id string. The RealMedia container tag is "cook" (LE
/// four-cc), matching this id.
pub const CODEC_ID_STR: &str = "cook";

/// Register cook in the given codec registry.
pub fn register_codecs(reg: &mut CodecRegistry) {
    codec::register(reg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_decoder() {
        let mut reg = CodecRegistry::new();
        register_codecs(&mut reg);
        let cid = oxideav_core::CodecId::new(CODEC_ID_STR);
        assert!(reg.has_decoder(&cid));
    }
}
