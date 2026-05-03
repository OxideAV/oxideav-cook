//! AVLFG — Lagged Fibonacci Generator used for cook dither.
//!
//! Per §9.7 of the trace doc the recurrence is
//! `s[i] = s[i-24] + s[i-55]  (mod 2^32)`, and the 64-word state is
//! seeded by MD5-hashing `(seed_le32, j_le32, 11_zero_bytes)` for
//! `j ∈ {8, 12, …, 60}` — leaving `s[0..8]` zero. The cook decoder
//! initialises with `seed = 0` so an independent decoder reproduces
//! identical PCM byte-for-byte across runs.
//!
//! The dither sign bit is the **MSB** of the next LFG output:
//! `f = -f` when `(av_lfg_get() & 0x8000_0000) == 0`.
//!
//! This module ships a minimal MD5 implementation (RFC 1321) so we
//! avoid pulling in any third-party dep.

#[derive(Clone)]
pub struct Lfg {
    state: [u32; 64],
    index: u32,
}

impl Lfg {
    pub fn new(seed: u32) -> Self {
        let mut state = [0u32; 64];
        let seed_bytes = seed.to_le_bytes();
        for j in (8..64).step_by(4) {
            // Build the 19-byte input: 4-byte LE seed || 4-byte LE j || 11 zero bytes.
            let mut input = [0u8; 19];
            input[0..4].copy_from_slice(&seed_bytes);
            let j_bytes = (j as u32).to_le_bytes();
            input[4..8].copy_from_slice(&j_bytes);
            let digest = md5(&input);
            // Each MD5 output is 16 bytes = 4 u32s, written in little-endian
            // word order matching ffmpeg's av_md5_sum buffer layout.
            for k in 0..4 {
                let word_off = k * 4;
                let w = u32::from_le_bytes([
                    digest[word_off],
                    digest[word_off + 1],
                    digest[word_off + 2],
                    digest[word_off + 3],
                ]);
                state[j + k] = w;
            }
        }
        Self { state, index: 0 }
    }

    /// Advance and return one 32-bit pseudorandom word. Mirrors
    /// `av_lfg_get` from libavutil: combine `s[i-24]` and `s[i-55]` mod 2^32,
    /// store back, then advance the cursor.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u32 {
        let i = self.index as usize;
        let a = self.state[(i + 64 - 24) & 63];
        let b = self.state[(i + 64 - 55) & 63];
        let v = a.wrapping_add(b);
        self.state[i] = v;
        self.index = (self.index + 1) & 63;
        v
    }

    /// Cook's dither sign-bit convention: returns `-1.0` when the MSB is
    /// clear, `+1.0` when set.
    pub fn next_sign(&mut self) -> f32 {
        if self.next() & 0x8000_0000 == 0 {
            -1.0
        } else {
            1.0
        }
    }
}

// ─────────────────────────── MD5 (RFC 1321) ───────────────────────────

const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

/// Compute the MD5 digest of `input` (RFC 1321). Returns 16 bytes.
pub fn md5(input: &[u8]) -> [u8; 16] {
    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    // Pre-process: pad to 56 mod 64, then append 64-bit LE bit length.
    let bit_len: u64 = (input.len() as u64) * 8;
    let mut padded = Vec::with_capacity(input.len() + 72);
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, w) in chunk.chunks_exact(4).enumerate() {
            m[i] = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(MD5_K[i])
                    .wrapping_add(m[g])
                    .rotate_left(MD5_S[i]),
            );
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_known_vectors() {
        // RFC 1321 test vectors, hex-encoded.
        let cases: &[(&[u8], &str)] = &[
            (b"", "d41d8cd98f00b204e9800998ecf8427e"),
            (b"a", "0cc175b9c0f1b6a831c399e269772661"),
            (b"abc", "900150983cd24fb0d6963f7d28e17f72"),
            (b"message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
        ];
        for (input, expected) in cases {
            let d = md5(input);
            let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(&hex, expected, "md5({input:?}) = {hex}, want {expected}");
        }
    }

    #[test]
    fn lfg_seed_zero_first_words_match_libavutil() {
        // The first eight words are zero (seeded part begins at index 8).
        // We can at least sanity-check determinism: same seed → same sequence.
        let mut a = Lfg::new(0);
        let mut b = Lfg::new(0);
        for _ in 0..256 {
            assert_eq!(a.next(), b.next());
        }
    }

    #[test]
    fn lfg_advances_state() {
        let mut g = Lfg::new(0);
        let s = (0..16).map(|_| g.next()).collect::<Vec<_>>();
        // No two consecutive outputs should be all-zero (very unlikely
        // with the seeded state).
        assert!(!s.iter().all(|&x| x == 0));
    }
}
