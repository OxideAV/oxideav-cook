//! XOR descrambler — Cook obfuscation layer (§5.1, §9.2 of the trace doc).
//!
//! Every input packet is XORed with rotations of the 32-bit constant
//! `0x37C511F2`. The rotation index is `(buf as usize) & 3` (= LSBs of
//! the buffer's *address* in the original libavcodec implementation —
//! i.e. each call works against a constant rotation chosen by the
//! pointer alignment, so the four constants cycle by 32-bit-word
//! position within the input).
//!
//! For an independent decoder we only ever consume packets that start
//! at a byte-aligned offset, so we apply the descrambler word-by-word
//! starting at offset 0 with rotation 0 (i.e. constant `0x37C511F2`).
//! Behavioural confirmation: long zero-bit subpackets descramble to
//! repeating `0x11F237C5` (= ROR16) in the on-disk bytes, see §5.1.
//! In our implementation we work in the post-load reader space, so we
//! XOR with the un-rotated constant and that matches.

/// The four byte-rotations of the XOR constant. Index 0 is the original
/// `0x37C511F2`; indices 1..=3 are ROR8, ROR16 and ROR24 respectively.
pub const KEY: [u32; 4] = [0x37C511F2, 0xF237C511, 0x11F237C5, 0xC511F237];

/// Descramble `input` into `output`, both little/byte-stream interpreted.
///
/// The cook payload is always a multiple of 4 bytes long for purposes of
/// the descrambler; trailing bytes (if any) are XORed against the lowest
/// bytes of the constant. Rotation chosen by `align` (lowest two bits of
/// the original input pointer).
pub fn descramble(input: &[u8], align: usize, output: &mut Vec<u8>) {
    let key = KEY[align & 3];
    output.clear();
    output.reserve(input.len());
    let mut i = 0;
    let chunks = input.len() / 4;
    while i < chunks {
        let off = i * 4;
        // Cook's libavcodec implementation reads each input word as a
        // 32-bit big-endian integer, XORs with `key`, and stores it back
        // big-endian. We reproduce that byte-exactly.
        let w = u32::from_be_bytes([input[off], input[off + 1], input[off + 2], input[off + 3]]);
        let x = w ^ key;
        output.extend_from_slice(&x.to_be_bytes());
        i += 1;
    }
    // Tail bytes: XOR each against the matching byte of `key` (BE byte
    // order) so the partial-word case behaves identically to the
    // four-byte case.
    let tail_off = chunks * 4;
    let key_bytes = key.to_be_bytes();
    for (j, &b) in input[tail_off..].iter().enumerate() {
        output.push(b ^ key_bytes[j]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeros_descramble_to_constant() {
        let zeros = [0u8; 16];
        let mut out = Vec::new();
        descramble(&zeros, 0, &mut out);
        // 4 BE u32s, all equal to 0x37C511F2.
        for chunk in out.chunks(4) {
            assert_eq!(chunk, &[0x37, 0xC5, 0x11, 0xF2]);
        }
    }

    #[test]
    fn descramble_is_involution() {
        let input: Vec<u8> = (0u8..32).collect();
        let mut once = Vec::new();
        descramble(&input, 0, &mut once);
        let mut twice = Vec::new();
        descramble(&once, 0, &mut twice);
        assert_eq!(twice, input);
    }

    #[test]
    fn rotation_indexes_match_ror() {
        assert_eq!(KEY[0].rotate_right(8), KEY[1]);
        assert_eq!(KEY[0].rotate_right(16), KEY[2]);
        assert_eq!(KEY[0].rotate_right(24), KEY[3]);
    }
}
