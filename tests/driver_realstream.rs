//! Real-stream cross-check of the per-call [`Driver`] orchestrator.
//!
//! Walks the bundled `tests/fixtures/FUN_RM_32.rm` to its 144
//! validator-pinned 465-byte audio payloads and drives them through
//! [`oxideav_cook::Driver`]:
//!
//! - [`Driver::prepare_call`] on every packet must succeed, return the
//!   packet verbatim (default common-mode-off, the validated path —
//!   `docs/audio/cook/validation/04-cook-stream-validation.md` §4.3 /
//!   §5), and partition it into 5 × 93-byte sub-packet slots that
//!   recombine to the input.
//! - [`Driver::advance_after_decode`] walked across the 144-call
//!   sequence with the validator-pinned per-call PCM budgets must
//!   reproduce the validator's `2 936 832`-byte total exactly.
//! - [`Driver::decode_call`] with both buffer sizes wired correctly
//!   drives the frame-body walk (§1.1 gain count + §2.1 subband
//!   geometry) to the documented §3.2 BSS codebook blocker
//!   (the §0.2 envelope/coupling tree gaps)
//!   without advancing the cursor.
//!
//! This file deliberately shares the fixture-walking helpers with
//! `realstream_fixture.rs` rather than depending on it: every test
//! module is built standalone in `cargo test`.

#![allow(dead_code)]

use oxideav_cook::{
    flavor_record, CommonMode, CookCookie, DecodeConfig, DecodeGate, Descriptor, Driver, Error,
    EXTENDED_COOKIE_LEN, RADECODE_FLAGS_DECODE,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/FUN_RM_32.rm");

const VALIDATED_PACKETS: u32 = 144;
const VALIDATED_PACKET_PAYLOAD: u32 = 465;
const VALIDATED_PACKET_HEADER_BYTES: u32 = 12;
const VALIDATED_SUB_PACKET_SIZE: u16 = 93;
const VALIDATED_CHANNELS: u16 = 2;
const VALIDATED_FLAVOR_INDEX: u8 = 21;
const VALIDATED_CODED_FRAME_SIZE: u32 = 465;
const VALIDATED_MDPR_TYPE_SPECIFIC_DATA_LEN: u32 = 94;
const VALIDATED_TOTAL_PCM_BYTES: u64 = 2_936_832;
const VALIDATED_PCM_BYTES_PER_CALL: u32 = 20_480;
const VALIDATED_FIRST_CALL_PCM_BYTES: u64 = 8_192;

const VALIDATED_COOKIE: [u8; 16] = [
    0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x04,
];

fn be_u16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be_u32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

#[derive(Debug)]
struct Chunk<'a> {
    fcc: [u8; 4],
    size: u32,
    body: &'a [u8],
}

fn walk_top_level_chunks(file: &[u8]) -> Vec<Chunk<'_>> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 10 <= file.len() {
        let fcc = [file[p], file[p + 1], file[p + 2], file[p + 3]];
        let size = be_u32(file, p + 4);
        if size < 10 || p + size as usize > file.len() {
            break;
        }
        let body = &file[p + 8..p + size as usize];
        out.push(Chunk { fcc, size, body });
        p += size as usize;
    }
    out
}

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
        .expect("Cook cookie inside audio MDPR");
    let tsd_end = cookie_off + EXTENDED_COOKIE_LEN;
    let tsd_start = tsd_end - VALIDATED_MDPR_TYPE_SPECIFIC_DATA_LEN as usize;
    &body[tsd_start..tsd_end]
}

/// Returns the bundled fixture's 144 audio packet payloads (each 465 B).
fn collect_packet_payloads(file: &[u8]) -> Vec<&[u8]> {
    let chunks = walk_top_level_chunks(file);
    let data = chunks
        .iter()
        .find(|c| &c.fcc == b"DATA")
        .expect("DATA chunk present");
    let body = data.body;
    let num_packets = be_u32(body, 2);
    let mut p = 10usize;
    let mut payloads = Vec::with_capacity(num_packets as usize);
    while p + VALIDATED_PACKET_HEADER_BYTES as usize <= body.len() {
        let plen = be_u16(body, p + 2) as usize;
        if plen < VALIDATED_PACKET_HEADER_BYTES as usize || p + plen > body.len() {
            break;
        }
        let payload = &body[p + VALIDATED_PACKET_HEADER_BYTES as usize..p + plen];
        payloads.push(payload);
        p += plen;
        if payloads.len() == num_packets as usize {
            break;
        }
    }
    assert_eq!(payloads.len(), VALIDATED_PACKETS as usize);
    payloads
}

fn real_driver() -> Driver {
    let tsd = extract_audio_mdpr_tsd(FIXTURE);
    let cookie_blob: &[u8] = &tsd[tsd.len() - EXTENDED_COOKIE_LEN..];
    let cookie = CookCookie::parse(cookie_blob).expect("cookie parses");
    let descriptor = Descriptor {
        channels_divisor: VALIDATED_CHANNELS,
        sub_packet_size: VALIDATED_SUB_PACKET_SIZE,
    };
    let flavor = flavor_record(VALIDATED_FLAVOR_INDEX).expect("flavor 21");
    let cfg = DecodeConfig::from_inputs(&cookie, &descriptor, &flavor, VALIDATED_CODED_FRAME_SIZE)
        .expect("real-stream wiring");
    Driver::new(cfg)
}

#[test]
fn prepare_call_passes_every_real_packet_verbatim() {
    let driver = real_driver();
    let payloads = collect_packet_payloads(FIXTURE);
    for (i, payload) in payloads.iter().enumerate() {
        let prepared = driver
            .prepare_call(payload, 0)
            .unwrap_or_else(|e| panic!("prepare_call packet {i}: {e:?}"));
        // Default common-mode-off: descrambled bytes equal the input
        // verbatim (zero-copy borrow).
        assert_eq!(
            prepared.descrambled(),
            *payload,
            "packet {i} descrambled bytes vs input"
        );
        // Sub-packet split: 5 × 93 bytes covering the full 465 input.
        let slots: Vec<_> = prepared
            .iter_sub_packets()
            .map(|r| r.expect("slot slice ok").to_vec())
            .collect();
        assert_eq!(slots.len(), 5, "packet {i} slot count");
        for (k, slot) in slots.iter().enumerate() {
            assert_eq!(slot.len(), 93, "packet {i} slot {k} byte count");
        }
        let recombined: Vec<u8> = slots.into_iter().flatten().collect();
        assert_eq!(
            recombined, *payload,
            "packet {i} slot concatenation matches input"
        );
    }
}

#[test]
fn advance_after_decode_walks_full_144_call_cadence() {
    // Walking the validator-pinned per-call PCM budget across the 144
    // packets must reproduce the 2 936 832-byte total at call 144.
    let mut driver = real_driver();
    let payloads = collect_packet_payloads(FIXTURE);
    assert_eq!(payloads.len(), VALIDATED_PACKETS as usize);

    for (i, _payload) in payloads.iter().enumerate() {
        let want_pcm = if i == 0 {
            VALIDATED_FIRST_CALL_PCM_BYTES as usize
        } else {
            VALIDATED_PCM_BYTES_PER_CALL as usize
        };
        // The PCM byte range advancing across calls must match the
        // validator's first-call warm-up + steady-state cadence.
        let range = driver.next_call_pcm_byte_range();
        if i == 0 {
            assert_eq!(range, 0u64..VALIDATED_FIRST_CALL_PCM_BYTES);
        } else {
            let expected_start = VALIDATED_FIRST_CALL_PCM_BYTES
                + (i as u64 - 1) * VALIDATED_PCM_BYTES_PER_CALL as u64;
            let expected_end = expected_start + VALIDATED_PCM_BYTES_PER_CALL as u64;
            assert_eq!(range, expected_start..expected_end, "call {i} pcm range");
        }
        driver
            .advance_after_decode(want_pcm)
            .unwrap_or_else(|e| panic!("advance call {i}: {e:?}"));
    }

    assert_eq!(driver.calls_completed(), VALIDATED_PACKETS);
    assert_eq!(driver.total_pcm_emitted(), VALIDATED_TOTAL_PCM_BYTES);
}

#[test]
fn decode_call_on_first_real_packet_surfaces_bss_blocker() {
    // With both buffer sizes wired correctly to the validator's pinned
    // per-call budgets, decode_call drives the real first packet through
    // the §0.2 frame walk: its head parses (sub-packet flag, coupling
    // control, envelope seed), and the walk stops at the field-5
    // envelope-tree gap (the 31-entry envelope VLC family is not among
    // the staged tables) — the cursor does NOT advance (no partial
    // state published on the GAP signal). The real packet 0 opens with
    // the fixed-width coupling branch, so the walk gets past field 3.
    let mut driver = real_driver();
    let payloads = collect_packet_payloads(FIXTURE);
    let packet = payloads[0];
    let mut out = vec![0u8; VALIDATED_FIRST_CALL_PCM_BYTES as usize];

    let err = driver.decode_call(packet, &mut out, 0).unwrap_err();
    assert!(
        matches!(
            err,
            Error::EnvelopeValueTreeUnavailable | Error::CouplingIndexTreeUnavailable
        ),
        "expected a §0.2 tree gap, got {err:?}"
    );
    assert_eq!(driver.calls_completed(), 0);
    assert_eq!(driver.total_pcm_emitted(), 0);
}

#[test]
fn decode_call_rejects_wrong_input_before_backend() {
    // A length error must surface as the typed mismatch — never as the
    // §3.2 BSS-blocker signal, which is reserved for the frame-body walk.
    let mut driver = real_driver();
    let bad_packet = vec![0u8; 464];
    let mut out = vec![0u8; VALIDATED_FIRST_CALL_PCM_BYTES as usize];
    let err = driver.decode_call(&bad_packet, &mut out, 0).unwrap_err();
    assert_eq!(
        err,
        Error::CallInputLengthMismatch {
            got: 464,
            expected: 465
        }
    );
    // Distinct from the backend gap signals — invariants confirmed.
    assert_ne!(err, Error::EnvelopeValueTreeUnavailable);
    assert_eq!(driver.calls_completed(), 0);
}

#[test]
fn observe_gate_walks_all_144_real_packets_emitting_zeroed_pcm() {
    // validation/04 §4.3: with `flags` bit 0 = 0 the backend emits
    // zeroed overlap-add output independent of the input. Feed all
    // 144 real packets through decode_call_with_flags(flags = 0) —
    // every call must complete, every output byte must be zero, and
    // the cadence must reproduce the validator's pinned totals
    // (8 192-byte warm-up, 20 480 bytes/call steady state,
    // 2 936 832 bytes at call 144).
    let mut driver = real_driver();
    let payloads = collect_packet_payloads(FIXTURE);
    assert_eq!(payloads.len(), VALIDATED_PACKETS as usize);

    for (i, payload) in payloads.iter().enumerate() {
        let want_pcm = if i == 0 {
            VALIDATED_FIRST_CALL_PCM_BYTES as usize
        } else {
            VALIDATED_PCM_BYTES_PER_CALL as usize
        };
        assert_eq!(driver.next_call_pcm_bytes() as usize, want_pcm);
        // Poison the output buffer so the zero-fill is observable.
        let mut out = vec![0xA5u8; want_pcm];
        driver
            .decode_call_with_flags(payload, &mut out, 0, 0)
            .unwrap_or_else(|e| panic!("observe-gate call {i}: {e:?}"));
        assert!(
            out.iter().all(|&b| b == 0),
            "call {i}: observe-gate output must be zeroed PCM"
        );
    }

    assert_eq!(driver.calls_completed(), VALIDATED_PACKETS);
    assert_eq!(driver.total_pcm_emitted(), VALIDATED_TOTAL_PCM_BYTES);
}

#[test]
fn observe_gate_output_matches_for_real_and_all_ff_input() {
    // The validator's exact §4.3 verification, reproduced: an
    // all-0xFF input gives the same zero output as the real packet
    // on the observe gate.
    let mut d_real = real_driver();
    let mut d_ff = real_driver();
    let payload = collect_packet_payloads(FIXTURE)[0];
    let all_ff = vec![0xFFu8; payload.len()];
    let mut out_real = vec![0x33u8; VALIDATED_FIRST_CALL_PCM_BYTES as usize];
    let mut out_ff = vec![0xCCu8; VALIDATED_FIRST_CALL_PCM_BYTES as usize];
    d_real
        .decode_call_with_flags(payload, &mut out_real, 0, 0)
        .unwrap();
    d_ff.decode_call_with_flags(&all_ff, &mut out_ff, 0, 0)
        .unwrap();
    assert_eq!(out_real, out_ff, "observe output is input-independent");
}

#[test]
fn decode_gate_constant_maps_to_decode_and_reaches_tree_gap() {
    // RADECODE_FLAGS_DECODE (= 1) maps to the real-decode gate
    // ((~1) & 1 = 0 forwarded to the backend), which drives the §0.2
    // frame walk to the envelope/coupling tree gap — and the cursor
    // must not move.
    assert_eq!(
        DecodeGate::from_flags(RADECODE_FLAGS_DECODE),
        DecodeGate::Decode
    );
    assert_eq!(
        DecodeGate::from_flags(RADECODE_FLAGS_DECODE).backend_gate_bit(),
        0
    );
    assert_eq!(DecodeGate::from_flags(0).backend_gate_bit(), 1);

    let mut driver = real_driver();
    let payload = collect_packet_payloads(FIXTURE)[0];
    let mut out = vec![0u8; VALIDATED_FIRST_CALL_PCM_BYTES as usize];
    let err = driver
        .decode_call_with_flags(payload, &mut out, 0, RADECODE_FLAGS_DECODE)
        .unwrap_err();
    assert!(matches!(
        err,
        Error::EnvelopeValueTreeUnavailable | Error::CouplingIndexTreeUnavailable
    ));
    assert_eq!(driver.calls_completed(), 0);
    assert_eq!(driver.total_pcm_emitted(), 0);
}

#[test]
fn default_common_mode_matches_validated_real_stream_path() {
    // The validated real-stream path is common-mode-off (validation/04
    // §4.3 / §5: 144 packets fed straight from the container with no
    // external descramble). The driver's construction default must
    // match it, so consumers wiring a real stream with `Driver::new`
    // alone reproduce the validator's exact mode.
    let driver = real_driver();
    assert!(!driver.common_mode().is_on());
    // Confirm by feeding a real packet through the off-path and seeing
    // the descrambled bytes are the input verbatim.
    let payload = collect_packet_payloads(FIXTURE)[0];
    let prepared = driver.prepare_call(payload, 0xDEAD_BEEF).unwrap();
    assert_eq!(
        prepared.descrambled(),
        payload,
        "off-path is verbatim, key argument ignored"
    );
}

#[test]
fn with_common_mode_on_runs_xor_pass_self_inverse() {
    // Toggling common-mode-on at construction must route prepare_call
    // through the XOR descramble. The on-path has no bit-exact ground
    // truth (validation/04 §4.3: the validated path is off), so the
    // test pins only the algebraic property (self-inverse under
    // double application with the same key) plus byte-count
    // preservation — same regime the descramble module's tests use.
    let driver = real_driver().with_common_mode(CommonMode::on());
    let payload = collect_packet_payloads(FIXTURE)[42];
    let key = 0x1234_5678u32;
    let first_pass = driver.prepare_call(payload, key).unwrap();
    let scrambled = first_pass.descrambled().to_vec();
    assert_eq!(scrambled.len(), payload.len());
    assert_ne!(&scrambled[..], payload, "on-path changes bytes");
    // Second application restores the input.
    let second_pass = driver.prepare_call(&scrambled, key).unwrap();
    assert_eq!(second_pass.descrambled(), payload);
}
