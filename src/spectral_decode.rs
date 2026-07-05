//! Spectral entropy → per-coefficient quantised digits (§2.2 / §3.1).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §2.2 (the
//! reciprocal-multiply index decomposition) and §3.1 (the vector-VLC
//! grouping — *"each symbol expands to `dim` coefficients"*), plus
//! `docs/audio/cook/provenance/05-cook-backend.md` evidence #7.
//!
//! ## What this module does
//!
//! This is the bridge between the two already-wired pinned primitives:
//!
//! - the §3.1 spectral VLC walk ([`crate::codebook::SpectralHuffman`]),
//!   which decodes one **packed vector symbol** per read; and
//! - the §2.2 division-free index decomposition
//!   ([`crate::index_decomp::decompose_index`]), which splits a packed
//!   index into base-`radix` digits via the `0x8fac` reciprocal-multiply.
//!
//! `spec/05` §3.1 pins that a decoded symbol *"expands to `dim`
//! coefficients"*, and `spec/05` §2.2 pins that the decomposition splits
//! the index into *"(codebook-symbol, in-symbol-position)"* one base-`n`
//! digit at a time. Composing them, a symbol is the mixed-radix number
//! `Σ digit[i] · radix^i` for `dim` digits, each a per-coefficient
//! quantised **level** in `0..radix`. [`decompose_symbol`] recovers those
//! `dim` digits (least-significant first) and [`decode_band_digits`] runs
//! the VLC walk over a whole band, concatenating each symbol's `dim`
//! digits into the band's per-coefficient level array.
//!
//! The per-category **radix** is the pinned `0x8fac` table
//! ([`crate::index_decomp::INDEX_RADIX`] `{14, 10, 7, 5, 4, 3, 2}` =
//! `level_count + 1`); the per-category **vector dimension** `dim` is the
//! pinned `0x9170` / `0x918c` table
//! ([`crate::spectral::CategoryVectorDims`]). For a category the natural
//! codebook is the one whose symbol count equals `radix^dim` (the
//! §3.1 *"codebook with the matching symbol count"*); that numeric
//! consistency is checked by [`natural_codebook_for`] but the actual
//! per-band codebook / lo-or-hi-branch **selection** the decoder makes
//! stays a recorded `spec/05` §3.1 GAP, so the codebook and `dim` are
//! caller inputs here.
//!
//! ## The provenance/07 pins (rounds 7/8 staging)
//!
//! `docs/audio/cook/provenance/07-cook-spectral-decode.md` resolved the
//! three gates the earlier rounds recorded as GAPs:
//!
//! - **Per-band codebook selection (item 1):** the per-band category
//!   *is* the codebook — *"category 0..6 → codebook 0..6; there is no
//!   separate codebook-id field"* — wired as
//!   [`codebook_for_category`]; **category 7** is the guarded
//!   *"empty band"* sentinel (`cook.dll!0x46c0` / `0x69f0` / `0x6a80`
//!   early-out), wired as [`BandCategory`] /
//!   [`EMPTY_BAND_CATEGORY`]. `dim_lo` is the codebook's **vector
//!   dimension** (`(level_count+1)^dim_lo` equals every codebook's
//!   symbol count — verified in tests), and the per-band line count is
//!   the caller's §2.1 geometry input.
//! - **Level → signed value (item 2):** each digit is an **unsigned
//!   magnitude level** (no centering); one out-of-band sign bit per
//!   **non-zero** magnitude, read immediately after the vector
//!   codeword (`cook.dll!0x3fc0`, bit `0` → `+1`, bit `1` → `−1`);
//!   the magnitude reconstructs through the `0x8fc8` expectation row
//!   ([`crate::expectation::dequantise_level`]).
//! - **Escape read (item 3):** the *"duplicate max-length codewords"*
//!   are the sign multiplicity of the magnitude-only code — *"there is
//!   no literal-magnitude escape"*; the only post-codeword bits are the
//!   `popcount(non-zero digits) ∈ [0, dim_lo]` sign bits. The walk's
//!   first-match resolution plus the sign reads is therefore the
//!   complete decode; wired as [`decode_vector`] / [`decode_band`] /
//!   [`decode_band_coefficients`].
//!
//! ## What stays open
//!
//! The **intra-vector digit → coefficient order** (the decomposition
//! peels digits by repeated `mod radix`, i.e. least-significant first;
//! whether the cursor stores them in that order or reversed is not
//! pinned — this module uses decomposition order for both digits and
//! their sign bits, as a documented convention). The `0x9150`
//! scale-selector semantics ride in [`crate::expectation`]'s recorded
//! gap.
//!
//! ## Wall-respect note
//!
//! Every step composes independently-pinned primitives (the VLC walk,
//! the reciprocal-multiply decomposition, the sign LUT, the expectation
//! rows); no new algorithm is introduced.

use crate::{
    category::CategoryIndex,
    codebook::SpectralHuffman,
    expectation::dequantise_level,
    index_decomp::{decompose_index, index_radix},
    spectral::{symbols_for_band, CategoryVectorDims, SpectralCodebook},
    Error,
};

/// The guarded "empty band" category sentinel — `cook.dll!0x46c0` /
/// `0x69f0` / `0x6a80` early-out on `category == 7` and emit no
/// coefficients for the band (`provenance/07` item 1).
pub const EMPTY_BAND_CATEGORY: u8 = 7;

/// A band's decode disposition: a coded category (0..=6, which *is*
/// the spectral codebook index) or the empty-band sentinel (7).
///
/// `provenance/07` item 1: *"the category index directly selects one
/// of the seven spectral VLC codebooks; there is no separate
/// codebook-id field"*, and category 7 is the guarded empty band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandCategory {
    /// Category 0..=6 — decoded through spectral codebook `cat`.
    Coded(CategoryIndex),
    /// Category 7 — the band carries no coefficients.
    Empty,
}

impl BandCategory {
    /// Classify a raw per-band category value (`0..=7`).
    ///
    /// # Errors
    ///
    /// [`Error::CategoryOutOfRange`] for raw values above
    /// [`EMPTY_BAND_CATEGORY`] — the bit-allocation pass never assigns
    /// one.
    pub fn from_raw(raw: u8) -> Result<Self, Error> {
        if raw == EMPTY_BAND_CATEGORY {
            Ok(BandCategory::Empty)
        } else {
            Ok(BandCategory::Coded(CategoryIndex::new(raw)?))
        }
    }

    /// The raw category value (`0..=7`).
    pub const fn raw(self) -> u8 {
        match self {
            BandCategory::Coded(c) => c.get(),
            BandCategory::Empty => EMPTY_BAND_CATEGORY,
        }
    }
}

/// The spectral codebook a coded category selects — the identity map
/// of `provenance/07` item 1 (*"the codebook is the category"*).
pub fn codebook_for_category(category: CategoryIndex) -> SpectralCodebook {
    // A CategoryIndex is 0..=6 by construction, exactly the codebook
    // range, so the checked constructor cannot fail.
    SpectralCodebook::new(category.get()).expect("category 0..=6 is a valid codebook index")
}

/// One decoded per-coefficient `(magnitude level, sign bit)` pair.
///
/// `level` is the unsigned magnitude digit in
/// `0..=level_count[category]`; `sign_bit` is the out-of-band bit read
/// after the vector codeword (`0` → `+1`, `1` → `−1` through the
/// `0xa148` LUT) — always `0` for a zero level, which carries no sign
/// bit on the wire (`provenance/07` items 2/3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedLevel {
    /// Unsigned magnitude level.
    pub level: u32,
    /// Out-of-band sign bit (`0` = positive; meaningless for level 0).
    pub sign_bit: u32,
}

/// Decompose one decoded VLC symbol into its `dim` per-coefficient
/// quantised digits (least-significant first), base `radix[category]`.
///
/// `spec/05` §2.2 / §3.1: the symbol is the mixed-radix number
/// `Σ digit[i] · radix^i`; this applies
/// [`decompose_index`](crate::index_decomp::decompose_index) `dim` times,
/// peeling one base-`radix` digit per coefficient. Each returned digit is a
/// quantised level in `0..radix`.
///
/// # Errors
///
/// - [`Error::IndexRecipOutOfRange`] if `category` is out of the `0..=6`
///   reciprocal-table range (cannot happen for a real [`CategoryIndex`]).
/// - [`Error::SpectralVectorDimZero`] if `dim == 0`.
pub fn decompose_symbol(symbol: u32, category: CategoryIndex, dim: u32) -> Result<Vec<u32>, Error> {
    if dim == 0 {
        return Err(Error::SpectralVectorDimZero);
    }
    let mut idx = symbol;
    let mut digits = Vec::with_capacity(dim as usize);
    for _ in 0..dim {
        let (quotient, remainder) = decompose_index(idx, category.get())?;
        digits.push(remainder);
        idx = quotient;
    }
    Ok(digits)
}

/// Compose `dim` per-coefficient digits (least-significant first) back into
/// a packed symbol — the inverse of [`decompose_symbol`].
///
/// `symbol = Σ digit[i] · radix^i`. Exposed so callers (and tests) can
/// round-trip the §2.2 decomposition against the §3.1 VLC codeword.
///
/// # Errors
///
/// [`Error::IndexRecipOutOfRange`] if `category` is out of range.
pub fn compose_symbol(digits: &[u32], category: CategoryIndex) -> Result<u32, Error> {
    let radix = index_radix(category.get())?;
    let mut symbol = 0u32;
    for &d in digits.iter().rev() {
        symbol = symbol * radix + d;
    }
    Ok(symbol)
}

/// The codebook index whose symbol count equals `radix^dim` for a category
/// and vector dimension — the §3.1 *"codebook with the matching symbol
/// count"*.
///
/// Returns `radix[category]^dim` (the required codebook symbol count); the
/// caller matches it against the seven
/// [`crate::spectral::SpectralCodebook::symbol_count`] values. This is a
/// numeric-consistency helper, **not** the pinned per-band selection (which
/// stays a `spec/05` §3.1 GAP): it lets a caller confirm a chosen
/// codebook/`dim` pair is self-consistent with the §2.2 radix.
///
/// # Errors
///
/// [`Error::IndexRecipOutOfRange`] if `category` is out of range.
pub fn natural_codebook_for(category: CategoryIndex, dim: u32) -> Result<u32, Error> {
    let radix = index_radix(category.get())?;
    Ok(radix.saturating_pow(dim))
}

/// Decode one coded band's spectral coefficients as quantised digits by
/// running the §3.1 VLC walk over `huffman` and decomposing each symbol.
///
/// `line_count` is the band's spectral-line count (§2.1); the walk reads
/// [`symbols_for_band(line_count, dim)`](crate::spectral::symbols_for_band)
/// symbols, expands each to `dim` digits, and concatenates — yielding
/// `symbols · dim` per-coefficient quantised levels (`>= line_count`; the
/// last symbol may over-cover). The returned levels are the caller
/// `values` input for [`crate::reconstruct::reconstruct_band`].
///
/// `huffman` is the caller-selected codebook (selection is a §3.1 GAP);
/// `category` supplies the §2.2 radix and `dim` the §3.1 grouping.
///
/// # Errors
///
/// - [`Error::SpectralVectorDimZero`] if `dim == 0`.
/// - [`Error::SpectralVlcNoMatch`] if the bitstream is exhausted / malformed.
/// - [`Error::IndexRecipOutOfRange`] if `category` is out of range.
pub fn decode_band_digits(
    huffman: &SpectralHuffman,
    reader: &mut crate::bitreader::FrameBitReader,
    category: CategoryIndex,
    dim: u32,
    line_count: u32,
) -> Result<Vec<u32>, Error> {
    let symbols = symbols_for_band(line_count, dim)?;
    let mut out = Vec::with_capacity((symbols * dim) as usize);
    for _ in 0..symbols {
        let symbol = huffman.decode_symbol(reader)?;
        out.extend(decompose_symbol(symbol, category, dim)?);
    }
    Ok(out)
}

/// Decode one spectral **vector** — codeword, magnitude digits, and the
/// out-of-band sign bits — per `provenance/07` items 1–3.
///
/// Reads one VLC symbol from `huffman` (the category's codebook — see
/// [`codebook_for_category`]), peels it into the category's `dim_lo`
/// magnitude digits, then reads **one sign bit per non-zero digit**, in
/// digit order, immediately after the codeword (`cook.dll!0x3fc0`;
/// zero digits consume no bit). Returns the `dim_lo`
/// [`SignedLevel`] pairs.
///
/// # Errors
///
/// - [`Error::SpectralVlcNoMatch`] on a malformed/exhausted bitstream.
/// - [`Error::IndexRecipOutOfRange`] if `category` is out of range
///   (cannot happen for a real [`CategoryIndex`]).
pub fn decode_vector(
    huffman: &SpectralHuffman,
    reader: &mut crate::bitreader::FrameBitReader,
    category: CategoryIndex,
) -> Result<Vec<SignedLevel>, Error> {
    let dim = CategoryVectorDims::for_category(category).lo;
    let symbol = huffman.decode_symbol(reader)?;
    let digits = decompose_symbol(symbol, category, dim)?;
    let mut out = Vec::with_capacity(digits.len());
    for level in digits {
        // provenance/07 item 2 step 2: one sign bit per NON-ZERO
        // magnitude; "zero-magnitude coefficients consume no sign bit
        // (the walk skips the read when the digit is 0)".
        let sign_bit = if level != 0 { reader.read_bit() } else { 0 };
        out.push(SignedLevel { level, sign_bit });
    }
    Ok(out)
}

/// Decode one coded band's `(level, sign)` pairs by running
/// [`decode_vector`] over `ceil(line_count / dim_lo)` vectors.
///
/// Returns `symbols × dim_lo` pairs (`>= line_count`; the last vector
/// may over-cover the band — truncate to the band's line count when
/// filling the spectrum, as [`decode_band_coefficients`] does).
///
/// # Errors
///
/// See [`decode_vector`]; also [`Error::SpectralVectorDimZero`] for a
/// zero line count grouping.
pub fn decode_band(
    huffman: &SpectralHuffman,
    reader: &mut crate::bitreader::FrameBitReader,
    category: CategoryIndex,
    line_count: u32,
) -> Result<Vec<SignedLevel>, Error> {
    let dim = CategoryVectorDims::for_category(category).lo;
    let symbols = symbols_for_band(line_count, dim)?;
    let mut out = Vec::with_capacity((symbols * dim) as usize);
    for _ in 0..symbols {
        out.extend(decode_vector(huffman, reader, category)?);
    }
    Ok(out)
}

/// Decode one band straight to reconstructed spectral coefficients —
/// the complete pinned §3 chain: codebook-by-category VLC walk,
/// magnitude digits, out-of-band signs, and the
/// `sign × expectation[cat][level] × band_gain` reconstruction
/// ([`crate::expectation::dequantise_level`]).
///
/// - [`BandCategory::Empty`] (category 7) emits `line_count` zeros and
///   **reads nothing** — the `cook.dll!0x46c0` early-out.
/// - A coded category selects its codebook by identity, decodes
///   `ceil(line_count / dim_lo)` vectors, reconstructs each
///   coefficient, and truncates the final vector's over-coverage to
///   exactly `line_count` values.
///
/// # Errors
///
/// See [`decode_band`] / [`crate::expectation::dequantise_level`].
pub fn decode_band_coefficients(
    reader: &mut crate::bitreader::FrameBitReader,
    band: BandCategory,
    line_count: u32,
    band_gain: f32,
) -> Result<Vec<f32>, Error> {
    let category = match band {
        BandCategory::Empty => return Ok(vec![0.0; line_count as usize]),
        BandCategory::Coded(c) => c,
    };
    let huffman = crate::codebook::spectral_huffman(codebook_for_category(category));
    let pairs = decode_band(huffman, reader, category, line_count)?;
    let mut out = Vec::with_capacity(line_count as usize);
    for p in pairs.into_iter().take(line_count as usize) {
        out.push(dequantise_level(category, p.level, p.sign_bit, band_gain)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitreader::FrameBitReader;
    use crate::codebook::spectral_huffman;
    use crate::index_decomp::INDEX_RADIX;
    use crate::spectral::{CategoryVectorDims, SpectralCodebook};
    use crate::tables::SPECTRAL_CODEBOOK_SYMBOL_COUNTS;

    fn cat(c: u8) -> CategoryIndex {
        CategoryIndex::new(c).unwrap()
    }

    fn pack_codewords(words: &[(u32, u32)]) -> Vec<u8> {
        let mut bits: Vec<u8> = Vec::new();
        for &(code, len) in words {
            for b in (0..len).rev() {
                bits.push(((code >> b) & 1) as u8);
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

    #[test]
    fn decompose_then_compose_is_identity() {
        // For every category, a symbol in 0..radix^dim decomposes to `dim`
        // base-radix digits and recomposes exactly.
        for c in 0..7u8 {
            let category = cat(c);
            let radix = INDEX_RADIX[c as usize];
            let dim = CategoryVectorDims::for_category(category).lo;
            // Sample a spread of symbols across the codebook range.
            let count = radix.pow(dim);
            for &symbol in &[0u32, 1, radix, radix + 3, count / 2, count - 1] {
                if symbol >= count {
                    continue;
                }
                let digits = decompose_symbol(symbol, category, dim).unwrap();
                assert_eq!(digits.len(), dim as usize);
                for &d in &digits {
                    assert!(d < radix, "digit {d} >= radix {radix} (cat {c})");
                }
                assert_eq!(compose_symbol(&digits, category).unwrap(), symbol);
            }
        }
    }

    #[test]
    fn natural_codebook_matches_the_vendored_symbol_counts() {
        // radix[c]^dim_lo[c] == symbol_count of codebook c: the §3.1
        // "matching symbol count" is self-consistent with the §2.2 radix
        // for the low-branch dimension.
        for c in 0..7u8 {
            let category = cat(c);
            let dim = CategoryVectorDims::for_category(category).lo;
            let want = SPECTRAL_CODEBOOK_SYMBOL_COUNTS[c as usize] as u32;
            assert_eq!(
                natural_codebook_for(category, dim).unwrap(),
                want,
                "cat {c}: radix^dim_lo should equal codebook {c} symbol count"
            );
        }
    }

    #[test]
    fn decode_band_round_trips_through_vlc_and_decomposition() {
        // Build a band of known quantised digits, compose them into symbols,
        // encode the symbols' codewords, and decode+decompose back — the
        // full VLC → digits path reproduces the digits (using cb6, the
        // escape-free strict prefix code, so every symbol round-trips).
        let c = 6u8;
        let category = cat(c);
        let dim = CategoryVectorDims::for_category(category).lo; // 5
        let radix = INDEX_RADIX[c as usize]; // 2
        let huffman = spectral_huffman(SpectralCodebook::new(c).unwrap());

        // Three symbols worth of digits (dim each), all < radix.
        let want_digits: Vec<u32> = vec![
            1, 0, 1, 1, 0, // symbol A
            0, 1, 1, 0, 1, // symbol B
            1, 1, 0, 0, 0, // symbol C
        ];
        let line_count = want_digits.len() as u32; // exactly 3*dim
                                                   // Compose each dim-chunk into a symbol, then pack its codeword.
        let mut words = Vec::new();
        for chunk in want_digits.chunks(dim as usize) {
            let symbol = compose_symbol(chunk, category).unwrap();
            assert!(symbol < radix.pow(dim));
            words.push(huffman.codeword(symbol).unwrap());
        }
        let bytes = pack_codewords(&words);
        let mut reader = FrameBitReader::new(&bytes);

        let got = decode_band_digits(huffman, &mut reader, category, dim, line_count).unwrap();
        assert_eq!(got, want_digits);
    }

    #[test]
    fn decode_band_reads_ceil_div_symbols() {
        // A band whose line_count is not a multiple of dim over-covers by
        // less than one dim (the last symbol's extra digits).
        let c = 6u8;
        let category = cat(c);
        let dim = CategoryVectorDims::for_category(category).lo; // 5
        let huffman = spectral_huffman(SpectralCodebook::new(c).unwrap());
        // line_count = 7 -> ceil(7/5) = 2 symbols -> 10 digits.
        let words = vec![huffman.codeword(0).unwrap(), huffman.codeword(0).unwrap()];
        let bytes = pack_codewords(&words);
        let mut reader = FrameBitReader::new(&bytes);
        let got = decode_band_digits(huffman, &mut reader, category, dim, 7).unwrap();
        assert_eq!(got.len(), 10);
    }

    #[test]
    fn decompose_rejects_zero_dim() {
        assert_eq!(
            decompose_symbol(5, cat(0), 0).unwrap_err(),
            Error::SpectralVectorDimZero
        );
    }

    // ----- provenance/07 pinned selection + signed decode -----------

    /// Pack a sequence of `(value, nbits)` fields MSB-first.
    fn pack_fields(fields: &[(u32, u32)]) -> Vec<u8> {
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

    #[test]
    fn codebook_is_the_category() {
        // provenance/07 item 1: category 0..6 → codebook 0..6, identity.
        for c in 0..7u8 {
            assert_eq!(codebook_for_category(cat(c)).get(), c);
        }
    }

    #[test]
    fn band_category_classifies_the_empty_sentinel() {
        for c in 0..7u8 {
            assert_eq!(
                BandCategory::from_raw(c).unwrap(),
                BandCategory::Coded(cat(c))
            );
            assert_eq!(BandCategory::from_raw(c).unwrap().raw(), c);
        }
        assert_eq!(BandCategory::from_raw(7).unwrap(), BandCategory::Empty);
        assert_eq!(BandCategory::from_raw(7).unwrap().raw(), 7);
        assert!(matches!(
            BandCategory::from_raw(8),
            Err(Error::CategoryOutOfRange { got: 8 })
        ));
    }

    #[test]
    fn vector_dims_product_is_twenty_for_every_category() {
        // Observed identity of the two staged tables: dim_lo[c] ×
        // dim_hi[c] == 20 for all seven categories ({2,2,2,4,4,5,5} ×
        // {10,10,10,5,5,4,4}). provenance/07 item 1 reads dim_hi as the
        // per-band line count decoded as ceil(dim_hi/dim_lo) vectors;
        // under this product identity dim_hi also equals the
        // vectors-per-20-line-band count — the distinction is a
        // recorded docs nuance, and this test pins the arithmetic fact
        // either reading must satisfy.
        for c in 0..7u8 {
            let d = CategoryVectorDims::for_category(cat(c));
            assert_eq!(d.lo * d.hi, 20, "cat {c}: dim_lo × dim_hi must be 20");
        }
    }

    #[test]
    fn decode_vector_reads_one_sign_bit_per_nonzero_digit() {
        // cb6 (strict prefix code, dim 5, radix 2): digits [1,0,1,1,0]
        // carry three sign bits, read in digit order.
        let c = 6u8;
        let category = cat(c);
        let huffman = spectral_huffman(SpectralCodebook::new(c).unwrap());
        let digits = [1u32, 0, 1, 1, 0];
        let signs = [1u32, 0, 1]; // for the three non-zero digits
        let symbol = compose_symbol(&digits, category).unwrap();
        let (code, len) = huffman.codeword(symbol).unwrap();
        let mut fields = vec![(code, len)];
        for &s in &signs {
            fields.push((s, 1));
        }
        let bytes = pack_fields(&fields);
        let mut reader = FrameBitReader::new(&bytes);

        let got = decode_vector(huffman, &mut reader, category).unwrap();
        assert_eq!(got.len(), 5);
        let mut sign_iter = signs.iter();
        for (i, p) in got.iter().enumerate() {
            assert_eq!(p.level, digits[i], "digit {i}");
            if digits[i] != 0 {
                assert_eq!(p.sign_bit, *sign_iter.next().unwrap(), "sign of digit {i}");
            } else {
                assert_eq!(p.sign_bit, 0, "zero digit {i} must stay unsigned");
            }
        }
        // Exactly codeword + 3 sign bits consumed.
        assert_eq!(reader.bit_cursor(), len + 3);
    }

    #[test]
    fn decode_vector_all_zero_digits_consume_no_sign_bits() {
        let c = 6u8;
        let category = cat(c);
        let huffman = spectral_huffman(SpectralCodebook::new(c).unwrap());
        let (code, len) = huffman.codeword(0).unwrap(); // digits [0;5]
        let bytes = pack_fields(&[(code, len)]);
        let mut reader = FrameBitReader::new(&bytes);
        let got = decode_vector(huffman, &mut reader, category).unwrap();
        assert!(got.iter().all(|p| p.level == 0 && p.sign_bit == 0));
        assert_eq!(reader.bit_cursor(), len, "no sign bits for zero digits");
    }

    #[test]
    fn empty_band_emits_zeros_and_reads_nothing() {
        let bytes = [0xffu8; 8];
        let mut reader = FrameBitReader::new(&bytes);
        let got = decode_band_coefficients(&mut reader, BandCategory::Empty, 10, 2.0).unwrap();
        assert_eq!(got, vec![0.0; 10]);
        assert_eq!(reader.bit_cursor(), 0, "category 7 early-out reads nothing");
    }

    #[test]
    fn decode_band_coefficients_reconstructs_the_pinned_closed_form() {
        // Two cb6 vectors covering a 10-line band; verify each
        // coefficient equals sign × expectation[cat][level] × gain.
        let c = 6u8;
        let category = cat(c);
        let huffman = spectral_huffman(SpectralCodebook::new(c).unwrap());
        let digits_a = [1u32, 1, 0, 0, 1];
        let digits_b = [0u32, 1, 1, 0, 0];
        let signs_a = [0u32, 1, 0];
        let signs_b = [1u32, 0];
        let band_gain = 4.0f32;

        let mut fields = Vec::new();
        let sym_a = compose_symbol(&digits_a, category).unwrap();
        fields.push(huffman.codeword(sym_a).unwrap());
        fields.extend(signs_a.iter().map(|&s| (s, 1)));
        let sym_b = compose_symbol(&digits_b, category).unwrap();
        fields.push(huffman.codeword(sym_b).unwrap());
        fields.extend(signs_b.iter().map(|&s| (s, 1)));
        let bytes = pack_fields(&fields);
        let mut reader = FrameBitReader::new(&bytes);

        let got =
            decode_band_coefficients(&mut reader, BandCategory::Coded(category), 10, band_gain)
                .unwrap();
        assert_eq!(got.len(), 10);

        let all_digits: Vec<u32> = digits_a.iter().chain(digits_b.iter()).copied().collect();
        let mut signs = signs_a.iter().chain(signs_b.iter());
        for (i, &level) in all_digits.iter().enumerate() {
            let sign_bit = if level != 0 {
                *signs.next().unwrap()
            } else {
                0
            };
            let want =
                crate::expectation::dequantise_level(category, level, sign_bit, band_gain).unwrap();
            assert_eq!(got[i], want, "coefficient {i}");
        }
    }

    #[test]
    fn decode_band_truncates_the_last_vector_overcoverage() {
        // line_count = 7 with dim 5 → 2 vectors → 10 pairs, truncated
        // to 7 coefficients.
        let c = 6u8;
        let category = cat(c);
        let huffman = spectral_huffman(SpectralCodebook::new(c).unwrap());
        let (code, len) = huffman.codeword(0).unwrap();
        let bytes = pack_fields(&[(code, len), (code, len)]);
        let mut reader = FrameBitReader::new(&bytes);
        let got =
            decode_band_coefficients(&mut reader, BandCategory::Coded(category), 7, 1.0).unwrap();
        assert_eq!(got.len(), 7);
    }
}
