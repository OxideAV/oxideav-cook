//! End-to-end real-stream cross-check of [`oxideav_cook::DecodeConfig`].
//!
//! All inputs are the validated values from
//! `docs/audio/cook/validation/04-cook-stream-validation.md` for the
//! `FUN_RM_32.rm` stream. The expected outputs are the exact numbers
//! the validator measured when driving `cook.dll` against the real
//! `.rm` packets:
//!
//! - 144 `RADecode` calls, all returning S_OK
//! - 2 936 832 total PCM bytes (= 16.649 s of stereo 44.1 kHz 16-bit)
//! - 20 480 PCM bytes per steady-state call (= 5 sub-packets × 1024
//!   samples × 2 ch × 2 bytes)
//! - 8 192 PCM bytes on the first call (one frame, overlap-add warm-up)

use oxideav_cook::{
    flavor_indices_matching_cookie, flavor_record, CookCookie, DecodeConfig, Descriptor,
    RADECODE_FLAGS_DECODE,
};

/// Validated 16-byte extradata cookie.
const FUN_RM_32_COOKIE: [u8; 16] = [
    0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x04,
];

/// Validated descriptor scalars: `+0x06 = channels = 2`,
/// `+0x0a = sub_packet_size = 93`.
const FUN_RM_32_DESCRIPTOR: Descriptor = Descriptor {
    channels_divisor: 2,
    sub_packet_size: 93,
};

/// Validated per-stream `coded_frame_size` from the RealAudio v5
/// header (`= 465 = 5 × 93`).
const FUN_RM_32_FRAME_BYTES: u32 = 465;

/// Validated DATA-chunk packet count from the container parse.
const FUN_RM_32_RADECODE_CALLS: u32 = 144;

/// Validated total PCM byte count produced for the real stream.
const FUN_RM_32_TOTAL_PCM_BYTES: u64 = 2_936_832;

/// Validated steady-state PCM bytes per `RADecode` call.
const FUN_RM_32_PCM_BYTES_PER_CALL: u32 = 20_480;

/// Validated first-call (overlap-add warm-up) PCM byte count.
///
/// `8 192 = 2 × 1024 samples × 2 ch × 2 bytes` — the decoder primes
/// the overlap-add chain with two transform-frames' worth of PCM on
/// its first call.
const FUN_RM_32_FIRST_CALL_PCM_BYTES: u64 = 8_192;

#[test]
fn fun_rm_32_decode_config_matches_validator() {
    let cookie = CookCookie::parse(&FUN_RM_32_COOKIE).expect("cookie parses");
    let flavor = flavor_record(21).expect("flavor 21 present");

    let cfg = DecodeConfig::from_inputs(
        &cookie,
        &FUN_RM_32_DESCRIPTOR,
        &flavor,
        FUN_RM_32_FRAME_BYTES,
    )
    .expect("real-stream inputs wire cleanly");

    // Per-stream geometry, pinned by validation/04 §4.
    assert_eq!(cfg.channels, 2, "channels");
    assert_eq!(cfg.sample_rate_hz, 44_100, "sample rate");
    assert_eq!(cfg.samples_per_frame, 1024, "samples per frame");
    assert_eq!(cfg.subband_count, 32, "subband count");
    assert_eq!(cfg.stereo_mode, 4, "stereo / coupling mode");
    assert_eq!(cfg.frame_bytes, 465, "coded frame size");
    assert_eq!(cfg.sub_packet_size, 93, "descriptor +0x0a");

    // Derived per-call geometry, pinned by validation/04 §5.
    assert_eq!(cfg.sub_packets_per_call, 5, "465 / 93");
    assert_eq!(
        cfg.pcm_bytes_per_call, FUN_RM_32_PCM_BYTES_PER_CALL,
        "5 × 1024 × 2 ch × 2 bytes"
    );

    // Decode-enable flag is the validated bit.
    assert_eq!(
        RADECODE_FLAGS_DECODE, 1,
        "RADecode flags=1 enables backend decode per validation §4.3"
    );

    // Total-PCM accounting matches the validator end-to-end:
    // one warm-up call + 143 steady-state calls = 2 936 832 bytes.
    let total = cfg.total_pcm_bytes(FUN_RM_32_RADECODE_CALLS);
    assert_eq!(
        total, FUN_RM_32_TOTAL_PCM_BYTES,
        "total PCM bytes over 144 RADecode calls"
    );
    // First call alone is the overlap-add warm-up frame.
    assert_eq!(
        cfg.total_pcm_bytes(1),
        FUN_RM_32_FIRST_CALL_PCM_BYTES,
        "first-call PCM (overlap-add warm-up)"
    );

    // Recover wall-clock duration: 2 936 832 / 4 bytes per stereo frame
    // / 44 100 Hz = 16.6 s, matching MDPR duration=16716 ms.
    let frames = (total / 4) as f64;
    let secs = frames / cfg.sample_rate_hz as f64;
    assert!(
        (secs - 16.649).abs() < 1e-3,
        "decoded duration {secs:.3}s should match validator's 16.649 s"
    );
}

#[test]
fn fun_rm_32_cookie_matches_records_21_and_22() {
    // The cookie carries (channels, subband_count, stereo_mode,
    // samples_per_frame) but NOT (frame_bytes, sample_rate_hz,
    // coupling_mode), so multiple flavor records can agree with it.
    // On the validated FUN_RM_32.rm stream the cookie's 4-tuple is
    // (2, 32, 4, 1024), which records 21 and 22 share — they differ
    // only in `frame_bytes` (744 vs 1024). The container's
    // `coded_frame_size=465` cannot disambiguate by equality either;
    // the .ra5 header's explicit `flavor=21` is what selects the
    // correct record at open time.
    let cookie = CookCookie::parse(&FUN_RM_32_COOKIE).expect("cookie parses");
    let matches = flavor_indices_matching_cookie(&cookie);
    assert!(
        matches.contains(&21),
        "record 21 should match real-stream cookie: {matches:?}"
    );
    assert!(
        matches.contains(&22),
        "record 22 should also match real-stream cookie: {matches:?}"
    );
    // No record outside the (channels=2, subband=32, stereo_mode=4,
    // spf=1024) 4-tuple may appear.
    for idx in &matches {
        let r = flavor_record(*idx).expect("matching record present");
        assert_eq!(r.channels, 2);
        assert_eq!(r.subband_count, 32);
        assert_eq!(r.stereo_mode, 4);
        assert_eq!(r.samples_per_frame, 1024);
    }
}
