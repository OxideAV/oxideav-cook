//! Joint-stereo / coupling-mode classification of a flavor record's two
//! mode selectors.
//!
//! Source-of-truth: `docs/audio/cook/spec/02-cook-flavor-and-extradata-
//! layout.md` §1 (the flavor geometry record layout) and the extracted
//! `docs/audio/cook/tables/flavor-geometry-table.{csv,meta}`. The two
//! leading record fields are the codec's joint-coding selectors:
//!
//! - **`+0x00` coupling/region mode** — spec/02 §1 line 30:
//!   *"joint-coding / coupling-region selector (0 for the plain
//!   mono/stereo flavors; small non-zero values for the coupled stereo
//!   and multichannel flavors)"*. The extracted table carries the values
//!   `{0, 1, 2, 5, 6, 8, 17, 19}`: `0` on every plain (uncoupled) record,
//!   a small non-zero value on every coupled one.
//! - **`+0x04` stereo mode** — spec/02 §1 line 31:
//!   *"secondary mode selector (0 for mono; 2–5 for the stereo / surround
//!   families)"*. The extracted table carries the values `{0, 2, 3, 4,
//!   5}`: `0` on every mono record, `2..=5` on the stereo / surround
//!   families.
//!
//! Spec/01 §5 inventories *"joint-stereo / multichannel coupling"* as the
//! late decode stage; this module wires the **typed classification** of
//! the two selectors that gate it — turning the raw `u32` fields a
//! [`crate::flavor::FlavorRecord`] carries into a checked discriminator —
//! without yet implementing the coupling DSP itself (that worker is a
//! recorded GAP, see below).
//!
//! ## What this module provides
//!
//! - [`StereoMode`] — a typed view of the `+0x04` stereo-mode selector:
//!   [`StereoMode::Mono`] (`0`) versus the [`StereoMode::Stereo`] family
//!   carrying the raw `2..=5` value. Built by [`StereoMode::from_raw`],
//!   which rejects any value outside `{0} ∪ {2..=5}` (the set spec/02 §1
//!   pins and the extracted table exhibits) with
//!   [`crate::Error::StereoModeUnsupported`]. The reserved value `1` —
//!   which spec/02 §1 does **not** assign and which never appears in the
//!   extracted table — is rejected.
//! - [`CouplingMode`] — a typed view of the `+0x00` coupling/region
//!   selector: [`CouplingMode::None`] (`0`, plain/uncoupled) versus
//!   [`CouplingMode::Coupled`] carrying the raw non-zero region value.
//!   Built infallibly by [`CouplingMode::from_raw`] (spec/02 §1 admits
//!   *"small non-zero values"* without enumerating them, so any non-zero
//!   value classifies as coupled; the value is preserved verbatim).
//! - [`FlavorRecord::stereo_mode_class`] /
//!   [`FlavorRecord::coupling_mode_class`] — the two classifiers as
//!   methods on the record, reading its already-parsed raw fields.
//! - [`FlavorRecord::is_coupled`] / [`FlavorRecord::is_stereo`] — the two
//!   boolean shortcuts.
//!
//! ## What this module does *not* cover (DOCS-GAP)
//!
//! The *meaning* of each individual stereo-mode value `2/3/4/5` and each
//! coupling-region value (`1/2/5/6/8/17/19`) — which coupling algorithm,
//! how many coupled subbands, the per-band coupling coefficients — is a
//! recorded GAP: spec/02 §1 enumerates the value **ranges** and their
//! plain-versus-coupled split, and spec/01 §5 names the coupling stage in
//! the pipeline inventory, but neither pins the coupling worker's
//! arithmetic. This module classifies the selectors; it does not perform
//! the joint-stereo / multichannel coupling DSP.

use crate::{flavor::FlavorRecord, Error};

/// Typed view of a flavor record's `+0x04` **stereo mode** selector.
///
/// Spec/02 §1 line 31: *"0 for mono; 2–5 for the stereo / surround
/// families"*. The reserved value `1` is not assigned by the spec and
/// does not occur in the extracted geometry table, so it is not
/// constructible through [`StereoMode::from_raw`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StereoMode {
    /// Single-channel (mono) flavor — stereo-mode selector `0`.
    Mono,
    /// A stereo / surround flavor, carrying the raw selector value in
    /// `2..=5` (spec/02 §1: *"2–5 for the stereo / surround families"*).
    /// The discrete value meaning per family is a recorded GAP.
    Stereo(u8),
}

/// Lowest raw stereo-mode value classified as a stereo / surround
/// family (spec/02 §1: *"2–5"*).
pub const STEREO_MODE_MIN: u32 = 2;

/// Highest raw stereo-mode value classified as a stereo / surround
/// family (spec/02 §1: *"2–5"*).
pub const STEREO_MODE_MAX: u32 = 5;

impl StereoMode {
    /// Classify a raw `+0x04` stereo-mode selector.
    ///
    /// `0` → [`StereoMode::Mono`]; `2..=5` → [`StereoMode::Stereo`]
    /// carrying the raw value. Per spec/02 §1 the value `1` is reserved
    /// (unassigned, and absent from the extracted table) and any value
    /// `> 5` is undocumented; both raise
    /// [`Error::StereoModeUnsupported`].
    ///
    /// # Errors
    ///
    /// [`Error::StereoModeUnsupported`] for any raw value outside
    /// `{0} ∪ {2..=5}`.
    pub fn from_raw(raw: u32) -> Result<Self, Error> {
        match raw {
            0 => Ok(StereoMode::Mono),
            STEREO_MODE_MIN..=STEREO_MODE_MAX => Ok(StereoMode::Stereo(raw as u8)),
            _ => Err(Error::StereoModeUnsupported { got: raw }),
        }
    }

    /// The raw selector value this classification was built from
    /// (`0` for [`StereoMode::Mono`], the carried `2..=5` value for
    /// [`StereoMode::Stereo`]).
    pub const fn raw(self) -> u32 {
        match self {
            StereoMode::Mono => 0,
            StereoMode::Stereo(v) => v as u32,
        }
    }

    /// Whether this is one of the stereo / surround families (i.e. not
    /// [`StereoMode::Mono`]).
    pub const fn is_stereo(self) -> bool {
        matches!(self, StereoMode::Stereo(_))
    }
}

/// Typed view of a flavor record's `+0x00` **coupling/region mode**
/// selector.
///
/// Spec/02 §1 line 30: *"0 for the plain mono/stereo flavors; small
/// non-zero values for the coupled stereo and multichannel flavors"*.
/// The spec admits the non-zero values as a family without enumerating a
/// closed set, so any non-zero value classifies as
/// [`CouplingMode::Coupled`] and the raw region value is preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CouplingMode {
    /// No joint coupling — coupling-region selector `0` (the *"plain
    /// mono/stereo flavors"*).
    None,
    /// A coupled stereo / multichannel flavor, carrying the raw non-zero
    /// region value (spec/02 §1: *"small non-zero values"*; the
    /// extracted table exhibits `{1, 2, 5, 6, 8, 17, 19}`). The discrete
    /// region meaning is a recorded GAP.
    Coupled(u32),
}

impl CouplingMode {
    /// Classify a raw `+0x00` coupling/region selector.
    ///
    /// `0` → [`CouplingMode::None`]; any non-zero value →
    /// [`CouplingMode::Coupled`] carrying the raw value. Total: spec/02
    /// §1 documents only the zero / non-zero split, so this never fails.
    pub const fn from_raw(raw: u32) -> Self {
        if raw == 0 {
            CouplingMode::None
        } else {
            CouplingMode::Coupled(raw)
        }
    }

    /// The raw selector value this classification was built from
    /// (`0` for [`CouplingMode::None`], the carried value for
    /// [`CouplingMode::Coupled`]).
    pub const fn raw(self) -> u32 {
        match self {
            CouplingMode::None => 0,
            CouplingMode::Coupled(v) => v,
        }
    }

    /// Whether this flavor applies joint coupling (i.e. is
    /// [`CouplingMode::Coupled`]).
    pub const fn is_coupled(self) -> bool {
        matches!(self, CouplingMode::Coupled(_))
    }
}

impl FlavorRecord {
    /// Classify this record's `+0x00` coupling/region selector
    /// ([`FlavorRecord::coupling_mode`]) into a typed [`CouplingMode`].
    ///
    /// Total — see [`CouplingMode::from_raw`].
    pub const fn coupling_mode_class(&self) -> CouplingMode {
        CouplingMode::from_raw(self.coupling_mode)
    }

    /// Classify this record's `+0x04` stereo-mode selector
    /// ([`FlavorRecord::stereo_mode`]) into a typed [`StereoMode`].
    ///
    /// # Errors
    ///
    /// [`Error::StereoModeUnsupported`] if the record carries a
    /// stereo-mode value outside `{0} ∪ {2..=5}`. Every well-formed
    /// vendored record stays in range; the fallible signature guards a
    /// malformed/out-of-spec record.
    pub fn stereo_mode_class(&self) -> Result<StereoMode, Error> {
        StereoMode::from_raw(self.stereo_mode)
    }

    /// Whether this flavor applies joint coupling (its coupling/region
    /// selector is non-zero).
    pub const fn is_coupled(&self) -> bool {
        self.coupling_mode_class().is_coupled()
    }

    /// Whether this flavor is one of the stereo / surround families (its
    /// stereo-mode selector is `2..=5`).
    ///
    /// Reads the raw field directly so the predicate stays total: an
    /// out-of-spec stereo-mode value reports `false` (it is neither the
    /// `0` mono marker nor a `2..=5` stereo value) rather than erroring.
    pub const fn is_stereo(&self) -> bool {
        self.stereo_mode >= STEREO_MODE_MIN && self.stereo_mode <= STEREO_MODE_MAX
    }
}

// ---- §4.3 recovered coupling rotation table -------------------------
//
// The per-coupling-width coefficient table `spec/05` §4.3 recorded as a
// runtime-built GAP was recovered by the docs round-8 init drive
// (`provenance/06`, `tables/coupling-rotation-coeffs.meta`): the builder
// `cook.dll!0x40a0` runs at `RAInitDecoder` (tail-called from the window
// builder `0x3290`) and fills two buffers hung off the decode state —
// 256 unit-circle `(cos θ, sin θ)` pairs at `+0x47b4` and a 512-entry
// bit-reversal index at `+0x47b8` — for the recovered coupling width
// `1 << 9 = 512` (state `+0x47ac`). The accessors below expose the
// **logical** coefficient table `coef[j]` the §4.2 mirror split reads:
// the permutation maps a coupling index onto its element slot in the
// flattened pair buffer, and it is an involution (a bit reversal), so
// `coef[j] = flat[perm[j]]`.

/// The recovered coupling-index bit width (state `+0x1c` / `+0x47ac`):
/// the init drive built the table for `Ncoup = 1 << 9`.
pub const COUPLING_RECOVERED_BITS: u32 = 9;

/// The recovered coupling table length `Ncoup = 512`.
pub const COUPLING_RECOVERED_LEN: u32 = 1 << COUPLING_RECOVERED_BITS;

/// The logical §4.2/§4.3 coupling coefficient table `coef[0..Ncoup]`,
/// de-permuted from the two recovered buffers
/// ([`crate::tables::coupling_rotation_coeffs`] flattened, indexed
/// through [`crate::tables::coupling_index_permutation`]).
///
/// `coef[j]` is what `spec/05` §4.2 reads as the channel-0 factor for
/// coupling index `j`; the channel-1 factor is the mirror partner
/// `coef[Ncoup - 1 - j]`. The de-permuted table splits into a cosine
/// half followed by a sine half (the recovered pairs are unit-circle),
/// so the mirror pair behaves as the §4.2 `(cos θ_j, sin θ_j)` pan —
/// pinned numerically by the unit tests from the recovered values
/// alone.
pub fn coupling_coefficient_table() -> &'static [f32] {
    use std::sync::OnceLock;
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| {
        let pairs = crate::tables::coupling_rotation_coeffs();
        let perm = crate::tables::coupling_index_permutation();
        // Flatten the (cos, sin) pairs in buffer order, then read
        // through the permutation: coef[j] = flat[perm[j]].
        let flat: Vec<f32> = pairs.iter().flat_map(|p| p.iter().copied()).collect();
        perm.iter().map(|&s| flat[s as usize]).collect()
    })
}

/// One §4.3 coupling coefficient `coef[j]` of the recovered
/// `Ncoup = 512` table.
///
/// # Errors
///
/// [`Error::CouplingIndexOutOfRange`] when
/// `j >= `[`COUPLING_RECOVERED_LEN`].
pub fn coupling_coefficient(j: u32) -> Result<f32, Error> {
    if j >= COUPLING_RECOVERED_LEN {
        return Err(Error::CouplingIndexOutOfRange {
            got: j,
            ncoup: COUPLING_RECOVERED_LEN,
        });
    }
    Ok(coupling_coefficient_table()[j as usize])
}

/// The §4.2 mirror pan pair `(coef[j], coef[Ncoup-1-j])` for a coupling
/// index of the recovered table — channel 0's and channel 1's factors.
///
/// # Errors
///
/// [`Error::CouplingIndexOutOfRange`] when `j >= Ncoup`.
pub fn coupling_pan_pair(j: u32) -> Result<(f32, f32), Error> {
    let partner = crate::spectral::mirror_partner_index(j, COUPLING_RECOVERED_LEN)?;
    let t = coupling_coefficient_table();
    Ok((t[j as usize], t[partner as usize]))
}

/// Split one coupled coefficient by the §4.2 mirror rotation over the
/// **recovered** §4.3 table —
/// [`crate::spectral::split_coupled_coefficient`] with the vendored
/// values instead of a caller-supplied slice.
///
/// # Errors
///
/// [`Error::CouplingIndexOutOfRange`] when `j >= Ncoup`.
pub fn split_coupled_recovered(c: f32, j: u32) -> Result<(f32, f32), Error> {
    crate::spectral::split_coupled_coefficient(c, j, coupling_coefficient_table())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flavor::{flavor_record, iter_flavor_records, FLAVOR_COUNT};

    #[test]
    fn stereo_mode_classifies_documented_values() {
        // spec/02 §1: 0 -> Mono, 2..=5 -> Stereo(value).
        assert_eq!(StereoMode::from_raw(0).unwrap(), StereoMode::Mono);
        for v in STEREO_MODE_MIN..=STEREO_MODE_MAX {
            assert_eq!(
                StereoMode::from_raw(v).unwrap(),
                StereoMode::Stereo(v as u8)
            );
        }
    }

    #[test]
    fn stereo_mode_rejects_reserved_and_oob() {
        // 1 is reserved (unassigned by spec/02 §1, absent from the table)
        // and anything above 5 is undocumented.
        for got in [1u32, 6, 7, 100, u32::MAX] {
            assert_eq!(
                StereoMode::from_raw(got).unwrap_err(),
                Error::StereoModeUnsupported { got }
            );
        }
    }

    #[test]
    fn stereo_mode_raw_roundtrips() {
        for raw in [0u32, 2, 3, 4, 5] {
            assert_eq!(StereoMode::from_raw(raw).unwrap().raw(), raw);
        }
    }

    #[test]
    fn stereo_mode_is_stereo_flag() {
        assert!(!StereoMode::Mono.is_stereo());
        for v in 2u8..=5 {
            assert!(StereoMode::Stereo(v).is_stereo());
        }
    }

    #[test]
    fn coupling_mode_splits_zero_and_nonzero() {
        assert_eq!(CouplingMode::from_raw(0), CouplingMode::None);
        for raw in [1u32, 2, 5, 6, 8, 17, 19, 0xffff_ffff] {
            assert_eq!(CouplingMode::from_raw(raw), CouplingMode::Coupled(raw));
        }
    }

    #[test]
    fn coupling_mode_raw_and_flags() {
        assert_eq!(CouplingMode::None.raw(), 0);
        assert!(!CouplingMode::None.is_coupled());
        for raw in [1u32, 2, 19] {
            let c = CouplingMode::from_raw(raw);
            assert_eq!(c.raw(), raw);
            assert!(c.is_coupled());
        }
    }

    #[test]
    fn record_classifiers_agree_with_raw_fields() {
        // Every vendored record classifies without error, and the typed
        // view round-trips back to the raw field value.
        for (idx, rec) in iter_flavor_records() {
            let cm = rec.coupling_mode_class();
            assert_eq!(cm.raw(), rec.coupling_mode, "coupling raw mismatch @ {idx}");
            assert_eq!(cm.is_coupled(), rec.is_coupled());
            assert_eq!(cm.is_coupled(), rec.coupling_mode != 0);

            let sm = rec
                .stereo_mode_class()
                .unwrap_or_else(|e| panic!("record {idx} stereo mode out of spec: {e:?}"));
            assert_eq!(sm.raw(), rec.stereo_mode, "stereo raw mismatch @ {idx}");
            assert_eq!(sm.is_stereo(), rec.is_stereo());
            assert_eq!(sm.is_stereo(), (2..=5).contains(&rec.stereo_mode));
        }
    }

    #[test]
    fn mono_records_are_uncoupled_mono() {
        // Records 1..=8 (and others) are plain mono presets: coupling 0,
        // stereo 0, channels 1.
        let r1 = flavor_record(1).unwrap();
        assert_eq!(r1.channels, 1);
        assert_eq!(r1.coupling_mode_class(), CouplingMode::None);
        assert_eq!(r1.stereo_mode_class().unwrap(), StereoMode::Mono);
        assert!(!r1.is_coupled());
        assert!(!r1.is_stereo());
    }

    #[test]
    fn record_21_is_coupled_stereo() {
        // The real-stream flavor: coupling_mode 2, stereo_mode 4 (spec/02
        // §2.1 validated cookie). Coupled stereo.
        let r = flavor_record(21).unwrap();
        assert_eq!(r.coupling_mode_class(), CouplingMode::Coupled(2));
        assert_eq!(r.stereo_mode_class().unwrap(), StereoMode::Stereo(4));
        assert!(r.is_coupled());
        assert!(r.is_stereo());
        assert_eq!(r.channels, 2);
    }

    #[test]
    fn sentinel_record_30_classifies() {
        // Sentinel (17, 5, …): coupling 17 (coupled), stereo 5.
        let s = flavor_record(FLAVOR_COUNT - 1).unwrap();
        assert_eq!(s.coupling_mode_class(), CouplingMode::Coupled(17));
        assert_eq!(s.stereo_mode_class().unwrap(), StereoMode::Stereo(5));
    }

    #[test]
    fn coupling_and_stereo_mode_classifications_match_the_table() {
        // Empirical cross-check against the vendored table, pinning the
        // two facts the data exhibits without over-claiming:
        //
        // (1) The selectors are NOT redundant with `channels`: the
        //     leading record (index 0) is `(coupling 1, stereo 0,
        //     channels 1)` — a coupled selector on a single-channel
        //     mono flavor — and the index-30 sentinel is `(coupling 17,
        //     stereo 5, channels 1)`, coupled+stereo-mode yet
        //     single-channel. So neither flag may be derived from
        //     `channels`.
        let r0 = flavor_record(0).unwrap();
        assert!(r0.is_coupled(), "record 0 carries coupling-region 1");
        assert!(!r0.is_stereo(), "record 0 is stereo-mode 0 (mono)");
        assert_eq!(r0.channels, 1);

        // (2) Across the whole table, every record carrying a stereo
        //     mode (`2..=5`) is also coupled (non-zero region) — but the
        //     converse fails (record 0 is coupled with stereo-mode 0),
        //     so the two classifiers are genuinely distinct views and
        //     `is_stereo()` is read from its own field, never inferred
        //     from `is_coupled()`.
        let mut coupled_mono_seen = false;
        for (idx, rec) in iter_flavor_records() {
            if rec.is_stereo() {
                assert!(
                    rec.is_coupled(),
                    "stereo record {idx} should also be coupled: {rec:?}"
                );
            }
            if rec.is_coupled() && !rec.is_stereo() {
                coupled_mono_seen = true;
            }
        }
        assert!(
            coupled_mono_seen,
            "expected at least one coupled record with stereo-mode 0 (e.g. index 0)"
        );

        // And the classic pairing: coupled AND stereo (record 21).
        let r21 = flavor_record(21).unwrap();
        assert!(r21.is_coupled() && r21.is_stereo());
    }

    // ---- §4.3 recovered rotation table ------------------------------

    #[test]
    fn coupling_table_has_the_recovered_width() {
        assert_eq!(COUPLING_RECOVERED_LEN, 512);
        assert_eq!(
            coupling_coefficient_table().len(),
            COUPLING_RECOVERED_LEN as usize
        );
        assert_eq!(
            crate::spectral::coupling_table_len(COUPLING_RECOVERED_BITS),
            COUPLING_RECOVERED_LEN
        );
    }

    #[test]
    fn depermuted_table_is_a_cosine_then_sine_ramp() {
        // Numeric structure of the recovered values (no formula was
        // used to build them): coef[0] = 1, the first half decreases
        // monotonically from +1 toward −1 (a cosine sweep), and the
        // second half opens at 0 (coef[256] = sin 0).
        let t = coupling_coefficient_table();
        assert_eq!(t[0], 1.0);
        for j in 1..256usize {
            assert!(
                t[j] < t[j - 1],
                "cos half must decrease at {j}: {} !< {}",
                t[j],
                t[j - 1]
            );
        }
        assert!(t[255] < -0.999, "cos half must approach -1");
        assert!(t[256].abs() < 1e-6, "sine half must open at 0");
    }

    #[test]
    fn mirror_pan_pairs_conserve_energy() {
        // The §4.2 pair (coef[j], coef[Ncoup-1-j]) behaves as a
        // (cos θ, sin θ) pan: its energy stays within ~1.3% of unity
        // across the whole recovered table (the residual is the
        // half-step phase offset between the two halves).
        for j in 0..COUPLING_RECOVERED_LEN {
            let (a, b) = coupling_pan_pair(j).unwrap();
            let e = a * a + b * b;
            assert!(
                (e - 1.0).abs() < 0.02,
                "pan energy at j={j} is {e}, expected ~1"
            );
        }
    }

    #[test]
    fn pan_endpoints_steer_fully() {
        // j = 0: all energy to channel 0; j = Ncoup-1: all to channel 1.
        let (a0, b0) = coupling_pan_pair(0).unwrap();
        assert_eq!(a0, 1.0);
        assert!(b0.abs() < 0.02, "j=0 partner {b0} must be ~0");
        let (a1, b1) = coupling_pan_pair(COUPLING_RECOVERED_LEN - 1).unwrap();
        assert!(
            a1.abs() < 0.02 && (b1 - 1.0).abs() < 1e-6,
            "j=Ncoup-1 pair ({a1}, {b1}) must steer to channel 1"
        );
    }

    #[test]
    fn split_coupled_recovered_matches_the_generic_split() {
        for &j in &[0u32, 1, 64, 255, 256, 300, 511] {
            let (a, b) = split_coupled_recovered(2.0, j).unwrap();
            let (ga, gb) =
                crate::spectral::split_coupled_coefficient(2.0, j, coupling_coefficient_table())
                    .unwrap();
            assert_eq!((a, b), (ga, gb));
            assert_eq!(a, 2.0 * coupling_coefficient(j).unwrap());
        }
    }

    #[test]
    fn coupling_index_out_of_range_is_rejected() {
        assert_eq!(
            coupling_coefficient(COUPLING_RECOVERED_LEN).unwrap_err(),
            Error::CouplingIndexOutOfRange {
                got: COUPLING_RECOVERED_LEN,
                ncoup: COUPLING_RECOVERED_LEN
            }
        );
        assert!(coupling_pan_pair(COUPLING_RECOVERED_LEN).is_err());
        assert!(split_coupled_recovered(1.0, COUPLING_RECOVERED_LEN).is_err());
    }
}
