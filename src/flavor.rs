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
}
