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
//! ## What stays a GAP (not wired)
//!
//! The **level → signed reconstructed value** mapping (any centering of
//! the `0..radix` level, and the sign-bit read protocol) is not pinned by
//! the trace; the digits this module produces are the raw quantised
//! levels, fed onward as the caller `values` input to
//! [`crate::reconstruct::reconstruct_band`] (which applies the pinned
//! §3.1 `value · sign · dequant_scale · band_gain` chain). The per-band
//! **codebook selection** and the **escape read** past a duplicated
//! codeword are the other recorded gaps.
//!
//! ## Wall-respect note
//!
//! Every step here composes two independently-pinned primitives (the VLC
//! walk and the reciprocal-multiply decomposition); no new algorithm is
//! introduced. The mixed-radix composition is the direct inverse of the
//! §2.2 decomposition the trace pins.

use crate::{
    category::CategoryIndex,
    codebook::SpectralHuffman,
    index_decomp::{decompose_index, index_radix},
    spectral::symbols_for_band,
    Error,
};

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
}
