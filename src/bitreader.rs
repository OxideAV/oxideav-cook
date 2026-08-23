//! MSB-first big-endian frame bit reader (`spec/05` §0.1).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §0.1 (the bit
//! reader state block and the two reader primitives) and
//! `docs/audio/cook/provenance/05-cook-backend.md` evidence #1 (the
//! closed-form assembly `word << pos | next >> (32-pos)` then `>> (32-n)`
//! and the four-field reader state block).
//!
//! ## What the trace pins (wired here)
//!
//! The backend reads each sub-packet's frame bitstream **MSB-first,
//! big-endian**, through a small reader state block held at fixed offsets
//! in the per-channel decode context (`spec/05` §0.1):
//!
//! | Context offset | Field | Meaning |
//! | -------------- | ----- | ------- |
//! | `+0x479c` | word pointer | pointer to the current 32-bit big-endian input word |
//! | `+0x47a0` | bit position | bits already consumed from the current word (`0..31`) |
//! | `+0x47a4` | bit cursor | running count of bits consumed in this frame |
//! | `+0x47a8` | bit limit | total frame size in bits; reads past it return `0` |
//!
//! Two reader primitives are used throughout the backend frame body:
//!
//! - **read-`n`-bits** (`cook.dll!0x3f40`): assembles `n` bits MSB-first
//!   across the word boundary — the closed form is
//!   `word << pos | next >> (32 - pos)`, then `>> (32 - n)` — advances
//!   the cursor, and returns `0` (with the cursor clamped to the bit
//!   limit) once the limit is hit. `n` is a caller immediate. Wired as
//!   [`FrameBitReader::read_bits`].
//! - **read-1-bit** (`cook.dll!0x3fc0`): returns the next single bit; the
//!   binary arithmetic-shifts it so the result is `0` / `-1` (a signed
//!   flag), and it likewise returns `0` at end of frame. Wired as the
//!   unsigned [`FrameBitReader::read_bit`] (`0` / `1`) plus the signed
//!   flag form [`FrameBitReader::read_flag`] (`0` / `-1`) that mirrors the
//!   binary's arithmetic-shift result.
//!
//! ## Word view of the input
//!
//! The reader's `word pointer` walks the input one **big-endian 32-bit
//! word** at a time, and the assembly straddles a word boundary by also
//! reading the *next* word (`next >> (32 - pos)`). This module therefore
//! views the input byte slice as a sequence of big-endian `u32` words: a
//! word index `w` reads bytes `[4w .. 4w+4]` big-endian, and any byte
//! position at or past the slice end (a trailing partial word, or the
//! word *after* the last one consulted by the straddle) reads as `0`. The
//! authoritative end-of-stream gate is the **bit limit** (`+0x47a8`), set
//! by the caller to the frame size in bits; a read whose span reaches or
//! crosses the limit yields `0` and clamps the cursor at the limit,
//! exactly as the binary does. The zero-extension of out-of-range words is
//! the same observable behaviour reached through the bit-limit clamp on
//! any well-formed frame (the limit is never larger than the supplied
//! bytes for a real packet), so it is a totalising convenience, not an
//! independent claim.
//!
//! ## What stays a GAP (not wired)
//!
//! This module wires *only* the reader primitives. The frame body that
//! drives them — the gain envelope (§1), the category/quant walk (§2), the
//! spectral VLC descent (§3.1, the bit-by-bit walk `cook.dll!0x3a50` over
//! the BSS-built codebooks of §3.2) and the inverse transform (§5) — is
//! not assembled here; those stages and the runtime-built codebook /
//! coupling tables remain recorded GAPs (`spec/05` §3.2 / §4.3 / §6).
//!
//! ## Wall-respect note
//!
//! Every fact here is anchored to `spec/05` §0.1 and `provenance/05`
//! evidence #1; no algorithmic content beyond the two pinned reader
//! primitives and the four-field state block is wired.

/// Number of bits in one input word (`spec/05` §0.1: the reader walks the
/// input as 32-bit big-endian words; `32 - pos` / `32 - n` are the
/// closed-form shift amounts).
pub const WORD_BITS: u32 = 32;

/// Number of bytes in one input word.
pub const WORD_BYTES: usize = 4;

/// Context offset of the reader's **word pointer** field (`+0x47ac`,
/// `spec/05` §0.1; round 9 corrected the four offsets — they were
/// observed directly as the stores a live `RADecode` makes into the
/// backend object, `0x10` above the earlier static reading). Surfaced
/// as a named constant only — the Rust reader holds the equivalent
/// state inline.
pub const CTX_WORD_POINTER_OFFSET: u32 = 0x47ac;

/// Context offset of the reader's **bit position** field (`+0x47b0`,
/// bits already consumed from the current word, `0..31`).
pub const CTX_BIT_POSITION_OFFSET: u32 = 0x47b0;

/// Context offset of the reader's **bit cursor** field (`+0x47b4`, the
/// running count of bits consumed in this frame — the field whose
/// stores the round-9 trace watched to recover the wire layout).
pub const CTX_BIT_CURSOR_OFFSET: u32 = 0x47b4;

/// Context offset of the reader's **bit limit** field (`+0x47b8`, total
/// frame size in bits; reads at or past it return `0`).
pub const CTX_BIT_LIMIT_OFFSET: u32 = 0x47b8;

/// MSB-first big-endian frame bit reader (`spec/05` §0.1).
///
/// Holds the four-field reader state inline (the binary keeps it at
/// `[ctx+0x47ac..0x47b8]`): the input byte slice viewed as big-endian
/// 32-bit words, the running **bit cursor** (`+0x47a4`), and the **bit
/// limit** (`+0x47a8`). The word pointer (`+0x479c`) and within-word bit
/// position (`+0x47a0`) are derived from the cursor on each read
/// (`word = cursor / 32`, `pos = cursor % 32`), the same decomposition the
/// binary maintains incrementally.
#[derive(Debug, Clone)]
pub struct FrameBitReader<'a> {
    /// Frame input bytes, viewed as a sequence of big-endian 32-bit words.
    data: &'a [u8],
    /// Running count of bits consumed in this frame (`+0x47a4`).
    cursor: u32,
    /// Total frame size in bits (`+0x47a8`); reads at or past it return
    /// `0` and clamp the cursor here.
    limit: u32,
}

impl<'a> FrameBitReader<'a> {
    /// Builds a reader over `data` with the bit limit set to the full byte
    /// length (`data.len() * 8` bits). This is the natural framing when a
    /// whole sub-packet is the frame; callers with a sub-bit-exact frame
    /// size use [`FrameBitReader::with_bit_limit`].
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        let limit = (data.len() as u64 * 8) as u32;
        Self {
            data,
            cursor: 0,
            limit,
        }
    }

    /// Builds a reader over `data` with an explicit **bit limit**
    /// (`+0x47a8`, `spec/05` §0.1): a read whose span reaches or crosses
    /// `bit_limit` yields `0` and clamps the cursor at the limit. The
    /// limit is the authoritative end-of-frame gate; it is independent of
    /// (and normally `<=`) the supplied byte length.
    #[must_use]
    pub fn with_bit_limit(data: &'a [u8], bit_limit: u32) -> Self {
        Self {
            data,
            cursor: 0,
            limit: bit_limit,
        }
    }

    /// The running **bit cursor** (`+0x47a4`): bits consumed so far this
    /// frame. Clamped at [`FrameBitReader::bit_limit`] once a read hits
    /// the limit.
    #[must_use]
    pub fn bit_cursor(&self) -> u32 {
        self.cursor
    }

    /// The **bit limit** (`+0x47a8`): the frame size in bits at or past
    /// which reads return `0`.
    #[must_use]
    pub fn bit_limit(&self) -> u32 {
        self.limit
    }

    /// The current **word pointer** (`+0x479c`), as a word index: the
    /// big-endian 32-bit word the next read starts in (`cursor / 32`).
    #[must_use]
    pub fn word_index(&self) -> u32 {
        self.cursor / WORD_BITS
    }

    /// The current within-word **bit position** (`+0x47a0`, `0..31`): bits
    /// already consumed from the current word (`cursor % 32`).
    #[must_use]
    pub fn bit_position(&self) -> u32 {
        self.cursor % WORD_BITS
    }

    /// Bits remaining before the bit limit (`limit - cursor`, saturating).
    #[must_use]
    pub fn bits_remaining(&self) -> u32 {
        self.limit.saturating_sub(self.cursor)
    }

    /// Whether the cursor has reached the bit limit (every further read
    /// returns `0`).
    #[must_use]
    pub fn at_end(&self) -> bool {
        self.cursor >= self.limit
    }

    /// Reads the big-endian 32-bit word at word index `w`, zero-extending
    /// any byte at or past the slice end (`spec/05` §0.1: the word pointer
    /// straddle reads the *next* word, which past the input is `0`; the
    /// authoritative gate is the bit limit).
    fn word_at(&self, w: u32) -> u32 {
        let base = (w as usize).wrapping_mul(WORD_BYTES);
        let mut acc: u32 = 0;
        let mut i = 0;
        while i < WORD_BYTES {
            let byte = self.data.get(base.wrapping_add(i)).copied().unwrap_or(0);
            acc = (acc << 8) | u32::from(byte);
            i += 1;
        }
        acc
    }

    /// Reads `n` bits MSB-first (`read-n-bits`, `cook.dll!0x3f40`,
    /// `spec/05` §0.1).
    ///
    /// Assembles `n` bits across the word boundary by the pinned closed
    /// form `word << pos | next >> (32 - pos)`, then `>> (32 - n)`, and
    /// advances the cursor by `n`. A read whose span reaches or crosses
    /// the bit limit returns `0` and clamps the cursor at the limit, with
    /// no partial value — exactly the binary's end-of-frame behaviour.
    ///
    /// `n` must be in `1..=32` (the binary's `n` is a caller immediate in
    /// that range; `n = 0` and `n > 32` are not produced by any worker and
    /// have no MSB-first meaning under the `32 - n` shift). An out-of-range
    /// `n` returns `0` without advancing, matching the limit-clamped
    /// no-value path.
    pub fn read_bits(&mut self, n: u32) -> u32 {
        if n == 0 || n > WORD_BITS {
            return 0;
        }
        // End-of-frame gate (+0x47a8): a span reaching or crossing the
        // limit yields 0 and clamps the cursor.
        if self.cursor.saturating_add(n) > self.limit {
            self.cursor = self.limit;
            return 0;
        }

        let w = self.cursor / WORD_BITS;
        let pos = self.cursor % WORD_BITS;

        // word << pos | next >> (32 - pos)   (the straddle; when pos == 0
        // the `next >> 32` term is zero and the shift-by-32 is avoided).
        let cur = self.word_at(w);
        let aligned = if pos == 0 {
            cur
        } else {
            let next = self.word_at(w + 1);
            (cur << pos) | (next >> (WORD_BITS - pos))
        };

        // >> (32 - n): drop the low (32 - n) bits, leaving the n MSBs.
        let value = aligned >> (WORD_BITS - n);

        self.cursor += n;
        value
    }

    /// Reads the next single bit as an unsigned `0` / `1`
    /// (`read-1-bit`, `cook.dll!0x3fc0`, `spec/05` §0.1). Returns `0` at
    /// end of frame. This is `read_bits(1)`.
    pub fn read_bit(&mut self) -> u32 {
        self.read_bits(1)
    }

    /// Advances the cursor by `n` bits without assembling a value —
    /// the walk-level skip a caller uses to step over a region whose
    /// *values* it holds from another source (e.g. the injected
    /// envelope array of [`crate::frame::EnvelopeInjection`], whose
    /// wire form is the unstaged envelope VLC). Clamps at the bit limit
    /// like every read.
    pub fn skip_bits(&mut self, n: u32) {
        self.cursor = self.cursor.saturating_add(n).min(self.limit);
    }

    /// Reads the next single bit as the **signed flag** the binary
    /// produces (`read-1-bit` arithmetic-shifts the bit so a set bit is
    /// `-1` and a clear bit is `0`; `spec/05` §0.1). Returns `0` at end of
    /// frame. Used for the VLC bit-by-bit walk and one-bit flags.
    pub fn read_flag(&mut self) -> i32 {
        // Arithmetic-shift of a single MSB-first bit: 1 -> -1, 0 -> 0.
        0i32.wrapping_sub(self.read_bit() as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `spec/05` §0.1: the four reader-state context offsets are
    /// consecutive 4-byte fields starting at `+0x479c`.
    #[test]
    fn ctx_offsets_are_consecutive_words() {
        assert_eq!(CTX_BIT_POSITION_OFFSET, CTX_WORD_POINTER_OFFSET + 4);
        assert_eq!(CTX_BIT_CURSOR_OFFSET, CTX_BIT_POSITION_OFFSET + 4);
        assert_eq!(CTX_BIT_LIMIT_OFFSET, CTX_BIT_CURSOR_OFFSET + 4);
    }

    /// A fresh reader starts at cursor 0, word 0, bit position 0, with the
    /// default limit at the full byte length in bits.
    #[test]
    fn new_starts_at_origin() {
        let r = FrameBitReader::new(&[0x12, 0x34, 0x56, 0x78]);
        assert_eq!(r.bit_cursor(), 0);
        assert_eq!(r.word_index(), 0);
        assert_eq!(r.bit_position(), 0);
        assert_eq!(r.bit_limit(), 32);
        assert_eq!(r.bits_remaining(), 32);
        assert!(!r.at_end());
    }

    /// MSB-first: reading the top bits of a known big-endian word returns
    /// the high bits in order. `0x12 = 0001_0010`, so the first four bits
    /// are `0001 = 1`, the next four `0010 = 2`.
    #[test]
    fn read_bits_msb_first_within_word() {
        let mut r = FrameBitReader::new(&[0x12, 0x34, 0x56, 0x78]);
        assert_eq!(r.read_bits(4), 0x1);
        assert_eq!(r.read_bits(4), 0x2);
        assert_eq!(r.read_bits(8), 0x34);
        assert_eq!(r.bit_cursor(), 16);
        assert_eq!(r.bit_position(), 16);
    }

    /// Reading a full 32-bit word returns the big-endian word value.
    #[test]
    fn read_full_word() {
        let mut r = FrameBitReader::new(&[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(r.read_bits(32), 0xDEAD_BEEF);
        assert_eq!(r.bit_cursor(), 32);
        assert!(r.at_end());
    }

    /// The straddle: a read that crosses the 32-bit word boundary
    /// assembles `word << pos | next >> (32 - pos)` correctly. Two words
    /// `0x0000_00FF` `0xFF00_0000` form the bit pattern `...0000 1111_1111
    /// 1111_1111 0000...`; reading 24 bits then 16 bits walks across the
    /// boundary at bit 32.
    #[test]
    fn read_bits_across_word_boundary() {
        // bytes: 00 00 00 FF | FF 00 00 00
        let data = [0x00, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x00];
        let mut r = FrameBitReader::with_bit_limit(&data, 64);
        // First 24 bits are all zero.
        assert_eq!(r.read_bits(24), 0);
        assert_eq!(r.bit_cursor(), 24);
        // Next 16 bits span [24..40): low 8 of word 0 (0xFF) then high 8
        // of word 1 (0xFF) => 0xFFFF.
        assert_eq!(r.read_bits(16), 0xFFFF);
        assert_eq!(r.bit_cursor(), 40);
        assert_eq!(r.word_index(), 1);
        assert_eq!(r.bit_position(), 8);
        // Remaining 24 bits of word 1 are zero.
        assert_eq!(r.read_bits(24), 0);
        assert!(r.at_end());
    }

    /// A read straddling exactly at `pos == 0` after a full word avoids
    /// the shift-by-32 and reads the next whole word.
    #[test]
    fn read_word_then_next_word() {
        let data = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let mut r = FrameBitReader::with_bit_limit(&data, 64);
        assert_eq!(r.read_bits(32), 0x1122_3344);
        assert_eq!(r.bit_position(), 0);
        assert_eq!(r.read_bits(32), 0x5566_7788);
    }

    /// read_bit / read_flag agree on the same bits: `0xA0 = 1010_0000`.
    #[test]
    fn read_bit_and_flag_forms() {
        let mut r = FrameBitReader::new(&[0xA0, 0x00, 0x00, 0x00]);
        assert_eq!(r.read_bit(), 1);
        assert_eq!(r.read_bit(), 0);
        assert_eq!(r.read_bit(), 1);
        assert_eq!(r.read_bit(), 0);

        let mut r2 = FrameBitReader::new(&[0xA0, 0x00, 0x00, 0x00]);
        // Signed-flag form: set bit -> -1, clear bit -> 0.
        assert_eq!(r2.read_flag(), -1);
        assert_eq!(r2.read_flag(), 0);
        assert_eq!(r2.read_flag(), -1);
        assert_eq!(r2.read_flag(), 0);
    }

    /// Reads at or past the bit limit return 0 and clamp the cursor at the
    /// limit (`spec/05` §0.1: "reads past it return 0").
    #[test]
    fn read_past_bit_limit_returns_zero_and_clamps() {
        // 8 bits of real data, limit set to 8 bits.
        let data = [0xFF, 0x00, 0x00, 0x00];
        let mut r = FrameBitReader::with_bit_limit(&data, 8);
        assert_eq!(r.read_bits(8), 0xFF);
        assert_eq!(r.bit_cursor(), 8);
        assert!(r.at_end());
        // A read that would cross the limit returns 0 and clamps.
        assert_eq!(r.read_bits(4), 0);
        assert_eq!(r.bit_cursor(), 8);
        assert_eq!(r.bits_remaining(), 0);
        // read_bit / read_flag also return 0 at end of frame.
        assert_eq!(r.read_bit(), 0);
        assert_eq!(r.read_flag(), 0);
    }

    /// A read whose last bit lands exactly on the limit is permitted; one
    /// bit further is not.
    #[test]
    fn read_exactly_to_limit_is_allowed() {
        let data = [0xFF, 0xFF, 0xFF, 0xFF];
        let mut r = FrameBitReader::with_bit_limit(&data, 12);
        assert_eq!(r.read_bits(12), 0xFFF);
        assert_eq!(r.bit_cursor(), 12);
        assert!(r.at_end());
        assert_eq!(r.read_bit(), 0);
    }

    /// Out-of-range `n` (0 or > 32) returns 0 without advancing the
    /// cursor — no MSB-first meaning under the `32 - n` shift.
    #[test]
    fn read_bits_out_of_range_n_is_inert() {
        let mut r = FrameBitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(r.read_bits(0), 0);
        assert_eq!(r.bit_cursor(), 0);
        assert_eq!(r.read_bits(33), 0);
        assert_eq!(r.bit_cursor(), 0);
        // A valid read still works afterwards.
        assert_eq!(r.read_bits(4), 0xF);
    }

    /// Sequential single-bit reads reproduce the same value as one
    /// multi-bit read of the same span (`read_bits(1)` == `read_bit`,
    /// and the bits compose MSB-first). `0x96 = 1001_0110`.
    #[test]
    fn single_bit_reads_compose_to_multibit() {
        let bytes = [0x96, 0x00, 0x00, 0x00];
        let mut multi = FrameBitReader::new(&bytes);
        let whole = multi.read_bits(8);

        let mut single = FrameBitReader::new(&bytes);
        let mut acc = 0u32;
        for _ in 0..8 {
            acc = (acc << 1) | single.read_bit();
        }
        assert_eq!(acc, whole);
        assert_eq!(acc, 0x96);
    }

    /// The reader walks a whole multi-word frame consistently: cursor,
    /// word index and bit position stay in lockstep across mixed-width
    /// reads.
    #[test]
    fn cursor_state_stays_consistent() {
        let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        let mut r = FrameBitReader::with_bit_limit(&data, 64);
        r.read_bits(3);
        assert_eq!(r.bit_cursor(), 3);
        assert_eq!(r.word_index(), 0);
        assert_eq!(r.bit_position(), 3);
        r.read_bits(30); // crosses into word 1
        assert_eq!(r.bit_cursor(), 33);
        assert_eq!(r.word_index(), 1);
        assert_eq!(r.bit_position(), 1);
        r.read_bits(31);
        assert_eq!(r.bit_cursor(), 64);
        assert!(r.at_end());
    }
}
