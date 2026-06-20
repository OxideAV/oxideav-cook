//! Post-entropy spectral reconstruction → inverse-transform input
//! (frame-syntax §2.1 / §2.2 / §3.1 / §4.2 integration).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §2.1 (the
//! subband → coefficient-range axis), §2.2 (the per-band quantiser closed
//! form), §3.1 (the dequant assembly `value * sign * scale * gain` and the
//! per-symbol grouping) and §4.2 (the joint-stereo mirror-index rotation),
//! plus `docs/audio/cook/provenance/05-cook-backend.md` evidence #4 / #7 /
//! #10 / #13.
//!
//! ## Where this sits in the frame body
//!
//! `spec/05` §0 pins the backend per-frame body as four sub-stages — gain
//! control → category/quant walk → spectral VLC dequant → inverse
//! transform — and, for stereo, the coupling split. The *entropy* read of
//! each stage (the VLC walk that recovers the gain-segment records, the
//! per-band quantiser indices, and the spectral symbol values) descends
//! the seven Huffman codebooks whose per-symbol code/length bytes are
//! built in the decoder's `.data` BSS at init and are **not** present in
//! the file image (`spec/05` §3.2 — docs-gap #1775). That is the one hard
//! blocker the frame walk stops at.
//!
//! *Downstream* of that blocker, however, the trace pins the entire
//! **reconstruction arithmetic** statically: once the per-band symbol
//! values, sign bits, dequant scales and per-band gains are in hand, the
//! dequantiser `cook.dll!0x4600` assembles each spectral line as
//! `value * sign * dequant_scale * band_gain` (§3.1) over the band's
//! coefficient range (§2.1). This module wires that pinned arithmetic into
//! a coherent **band → spectrum** pipeline that produces the
//! **inverse-transform input array** — the iMDCT feed — *given* the
//! entropy-decoded values. The stereo decouple (§4.2) is layered on top in
//! `decouple_stereo`.
//!
//! Splitting the decode this way keeps the wall intact: the BSS-built
//! codebook bytes are the caller's input (a §3.2 GAP a future Validator
//! round fills), and everything this module computes is the trace's own
//! closed-form arithmetic. A consumer that dumps the BSS codebooks can
//! feed their decoded values straight into [`reconstruct_band`] /
//! [`reconstruct_spectrum`] to obtain the iMDCT input without re-deriving
//! any DSP.
//!
//! ## What this module provides
//!
//! - [`reconstruct_band`] — fill one subband's coefficient range
//!   `[start..end)` (§2.1) of a spectrum buffer from the band's decoded
//!   values + sign bits, scaled by the §3.1 dequant scale and the §1–§2
//!   per-band gain (via [`crate::spectral::spectral_coefficient`]).
//! - [`BandReconstruction`] — the per-band input bundle (category, decoded
//!   values, sign bits, dequant scale, band gain) the band fill consumes.
//! - [`reconstruct_spectrum`] — drive [`reconstruct_band`] over every
//!   subband of a [`crate::subband::SubbandGeometry`], producing one
//!   channel's complete iMDCT-input spectrum.
//!
//! ## What stays a GAP (caller-supplied inputs)
//!
//! The per-symbol codebook **values** (§3.2 BSS GAP), the **sign bits**
//! and per-band **quantiser indices** the VLC walk reads are all caller
//! inputs — this module never fabricates them. It wires only the trace's
//! pinned reconstruction arithmetic over those inputs.
//!
//! ## Wall-respect note
//!
//! Every fact here is anchored to `spec/05` §2.1 / §2.2 / §3.1 and the
//! matching provenance evidence; the band → coefficient-range geometry and
//! the dequant multiply chain are the trace's own. No BSS-resident bytes
//! are guessed.

use crate::{
    category::CategoryIndex, spectral::spectral_coefficient, subband::SubbandGeometry, Error,
};

/// The per-band inputs the §3.1 dequant fill consumes for one subband.
///
/// All three of `values`, `sign_bits` length-match the band's coded line
/// count; `values` carries the decoded codebook value for each spectral
/// line (a §3.2 BSS GAP supplies them), `sign_bits` the embedded sign bit
/// per line (`0` → `+1.0`, non-zero → `-1.0`, via
/// [`crate::spectral::sign_from_bit`]), `dequant_scale` the §3.1 scale
/// read as `[q*4 + 0x9150]` for the band, and `band_gain` the §1–§2
/// per-band gain applied to every coefficient of the band.
///
/// `category` is the band's assigned quantiser category (§2.2). It is
/// carried for completeness — the dequant fill itself needs only the
/// scalar scale + gain — and lets a consumer thread the category through
/// the reconstruction for diagnostics / future grouping checks.
#[derive(Debug, Clone)]
pub struct BandReconstruction<'a> {
    /// The band's assigned quantiser category (§2.2).
    pub category: CategoryIndex,
    /// Decoded codebook value per spectral line (length == line count).
    pub values: &'a [f32],
    /// Embedded sign bit per spectral line (length == line count).
    pub sign_bits: &'a [u32],
    /// The §3.1 dequant scale read as `[q*4 + 0x9150]` for the band.
    pub dequant_scale: f32,
    /// The §1–§2 per-band gain applied to every coefficient.
    pub band_gain: f32,
}

/// Fill one subband's coefficient range `[start..end)` of `spectrum`
/// (§2.1) with the band's dequantised, sign-restored, gain-scaled
/// coefficients (§3.1).
///
/// `geometry` supplies the band's half-open line range
/// `[start_line[band] .. start_line[band+1])` (§2.1, the `0x8c40` LUT);
/// `recon` supplies the band's decoded values, sign bits, dequant scale
/// and per-band gain. Each line `start + i` of `spectrum` is set to
/// `spectral_coefficient(values[i], sign_bits[i], dequant_scale,
/// band_gain)` — exactly the §3.1 assembly
/// `value * sign * dequant_scale * band_gain`.
///
/// # Errors
///
/// - [`Error::BitAllocAxisOutOfRange`] when `band >= subband_count`.
/// - [`Error::BandValueCountMismatch`] when `recon.values.len()` (or
///   `recon.sign_bits.len()`) is not the band's coded line count.
/// - [`Error::BitAllocAxisOutOfRange`] (mapped via the range) when
///   `spectrum` is too short to hold the band's `[start..end)` range.
pub fn reconstruct_band(
    spectrum: &mut [f32],
    geometry: &SubbandGeometry,
    band: u32,
    recon: &BandReconstruction<'_>,
) -> Result<(), Error> {
    let range = geometry.line_range(band)?;
    let line_count = range.end - range.start;
    if recon.values.len() as u32 != line_count || recon.sign_bits.len() as u32 != line_count {
        // Report against the value slice (the primary GAP-sourced input);
        // the sign slice must match it.
        let got = if recon.values.len() as u32 != line_count {
            recon.values.len()
        } else {
            recon.sign_bits.len()
        };
        return Err(Error::BandValueCountMismatch { line_count, got });
    }
    let start = range.start as usize;
    let end = range.end as usize;
    if end > spectrum.len() {
        // The band's top line is past the spectrum buffer — treat as an
        // axis-range error (the buffer must span every coded line).
        return Err(Error::BitAllocAxisOutOfRange {
            got: band.min(u32::from(u8::MAX)) as u8,
        });
    }
    for (i, slot) in spectrum[start..end].iter_mut().enumerate() {
        *slot = spectral_coefficient(
            recon.values[i],
            recon.sign_bits[i],
            recon.dequant_scale,
            recon.band_gain,
        );
    }
    Ok(())
}

/// Reconstruct one channel's complete iMDCT-input spectrum from a
/// per-band reconstruction map (§2 / §3.1).
///
/// `geometry` is the stream's [`SubbandGeometry`]; `bands` carries one
/// [`BandReconstruction`] per coded subband (length == `subband_count`).
/// The returned vector is `geometry.total_coded_lines()` long: each
/// subband's coefficient range is filled by [`reconstruct_band`], so the
/// whole coded spectrum is assembled gap-free (the §2.1 ranges tile
/// `[start_line[0] .. total_coded_lines)` without overlap). Lines below
/// `start_line[0]` (if any) are left `0.0`.
///
/// # Errors
///
/// - [`Error::SpectrumBandCountMismatch`] when `bands.len()` is not the
///   stream's subband count.
/// - any error [`reconstruct_band`] raises for a band (value-count
///   mismatch, axis range).
pub fn reconstruct_spectrum(
    geometry: &SubbandGeometry,
    bands: &[BandReconstruction<'_>],
) -> Result<Vec<f32>, Error> {
    let subband_count = geometry.subband_count();
    if bands.len() as u32 != subband_count {
        return Err(Error::SpectrumBandCountMismatch {
            subband_count,
            got: bands.len(),
        });
    }
    let total = geometry.total_coded_lines() as usize;
    let mut spectrum = vec![0.0f32; total];
    for (band, recon) in bands.iter().enumerate() {
        reconstruct_band(&mut spectrum, geometry, band as u32, recon)?;
    }
    Ok(spectrum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(c: u8) -> CategoryIndex {
        CategoryIndex::new(c).unwrap()
    }

    #[test]
    fn band_fill_writes_pinned_dequant_chain() {
        // A 20-subband stream: band 0 is one line wide (identity run).
        let geom = SubbandGeometry::new(20).unwrap();
        let range = geom.line_range(0).unwrap();
        assert_eq!(range.end - range.start, 1);

        let values = [3.0f32];
        let signs = [0u32];
        let recon = BandReconstruction {
            category: cat(2),
            values: &values,
            sign_bits: &signs,
            dequant_scale: 0.25,
            band_gain: 2.0,
        };
        let mut spectrum = vec![0.0f32; geom.total_coded_lines() as usize];
        reconstruct_band(&mut spectrum, &geom, 0, &recon).unwrap();
        // value * sign(+1) * scale * gain = 3 * 1 * 0.25 * 2 = 1.5.
        assert_eq!(spectrum[range.start as usize], 1.5);
    }

    #[test]
    fn band_fill_restores_sign_per_line() {
        // Band 12 is the first compressed band; pick a multi-line band to
        // exercise per-line sign restoration.
        let geom = SubbandGeometry::new(20).unwrap();
        // Find a band wider than one line.
        let band = (0..geom.subband_count())
            .find(|&b| geom.line_count(b).unwrap() >= 2)
            .unwrap_or(0);
        let lc = geom.line_count(band).unwrap() as usize;
        let values: Vec<f32> = (0..lc).map(|i| (i + 1) as f32).collect();
        // Alternate sign bits.
        let signs: Vec<u32> = (0..lc).map(|i| (i % 2) as u32).collect();
        let recon = BandReconstruction {
            category: cat(0),
            values: &values,
            sign_bits: &signs,
            dequant_scale: 1.0,
            band_gain: 1.0,
        };
        let mut spectrum = vec![0.0f32; geom.total_coded_lines() as usize];
        reconstruct_band(&mut spectrum, &geom, band, &recon).unwrap();
        let range = geom.line_range(band).unwrap();
        for (i, line) in (range.start..range.end).enumerate() {
            let sign = if signs[i] == 0 { 1.0 } else { -1.0 };
            assert_eq!(spectrum[line as usize], values[i] * sign);
        }
    }

    #[test]
    fn band_fill_rejects_value_count_mismatch() {
        let geom = SubbandGeometry::new(20).unwrap();
        // Band 0 is one line; hand it two values.
        let values = [1.0f32, 2.0];
        let signs = [0u32, 0];
        let recon = BandReconstruction {
            category: cat(0),
            values: &values,
            sign_bits: &signs,
            dequant_scale: 1.0,
            band_gain: 1.0,
        };
        let mut spectrum = vec![0.0f32; geom.total_coded_lines() as usize];
        assert_eq!(
            reconstruct_band(&mut spectrum, &geom, 0, &recon).unwrap_err(),
            Error::BandValueCountMismatch {
                line_count: 1,
                got: 2
            }
        );
    }

    #[test]
    fn band_fill_rejects_sign_count_mismatch() {
        // Band 0 is one line; the value slice matches but the sign slice
        // is empty — the mismatch is reported against the sign slice.
        let geom = SubbandGeometry::new(20).unwrap();
        assert_eq!(geom.line_count(0).unwrap(), 1);
        let values = [1.0f32];
        let signs: [u32; 0] = []; // zero signs vs one line.
        let recon = BandReconstruction {
            category: cat(0),
            values: &values,
            sign_bits: &signs,
            dequant_scale: 1.0,
            band_gain: 1.0,
        };
        let mut spectrum = vec![0.0f32; geom.total_coded_lines() as usize];
        assert_eq!(
            reconstruct_band(&mut spectrum, &geom, 0, &recon).unwrap_err(),
            Error::BandValueCountMismatch {
                line_count: 1,
                got: 0
            }
        );
    }

    #[test]
    fn band_fill_rejects_out_of_range_band() {
        let geom = SubbandGeometry::new(20).unwrap();
        let values = [1.0f32];
        let signs = [0u32];
        let recon = BandReconstruction {
            category: cat(0),
            values: &values,
            sign_bits: &signs,
            dequant_scale: 1.0,
            band_gain: 1.0,
        };
        let mut spectrum = vec![0.0f32; geom.total_coded_lines() as usize];
        assert!(matches!(
            reconstruct_band(&mut spectrum, &geom, 20, &recon),
            Err(Error::BitAllocAxisOutOfRange { .. })
        ));
    }

    #[test]
    fn spectrum_assembles_every_band_gap_free() {
        let geom = SubbandGeometry::new(20).unwrap();
        // Build one band reconstruction per subband, each filled with its
        // line index as the value and sign bit 0.
        let mut value_store: Vec<Vec<f32>> = Vec::new();
        let mut sign_store: Vec<Vec<u32>> = Vec::new();
        for band in 0..geom.subband_count() {
            let lc = geom.line_count(band).unwrap() as usize;
            value_store.push((0..lc).map(|i| (band as f32) + i as f32 / 100.0).collect());
            sign_store.push(vec![0; lc]);
        }
        let bands: Vec<BandReconstruction> = (0..geom.subband_count() as usize)
            .map(|b| BandReconstruction {
                category: cat(0),
                values: &value_store[b],
                sign_bits: &sign_store[b],
                dequant_scale: 1.0,
                band_gain: 1.0,
            })
            .collect();
        let spectrum = reconstruct_spectrum(&geom, &bands).unwrap();
        assert_eq!(spectrum.len(), geom.total_coded_lines() as usize);
        // Every band's range carries its own band value prefix.
        for band in 0..geom.subband_count() {
            let range = geom.line_range(band).unwrap();
            for (i, line) in (range.start..range.end).enumerate() {
                assert_eq!(spectrum[line as usize], (band as f32) + i as f32 / 100.0);
            }
        }
    }

    #[test]
    fn spectrum_rejects_band_count_mismatch() {
        let geom = SubbandGeometry::new(20).unwrap();
        let values = [0.0f32];
        let signs = [0u32];
        // Five band reconstructions for a 20-subband stream.
        let bands: Vec<BandReconstruction> = (0..5)
            .map(|_| BandReconstruction {
                category: cat(0),
                values: &values,
                sign_bits: &signs,
                dequant_scale: 1.0,
                band_gain: 1.0,
            })
            .collect();
        assert_eq!(
            reconstruct_spectrum(&geom, &bands).unwrap_err(),
            Error::SpectrumBandCountMismatch {
                subband_count: 20,
                got: 5
            }
        );
    }
}
