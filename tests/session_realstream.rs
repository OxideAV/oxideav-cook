//! Real-stream cross-check of the `RADecode` call-sequence session.
//!
//! Walks the bundled `tests/fixtures/FUN_RM_32.rm` RealMedia file
//! directly from the wire bytes (no library demuxer) to its 144 audio
//! packets, then exercises [`oxideav_cook::CallSession`] against them:
//!
//! - The session's per-call expected input length equals the
//!   container's 465-byte payload length (`validation/04` §2.2 / §5).
//! - The session's per-call PCM budget steps from the 8 192-byte
//!   warm-up on call 0 to the steady-state 20 480-byte budget on every
//!   subsequent call (`validation/04` §5).
//! - Walking the full 144-call sequence with the right buffer lengths
//!   accumulates exactly the validator's pinned 2 936 832-byte total
//!   PCM (`validation/04` §5).
//! - Wrong input / output lengths surface as the typed
//!   `Error::CallInputLengthMismatch` / `Error::CallOutputLengthMismatch`
//!   without advancing the session state.
//!
//! Source: `docs/audio/cook/spec/01-cook-decoder-structure.md` §5 +
//! `docs/audio/cook/validation/04-cook-stream-validation.md` §5.

use oxideav_cook::{flavor_record, CallSession, CookCookie, DecodeConfig, Descriptor, Error};

const FIXTURE: &[u8] = include_bytes!("fixtures/FUN_RM_32.rm");

// Validator pins (validation/04 §2 / §5).
const VALIDATED_PACKETS: usize = 144;
const VALIDATED_PACKET_PAYLOAD: usize = 465;
const VALIDATED_TOTAL_PCM_BYTES: u64 = 2_936_832;
const VALIDATED_WARMUP_PCM_BYTES: u32 = 8_192;
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
/// `DATA` chunk body.
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
/// a slice.
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

fn real_config() -> DecodeConfig {
    let cookie_bytes = [
        0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00,
        0x04,
    ];
    let cookie = CookCookie::parse(&cookie_bytes).unwrap();
    let flavor = flavor_record(21).unwrap();
    DecodeConfig::from_inputs(&cookie, &REAL_DESCRIPTOR, &flavor, 465).unwrap()
}

/// Walking the bundled 144-packet sequence with the validator-pinned
/// per-call output budget (warm-up on call 0, steady-state thereafter)
/// reproduces the validator's `2 936 832`-byte total PCM exactly.
#[test]
fn full_144_call_session_matches_validator_total() {
    let payloads = audio_payloads(FIXTURE);
    assert_eq!(payloads.len(), VALIDATED_PACKETS);

    let cfg = real_config();
    let mut session = CallSession::from_config(&cfg);

    // Walk the full sequence.
    for (idx, pkt) in payloads.iter().enumerate() {
        let expected_in = session.next_call_expected_input_len() as usize;
        let expected_out = session.next_call_pcm_bytes() as usize;
        assert_eq!(
            expected_in, VALIDATED_PACKET_PAYLOAD,
            "call {idx} input length"
        );
        let want_out = if idx == 0 {
            VALIDATED_WARMUP_PCM_BYTES as usize
        } else {
            VALIDATED_STEADY_PCM_BYTES as usize
        };
        assert_eq!(
            expected_out, want_out,
            "call {idx} PCM budget ({} on call 0, {} thereafter)",
            VALIDATED_WARMUP_PCM_BYTES, VALIDATED_STEADY_PCM_BYTES,
        );
        // The PCM byte range starts at the running total.
        assert_eq!(
            session.next_call_pcm_byte_range().start,
            session.total_pcm_emitted(),
            "call {idx} range.start tracks the cursor"
        );
        session.advance_one_call(pkt.len(), want_out).unwrap();
    }

    assert_eq!(session.calls_completed(), VALIDATED_PACKETS as u32);
    assert_eq!(session.total_pcm_emitted(), VALIDATED_TOTAL_PCM_BYTES);
}

/// The session's running PCM cursor matches
/// `DecodeConfig::total_pcm_bytes(calls_completed)` at every step of
/// the 144-call walk.
#[test]
fn running_cursor_tracks_decode_config_total() {
    let payloads = audio_payloads(FIXTURE);
    let cfg = real_config();
    let mut session = CallSession::from_config(&cfg);
    for (idx, pkt) in payloads.iter().enumerate() {
        let want_total = cfg.total_pcm_bytes(idx as u32);
        assert_eq!(session.total_pcm_emitted(), want_total, "before call {idx}");
        let want_out = if idx == 0 {
            VALIDATED_WARMUP_PCM_BYTES as usize
        } else {
            VALIDATED_STEADY_PCM_BYTES as usize
        };
        session.advance_one_call(pkt.len(), want_out).unwrap();
    }
    assert_eq!(
        session.total_pcm_emitted(),
        cfg.total_pcm_bytes(VALIDATED_PACKETS as u32)
    );
}

/// Supplying a real container payload of the wrong length surfaces the
/// typed input mismatch (and leaves the session state untouched).
#[test]
fn wrong_input_length_surfaces_typed_error() {
    let payloads = audio_payloads(FIXTURE);
    let mut session = CallSession::from_config(&real_config());
    // Take a real packet, slice off one byte to make 464.
    let short = &payloads[0][..VALIDATED_PACKET_PAYLOAD - 1];
    let err = session
        .advance_one_call(short.len(), VALIDATED_WARMUP_PCM_BYTES as usize)
        .unwrap_err();
    assert_eq!(
        err,
        Error::CallInputLengthMismatch {
            got: VALIDATED_PACKET_PAYLOAD - 1,
            expected: VALIDATED_PACKET_PAYLOAD,
        }
    );
    assert_eq!(session.calls_completed(), 0);
    assert_eq!(session.total_pcm_emitted(), 0);
}

/// On a steady-state call, supplying the first-call warm-up budget
/// surfaces the typed output mismatch (the session distinguishes the
/// two budgets per call index).
#[test]
fn wrong_output_length_on_steady_state_call_surfaces_typed_error() {
    let payloads = audio_payloads(FIXTURE);
    let mut session = CallSession::from_config(&real_config());
    // Walk the warm-up correctly.
    session
        .advance_one_call(payloads[0].len(), VALIDATED_WARMUP_PCM_BYTES as usize)
        .unwrap();
    // Now call 1 wants steady-state, not warm-up.
    let err = session
        .advance_one_call(payloads[1].len(), VALIDATED_WARMUP_PCM_BYTES as usize)
        .unwrap_err();
    assert_eq!(
        err,
        Error::CallOutputLengthMismatch {
            got: VALIDATED_WARMUP_PCM_BYTES as usize,
            expected: VALIDATED_STEADY_PCM_BYTES as usize,
        }
    );
    // State unchanged after the rejection.
    assert_eq!(session.calls_completed(), 1);
    assert_eq!(
        session.total_pcm_emitted(),
        VALIDATED_WARMUP_PCM_BYTES as u64
    );
}
