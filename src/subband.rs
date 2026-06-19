//! Subband → spectral-coefficient-range geometry (frame-syntax §2.1).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §2.1 (the
//! subband axis) and `docs/audio/cook/provenance/05-cook-backend.md`
//! evidence #4 (*"`0x3dc0`/`0x3d10`/`0x4600` index `[band*4 +
//! 0x60bd8c40]`; scalar `0.5` at `0x8c3c`"*), backed by the extracted
//! `docs/audio/cook/tables/category-index-lut.{csv,meta}` (RVA `0x8c40`,
//! 51 × u32 LE).
//!
//! ## What the trace pins (wired here)
//!
//! §2.1 records that the **same** monotone LUT at RVA `0x8c40` that
//! [`crate::bit_alloc`] reads as a category map is *also* — read as
//! `[band*4 + 0x8c40]` — the **start spectral line of each subband**:
//!
//! > *"it maps a band/position index onto the 20 quantiser categories
//! > and, read as `[band*4 + 0x8c40]`, also gives the start spectral
//! > line of each subband (it is the identity `0..11` over the first
//! > twelve subbands and then compresses)."*
//!
//! Both the per-band dequant walk (§2.2, worker `0x4600`) and the
//! joint-stereo coupling split (§4, worker `0x3dc0`/`0x3d10`) use this
//! table to turn a subband index into the half-open coefficient range
//! `[start_line[band] .. start_line[band + 1])` the band occupies. The
//! companion scalar `0.5` (f32) at RVA `0x8c3c` (the word immediately
//! before the table) is surfaced as [`SUBBAND_HALF_SCALAR`].
//!
//! This module turns those facts into a typed accessor:
//!
//! - [`subband_start_line`] — the start spectral line of one subband
//!   (`start_line[band] = lut[band]`), read from the vendored LUT.
//! - [`subband_line_range`] — the half-open `[start, end)` coefficient
//!   range one subband occupies, `start_line[band] .. start_line[band+1]`.
//! - [`SubbandGeometry`] — the per-stream geometry for a fixed
//!   `subband_count`: it caches the `subband_count + 1` boundary lines
//!   and answers per-band range / width queries plus the total coded
//!   spectral-line count.
//!
//! ## What stays a GAP (not wired)
//!
//! The §2.2 *category-assignment* bit-allocation loop (which per-band
//! category each subband is assigned, driven by the in-decoder
//! bit-budget loop keyed off the `0x8f38` per-category expected-cost LUT)
//! is **not** wired here — `0x8f38` is not among the extracted tables and
//! the loop is a recorded DOCS-GAP (§2.2). This module pins only the
//! static band → coefficient-range *geometry* the trace records
//! verbatim, the mapping every later stage consumes.
//!
//! ## Wall-respect note
//!
//! Every fact here is anchored to `spec/05` §2.1 / `provenance/05`
//! evidence #4 and the vendored `category-index-lut` table; no
//! algorithmic content beyond the pinned band → line geometry is wired.

use crate::{tables::category_index_lut, Error};

/// `.rdata` RVA of the subband start-line / category LUT (`0x8c40`,
/// `spec/05` §2.1). Shared with [`crate::bit_alloc`] (the same table,
/// read for its two roles).
pub const SUBBAND_START_LINE_LUT_RVA: u32 = 0x8c40;

/// `.rdata` RVA of the companion `0.5` scalar (`0x8c3c`, the f32 word
/// immediately before the LUT; `spec/05` §2.1 / `provenance/05`
/// evidence #4).
pub const SUBBAND_HALF_SCALAR_RVA: u32 = 0x8c3c;

/// The companion `0.5` scalar at RVA `0x8c3c` (`spec/05` §2.1). Used
/// alongside the start-line LUT to convert a subband index to a
/// coefficient range; surfaced as the pinned constant (the value is the
/// f32 `0.5`, one word before the LUT head).
pub const SUBBAND_HALF_SCALAR: f32 = 0.5;

/// Number of subbands whose start line is the identity index
/// (`spec/05` §2.1: *"the identity `0..11` over the first twelve
/// subbands and then compresses"*). For `band < 12`,
/// `start_line[band] == band`.
pub const SUBBAND_IDENTITY_RUN: u32 = 12;

/// The start spectral line of one subband — `start_line[band] =
/// lut[band]` from the `0x8c40` LUT read as `[band*4 + 0x8c40]`
/// (`spec/05` §2.1).
///
/// The first twelve entries are the identity `0..=11`; the table then
/// compresses (the `category-index-lut.csv` run-length pattern). The
/// underlying CSV load is `OnceLock`-cached.
///
/// # Errors
///
/// Returns [`Error::BitAllocAxisOutOfRange`] when `band` is at or past
/// the 51-entry LUT length (the same bound [`crate::bit_alloc`] enforces
/// — the band index is a position into the one shared table).
pub fn subband_start_line(band: u32) -> Result<u32, Error> {
    let lut = category_index_lut();
    let idx = band as usize;
    if idx >= lut.len() {
        return Err(Error::BitAllocAxisOutOfRange {
            got: band.min(u32::from(u8::MAX)) as u8,
        });
    }
    Ok(lut[idx])
}

/// The half-open coefficient range `[start, end)` one subband occupies —
/// `start_line[band] .. start_line[band + 1]` (`spec/05` §2.1).
///
/// `band` and `band + 1` are both read from the start-line LUT; the
/// band's spectral-line span is the gap between consecutive boundaries.
///
/// # Errors
///
/// Returns [`Error::BitAllocAxisOutOfRange`] when `band` or `band + 1`
/// is at or past the LUT length.
pub fn subband_line_range(band: u32) -> Result<core::ops::Range<u32>, Error> {
    let start = subband_start_line(band)?;
    let end = subband_start_line(band + 1)?;
    Ok(start..end)
}

/// Per-stream subband geometry: the `subband_count + 1` boundary lines
/// of a fixed-`subband_count` stream and the per-band range queries the
/// dequant walk (§2.2) and the coupling split (§4) drive off it.
///
/// Built by [`SubbandGeometry::new`] from the stream's `subband_count`
/// (the flavor-record / cookie field, e.g. `20` on the validated
/// `FUN_RM_32.rm` stream). The boundaries are read once from the
/// vendored `0x8c40` LUT and cached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubbandGeometry {
    /// `subband_count + 1` consecutive start lines: `boundaries[b]` is
    /// the start line of subband `b`, `boundaries[subband_count]` is the
    /// one-past-the-last coded line (the total coded line count).
    boundaries: Vec<u32>,
}

impl SubbandGeometry {
    /// Build the geometry for a stream with `subband_count` coded
    /// subbands.
    ///
    /// Reads `subband_count + 1` consecutive boundary lines from the
    /// `0x8c40` start-line LUT. Per `spec/05` §2.1 the first twelve are
    /// the identity `0..=11` and the table then compresses, so the
    /// boundaries are non-decreasing.
    ///
    /// # Errors
    ///
    /// - [`Error::CookieZeroSubbandCount`] when `subband_count == 0`
    ///   (no coded band — the same structural rejection
    ///   [`crate::CookCookie::validate_geometry`] applies).
    /// - [`Error::BitAllocAxisOutOfRange`] when `subband_count + 1`
    ///   exceeds the 51-entry LUT length.
    pub fn new(subband_count: u32) -> Result<Self, Error> {
        if subband_count == 0 {
            return Err(Error::CookieZeroSubbandCount);
        }
        let mut boundaries = Vec::with_capacity(subband_count as usize + 1);
        for b in 0..=subband_count {
            boundaries.push(subband_start_line(b)?);
        }
        Ok(SubbandGeometry { boundaries })
    }

    /// The stream's coded subband count.
    #[must_use]
    pub fn subband_count(&self) -> u32 {
        (self.boundaries.len() - 1) as u32
    }

    /// The start spectral line of subband `band`.
    ///
    /// # Errors
    ///
    /// [`Error::BitAllocAxisOutOfRange`] when `band > subband_count`.
    pub fn start_line(&self, band: u32) -> Result<u32, Error> {
        self.boundaries
            .get(band as usize)
            .copied()
            .ok_or(Error::BitAllocAxisOutOfRange {
                got: band.min(u32::from(u8::MAX)) as u8,
            })
    }

    /// The half-open coefficient range `[start, end)` of subband `band`.
    ///
    /// # Errors
    ///
    /// [`Error::BitAllocAxisOutOfRange`] when `band >= subband_count`
    /// (the last band's `end` boundary is `boundaries[subband_count]`).
    pub fn line_range(&self, band: u32) -> Result<core::ops::Range<u32>, Error> {
        if band >= self.subband_count() {
            return Err(Error::BitAllocAxisOutOfRange {
                got: band.min(u32::from(u8::MAX)) as u8,
            });
        }
        Ok(self.boundaries[band as usize]..self.boundaries[band as usize + 1])
    }

    /// The number of spectral lines subband `band` occupies (the width
    /// of its [`SubbandGeometry::line_range`]).
    ///
    /// # Errors
    ///
    /// [`Error::BitAllocAxisOutOfRange`] when `band >= subband_count`.
    pub fn line_count(&self, band: u32) -> Result<u32, Error> {
        let r = self.line_range(band)?;
        Ok(r.end - r.start)
    }

    /// Total coded spectral lines across all subbands — the one-past-the
    /// -last boundary `boundaries[subband_count]`.
    #[must_use]
    pub fn total_coded_lines(&self) -> u32 {
        *self.boundaries.last().expect("at least one boundary")
    }

    /// The number of §3.1 VLC symbols subband `band` needs when each
    /// symbol expands to `dim` coefficients —
    /// `ceil(line_count(band) / dim)` (`spec/05` §3.1 grouping, via
    /// [`crate::spectral::symbols_for_band`]).
    ///
    /// `dim` is the band's per-category vector dimension (one of the §2.2
    /// [`crate::spectral::CategoryVectorDims`] branches, `2..=10`); it is
    /// supplied by the caller because the category-assignment loop and the
    /// lo-or-hi branch selection are recorded §2.2 / §3.1 GAPs. This ties
    /// the pinned §2.1 band geometry to the pinned §3.1 grouping
    /// arithmetic, sizing the dequant walk's per-band symbol read.
    ///
    /// # Errors
    ///
    /// - [`Error::BitAllocAxisOutOfRange`] when `band >= subband_count`.
    /// - [`Error::SpectralVectorDimZero`] when `dim == 0`.
    pub fn band_symbol_count(&self, band: u32, dim: u32) -> Result<u32, Error> {
        let line_count = self.line_count(band)?;
        crate::spectral::symbols_for_band(line_count, dim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rva_constants_are_adjacent_words() {
        // `0x8c3c` (the 0.5 scalar) is the f32 word immediately before
        // the LUT head `0x8c40`.
        assert_eq!(SUBBAND_HALF_SCALAR_RVA + 4, SUBBAND_START_LINE_LUT_RVA);
        assert_eq!(SUBBAND_HALF_SCALAR, 0.5);
    }

    #[test]
    fn start_line_is_identity_over_first_twelve() {
        // spec/05 §2.1: identity 0..11 over the first twelve subbands.
        for band in 0..SUBBAND_IDENTITY_RUN {
            assert_eq!(subband_start_line(band).unwrap(), band, "band {band}");
        }
    }

    #[test]
    fn start_line_compresses_after_twelve() {
        // After the identity run the table compresses: consecutive
        // boundaries grow by 0 or 1 (the run-length pattern in the CSV).
        let mut prev = subband_start_line(0).unwrap();
        for band in 1..51 {
            let cur = subband_start_line(band).unwrap();
            assert!(cur >= prev, "not monotone at band {band}");
            assert!(cur - prev <= 1, "boundary jumped >1 at band {band}");
            prev = cur;
        }
    }

    #[test]
    fn start_line_rejects_out_of_range_band() {
        // 51-entry LUT: band 51 and beyond are out of range.
        assert!(matches!(
            subband_start_line(51),
            Err(Error::BitAllocAxisOutOfRange { .. })
        ));
    }

    #[test]
    fn line_range_is_consecutive_boundaries() {
        for band in 0..50 {
            let r = subband_line_range(band).unwrap();
            assert_eq!(r.start, subband_start_line(band).unwrap());
            assert_eq!(r.end, subband_start_line(band + 1).unwrap());
        }
    }

    #[test]
    fn geometry_rejects_zero_subbands() {
        assert!(matches!(
            SubbandGeometry::new(0),
            Err(Error::CookieZeroSubbandCount)
        ));
    }

    #[test]
    fn geometry_boundaries_tile_without_gap() {
        // The per-band ranges tile [0, total) with no gap or overlap:
        // band b ends exactly where band b+1 starts.
        let geom = SubbandGeometry::new(20).unwrap();
        assert_eq!(geom.subband_count(), 20);
        let mut expected_start = geom.start_line(0).unwrap();
        let mut walked = 0u32;
        for band in 0..geom.subband_count() {
            let r = geom.line_range(band).unwrap();
            assert_eq!(r.start, expected_start, "gap before band {band}");
            walked += geom.line_count(band).unwrap();
            expected_start = r.end;
        }
        // The walked total equals the last boundary minus the first.
        assert_eq!(
            walked,
            geom.total_coded_lines() - geom.start_line(0).unwrap()
        );
    }

    #[test]
    fn geometry_total_lines_matches_last_boundary() {
        let geom = SubbandGeometry::new(20).unwrap();
        assert_eq!(geom.total_coded_lines(), subband_start_line(20).unwrap());
    }

    #[test]
    fn geometry_line_range_rejects_last_and_beyond() {
        let geom = SubbandGeometry::new(20).unwrap();
        // band == subband_count has no `end` boundary in range terms.
        assert!(matches!(
            geom.line_range(20),
            Err(Error::BitAllocAxisOutOfRange { .. })
        ));
        // start_line(subband_count) IS valid (the one-past boundary).
        assert!(geom.start_line(20).is_ok());
        assert!(geom.start_line(21).is_err());
    }

    #[test]
    fn band_symbol_count_ties_geometry_to_grouping() {
        // Each band's symbol count is ceil(line_count / dim); for the
        // unit-width first twelve bands a dim of 2 needs exactly 1 symbol
        // (1 line over-covered by a 2-coefficient symbol).
        let geom = SubbandGeometry::new(20).unwrap();
        for band in 0..(SUBBAND_IDENTITY_RUN - 1) {
            assert_eq!(geom.line_count(band).unwrap(), 1);
            assert_eq!(geom.band_symbol_count(band, 2).unwrap(), 1, "band {band}");
            assert_eq!(geom.band_symbol_count(band, 10).unwrap(), 1, "band {band}");
        }
    }

    #[test]
    fn band_symbol_count_rejects_bad_inputs() {
        let geom = SubbandGeometry::new(20).unwrap();
        assert!(matches!(
            geom.band_symbol_count(20, 2),
            Err(Error::BitAllocAxisOutOfRange { .. })
        ));
        assert_eq!(
            geom.band_symbol_count(0, 0).unwrap_err(),
            Error::SpectralVectorDimZero
        );
    }

    #[test]
    fn geometry_first_twelve_bands_are_unit_width() {
        // Identity run: each of the first 12 boundaries is the identity,
        // so each of bands 0..11 is exactly one spectral line wide.
        let geom = SubbandGeometry::new(20).unwrap();
        for band in 0..(SUBBAND_IDENTITY_RUN - 1) {
            assert_eq!(geom.line_count(band).unwrap(), 1, "band {band}");
        }
    }
}
