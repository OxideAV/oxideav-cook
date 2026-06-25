//! Joint-stereo coupling control read — frame-syntax part 4.1
//! (worker `cook.dll!0x3d10`).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §4.1 and
//! `docs/audio/cook/provenance/05-cook-backend.md` evidence #12
//! (*"coupling control read by `0x3d10`; coupling index VLC-or-fixed
//! (`+0x1c` bits)"*; *"`0x3d10` … then `0x3a50` (VLC) or `0x3f40` with
//! `n=[ctx+0x1c]`"*) and evidence #13 (`Ncoup = 1 << bits`).
//!
//! ## What the trace pins (wired here)
//!
//! For a stereo flavor (`channels == 2`) the backend runs `cook.dll!0x3dc0`,
//! whose first step (`cook.dll!0x3d10`) reads the **coupling control**
//! before the coupled spectrum (`spec/05` §4.1):
//!
//! 1. **Per-band coupling-index read mode.** For each coupling band one
//!    rotation index `j` is read, *either* VLC-coded (the `cook.dll!0x3a50`
//!    walk, taken when the leading flag bit is set) *or* as a fixed-width
//!    field (`read-n-bits` with `n =` the per-flavor coupling-index bit
//!    width, context `+0x1c`). The leading flag bit is read once and
//!    selects the mode for the whole coupling control. [`CouplingReadMode`]
//!    types the two branches; [`read_coupling_mode`] reads the leading flag
//!    and classifies. The **fixed-width** branch is fully implementable from
//!    the [`FrameBitReader`] ([`read_fixed_coupling_index`]); the
//!    **VLC** branch descends the §3.2 BSS-built codebooks and is the
//!    recorded blocker.
//! 2. **Coupling table length.** `Ncoup = 1 << coupling_bits` is the number
//!    of quantised rotation angles for one coupling band — the length of
//!    the per-coupling-width coefficient table the §4.2 mirror-index split
//!    reads (already exposed as [`crate::spectral::coupling_table_len`];
//!    re-checked here as [`coupling_table_len`] for the control read).
//!
//! ## What stays a GAP (not wired)
//!
//! - The **coupling-band boundary derivation** — `spec/05` §4.1 states
//!   *"the coupled region spans a contiguous range of subbands, with the
//!   boundary derived from the per-channel subband count (context `+0x18`)
//!   and a per-flavor coupling start"* but gives **no closed form** for the
//!   start/count, so the exact boundary is a recorded DOCS-GAP. The caller
//!   supplies the contiguous `[first..last)` coupling-band range (the
//!   §4 [`crate::frame::StereoCoupling`] input).
//! - The **VLC coupling-index read** descends the seven §3.2 BSS codebooks
//!   (built at init, not in the file image) — the documented blocker.
//!   [`read_coupling_index`] performs the fixed-width branch directly and
//!   surfaces [`Error::SpectralCodebookBytesUnavailable`] for the VLC
//!   branch rather than guessing.
//! - The **rotation coefficient values** (§4.3) are BSS-built (caller
//!   input); only the mirror-index closed form is pinned (§4.2, wired in
//!   [`crate::spectral`]).
//!
//! ## Wall-respect note
//!
//! Every fact here is anchored to `spec/05` §4.1 / §4.2 and
//! `provenance/05` evidence #12 / #13. The fixed-width read is the
//! trace's `read-n-bits(n=[ctx+0x1c])`; the VLC branch and the band
//! boundary are surfaced as the documented blocker / GAP, not guessed.

use crate::{bitreader::FrameBitReader, Error};

/// Context offset of the per-flavor coupling-index bit width
/// (`+0x1c`, `spec/05` §4.1: *"`read-n-bits` with `n` = context `+0x1c`,
/// the per-flavor coupling-index bit width"*).
pub const CTX_COUPLING_INDEX_BITS_OFFSET: u32 = 0x1c;

/// Context offset of the per-channel subband count the coupling-band
/// boundary is derived from (`+0x18`, `spec/05` §4.1). The exact
/// derivation is a recorded GAP; this constant surfaces the offset the
/// trace names.
pub const CTX_SUBBAND_COUNT_OFFSET: u32 = 0x18;

/// The per-band coupling-index read mode (`spec/05` §4.1): the leading
/// flag bit of the coupling control selects one of these for the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CouplingReadMode {
    /// The leading flag bit was **set** — each coupling-band index is
    /// VLC-coded via the `cook.dll!0x3a50` walk over the §3.2 BSS-built
    /// codebooks (the recorded blocker).
    Vlc,
    /// The leading flag bit was **clear** — each coupling-band index is a
    /// fixed-width field (`read-n-bits` with `n =` the per-flavor coupling
    /// bit width, context `+0x1c`). Fully implementable from the bit
    /// reader.
    Fixed {
        /// The per-flavor coupling-index bit width (context `+0x1c`).
        bits: u32,
    },
}

/// The length of the per-coupling-width rotation coefficient table —
/// `Ncoup = 1 << coupling_bits` (`spec/05` §4.2, evidence #13).
///
/// Identical to [`crate::spectral::coupling_table_len`]; re-exposed here
/// for the coupling-control read where the bit width is in hand.
#[inline]
#[must_use]
pub const fn coupling_table_len(coupling_bits: u32) -> u32 {
    1u32 << coupling_bits
}

/// Read the leading coupling-control flag bit and classify the per-band
/// index read mode (`spec/05` §4.1).
///
/// `coupling_bits` is the per-flavor coupling-index bit width (context
/// `+0x1c`) used by the fixed-width branch. The leading flag bit is read
/// from `reader`: set → [`CouplingReadMode::Vlc`], clear →
/// [`CouplingReadMode::Fixed`].
///
/// This consumes exactly one bit from the reader (the flag); the per-band
/// index reads follow.
#[must_use]
pub fn read_coupling_mode(reader: &mut FrameBitReader, coupling_bits: u32) -> CouplingReadMode {
    if reader.read_bit() != 0 {
        CouplingReadMode::Vlc
    } else {
        CouplingReadMode::Fixed {
            bits: coupling_bits,
        }
    }
}

/// Read one fixed-width coupling/rotation index from the bitstream —
/// `read-n-bits(coupling_bits)` (`spec/05` §4.1, the non-VLC branch).
///
/// Returns the raw index `j` in `0..Ncoup` (`Ncoup = 1 << coupling_bits`),
/// the angle quantiser the §4.2 mirror split consumes. This is the
/// `cook.dll!0x3f40` read with `n = [ctx+0x1c]`.
#[must_use]
pub fn read_fixed_coupling_index(reader: &mut FrameBitReader, coupling_bits: u32) -> u32 {
    reader.read_bits(coupling_bits)
}

/// Read one coupling/rotation index in the resolved [`CouplingReadMode`].
///
/// - [`CouplingReadMode::Fixed`] → the fixed-width
///   [`read_fixed_coupling_index`] (fully implemented).
/// - [`CouplingReadMode::Vlc`] → the `cook.dll!0x3a50` walk over the §3.2
///   BSS-built codebooks, which are not in the file image (docs-gap
///   #1775); surfaced as [`Error::SpectralCodebookBytesUnavailable`]
///   rather than guessed.
///
/// # Errors
///
/// Returns [`Error::SpectralCodebookBytesUnavailable`] for the VLC branch.
pub fn read_coupling_index(
    reader: &mut FrameBitReader,
    mode: CouplingReadMode,
) -> Result<u32, Error> {
    match mode {
        CouplingReadMode::Fixed { bits } => Ok(read_fixed_coupling_index(reader, bits)),
        CouplingReadMode::Vlc => Err(Error::SpectralCodebookBytesUnavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coupling_table_len_matches_spectral_module() {
        for bits in 0u32..=6 {
            assert_eq!(
                coupling_table_len(bits),
                crate::spectral::coupling_table_len(bits)
            );
        }
    }

    #[test]
    fn read_mode_flag_set_is_vlc() {
        // Leading bit 1 → VLC mode (then the index read would descend the
        // §3.2 BSS codebooks).
        let frame = [0b1000_0000u8, 0, 0, 0, 0, 0, 0, 0];
        let mut reader = FrameBitReader::new(&frame);
        let mode = read_coupling_mode(&mut reader, 3);
        assert_eq!(mode, CouplingReadMode::Vlc);
        assert_eq!(reader.bit_cursor(), 1);
    }

    #[test]
    fn read_mode_flag_clear_is_fixed() {
        // Leading bit 0 → fixed-width mode carrying the coupling bit width.
        let frame = [0b0000_0000u8, 0, 0, 0, 0, 0, 0, 0];
        let mut reader = FrameBitReader::new(&frame);
        let mode = read_coupling_mode(&mut reader, 4);
        assert_eq!(mode, CouplingReadMode::Fixed { bits: 4 });
        assert_eq!(reader.bit_cursor(), 1);
    }

    #[test]
    fn fixed_index_reads_n_bits() {
        // After the leading flag bit (0 → fixed mode), the fixed-width
        // read returns a value strictly inside the coupling table range
        // and equals a direct read_bits(3) on the same post-flag cursor.
        let frame = [0b0101_0000u8, 0, 0, 0, 0, 0, 0, 0];
        let mut reader = FrameBitReader::new(&frame);
        let mode = read_coupling_mode(&mut reader, 3);
        assert_eq!(mode, CouplingReadMode::Fixed { bits: 3 });
        // Cross-check against a fresh reader advanced past the flag.
        let mut bare = FrameBitReader::new(&frame);
        let _ = bare.read_bit();
        let expected = bare.read_bits(3);
        let j = read_coupling_index(&mut reader, mode).unwrap();
        assert_eq!(j, expected);
        assert!(j < coupling_table_len(3));
    }

    #[test]
    fn fixed_index_in_range_for_width() {
        // A fixed-width read is always < Ncoup = 1 << bits, even for an
        // all-ones payload (leading bit 0 forces fixed mode, rest 1s).
        for bits in 1u32..=6 {
            let mut frame = [0xFFu8; 8];
            frame[0] = 0x7F; // leading bit 0 (fixed mode), rest all 1s.
            let mut reader = FrameBitReader::new(&frame);
            let mode = read_coupling_mode(&mut reader, bits);
            assert_eq!(mode, CouplingReadMode::Fixed { bits });
            let j = read_coupling_index(&mut reader, mode).unwrap();
            assert!(j < coupling_table_len(bits), "bits {bits} j {j}");
        }
    }

    #[test]
    fn vlc_index_surfaces_bss_blocker() {
        // The VLC branch descends the §3.2 BSS codebooks (docs-gap #1775).
        let mut reader = FrameBitReader::new(&[0u8; 4]);
        assert_eq!(
            read_coupling_index(&mut reader, CouplingReadMode::Vlc).unwrap_err(),
            Error::SpectralCodebookBytesUnavailable
        );
    }

    #[test]
    fn context_offsets_match_spec() {
        assert_eq!(CTX_COUPLING_INDEX_BITS_OFFSET, 0x1c);
        assert_eq!(CTX_SUBBAND_COUNT_OFFSET, 0x18);
    }

    #[test]
    fn read_multiple_fixed_indices_for_bands() {
        // Read three fixed-width 2-bit indices after the flag bit; each
        // is a valid base-Ncoup digit, and the sequence matches a direct
        // read on a fresh reader advanced past the same flag.
        let frame = [0b0_11_01_11u8, 0, 0, 0, 0, 0, 0, 0];
        let mut reader = FrameBitReader::new(&frame);
        let mode = read_coupling_mode(&mut reader, 2);
        let mut got = Vec::new();
        for _ in 0..3 {
            got.push(read_coupling_index(&mut reader, mode).unwrap());
        }
        // Reference: same flag + three read_bits(2) on a bare reader.
        let mut bare = FrameBitReader::new(&frame);
        let _ = bare.read_bit();
        let want: Vec<u32> = (0..3).map(|_| bare.read_bits(2)).collect();
        assert_eq!(got, want);
        for &j in &got {
            assert!(j < coupling_table_len(2), "j {j} out of range");
        }
    }
}
