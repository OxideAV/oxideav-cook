//! Per-flavor geometry table.
//!
//! A *flavor* is a preset bundle of (sample rate, channels,
//! samples-per-frame, transform geometry, packet size) selected by a
//! small integer index carried in the stream header. The decoder keeps
//! this table baked in; here the same geometry is loaded from the
//! vendored facts file `tables/flavor-geometry-table.csv` (31 records,
//! indices 0–30) and parsed on demand so the numeric values are never
//! retyped into Rust source.
//!
//! Each record is seven fields:
//! `coupling_mode, stereo_mode, samples_per_frame, channels,
//! subband_count, frame_bytes, sample_rate_hz`.

/// The vendored geometry table (header row + 31 records).
const FLAVOR_TABLE_CSV: &str = include_str!("../tables/flavor-geometry-table.csv");

/// Number of well-formed geometry records in the table (indices 0–30).
///
/// The decoder advertises 34 flavors, but that count is a hardcoded
/// property-descriptor count, not the number of decodable geometry
/// presets: only indices with a well-formed geometry record (0–30, with
/// index 30 a single-subband sentinel) carry usable geometry.
pub const FLAVOR_COUNT: u8 = 31;

/// One row of the flavor geometry table.
///
/// All fields are stored as `u32` exactly as the table encodes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlavorRecord {
    /// Joint-coding / coupling-region selector (0 for plain mono/stereo
    /// flavors; small non-zero values for coupled stereo / multichannel).
    pub coupling_mode: u32,
    /// Secondary mode selector (0 for mono; 2–5 for stereo / surround).
    pub stereo_mode: u32,
    /// Transform frame length: one of 256 / 512 / 1024.
    pub samples_per_frame: u32,
    /// Channel count: 1 or 2.
    pub channels: u32,
    /// Number of coded subbands.
    pub subband_count: u32,
    /// Per-frame coded size in bytes.
    pub frame_bytes: u32,
    /// Sample rate in Hz: 8000 / 11025 / 22050 / 44100.
    pub sample_rate_hz: u32,
}

impl FlavorRecord {
    /// Parse one comma-separated record line into a [`FlavorRecord`].
    ///
    /// Returns `None` if the line does not hold exactly seven `u32`
    /// fields.
    fn parse_line(line: &str) -> Option<Self> {
        let mut it = line.split(',').map(|f| f.trim().parse::<u32>().ok());
        let mut next = || it.next().flatten();
        let rec = FlavorRecord {
            coupling_mode: next()?,
            stereo_mode: next()?,
            samples_per_frame: next()?,
            channels: next()?,
            subband_count: next()?,
            frame_bytes: next()?,
            sample_rate_hz: next()?,
        };
        // Reject lines with trailing extra columns.
        if it.next().is_some() {
            return None;
        }
        Some(rec)
    }
}

/// Iterator over the table's data lines (header skipped, blank lines
/// dropped).
fn data_lines() -> impl Iterator<Item = &'static str> {
    FLAVOR_TABLE_CSV
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty())
}

/// Look up the geometry record for a flavor `index`.
///
/// Returns `None` for indices `>= FLAVOR_COUNT` (no well-formed
/// geometry record exists there).
pub fn flavor_record(index: u8) -> Option<FlavorRecord> {
    data_lines()
        .nth(index as usize)
        .and_then(FlavorRecord::parse_line)
}

/// Iterator over every well-formed `(index, record)` pair in the
/// vendored geometry table.
///
/// Visits exactly [`FLAVOR_COUNT`] (= 31) pairs, in table order from
/// index `0` to index `30`. The closing pair (index 30) is the
/// single-subband sentinel (`subband_count = 1`) called out by
/// `docs/audio/cook/spec/02-cook-flavor-and-extradata-layout.md`
/// §1.1.
pub fn iter_flavor_records() -> impl Iterator<Item = (u8, FlavorRecord)> {
    data_lines()
        .enumerate()
        .filter_map(|(i, l)| FlavorRecord::parse_line(l).map(|r| (i as u8, r)))
}

/// Return every flavor index whose geometry record matches the five
/// fields a cookie itself carries (channels, subband count, stereo
/// mode, and `samples_per_frame` recovered from the cookie's
/// `[4..6]` product divided by the cookie's channel count).
///
/// A cookie does **not** carry `frame_bytes`, `sample_rate_hz`, or
/// `coupling_mode`, so multiple flavor records can describe the same
/// cookie — most notably on the bundled real stream (`flavor_record(21)`
/// and `flavor_record(22)` both agree with its cookie; the container's
/// `coded_frame_size` is what selects between them). Returns the
/// possibly-multi-element list in table order so callers can
/// disambiguate with container-supplied geometry.
///
/// Anchored by `docs/audio/cook/validation/04-cook-stream-validation.md`
/// §4.1 (the cookie field-set) and §4.4 (record-21 cross-check).
pub fn flavor_indices_matching_cookie(cookie: &crate::cookie::CookCookie) -> Vec<u8> {
    iter_flavor_records()
        .filter(|(_, r)| cookie.matches_flavor(r))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_exactly_31_records() {
        assert_eq!(data_lines().count(), FLAVOR_COUNT as usize);
        // Every record must parse cleanly.
        for line in data_lines() {
            assert!(
                FlavorRecord::parse_line(line).is_some(),
                "unparseable record: {line:?}"
            );
        }
    }

    #[test]
    fn first_records_match_known_geometry() {
        // (samples_per_frame, channels, sample_rate) for the leading
        // well-known mono flavors.
        let r0 = flavor_record(0).unwrap();
        assert_eq!(
            (r0.samples_per_frame, r0.channels, r0.sample_rate_hz),
            (256, 1, 8000)
        );
        let r4 = flavor_record(4).unwrap();
        assert_eq!(
            (r4.samples_per_frame, r4.channels, r4.sample_rate_hz),
            (1024, 1, 44100)
        );
    }

    #[test]
    fn record_21_is_the_real_stream_flavor() {
        let r = flavor_record(21).unwrap();
        assert_eq!(r.channels, 2);
        assert_eq!(r.sample_rate_hz, 44100);
        assert_eq!(r.subband_count, 32);
        assert_eq!(r.samples_per_frame, 1024);
        assert_eq!(r.stereo_mode, 4);
        assert_eq!(r.coupling_mode, 2);
    }

    #[test]
    fn every_record_is_well_formed() {
        for i in 0..FLAVOR_COUNT {
            let r = flavor_record(i).expect("record present");
            assert!(matches!(r.samples_per_frame, 256 | 512 | 1024), "spf {r:?}");
            assert!(matches!(r.channels, 1 | 2), "channels {r:?}");
            assert!(
                matches!(r.sample_rate_hz, 8000 | 11025 | 22050 | 44100),
                "rate {r:?}"
            );
        }
    }

    #[test]
    fn out_of_range_index_is_none() {
        assert!(flavor_record(FLAVOR_COUNT).is_none());
        assert!(flavor_record(u8::MAX).is_none());
    }

    #[test]
    fn iter_visits_every_record_in_order() {
        let collected: Vec<(u8, FlavorRecord)> = iter_flavor_records().collect();
        assert_eq!(collected.len(), FLAVOR_COUNT as usize);
        for (i, (idx, rec)) in collected.iter().enumerate() {
            assert_eq!(*idx, i as u8, "iter must visit indices in order");
            assert_eq!(Some(*rec), flavor_record(*idx));
        }
        // Spec/02 §1.1 sentinel: index 30 is the single-subband entry.
        let (last_idx, last_rec) = collected.last().copied().unwrap();
        assert_eq!(last_idx, FLAVOR_COUNT - 1);
        assert_eq!(last_rec.subband_count, 1);
    }

    #[test]
    fn cookie_matches_both_record_21_and_22_for_real_stream() {
        // The cookie carries (channels, subband_count, stereo_mode,
        // samples_per_frame); the real FUN_RM_32.rm cookie produces
        // (2, 32, 4, 1024). Records 21 and 22 both share that 4-tuple
        // and differ only in `frame_bytes` (744 vs 1024). This pins
        // the cookie ambiguity called out in
        // `docs/audio/cook/validation/04-cook-stream-validation.md`
        // §4.4 and is the reason `DecodeConfig::from_inputs` takes
        // a `frame_bytes` argument separately.
        const REAL_COOKIE: [u8; 16] = [
            0x01, 0x00, 0x00, 0x03, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
            0x00, 0x04,
        ];
        let c = crate::cookie::CookCookie::parse(&REAL_COOKIE).unwrap();
        let matches = flavor_indices_matching_cookie(&c);
        assert!(
            matches.contains(&21),
            "record 21 must match real-stream cookie: {matches:?}"
        );
        assert!(
            matches.contains(&22),
            "record 22 must match real-stream cookie (frame_bytes differs): {matches:?}"
        );
        // The two matching records differ only in frame_bytes; the
        // cookie cannot disambiguate.
        for idx in &matches {
            let r = flavor_record(*idx).unwrap();
            assert_eq!(r.channels, 2);
            assert_eq!(r.subband_count, 32);
            assert_eq!(r.stereo_mode, 4);
            assert_eq!(r.samples_per_frame, 1024);
        }
    }

    #[test]
    fn unmatchable_cookie_returns_empty() {
        // Construct a cookie whose 5 fields cannot describe any
        // vendored record (subband count 99 is outside every record).
        let bogus = crate::cookie::CookCookie {
            selector: crate::cookie::SELECTOR_EXTENDED,
            samples_per_frame_x_channels: 2048,
            subband_count: 99,
            reserved: 0,
            channels: 2,
            stereo_mode: 4,
        };
        assert!(flavor_indices_matching_cookie(&bogus).is_empty());
    }
}
