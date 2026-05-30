//! Real-stream cross-check: the `FUN_RM_32.rm` extradata cookie against
//! flavor geometry record 21.
//!
//! The 16 cookie bytes are copied verbatim from a real stereo 44.1 kHz
//! Cook stream. Every field the cookie carries must agree with the named
//! flavor record the stream header selects (flavor 21).

use oxideav_cook::{flavor_record, CookCookie};

/// The 16-byte extradata cookie of the real stream.
const FUN_RM_32_COOKIE: [u8; 16] = [
    0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x04,
];

#[test]
fn real_cookie_agrees_with_flavor_21() {
    let cookie = CookCookie::parse(&FUN_RM_32_COOKIE).expect("cookie parses");
    let record = flavor_record(21).expect("flavor 21 present");

    // Field-by-field, as pinned by the real-stream validation.
    assert_eq!(cookie.selector, 0x0100_0003, "extended selector");
    assert_eq!(cookie.samples_per_frame_x_channels, 2048, "spf × channels");
    assert_eq!(cookie.subband_count, 32, "subband count");
    assert_eq!(cookie.channels, 2, "channels");
    assert_eq!(cookie.stereo_mode, 4, "stereo mode");
    assert_eq!(
        cookie.samples_per_frame(),
        Some(1024),
        "recovered spf = 2048 / 2"
    );

    // The cookie self-describes the same configuration as record 21.
    assert!(
        cookie.matches_flavor(&record),
        "cookie {cookie:?} should match flavor record {record:?}"
    );

    // Spot-check the record's own geometry.
    assert_eq!(record.channels, 2);
    assert_eq!(record.subband_count, 32);
    assert_eq!(record.samples_per_frame, 1024);
    assert_eq!(record.stereo_mode, 4);
    assert_eq!(record.sample_rate_hz, 44100);
}
