//! Per-buffer XOR descramble — the first byte-touching stage of the
//! `RADecode` decode driver.
//!
//! Source-of-truth: `docs/audio/cook/spec/01-cook-decoder-structure.md`
//! §5 (the `0x1283` loop inside the decode driver `0x1260`, with the
//! Round-3 audit-clarification block) and
//! `docs/audio/cook/validation/04-cook-stream-validation.md` §4.3 / §5.
//!
//! ## What the binary does
//!
//! `RADecode` (`0x1260`) optionally applies a **word-wise (32-bit) XOR
//! pass** over the input region before handing units to the backend.
//! Each 32-bit word is XORed with a per-call key `input_ptr XOR
//! input_len` (spec/01 §5). The pass is **conditional on the
//! common-mode flag** at context `+0x30`: it runs only when that flag is
//! non-zero. The flag is zero-initialised by the constructor and set to
//! `1` by `RASetComMode` (export ordinal 18, worker `0x16a0`, spec/01
//! §2); there is no SPI to clear it again in this build. When common
//! mode is **off** (the default) the input is consumed verbatim with no
//! XOR pass — which is exactly the path the validator drove: the 144
//! real packets of `FUN_RM_32.rm` were fed straight from the container
//! and matched 144/144 S_OK with no external descramble
//! (validation/04 §4.3 / §5).
//!
//! ## Pure-Rust shape
//!
//! The binary mutates the input region in place (`xor [this],edx` over
//! 32-bit slots). In safe Rust the input is a `&[u8]` we cannot mutate,
//! so [`xor_descramble`] returns an owned descrambled buffer and
//! [`xor_descramble_into`] writes into a caller-owned slice.
//!
//! The key in the binary is the 32-bit value `input_ptr ^ input_len`;
//! a stable input pointer does not exist in safe Rust, so [`xor_key`]
//! takes both factors as explicit `u32` arguments and the descrambler
//! itself takes the already-computed key. This keeps the arithmetic
//! byte-identical to the binary without any unsafe pointer-to-int
//! conversion. A real stream supplies whatever 32-bit value the binary
//! would see for the pointer; tests and external callers supply any
//! 32-bit nonce.
//!
//! ## Endianness
//!
//! The decoder is PE32 i386 (little-endian). The pass reads each 32-bit
//! word little-endian, XORs it with the key, and writes it back
//! little-endian, so byte order is preserved under round-trip.
//!
//! ## Tail handling (DOCS-GAP)
//!
//! The binary loop is word-aligned (32-bit slots). What it does with a
//! trailing `input.len() % 4 != 0` partial word is **not pinned** by
//! spec/01 §5 or validation/04 (the validated stream's 465-byte packets
//! never exercised the on-path, so there is no ground truth). This is a
//! recorded DOCS-GAP. The conservative choice taken here is to **copy
//! the trailing `< 4` bytes verbatim** (no XOR). This keeps the pass
//! self-inverse for any length and never fabricates key bytes for a
//! partial slot.

use std::borrow::Cow;

/// Common-mode toggle gating the per-buffer XOR descramble.
///
/// Mirrors the context `+0x30` flag of spec/01 §5: zero-initialised by
/// the constructor (so the default is [`CommonMode::off`]) and set to
/// `1` by `RASetComMode` ([`CommonMode::on`]). There is no SPI in this
/// build to turn it off again once set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommonMode(bool);

impl CommonMode {
    /// Common mode off — the constructor default. The input buffer is
    /// consumed verbatim with no XOR pass (validation/04 §4.3: the only
    /// path with real-stream PCM ground truth).
    pub const fn off() -> Self {
        CommonMode(false)
    }

    /// Common mode on — the state `RASetComMode` (spec/01 §2) installs by
    /// writing `1` to context `+0x30`. The per-buffer XOR descramble of
    /// spec/01 §5 runs.
    pub const fn on() -> Self {
        CommonMode(true)
    }

    /// Whether the XOR descramble pass is active.
    pub const fn is_on(&self) -> bool {
        self.0
    }
}

/// The per-call descramble key.
///
/// The binary computes `input_ptr XOR input_len` (spec/01 §5,
/// validation/04 §4.3). Both factors are explicit `u32` arguments so the
/// arithmetic is identical to the binary while the descrambler stays free
/// of any unsafe pointer-to-int conversion: `in_ptr` is whatever 32-bit
/// value represents the input pointer (the address the binary would see
/// for a real stream; any 32-bit nonce for tests / external callers) and
/// `in_len` is the input length in bytes.
pub fn xor_key(in_ptr: u32, in_len: u32) -> u32 {
    in_ptr ^ in_len
}

/// Word-wise XOR-descramble `input`, returning an owned buffer.
///
/// Reads 32-bit little-endian words from `input`, XORs each with `key`
/// (compute it via [`xor_key`]), and writes the same little-endian word
/// back. A trailing partial word (`input.len() % 4 != 0`) is copied
/// verbatim — see the module-level tail-handling DOCS-GAP note.
///
/// XOR is involutive, so `xor_descramble(&xor_descramble(buf, k), k)`
/// reproduces `buf` for any `k`. Source: spec/01 §5, validation/04 §4.3.
pub fn xor_descramble(input: &[u8], key: u32) -> Vec<u8> {
    let mut out = vec![0u8; input.len()];
    xor_descramble_into(input, key, &mut out);
    out
}

/// Word-wise XOR-descramble `input` into a caller-owned `out` (no
/// allocation). `out.len()` must equal `input.len()`.
///
/// Same word-wise little-endian pass and verbatim-tail behaviour as
/// [`xor_descramble`]; this variant exists for callers that already own
/// the output buffer. Source: spec/01 §5, validation/04 §4.3.
///
/// # Panics
///
/// Panics if `out.len() != input.len()`.
pub fn xor_descramble_into(input: &[u8], key: u32, out: &mut [u8]) {
    assert_eq!(
        input.len(),
        out.len(),
        "xor_descramble_into: out buffer length must equal input length"
    );
    let full_words = input.len() / 4;
    for w in 0..full_words {
        let off = w * 4;
        let word = u32::from_le_bytes([input[off], input[off + 1], input[off + 2], input[off + 3]]);
        out[off..off + 4].copy_from_slice(&(word ^ key).to_le_bytes());
    }
    // Tail: a partial word of `< 4` bytes is copied verbatim (DOCS-GAP,
    // conservative choice — see module docs).
    let tail = full_words * 4;
    out[tail..].copy_from_slice(&input[tail..]);
}

/// Descramble one input buffer according to the common-mode flag.
///
/// When `common_mode` is [`CommonMode::off`] (the constructor default)
/// the packet is returned verbatim as a zero-copy [`Cow::Borrowed`] —
/// the real-stream path validated end-to-end in validation/04 §4.3 / §5.
/// When it is [`CommonMode::on`] the word-wise XOR pass of [`xor_descramble`]
/// runs and the result is a [`Cow::Owned`]. Source: spec/01 §5.
pub fn descramble_packet(common_mode: CommonMode, packet: &[u8], key: u32) -> Cow<'_, [u8]> {
    if common_mode.is_on() {
        Cow::Owned(xor_descramble(packet, key))
    } else {
        Cow::Borrowed(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `xor_key` is exactly `in_ptr ^ in_len` (spec/01 §5).
    #[test]
    fn xor_key_is_ptr_xor_len() {
        assert_eq!(xor_key(0, 0), 0);
        assert_eq!(xor_key(0xDEAD_BEEF, 0), 0xDEAD_BEEF);
        assert_eq!(xor_key(0, 465), 465);
        assert_eq!(xor_key(0x1000_0000, 0x0000_01D1), 0x1000_01D1);
        assert_eq!(xor_key(0xFFFF_FFFF, 0xFFFF_FFFF), 0);
    }

    /// The pass is self-inverse for an arbitrary buffer and key.
    #[test]
    fn xor_descramble_is_self_inverse() {
        let buf: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        let key = 0xA5A5_5A5Au32;
        let once = xor_descramble(&buf, key);
        let twice = xor_descramble(&once, key);
        assert_eq!(twice, buf, "double XOR with same key restores input");
        assert_ne!(once, buf, "single XOR with a non-zero key changes bytes");
        assert_eq!(once.len(), buf.len(), "length preserved");
    }

    /// A length with a non-zero partial word (`% 4 == 1`, like the
    /// validated 465-byte packet) round-trips, and the verbatim tail
    /// byte survives a single pass unchanged (DOCS-GAP tail choice).
    #[test]
    fn tail_partial_word_round_trips_and_is_verbatim() {
        let buf: Vec<u8> = (0u32..465).map(|i| i.wrapping_mul(7) as u8).collect();
        assert_eq!(buf.len() % 4, 1, "465 leaves a 1-byte tail");
        let key = 0x1234_5678u32;
        let once = xor_descramble(&buf, key);
        // Tail byte is copied verbatim (no XOR) on a single pass.
        assert_eq!(
            *once.last().unwrap(),
            *buf.last().unwrap(),
            "trailing partial-word byte is verbatim"
        );
        // Full round-trip still restores everything.
        assert_eq!(xor_descramble(&once, key), buf);
    }

    /// The `_into` variant matches the allocating variant byte-for-byte.
    #[test]
    fn into_variant_matches_allocating() {
        let buf: Vec<u8> = (0u8..200).collect();
        let key = 0x0BAD_F00Du32;
        let owned = xor_descramble(&buf, key);
        let mut out = vec![0u8; buf.len()];
        xor_descramble_into(&buf, key, &mut out);
        assert_eq!(out, owned);
    }

    #[test]
    #[should_panic(expected = "out buffer length must equal input length")]
    fn into_variant_rejects_length_mismatch() {
        let buf = [1u8, 2, 3, 4];
        let mut out = [0u8; 3];
        xor_descramble_into(&buf, 0, &mut out);
    }

    /// Default common-mode-off path bypasses the XOR and is zero-copy.
    #[test]
    fn common_mode_off_bypasses_xor() {
        assert!(!CommonMode::default().is_on(), "default is off");
        assert!(!CommonMode::off().is_on());
        assert!(CommonMode::on().is_on());

        let buf: Vec<u8> = (0u8..100).collect();
        // Even with a non-zero key, off-mode returns the input verbatim.
        let got = descramble_packet(CommonMode::off(), &buf, 0xDEAD_BEEF);
        assert!(matches!(got, Cow::Borrowed(_)), "off-path is zero-copy");
        assert_eq!(&*got, &buf[..]);
    }

    /// On-mode runs the pass (owned) and is self-inverse.
    #[test]
    fn common_mode_on_runs_self_inverse_pass() {
        let buf: Vec<u8> = (0u8..=255).cycle().take(465).collect();
        let key = xor_key(0x6000_0000, buf.len() as u32);
        let scrambled = descramble_packet(CommonMode::on(), &buf, key);
        assert!(matches!(scrambled, Cow::Owned(_)), "on-path allocates");
        assert_ne!(&*scrambled, &buf[..], "on-path changes bytes");
        let restored = descramble_packet(CommonMode::on(), &scrambled, key);
        assert_eq!(&*restored, &buf[..], "double application restores input");
    }
}
