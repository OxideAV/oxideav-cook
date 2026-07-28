//! End-to-end spectral entropy → reconstructed spectrum, given the
//! §2.2 per-band category assignment as the single remaining bit-stream
//! GAP input.
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §2–§4 and
//! `docs/audio/cook/provenance/07-cook-spectral-decode.md` items 1–3.
//!
//! ## What this module wires
//!
//! With the round-7/8 staging in hand — the codebook-by-category
//! selection, the magnitude+out-of-band-sign convention, the `0x8fc8`
//! expectation reconstruction, and (for stereo) the recovered §4.3
//! coupling table — the entropy read of a whole frame is now assembled:
//! [`decode_spectrum`] walks the §2.1 subband geometry and, per band,
//! runs the pinned §3 decode ([`crate::spectral_decode::decode_band_coefficients`])
//! straight into the band's coefficient range, producing one channel's
//! reconstructed iMDCT-input spectrum from the raw bitstream. For stereo
//! [`decode_frame_spectrum`] decodes the single coupled spectrum and
//! splits it by the recovered §4.3 mirror rotation
//! ([`crate::reconstruct::decouple_stereo_recovered`]).
//!
//! ## The category-assignment routing piece (now wired)
//!
//! The **per-band category assignment** — which category (0..6, or the
//! empty-band 7) the in-decoder bit-allocation loop `cook.dll!0x4800`
//! assigns to each subband — was the single remaining routing GAP:
//! `spec/05` §2.2 and `provenance/07` item 1 pinned the loop's cost LUT
//! (`0x8f38`) and its round bounds, but not the assignment itself.
//! `provenance/08` (`§2.2`) recovers it: categories are **not
//! transmitted** — they are **computed** from a per-band value array and
//! a bit budget by the base pass `cat[b] = clip((32 + off − v[b]) >> 1,
//! 0, 7)` (offset chosen to best-match the budget) plus a Stage-2 ±1
//! refinement. That loop is [`crate::category_assignment`]; this module
//! consumes it. [`decode_spectrum`] still takes an explicit
//! [`BandCategory`] list (so the exact wire values remain a caller input
//! where a real frame's `v[]`/budget are not captured), and
//! [`decode_spectrum_assigned`] computes that list from `(v[], budget,
//! refinement_bound)` through the recovered §2.2 pass and decodes in one
//! call. Everything downstream of the assignment (the entropy read, the
//! reconstruction, the coupling split, the synthesis) is wired
//! bit-exactly.
//!
//! The gain-envelope **per-segment records** (§1.2, VLC-gated by an
//! unpinned codebook) and the §4.1 **coupling-band boundary** derivation
//! are likewise recorded gaps; this module takes a flat gain and a
//! caller-supplied coupling-band range / index list respectively.
//!
//! ## Wall-respect note
//!
//! Every step composes an independently-pinned primitive; the only
//! inputs the trace does not pin (the category list, the coupling-band
//! range + indices, the per-band gains) are explicit function
//! parameters, never fabricated.

use crate::{
    bitreader::FrameBitReader,
    category_assignment::assign_band_categories,
    reconstruct::{decouple_stereo_recovered, StereoSpectra},
    spectral_decode::{decode_band_coefficients, BandCategory},
    subband::SubbandGeometry,
    Error,
};

/// Decode one channel's spectral entropy section straight to its
/// reconstructed iMDCT-input spectrum.
///
/// `reader` is positioned at the start of the §3 spectral data;
/// `geometry` is the stream's §2.1 subband map; `categories` carries one
/// [`BandCategory`] per subband (the §2.2 assignment — a caller GAP
/// input, see the module docs); `band_gains` carries one per-band gain
/// (the §1–§2 scale) per subband, or a single-element `&[g]` broadcast
/// to every band, or `&[1.0]` for unity.
///
/// Each band decodes `line_count` coefficients through its
/// category-selected codebook (empty-band category 7 emits zeros and
/// reads nothing) and fills the band's coefficient range; the result is
/// `geometry.total_coded_lines()` long.
///
/// # Errors
///
/// - [`Error::SpectrumBandCountMismatch`] when `categories.len()` is not
///   the subband count.
/// - [`Error::GainProfileCountMismatch`] when `band_gains` is neither
///   length 1 nor the subband count.
/// - any error [`decode_band_coefficients`] raises (VLC no-match,
///   expectation range).
pub fn decode_spectrum(
    reader: &mut FrameBitReader,
    geometry: &SubbandGeometry,
    categories: &[BandCategory],
    band_gains: &[f32],
) -> Result<Vec<f32>, Error> {
    let subband_count = geometry.subband_count();
    if categories.len() as u32 != subband_count {
        return Err(Error::SpectrumBandCountMismatch {
            subband_count,
            got: categories.len(),
        });
    }
    if band_gains.len() != 1 && band_gains.len() as u32 != subband_count {
        return Err(Error::GainProfileCountMismatch {
            got: band_gains.len(),
            expected: subband_count as usize,
        });
    }
    let total = geometry.total_coded_lines() as usize;
    let mut spectrum = vec![0.0f32; total];
    for band in 0..subband_count {
        let range = geometry.line_range(band)?;
        let line_count = range.end - range.start;
        let gain = if band_gains.len() == 1 {
            band_gains[0]
        } else {
            band_gains[band as usize]
        };
        let coeffs = decode_band_coefficients(reader, categories[band as usize], line_count, gain)?;
        spectrum[range.start as usize..range.end as usize].copy_from_slice(&coeffs);
    }
    Ok(spectrum)
}

/// Decode one channel's spectral entropy section, **computing** the
/// per-band category assignment from the §2.2 bit-allocation pass rather
/// than taking it as a caller input.
///
/// `values` is the per-band value array `v[]` (one per subband — the
/// band priority / envelope index the §2 quant walk yields); `budget` is
/// the frame bit budget and `refinement_bound` is the Stage-2 pass bound
/// (`M`, decode-state `+0x28`). The recovered §2.2 pass
/// ([`assign_band_categories`]) turns `(values, budget, refinement_bound)`
/// into the per-band [`BandCategory`] list, which then routes through the
/// same codebook-by-category §3 decode as [`decode_spectrum`].
///
/// This is the routing bridge between the quantiser indices and the
/// spectral entropy read; see the module docs for the exact algorithm and
/// its validated regime.
///
/// # Errors
///
/// - [`Error::SpectrumBandCountMismatch`] when `values.len()` is not the
///   subband count.
/// - any error [`decode_spectrum`] raises.
pub fn decode_spectrum_assigned(
    reader: &mut FrameBitReader,
    geometry: &SubbandGeometry,
    values: &[i32],
    budget: i32,
    refinement_bound: u32,
    band_gains: &[f32],
) -> Result<Vec<f32>, Error> {
    let subband_count = geometry.subband_count();
    if values.len() as u32 != subband_count {
        return Err(Error::SpectrumBandCountMismatch {
            subband_count,
            got: values.len(),
        });
    }
    let categories = assign_band_categories(values, budget, refinement_bound)?;
    decode_spectrum(reader, geometry, &categories, band_gains)
}

/// The coupling inputs [`decode_frame_spectrum`] consumes for a stereo
/// frame — the §4.1 coupling-band range and one rotation index per band.
///
/// Both are caller inputs: §4.1's coupling-band **boundary derivation**
/// has no closed form (a recorded gap), and the per-band rotation
/// indices are read through the coupling control (fixed-width or
/// VLC-gated — [`crate::coupling_control`]). The rotation **coefficient
/// values** are the recovered §4.3 table
/// ([`crate::coupling::coupling_coefficient_table`]), no longer a GAP.
#[derive(Debug, Clone)]
pub struct FrameCoupling<'a> {
    /// Contiguous coupling-band subband range `[first..last)` (§4.1).
    pub coupling_bands: core::ops::Range<u32>,
    /// One rotation index `j` per coupling band (§4.1).
    pub indices: &'a [u32],
}

/// One decoded frame's per-channel reconstructed spectrum.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodedSpectrum {
    /// Mono — one reconstructed spectrum.
    Mono(Vec<f32>),
    /// Stereo — the §4 decouple of the single coded coupled spectrum.
    Stereo(StereoSpectra),
}

/// Decode one frame's spectral section to a per-channel spectrum,
/// routing mono vs stereo through the recovered §4.3 coupling split.
///
/// - `channels == 1`: [`DecodedSpectrum::Mono`] of [`decode_spectrum`].
/// - `channels == 2`: decode the single coupled spectrum, then split it
///   with [`decouple_stereo_recovered`] over `coupling` (required).
///
/// # Errors
///
/// - [`Error::CookieInvalidChannels`] for a channel count other than 1/2.
/// - [`Error::StereoCouplingMissing`] when `channels == 2` and
///   `coupling` is `None`.
/// - any error [`decode_spectrum`] / [`decouple_stereo_recovered`] raises.
pub fn decode_frame_spectrum(
    reader: &mut FrameBitReader,
    geometry: &SubbandGeometry,
    categories: &[BandCategory],
    band_gains: &[f32],
    channels: u16,
    coupling: Option<&FrameCoupling<'_>>,
) -> Result<DecodedSpectrum, Error> {
    let coupled = decode_spectrum(reader, geometry, categories, band_gains)?;
    match channels {
        1 => Ok(DecodedSpectrum::Mono(coupled)),
        2 => {
            let c = coupling.ok_or(Error::StereoCouplingMissing)?;
            let stereo =
                decouple_stereo_recovered(&coupled, geometry, c.coupling_bands.clone(), c.indices)?;
            Ok(DecodedSpectrum::Stereo(stereo))
        }
        other => Err(Error::CookieInvalidChannels { got: other }),
    }
}

/// Decode one frame's spectral section to a per-channel spectrum,
/// **computing** the §2.2 per-band category assignment from the
/// bit-allocation pass rather than taking an explicit category list.
///
/// This is [`decode_frame_spectrum`] with the category list replaced by
/// the recovered §2.2 assignment ([`assign_band_categories`] over
/// `(values, budget, refinement_bound)`): the routing bridge that threads
/// the in-decoder bit-allocation loop through the mono/stereo split in a
/// single call. It is the value-array analog of [`decode_frame_spectrum`]
/// and the mono-plus-stereo analog of [`decode_spectrum_assigned`].
///
/// - `channels == 1`: [`DecodedSpectrum::Mono`] of the assigned decode.
/// - `channels == 2`: decode the single coupled spectrum, then split it
///   with [`decouple_stereo_recovered`] over `coupling` (required).
///
/// `values`, `budget`, `refinement_bound` and (for stereo) the coupling
/// band range + indices are the genuinely-unpinned §2.2/§4.1 inputs
/// supplied by the caller; everything else is the format's own pinned
/// arithmetic (see the module docs).
///
/// # Errors
///
/// - [`Error::SpectrumBandCountMismatch`] when `values.len()` is not the
///   subband count.
/// - [`Error::CookieInvalidChannels`] for a channel count other than 1/2.
/// - [`Error::StereoCouplingMissing`] when `channels == 2` and `coupling`
///   is `None`.
/// - any error [`decode_frame_spectrum`] / [`assign_band_categories`]
///   raises.
// Mirrors `decode_frame_spectrum`'s parameter list plus the three §2.2
// bit-allocation scalars (`values` / `budget` / `refinement_bound`); the
// inputs are the frame's own decode arguments, not incidental config.
#[allow(clippy::too_many_arguments)]
pub fn decode_frame_spectrum_assigned(
    reader: &mut FrameBitReader,
    geometry: &SubbandGeometry,
    values: &[i32],
    budget: i32,
    refinement_bound: u32,
    band_gains: &[f32],
    channels: u16,
    coupling: Option<&FrameCoupling<'_>>,
) -> Result<DecodedSpectrum, Error> {
    let subband_count = geometry.subband_count();
    if values.len() as u32 != subband_count {
        return Err(Error::SpectrumBandCountMismatch {
            subband_count,
            got: values.len(),
        });
    }
    let categories = assign_band_categories(values, budget, refinement_bound)?;
    decode_frame_spectrum(
        reader,
        geometry,
        &categories,
        band_gains,
        channels,
        coupling,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::category::CategoryIndex;
    use crate::codebook::spectral_huffman;
    use crate::expectation::dequantise_level;
    use crate::spectral::SpectralCodebook;
    use crate::spectral_decode::{codebook_for_category, compose_symbol};

    fn cat(c: u8) -> CategoryIndex {
        CategoryIndex::new(c).unwrap()
    }

    /// Pack `(value, nbits)` fields MSB-first into a byte buffer.
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

    // Encode one cb6 band (dim 5) of `line_count` lines from explicit
    // digits+signs, appending the fields to `fields`.
    fn encode_cb6_band(fields: &mut Vec<(u32, u32)>, digits: &[u32], signs: &[u32]) {
        let category = cat(6);
        let huffman = spectral_huffman(SpectralCodebook::new(6).unwrap());
        let mut sign_iter = signs.iter();
        for chunk in digits.chunks(5) {
            let mut padded = chunk.to_vec();
            padded.resize(5, 0);
            let symbol = compose_symbol(&padded, category).unwrap();
            fields.push(huffman.codeword(symbol).unwrap());
            for &d in &padded {
                if d != 0 {
                    fields.push((*sign_iter.next().unwrap_or(&0), 1));
                }
            }
        }
    }

    #[test]
    fn decode_spectrum_rejects_wrong_category_count() {
        let geom = SubbandGeometry::new(12).unwrap();
        let bytes = [0u8; 8];
        let mut reader = FrameBitReader::new(&bytes);
        let cats = vec![BandCategory::Empty; 11];
        assert_eq!(
            decode_spectrum(&mut reader, &geom, &cats, &[1.0]).unwrap_err(),
            Error::SpectrumBandCountMismatch {
                subband_count: 12,
                got: 11
            }
        );
    }

    #[test]
    fn all_empty_bands_decode_to_silence_reading_nothing() {
        let geom = SubbandGeometry::new(20).unwrap();
        let bytes = [0xffu8; 16];
        let mut reader = FrameBitReader::new(&bytes);
        let cats = vec![BandCategory::Empty; 20];
        let spectrum = decode_spectrum(&mut reader, &geom, &cats, &[1.0]).unwrap();
        assert_eq!(spectrum.len(), geom.total_coded_lines() as usize);
        assert!(spectrum.iter().all(|&v| v == 0.0));
        assert_eq!(reader.bit_cursor(), 0, "empty bands read nothing");
    }

    #[test]
    fn mono_frame_decodes_the_pinned_coefficients() {
        // First 12 subbands are identity (1 line each); decode them with
        // cb6 and the rest empty. Because each band is 1 line, dim-5 cb6
        // over-covers — but decode_band_coefficients truncates to the
        // band's line_count (1), so each band consumes one cb6 vector +
        // its sign bits.
        let geom = SubbandGeometry::new(20).unwrap();
        let sub = geom.subband_count();
        let mut cats = vec![BandCategory::Empty; sub as usize];
        // Bands 0..4 coded (cb6), each 1 line wide.
        let mut fields = Vec::new();
        // Per band a single cb6 vector [1,0,0,0,0] with one sign bit.
        let per_band_digits = [1u32, 0, 0, 0, 0];
        for band in 0..4u32 {
            cats[band as usize] = BandCategory::Coded(cat(6));
            let sign = band & 1; // alternate signs
            encode_cb6_band(&mut fields, &per_band_digits, &[sign]);
        }
        let bytes = pack(&fields);
        let mut reader = FrameBitReader::new(&bytes);
        let gain = 4.0f32;
        let spectrum = decode_spectrum(&mut reader, &geom, &cats, &[gain]).unwrap();

        // Each coded band's single coefficient equals the reconstruction
        // of level=1 with its sign, at gain 4.
        for band in 0..4u32 {
            let r = geom.line_range(band).unwrap();
            let sign = band & 1;
            let want = dequantise_level(cat(6), 1, sign, gain).unwrap();
            assert_eq!(spectrum[r.start as usize], want, "band {band}");
        }
        // The empty bands stay silent.
        let r5 = geom.line_range(5).unwrap();
        assert_eq!(spectrum[r5.start as usize], 0.0);
        // The decode carried real, non-silent energy.
        let energy: f32 = spectrum.iter().map(|v| v * v).sum();
        assert!(energy > 0.0, "decoded spectrum must carry energy");
    }

    #[test]
    fn stereo_frame_routes_through_recovered_coupling() {
        let geom = SubbandGeometry::new(20).unwrap();
        let sub = geom.subband_count();
        let mut cats = vec![BandCategory::Empty; sub as usize];
        let mut fields = Vec::new();
        let digits = [1u32, 0, 0, 0, 0];
        for band in 0..6u32 {
            cats[band as usize] = BandCategory::Coded(cat(6));
            encode_cb6_band(&mut fields, &digits, &[0]);
        }
        let bytes = pack(&fields);
        let mut reader = FrameBitReader::new(&bytes);

        // Couple bands [2..5), index 0 (full channel-0 steer for the
        // recovered table: coef[0]=1, coef[Ncoup-1]≈0).
        let indices = [0u32, 0, 0];
        let coupling = FrameCoupling {
            coupling_bands: 2..5,
            indices: &indices,
        };
        let out =
            decode_frame_spectrum(&mut reader, &geom, &cats, &[2.0], 2, Some(&coupling)).unwrap();
        match out {
            DecodedSpectrum::Stereo(s) => {
                let total = geom.total_coded_lines() as usize;
                assert_eq!(s.ch0.len(), total);
                assert_eq!(s.ch1.len(), total);
                // Band 2 line: index 0 → ch0 carries the coupled value,
                // ch1 ≈ 0 (recovered coef[511] ≈ 0).
                let r2 = geom.line_range(2).unwrap();
                let coupled_val = dequantise_level(cat(6), 1, 0, 2.0).unwrap();
                let (a, b) = crate::coupling::coupling_pan_pair(0).unwrap();
                assert!((s.ch0[r2.start as usize] - coupled_val * a).abs() < 1e-4);
                assert!((s.ch1[r2.start as usize] - coupled_val * b).abs() < 1e-4);
            }
            other => panic!("expected stereo, got {other:?}"),
        }
    }

    #[test]
    fn stereo_without_coupling_is_rejected() {
        let geom = SubbandGeometry::new(20).unwrap();
        let cats = vec![BandCategory::Empty; 20];
        let bytes = [0u8; 8];
        let mut reader = FrameBitReader::new(&bytes);
        assert_eq!(
            decode_frame_spectrum(&mut reader, &geom, &cats, &[1.0], 2, None).unwrap_err(),
            Error::StereoCouplingMissing
        );
    }

    #[test]
    fn assigned_decode_computes_the_category_list() {
        use crate::category_assignment::assign_base_categories;
        // A tiny-budget frame: the §2.2 base pass assigns the empty band
        // to every subband, so the assigned decode reads nothing and is
        // silent — end-to-end through the recovered assignment loop.
        let geom = SubbandGeometry::new(8).unwrap();
        let values = vec![0i32; 8];
        // budget <= K (32) -> all category 7 (empty).
        assert!(assign_base_categories(&values, 20).iter().all(|&c| c == 7));
        let bytes = [0xffu8; 16];
        let mut reader = FrameBitReader::new(&bytes);
        let spectrum =
            decode_spectrum_assigned(&mut reader, &geom, &values, 20, 1, &[1.0]).unwrap();
        assert!(spectrum.iter().all(|&v| v == 0.0));
        assert_eq!(reader.bit_cursor(), 0, "all-empty assignment reads nothing");
    }

    #[test]
    fn assigned_decode_matches_explicit_categories() {
        use crate::category_assignment::assign_band_categories;
        // Choose (v, budget) that assign a coded mix, encode a bitstream
        // matching that computed category list, and confirm the assigned
        // path equals the explicit-category path bit for bit.
        let geom = SubbandGeometry::new(8).unwrap();
        let values = vec![0i32; 8];
        let budget = 500; // base pass -> all category 0 (finest, cost 52).
        let cats = assign_band_categories(&values, budget, 1).unwrap();
        assert!(cats.iter().all(|c| matches!(c, BandCategory::Coded(_))));

        // Encode one cb0-vector per band matching the computed categories.
        let mut fields = Vec::new();
        for (band, c) in cats.iter().enumerate() {
            if let BandCategory::Coded(ci) = c {
                let cb = codebook_for_category(*ci);
                let huffman = spectral_huffman(cb);
                let dim = crate::spectral::category_vector_dims(*ci).lo;
                // A single non-zero leading digit + zeros, one sign bit.
                let mut digits = vec![0u32; dim as usize];
                digits[0] = 1 + (band as u32 & 1);
                let symbol = compose_symbol(&digits, *ci).unwrap();
                fields.push(huffman.codeword(symbol).unwrap());
                fields.push((band as u32 & 1, 1)); // sign for the non-zero digit
            }
        }
        let bytes = pack(&fields);

        let mut reader_a = FrameBitReader::new(&bytes);
        let via_assigned =
            decode_spectrum_assigned(&mut reader_a, &geom, &values, budget, 1, &[2.0]).unwrap();
        let mut reader_b = FrameBitReader::new(&bytes);
        let via_explicit = decode_spectrum(&mut reader_b, &geom, &cats, &[2.0]).unwrap();
        assert_eq!(via_assigned, via_explicit);
        assert_eq!(reader_a.bit_cursor(), reader_b.bit_cursor());
        assert!(via_assigned.iter().any(|&v| v != 0.0), "coded energy");
    }

    #[test]
    fn assigned_decode_rejects_wrong_value_count() {
        let geom = SubbandGeometry::new(8).unwrap();
        let bytes = [0u8; 8];
        let mut reader = FrameBitReader::new(&bytes);
        assert_eq!(
            decode_spectrum_assigned(&mut reader, &geom, &[0i32; 7], 100, 1, &[1.0]).unwrap_err(),
            Error::SpectrumBandCountMismatch {
                subband_count: 8,
                got: 7
            }
        );
    }

    #[test]
    fn assigned_frame_spectrum_mono_matches_two_step() {
        // The one-call value-array path equals computing the category list
        // then routing it through decode_frame_spectrum — mono.
        use crate::category_assignment::assign_band_categories;
        let geom = SubbandGeometry::new(8).unwrap();
        let values = vec![0i32; 8];
        let budget = 500; // all-coded finest category.
        let cats = assign_band_categories(&values, budget, 1).unwrap();
        let mut fields = Vec::new();
        for (band, c) in cats.iter().enumerate() {
            if let BandCategory::Coded(ci) = c {
                let cb = codebook_for_category(*ci);
                let huffman = spectral_huffman(cb);
                let dim = crate::spectral::category_vector_dims(*ci).lo as usize;
                let mut digits = vec![0u32; dim];
                digits[0] = 1 + (band as u32 & 1);
                let symbol = compose_symbol(&digits, *ci).unwrap();
                fields.push(huffman.codeword(symbol).unwrap());
                fields.push((band as u32 & 1, 1));
            }
        }
        let bytes = pack(&fields);

        let mut ra = FrameBitReader::new(&bytes);
        let via_assigned =
            decode_frame_spectrum_assigned(&mut ra, &geom, &values, budget, 1, &[2.0], 1, None)
                .unwrap();
        let mut rb = FrameBitReader::new(&bytes);
        let via_explicit = decode_frame_spectrum(&mut rb, &geom, &cats, &[2.0], 1, None).unwrap();
        assert_eq!(via_assigned, via_explicit);
        assert!(matches!(via_assigned, DecodedSpectrum::Mono(_)));
    }

    #[test]
    fn assigned_frame_spectrum_stereo_routes_through_coupling() {
        use crate::category_assignment::assign_band_categories;
        let geom = SubbandGeometry::new(8).unwrap();
        let values = vec![0i32; 8];
        let budget = 500;
        let cats = assign_band_categories(&values, budget, 1).unwrap();
        let mut fields = Vec::new();
        for (band, c) in cats.iter().enumerate() {
            if let BandCategory::Coded(ci) = c {
                let huffman = spectral_huffman(codebook_for_category(*ci));
                let dim = crate::spectral::category_vector_dims(*ci).lo as usize;
                let mut digits = vec![0u32; dim];
                digits[0] = 1;
                let symbol = compose_symbol(&digits, *ci).unwrap();
                fields.push(huffman.codeword(symbol).unwrap());
                fields.push((band as u32 & 1, 1));
            }
        }
        let bytes = pack(&fields);
        let indices = [0u32, 0, 0];
        let coupling = FrameCoupling {
            coupling_bands: 2..5,
            indices: &indices,
        };
        let mut ra = FrameBitReader::new(&bytes);
        let via_assigned = decode_frame_spectrum_assigned(
            &mut ra,
            &geom,
            &values,
            budget,
            1,
            &[2.0],
            2,
            Some(&coupling),
        )
        .unwrap();
        let mut rb = FrameBitReader::new(&bytes);
        let via_explicit =
            decode_frame_spectrum(&mut rb, &geom, &cats, &[2.0], 2, Some(&coupling)).unwrap();
        assert_eq!(via_assigned, via_explicit);
        assert!(matches!(via_assigned, DecodedSpectrum::Stereo(_)));
    }

    #[test]
    fn assigned_frame_spectrum_rejects_wrong_value_count() {
        let geom = SubbandGeometry::new(8).unwrap();
        let bytes = [0u8; 8];
        let mut reader = FrameBitReader::new(&bytes);
        assert_eq!(
            decode_frame_spectrum_assigned(&mut reader, &geom, &[0i32; 7], 100, 1, &[1.0], 1, None)
                .unwrap_err(),
            Error::SpectrumBandCountMismatch {
                subband_count: 8,
                got: 7
            }
        );
    }

    #[test]
    fn codebook_selection_is_the_category_end_to_end() {
        // A band coded with category c must decode through codebook c —
        // pin the identity by decoding one cb3 (dim 4) band and checking
        // it round-trips.
        let geom = SubbandGeometry::new(12).unwrap();
        let sub = geom.subband_count();
        let mut cats = vec![BandCategory::Empty; sub as usize];
        cats[0] = BandCategory::Coded(cat(3));
        assert_eq!(codebook_for_category(cat(3)).get(), 3);
        // Band 0 is 1 line; cb3 dim 4 → one vector, truncated to 1 line.
        let huffman = spectral_huffman(SpectralCodebook::new(3).unwrap());
        // digits [2,0,0,0] → one non-zero sign bit.
        let symbol = compose_symbol(&[2, 0, 0, 0], cat(3)).unwrap();
        let mut fields = vec![huffman.codeword(symbol).unwrap()];
        fields.push((0, 1)); // sign for the single non-zero digit
        let bytes = pack(&fields);
        let mut reader = FrameBitReader::new(&bytes);
        let spectrum = decode_spectrum(&mut reader, &geom, &cats, &[1.0]).unwrap();
        let r0 = geom.line_range(0).unwrap();
        let want = dequantise_level(cat(3), 2, 0, 1.0).unwrap();
        assert_eq!(spectrum[r0.start as usize], want);
    }
}
