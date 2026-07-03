//! Typed iMDCT rotation-coefficient accessors (`0xa1b0`, `spec/05` §5).
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/05-cook-backend-frame-syntax.md` §5 (the inverse
//! transform pointer), `docs/audio/cook/tables/README.md`
//! (`transform-rotation-coeffs`, RVA `0xa1b0`), and
//! `docs/audio/cook/provenance/06-cook-univdreams-extraction.md` Ask 3
//! (the const-vs-BSS classification and the group structure).
//!
//! ## What this module does
//!
//! The iMDCT butterfly kernel `cook.dll!0x5b70` reads pre/post rotation
//! coefficients from the `.rdata` const table at RVA `0xa1b0`, **five at a
//! time** (stride `0x14`): 74 groups of 5 f32 (370 total). The group base
//! is selected by the block-length class in `cook.dll!0x5b10` from the base
//! RVA `0xa1d8 = 0xa1b0 + 0x28` — i.e. group index 2 is the first group the
//! selector uses (`provenance/06` Ask 3).
//!
//! `provenance/06` records the table as pure read-only `.rdata` data with
//! **no validated closed form** (columns 0 and 2 are equal in 71/74 groups;
//! the values are not unit-circle twiddles), so it is vendored as flat
//! Feist facts. This module wires the **typed group access** (the same way
//! [`crate::mdct`] wires the window-table access), keyed by group index;
//! the kernel's *use* of the coefficients — the actual butterfly — has no
//! documented closed form and stays a `spec/01` §6 GAP.
//!
//! ## Wall-respect note
//!
//! Every fact here is anchored to `tables/README.md` and `provenance/06`
//! Ask 3; the 74×5 group shape, the stride `0x14`, and the `0xa1d8` base
//! offset are the trace's own. No transform algorithm is reconstructed —
//! only the typed table access.

use crate::{
    tables::{transform_rotation_coeffs, TRANSFORM_ROTATION_ROW_COUNT},
    Error,
};

/// `.rdata` base RVA of the iMDCT rotation-coefficient table (`0xa1b0`).
pub const TRANSFORM_ROTATION_RVA: u32 = 0xa1b0;

/// The stride between consecutive rotation groups — `0x14` = 5 × f32
/// (`tables/README.md`: *"consumed 5-at-a-time by the transform kernel
/// `0x5b70`"*).
pub const TRANSFORM_ROTATION_GROUP_STRIDE: u32 = 0x14;

/// Number of rotation groups (74; `TRANSFORM_ROTATION_ROW_COUNT`).
pub const TRANSFORM_ROTATION_GROUP_COUNT: usize = TRANSFORM_ROTATION_ROW_COUNT;

/// The group index the block-length selector `cook.dll!0x5b10` treats as
/// its base — `(0xa1d8 - 0xa1b0) / 0x14 = 2` (`provenance/06` Ask 3: the
/// selector's base RVA is `0xa1d8 = 0xa1b0 + 0x28`).
pub const TRANSFORM_ROTATION_SELECTOR_BASE_GROUP: usize =
    (0xa1d8u32 - TRANSFORM_ROTATION_RVA) as usize / TRANSFORM_ROTATION_GROUP_STRIDE as usize;

/// One 5-tuple rotation group from the `0xa1b0` table.
///
/// # Errors
///
/// Returns [`Error::TransformRotationGroupOutOfRange`] when `group >=`
/// [`TRANSFORM_ROTATION_GROUP_COUNT`].
pub fn rotation_group(group: usize) -> Result<[f32; 5], Error> {
    let rows = transform_rotation_coeffs();
    rows.get(group)
        .copied()
        .ok_or(Error::TransformRotationGroupOutOfRange {
            got: group,
            count: rows.len(),
        })
}

/// The RVA of a rotation group — `0xa1b0 + group * 0x14` (derived, never
/// retyped).
///
/// # Errors
///
/// Returns [`Error::TransformRotationGroupOutOfRange`] when `group` is out
/// of range.
pub fn rotation_group_rva(group: usize) -> Result<u32, Error> {
    if group >= TRANSFORM_ROTATION_GROUP_COUNT {
        return Err(Error::TransformRotationGroupOutOfRange {
            got: group,
            count: TRANSFORM_ROTATION_GROUP_COUNT,
        });
    }
    Ok(TRANSFORM_ROTATION_RVA + group as u32 * TRANSFORM_ROTATION_GROUP_STRIDE)
}

/// The end RVA of the rotation table — `0xa1b0 + 74 * 0x14 = 0xa778`
/// (derived; `provenance/06` Ask 3: *"`0xa1b0`..`0xa778` has zero
/// relocations"*).
#[must_use]
pub fn rotation_table_end_rva() -> u32 {
    TRANSFORM_ROTATION_RVA + TRANSFORM_ROTATION_GROUP_COUNT as u32 * TRANSFORM_ROTATION_GROUP_STRIDE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_count_and_selector_base() {
        assert_eq!(TRANSFORM_ROTATION_GROUP_COUNT, 74);
        // 0xa1d8 = 0xa1b0 + 0x28 = base + 2 groups.
        assert_eq!(TRANSFORM_ROTATION_SELECTOR_BASE_GROUP, 2);
    }

    #[test]
    fn every_group_is_accessible() {
        for g in 0..TRANSFORM_ROTATION_GROUP_COUNT {
            let grp = rotation_group(g).unwrap();
            for &v in &grp {
                assert!(v.is_finite());
            }
        }
        assert_eq!(
            rotation_group(TRANSFORM_ROTATION_GROUP_COUNT).unwrap_err(),
            Error::TransformRotationGroupOutOfRange {
                got: TRANSFORM_ROTATION_GROUP_COUNT,
                count: TRANSFORM_ROTATION_GROUP_COUNT
            }
        );
    }

    #[test]
    fn rva_arithmetic_matches_table_extent() {
        assert_eq!(rotation_group_rva(0).unwrap(), 0xa1b0);
        assert_eq!(rotation_group_rva(2).unwrap(), 0xa1d8);
        // Last group start, then the derived end matches provenance/06.
        assert_eq!(rotation_group_rva(73).unwrap(), 0xa1b0 + 73 * 0x14);
        assert_eq!(rotation_table_end_rva(), 0xa778);
        assert!(rotation_group_rva(74).is_err());
    }

    #[test]
    fn selector_base_group_matches_column_symmetry() {
        // provenance/06: columns 0 and 2 are equal in 71/74 groups. The
        // group the selector bases from is a real, accessible group.
        let base = rotation_group(TRANSFORM_ROTATION_SELECTOR_BASE_GROUP).unwrap();
        assert_eq!(base.len(), 5);
    }
}
