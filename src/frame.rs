//! Backend per-frame body orchestrator (frame-syntax §0–§5).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §0.2 (the
//! pre-spectral wire field order, pinned by behavioural trace on three
//! real frames), §1 (the band-envelope array), §2 (the category walk +
//! allocator budget rule), §3 (spectral VLC), §4 (joint-stereo
//! coupling), plus `docs/audio/cook/provenance/09-cook-frame-read-layout.md`
//! and the vendored `tables/frame-read-layout.csv` /
//! `tables/live-frame-params.csv`.
//!
//! ## The §0.2 wire order (round 9)
//!
//! The order in which the backend actually consumes a frame:
//!
//! | # | field | width |
//! | - | ----- | ----- |
//! | 1 | sub-packet flag | 1 |
//! | 2 | coupling-control mode flag (stereo) | 1 |
//! | 3 | coupling index × `Ncoupband` | `coupling_bits` fixed **or** VLC |
//! | 4 | envelope seed | 6 |
//! | 5 | envelope value × `Nb − 1` | VLC (31-entry tree family) |
//! | 6 | frame scalar | 7 |
//! | — | bit-allocation call (no bits): `budget = bit_limit − cursor` | |
//! | 7 | spectral symbols + out-of-band signs | VLC / 1 |
//!
//! > **Round-9 correction.** Earlier revisions of this walk read the
//! > frame head as a §1.1 *time-domain gain envelope* (a 6-bit segment
//! > count biased −6, then per-segment records). The live trace
//! > withdrew that reading: the head worker performs one 6-bit read and
//! > `Nb − 1` VLC reads and nothing else, and its output buffer is the
//! > allocator's per-band value array `v[]`. Whether this flavor
//! > carries a time-domain gain envelope at all is open (spec/05 §1.2).
//!
//! ## What the walk runs vs where it stops
//!
//! [`read_frame_head`] consumes fields 1–4 (the fixed-width prefix plus
//! the fixed-branch coupling indices). Two wire reads remain gated on
//! **unstaged VLC tree contents**:
//!
//! - field 3's VLC branch (coupling mode flag = 1) — the coupling-index
//!   tree ([`crate::Error::CouplingIndexTreeUnavailable`]);
//! - field 5 — the `Nb − 1` envelope values, read through a separate
//!   **31-entry tree family** at `backend+0x44c8` (tree `max(0, k−3)`
//!   for symbol `k`) whose per-symbol codes are **not** among the
//!   staged tables ([`crate::Error::EnvelopeValueTreeUnavailable`]).
//!
//! [`decode_frame_body`] therefore takes an optional
//! [`EnvelopeInjection`] — the caller-supplied `v[]` **and** the bit
//! cursor where field 6 begins (both captured for three real frames in
//! `tables/live-frame-params.csv` / `live-frame-allocator-io.csv`) — and
//! runs the rest of the frame: the 7-bit scalar, the §2.2 allocator
//! (`budget = bit_limit − cursor`, the round-9 budget rule) computing
//! the per-band categories, the §3 codebook-by-category spectral read,
//! and (for stereo) the §4 pan split. Without an injection it stops at
//! field 5 with the typed envelope-tree gap.
//!
//! ## Per-flavor layout inputs
//!
//! [`FrameLayout`] carries the per-stream §0.2 parameters. For the
//! validated flavor (record 21/22: stereo 44.1 kHz, 1024-line frames,
//! geometry `subband_count = 32`) the traced values are `Nb = 34`,
//! `coupling_bits = 4`, `Ncoupband = 16`, `M = 128`
//! ([`FrameLayout::validated_stereo`], from `live-frame-params`). The
//! §4.1 coupling **band mapping** (which subbands each coupling index
//! covers) is *not* pinned; [`CouplingMap`] makes it an explicit caller
//! input. The consistency `2 + 16 × 2 = 34` (two uncoupled low bands,
//! sixteen coupling bands of two subbands) fits every traced number and
//! is the documented default hypothesis — flagged, not fact.
//!
//! ## Wall-respect note
//!
//! The wire order, widths and reader primitives are the staged
//! `frame-read-layout` facts; the budget rule and `M` are the staged
//! live-frame facts; the envelope-tree and coupling-tree contents are
//! typed gaps, never guessed. The per-band reconstruction gain (how the
//! §1–§2 "per-band gain" derives from `v[b]`) is likewise a caller
//! input ([`decode_frame_body`]'s `band_gains`), not fabricated.

use crate::{
    bitreader::FrameBitReader,
    category_assignment::assign_categories,
    coupling::CouplingPanWidth,
    coupling_control::{read_coupling_mode, read_fixed_coupling_index, CouplingReadMode},
    frame_decode::{decode_frame_spectrum, DecodedSpectrum, FrameCoupling},
    reconstruct::{decouple_stereo, reconstruct_spectrum, BandReconstruction, StereoSpectra},
    spectral_decode::BandCategory,
    subband::SubbandGeometry,
    Error,
};

/// The per-stream §0.2 frame-layout parameters the walk consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameLayout {
    /// Channel count (1 or 2); the coupling control (fields 2–3) exists
    /// only in the stereo body.
    pub channels: u16,
    /// The per-flavor coupling-index bit width (context `+0x1c`; also
    /// selects the §4.3 pan table).
    pub coupling_bits: u32,
    /// Coupling bands per frame (`Ncoupband`; 16 on the traced flavor).
    pub coupling_band_count: u32,
    /// The allocator band count `Nb` (decode-state `+0x20`; 34 live).
    pub band_count: u32,
    /// The Stage-2 refinement bound `M` (decode-state `+0x28`; 128
    /// live, pinned by replay).
    pub refinement_bound: u32,
    /// The §4.1 coupling band mapping (unpinned — caller input).
    pub coupling_map: CouplingMap,
}

/// The §4.1 coupling band → subband mapping (a recorded docs question:
/// the coupling band *range* was not pinned by the round-9 trace).
///
/// Coupling band `k` covers `subbands_per_index` consecutive subbands
/// starting at `start_band + k × subbands_per_index`; subbands below
/// `start_band` are outside the coupling split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CouplingMap {
    /// First coupled subband.
    pub start_band: u32,
    /// Subbands covered per coupling index.
    pub subbands_per_index: u32,
}

impl FrameLayout {
    /// The traced layout of the validated flavor (records 21/22 —
    /// stereo 44.1 kHz, 1024-line frames): `coupling_bits = 4`,
    /// `Ncoupband = 16`, `Nb = 34`, `M = 128`
    /// (`tables/live-frame-params.csv`, provenance/09), with the
    /// documented default [`CouplingMap`] hypothesis
    /// (`start_band = 2`, two subbands per index: `2 + 16 × 2 = 34`).
    #[must_use]
    pub fn validated_stereo() -> Self {
        FrameLayout {
            channels: 2,
            coupling_bits: 4,
            coupling_band_count: 16,
            band_count: 34,
            refinement_bound: 128,
            coupling_map: CouplingMap {
                start_band: 2,
                subbands_per_index: 2,
            },
        }
    }

    /// The layout for a stream's flavor geometry, when the traced
    /// per-flavor §0.2 parameters are known for it.
    ///
    /// Only the validated flavor family (stereo, `subband_count = 32`,
    /// 1024 samples per frame — records 21/22) has traced values; any
    /// other geometry returns [`Error::FrameLayoutUnknown`] rather than
    /// a fabricated layout.
    pub fn for_flavor_geometry(
        channels: u16,
        subband_count: u32,
        samples_per_frame: u32,
    ) -> Result<Self, Error> {
        if channels == 2 && subband_count == 32 && samples_per_frame == 1024 {
            return Ok(Self::validated_stereo());
        }
        Err(Error::FrameLayoutUnknown {
            channels,
            subband_count,
            samples_per_frame,
        })
    }

    /// The §4.3 pan-table selector for this layout's coupling width.
    ///
    /// # Errors
    ///
    /// [`Error::CouplingPanWidthUnsupported`] when `coupling_bits` is
    /// outside the stored `2..=6`.
    pub fn pan_width(&self) -> Result<CouplingPanWidth, Error> {
        CouplingPanWidth::from_bits(self.coupling_bits)
    }
}

/// The fixed-width §0.2 frame head — fields 1–4 of the wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHead {
    /// Field 1 — the 1-bit sub-packet flag (`0` in every traced frame;
    /// semantics NOT established, spec/05 §0.2).
    pub subpacket_flag: u32,
    /// Fields 2–3 — the coupling control (stereo only): the resolved
    /// read mode and, for the fixed branch, the `Ncoupband` indices.
    pub coupling: Option<FrameHeadCoupling>,
    /// Field 4 — the 6-bit envelope seed (stored into `v[0]` by the
    /// envelope worker; the staged live captures show the *stored*
    /// `v[0]` differing from the re-derived wire field, a recorded
    /// docs question — see the crate README).
    pub envelope_seed: u32,
    /// Bit cursor after field 4 — where the field-5 envelope VLC
    /// begins.
    pub bits_consumed: u32,
}

/// The stereo coupling control of a frame head (fields 2–3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeadCoupling {
    /// The mode flag's resolved read mode (fixed-width vs VLC).
    pub mode: CouplingReadMode,
    /// One coupling index per coupling band (fixed branch only; the VLC
    /// branch is gated on the unstaged coupling tree).
    pub indices: Vec<u32>,
}

/// Read the §0.2 fixed-width frame head — fields 1–4 — from the start
/// of one sub-packet's bitstream.
///
/// # Errors
///
/// - [`Error::CouplingIndexTreeUnavailable`] when the coupling mode
///   flag selects the VLC branch (field 3b): the coupling-index VLC
///   tree contents are not among the staged tables.
/// - [`Error::CouplingIndexOutOfRange`] when a fixed-width coupling
///   index reaches `Ncoup = (1 << coupling_bits) − 1` (the pan table
///   holds `Ncoup` entries and the traced indices stay `0..=14` at
///   width 4).
pub fn read_frame_head(
    reader: &mut FrameBitReader<'_>,
    layout: &FrameLayout,
) -> Result<FrameHead, Error> {
    let subpacket_flag = reader.read_bits(1);
    let coupling = if layout.channels == 2 {
        let mode = read_coupling_mode(reader, layout.coupling_bits);
        let indices = match mode {
            CouplingReadMode::Fixed { bits } => {
                let ncoup = (1u32 << layout.coupling_bits) - 1;
                let mut indices = Vec::with_capacity(layout.coupling_band_count as usize);
                for _ in 0..layout.coupling_band_count {
                    let j = read_fixed_coupling_index(reader, bits);
                    if j >= ncoup {
                        return Err(Error::CouplingIndexOutOfRange { got: j, ncoup });
                    }
                    indices.push(j);
                }
                indices
            }
            CouplingReadMode::Vlc => return Err(Error::CouplingIndexTreeUnavailable),
        };
        Some(FrameHeadCoupling { mode, indices })
    } else {
        None
    };
    let envelope_seed = reader.read_bits(6);
    Ok(FrameHead {
        subpacket_flag,
        coupling,
        envelope_seed,
        bits_consumed: reader.bit_cursor(),
    })
}

/// A caller-supplied stand-in for the field-5 envelope VLC read — the
/// per-band value array `v[]` and the bit cursor where field 6 (the
/// 7-bit frame scalar) begins.
///
/// The `Nb − 1` envelope values are read through the 31-entry VLC tree
/// family at `backend+0x44c8` whose contents are **not** among the
/// staged tables; for the three traced frames both the values and the
/// post-envelope cursor were captured live
/// (`tables/live-frame-allocator-io.csv` / `live-frame-params.csv`:
/// the allocator cursor is `bit_limit − alloc_budget`, and field 6
/// occupies the 7 bits before it).
#[derive(Debug, Clone)]
pub struct EnvelopeInjection<'a> {
    /// The per-band value array `v[0..Nb]` the envelope worker stored.
    pub values: &'a [i32],
    /// Bit cursor at the start of field 6 (the 7-bit frame scalar) —
    /// `bit_limit − alloc_budget − 7` for a captured frame.
    pub cursor_at_frame_scalar: u32,
}

/// One fully-walked §0.2 frame body (through the spectral stage).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameBody {
    /// The fixed-width head (fields 1–4).
    pub head: FrameHead,
    /// The per-band envelope value array `v[]` (field 4 seed + field 5
    /// values — injected; see [`EnvelopeInjection`]).
    pub envelope: Vec<i32>,
    /// Field 6 — the 7-bit frame scalar (semantics NOT established;
    /// `109 / 89 / 103` on the traced frames).
    pub frame_scalar: u32,
    /// The §2.2 allocator budget — `bit_limit − cursor` at the
    /// allocator call (the round-9 budget rule).
    pub budget: i32,
    /// The computed per-band categories (`0..=7`).
    pub categories: Vec<u8>,
    /// The §3 spectral read routed through the §4 split (stereo) or
    /// straight (mono).
    pub spectrum: DecodedSpectrum,
    /// Bits consumed by the whole walk (head + scalar + spectral).
    pub bits_consumed: u32,
}

/// Walk one §0.2 frame body end to end: head (fields 1–4), the
/// envelope (field 5 — injected, see below), the 7-bit scalar (field
/// 6), the §2.2 allocator (`budget = bit_limit − cursor`), the §3
/// codebook-by-category spectral read and (stereo) the §4 pan split.
///
/// `envelope` supplies the field-5 stand-in; `None` stops the walk at
/// the envelope-tree gap. `band_gains` is the per-band reconstruction
/// gain (`&[1.0]` for unity — the `v[b]` → gain law is a recorded docs
/// question, so it is a caller input).
///
/// # Errors
///
/// - [`Error::EnvelopeValueTreeUnavailable`] when `envelope` is `None` —
///   the 31-entry envelope VLC tree family is not among the staged
///   tables.
/// - [`Error::SpectrumBandCountMismatch`] when the injected `values`
///   length is not `Nb`.
/// - [`Error::FrameCursorOutOfRange`] when the injected cursor lies
///   before the head or past the bit limit.
/// - [`Error::CouplingMapMismatch`] when the layout's coupling map does
///   not tile `Nb` (`start + Ncoupband × per != Nb`).
/// - any error [`read_frame_head`] or the spectral decode raises.
pub fn decode_frame_body(
    frame: &[u8],
    layout: &FrameLayout,
    envelope: Option<&EnvelopeInjection<'_>>,
    band_gains: &[f32],
) -> Result<FrameBody, Error> {
    let mut reader = FrameBitReader::new(frame);
    let head = read_frame_head(&mut reader, layout)?;
    let Some(inj) = envelope else {
        return Err(Error::EnvelopeValueTreeUnavailable);
    };
    if inj.values.len() as u32 != layout.band_count {
        return Err(Error::SpectrumBandCountMismatch {
            subband_count: layout.band_count,
            got: inj.values.len(),
        });
    }
    if inj.cursor_at_frame_scalar < head.bits_consumed
        || inj.cursor_at_frame_scalar + 7 > reader.bit_limit()
    {
        return Err(Error::FrameCursorOutOfRange {
            got: inj.cursor_at_frame_scalar,
            head: head.bits_consumed,
            limit: reader.bit_limit(),
        });
    }
    // Field 5 — the envelope VLC region — is skipped via the injected
    // cursor (its width is frame-dependent; the values are supplied).
    reader.skip_bits(inj.cursor_at_frame_scalar - reader.bit_cursor());
    // Field 6 — the 7-bit frame scalar.
    let frame_scalar = reader.read_bits(7);
    // The §2.2 allocator: budget = bit_limit − cursor, no bits consumed.
    let budget = reader.bit_limit() as i32 - reader.bit_cursor() as i32;
    let assignment = assign_categories(inj.values, budget, layout.refinement_bound);
    let categories = assignment.categories;
    let band_cats: Vec<BandCategory> = categories
        .iter()
        .map(|&c| BandCategory::from_raw(c))
        .collect::<Result<_, _>>()?;
    // §3 spectral read over the Nb-band geometry, routed through §4.
    let geometry = SubbandGeometry::new(layout.band_count)?;
    let spectrum = if layout.channels == 2 {
        let coupling_head = head.coupling.as_ref().ok_or(Error::StereoCouplingMissing)?;
        let map = layout.coupling_map;
        let mapped = map.start_band + layout.coupling_band_count * map.subbands_per_index;
        if map.subbands_per_index == 0 || mapped != layout.band_count {
            return Err(Error::CouplingMapMismatch {
                start_band: map.start_band,
                subbands_per_index: map.subbands_per_index,
                coupling_bands: layout.coupling_band_count,
                band_count: layout.band_count,
            });
        }
        // Expand one index per coupling band to one per subband.
        let expanded: Vec<u32> = (map.start_band..layout.band_count)
            .map(|b| {
                coupling_head.indices[((b - map.start_band) / map.subbands_per_index) as usize]
            })
            .collect();
        let coupling = FrameCoupling {
            coupling_bands: map.start_band..layout.band_count,
            indices: &expanded,
            pan_width: layout.pan_width()?,
        };
        decode_frame_spectrum(
            &mut reader,
            &geometry,
            &band_cats,
            band_gains,
            2,
            Some(&coupling),
        )?
    } else {
        decode_frame_spectrum(&mut reader, &geometry, &band_cats, band_gains, 1, None)?
    };
    Ok(FrameBody {
        head,
        envelope: inj.values.to_vec(),
        frame_scalar,
        budget,
        categories,
        spectrum,
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
/// (normally the wired §3 entropy read's output), the per-band gains/scales, and the coupling indices +
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

    use crate::codebook::spectral_huffman;
    use crate::spectral_decode::{codebook_for_category, compose_symbol};
    use crate::tables::{live_frame_allocator_io, live_frame_params};

    /// Pack `(value, nbits)` fields MSB-first into a byte buffer of
    /// exactly `len` bytes (zero-padded).
    fn pack_to(fields: &[(u32, u32)], len: usize) -> Vec<u8> {
        let mut bits: Vec<u8> = Vec::new();
        for &(v, n) in fields {
            for b in (0..n).rev() {
                bits.push(((v >> b) & 1) as u8);
            }
        }
        assert!(bits.len() <= len * 8, "fields overflow the frame");
        let mut bytes = vec![0u8; len];
        for (i, &bit) in bits.iter().enumerate() {
            if bit != 0 {
                bytes[i / 8] |= 0x80 >> (i % 8);
            }
        }
        bytes
    }

    fn stereo_layout() -> FrameLayout {
        FrameLayout::validated_stereo()
    }

    #[test]
    fn layout_for_flavor_geometry_matches_only_the_traced_family() {
        let l = FrameLayout::for_flavor_geometry(2, 32, 1024).unwrap();
        assert_eq!(l, FrameLayout::validated_stereo());
        assert_eq!(l.band_count, 34);
        assert_eq!(l.coupling_bits, 4);
        assert_eq!(l.coupling_band_count, 16);
        assert_eq!(l.refinement_bound, 128);
        // The default coupling-map hypothesis tiles Nb exactly.
        assert_eq!(
            l.coupling_map.start_band + l.coupling_band_count * l.coupling_map.subbands_per_index,
            l.band_count
        );
        // Untraced geometries are refused, not fabricated.
        assert!(matches!(
            FrameLayout::for_flavor_geometry(1, 32, 1024),
            Err(Error::FrameLayoutUnknown { .. })
        ));
        assert!(matches!(
            FrameLayout::for_flavor_geometry(2, 20, 512),
            Err(Error::FrameLayoutUnknown { .. })
        ));
    }

    #[test]
    fn head_reads_the_wire_order_fields() {
        // §0.2 fields 1-4: flag, coupling mode 0 (fixed), sixteen 4-bit
        // indices, 6-bit seed — 1 + 1 + 64 + 6 = 72 bits.
        let layout = stereo_layout();
        let mut fields = vec![(0u32, 1), (0u32, 1)];
        let want_indices: Vec<u32> = (0..16u32).map(|k| (k * 5) % 15).collect();
        for &j in &want_indices {
            fields.push((j, 4));
        }
        fields.push((23, 6)); // seed
        let frame = pack_to(&fields, 93);
        let mut reader = FrameBitReader::new(&frame);
        let head = read_frame_head(&mut reader, &layout).unwrap();
        assert_eq!(head.subpacket_flag, 0);
        let coupling = head.coupling.as_ref().unwrap();
        assert!(matches!(
            coupling.mode,
            crate::coupling_control::CouplingReadMode::Fixed { bits: 4 }
        ));
        assert_eq!(coupling.indices, want_indices);
        assert_eq!(head.envelope_seed, 23);
        assert_eq!(head.bits_consumed, 72);
        assert_eq!(reader.bit_cursor(), 72);
    }

    #[test]
    fn head_vlc_coupling_branch_surfaces_the_tree_gap() {
        let layout = stereo_layout();
        let frame = pack_to(&[(0, 1), (1, 1)], 93); // mode flag = 1 → VLC.
        let mut reader = FrameBitReader::new(&frame);
        assert_eq!(
            read_frame_head(&mut reader, &layout).unwrap_err(),
            Error::CouplingIndexTreeUnavailable
        );
    }

    #[test]
    fn head_rejects_out_of_table_fixed_index() {
        // A 4-bit index of 15 is one past the w=4 pan table (Ncoup 15).
        let layout = stereo_layout();
        let mut fields = vec![(0u32, 1), (0u32, 1)];
        fields.push((15, 4));
        let frame = pack_to(&fields, 93);
        let mut reader = FrameBitReader::new(&frame);
        assert_eq!(
            read_frame_head(&mut reader, &layout).unwrap_err(),
            Error::CouplingIndexOutOfRange { got: 15, ncoup: 15 }
        );
    }

    #[test]
    fn mono_head_has_no_coupling_control() {
        // The coupling control exists only in the stereo body (§0.2):
        // a mono layout reads flag then seed directly.
        let layout = FrameLayout {
            channels: 1,
            coupling_bits: 4,
            coupling_band_count: 0,
            band_count: 34,
            refinement_bound: 128,
            coupling_map: CouplingMap {
                start_band: 0,
                subbands_per_index: 1,
            },
        };
        let frame = pack_to(&[(1, 1), (42, 6)], 93);
        let mut reader = FrameBitReader::new(&frame);
        let head = read_frame_head(&mut reader, &layout).unwrap();
        assert_eq!(head.subpacket_flag, 1);
        assert!(head.coupling.is_none());
        assert_eq!(head.envelope_seed, 42);
        assert_eq!(head.bits_consumed, 7);
    }

    #[test]
    fn body_without_injection_stops_at_the_envelope_tree_gap() {
        let layout = stereo_layout();
        let frame = pack_to(&[(0, 1), (0, 1)], 93);
        assert_eq!(
            decode_frame_body(&frame, &layout, None, &[1.0]).unwrap_err(),
            Error::EnvelopeValueTreeUnavailable
        );
    }

    #[test]
    fn injected_cursor_bounds_are_enforced() {
        let layout = stereo_layout();
        let frame = pack_to(&[(0, 1), (0, 1)], 93);
        let v = vec![10i32; 34];
        // Before the head end (72 bits).
        let inj = EnvelopeInjection {
            values: &v,
            cursor_at_frame_scalar: 10,
        };
        assert!(matches!(
            decode_frame_body(&frame, &layout, Some(&inj), &[1.0]),
            Err(Error::FrameCursorOutOfRange { got: 10, .. })
        ));
        // Past the bit limit.
        let inj = EnvelopeInjection {
            values: &v,
            cursor_at_frame_scalar: 744,
        };
        assert!(matches!(
            decode_frame_body(&frame, &layout, Some(&inj), &[1.0]),
            Err(Error::FrameCursorOutOfRange { got: 744, .. })
        ));
        // Wrong band count.
        let short = vec![10i32; 33];
        let inj = EnvelopeInjection {
            values: &short,
            cursor_at_frame_scalar: 172,
        };
        assert!(matches!(
            decode_frame_body(&frame, &layout, Some(&inj), &[1.0]),
            Err(Error::SpectrumBandCountMismatch { .. })
        ));
    }

    #[test]
    fn coupling_map_must_tile_the_band_count() {
        let mut layout = stereo_layout();
        layout.coupling_map = CouplingMap {
            start_band: 3,
            subbands_per_index: 2,
        }; // 3 + 32 = 35 != 34.
        let frame = pack_to(&[(0, 1), (0, 1)], 93);
        let v = vec![10i32; 34];
        let inj = EnvelopeInjection {
            values: &v,
            cursor_at_frame_scalar: 172,
        };
        assert!(matches!(
            decode_frame_body(&frame, &layout, Some(&inj), &[1.0]),
            Err(Error::CouplingMapMismatch { .. })
        ));
    }

    #[test]
    fn injected_walk_decodes_a_full_synthetic_stereo_frame() {
        // The assembled §0.2 walk end to end on a 93-byte synthetic
        // frame shaped exactly like the traced packet 2: fields 1-4
        // (fixed coupling), a 100-bit stand-in for the field-5 envelope
        // VLC (values injected from the staged live capture), the 7-bit
        // scalar at bits 172..179, the allocator at cursor 179 with the
        // round-9 budget rule (744 - 179 = 565 — the live budget), the
        // computed categories (== the live capture, pinned in
        // category_assignment), the §3 spectral read and the §4 pan
        // split.
        let layout = stereo_layout();
        let live = &live_frame_allocator_io()[0];
        let params = &live_frame_params()[0];
        assert_eq!(live.packet, 2);

        let mut fields = vec![(0u32, 1), (0u32, 1)];
        let indices: Vec<u32> = (0..16u32).map(|k| k % 15).collect();
        for &j in &indices {
            fields.push((j, 4));
        }
        fields.push((17, 6)); // the seed field the extractor re-derived.
                              // Field 5 stand-in: 100 arbitrary bits (the injected values
                              // replace their decode).
        for _ in 0..100 {
            fields.push((1, 1));
        }
        assert_eq!(
            fields.iter().map(|&(_, n)| n).sum::<u32>(),
            172,
            "the 7-bit scalar must start at bit 172"
        );
        fields.push((109, 7)); // the traced frame scalar.

        // Spectral section: encode each coded band's full vector group
        // for the categories the allocator computes from (v, budget).
        let budget = 744 - 179;
        assert_eq!(budget, params.alloc_budget);
        let assignment = crate::category_assignment::assign_categories(&live.values, budget, 128);
        assert_eq!(assignment.categories, live.categories, "live cats");
        for &c in &assignment.categories {
            if c == 7 {
                continue;
            }
            let ci = crate::category::CategoryIndex::new(c).unwrap();
            let huffman = spectral_huffman(codebook_for_category(ci));
            let dims = crate::spectral::category_vector_dims(ci);
            let mut digits = vec![0u32; dims.lo as usize];
            digits[0] = 1;
            let first = compose_symbol(&digits, ci).unwrap();
            fields.push(huffman.codeword(first).unwrap());
            fields.push((0, 1)); // sign for the single non-zero digit.
            let zero = compose_symbol(&vec![0u32; dims.lo as usize], ci).unwrap();
            for _ in 1..dims.hi {
                fields.push(huffman.codeword(zero).unwrap());
            }
        }
        let total_bits: u32 = fields.iter().map(|&(_, n)| n).sum();
        assert!(total_bits <= 744, "frame must fit 93 bytes ({total_bits})");
        let frame = pack_to(&fields, 93);

        let inj = EnvelopeInjection {
            values: &live.values,
            cursor_at_frame_scalar: 172,
        };
        let body = decode_frame_body(&frame, &layout, Some(&inj), &[1.0]).unwrap();
        assert_eq!(body.head.envelope_seed, 17);
        assert_eq!(body.head.coupling.as_ref().unwrap().indices, indices);
        assert_eq!(body.frame_scalar, 109);
        assert_eq!(body.budget, 565);
        assert_eq!(body.categories, live.categories);
        assert_eq!(body.bits_consumed, total_bits, "bit-exact consumption");
        match &body.spectrum {
            DecodedSpectrum::Stereo(s) => {
                assert_eq!(s.ch0.len(), 680);
                assert_eq!(s.ch1.len(), 680);
                // Band 0 (category 0, uncoupled low band): outside the
                // coupling range both channels stay zero under the
                // current split (the uncoupled low-band routing is a
                // recorded docs question).
                // Band 2 (first coupled band, index j=2): the band's
                // first line carries the dequantised level split by the
                // w=4 pan pair.
                let ci = crate::category::CategoryIndex::new(live.categories[2]).unwrap();
                let val = crate::expectation::dequantise_level(ci, 1, 0, 1.0).unwrap();
                let (a, b) =
                    crate::coupling::coupling_pan_pair(CouplingPanWidth::W4, indices[0]).unwrap();
                let line = 40; // band 2's first line.
                assert!((s.ch0[line] - val * a).abs() < 1e-5);
                assert!((s.ch1[line] - val * b).abs() < 1e-5);
                let energy: f32 = s.ch0.iter().chain(s.ch1.iter()).map(|v| v * v).sum();
                assert!(energy > 0.0, "the frame decodes real energy");
            }
            other => panic!("expected stereo, got {other:?}"),
        }

        // Determinism.
        let again = decode_frame_body(&frame, &layout, Some(&inj), &[1.0]).unwrap();
        assert_eq!(&again, &body);
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
