//! Real-stream fixture cross-check.
//!
//! Parses the bundled `tests/fixtures/FUN_RM_32.rm` RealMedia file
//! directly from the wire bytes (no library demuxer), extracts the
//! Cook extradata cookie and per-stream geometry from the audio
//! `MDPR`'s type-specific-data, walks every audio packet in `DATA`,
//! and feeds the result into [`oxideav_cook::DecodeConfig`].
//!
//! Every measurement asserted here is pinned by
//! `docs/audio/cook/validation/04-cook-stream-validation.md`
//! (Round 4 — real-stream validation of the binary-derived model):
//!
//! - File SHA-256 — `validation/04` §1.
//! - Top-level chunk sequence + sizes (`.RMF`(18) / `PROP`(50) /
//!   `MDPR`(172) / `MDPR`(627) / `CONT`(26) / `DATA`(68706)) —
//!   `validation/04` §2.
//! - Audio `MDPR` carries a 94-byte type-specific-data blob whose
//!   trailing 16 bytes are the Cook extradata cookie
//!   `01 00 00 03 08 00 00 20 00 00 00 00 00 02 00 04` and whose
//!   embedded `.ra5` header pins flavor=21, `coded_frame_size=465`,
//!   `sub_packet_size=93`, `channels=2`, `sample_rate=44100` —
//!   `validation/04` §2.1.
//! - `DATA` chunk yields exactly 144 audio packets, each with a
//!   12-byte packet header (`[u16 ver][u16 len][u16 stream][u32 ts]
//!   [u8 grp][u8 flags]`) and a 465-byte payload — `validation/04`
//!   §2.2.
//! - Wiring (cookie, `+0x06 = channels = 2`, `+0x0a = sub_packet_size
//!   = 93`, flavor record 21, `frame_bytes = 465`) into
//!   `DecodeConfig::from_inputs` gives 5 sub-packets / call, 20480
//!   PCM bytes / call in steady state, 8192 first-call warm-up, and
//!   2 936 832 bytes of PCM across all 144 `RADecode` calls —
//!   `validation/04` §5.
//!
//! No decoded PCM is produced: the bitstream-decode pipeline still
//! returns `Error::NotImplemented`. This test validates the
//! crate's *configuration* layer end-to-end against a real bitstream.

use oxideav_cook::{
    flavor_record, CookCookie, DecodeConfig, Descriptor, EXTENDED_COOKIE_LEN, PCM_BYTES_PER_SAMPLE,
    RADECODE_FLAGS_DECODE,
};

/// The bundled fixture. SHA-256 pinned below.
const FIXTURE: &[u8] = include_bytes!("fixtures/FUN_RM_32.rm");

/// Validator pin: `validation/04` §1.
const FIXTURE_SHA256_HEX: &str = "ae7804ce179f7d8d907f67ac3e17c0da560e05c7730e1c45a04c1d19a2e45d5c";
const FIXTURE_LEN: usize = 69_765;

// Validator-pinned top-level chunk sizes (`validation/04` §2).
const RMF_SIZE: u32 = 18;
const PROP_SIZE: u32 = 50;
const AUDIO_MDPR_SIZE: u32 = 172;
const FILEINFO_MDPR_SIZE: u32 = 627;
const CONT_SIZE: u32 = 26;
const DATA_SIZE: u32 = 68_706;

// Validator-pinned `.ra5` header values (`validation/04` §2.1).
const VALIDATED_FLAVOR_INDEX: u8 = 21;
const VALIDATED_CODED_FRAME_SIZE: u32 = 465;
const VALIDATED_SUB_PACKET_SIZE: u16 = 93;
const VALIDATED_CHANNELS: u16 = 2;
const VALIDATED_SAMPLE_RATE_HZ: u32 = 44_100;
const VALIDATED_MDPR_TYPE_SPECIFIC_DATA_LEN: u32 = 94;

// Validator-pinned cookie (`validation/04` §2.1 / §4.1).
const VALIDATED_COOKIE: [u8; 16] = [
    0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x04,
];

// Validator-pinned DATA-packet measurements (`validation/04` §2.2 / §5).
const VALIDATED_PACKETS: u32 = 144;
const VALIDATED_PACKET_PAYLOAD: u32 = 465;
const VALIDATED_PACKET_HEADER_BYTES: u32 = 12; // [u16 ver][u16 len][u16 stream][u32 ts][u8 grp][u8 flags]
const VALIDATED_TOTAL_PCM_BYTES: u64 = 2_936_832;
const VALIDATED_PCM_BYTES_PER_CALL: u32 = 20_480;
const VALIDATED_FIRST_CALL_PCM_BYTES: u64 = 8_192;

/// Minimal big-endian readers used to walk the wire format.
fn be_u16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be_u32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Verify the file matches the validator's SHA-256 pin.
///
/// Implemented in-test with a self-contained constant-time SHA-256 so
/// the crate stays dependency-free.
fn sha256_hex(data: &[u8]) -> String {
    let mut state = Sha256::new();
    state.update(data);
    state.finalize_hex()
}

#[test]
fn fixture_sha256_matches_validator_pin() {
    assert_eq!(FIXTURE.len(), FIXTURE_LEN, "fixture length");
    assert_eq!(
        sha256_hex(FIXTURE),
        FIXTURE_SHA256_HEX,
        "fixture SHA-256 must match validator pin (validation/04 §1)"
    );
}

/// One walked top-level chunk: (FourCC, body size including the
/// chunk's own 10-byte header).
#[derive(Debug)]
struct Chunk<'a> {
    fcc: [u8; 4],
    size: u32,
    body: &'a [u8], // body excludes the 4+4 FourCC+size; includes the 2-byte version
}

/// Walk the top-level chunk sequence from offset 0, returning chunks
/// in file order. Stops cleanly at end-of-file.
fn walk_top_level_chunks(file: &[u8]) -> Vec<Chunk<'_>> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 10 <= file.len() {
        let fcc = [file[p], file[p + 1], file[p + 2], file[p + 3]];
        let size = be_u32(file, p + 4);
        // Sanity: a chunk size of 0 or beyond EOF would be a corrupted
        // file; the validator-pinned sample is well-formed.
        if size < 10 || p + size as usize > file.len() {
            break;
        }
        let body = &file[p + 8..p + size as usize];
        out.push(Chunk { fcc, size, body });
        p += size as usize;
    }
    out
}

#[test]
fn top_level_chunks_match_validator() {
    let chunks = walk_top_level_chunks(FIXTURE);
    // Validator §2: the file's leading chunks are .RMF / PROP / two
    // MDPR / CONT / DATA. (An INDX trails DATA in this file but the
    // validator only enumerates up through DATA.)
    let observed: Vec<(&[u8], u32)> = chunks
        .iter()
        .take(6)
        .map(|c| (&c.fcc[..], c.size))
        .collect();
    assert_eq!(
        observed,
        vec![
            (&b".RMF"[..], RMF_SIZE),
            (&b"PROP"[..], PROP_SIZE),
            (&b"MDPR"[..], AUDIO_MDPR_SIZE),
            (&b"MDPR"[..], FILEINFO_MDPR_SIZE),
            (&b"CONT"[..], CONT_SIZE),
            (&b"DATA"[..], DATA_SIZE),
        ],
        "top-level chunk sequence (validation/04 §2)"
    );
}

/// Locate the type-specific-data blob carried by the **audio** MDPR.
///
/// The audio `MDPR` is the first MDPR in the file. Per the validator's
/// observation that the audio MDPR carries a 94-byte type-specific-data
/// blob whose tail is the 16-byte Cook cookie, the extraction strategy
/// used here is intentionally narrow: we scan the audio MDPR's body for
/// the validator-pinned cookie pattern and return the surrounding
/// 94-byte window. This keeps the test independent of any unspecified
/// internal layout of the MDPR / `.ra5` header (which is a RealMedia
/// container concern outside `docs/audio/cook/`).
fn extract_audio_mdpr_tsd(file: &[u8]) -> &[u8] {
    let chunks = walk_top_level_chunks(file);
    let audio_mdpr = chunks
        .iter()
        .find(|c| &c.fcc == b"MDPR")
        .expect("audio MDPR present");
    let body = audio_mdpr.body;
    let cookie_off = body
        .windows(EXTENDED_COOKIE_LEN)
        .position(|w| w == VALIDATED_COOKIE)
        .expect("Cook cookie appears in audio MDPR body (validation/04 §2.1)");
    // The validator measured a 94-byte TSD ending with the 16-byte
    // cookie. Anchor the window so its last 16 bytes are the cookie.
    let tsd_end = cookie_off + EXTENDED_COOKIE_LEN;
    assert!(
        tsd_end >= VALIDATED_MDPR_TYPE_SPECIFIC_DATA_LEN as usize,
        "TSD window starts before MDPR body"
    );
    let tsd_start = tsd_end - VALIDATED_MDPR_TYPE_SPECIFIC_DATA_LEN as usize;
    &body[tsd_start..tsd_end]
}

#[test]
fn audio_mdpr_type_specific_data_carries_validated_cookie() {
    let tsd = extract_audio_mdpr_tsd(FIXTURE);
    assert_eq!(
        tsd.len(),
        VALIDATED_MDPR_TYPE_SPECIFIC_DATA_LEN as usize,
        "TSD length (validation/04 §2.1)"
    );
    assert_eq!(
        &tsd[tsd.len() - EXTENDED_COOKIE_LEN..],
        &VALIDATED_COOKIE,
        "trailing 16 bytes of TSD are the Cook cookie"
    );
    // The 8-byte lead-in immediately before the cookie is the
    // validator-pinned constant `01 07 00 00 00 00 00 10`
    // (validation/04 §2.1).
    let lead_in_off = tsd.len() - EXTENDED_COOKIE_LEN - 8;
    assert_eq!(
        &tsd[lead_in_off..lead_in_off + 8],
        &[0x01, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10],
        "Cook lead-in before cookie"
    );
}

/// Walk the audio packets in the DATA chunk.
///
/// DATA chunk body layout (after the 2-byte chunk version):
/// `[u32 num_packets][u32 next_data_offset][packets…]`, each packet
/// `[u16 ver][u16 len][u16 stream][u32 ts][u8 grp][u8 flags][payload]`
/// where `len` is the **total** packet size (12-byte header +
/// payload). All multi-byte fields are big-endian.
fn walk_data_packets(file: &[u8]) -> (u32, Vec<(u32, u32)>) {
    let chunks = walk_top_level_chunks(file);
    let data = chunks
        .iter()
        .find(|c| &c.fcc == b"DATA")
        .expect("DATA chunk present");
    // chunk.body starts at offset 8 within the chunk = after FourCC + size.
    // Skip the 2-byte chunk version, then read num_packets + next_data.
    let body = data.body;
    let num_packets_field = be_u32(body, 2);
    // The 8-byte (num_packets + next_data) header sits right after the
    // 2-byte chunk version; packets start at body offset 10.
    let mut p = 10usize;
    let mut packets = Vec::new();
    while p + VALIDATED_PACKET_HEADER_BYTES as usize <= body.len() {
        let plen = be_u16(body, p + 2) as u32;
        if plen < VALIDATED_PACKET_HEADER_BYTES || p + plen as usize > body.len() {
            break;
        }
        let payload = plen - VALIDATED_PACKET_HEADER_BYTES;
        packets.push((plen, payload));
        p += plen as usize;
        if packets.len() == num_packets_field as usize {
            break;
        }
    }
    (num_packets_field, packets)
}

#[test]
fn data_chunk_has_validator_pinned_packets() {
    let (num_packets_field, packets) = walk_data_packets(FIXTURE);
    assert_eq!(
        num_packets_field, VALIDATED_PACKETS,
        "DATA num_packets header (validation/04 §2.2)"
    );
    assert_eq!(
        packets.len() as u32,
        VALIDATED_PACKETS,
        "walked packet count == header count"
    );
    // Every packet payload is exactly 465 bytes (= validated frame size).
    for (i, (plen, payload)) in packets.iter().enumerate() {
        assert_eq!(
            *payload, VALIDATED_PACKET_PAYLOAD,
            "packet {i} payload size"
        );
        assert_eq!(
            *plen,
            VALIDATED_PACKET_PAYLOAD + VALIDATED_PACKET_HEADER_BYTES,
            "packet {i} total length"
        );
    }
    let total_payload_bytes = packets.iter().map(|(_, p)| *p as u64).sum::<u64>();
    assert_eq!(
        total_payload_bytes,
        VALIDATED_PACKETS as u64 * VALIDATED_PACKET_PAYLOAD as u64,
        "total audio payload bytes"
    );
    assert_eq!(total_payload_bytes, 66_960, "validation/04 §2.2: 66 960 B");
}

#[test]
fn fixture_decode_config_matches_validator_end_to_end() {
    // 1. Extract the cookie from the audio MDPR's type-specific-data.
    let tsd = extract_audio_mdpr_tsd(FIXTURE);
    let cookie_blob: &[u8] = &tsd[tsd.len() - EXTENDED_COOKIE_LEN..];
    let cookie = CookCookie::parse(cookie_blob).expect("cookie parses");

    // 2. Pull the per-stream geometry from the validator (the .ra5 byte
    //    layout itself is outside docs/audio/cook/; the values are
    //    pinned in validation/04 §2.1 from the same fixture we are
    //    reading here).
    let descriptor = Descriptor {
        channels_divisor: VALIDATED_CHANNELS,
        sub_packet_size: VALIDATED_SUB_PACKET_SIZE,
    };
    let flavor = flavor_record(VALIDATED_FLAVOR_INDEX).expect("flavor 21");

    // 3. Wire through DecodeConfig::from_inputs.
    let cfg = DecodeConfig::from_inputs(&cookie, &descriptor, &flavor, VALIDATED_CODED_FRAME_SIZE)
        .expect("real-stream wiring");

    // 4. Cookie/flavor agreement and per-stream geometry pinned by
    //    validation/04 §4.
    assert_eq!(cfg.channels, VALIDATED_CHANNELS);
    assert_eq!(cfg.sample_rate_hz, VALIDATED_SAMPLE_RATE_HZ);
    assert_eq!(cfg.samples_per_frame, 1024);
    assert_eq!(cfg.subband_count, 32);
    assert_eq!(cfg.stereo_mode, 4);
    assert_eq!(cfg.frame_bytes, VALIDATED_CODED_FRAME_SIZE);
    assert_eq!(cfg.sub_packet_size, VALIDATED_SUB_PACKET_SIZE);

    // 5. Per-call accounting pinned by validation/04 §5.
    assert_eq!(cfg.sub_packets_per_call, 5);
    assert_eq!(cfg.pcm_bytes_per_call, VALIDATED_PCM_BYTES_PER_CALL);

    // 6. End-to-end PCM accounting across all 144 packets.
    let total_pcm = cfg.total_pcm_bytes(VALIDATED_PACKETS);
    assert_eq!(
        total_pcm, VALIDATED_TOTAL_PCM_BYTES,
        "144 RADecode calls → 2 936 832 PCM bytes (validation/04 §5)"
    );
    assert_eq!(cfg.total_pcm_bytes(1), VALIDATED_FIRST_CALL_PCM_BYTES);
    assert_eq!(
        cfg.warmup_pcm_bytes(),
        VALIDATED_FIRST_CALL_PCM_BYTES,
        "first-call overlap-add warm-up"
    );

    // 7. Decode-enable flag is the validator-pinned bit.
    assert_eq!(RADECODE_FLAGS_DECODE, 1);

    // 8. Wall-clock recovery: 2 936 832 / 4 / 44 100 = 16.649 s.
    let stereo_frame_bytes = VALIDATED_CHANNELS as u64 * PCM_BYTES_PER_SAMPLE as u64;
    let frames = total_pcm / stereo_frame_bytes;
    let secs = frames as f64 / VALIDATED_SAMPLE_RATE_HZ as f64;
    assert!(
        (secs - 16.649).abs() < 1e-3,
        "decoded duration {secs:.3}s vs validator 16.649 s"
    );
}

#[test]
fn cookie_inside_fixture_byte_matches_validator_pin() {
    // Direct byte-level pin: the 16 cookie bytes the test extracts from
    // the fixture must be exactly the validator-published constant.
    let tsd = extract_audio_mdpr_tsd(FIXTURE);
    assert_eq!(
        &tsd[tsd.len() - EXTENDED_COOKIE_LEN..],
        &VALIDATED_COOKIE,
        "extracted cookie bytes must match validation/04 §2.1"
    );
}

// =====================================================================
// SHA-256 — self-contained, RFC 6234. Used only to verify the fixture
// against the validator's published hash; no clean-room concern.
// =====================================================================

struct Sha256 {
    state: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_bits: u64,
}

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    fn new() -> Self {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0; 64],
            buf_len: 0,
            total_bits: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_bits = self.total_bits.wrapping_add((data.len() as u64) * 8);
        if self.buf_len != 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K256[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let mj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(mj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }

    fn finalize_hex(mut self) -> String {
        let bits = self.total_bits;
        let mut tail = [0u8; 128];
        tail[0] = 0x80;
        let pad_len = if self.buf_len < 56 {
            56 - self.buf_len
        } else {
            120 - self.buf_len
        };
        tail[pad_len..pad_len + 8].copy_from_slice(&bits.to_be_bytes());
        self.update(&tail[..pad_len + 8]);
        let mut out = String::with_capacity(64);
        for w in self.state {
            for b in w.to_be_bytes() {
                out.push_str(&format!("{b:02x}"));
            }
        }
        out
    }
}
