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

// ---- §4.3 per-coupling-width pan-coefficient tables ------------------
//
// The per-coupling-width coefficient tables `spec/05` §4.3 once recorded
// as a runtime-built GAP are CONST `.rdata` inside the image (round 9):
// five contiguous tables at RVA `0x8d0c`, one per coupling width
// `w = 2..=6`, of length `(1 << w) - 1`, ending exactly at the dispatch
// pointer array at `0x8ee8` that indexes them by width. All 119 values
// satisfy the constant-power identity `t[j]^2 + t[n-1-j]^2 = 1` to
// better than 1e-6, each row is strictly decreasing, and each centre is
// `1/sqrt2` — the mirror-index pan law of §4.2. The identification is
// behavioural, not just structural: the range has exactly one consumer
// in the whole image (the §4.2 stereo split at `cook.dll!0x3e96`), and
// zero-filling the `coupling_bits`-selected table moves 3060/4096 PCM
// bytes of a real decoded frame while the other four rows are bit-inert
// (`tables/probe_coupling_table_ablation.js`; spec/05 §4.3,
// provenance/09 §5, provenance/10).
//
// The init-built 256-pair rotation buffer and 512-entry bit-reversal
// index the docs round 8 staged (`tables/coupling-rotation-coeffs`,
// `tables/coupling-index-permutation`) are NOT this consumer's table —
// they remain vendored through [`crate::tables`] as recovered numeric
// facts whose consuming stage is unpinned (a bit-reversal permutation
// is transform-kernel shaped; the staged §4.3 label predates the
// round-9 ablation and is flagged in the crate README).

/// `.rdata` RVA of the five concatenated §4.3 pan-coefficient tables
/// (`tables/coupling-pan-coeffs.meta`, spec/05 §4.3).
pub const COUPLING_PAN_TABLE_RVA: u32 = 0x8d0c;

/// RVA of the dispatch pointer array indexing the five tables by
/// coupling width (`[width*4 + 0x8ee8]`, spec/05 §4.3). The tables end
/// exactly here: `0x8d0c + 4 x 119 = 0x8ee8`.
pub const COUPLING_PAN_DISPATCH_RVA: u32 =
    COUPLING_PAN_TABLE_RVA + (crate::tables::COUPLING_PAN_TOTAL_LEN as u32) * 4;

/// Typed selector over the five coupling-index bit widths the binary
/// stores a §4.3 pan table for (`w = 2..=6`; spec/05 §4.3).
///
/// The validated `FUN_RM_32.rm` stream selects `w = 4`
/// (`coupling_bits = 4`; provenance/09 §2), the only row with
/// behavioural evidence of being consumed — the other four are staged
/// as extracted bytes reachable by the same dispatch code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CouplingPanWidth {
    /// Coupling width 2 — 3-entry table at `0x8d0c`.
    W2,
    /// Coupling width 3 — 7-entry table at `0x8d18`.
    W3,
    /// Coupling width 4 — 15-entry table at `0x8d34` (the validated
    /// stream's width).
    W4,
    /// Coupling width 5 — 31-entry table at `0x8d70`.
    W5,
    /// Coupling width 6 — 63-entry table at `0x8dec`.
    W6,
}

impl CouplingPanWidth {
    /// All five stored widths, in table (row) order.
    pub const ALL: [CouplingPanWidth; 5] = [
        CouplingPanWidth::W2,
        CouplingPanWidth::W3,
        CouplingPanWidth::W4,
        CouplingPanWidth::W5,
        CouplingPanWidth::W6,
    ];

    /// Build from the per-flavor coupling-index bit width (context
    /// `+0x1c`).
    ///
    /// # Errors
    ///
    /// [`Error::CouplingPanWidthUnsupported`] for any width outside
    /// `2..=6` — the dispatch array's slots 0/1 are NULL and it has no
    /// slot past 6.
    pub fn from_bits(coupling_bits: u32) -> Result<Self, Error> {
        match coupling_bits {
            2 => Ok(CouplingPanWidth::W2),
            3 => Ok(CouplingPanWidth::W3),
            4 => Ok(CouplingPanWidth::W4),
            5 => Ok(CouplingPanWidth::W5),
            6 => Ok(CouplingPanWidth::W6),
            other => Err(Error::CouplingPanWidthUnsupported { got: other }),
        }
    }

    /// The coupling-index bit width `w`.
    #[must_use]
    pub const fn bits(self) -> u32 {
        match self {
            CouplingPanWidth::W2 => 2,
            CouplingPanWidth::W3 => 3,
            CouplingPanWidth::W4 => 4,
            CouplingPanWidth::W5 => 5,
            CouplingPanWidth::W6 => 6,
        }
    }

    /// Row index of this width's table inside the five-row vendored
    /// concatenation.
    #[must_use]
    pub const fn row_index(self) -> usize {
        (self.bits() - 2) as usize
    }

    /// The table length `Ncoup = (1 << w) - 1` — one less than the
    /// power of two, matching the observed coupling-index range
    /// (`0..=14` on the `w = 4` stream; provenance/09 §2).
    #[must_use]
    pub const fn table_len(self) -> u32 {
        (1 << self.bits()) - 1
    }

    /// RVA of this width's table head (pure RVA arithmetic from the
    /// `.meta` table head; `0x8d0c / 0x8d18 / 0x8d34 / 0x8d70 /
    /// 0x8dec`).
    #[must_use]
    pub const fn rva(self) -> u32 {
        let mut off = 0u32;
        let mut i = 0usize;
        while i < self.row_index() {
            off += (crate::tables::COUPLING_PAN_ROW_LENS[i] as u32) * 4;
            i += 1;
        }
        COUPLING_PAN_TABLE_RVA + off
    }
}

/// The §4.3 pan-coefficient table for one coupling width
/// (`tables/coupling-pan-coeffs.csv`, row selected as the binary's
/// `[width*4 + 0x8ee8]` dispatch does).
///
/// The returned slice has exactly [`CouplingPanWidth::table_len`]
/// elements, strictly decreasing with `1/sqrt2` at its centre.
#[must_use]
pub fn coupling_pan_table(width: CouplingPanWidth) -> &'static [f32] {
    crate::tables::coupling_pan_coeffs()[width.row_index()]
}

/// The §4.2 mirror pan pair `(t[j], t[Ncoup-1-j])` for one coupling
/// index — channel 0's and channel 1's factors.
///
/// # Errors
///
/// [`Error::CouplingIndexOutOfRange`] when `j >= Ncoup`.
pub fn coupling_pan_pair(width: CouplingPanWidth, j: u32) -> Result<(f32, f32), Error> {
    let t = coupling_pan_table(width);
    let partner = crate::spectral::mirror_partner_index(j, width.table_len())?;
    Ok((t[j as usize], t[partner as usize]))
}

/// Split one coupled coefficient by the §4.2 mirror rotation over the
/// vendored §4.3 pan table for `width` —
/// [`crate::spectral::split_coupled_coefficient`] with the extracted
/// values instead of a caller-supplied slice.
///
/// # Errors
///
/// [`Error::CouplingIndexOutOfRange`] when `j >= Ncoup`.
pub fn split_coupled_recovered(
    c: f32,
    width: CouplingPanWidth,
    j: u32,
) -> Result<(f32, f32), Error> {
    crate::spectral::split_coupled_coefficient(c, j, coupling_pan_table(width))
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

    // ---- §4.3 per-coupling-width pan tables --------------------------

    #[test]
    fn pan_widths_cover_the_dispatch_array() {
        for (i, w) in CouplingPanWidth::ALL.iter().enumerate() {
            assert_eq!(w.row_index(), i);
            assert_eq!(w.bits(), (i + 2) as u32);
            assert_eq!(w.table_len(), (1 << w.bits()) - 1);
            assert_eq!(coupling_pan_table(*w).len() as u32, w.table_len());
            assert_eq!(CouplingPanWidth::from_bits(w.bits()).unwrap(), *w);
        }
        // spec/05 §4.3 row heads: 0x8d0c / 0x8d18 / 0x8d34 / 0x8d70 / 0x8dec.
        let rvas: Vec<u32> = CouplingPanWidth::ALL.iter().map(|w| w.rva()).collect();
        assert_eq!(rvas, [0x8d0c, 0x8d18, 0x8d34, 0x8d70, 0x8dec]);
        // The five tables end exactly at the dispatch pointer array.
        assert_eq!(COUPLING_PAN_DISPATCH_RVA, 0x8ee8);
        // Widths outside 2..=6 have no table (dispatch slots 0/1 NULL).
        for bad in [0u32, 1, 7, 32] {
            assert_eq!(
                CouplingPanWidth::from_bits(bad).unwrap_err(),
                Error::CouplingPanWidthUnsupported { got: bad }
            );
        }
    }

    #[test]
    fn pan_pairs_conserve_power_and_steer_across_the_table() {
        for w in CouplingPanWidth::ALL {
            let n = w.table_len();
            for j in 0..n {
                let (a, b) = coupling_pan_pair(w, j).unwrap();
                let e = f64::from(a) * f64::from(a) + f64::from(b) * f64::from(b);
                assert!(
                    (e - 1.0).abs() < 1e-6,
                    "width {w:?} j={j}: pan energy {e} != 1"
                );
            }
            // j = 0 steers most of the energy to channel 0; the last
            // index mirrors it to channel 1; the centre index splits
            // evenly (both factors 1/sqrt2).
            let (a0, b0) = coupling_pan_pair(w, 0).unwrap();
            assert!(a0 > b0, "width {w:?}: j=0 must favour channel 0");
            let (al, bl) = coupling_pan_pair(w, n - 1).unwrap();
            assert_eq!((al, bl), (b0, a0), "mirror symmetry at the ends");
            let (ac, bc) = coupling_pan_pair(w, n / 2).unwrap();
            assert_eq!(ac, bc, "centre index must split evenly");
            assert!((ac - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        }
    }

    #[test]
    fn split_coupled_recovered_matches_the_generic_split() {
        let w = CouplingPanWidth::W4; // the validated stream's width
        for j in 0..w.table_len() {
            let (a, b) = split_coupled_recovered(2.0, w, j).unwrap();
            let (ga, gb) =
                crate::spectral::split_coupled_coefficient(2.0, j, coupling_pan_table(w)).unwrap();
            assert_eq!((a, b), (ga, gb));
        }
    }

    #[test]
    fn coupling_index_out_of_range_is_rejected() {
        let w = CouplingPanWidth::W4;
        assert_eq!(
            coupling_pan_pair(w, w.table_len()).unwrap_err(),
            Error::CouplingIndexOutOfRange {
                got: w.table_len(),
                ncoup: w.table_len()
            }
        );
        assert!(split_coupled_recovered(1.0, w, w.table_len()).is_err());
    }

    // ---- init-built rotation buffers (consuming stage unpinned) ------

    #[test]
    fn init_rotation_buffers_depermute_to_a_quarter_turn_sweep() {
        // The round-8 init drive recovered 256 unit-circle (cos, sin)
        // pairs plus a 512-entry bit-reversal index. De-permuted
        // (flat[perm[j]]), the values are the quarter-turn sweep
        // cos(j*pi/256) for j < 256 then sin(r*pi/256) — a transform-
        // kernel-shaped rotation ramp. The §4.2 stereo split does NOT
        // read these buffers (the round-9 ablation pinned its table to
        // the `.rdata` pan rows above); their consuming stage stays
        // unpinned, so the structure is validated here as extracted
        // numeric fact only.
        let pairs = crate::tables::coupling_rotation_coeffs();
        let perm = crate::tables::coupling_index_permutation();
        let flat: Vec<f32> = pairs.iter().flat_map(|p| p.iter().copied()).collect();
        let t: Vec<f32> = perm.iter().map(|&s| flat[s as usize]).collect();
        assert_eq!(t.len(), 512);
        for (j, &tj) in t[..256].iter().enumerate() {
            let want = (std::f64::consts::PI * j as f64 / 256.0).cos() as f32;
            assert!((tj - want).abs() < 1e-6, "slot {j}: {tj} vs cos {want}");
        }
        for (r, &tr) in t[256..].iter().enumerate() {
            let want = (std::f64::consts::PI * r as f64 / 256.0).sin() as f32;
            assert!((tr - want).abs() < 1e-6, "slot 256+{r}: {tr} vs sin {want}");
        }
    }
}
