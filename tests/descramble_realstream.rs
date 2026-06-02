//! Real-stream cross-check of the per-buffer XOR descramble stage.
//!
//! Walks the bundled `tests/fixtures/FUN_RM_32.rm` RealMedia file
//! directly from the wire bytes (no library demuxer) to its audio
//! packets, then exercises
//! [`oxideav_cook::descramble_packet`] against them:
//!
//! - **Common-mode off** (the constructor default) returns the packet
//!   verbatim as a zero-copy `Cow::Borrowed`. This is the only path with
//!   real-stream PCM ground truth: the validator fed the 144 packets
//!   straight from the container with no external descramble and matched
//!   144/144 S_OK (`docs/audio/cook/validation/04-cook-stream-validation.md`
//!   §4.3 / §5).
//! - **Common-mode on** runs the word-wise XOR pass. The validator never
//!   drove the on-path on a real stream, so it has **no bit-exact ground
//!   truth**; the assertions here pin its *algebraic* properties only —
//!   self-inverse, byte-count preservation — for two arbitrary keys and
//!   on two different packets (the first packet and mid-stream packet
//!   100). Source: `spec/01` §5, `validation/04` §4.3.
//!
//! The existing wire-level cross-check (`realstream_fixture.rs`) keeps
//! its own assertions; this file only walks the same fixture to obtain
//! the validated 465-byte payloads.

use oxideav_cook::{descramble_packet, xor_descramble, xor_key, CommonMode};
use std::borrow::Cow;

const FIXTURE: &[u8] = include_bytes!("fixtures/FUN_RM_32.rm");

// Validator pins (validation/04 §2.2 / §5).
const VALIDATED_PACKETS: usize = 144;
const VALIDATED_PACKET_PAYLOAD: usize = 465;
const PACKET_HEADER_BYTES: usize = 12; // [u16 ver][u16 len][u16 stream][u32 ts][u8 grp][u8 flags]

fn be_u16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be_u32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Walk the top-level RealMedia chunk sequence and return the audio
/// `DATA` chunk body (everything after its FourCC + size).
fn data_chunk_body(file: &[u8]) -> &[u8] {
    let mut p = 0;
    while p + 10 <= file.len() {
        let fcc = [file[p], file[p + 1], file[p + 2], file[p + 3]];
        let size = be_u32(file, p + 4) as usize;
        if size < 10 || p + size > file.len() {
            break;
        }
        if &fcc == b"DATA" {
            return &file[p + 8..p + size];
        }
        p += size;
    }
    panic!("DATA chunk present");
}

/// Walk the audio packets in `DATA` and return each 465-byte payload as
/// a slice. Layout mirrors `realstream_fixture.rs`: a 2-byte chunk
/// version, an 8-byte (num_packets + next_data) header, then packets
/// `[u16 ver][u16 len][u16 stream][u32 ts][u8 grp][u8 flags][payload]`
/// with `len` the total packet size, all big-endian.
fn audio_payloads(file: &[u8]) -> Vec<&[u8]> {
    let body = data_chunk_body(file);
    let num_packets = be_u32(body, 2) as usize;
    let mut p = 10usize;
    let mut out = Vec::new();
    while p + PACKET_HEADER_BYTES <= body.len() {
        let plen = be_u16(body, p + 2) as usize;
        if plen < PACKET_HEADER_BYTES || p + plen > body.len() {
            break;
        }
        out.push(&body[p + PACKET_HEADER_BYTES..p + plen]);
        p += plen;
        if out.len() == num_packets {
            break;
        }
    }
    out
}

#[test]
fn fixture_yields_validated_packets() {
    let payloads = audio_payloads(FIXTURE);
    assert_eq!(payloads.len(), VALIDATED_PACKETS, "144 audio packets");
    for (i, pkt) in payloads.iter().enumerate() {
        assert_eq!(pkt.len(), VALIDATED_PACKET_PAYLOAD, "packet {i} payload");
    }
}

/// Common-mode-off returns the validated packet verbatim, zero-copy.
#[test]
fn off_path_is_verbatim_zero_copy() {
    let payloads = audio_payloads(FIXTURE);
    let first = payloads[0];
    assert_eq!(first.len(), VALIDATED_PACKET_PAYLOAD);

    let got = descramble_packet(CommonMode::off(), first, 0);
    assert!(
        matches!(got, Cow::Borrowed(_)),
        "off-path must be Cow::Borrowed (no allocation)"
    );
    // Borrowed slice is byte-identical to and aliases the source packet.
    assert_eq!(&*got, first);
    assert_eq!(got.as_ptr(), first.as_ptr(), "off-path aliases the input");
}

/// On-path is self-inverse on the first packet for two arbitrary keys.
#[test]
fn on_path_self_inverse_first_packet() {
    let payloads = audio_payloads(FIXTURE);
    let first = payloads[0];

    for key in [xor_key(0x6000_0000, first.len() as u32), 0xDEAD_BEEFu32] {
        let scrambled = descramble_packet(CommonMode::on(), first, key);
        assert!(matches!(scrambled, Cow::Owned(_)), "on-path allocates");
        assert_eq!(scrambled.len(), first.len(), "byte count preserved");
        assert_ne!(&*scrambled, first, "non-zero key changes bytes");
        let restored = descramble_packet(CommonMode::on(), &scrambled, key);
        assert_eq!(&*restored, first, "double application restores packet");
    }
}

/// Same algebraic check on a mid-stream packet (index 100).
#[test]
fn on_path_self_inverse_packet_100() {
    let payloads = audio_payloads(FIXTURE);
    assert!(payloads.len() > 100, "need a mid-stream packet");
    let pkt = payloads[100];
    assert_eq!(pkt.len(), VALIDATED_PACKET_PAYLOAD);

    for key in [0x0BAD_F00Du32, xor_key(0x12, 0x3456)] {
        let once = xor_descramble(pkt, key);
        assert_eq!(once.len(), pkt.len());
        assert_eq!(
            xor_descramble(&once, key),
            pkt,
            "self-inverse on packet 100"
        );
    }
}
