//! Subband → spectral-coefficient-range geometry (frame-syntax §2.1 /
//! §3.1).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §2.1–§2.2 and
//! §3.1, `docs/audio/cook/provenance/07-cook-spectral-decode.md` item 1
//! (the lo/hi vector-dimension split), `provenance/09` §3 (the live band
//! count `Nb = 34` on the 1024-line / 32-subband stereo flavor), backed
//! by the extracted per-category tables
//! `docs/audio/cook/tables/category-vector-dim-{lo,hi}.{csv,meta}` and
//! `category-index-lut.{csv,meta}` (RVA `0x8c40`, 51 × u32 LE).
//!
//! ## The band width — derived from the staged category tables
//!
//! `provenance/07` pins `dim_lo` (`0x9170`, `{2,2,2,4,4,5,5}`) as the
//! per-symbol vector dimension of each spectral codebook (the codebook's
//! symbol count is exactly `(level_count+1)^dim_lo`) and records that
//! the category walk advances the coefficient cursor by `dim_lo` per
//! symbol; `dim_hi` (`0x918c`, `{10,10,10,5,5,4,4}`) is the per-band
//! symbol count. The two staged tables satisfy
//!
//! ```text
//! dim_lo[c] × dim_hi[c] = 20   for every category c = 0..=6
//! ```
//!
//! so a coded band is **20 spectral lines** regardless of its category:
//! ten 2-line vectors, five 4-line vectors, or four 5-line vectors. That
//! is the geometry this module wires ([`LINES_PER_BAND`]): band `b`
//! occupies lines `[20·b, 20·b + 20)`, the live `Nb = 34` bands of the
//! validated flavor cover 680 of its 1024 transform lines, and the
//! 51-entry `0x8c40` LUT spans exactly `floor(1024 / 20)` whole subbands
//! ([`MAX_SUBBANDS`]).
//!
//! ## The `0x8c40` LUT
//!
//! `spec/05` §2.1 describes the 51-entry monotone `0x8c40` LUT as a
//! band/position → category map *and* says it is "read as
//! `[band*4 + 0x8c40]`" by the dequant walk and the coupling split. An
//! earlier revision of this module took that second sentence literally
//! as the per-band **start line** (so bands 0..11 were one line wide and
//! 20 subbands covered 15 lines) — a reading the staged vector dimensions
//! rule out (the 20-line product above) and that leaves a 1024-sample
//! frame with a handful of spectral lines. The LUT is vendored and typed
//! in [`crate::bit_alloc`] under its pinned role (a 51-position →
//! 0..19 index map); what it indexes per subband at decode time remains
//! a recorded docs question, not a geometry this crate fabricates.
//!
//! ## What this module provides
//!
//! - [`SubbandGeometry`] — the per-stream geometry for a fixed
//!   `subband_count`: per-band line ranges, widths and the total coded
//!   line count, all multiples of [`LINES_PER_BAND`].
//! - [`SubbandGeometry::band_symbol_count`] — the §3.1 per-band vector
//!   symbol count `ceil(20 / dim)` for a category's `dim_lo`, equal to
//!   the staged `dim_hi` for every category (pinned by a test).

use crate::Error;

/// `.rdata` RVA of the 51-entry monotone category/position LUT
/// (`0x8c40`, `spec/05` §2.1). Typed in [`crate::bit_alloc`].
pub const SUBBAND_CATEGORY_LUT_RVA: u32 = 0x8c40;

/// `.rdata` RVA of the companion `0.5` scalar (`0x8c3c`, the f32 word
/// immediately before the LUT; `spec/05` §2.1 / `provenance/05`
/// evidence #4).
pub const SUBBAND_HALF_SCALAR_RVA: u32 = 0x8c3c;

/// The companion `0.5` scalar at RVA `0x8c3c` (`spec/05` §2.1).
pub const SUBBAND_HALF_SCALAR: f32 = 0.5;

/// Spectral lines per coded band — `dim_lo[c] × dim_hi[c] = 20` for
/// every category (derived from the staged
/// `category-vector-dim-{lo,hi}` tables; see the module docs).
pub const LINES_PER_BAND: u32 = 20;

/// The largest subband count the geometry admits — the 1024-line long
/// transform holds `floor(1024 / 20) = 51` whole bands, which is also
/// the length of the `0x8c40` LUT.
pub const MAX_SUBBANDS: u32 = 51;

/// Per-stream subband geometry: the band → coefficient-range map the
/// dequant walk (§2.2 / §3) and the coupling split (§4) drive off.
///
/// Built by [`SubbandGeometry::new`] from the stream's coded band count
/// (the allocator's `Nb`, 34 on the validated `FUN_RM_32.rm` stream).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubbandGeometry {
    subband_count: u32,
}

impl SubbandGeometry {
    /// Build the geometry for a stream with `subband_count` coded bands
    /// of [`LINES_PER_BAND`] lines each.
    ///
    /// # Errors
    ///
    /// - [`Error::CookieZeroSubbandCount`] when `subband_count == 0`.
    /// - [`Error::BitAllocAxisOutOfRange`] when `subband_count` exceeds
    ///   [`MAX_SUBBANDS`] (the 1024-line transform's whole-band capacity,
    ///   and the `0x8c40` LUT length).
    pub fn new(subband_count: u32) -> Result<Self, Error> {
        if subband_count == 0 {
            return Err(Error::CookieZeroSubbandCount);
        }
        if subband_count > MAX_SUBBANDS {
            return Err(Error::BitAllocAxisOutOfRange {
                got: subband_count.min(u32::from(u8::MAX)) as u8,
            });
        }
        Ok(SubbandGeometry { subband_count })
    }

    /// The stream's coded subband count.
    #[must_use]
    pub fn subband_count(&self) -> u32 {
        self.subband_count
    }

    /// The start spectral line of subband `band` — `20 · band`. Valid for
    /// `band <= subband_count` (the one-past boundary is the total).
    ///
    /// # Errors
    ///
    /// [`Error::BitAllocAxisOutOfRange`] when `band > subband_count`.
    pub fn start_line(&self, band: u32) -> Result<u32, Error> {
        if band > self.subband_count {
            return Err(Error::BitAllocAxisOutOfRange {
                got: band.min(u32::from(u8::MAX)) as u8,
            });
        }
        Ok(band * LINES_PER_BAND)
    }

    /// The half-open coefficient range `[20·band, 20·band + 20)` of
    /// subband `band`.
    ///
    /// # Errors
    ///
    /// [`Error::BitAllocAxisOutOfRange`] when `band >= subband_count`.
    pub fn line_range(&self, band: u32) -> Result<core::ops::Range<u32>, Error> {
        if band >= self.subband_count {
            return Err(Error::BitAllocAxisOutOfRange {
                got: band.min(u32::from(u8::MAX)) as u8,
            });
        }
        let start = band * LINES_PER_BAND;
        Ok(start..start + LINES_PER_BAND)
    }

    /// The number of spectral lines subband `band` occupies
    /// ([`LINES_PER_BAND`] for every coded band).
    ///
    /// # Errors
    ///
    /// [`Error::BitAllocAxisOutOfRange`] when `band >= subband_count`.
    pub fn line_count(&self, band: u32) -> Result<u32, Error> {
        let r = self.line_range(band)?;
        Ok(r.end - r.start)
    }

    /// Total coded spectral lines across all subbands —
    /// `20 · subband_count` (680 for the live `Nb = 34`).
    #[must_use]
    pub fn total_coded_lines(&self) -> u32 {
        self.subband_count * LINES_PER_BAND
    }

    /// The number of §3.1 VLC symbols subband `band` needs when each
    /// symbol expands to `dim` coefficients — `ceil(20 / dim)`
    /// (`spec/05` §3.1 grouping, via [`crate::spectral::symbols_for_band`]).
    /// For every category's `dim_lo` this equals the staged `dim_hi`.
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
        assert_eq!(SUBBAND_HALF_SCALAR_RVA + 4, SUBBAND_CATEGORY_LUT_RVA);
        assert_eq!(SUBBAND_HALF_SCALAR, 0.5);
    }

    #[test]
    fn band_width_is_the_staged_vector_dim_product() {
        // dim_lo[c] * dim_hi[c] == 20 for every category: the derivation
        // behind LINES_PER_BAND.
        let lo = crate::tables::category_vector_dim_lo();
        let hi = crate::tables::category_vector_dim_hi();
        for c in 0..7 {
            assert_eq!(lo[c] * hi[c], LINES_PER_BAND, "category {c}");
        }
        // And the LUT spans exactly floor(1024 / 20) = 51 whole subbands.
        assert_eq!(MAX_SUBBANDS, crate::tables::CATEGORY_INDEX_LUT_LEN as u32);
        assert_eq!(MAX_SUBBANDS, 1024 / LINES_PER_BAND);
    }

    #[test]
    fn geometry_rejects_zero_and_oversize() {
        assert!(matches!(
            SubbandGeometry::new(0),
            Err(Error::CookieZeroSubbandCount)
        ));
        assert!(SubbandGeometry::new(MAX_SUBBANDS).is_ok());
        assert!(matches!(
            SubbandGeometry::new(MAX_SUBBANDS + 1),
            Err(Error::BitAllocAxisOutOfRange { .. })
        ));
    }

    #[test]
    fn geometry_boundaries_tile_without_gap() {
        // The per-band ranges tile [0, total) with no gap or overlap.
        let geom = SubbandGeometry::new(34).unwrap();
        assert_eq!(geom.subband_count(), 34);
        let mut expected_start = 0u32;
        for band in 0..geom.subband_count() {
            let r = geom.line_range(band).unwrap();
            assert_eq!(r.start, expected_start, "gap before band {band}");
            assert_eq!(geom.line_count(band).unwrap(), LINES_PER_BAND);
            expected_start = r.end;
        }
        assert_eq!(expected_start, geom.total_coded_lines());
        // The live Nb = 34 covers 680 of the 1024 transform lines.
        assert_eq!(geom.total_coded_lines(), 680);
        assert!(geom.total_coded_lines() <= 1024);
    }

    #[test]
    fn geometry_line_range_rejects_last_and_beyond() {
        let geom = SubbandGeometry::new(20).unwrap();
        assert!(matches!(
            geom.line_range(20),
            Err(Error::BitAllocAxisOutOfRange { .. })
        ));
        // start_line(subband_count) IS valid (the one-past boundary).
        assert_eq!(geom.start_line(20).unwrap(), 400);
        assert!(geom.start_line(21).is_err());
    }

    #[test]
    fn band_symbol_count_equals_staged_dim_hi() {
        // ceil(20 / dim_lo[c]) == dim_hi[c] for every category — the
        // per-band symbol count the spectral walk reads.
        let geom = SubbandGeometry::new(34).unwrap();
        let lo = crate::tables::category_vector_dim_lo();
        let hi = crate::tables::category_vector_dim_hi();
        for c in 0..7 {
            for band in [0u32, 17, 33] {
                assert_eq!(
                    geom.band_symbol_count(band, lo[c]).unwrap(),
                    hi[c],
                    "category {c} band {band}"
                );
            }
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
}
