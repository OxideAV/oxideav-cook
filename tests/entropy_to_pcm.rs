//! End-to-end: a Cook-format spectral entropy bitstream → reconstructed
//! spectrum → 16-bit PCM through the runtime-recovered N=1024 window.
//!
//! This exercises the whole now-staged decode chain over the public API:
//! the codebook-by-category §3 entropy read (`decode_spectrum`), the
//! `0x8fc8` expectation reconstruction, and the §5 synthesis through the
//! runtime-recovered long-transform window
//! (`Synthesizer::with_recovered_long_window`). It produces **real,
//! energy-bearing, audible PCM** from a Cook-format bitstream.
//!
//! The §2.2 per-band **category assignment** (the in-decoder
//! bit-allocation loop `cook.dll!0x4800`) is now recovered:
//! `computed_categories_decode_to_audible_pcm` drives the whole chain
//! from `(value array, bit budget, refinement bound)` through the
//! recovered assignment pass (`decode_spectrum_assigned`) to PCM, while
//! `cook_entropy_bitstream_decodes_to_audible_pcm` supplies an explicit
//! category list (the path a caller uses when it already has the per-band
//! categories). Everything downstream of the assignment is the format's
//! own pinned arithmetic and recovered tables.

use oxideav_cook::{
    assign_band_categories, category_vector_dims, codebook_for_category, compose_symbol,
    decode_spectrum, decode_spectrum_assigned, f32_to_i16_sample, spectral_huffman, BandCategory,
    CategoryIndex, FrameBitReader, SpectralCodebook, SubbandGeometry, Synthesizer,
};

/// Pack `(value, nbits)` fields MSB-first into a byte buffer (the frame
/// bit reader consumes big-endian 32-bit words, MSB-first).
fn pack(fields: &[(u32, u32)]) -> Vec<u8> {
    let mut bits: Vec<u8> = Vec::new();
    for &(v, n) in fields {
        for b in (0..n).rev() {
            bits.push(((v >> b) & 1) as u8);
        }
    }
    while bits.len() % 32 != 0 {
        bits.push(0);
    }
    let mut bytes = vec![0u8; bits.len() / 8];
    for (i, &bit) in bits.iter().enumerate() {
        if bit != 0 {
            bytes[i / 8] |= 0x80 >> (i % 8);
        }
    }
    bytes
}

/// Encode one cb`c`-vector of explicit magnitude digits + signs.
fn encode_vector(fields: &mut Vec<(u32, u32)>, c: u8, digits: &[u32], signs: &[u32]) {
    let category = CategoryIndex::new(c).unwrap();
    let huffman = spectral_huffman(SpectralCodebook::new(c).unwrap());
    let symbol = compose_symbol(digits, category).unwrap();
    fields.push(huffman.codeword(symbol).unwrap());
    let mut sign_iter = signs.iter();
    for &d in digits {
        if d != 0 {
            fields.push((*sign_iter.next().unwrap(), 1));
        }
    }
}

#[test]
fn cook_entropy_bitstream_decodes_to_audible_pcm() {
    // A 12-subband stream (the identity run: bands 0..12 are one line
    // each). Code the first eight bands, each 1 line wide, with cb6
    // (dim 5, radix 2): one vector per band carrying a leading non-zero
    // magnitude so every band lands a real coefficient.
    let subband_count = 12u32;
    let geom = SubbandGeometry::new(subband_count).unwrap();
    let mut cats = vec![BandCategory::Empty; subband_count as usize];

    let mut fields = Vec::new();
    for band in 0..8u32 {
        cats[band as usize] = BandCategory::Coded(CategoryIndex::new(6).unwrap());
        // codebook == category identity, checked once.
        assert_eq!(
            codebook_for_category(CategoryIndex::new(6).unwrap()).get(),
            6
        );
        // digit pattern varies per band so the spectrum is not flat.
        let lead = 1u32;
        let digits = [lead, (band & 1), 0, 0, 0];
        let signs = [((band >> 1) & 1), (band & 1)];
        encode_vector(
            &mut fields,
            6,
            &digits,
            &signs[..digits.iter().filter(|&&d| d != 0).count()],
        );
    }
    let bytes = pack(&fields);

    // §3 entropy read → reconstructed coded spectrum (band gain 8.0 to
    // lift the coefficients into an audible range).
    let mut reader = FrameBitReader::new(&bytes);
    let coded = decode_spectrum(&mut reader, &geom, &cats, &[8.0]).unwrap();
    assert_eq!(coded.len(), geom.total_coded_lines() as usize);

    let spectral_energy: f32 = coded.iter().map(|v| v * v).sum();
    assert!(
        spectral_energy > 0.0,
        "the entropy read produced a silent spectrum"
    );

    // §5 synthesis through the runtime-recovered N=1024 window (hop 512).
    // Zero-pad the coded spectrum up to the hop, push one frame, and
    // read the emitted PCM.
    let mut synth = Synthesizer::with_recovered_long_window();
    let hop = synth.hop();
    assert_eq!(hop, 512);
    let mut full = vec![0.0f32; hop];
    full[..coded.len()].copy_from_slice(&coded);

    // Warm-up frame (zeroed tail) then a repeat: the second frame's
    // output carries both contributions and is the steady-state PCM.
    let _warmup = synth.push_spectrum(&full).unwrap();
    let samples = synth.push_spectrum(&full).unwrap();
    assert_eq!(samples.len(), hop);

    // The PCM is finite and NON-silent — real audible energy came out
    // the far end of the decode chain.
    assert!(samples.iter().all(|s| s.is_finite()));
    let pcm_energy: f32 = samples.iter().map(|s| s * s).sum();
    assert!(
        pcm_energy > 1e-6,
        "synthesis produced silent PCM (energy {pcm_energy})"
    );

    // Convert to the validator-pinned 16-bit LE format. This crate's
    // `imlt` is unit-normalised (samples in ~[-1, 1]); the vendor's
    // absolute output gain is part of the recorded
    // normalisation-convention caveat, so scale to full PCM range
    // (±32767) before quantising — the point here is that real,
    // structured audio comes out, not a specific absolute level.
    let peak = samples.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
    assert!(
        peak > 0.05,
        "synthesised frame is near-silent (peak {peak})"
    );
    let pcm16: Vec<i16> = samples
        .iter()
        .map(|&s| f32_to_i16_sample(s * 32767.0))
        .collect();
    assert_eq!(pcm16.len(), hop);
    let nonzero = pcm16.iter().filter(|&&s| s != 0).count();
    assert!(
        nonzero > hop / 4,
        "expected a substantially non-silent frame, got {nonzero}/{hop} non-zero samples"
    );

    // Deterministic: re-running the identical decode reproduces the
    // bytes exactly.
    let mut synth2 = Synthesizer::with_recovered_long_window();
    let _ = synth2.push_spectrum(&full).unwrap();
    let samples2 = synth2.push_spectrum(&full).unwrap();
    assert_eq!(samples, samples2);
}

#[test]
fn computed_categories_decode_to_audible_pcm() {
    // The full chain now runs from the §2.2 category-assignment loop:
    // (value array v[], bit budget, refinement bound) -> computed per-band
    // categories -> codebook-by-category entropy read -> reconstructed
    // spectrum -> synthesis -> 16-bit PCM. No category is supplied by the
    // caller; the recovered bit-allocation pass produces them.
    let subband_count = 12u32;
    let geom = SubbandGeometry::new(subband_count).unwrap();

    // A flat value array with a generous budget: the base pass assigns a
    // uniform coded category across every band (no empty bands), so every
    // subband lands a real coefficient.
    let values = vec![0i32; subband_count as usize];
    let budget = 500; // -> uniform finest category (0) for a flat input.
    let cats = assign_band_categories(&values, budget, 1).unwrap();
    assert!(
        cats.iter().all(|c| matches!(c, BandCategory::Coded(_))),
        "expected an all-coded assignment, got {cats:?}"
    );

    // Encode one codebook vector per band matching the computed category.
    let mut fields = Vec::new();
    for (band, c) in cats.iter().enumerate() {
        let BandCategory::Coded(ci) = c else {
            continue;
        };
        let huffman = spectral_huffman(codebook_for_category(*ci));
        let dim = category_vector_dims(*ci).lo as usize;
        let mut digits = vec![0u32; dim];
        digits[0] = 1 + (band as u32 & 1); // varying leading magnitude
        let symbol = compose_symbol(&digits, *ci).unwrap();
        fields.push(huffman.codeword(symbol).unwrap());
        fields.push((band as u32 & 1, 1)); // sign for the single non-zero digit
    }
    let bytes = pack(&fields);

    // §2.2 assignment + §3 entropy read in one call.
    let mut reader = FrameBitReader::new(&bytes);
    let coded = decode_spectrum_assigned(&mut reader, &geom, &values, budget, 1, &[8.0]).unwrap();
    assert_eq!(coded.len(), geom.total_coded_lines() as usize);
    let spectral_energy: f32 = coded.iter().map(|v| v * v).sum();
    assert!(spectral_energy > 0.0, "computed-category read was silent");

    // §5 synthesis to PCM.
    let mut synth = Synthesizer::with_recovered_long_window();
    let hop = synth.hop();
    let mut full = vec![0.0f32; hop];
    full[..coded.len()].copy_from_slice(&coded);
    let _warmup = synth.push_spectrum(&full).unwrap();
    let samples = synth.push_spectrum(&full).unwrap();
    assert_eq!(samples.len(), hop);
    assert!(samples.iter().all(|s| s.is_finite()));
    let pcm_energy: f32 = samples.iter().map(|s| s * s).sum();
    assert!(pcm_energy > 1e-6, "synthesis produced silent PCM");
    let pcm16: Vec<i16> = samples
        .iter()
        .map(|&s| f32_to_i16_sample(s * 32767.0))
        .collect();
    let nonzero = pcm16.iter().filter(|&&s| s != 0).count();
    assert!(
        nonzero > hop / 4,
        "expected a substantially non-silent frame"
    );

    // Determinism: the computed assignment + decode reproduces exactly.
    let mut reader2 = FrameBitReader::new(&bytes);
    let coded2 = decode_spectrum_assigned(&mut reader2, &geom, &values, budget, 1, &[8.0]).unwrap();
    assert_eq!(coded, coded2);
}

#[test]
fn empty_frame_decodes_to_silence() {
    // A frame whose bands are all the empty-band sentinel reads no
    // spectral bits and synthesizes to exact silence — the observe-gate
    // / warm-up consistency property, end to end.
    let geom = SubbandGeometry::new(20).unwrap();
    let cats = vec![BandCategory::Empty; 20];
    let bytes = [0u8; 16];
    let mut reader = FrameBitReader::new(&bytes);
    let coded = decode_spectrum(&mut reader, &geom, &cats, &[8.0]).unwrap();
    assert!(coded.iter().all(|&v| v == 0.0));
    assert_eq!(reader.bit_cursor(), 0);

    let mut synth = Synthesizer::with_recovered_long_window();
    let hop = synth.hop();
    let mut full = vec![0.0f32; hop];
    full[..coded.len()].copy_from_slice(&coded);
    let out = synth.push_spectrum(&full).unwrap();
    assert!(out.iter().all(|&s| s == 0.0));
}
