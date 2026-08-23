//! Real-stream cross-check of the resume-from-blocker synthesis path.
//!
//! Walks the bundled `tests/fixtures/FUN_RM_32.rm` to its 144
//! validator-pinned 465-byte audio payloads and drives them through
//! [`Driver::synthesized_call`] — the full `RADecode` orchestration
//! (descramble → sub-packet split → the statically-pinned §1.1/§2.1
//! frame-body prefix on the **real** bitstream) finished through the §5
//! [`SynthesisBackend`] with caller-supplied post-entropy spectra (the
//! §3.2 GAP input, silent here — docs-gap #1775):
//!
//! - Every real packet's prefix walk succeeds (144 calls × 5 sub-packets
//!   of real gain headers + subband geometry).
//! - With silent spectra the emitted PCM is byte-identical to the
//!   observe-gate output ([`Driver::decode_call_with_flags`] with
//!   `flags` bit 0 = 0 — `validation/04` §4.3), call by call.
//! - The 144-call walk reproduces the validator's `2 936 832`-byte
//!   total exactly, with the constant three-frame carry backlog.
//!
//! The frame-length synthesis window is a recorded GAP (only the
//! 3/7/15/31/64 rows are extracted); the exact-TDAC fixture window used
//! here is a test fixture, not a claim about the codec's window.
//!
//! This file deliberately shares the fixture-walking helpers with
//! `realstream_fixture.rs` rather than depending on it: every test
//! module is built standalone in `cargo test`.

use oxideav_cook::{
    flavor_record, CookCookie, DecodeConfig, Descriptor, Driver, Error, FrameSpectrum,
    StereoSpectra, SynthesisBackend, EXTENDED_COOKIE_LEN,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/FUN_RM_32.rm");

const VALIDATED_PACKETS: u32 = 144;
const VALIDATED_PACKET_HEADER_BYTES: u32 = 12;
const VALIDATED_TOTAL_PCM_BYTES: u64 = 2_936_832;

const VALIDATED_COOKIE: [u8; 16] = [
    0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x04,
];

fn be_u16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be_u32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

struct Chunk<'a> {
    fcc: [u8; 4],
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
        out.push(Chunk {
            fcc,
            body: &file[p + 8..p + size as usize],
        });
        p += size as usize;
    }
    out
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
        payloads.push(&body[p + VALIDATED_PACKET_HEADER_BYTES as usize..p + plen]);
        p += plen;
        if payloads.len() == num_packets as usize {
            break;
        }
    }
    assert_eq!(payloads.len(), VALIDATED_PACKETS as usize);
    payloads
}

fn real_config() -> DecodeConfig {
    assert_eq!(VALIDATED_COOKIE.len(), EXTENDED_COOKIE_LEN);
    let cookie = CookCookie::parse(&VALIDATED_COOKIE).unwrap();
    let descriptor = Descriptor {
        channels_divisor: 2,
        sub_packet_size: 93,
    };
    let flavor = flavor_record(21).unwrap();
    DecodeConfig::from_inputs(&cookie, &descriptor, &flavor, 465).unwrap()
}

/// Exact-TDAC synthesis-window test fixture (`sin(π(k+½)/2L)`) — the
/// frame-length window itself is a recorded GAP.
fn synthetic_tdac_window(hop: usize) -> Vec<f32> {
    (0..2 * hop)
        .map(|k| ((k as f64 + 0.5) * core::f64::consts::PI / (2.0 * hop as f64)).sin() as f32)
        .collect()
}

fn silent_stereo_frame() -> FrameSpectrum {
    FrameSpectrum::Stereo(StereoSpectra {
        ch0: vec![0.0; 50],
        ch1: vec![0.0; 50],
    })
}

#[test]
fn synthesized_walk_matches_observe_gate_on_all_144_real_packets() {
    let payloads = collect_packet_payloads(FIXTURE);
    let cfg = real_config();
    let window = synthetic_tdac_window(cfg.samples_per_frame as usize);
    let mut backend = SynthesisBackend::new(&cfg, &window).unwrap();
    let mut synth_driver = Driver::new(cfg);
    let mut observe_driver = Driver::new(cfg);
    let frames: Vec<FrameSpectrum> = (0..cfg.sub_packets_per_call)
        .map(|_| silent_stereo_frame())
        .collect();

    for (idx, payload) in payloads.iter().enumerate() {
        let budget = synth_driver.next_call_pcm_bytes() as usize;
        let mut synth_out = vec![0x5Au8; budget];
        synth_driver
            .synthesized_call(payload, &mut synth_out, 0, &mut backend, &frames)
            .unwrap_or_else(|e| panic!("synthesized_call failed on packet {idx}: {e}"));

        // The observe gate on the same packet (flags bit 0 = 0) —
        // validation/04 §4.3 pins its output as zeroed PCM.
        let mut observe_out = vec![0xA5u8; budget];
        observe_driver
            .decode_call_with_flags(payload, &mut observe_out, 0, 0)
            .unwrap();

        assert_eq!(
            synth_out, observe_out,
            "silent synthesis differs from the observe gate on packet {idx}"
        );
    }
    assert_eq!(synth_driver.calls_completed(), VALIDATED_PACKETS);
    assert_eq!(synth_driver.total_pcm_emitted(), VALIDATED_TOTAL_PCM_BYTES);
    // The constant carry backlog (pcm_bytes_per_call − warmup).
    assert_eq!(backend.buffered(), 12_288);
}

#[test]
fn synthesized_call_rejects_wrong_frame_count_without_advancing() {
    let payloads = collect_packet_payloads(FIXTURE);
    let cfg = real_config();
    let window = synthetic_tdac_window(cfg.samples_per_frame as usize);
    let mut backend = SynthesisBackend::new(&cfg, &window).unwrap();
    let mut driver = Driver::new(cfg);
    let mut out = vec![0u8; driver.next_call_pcm_bytes() as usize];
    let frames = vec![silent_stereo_frame(); 4]; // needs 5
    let err = driver
        .synthesized_call(payloads[0], &mut out, 0, &mut backend, &frames)
        .unwrap_err();
    assert_eq!(
        err,
        Error::FrameSpectrumCountMismatch {
            got: 4,
            expected: 5
        }
    );
    assert_eq!(driver.calls_completed(), 0);
    assert_eq!(backend.buffered(), 0);
}

#[test]
fn synthesized_call_validates_buffers_before_backend_state() {
    let payloads = collect_packet_payloads(FIXTURE);
    let cfg = real_config();
    let window = synthetic_tdac_window(cfg.samples_per_frame as usize);
    let mut backend = SynthesisBackend::new(&cfg, &window).unwrap();
    let mut driver = Driver::new(cfg);
    let frames = vec![silent_stereo_frame(); 5];

    // Wrong input size.
    let mut out = vec![0u8; driver.next_call_pcm_bytes() as usize];
    let err = driver
        .synthesized_call(&payloads[0][..464], &mut out, 0, &mut backend, &frames)
        .unwrap_err();
    assert!(matches!(err, Error::CallInputLengthMismatch { .. }));

    // Wrong output size (first call expects the 8 192-byte warm-up).
    let mut wrong_out = vec![0u8; 20_480];
    let err = driver
        .synthesized_call(payloads[0], &mut wrong_out, 0, &mut backend, &frames)
        .unwrap_err();
    assert!(matches!(err, Error::CallOutputLengthMismatch { .. }));

    // No state was touched by either rejection.
    assert_eq!(driver.calls_completed(), 0);
    assert_eq!(backend.buffered(), 0);
}

/// The real-data finding that foreshadowed the round-9 §1.1 withdrawal:
/// under the old "6-bit segment count biased −6" reading, 12 of the 144
/// validated call heads biased negative. Re-read under the pinned §0.2
/// layout the head bits are the 1-bit sub-packet flag and the coupling
/// mode flag — and the wire statistics fit that reading instead: the
/// sub-packet flag is 0 on 139 of the 144 call heads (the traced frames
/// all carried 0), and the coupling mode flag is genuinely mixed (107
/// of 144 select the VLC branch, matching the trace seeing both
/// branches on real frames).
#[test]
fn real_call_heads_fit_the_section02_flag_reading() {
    let payloads = collect_packet_payloads(FIXTURE);
    let flag0: usize = payloads.iter().filter(|p| p[0] >> 7 == 0).count();
    assert_eq!(flag0, 139, "sub-packet flag 0 on 139/144 call heads");
    let vlc: usize = payloads.iter().filter(|p| (p[0] >> 6) & 1 == 1).count();
    assert_eq!(vlc, 107, "coupling VLC branch on 107/144 call heads");
    // The old §1.1 reading's contradiction, kept as the historical
    // pointer: 12 of 144 heads read `< 6` under a top-6-bit field.
    let old_underflow = payloads.iter().filter(|p| (p[0] >> 2) < 6).count();
    assert_eq!(old_underflow, 12);
}
