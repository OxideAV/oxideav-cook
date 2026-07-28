//! Backend per-frame body orchestrator (frame-syntax §0–§5).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §0 (the frame
//! driver + bit reader), §1 (gain envelope), §2 (category/quant walk),
//! §3 (spectral VLC), §4 (joint-stereo coupling) and §5 (output stage),
//! plus `docs/audio/cook/provenance/05-cook-backend.md`.
//!
//! ## What this module does
//!
//! `spec/05` §0 pins the backend per-frame body as four sub-stages run
//! in a fixed order over one sub-packet's bitstream:
//!
//! > *"**gain control → category/quant walk → spectral VLC dequant →
//! > inverse transform**; … the stereo body additionally runs the
//! > coupling split."*
//!
//! This module assembles the **statically-pinned** prefix of that walk
//! into a single orchestrated entry point that mirrors the binary's
//! frame body, driving each stage off the modules already in this crate:
//!
//! 1. **Gain envelope (§1).** Reads the leading 6-bit segment-count
//!    field ([`crate::gain::read_segment_count`]) from the frame bit
//!    reader. The per-segment record reads (position + gain index) then
//!    descend the VLC walk — a §3.2 BSS GAP (see below).
//! 2. **Subband geometry (§2.1).** Builds the band → coefficient-range
//!    map ([`crate::subband::SubbandGeometry`]) the dequant walk and the
//!    coupling split both consume.
//! 3. **Spectral VLC dequant (§3).** Each coded band reads vector VLC
//!    symbols from one of the seven codebooks. The codebook *contents*
//!    (per-symbol code/length bytes) — once the §3.2 file-image GAP —
//!    were **recovered** and are now vendored and wired
//!    ([`crate::codebook`] / [`crate::spectral_decode`]); given a
//!    per-band category list the §3 read runs to completion
//!    ([`crate::frame_decode::decode_spectrum`]). What is **not** pinned
//!    by `spec/05` is the *pre-spectral read layout* that reaches §3 on a
//!    real frame: which recovered codebook the §1.2 gain-index and §2.2
//!    per-band quant-index VLC reads select, and how the
//!    category-assignment value array `v[]` is formed from the bitstream.
//!    So the walk cannot position the reader at the §3 data from the
//!    frame head and stops there, surfacing
//!    [`crate::Error::SpectralCodebookBytesUnavailable`] (the variant
//!    name kept for compatibility; it now denotes the read-layout gap,
//!    not a missing-bytes gap).
//!
//! The orchestrator is therefore a *driver to the documented blocker*:
//! it performs every stage the trace pins statically and reports exactly
//! the sub-step where the unpinned per-band read layout is required,
//! rather than guessing it. Callers that already hold the post-gain
//! reader position and a category list run the whole §3→§5 chain through
//! [`crate::frame_decode::decode_frame_spectrum`] /
//! [`crate::frame_decode::decode_frame_spectrum_assigned`].
//!
//! ## Post-entropy reconstruction (downstream of the blocker)
//!
//! The read-layout blocker gates only *reaching* the entropy read from
//! the frame head. Everything from the §3 read onward — given a category
//! list — is wired: the dequant assembly (§3.1), the per-band fill over
//! the §2.1 subband geometry, and the §4 stereo coupling split are pinned
//! statically. [`reconstruct_frame_spectrum`] ties those into a single
//! **post-entropy → iMDCT input** stage: given the entropy-decoded per-band
//! inputs (the decoded values + signs, and the recovered §4.3 coupling
//! `coef` table, supplied by the caller) it produces one
//! [`FrameSpectrum`] — a mono spectrum or a stereo [`StereoSpectra`] pair —
//! the iMDCT kernel would consume. The reconstruction arithmetic lives in
//! [`crate::reconstruct`]; this module routes it by channel count.
//!
//! ## What stays a GAP (not wired)
//!
//! - The **pre-spectral read layout** that reaches §3 on a real frame:
//!   which recovered codebook the §1.2 gain-index / §2.2 quant-index VLC
//!   reads select, and how the category-assignment value array `v[]` is
//!   formed from the bitstream — `spec/05` pins neither. This is the
//!   sub-step the walk stops at.
//! - The §2.2 category-*assignment* bit-allocation loop is itself
//!   **recovered and wired** ([`crate::category_assignment`]); it is the
//!   `v[]` **input** to it that the frame head does not pin.
//! - The iMDCT kernel (§5, the `0xa1b0` rotation table) is a recorded
//!   spec/01 §6 GAP; the recovered N=1024 window/twiddles feed the
//!   canonical transform ([`crate::imlt_direct`], [`crate::Synthesizer`]).
//!
//! ## Wall-respect note
//!
//! Every behavioural fact here is anchored to `spec/05` §0–§4; the stage
//! order and the band → line geometry are the trace's own. No per-band
//! read layout is guessed — the walk stops at the unpinned sub-step.

use crate::{
    bitreader::FrameBitReader,
    gain::read_segment_count,
    reconstruct::{decouple_stereo, reconstruct_spectrum, BandReconstruction, StereoSpectra},
    subband::SubbandGeometry,
    Error,
};

/// The decode progress one frame-body walk reached before the unpinned
/// pre-spectral read-layout blocker.
///
/// Returned by [`decode_frame_body`] on the real-decode path: it carries
/// the statically-pinned state assembled up to the §3 spectral-VLC
/// blocker so a consumer (or a future Validator round that dumps the BSS
/// codebooks) can resume exactly where the file-image facts run out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameWalk {
    /// Gain-envelope segment count read at the top of the frame (§1.1).
    /// The per-segment records themselves are VLC-gated (§3.2 GAP), so
    /// only the count is recovered before the blocker.
    pub gain_segment_count: u32,
    /// Per-stream subband geometry (§2.1) — the band → coefficient-range
    /// map the dequant walk and coupling split consume.
    pub subband_geometry: SubbandGeometry,
    /// Total coded spectral lines across all subbands
    /// (`subband_geometry.total_coded_lines()`), the number of
    /// coefficients the §3 dequant walk would fill.
    pub total_coded_lines: u32,
    /// Bits consumed from the frame bitstream up to the blocker.
    pub bits_consumed: u32,
}

/// Walk the backend per-frame body as far as the statically-pinned
/// frame-syntax allows, stopping at the unpinned pre-spectral read
/// layout.
///
/// `frame` is one sub-packet's bitstream (the §0 frame body); `channels`
/// and `subband_count` come from the wired [`crate::DecodeConfig`].
///
/// The walk:
///
/// 1. Reads the §1.1 gain-envelope segment count from `frame`.
/// 2. Builds the §2.1 subband geometry for `subband_count`.
/// 3. Cannot position the reader at the §3 spectral data and stops:
///    [`Error::SpectralCodebookBytesUnavailable`]. The seven codebooks'
///    bytes are recovered and wired (the §3 read runs given a category
///    list); what `spec/05` does not pin is the pre-spectral read layout
///    that reaches §3 on a real frame — which recovered codebook the
///    §1.2 gain-index / §2.2 quant-index VLC reads select, and how the
///    category-assignment `v[]` array is formed. The walk does **not**
///    guess it.
///
/// This is the faithful "drive to the documented blocker" entry point:
/// every pinned stage runs, and the precise sub-step needing the unpinned
/// read layout is surfaced as a typed error rather than fabricated.
///
/// # Errors
///
/// - [`Error::GainSegmentCountUnderflow`] if the §1.1 segment-count
///   field biases negative.
/// - [`Error::CookieZeroSubbandCount`] if `subband_count == 0`.
/// - [`Error::BitAllocAxisOutOfRange`] if `subband_count` exceeds the
///   51-entry subband LUT.
/// - [`Error::SpectralCodebookBytesUnavailable`] — the read-layout
///   blocker; always returned for a non-trivial stream, after the pinned
///   prefix has run.
pub fn decode_frame_body(
    frame: &[u8],
    channels: u16,
    subband_count: u32,
) -> Result<FrameWalk, Error> {
    // Run the statically-pinned prefix (§1.1 gain count + §2.1 subband
    // geometry); any stage-1/2 error surfaces here. If the prefix
    // succeeds the walk has reached the §3 spectral-VLC dequant step:
    // the codebooks are recovered, but the pre-spectral read layout that
    // positions the reader there (§1.2 gain-index / §2.2 quant-index VLC
    // codebook selection, and the `v[]` formation the category-assignment
    // loop consumes) is not pinned by spec/05 — stop precisely here
    // rather than guess it.
    let _prefix = frame_body_prefix(frame, channels, subband_count)?;
    Err(Error::SpectralCodebookBytesUnavailable)
}

/// Walk the pinned prefix and return the assembled [`FrameWalk`] state up
/// to (but not raising) the read-layout blocker.
///
/// Identical to [`decode_frame_body`] except it returns the
/// [`FrameWalk`] instead of the [`Error::SpectralCodebookBytesUnavailable`]
/// terminator, so callers (and tests) can inspect the statically-pinned
/// state the walk recovered before the documented blocker.
///
/// # Errors
///
/// Same as [`decode_frame_body`] minus the terminal
/// [`Error::SpectralCodebookBytesUnavailable`].
pub fn frame_body_prefix(
    frame: &[u8],
    channels: u16,
    subband_count: u32,
) -> Result<FrameWalk, Error> {
    let _ = channels;
    let mut reader = FrameBitReader::new(frame);
    let gain_segment_count = read_segment_count(&mut reader)?;
    let subband_geometry = SubbandGeometry::new(subband_count)?;
    let total_coded_lines = subband_geometry.total_coded_lines();
    Ok(FrameWalk {
        gain_segment_count,
        subband_geometry,
        total_coded_lines,
        bits_consumed: reader.bit_cursor(),
    })
}

/// One frame's reconstructed inverse-transform input — the iMDCT feed —
/// routed by channel count.
///
/// `spec/05` §0 pins the backend body as four sub-stages then the inverse
/// transform; for stereo it additionally runs the §4 coupling split before
/// the transform. This is the assembled output of the *reconstruction*
/// portion (everything between the §3.2 entropy blocker and the iMDCT
/// kernel itself, which is a separate spec/01 §6 GAP): one dequantised,
/// gain-scaled spectrum for mono, or two decoupled per-channel spectra for
/// stereo.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameSpectrum {
    /// A single channel's iMDCT-input spectrum (the §3.1-reconstructed
    /// coefficients of [`reconstruct_spectrum`]).
    Mono(Vec<f32>),
    /// Two per-channel iMDCT-input spectra produced by the §4 stereo
    /// decouple of a single coupled spectrum.
    Stereo(StereoSpectra),
}

/// The stereo coupling inputs the §4 decouple consumes, alongside the
/// per-band reconstruction inputs.
///
/// `spec/05` §4.1 reads one coupling/rotation index `j` per coupling band
/// over a contiguous subband range, and §4.2 splits each coupled
/// coefficient by `(coef[j], coef[Ncoup-1-j])`. The `coef` table values
/// are a §4.3 BSS GAP supplied by the caller.
#[derive(Debug, Clone)]
pub struct StereoCoupling<'a> {
    /// Contiguous coupling-band subband range `[first..last)` (§4.1).
    pub coupling_bands: core::ops::Range<u32>,
    /// One rotation index `j` per coupling band (§4.1).
    pub indices: &'a [u32],
    /// Per-coupling-width rotation coefficient table (§4.3 BSS GAP).
    pub coef: &'a [f32],
}

/// Reconstruct one frame's inverse-transform input from the
/// entropy-decoded per-band inputs, routing mono vs stereo (§2 / §3.1 /
/// §4 integration).
///
/// This is the coherent **post-entropy → iMDCT input** stage: it ties the
/// per-band [`reconstruct_spectrum`] fill (§3.1, over the §2.1 subband
/// geometry) to the §4 [`decouple_stereo`] split, routed by `channels`.
/// All entropy-dependent inputs — the per-band decoded values + sign bits
/// (§3.2 BSS GAP), the per-band gains/scales, and the coupling indices +
/// `coef` table (§4.3 BSS GAP) — are caller inputs; this function performs
/// only the trace's pinned reconstruction arithmetic.
///
/// - `geometry` is the stream's [`SubbandGeometry`] (built from the cookie
///   `subband_count`).
/// - `bands` carries one [`BandReconstruction`] per coded subband; for
///   stereo it is the *coupled* (down-mixed) spectrum's per-band inputs.
/// - `channels` selects the route: `1` → [`FrameSpectrum::Mono`] of the
///   reconstructed spectrum; `2` → [`FrameSpectrum::Stereo`] of the §4
///   decouple, which requires `coupling` to be `Some`.
/// - `coupling` supplies the §4 stereo coupling inputs; it is ignored for
///   mono and required (`Some`) for stereo.
///
/// # Errors
///
/// - [`Error::CookieInvalidChannels`] when `channels` is neither `1` nor
///   `2`.
/// - [`Error::StereoCouplingMissing`] when `channels == 2` but `coupling`
///   is `None`.
/// - any error [`reconstruct_spectrum`] or [`decouple_stereo`] raises
///   (band-count / value-count / coupling-index mismatch, axis range).
pub fn reconstruct_frame_spectrum(
    geometry: &SubbandGeometry,
    bands: &[BandReconstruction<'_>],
    channels: u16,
    coupling: Option<&StereoCoupling<'_>>,
) -> Result<FrameSpectrum, Error> {
    let coupled = reconstruct_spectrum(geometry, bands)?;
    match channels {
        1 => Ok(FrameSpectrum::Mono(coupled)),
        2 => {
            let c = coupling.ok_or(Error::StereoCouplingMissing)?;
            let stereo = decouple_stereo(
                &coupled,
                geometry,
                c.coupling_bands.clone(),
                c.indices,
                c.coef,
            )?;
            Ok(FrameSpectrum::Stereo(stereo))
        }
        other => Err(Error::CookieInvalidChannels { got: other }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::category::CategoryIndex;

    // A frame whose leading 6-bit field is `001000` = 8 → 8 − 6 = 2
    // gain segments (a well-formed, non-flat envelope head).
    fn frame_two_segments() -> [u8; 8] {
        // top 6 bits = 001000 = 8.
        [0b0010_0000, 0, 0, 0, 0, 0, 0, 0]
    }

    #[test]
    fn prefix_recovers_gain_count_and_geometry() {
        let frame = frame_two_segments();
        let walk = frame_body_prefix(&frame, 2, 20).unwrap();
        assert_eq!(walk.gain_segment_count, 2);
        assert_eq!(walk.subband_geometry.subband_count(), 20);
        assert_eq!(
            walk.total_coded_lines,
            walk.subband_geometry.total_coded_lines()
        );
        // The gain segment-count read consumed exactly 6 bits.
        assert_eq!(walk.bits_consumed, 6);
    }

    #[test]
    fn decode_stops_at_documented_read_layout_blocker() {
        // The full walk runs the pinned prefix then surfaces the
        // read-layout blocker — not NotImplemented, not a guess.
        let frame = frame_two_segments();
        assert_eq!(
            decode_frame_body(&frame, 2, 20).unwrap_err(),
            Error::SpectralCodebookBytesUnavailable
        );
    }

    #[test]
    fn decode_surfaces_gain_underflow_before_blocker() {
        // A raw segment-count field < 6 biases negative — that §1.1 error
        // fires before the §3 blocker is reached.
        let frame = [0u8; 8]; // top 6 bits = 0 → bias -6 → underflow.
        assert!(matches!(
            decode_frame_body(&frame, 2, 20),
            Err(Error::GainSegmentCountUnderflow { raw: 0 })
        ));
    }

    #[test]
    fn decode_rejects_zero_subband_count() {
        let frame = frame_two_segments();
        assert_eq!(
            decode_frame_body(&frame, 2, 0).unwrap_err(),
            Error::CookieZeroSubbandCount
        );
    }

    #[test]
    fn prefix_band_geometry_tiles() {
        // The recovered geometry tiles its coefficient range gap-free —
        // the band → line map the §3 dequant walk would consume.
        let frame = frame_two_segments();
        let walk = frame_body_prefix(&frame, 1, 12).unwrap();
        let geom = &walk.subband_geometry;
        let mut expected = geom.start_line(0).unwrap();
        for band in 0..geom.subband_count() {
            let r = geom.line_range(band).unwrap();
            assert_eq!(r.start, expected);
            expected = r.end;
        }
    }

    #[test]
    fn mono_and_stereo_reach_same_blocker() {
        // The §4 coupling split is past the §3 blocker, so mono and
        // stereo both stop at the same documented sub-step.
        let frame = frame_two_segments();
        assert_eq!(
            decode_frame_body(&frame, 1, 20).unwrap_err(),
            Error::SpectralCodebookBytesUnavailable
        );
        assert_eq!(
            decode_frame_body(&frame, 2, 20).unwrap_err(),
            Error::SpectralCodebookBytesUnavailable
        );
    }

    // ----- frame-spectrum integration (§2 / §3.1 / §4) -----

    fn cat(c: u8) -> CategoryIndex {
        CategoryIndex::new(c).unwrap()
    }

    // Build per-band reconstruction inputs for `geom`, each band's single
    // line carrying `band+1` as the value (sign 0, unit scale + gain).
    fn unit_bands(geom: &SubbandGeometry, values: &mut Vec<Vec<f32>>, signs: &mut Vec<Vec<u32>>) {
        for band in 0..geom.subband_count() {
            let lc = geom.line_count(band).unwrap() as usize;
            values.push(vec![(band + 1) as f32; lc]);
            signs.push(vec![0u32; lc]);
        }
    }

    #[test]
    fn frame_spectrum_mono_is_reconstructed_spectrum() {
        let geom = SubbandGeometry::new(20).unwrap();
        let mut values = Vec::new();
        let mut signs = Vec::new();
        unit_bands(&geom, &mut values, &mut signs);
        let bands: Vec<BandReconstruction> = (0..geom.subband_count() as usize)
            .map(|b| BandReconstruction {
                category: cat(0),
                values: &values[b],
                sign_bits: &signs[b],
                dequant_scale: 1.0,
                band_gain: 1.0,
            })
            .collect();
        let out = reconstruct_frame_spectrum(&geom, &bands, 1, None).unwrap();
        match out {
            FrameSpectrum::Mono(spectrum) => {
                assert_eq!(spectrum.len(), geom.total_coded_lines() as usize);
                // Band 0's single line carries value 1.0.
                let r0 = geom.line_range(0).unwrap();
                assert_eq!(spectrum[r0.start as usize], 1.0);
            }
            other => panic!("expected mono, got {other:?}"),
        }
    }

    #[test]
    fn frame_spectrum_stereo_routes_through_decouple() {
        let geom = SubbandGeometry::new(20).unwrap();
        let mut values = Vec::new();
        let mut signs = Vec::new();
        unit_bands(&geom, &mut values, &mut signs);
        let bands: Vec<BandReconstruction> = (0..geom.subband_count() as usize)
            .map(|b| BandReconstruction {
                category: cat(0),
                values: &values[b],
                sign_bits: &signs[b],
                dequant_scale: 1.0,
                band_gain: 1.0,
            })
            .collect();
        // Couple bands [2..5); coef {1.0, 0.0} steers by index.
        let coef = [1.0f32, 0.0];
        let indices = [0u32, 1, 0];
        let coupling = StereoCoupling {
            coupling_bands: 2..5,
            indices: &indices,
            coef: &coef,
        };
        let out = reconstruct_frame_spectrum(&geom, &bands, 2, Some(&coupling)).unwrap();
        match out {
            FrameSpectrum::Stereo(s) => {
                let total = geom.total_coded_lines() as usize;
                assert_eq!(s.ch0.len(), total);
                assert_eq!(s.ch1.len(), total);
                // Band 2 (index 0): coef[0]=1 -> ch0 carries the coupled
                // value (band 2's value = 3.0), ch1 = 0.
                let r2 = geom.line_range(2).unwrap();
                assert_eq!(s.ch0[r2.start as usize], 3.0);
                assert_eq!(s.ch1[r2.start as usize], 0.0);
            }
            other => panic!("expected stereo, got {other:?}"),
        }
    }

    #[test]
    fn frame_spectrum_stereo_without_coupling_errors() {
        let geom = SubbandGeometry::new(20).unwrap();
        let mut values = Vec::new();
        let mut signs = Vec::new();
        unit_bands(&geom, &mut values, &mut signs);
        let bands: Vec<BandReconstruction> = (0..geom.subband_count() as usize)
            .map(|b| BandReconstruction {
                category: cat(0),
                values: &values[b],
                sign_bits: &signs[b],
                dequant_scale: 1.0,
                band_gain: 1.0,
            })
            .collect();
        assert_eq!(
            reconstruct_frame_spectrum(&geom, &bands, 2, None).unwrap_err(),
            Error::StereoCouplingMissing
        );
    }

    #[test]
    fn frame_spectrum_rejects_invalid_channel_count() {
        let geom = SubbandGeometry::new(20).unwrap();
        let mut values = Vec::new();
        let mut signs = Vec::new();
        unit_bands(&geom, &mut values, &mut signs);
        let bands: Vec<BandReconstruction> = (0..geom.subband_count() as usize)
            .map(|b| BandReconstruction {
                category: cat(0),
                values: &values[b],
                sign_bits: &signs[b],
                dequant_scale: 1.0,
                band_gain: 1.0,
            })
            .collect();
        for ch in [0u16, 3, 6] {
            assert_eq!(
                reconstruct_frame_spectrum(&geom, &bands, ch, None).unwrap_err(),
                Error::CookieInvalidChannels { got: ch }
            );
        }
    }
}
