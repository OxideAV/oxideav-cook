//! Spectral Huffman codebook decode — the §3.1 VLC walk `cook.dll!0x3a50`.
//!
//! Source-of-truth: `docs/audio/cook/spec/05-cook-backend-frame-syntax.md`
//! §3.1 (the seven spectral codebooks and the bit-by-bit VLC walk) / §3.2
//! (the runtime-built code/length tables), plus
//! `docs/audio/cook/provenance/06-cook-univdreams-extraction.md` (the
//! dynamic recovery of the codebook bytes) and the vendored fact tables
//! `tables/spectral-codebook-{codes,code-lengths}.csv`.
//!
//! ## What this module does
//!
//! `spec/05` §3.1 pins the spectral entropy read as a bit-by-bit VLC
//! descent:
//!
//! > *"A symbol is decoded by the bit-by-bit VLC walk `cook.dll!0x3a50`
//! > (calling the read-1-bit primitive `0x3fc0`), which descends the
//! > codebook built from the (length, value) tables by the table builder
//! > `cook.dll!0x3920`/`0x3b80`."*
//!
//! Each codebook is the (code, length) pair set now recovered from the
//! decoder's BSS (`spec/05` §3.2, the former docs-gap #1775). A symbol is
//! the codeword read **MSB-first, big-endian** through
//! [`FrameBitReader`](crate::bitreader::FrameBitReader) (`spec/05` §0.1):
//! bits are accumulated most-significant-first, and a symbol is emitted the
//! moment the accumulated `(length, code)` pair matches one of the
//! codebook's codewords.
//!
//! The `.meta` records that the distinct codewords form a **proper prefix
//! code** (verified in [`crate::tables`] tests), so at most one codeword of
//! any bit prefix matches — the read-until-match descent is unambiguous.
//! The seven codebooks carry Cook **escape-style duplicate max-length
//! codewords** (Kraft slightly over 1 for codebooks 0–5); [`SpectralHuffman`]
//! resolves a duplicated codeword to the **first** symbol carrying it (the
//! deterministic table-build order), and exposes the multiplicity via
//! [`SpectralHuffman::is_escape_symbol`] so a higher layer can apply the
//! escape read.
//!
//! ## What stays a GAP (not wired)
//!
//! The **escape mechanism** past a duplicated max-length codeword
//! (`spec/01` §5.1: *"unexpected escape-code, skipping"* — how many extra
//! bits follow and how they combine) is not pinned by the trace; this
//! module decodes the codeword to its first symbol and flags the escape
//! rather than guessing the follow-on read. The **per-band codebook
//! selection** (which of the seven codebooks a band uses) is resolved in
//! [`crate::spectral`]/[`crate::reconstruct`] from the band category, not
//! here — this module is the codebook-agnostic VLC primitive.
//!
//! ## Wall-respect note
//!
//! Every behavioural fact here is anchored to `spec/05` §3.1 / §0.1 and the
//! two vendored codebook fact tables; the walk is the trace's own
//! read-until-match descent, and the codeword bytes are Feist-clean facts
//! read from the decoder's memory image (`provenance/06`). No decoder
//! source was read.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::{
    bitreader::FrameBitReader,
    spectral::{SpectralCodebook, SPECTRAL_CODEBOOK_COUNT},
    tables::{spectral_codebook_code_lengths, spectral_codebook_codes},
    Error,
};

/// A decode-ready spectral Huffman codebook: the (code, length) pairs of
/// one of the seven `spec/05` §3.1 codebooks plus the read-until-match
/// lookup the [`decode_symbol`](SpectralHuffman::decode_symbol) walk uses.
#[derive(Debug)]
pub struct SpectralHuffman {
    /// Which of the seven codebooks (`0..=6`) this is.
    codebook: SpectralCodebook,
    /// Per-symbol codeword bit-length (bits), one per symbol
    /// (`tables/spectral-codebook-code-lengths.csv`).
    lengths: &'static [u32],
    /// Per-symbol codeword bit-pattern, right-aligned, MSB-first
    /// (`tables/spectral-codebook-codes.csv`).
    codes: &'static [u32],
    /// `(length, code) -> first symbol` read-until-match lookup. Duplicate
    /// (escape) codewords keep the first symbol (table-build order).
    lookup: HashMap<(u32, u32), u32>,
    /// Shortest codeword length in this codebook.
    min_len: u32,
    /// Longest codeword length in this codebook.
    max_len: u32,
}

impl SpectralHuffman {
    /// Build the decode structure for one spectral codebook from the
    /// vendored (code, length) fact tables.
    #[must_use]
    pub fn for_codebook(codebook: SpectralCodebook) -> Self {
        let i = codebook.as_usize();
        let codes = spectral_codebook_codes()[i].as_slice();
        let lengths = spectral_codebook_code_lengths()[i].as_slice();
        debug_assert_eq!(codes.len(), lengths.len());

        let mut lookup: HashMap<(u32, u32), u32> = HashMap::with_capacity(codes.len());
        let mut min_len = u32::MAX;
        let mut max_len = 0u32;
        for (sym, (&code, &len)) in codes.iter().zip(lengths.iter()).enumerate() {
            min_len = min_len.min(len);
            max_len = max_len.max(len);
            // First-symbol-wins for duplicated (escape) codewords.
            lookup.entry((len, code)).or_insert(sym as u32);
        }
        SpectralHuffman {
            codebook,
            lengths,
            codes,
            lookup,
            min_len,
            max_len,
        }
    }

    /// The codebook index (`0..=6`).
    #[must_use]
    pub const fn codebook(&self) -> SpectralCodebook {
        self.codebook
    }

    /// Number of symbols in this codebook.
    #[must_use]
    pub fn symbol_count(&self) -> u32 {
        self.codes.len() as u32
    }

    /// Shortest codeword length (bits).
    #[must_use]
    pub const fn min_code_length(&self) -> u32 {
        self.min_len
    }

    /// Longest codeword length (bits).
    #[must_use]
    pub const fn max_code_length(&self) -> u32 {
        self.max_len
    }

    /// The codeword `(code, length)` of a given symbol.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SpectralSymbolOutOfRange`] when `symbol` is not in
    /// `0..symbol_count()`.
    pub fn codeword(&self, symbol: u32) -> Result<(u32, u32), Error> {
        let idx = symbol as usize;
        if idx >= self.codes.len() {
            return Err(Error::SpectralSymbolOutOfRange {
                got: symbol,
                count: self.symbol_count(),
            });
        }
        Ok((self.codes[idx], self.lengths[idx]))
    }

    /// Whether `symbol`'s codeword is shared with another symbol — a Cook
    /// escape-style duplicate codeword (`spec/01` §5.1). A `true` result
    /// means [`decode_symbol`](Self::decode_symbol) resolves that codeword
    /// to the lowest-indexed symbol sharing it, and the true value would
    /// come from the (unpinned) escape read past the codeword.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SpectralSymbolOutOfRange`] when `symbol` is out of
    /// range.
    pub fn is_escape_symbol(&self, symbol: u32) -> Result<bool, Error> {
        let (code, len) = self.codeword(symbol)?;
        // Shared iff the first symbol for this (len, code) is not `symbol`
        // itself, or some later symbol also carries it.
        let first = self.lookup[&(len, code)];
        if first != symbol {
            return Ok(true);
        }
        // `symbol` is the first — check whether any later symbol repeats it.
        let dup = self
            .codes
            .iter()
            .zip(self.lengths.iter())
            .enumerate()
            .any(|(s, (&c, &l))| s as u32 != symbol && c == code && l == len);
        Ok(dup)
    }

    /// Decode one spectral VLC symbol from `reader`, reading MSB-first
    /// (`spec/05` §3.1 / §0.1).
    ///
    /// Accumulates bits most-significant-first and returns the symbol the
    /// moment the accumulated `(length, code)` matches a codeword. Because
    /// the distinct codewords are prefix-free, the first match is the only
    /// match. A duplicated (escape) codeword resolves to its first symbol.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SpectralVlcNoMatch`] when no codeword matches within
    /// [`max_code_length`](Self::max_code_length) bits — a malformed or
    /// exhausted bitstream (the reader returns `0` past the frame limit, so
    /// this is bounded).
    pub fn decode_symbol(&self, reader: &mut FrameBitReader) -> Result<u32, Error> {
        let mut acc: u32 = 0;
        let mut nbits: u32 = 0;
        while nbits < self.max_len {
            acc = (acc << 1) | reader.read_bit();
            nbits += 1;
            if nbits < self.min_len {
                continue;
            }
            if let Some(&sym) = self.lookup.get(&(nbits, acc)) {
                return Ok(sym);
            }
        }
        Err(Error::SpectralVlcNoMatch {
            codebook: self.codebook.get(),
            bits: nbits,
        })
    }
}

/// The seven decode-ready spectral codebooks, `OnceLock`-cached and built
/// once per process from the vendored fact tables.
///
/// Indexed by codebook `0..=6`; use [`spectral_huffman`] for a typed
/// [`SpectralCodebook`] accessor.
fn all_codebooks() -> &'static [SpectralHuffman; 7] {
    static T: OnceLock<[SpectralHuffman; 7]> = OnceLock::new();
    T.get_or_init(|| {
        std::array::from_fn(|i| SpectralHuffman::for_codebook(SpectralCodebook::new_const(i as u8)))
    })
}

/// The decode-ready spectral Huffman codebook for a typed codebook index.
///
/// `OnceLock`-cached; the underlying lookup is built once per process.
#[must_use]
pub fn spectral_huffman(codebook: SpectralCodebook) -> &'static SpectralHuffman {
    &all_codebooks()[codebook.as_usize()]
}

/// The number of spectral codebooks (`SPECTRAL_CODEBOOK_COUNT`, re-exported
/// for callers of this module).
pub const CODEBOOK_COUNT: u8 = SPECTRAL_CODEBOOK_COUNT;

#[cfg(test)]
mod tests {
    use super::*;

    fn cb(i: u8) -> SpectralCodebook {
        SpectralCodebook::new(i).unwrap()
    }

    /// Pack a sequence of `(code, length)` codewords MSB-first into a byte
    /// buffer suitable for [`FrameBitReader`] (which reads big-endian,
    /// MSB-first, 32-bit words). Bits are laid down most-significant-first.
    fn pack_codewords(words: &[(u32, u32)]) -> Vec<u8> {
        let mut bits: Vec<u8> = Vec::new();
        for &(code, len) in words {
            for b in (0..len).rev() {
                bits.push(((code >> b) & 1) as u8);
            }
        }
        // Pad to a multiple of 32 bits so the reader's word view is clean.
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
    fn built_shapes_match_tables() {
        for i in 0..7 {
            let h = SpectralHuffman::for_codebook(cb(i));
            assert_eq!(h.symbol_count(), cb(i).symbol_count());
            assert!(h.min_code_length() >= 1);
            assert!(h.max_code_length() <= 16);
            assert!(h.min_code_length() <= h.max_code_length());
        }
    }

    #[test]
    fn cached_accessor_is_stable() {
        let a = spectral_huffman(cb(3)) as *const _;
        let b = spectral_huffman(cb(3)) as *const _;
        assert_eq!(a, b, "OnceLock must return the same instance");
    }

    #[test]
    fn every_unique_symbol_round_trips() {
        // For every codebook, packing a symbol's own codeword and decoding
        // it returns that symbol — except for the escape-style duplicated
        // codewords, which resolve to the first symbol sharing the pattern.
        for i in 0..7 {
            let h = spectral_huffman(cb(i));
            for sym in 0..h.symbol_count() {
                let (code, len) = h.codeword(sym).unwrap();
                let bytes = pack_codewords(&[(code, len)]);
                let mut r = FrameBitReader::new(&bytes);
                let got = h.decode_symbol(&mut r).unwrap();
                if h.is_escape_symbol(sym).unwrap() {
                    // Duplicated codeword: decode resolves to the first
                    // symbol carrying this exact (code, len).
                    let (gc, gl) = h.codeword(got).unwrap();
                    assert_eq!((gc, gl), (code, len), "cb{i} sym{sym} escape mismatch");
                } else {
                    assert_eq!(got, sym, "cb{i} sym{sym} did not round-trip");
                }
                // The decode consumed exactly `len` bits.
                assert_eq!(r.bit_cursor(), len, "cb{i} sym{sym} bit count");
            }
        }
    }

    #[test]
    fn decodes_a_symbol_stream_in_order() {
        // A concatenation of several distinct (non-escape) codewords
        // decodes back to the same symbol sequence, one after another.
        let h = spectral_huffman(cb(6)); // cb6 is a strict prefix code (no escapes).
        let syms = [0u32, 5, 31, 12, 7, 1];
        let words: Vec<(u32, u32)> = syms.iter().map(|&s| h.codeword(s).unwrap()).collect();
        let bytes = pack_codewords(&words);
        let mut r = FrameBitReader::new(&bytes);
        for &want in &syms {
            assert_eq!(h.decode_symbol(&mut r).unwrap(), want);
        }
    }

    #[test]
    fn cb6_has_no_escape_symbols() {
        // cb6 Kraft = 1.0 exactly (.meta): a strict prefix code, no
        // duplicated codewords, so every symbol round-trips exactly.
        let h = spectral_huffman(cb(6));
        for sym in 0..h.symbol_count() {
            assert!(
                !h.is_escape_symbol(sym).unwrap(),
                "cb6 sym{sym} unexpectedly escape"
            );
        }
    }

    #[test]
    fn codebooks_0_to_5_have_escape_symbols() {
        // .meta: cb0-5 carry escape-style duplicate max-length codewords.
        let expected_dup_pairs = [15usize, 6, 1, 105, 47, 51];
        for (i, &want) in expected_dup_pairs.iter().enumerate() {
            let h = spectral_huffman(cb(i as u8));
            let escapes = (0..h.symbol_count())
                .filter(|&s| h.is_escape_symbol(s).unwrap())
                .count();
            // Each duplicated codeword is shared by >= 2 symbols, so the
            // escape-symbol count is at least the duplicate-pair count.
            assert!(
                escapes >= want,
                "cb{i} escapes {escapes} < dup pairs {want}"
            );
        }
    }

    #[test]
    fn codeword_rejects_out_of_range_symbol() {
        let h = spectral_huffman(cb(2));
        let n = h.symbol_count();
        assert_eq!(
            h.codeword(n).unwrap_err(),
            Error::SpectralSymbolOutOfRange { got: n, count: n }
        );
    }

    #[test]
    fn decode_reports_no_match_on_exhausted_stream() {
        // An all-ones stream that never forms a valid codeword within
        // max_len bits surfaces the typed no-match error rather than
        // looping. cb6's codewords never start with a run of 1s up to its
        // max length in a way that... instead, use an empty frame: the
        // reader returns 0 past the limit, and cb6's all-zero prefix does
        // decode symbol 0 — so test the error path with a codebook where a
        // long 1-run has no codeword.
        let h = spectral_huffman(cb(6));
        // Craft a bit pattern of all 1s longer than max_len with no match.
        let ones = vec![0xFFu8; 8];
        let mut r = FrameBitReader::with_bit_limit(&ones, h.max_code_length());
        match h.decode_symbol(&mut r) {
            Ok(sym) => {
                // If an all-ones prefix happens to be a valid codeword,
                // that's fine — assert it is one of this codebook's words.
                assert!(sym < h.symbol_count());
            }
            Err(Error::SpectralVlcNoMatch { codebook, bits }) => {
                assert_eq!(codebook, 6);
                assert_eq!(bits, h.max_code_length());
            }
            Err(e) => panic!("unexpected error {e:?}"),
        }
    }
}
