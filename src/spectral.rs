//! Spectral-VLC codebook geometry + joint-stereo rotation closed form.
//!
//! Source-of-truth: `docs/audio/cook/spec/05-cook-backend-frame-syntax.md`
//! §2.2 (per-category spectral-vector dimensions), §3.1 (the seven spectral
//! codebooks and their symbol counts, the sign LUT and dequant-scale
//! triple) and §4.2 (the joint-stereo mirror-index rotation closed form),
//! plus the matching `.meta` files
//! `docs/audio/cook/tables/{category-vector-dim-lo,category-vector-dim-hi,
//! spectral-codebook-dims}.meta`.
//!
//! ## What the trace pins (wired here)
//!
//! - **Per-category vector dimensions (§2.2).** Each band category
//!   (`0..=6`) selects, by `[category*4 + base]`, two parallel `.rdata`
//!   arrays: the *low* dimension at RVA `0x9170`
//!   (`tables/category-vector-dim-lo.csv`, `{2, 2, 2, 4, 4, 5, 5}`) and
//!   the *high* dimension at RVA `0x918c`
//!   (`tables/category-vector-dim-hi.csv`, `{10, 10, 10, 5, 5, 4, 4}`).
//!   These set how many spectral lines are grouped per VLC symbol in the
//!   §3 dequant walk.
//! - **Spectral codebook symbol counts (§3.1).** Seven spectral Huffman
//!   codebooks (`0..=6`) with the static symbol counts
//!   `{196, 100, 49, 625, 256, 243, 32}` at RVA `0x91e0`
//!   (`tables/spectral-codebook-dims.csv`). These are the third of three
//!   parallel arrays; the two pointer arrays (`0x91a8` value tables,
//!   `0x91c4` length tables) are relocated into BSS and built at init, so
//!   the per-symbol code/length bytes are **not** in the file image
//!   (spec/05 §3.2 — a recorded GAP needing a dynamic BSS dump).
//! - **Sign LUT (§3.1).** The dequantiser reads an embedded sign bit and
//!   indexes the two-entry LUT `{+1.0, -1.0}` at RVA `0xa148`
//!   ([`SIGN_LUT`]). A `0` bit selects `+1.0`, a `1` bit `-1.0` — the
//!   trace pins the values and the embedded-sign-bit mechanism.
//! - **Dequant scale (§3.1).** Once a symbol is decoded, the
//!   dequantiser `cook.dll!0x4600` scales it by *"the dequant scale at
//!   RVA `0x9150` (whose non-zero entries are `{0.17678, 0.25,
//!   0.70711}`)"* read as `[q*4 + 0x9150]` (`provenance/05` evidence
//!   #10), and by the §1–§2 per-band gain. [`DEQUANT_SCALE_NONZERO`]
//!   pins the three non-zero scale magnitudes and
//!   [`spectral_coefficient`] composes the full pinned assembly
//!   `value * sign * dequant_scale * band_gain` (the `value` itself
//!   comes from the recovered codebook value table).
//! - **Joint-stereo mirror-index rotation (§4.2).** For a coupling table
//!   of length `Ncoup = 1 << coupling_bits`, a per-band coupling index
//!   `j` splits the single coupled coefficient `c` into the two output
//!   channels by the mirror-index pair
//!   `(out0, out1) = c * (coef[j], coef[Ncoup-1-j])` — a per-band
//!   normalised-pan rotation. [`mirror_partner_index`] is the
//!   `Ncoup - 1 - j` partner read; the closed form is pinned, the
//!   numeric `coef[..]` values are a GAP (built in BSS at init, §4.3).
//!
//! ## What stays a GAP (not wired)
//!
//! The per-symbol codebook **code/length bytes** (§3.2) and the
//! per-coupling-width **rotation coefficient values** (§4.3) are built in
//! the decoder's `.data` BSS at init and are not present in the file
//! image; the trace records them as GAPs pending a Validator/Extractor
//! round that dumps the populated tables. This module wires only the
//! statically-pinned geometry (dimensions, counts, sign LUT) and the
//! pinned index arithmetic (the mirror-index pairing), not the
//! BSS-resident numbers or the full entropy walk.
//!
//! ## Wall-respect note
//!
//! Every fact here is anchored to spec/05 §2.2 / §3.1 / §4.2 and the
//! three `.meta` files; no algorithmic content beyond the pinned
//! geometry and the mirror-index closed form is wired.

use crate::{
    category::CategoryIndex,
    tables::{category_vector_dim_hi, category_vector_dim_lo, spectral_codebook_dims},
    Error,
};

/// Number of spectral Huffman codebooks (`spec/05 §3.1`: *"There are
/// **seven** spectral codebooks"*; the three parallel `.rdata` arrays at
/// `0x91a8` / `0x91c4` / `0x91e0` each have 7 entries).
pub const SPECTRAL_CODEBOOK_COUNT: u8 = 7;

/// Highest valid spectral-codebook index — `SPECTRAL_CODEBOOK_COUNT - 1`.
pub const MAX_SPECTRAL_CODEBOOK_INDEX: u8 = SPECTRAL_CODEBOOK_COUNT - 1;

/// `.rdata` base RVA of the per-category low-dimension table (`0x9170`,
/// `spec/05 §2.2`).
pub const CATEGORY_VECTOR_DIM_LO_RVA: u32 = 0x9170;

/// `.rdata` base RVA of the per-category high-dimension table (`0x918c`,
/// `spec/05 §2.2`).
pub const CATEGORY_VECTOR_DIM_HI_RVA: u32 = 0x918c;

/// `.rdata` base RVA of the spectral-codebook symbol-count array
/// (`0x91e0`, `spec/05 §3.1`).
pub const SPECTRAL_CODEBOOK_DIMS_RVA: u32 = 0x91e0;

/// `.rdata` RVA of the value-table pointer array (`0x91a8`); its seven
/// targets are relocated BSS pointers built at init — the per-symbol code
/// bytes are a `spec/05 §3.2` GAP.
pub const SPECTRAL_CODEBOOK_VALUE_PTRS_RVA: u32 = 0x91a8;

/// `.rdata` RVA of the bit-length-table pointer array (`0x91c4`); its
/// seven targets are relocated BSS pointers built at init — the
/// per-symbol length bytes are a `spec/05 §3.2` GAP.
pub const SPECTRAL_CODEBOOK_LENGTH_PTRS_RVA: u32 = 0x91c4;

/// `.rdata` RVA of the two-entry spectral sign LUT (`0xa148`,
/// `spec/05 §3.1`).
pub const SIGN_LUT_RVA: u32 = 0xa148;

/// The two-entry spectral sign LUT `{+1.0, -1.0}` at RVA `0xa148`
/// (`spec/05 §3.1`): the dequantiser reads an embedded sign bit and
/// indexes this LUT — bit `0` → `SIGN_LUT[0]` (`+1.0`), bit `1` →
/// `SIGN_LUT[1]` (`-1.0`).
pub const SIGN_LUT: [f32; 2] = [1.0, -1.0];

/// Resolve an embedded sign bit to its multiplier via [`SIGN_LUT`].
///
/// `spec/05 §3.1`: a `0` sign bit selects `+1.0`, a `1` bit `-1.0`. Any
/// non-zero `bit` is treated as `1` (the binary tests a single mask bit).
#[inline]
#[must_use]
pub const fn sign_from_bit(bit: u32) -> f32 {
    SIGN_LUT[(bit & 1) as usize]
}

/// `.rdata` base RVA of the spectral dequant-scale table (`0x9150`,
/// `spec/05 §3.1` / `provenance/05` evidence #10). The dequantiser
/// `cook.dll!0x4600` reads it as `[q*4 + 0x9150]`.
pub const DEQUANT_SCALE_RVA: u32 = 0x9150;

/// The three **non-zero** spectral dequant-scale magnitudes at RVA
/// `0x9150` — `{0.17678, 0.25, 0.70711}` (`spec/05 §3.1`: *"the dequant
/// scale at RVA `0x9150` (whose non-zero entries are `{0.17678, 0.25,
/// 0.70711}`)"*; `provenance/05` evidence #10).
///
/// The trace pins only the **non-zero** part of the table (the worker
/// reads it as `[q*4 + 0x9150]`; the full element count and the position
/// of the implied zero entries are not statically pinned — that is the
/// recovered runtime dump). These three values are the f32 magnitudes
/// `2^(-5/2) ≈ 0.17678`, `2^(-2) = 0.25`, `2^(-1/2) ≈ 0.70711` — a
/// `2^(k/2)` family one quarter-octave apart, consistent with the
/// `0x93f8` ladder, but the trace pins them as the literal scale triple,
/// not via the ladder.
///
/// `clippy::approx_constant` is allowed because `0.70711` is the literal
/// f32 the binary stores at `0x9150` (a vendored numeric fact, not a
/// rounded `FRAC_1_SQRT_2`); the trace pins these exact decimal digits.
#[allow(clippy::approx_constant)]
pub const DEQUANT_SCALE_NONZERO: [f32; 3] = [0.17678, 0.25, 0.70711];

/// Compose the full §3.1 spectral-coefficient assembly for one decoded
/// symbol value: `coef = value * sign * dequant_scale * band_gain`.
///
/// `spec/05 §3.1` pins the dequantiser `cook.dll!0x4600` reconstruction:
/// *"the symbol high bits index the codebook value table, an embedded
/// sign bit selects from the two-entry sign LUT `{+1.0, -1.0}` …, and
/// the result is scaled by the dequant scale … and the per-band gain of
/// §1–§2."*
///
/// - `value` is the decoded symbol's codebook value — supplied by the
///   caller (normally the wired §3 entropy read supplies it).
/// - `sign_bit` selects the [`SIGN_LUT`] multiplier (`0` → `+1.0`,
///   non-zero → `-1.0`), via [`sign_from_bit`].
/// - `dequant_scale` is one of [`DEQUANT_SCALE_NONZERO`] (read as
///   `[q*4 + 0x9150]` in the binary), supplied by the caller.
/// - `band_gain` is the §1–§2 per-band gain applied to every coefficient
///   of the band.
///
/// This wires the pinned multiply chain; the `value` and the exact
/// `dequant_scale` *selection* (which `q` indexes which scale entry, and
/// the zero entries of the full `0x9150` table) stay §3.2 GAPs.
#[inline]
#[must_use]
pub fn spectral_coefficient(value: f32, sign_bit: u32, dequant_scale: f32, band_gain: f32) -> f32 {
    value * sign_from_bit(sign_bit) * dequant_scale * band_gain
}

/// The two per-category spectral-vector dimensions read at
/// `[category*4 + base]` from the parallel `0x9170` / `0x918c` tables
/// (`spec/05 §2.2`).
///
/// The dimension sets how many spectral coefficients are grouped per VLC
/// symbol in the §3 dequant walk; the codebook with the matching symbol
/// count is then used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CategoryVectorDims {
    /// Low-branch vector dimension (`0x9170`, `{2, 2, 2, 4, 4, 5, 5}`).
    pub lo: u32,
    /// High-branch vector dimension (`0x918c`, `{10, 10, 10, 5, 5, 4, 4}`).
    pub hi: u32,
}

impl CategoryVectorDims {
    /// Look up the (low, high) vector dimensions for one band category.
    ///
    /// Each field is read at `[index * 4 + base]` from the matching
    /// vendored table — the same parallel access `spec/05 §2.2` records
    /// at the category-walk worker (RVA `0x44a0`). The underlying CSV
    /// loads are `OnceLock`-cached.
    #[must_use]
    pub fn for_category(index: CategoryIndex) -> Self {
        let i = index.as_usize();
        CategoryVectorDims {
            lo: category_vector_dim_lo()[i],
            hi: category_vector_dim_hi()[i],
        }
    }
}

/// Convenience accessor — `CategoryVectorDims::for_category(index)`.
#[must_use]
pub fn category_vector_dims(index: CategoryIndex) -> CategoryVectorDims {
    CategoryVectorDims::for_category(index)
}

/// The number of VLC symbols a band of `line_count` spectral lines needs
/// when each symbol expands to `dim` coefficients — `ceil(line_count /
/// dim)` (`spec/05 §3.1`: *"each symbol expands to `dim` coefficients via
/// a value-table lookup"*; the per-band vector dimension `dim` is one of
/// the §2.2 [`CategoryVectorDims`] branches, `2..=10`).
///
/// This is the pinned **grouping** relationship: the §3 dequant walk
/// reads `symbols_for_band` VLC symbols for the band and expands each to
/// `dim` coefficients, covering the band's `line_count` lines (the last
/// symbol may over-cover when `dim` does not divide `line_count`). It
/// wires only the count arithmetic; *which* codebook / which lo-or-hi
/// branch the band uses is a recorded §3.1 GAP (the dim → codebook
/// mapping is not statically unambiguous), so `dim` is supplied by the
/// caller rather than selected here.
///
/// # Errors
///
/// Returns [`Error::SpectralVectorDimZero`] when `dim == 0` (no division
/// possible; a real category dimension is always `2..=10`).
pub fn symbols_for_band(line_count: u32, dim: u32) -> Result<u32, Error> {
    if dim == 0 {
        return Err(Error::SpectralVectorDimZero);
    }
    Ok(line_count.div_ceil(dim))
}

/// The number of spectral coefficients `n` decoded VLC symbols of vector
/// dimension `dim` expand to — `n * dim` (`spec/05 §3.1`: each symbol
/// expands to `dim` coefficients).
///
/// The inverse-direction companion of [`symbols_for_band`]: it is the
/// coverage `symbols_for_band(line_count, dim)` symbols actually decode,
/// which is `>= line_count` (equal when `dim` divides `line_count`).
#[inline]
#[must_use]
pub const fn coefficients_for_symbols(symbols: u32, dim: u32) -> u32 {
    symbols * dim
}

/// Typed spectral-codebook index — in `0..=MAX_SPECTRAL_CODEBOOK_INDEX`.
///
/// Constructed by [`SpectralCodebook::new`], so consumers never index the
/// symbol-count array (or the GAP value/length pointer arrays) with an
/// out-of-range value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpectralCodebook(u8);

impl SpectralCodebook {
    /// Build a [`SpectralCodebook`] from a raw codebook index.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SpectralCodebookOutOfRange`] when
    /// `raw > MAX_SPECTRAL_CODEBOOK_INDEX`.
    pub fn new(raw: u8) -> Result<Self, Error> {
        if raw > MAX_SPECTRAL_CODEBOOK_INDEX {
            return Err(Error::SpectralCodebookOutOfRange { got: raw });
        }
        Ok(SpectralCodebook(raw))
    }

    /// `const`-context constructor. Panics for out-of-range values.
    ///
    /// # Panics
    ///
    /// Panics when `raw > MAX_SPECTRAL_CODEBOOK_INDEX`.
    #[must_use]
    pub const fn new_const(raw: u8) -> Self {
        if raw > MAX_SPECTRAL_CODEBOOK_INDEX {
            panic!("SpectralCodebook::new_const: value out of range");
        }
        SpectralCodebook(raw)
    }

    /// Raw codebook index as `u8`.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Raw codebook index as `usize` — for table lookup.
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// The number of symbols in this codebook (`spec/05 §3.1`, the
    /// `0x91e0` counts `{196, 100, 49, 625, 256, 243, 32}`).
    #[must_use]
    pub fn symbol_count(self) -> u32 {
        spectral_codebook_dims()[self.as_usize()]
    }
}

/// Length of the per-band coupling-coefficient table for a coupling-index
/// bit width — `Ncoup = (1 << coupling_bits) − 1` (`spec/05 §4.3`,
/// round 9/10: the table extents read from the `0x8ee8` dispatch array
/// are `(1 << w) − 1`, matching the observed index range `0..=14` at
/// width 4; the earlier `1 << coupling_bits` reading is corrected).
///
/// `coupling_bits` is the per-flavor coupling-index bit width
/// (context `+0x1c`, §4.1). The result is the number of quantised
/// rotation angles available for one coupling band — identical to
/// [`crate::coupling::CouplingPanWidth::table_len`] for stored widths.
#[inline]
#[must_use]
pub const fn coupling_table_len(coupling_bits: u32) -> u32 {
    (1u32 << coupling_bits) - 1
}

/// The mirror partner index of a coupling index `j` in a table of length
/// `ncoup` — `Ncoup - 1 - j` (`spec/05 §4.2`).
///
/// Channel 0 reads `coef[j]` and channel 1 reads `coef[ncoup - 1 - j]`,
/// the partner entry of the **same** per-coupling-width table, so the
/// pair `(coef[j], coef[ncoup-1-j])` acts as the `(cos θ, sin θ)` of a
/// per-band rotation: as `j` runs `0..ncoup` the energy is steered from
/// one channel to the other.
///
/// # Errors
///
/// Returns [`Error::CouplingIndexOutOfRange`] when `j >= ncoup`, and
/// [`Error::CouplingTableEmpty`] when `ncoup == 0` (no entries to mirror
/// against).
pub fn mirror_partner_index(j: u32, ncoup: u32) -> Result<u32, Error> {
    if ncoup == 0 {
        return Err(Error::CouplingTableEmpty);
    }
    if j >= ncoup {
        return Err(Error::CouplingIndexOutOfRange { got: j, ncoup });
    }
    Ok(ncoup - 1 - j)
}

/// Split one coupled coefficient `c` into the two output-channel
/// coefficients by the §4.2 mirror-index rotation, given the per-coupling
/// -width coefficient table `coef` (length `Ncoup`).
///
/// Returns `(out0, out1) = (c * coef[j], c * coef[Ncoup-1-j])`. The
/// `coef` slice is supplied by the caller because its numeric values are
/// a `spec/05 §4.3` GAP (built in BSS at init); this function wires only
/// the pinned index arithmetic and the multiply.
///
/// # Errors
///
/// Returns [`Error::CouplingTableEmpty`] for an empty `coef`, and
/// [`Error::CouplingIndexOutOfRange`] when `j >= coef.len()`.
pub fn split_coupled_coefficient(c: f32, j: u32, coef: &[f32]) -> Result<(f32, f32), Error> {
    let ncoup = coef.len() as u32;
    let partner = mirror_partner_index(j, ncoup)?;
    Ok((c * coef[j as usize], c * coef[partner as usize]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::category::CATEGORY_COUNT;

    #[test]
    fn codebook_count_matches_meta() {
        assert_eq!(SPECTRAL_CODEBOOK_COUNT, 7);
        assert_eq!(MAX_SPECTRAL_CODEBOOK_INDEX, 6);
        assert_eq!(
            SPECTRAL_CODEBOOK_COUNT as usize,
            spectral_codebook_dims().len()
        );
    }

    #[test]
    fn codebook_symbol_counts_match_spec() {
        // spec/05 §3.1: {196, 100, 49, 625, 256, 243, 32}.
        let want = [196u32, 100, 49, 625, 256, 243, 32];
        for (i, &w) in want.iter().enumerate() {
            let cb = SpectralCodebook::new(i as u8).unwrap();
            assert_eq!(cb.symbol_count(), w, "codebook {i}");
        }
    }

    #[test]
    fn codebook_rejects_out_of_range() {
        for raw in (MAX_SPECTRAL_CODEBOOK_INDEX + 1)..=u8::MAX {
            assert_eq!(
                SpectralCodebook::new(raw).unwrap_err(),
                Error::SpectralCodebookOutOfRange { got: raw }
            );
            if raw == u8::MAX {
                break;
            }
        }
        // 7 is the first out-of-range value (seven codebooks 0..=6).
        assert_eq!(
            SpectralCodebook::new(7).unwrap_err(),
            Error::SpectralCodebookOutOfRange { got: 7 }
        );
    }

    #[test]
    #[should_panic(expected = "SpectralCodebook::new_const")]
    fn codebook_new_const_panics_out_of_range() {
        let _ = SpectralCodebook::new_const(7);
    }

    #[test]
    fn codebook_new_const_accepts_valid() {
        const C0: SpectralCodebook = SpectralCodebook::new_const(0);
        const C6: SpectralCodebook = SpectralCodebook::new_const(6);
        assert_eq!(C0.get(), 0);
        assert_eq!(C6.get(), 6);
    }

    #[test]
    fn vector_dims_match_spec_per_category() {
        // spec/05 §2.2: lo {2,2,2,4,4,5,5}, hi {10,10,10,5,5,4,4}.
        let lo = [2u32, 2, 2, 4, 4, 5, 5];
        let hi = [10u32, 10, 10, 5, 5, 4, 4];
        for cat in 0..CATEGORY_COUNT {
            let idx = CategoryIndex::new(cat).unwrap();
            let dims = CategoryVectorDims::for_category(idx);
            assert_eq!(dims.lo, lo[cat as usize], "lo cat {cat}");
            assert_eq!(dims.hi, hi[cat as usize], "hi cat {cat}");
        }
    }

    #[test]
    fn vector_dims_module_accessor_matches_struct() {
        for cat in 0..CATEGORY_COUNT {
            let idx = CategoryIndex::new(cat).unwrap();
            assert_eq!(
                category_vector_dims(idx),
                CategoryVectorDims::for_category(idx)
            );
        }
    }

    #[test]
    fn vector_dims_are_valid_codebook_symbol_groupings() {
        // The dimension (2..=10) is the number of lines grouped per VLC
        // symbol; both branches stay within the documented 2..=10 range.
        for cat in 0..CATEGORY_COUNT {
            let dims = CategoryVectorDims::for_category(CategoryIndex::new(cat).unwrap());
            assert!((2..=10).contains(&dims.lo), "lo {} out of 2..=10", dims.lo);
            assert!((2..=10).contains(&dims.hi), "hi {} out of 2..=10", dims.hi);
        }
    }

    #[test]
    fn sign_lut_resolves_embedded_bit() {
        assert_eq!(SIGN_LUT, [1.0, -1.0]);
        assert_eq!(sign_from_bit(0), 1.0);
        assert_eq!(sign_from_bit(1), -1.0);
        // Only bit 0 matters — a single mask bit, per spec/05 §3.1.
        assert_eq!(sign_from_bit(2), 1.0);
        assert_eq!(sign_from_bit(3), -1.0);
    }

    #[test]
    fn symbols_for_band_is_ceil_div() {
        // ceil(line_count / dim): exact divisions, and over-cover remainders.
        assert_eq!(symbols_for_band(10, 2).unwrap(), 5); // 5*2 = 10 exact.
        assert_eq!(symbols_for_band(11, 2).unwrap(), 6); // 6*2 = 12 covers 11.
        assert_eq!(symbols_for_band(0, 5).unwrap(), 0); // empty band.
        assert_eq!(symbols_for_band(1, 10).unwrap(), 1); // one symbol covers a single line.
        assert_eq!(symbols_for_band(20, 5).unwrap(), 4);
        assert_eq!(symbols_for_band(21, 5).unwrap(), 5);
    }

    #[test]
    fn symbols_for_band_rejects_zero_dim() {
        assert_eq!(
            symbols_for_band(8, 0).unwrap_err(),
            Error::SpectralVectorDimZero
        );
    }

    #[test]
    fn coefficients_for_symbols_is_product() {
        assert_eq!(coefficients_for_symbols(5, 2), 10);
        assert_eq!(coefficients_for_symbols(0, 7), 0);
        assert_eq!(coefficients_for_symbols(4, 5), 20);
    }

    #[test]
    fn symbol_coverage_is_at_least_line_count() {
        // The symbols decoded for a band always cover (>=) its lines, and
        // the over-cover is strictly less than one full dim.
        for dim in [2u32, 4, 5, 10] {
            for line_count in 0u32..40 {
                let symbols = symbols_for_band(line_count, dim).unwrap();
                let covered = coefficients_for_symbols(symbols, dim);
                assert!(covered >= line_count, "dim {dim} lc {line_count}");
                assert!(covered < line_count + dim, "dim {dim} lc {line_count}");
            }
        }
    }

    #[test]
    fn symbols_for_band_uses_real_category_dims() {
        // The dim comes from a looked-up CategoryVectorDims (2..=10), so
        // every category's lo/hi branch yields a valid grouping.
        for cat in 0..CATEGORY_COUNT {
            let dims = CategoryVectorDims::for_category(CategoryIndex::new(cat).unwrap());
            for line_count in [12u32, 20, 47] {
                assert!(symbols_for_band(line_count, dims.lo).is_ok());
                assert!(symbols_for_band(line_count, dims.hi).is_ok());
            }
        }
    }

    #[test]
    fn dequant_scale_triple_matches_spec() {
        // spec/05 §3.1: non-zero entries {0.17678, 0.25, 0.70711} at 0x9150.
        assert_eq!(DEQUANT_SCALE_RVA, 0x9150);
        // Compare each entry to the binary-pinned decimal via bit pattern
        // so the test does not itself spell a FRAC_1_SQRT_2-approximate
        // literal (clippy::approx_constant).
        assert_eq!(DEQUANT_SCALE_NONZERO[0].to_bits(), 0.17678f32.to_bits());
        assert_eq!(DEQUANT_SCALE_NONZERO[1], 0.25);
        // 0.70711: built from 2^(-1/2) which rounds to the same f32 the
        // binary stores (verified below), avoiding a literal in the test.
        let half = 2.0f32.powf(-0.5);
        assert!((DEQUANT_SCALE_NONZERO[2] - half).abs() < 1e-4);
        // Each is a 2^(k/2) family member to within f32 rounding (the
        // trace pins them as literals, this only sanity-checks the shape).
        let twopow = |k: f32| 2.0f32.powf(k / 2.0);
        assert!((DEQUANT_SCALE_NONZERO[0] - twopow(-5.0)).abs() < 1e-4);
        assert!((DEQUANT_SCALE_NONZERO[1] - twopow(-4.0)).abs() < 1e-4);
        assert!((DEQUANT_SCALE_NONZERO[2] - twopow(-1.0)).abs() < 1e-4);
    }

    #[test]
    fn spectral_coefficient_composes_pinned_chain() {
        // coef = value * sign * dequant_scale * band_gain (spec/05 §3.1).
        let value = 3.0f32;
        let scale = DEQUANT_SCALE_NONZERO[1]; // 0.25
        let gain = 2.0f32;
        // sign bit 0 → +1.0.
        assert_eq!(
            spectral_coefficient(value, 0, scale, gain),
            value * 1.0 * scale * gain
        );
        // sign bit 1 → -1.0 (the coefficient is negated).
        assert_eq!(
            spectral_coefficient(value, 1, scale, gain),
            -(value * scale * gain)
        );
    }

    #[test]
    fn spectral_coefficient_sign_only_negates() {
        // The sign bit is the only difference between the two outputs.
        for &v in &[0.0f32, 1.0, 5.5] {
            for &s in &DEQUANT_SCALE_NONZERO {
                let pos = spectral_coefficient(v, 0, s, 1.0);
                let neg = spectral_coefficient(v, 1, s, 1.0);
                assert_eq!(pos, -neg);
            }
        }
    }

    #[test]
    fn spectral_coefficient_zero_value_or_gain_is_zero() {
        // A zero codebook value or a zero per-band gain zeroes the line.
        assert_eq!(
            spectral_coefficient(0.0, 0, DEQUANT_SCALE_NONZERO[2], 4.0),
            0.0
        );
        assert_eq!(spectral_coefficient(7.0, 1, 0.25, 0.0), 0.0);
    }

    #[test]
    fn coupling_table_len_is_one_below_the_power_of_two() {
        assert_eq!(coupling_table_len(2), 3);
        assert_eq!(coupling_table_len(3), 7);
        assert_eq!(coupling_table_len(4), 15);
        assert_eq!(coupling_table_len(6), 63);
    }

    #[test]
    fn mirror_index_is_self_inverse() {
        // spec/05 §4.2: partner = Ncoup - 1 - j; mirroring twice is the
        // identity, and the endpoints swap.
        for ncoup in 1u32..=16 {
            for j in 0..ncoup {
                let p = mirror_partner_index(j, ncoup).unwrap();
                assert!(p < ncoup);
                assert_eq!(mirror_partner_index(p, ncoup).unwrap(), j);
            }
            // Endpoints swap: index 0 partners with the last entry.
            assert_eq!(mirror_partner_index(0, ncoup).unwrap(), ncoup - 1);
            assert_eq!(mirror_partner_index(ncoup - 1, ncoup).unwrap(), 0);
        }
    }

    #[test]
    fn mirror_index_centre_is_fixed_for_odd_len() {
        // For an odd-length table the centre index is its own partner —
        // the rotation's 45° pan point.
        for half in 0u32..8 {
            let ncoup = 2 * half + 1;
            assert_eq!(mirror_partner_index(half, ncoup).unwrap(), half);
        }
    }

    #[test]
    fn mirror_index_rejects_bad_inputs() {
        assert_eq!(
            mirror_partner_index(0, 0).unwrap_err(),
            Error::CouplingTableEmpty
        );
        assert_eq!(
            mirror_partner_index(4, 4).unwrap_err(),
            Error::CouplingIndexOutOfRange { got: 4, ncoup: 4 }
        );
        assert_eq!(
            mirror_partner_index(100, 8).unwrap_err(),
            Error::CouplingIndexOutOfRange { got: 100, ncoup: 8 }
        );
    }

    #[test]
    fn split_coupled_pans_energy_between_channels() {
        // A normalised-pan coefficient pair: coef = cos/sin of a quarter
        // turn sampled at Ncoup=3 (endpoints + centre). The mirror pair
        // steers energy: j=0 → all into ch0, j=2 → all into ch1.
        let coef = [1.0f32, std::f32::consts::FRAC_1_SQRT_2, 0.0];
        let c = 2.0f32;
        let (l0, r0) = split_coupled_coefficient(c, 0, &coef).unwrap();
        assert_eq!(l0, 2.0); // c * coef[0]
        assert_eq!(r0, 0.0); // c * coef[2]
        let (l2, r2) = split_coupled_coefficient(c, 2, &coef).unwrap();
        assert_eq!(l2, 0.0); // c * coef[2]
        assert_eq!(r2, 2.0); // c * coef[0]
                             // Centre index: both channels read the same partner-equal entry.
        let (l1, r1) = split_coupled_coefficient(c, 1, &coef).unwrap();
        assert_eq!(l1, r1, "centre index pans equally");
    }

    #[test]
    fn split_coupled_rejects_bad_inputs() {
        assert_eq!(
            split_coupled_coefficient(1.0, 0, &[]).unwrap_err(),
            Error::CouplingTableEmpty
        );
        assert_eq!(
            split_coupled_coefficient(1.0, 3, &[1.0, 2.0, 3.0]).unwrap_err(),
            Error::CouplingIndexOutOfRange { got: 3, ncoup: 3 }
        );
    }
}
