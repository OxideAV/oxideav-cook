//! Variable-Length-Code (Huffman) decoder for the cook VLC tables.
//!
//! Cook stores its tables as `(symbol, length, code)` triples (we have
//! them in `vlc_tables.rs`, transcribed verbatim from the clean-room
//! sidecar). The decode side walks the bitstream MSB-first and looks
//! up the matching code by length.
//!
//! This implementation builds a flat hash from `(length, code) → symbol`
//! once at construction (or, equivalently, walks the table linearly).
//! Cook's tables are small enough that linear search per code-length is
//! acceptable; the decoder isn't the hot path of a perf-critical app.

use oxideav_core::bits::BitReader;
use oxideav_core::{Error, Result};

use crate::vlc_tables::VlcEntry;

/// A built Huffman decoder for one cook VLC table.
pub struct Vlc {
    /// Sorted by code length ascending. Each entry is `(length, code, symbol)`.
    entries: Vec<(u8, u32, u32)>,
    /// Maximum code length present (in bits).
    max_len: u8,
}

impl Vlc {
    pub fn new(table: &[VlcEntry]) -> Self {
        let mut entries: Vec<(u8, u32, u32)> = table
            .iter()
            .map(|&(sym, len, code)| (len, code, sym))
            .collect();
        entries.sort_by_key(|&(len, code, _)| (len, code));
        let max_len = entries.iter().map(|e| e.0).max().unwrap_or(0);
        Self { entries, max_len }
    }

    pub fn max_len(&self) -> u8 {
        self.max_len
    }

    /// Decode the next symbol from `br`. MSB-first canonical: try
    /// progressively longer code lengths and find the matching entry.
    pub fn decode(&self, br: &mut BitReader<'_>) -> Result<u32> {
        // Walk by length: read one extra bit each iteration, then look up.
        let mut acc: u32 = 0;
        let mut have: u8 = 0;
        // Filter entries by min length actually present.
        let min_len = self.entries.first().map(|e| e.0).unwrap_or(0);
        for _ in 0..min_len {
            acc = (acc << 1) | br.read_u32(1)?;
            have += 1;
        }
        loop {
            // Linear search the slice of entries with this length.
            // The table is small, so this is fine.
            for &(len, code, sym) in &self.entries {
                if len == have && code == acc {
                    return Ok(sym);
                }
            }
            if have >= self.max_len {
                return Err(Error::invalid("cook VLC: no match (max length exceeded)"));
            }
            acc = (acc << 1) | br.read_u32(1)?;
            have += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vlc_tables;
    use oxideav_core::bits::{BitReader, BitWriter};

    #[test]
    fn sqvh6_roundtrip_each_symbol() {
        let v = Vlc::new(vlc_tables::SQVH_6);
        for &(sym, len, code) in vlc_tables::SQVH_6 {
            let mut bw = BitWriter::new();
            bw.write_u32(code, len as u32);
            // Pad with extra zero bits so the reader has bits to read past
            // the end of the codeword.
            bw.write_u32(0, 24);
            let bytes = bw.into_bytes();
            let mut br = BitReader::new(&bytes);
            let got = v.decode(&mut br).unwrap();
            assert_eq!(got, sym, "roundtrip code {code:b} len {len}");
        }
    }

    #[test]
    fn env_0_first_symbols() {
        let v = Vlc::new(vlc_tables::ENV_0);
        // Code "000" (3 bits) → symbol 10 per ENV_0[0].
        let bytes = [0b0000_0000];
        let mut br = BitReader::new(&bytes);
        assert_eq!(v.decode(&mut br).unwrap(), 10);
    }

    #[test]
    fn cpl_0_three_symbols() {
        let v = Vlc::new(vlc_tables::CPL_0);
        // CPL_0: (1, 1, 0b0), (0, 2, 0b10), (2, 2, 0b11).
        // Pack codes 0, 10, 11 = "0 10 11" = 5 bits "01011" = 0b01011000.
        let bytes = [0b01011000];
        let mut br = BitReader::new(&bytes);
        assert_eq!(v.decode(&mut br).unwrap(), 1);
        assert_eq!(v.decode(&mut br).unwrap(), 0);
        assert_eq!(v.decode(&mut br).unwrap(), 2);
    }

    #[test]
    fn all_tables_construct_without_panic() {
        for t in vlc_tables::ENV_TABLES {
            let v = Vlc::new(t);
            assert!(v.max_len() > 0);
        }
        for t in vlc_tables::CPL_TABLES {
            let v = Vlc::new(t);
            assert!(v.max_len() > 0);
        }
        for t in vlc_tables::SQVH_TABLES {
            let v = Vlc::new(t);
            assert!(v.max_len() > 0);
        }
    }

    #[test]
    fn sqvh_table_sizes_match_doc() {
        // Per §3 of the VLC tables doc.
        let expected = [181usize, 94, 48, 520, 209, 192, 32];
        for (cat, &n) in expected.iter().enumerate() {
            assert_eq!(vlc_tables::SQVH_TABLES[cat].len(), n, "sqvh[{cat}] size");
        }
    }
}
