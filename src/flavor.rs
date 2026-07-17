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
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub const FLAVOR_COUNT: u8 = 31;

/// Value returned by the ordinal-7 export `RAGetNumberOfFlavors`
/// (`cook.dll!0x1620`), pinned by
/// `docs/audio/cook/provenance/03-cook-audit.md` audit point #2 as the
/// hardcoded immediate `mov ax, 0x0f; ret` (= 15). Surfaced as a typed
/// constant so a stream sniffer that reproduces the binary's published
/// API surface can return the same legacy flavor-count without
/// retyping the literal.
///
/// Distinct from [`FLAVOR_COUNT`] (= 31, the count of decodable
/// geometry records in the vendored table) and from
/// [`RA_GET_NUMBER_OF_FLAVORS2_ADVERTISED`] (= 34, the property-
/// descriptor count returned by the ordinal-9 sibling).
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub const RA_GET_NUMBER_OF_FLAVORS_ADVERTISED: u8 = 0x0f;

/// Value returned by the ordinal-9 export `RAGetNumberOfFlavors2`
/// (`cook.dll!0x1630`), pinned by
/// `docs/audio/cook/provenance/03-cook-audit.md` audit point #2 as the
/// hardcoded immediate `mov ax, 0x22; ret` (= 34). The same `0x22`
/// immediate gates the `RASetFlavor` upper bound (`cmp …, 0x22` at
/// `cook.dll!0x1640`) and the property-worker bound at
/// `cook.dll!0x17a0`; audit point #12 resolves it as the
/// property-descriptor count, distinct from [`FLAVOR_COUNT`] (= 31,
/// the count of decodable geometry records).
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub const RA_GET_NUMBER_OF_FLAVORS2_ADVERTISED: u8 = 0x22;

/// The closing single-subband sentinel record's index. The geometry
/// record at this index is the non-music sentinel pinned by
/// `docs/audio/cook/spec/02-cook-flavor-and-extradata-layout.md` §1.1
/// (audit-resolved block: *"the last geometry record (index 30 =
/// `(17,5,1024,1,1,256,44100)`) has subband count 1 and is a sentinel /
/// non-music entry"*).
///
/// Use [`FlavorRecord::is_sentinel`] to discriminate this record at
/// runtime and [`iter_playable_flavor_records`] to walk only the 30
/// non-sentinel presets at indices 0..=29.
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub const SENTINEL_FLAVOR_INDEX: u8 = 30;

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
    /// True iff this record is the closing single-subband sentinel
    /// pinned by `docs/audio/cook/spec/02-cook-flavor-and-extradata-
    /// layout.md` §1.1 (the index-30 non-music entry).
    ///
    /// The discriminating field is `subband_count == 1`: every other
    /// well-formed flavor record carries a `subband_count` of at least
    /// `12` (audit point #14 / extracted table; the field grows with
    /// sample rate and bitrate per spec/02 §1 line 34), so the
    /// sentinel is the lone record that hits the minimum value.
    ///
    /// Useful for walkers that should skip the sentinel: see
    /// [`iter_playable_flavor_records`].
    pub fn is_sentinel(&self) -> bool {
        self.subband_count == 1
    }

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
#[doc(hidden)] // internal — exposed for tests/fuzz; not part of the stable API
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
#[doc(hidden)] // internal — exposed for tests/fuzz; not part of the stable API
pub fn iter_flavor_records() -> impl Iterator<Item = (u8, FlavorRecord)> {
    data_lines()
        .enumerate()
        .filter_map(|(i, l)| FlavorRecord::parse_line(l).map(|r| (i as u8, r)))
}

/// Iterator over every playable (non-sentinel) `(index, record)` pair
/// in the vendored geometry table.
///
/// Visits exactly `FLAVOR_COUNT - 1` (= 30) pairs, in table order from
/// index `0` to index `29` — the index-30 sentinel record
/// ([`SENTINEL_FLAVOR_INDEX`]) is filtered out by
/// [`FlavorRecord::is_sentinel`]. Use this walker when a consumer
/// wants to enumerate only the decodable music presets.
///
/// Anchored by `docs/audio/cook/spec/02-cook-flavor-and-extradata-
/// layout.md` §1.1 (sentinel record at index 30).
#[doc(hidden)] // internal — exposed for tests/fuzz; not part of the stable API
pub fn iter_playable_flavor_records() -> impl Iterator<Item = (u8, FlavorRecord)> {
    iter_flavor_records().filter(|(_, r)| !r.is_sentinel())
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
#[doc(hidden)] // internal — exposed for tests/fuzz; not part of the stable API
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
    fn advertised_counts_are_15_and_34() {
        // Pin the two published API-surface counts the binary's
        // `RAGetNumberOfFlavors` / `RAGetNumberOfFlavors2` exports
        // return as hardcoded immediates (audit point #2).
        assert_eq!(RA_GET_NUMBER_OF_FLAVORS_ADVERTISED, 15);
        assert_eq!(RA_GET_NUMBER_OF_FLAVORS2_ADVERTISED, 34);
        // The two advertised counts are distinct from each other and
        // distinct from the table-derived FLAVOR_COUNT.
        assert_ne!(
            RA_GET_NUMBER_OF_FLAVORS_ADVERTISED,
            RA_GET_NUMBER_OF_FLAVORS2_ADVERTISED
        );
        assert_ne!(RA_GET_NUMBER_OF_FLAVORS_ADVERTISED, FLAVOR_COUNT);
        assert_ne!(RA_GET_NUMBER_OF_FLAVORS2_ADVERTISED, FLAVOR_COUNT);
        // The sentinel index is the closing entry of the table.
        assert_eq!(SENTINEL_FLAVOR_INDEX, FLAVOR_COUNT - 1);
    }

    #[test]
    fn sentinel_predicate_fires_only_on_index_30() {
        // Walk every well-formed record. is_sentinel() is true exactly
        // at SENTINEL_FLAVOR_INDEX (= 30) and false on every other.
        for (idx, rec) in iter_flavor_records() {
            if idx == SENTINEL_FLAVOR_INDEX {
                assert!(
                    rec.is_sentinel(),
                    "index {idx} must be the sentinel (subband_count = 1): {rec:?}"
                );
                assert_eq!(rec.subband_count, 1);
            } else {
                assert!(
                    !rec.is_sentinel(),
                    "index {idx} is a playable preset, but is_sentinel() fired: {rec:?}"
                );
                // The discriminating threshold is `>= 2`: only the
                // sentinel hits the minimum `1`. The smallest playable
                // record (index 8: 9 kHz mono 8-kHz preset) carries
                // `subband_count = 9`; the rest grow with sample rate
                // and bitrate per spec/02 §1 line 34.
                assert!(
                    rec.subband_count >= 2,
                    "playable record at index {idx} should have subband_count >= 2: {rec:?}"
                );
            }
        }
        // The known sentinel shape, end-to-end:
        // `(17, 5, 1024, 1, 1, 256, 44100)` per spec/02 §1.1.
        let sentinel = flavor_record(SENTINEL_FLAVOR_INDEX).unwrap();
        assert!(sentinel.is_sentinel());
        assert_eq!(sentinel.coupling_mode, 17);
        assert_eq!(sentinel.stereo_mode, 5);
        assert_eq!(sentinel.samples_per_frame, 1024);
        assert_eq!(sentinel.channels, 1);
        assert_eq!(sentinel.subband_count, 1);
        assert_eq!(sentinel.frame_bytes, 256);
        assert_eq!(sentinel.sample_rate_hz, 44100);
    }

    #[test]
    fn iter_playable_yields_exactly_30_records() {
        // The playable walker visits FLAVOR_COUNT - 1 = 30 pairs,
        // covering indices 0..=29 in order, each !is_sentinel().
        let playable: Vec<(u8, FlavorRecord)> = iter_playable_flavor_records().collect();
        assert_eq!(playable.len(), (FLAVOR_COUNT - 1) as usize);
        for (i, (idx, rec)) in playable.iter().enumerate() {
            assert_eq!(
                *idx, i as u8,
                "iter_playable must visit consecutive indices 0..=29"
            );
            assert!(*idx < SENTINEL_FLAVOR_INDEX);
            assert!(!rec.is_sentinel());
            // Round-trip vs flavor_record at the same index.
            assert_eq!(Some(*rec), flavor_record(*idx));
        }
        // The sentinel must be absent from the playable walk.
        assert!(playable
            .iter()
            .all(|(idx, _)| *idx != SENTINEL_FLAVOR_INDEX));
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
