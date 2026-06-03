//! Real-stream cross-check of the per-`RADecode` sub-packet split.
//!
//! Walks the bundled `tests/fixtures/FUN_RM_32.rm` RealMedia file
//! directly from the wire bytes (no library demuxer) to its 144 audio
//! packets, then exercises [`oxideav_cook::SubPacketLayout`] against
//! them:
//!
//! - Each packet is exactly one `RADecode` call's input (465 bytes —
//!   `validation/04` §2.2 / §5).
//! - The layout partitions each call's input into **5 sub-packets of 93
//!   bytes** (`validation/04` §5).
//! - The whole-stream sub-packet ranges tile the 144 × 465 = 66 960-byte
//!   concatenated input with no gap or overlap.
//! - The PCM-offset accounting reproduces the validator's first-call
//!   warm-up (`8 192` bytes) and steady-state cadence (`20 480` bytes
//!   per call), summing to the pinned `2 936 832`-byte total after 144
//!   calls.
//!
//! Source: `docs/audio/cook/spec/01-cook-decoder-structure.md` §5 +
//! `docs/audio/cook/validation/04-cook-stream-validation.md` §4 / §5.

use oxideav_cook::{flavor_record, CookCookie, DecodeConfig, Descriptor, SubPacketLayout};

const FIXTURE: &[u8] = include_bytes!("fixtures/FUN_RM_32.rm");

// Validator pins (validation/04 §2 / §5).
const VALIDATED_PACKETS: usize = 144;
const VALIDATED_PACKET_PAYLOAD: usize = 465;
const VALIDATED_SUB_PACKETS_PER_CALL: u32 = 5;
const VALIDATED_SUB_PACKET_SIZE: u32 = 93;
const VALIDATED_TOTAL_PCM_BYTES: u64 = 2_936_832;
const VALIDATED_WARMUP_PCM_BYTES: u64 = 8_192;
const VALIDATED_STEADY_PCM_BYTES: u32 = 20_480;
const PACKET_HEADER_BYTES: usize = 12;

const REAL_DESCRIPTOR: Descriptor = Descriptor {
    channels_divisor: 2,
    sub_packet_size: 93,
};

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

fn real_layout() -> SubPacketLayout {
    // Cookie bytes match the validator-pinned 16-byte blob; the test
    // assembles them rather than re-walking the MDPR for brevity (the
    // realstream_fixture.rs test covers the MDPR walk + cross-check).
    let cookie_bytes = [
        0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00,
        0x04,
    ];
    let cookie = CookCookie::parse(&cookie_bytes).unwrap();
    let flavor = flavor_record(21).unwrap();
    let cfg = DecodeConfig::from_inputs(&cookie, &REAL_DESCRIPTOR, &flavor, 465).unwrap();
    SubPacketLayout::from_config(&cfg)
}

#[test]
fn fixture_yields_validated_packets() {
    let payloads = audio_payloads(FIXTURE);
    assert_eq!(payloads.len(), VALIDATED_PACKETS, "144 audio packets");
    for (i, pkt) in payloads.iter().enumerate() {
        assert_eq!(
            pkt.len(),
            VALIDATED_PACKET_PAYLOAD,
            "packet {i} is one RADecode-call input ({} bytes)",
            VALIDATED_PACKET_PAYLOAD
        );
    }
}

/// Each container packet is one `RADecode` call: the layout partitions
/// its 465-byte payload into exactly 5 × 93-byte sub-packet slots that
/// concatenate back to the input.
#[test]
fn each_packet_partitions_into_5_subpackets() {
    let payloads = audio_payloads(FIXTURE);
    let layout = real_layout();
    assert_eq!(layout.sub_packets_per_call, VALIDATED_SUB_PACKETS_PER_CALL);
    assert_eq!(layout.sub_packet_size as u32, VALIDATED_SUB_PACKET_SIZE);
    for (call_idx, pkt) in payloads.iter().enumerate() {
        let slots: Vec<_> = layout.iter_call(pkt).map(|r| r.unwrap()).collect();
        assert_eq!(
            slots.len(),
            VALIDATED_SUB_PACKETS_PER_CALL as usize,
            "packet {call_idx} has 5 sub-packets"
        );
        for (slot_idx, slot) in slots.iter().enumerate() {
            assert_eq!(
                slot.len(),
                VALIDATED_SUB_PACKET_SIZE as usize,
                "packet {call_idx} slot {slot_idx} is 93 bytes"
            );
        }
        // Concatenation equals the input packet payload.
        let recombined: Vec<u8> = slots.into_iter().flatten().copied().collect();
        assert_eq!(&recombined[..], *pkt, "packet {call_idx} round-trips");
    }
}

/// Whole-stream sub-packet byte ranges tile the concatenated 144 × 465-
/// byte input with no gap or overlap.
#[test]
fn whole_stream_subpacket_ranges_tile_input() {
    let layout = real_layout();
    let mut covered = 0u64;
    for call in 0..VALIDATED_PACKETS as u32 {
        for slot in 0..layout.sub_packets_per_call {
            let r = layout.call_byte_range(call, slot).unwrap();
            assert_eq!(
                r.start, covered,
                "(call {call}, slot {slot}) starts at the previous end"
            );
            covered = r.end;
        }
    }
    // 144 × 465 = 66 960 bytes (= the validator's audio payload total).
    assert_eq!(
        covered,
        VALIDATED_PACKETS as u64 * VALIDATED_PACKET_PAYLOAD as u64
    );
}

/// PCM offset accounting reproduces the validator's first-call warm-up
/// and steady-state cadence across all 144 calls.
#[test]
fn pcm_offsets_match_validator_across_all_144_calls() {
    let layout = real_layout();
    // Call 0 starts at offset 0 and emits 8 192 bytes (warm-up).
    assert_eq!(layout.pcm_offset_for_call(0), 0);
    assert_eq!(layout.warmup_pcm_bytes as u64, VALIDATED_WARMUP_PCM_BYTES);
    // Call 1 starts at the end of the warm-up.
    assert_eq!(layout.pcm_offset_for_call(1), VALIDATED_WARMUP_PCM_BYTES);
    // Every subsequent call advances by the steady-state budget.
    for k in 1..VALIDATED_PACKETS as u32 {
        let next = layout.pcm_offset_for_call(k + 1);
        let prev = layout.pcm_offset_for_call(k);
        assert_eq!(
            next - prev,
            VALIDATED_STEADY_PCM_BYTES as u64,
            "call {k} steady-state cadence"
        );
    }
    // One past the last call equals the validator's total PCM count.
    assert_eq!(
        layout.pcm_offset_for_call(VALIDATED_PACKETS as u32),
        VALIDATED_TOTAL_PCM_BYTES
    );
    assert_eq!(
        layout.total_pcm_bytes(VALIDATED_PACKETS as u32),
        VALIDATED_TOTAL_PCM_BYTES
    );
}

/// A truncated packet (one byte short) produces a typed mismatch error
/// on the first iteration and then terminates.
#[test]
fn truncated_packet_produces_typed_mismatch() {
    let payloads = audio_payloads(FIXTURE);
    let truncated = &payloads[0][..payloads[0].len() - 1];
    let layout = real_layout();
    let mut it = layout.iter_call(truncated);
    let first = it.next().expect("first iter yields the error");
    assert!(
        matches!(
            first,
            Err(oxideav_cook::Error::SubPacketInputLengthMismatch {
                got: 464,
                expected: 465
            })
        ),
        "got {first:?}"
    );
    assert!(it.next().is_none(), "iterator terminates after error");
}
