//! `RAGetFlavorProperty` property-ID dispatch surface.
//!
//! Source-of-truth:
//! `docs/audio/cook/spec/01-cook-decoder-structure.md` §4.2 (*"Property
//! IDs"*) and
//! `docs/audio/cook/spec/02-cook-flavor-and-extradata-layout.md` §1.2
//! (*"The flavor property-descriptor region"*), cross-checked by
//! `docs/audio/cook/provenance/03-cook-audit.md` audit point #13
//! (*"`RAGetFlavorProperty` 21-entry (0–20) jump table at `0x1be8`;
//! string props (cases 0/4/7) via strlen"*, **CONFIRMED** — *"`0x17a0`
//! MSVC jump table `table_va=0x1be8`, 21 cases"*).
//!
//! `RAGetFlavorProperty(flavor, property_id, out_ptr, out_len_ptr)`
//! (export ordinal 10, worker `cook.dll!0x1660`→`0x17a0`) dispatches on
//! `property_id` through an MSVC jump table at RVA `0x1be8` with **21
//! cases (IDs 0–20)**. Each case returns one property and writes its
//! byte length through `out_len_ptr`:
//!
//! - **IDs 0, 4, 7** return a pointer to a **NUL-terminated string**
//!   (the codec-family name + human-readable description strings of
//!   spec/01 §4.3); the byte length is computed at run time with a
//!   `strlen`, so it is not a fixed value.
//! - **Every other ID (1, 2, 3, 5, 6, 8..=20)** returns a **32-bit
//!   integer** scalar (rate / channels / frame sizes / bitrate); the
//!   returned length is the fixed value `4`.
//!
//! ## What this module does *not* cover
//!
//! What spec/01 §4.2 establishes is the **ID range (0–20)** and the
//! **return *kind*** (string vs 32-bit integer) per ID. The full
//! property-ID → *meaning* enumeration (which integer is the sample
//! rate, which the channel count, etc.) and the property-descriptor
//! structure's stride / field layout are an explicit **GAP** (spec/01
//! §4.2: *"The full property-ID → meaning enumeration is a GAP"*;
//! spec/02 §1.2: *"The exact stride and full field layout … are a
//! GAP"*). This module types only the dispatch surface that is pinned.

/// RVA of the MSVC jump table the flavor-property worker `0x17a0`
/// dispatches through (spec/01 §4.2 / audit #13: *"21-entry jump table
/// (cases 0–20, table at RVA `0x1be8`)"*).
pub const FLAVOR_PROPERTY_JUMP_TABLE_RVA: u32 = 0x1be8;

/// Number of cases in the flavor-property jump table — property IDs
/// `0..=20` (spec/01 §4.2: *"a 21-entry jump table (cases 0–20)"*).
pub const FLAVOR_PROPERTY_ID_COUNT: u8 = 21;

/// Largest valid flavor-property ID
/// ([`FLAVOR_PROPERTY_ID_COUNT`] − 1 = 20).
pub const MAX_FLAVOR_PROPERTY_ID: u8 = FLAVOR_PROPERTY_ID_COUNT - 1;

/// Fixed byte length the integer-property cases write through
/// `out_len_ptr` (spec/01 §4.2: *"a returned length of 4 marks a 32-bit
/// integer property"*).
pub const FLAVOR_PROPERTY_INTEGER_LEN: u32 = 4;

/// The kind of value a [`FlavorPropertyId`] case returns.
///
/// spec/01 §4.2 pins the dispatch into exactly two shapes: a fixed-width
/// 32-bit integer (returned length `4`), or a NUL-terminated string
/// whose length the worker computes with a `strlen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlavorPropertyKind {
    /// A 32-bit integer scalar. The worker writes the fixed length
    /// [`FLAVOR_PROPERTY_INTEGER_LEN`] (4) through `out_len_ptr`.
    Integer,
    /// A pointer to a NUL-terminated string (codec-family name /
    /// description, spec/01 §4.3). The worker computes the length with a
    /// `strlen`, so it is not a fixed value (cases 0, 4, 7).
    String,
}

impl FlavorPropertyKind {
    /// The fixed returned length for this kind, if it has one.
    ///
    /// [`FlavorPropertyKind::Integer`] always reports length
    /// [`FLAVOR_PROPERTY_INTEGER_LEN`] (4);
    /// [`FlavorPropertyKind::String`] reports `None` because its length
    /// is computed at run time with a `strlen` (spec/01 §4.2).
    pub const fn fixed_len(self) -> Option<u32> {
        match self {
            FlavorPropertyKind::Integer => Some(FLAVOR_PROPERTY_INTEGER_LEN),
            FlavorPropertyKind::String => None,
        }
    }
}

/// A validated flavor-property ID in the range `0..=20` the
/// `RAGetFlavorProperty` jump table covers (spec/01 §4.2).
///
/// Construct via [`FlavorPropertyId::new`]; the inner value is reachable
/// through [`FlavorPropertyId::get`]. Out-of-range IDs are rejected with
/// [`crate::Error::FlavorPropertyIdOutOfRange`] — the worker `0x17a0`
/// bounds the index against the same 21-case table before dispatching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlavorPropertyId(u8);

impl FlavorPropertyId {
    /// Construct a validated property ID, rejecting any value
    /// `> [MAX_FLAVOR_PROPERTY_ID]` (= 20).
    pub const fn new(id: u8) -> crate::Result<Self> {
        if id > MAX_FLAVOR_PROPERTY_ID {
            Err(crate::Error::FlavorPropertyIdOutOfRange { got: id })
        } else {
            Ok(FlavorPropertyId(id))
        }
    }

    /// The validated property-ID value (`0..=20`).
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The return *kind* of this property ID.
    ///
    /// spec/01 §4.2 / audit #13: cases **0, 4, 7** return a
    /// NUL-terminated string ([`FlavorPropertyKind::String`]); every
    /// other case returns a 32-bit integer
    /// ([`FlavorPropertyKind::Integer`]).
    pub const fn kind(self) -> FlavorPropertyKind {
        match self.0 {
            0 | 4 | 7 => FlavorPropertyKind::String,
            _ => FlavorPropertyKind::Integer,
        }
    }

    /// Whether this property ID returns a NUL-terminated string
    /// (cases 0, 4, 7).
    pub const fn is_string(self) -> bool {
        matches!(self.kind(), FlavorPropertyKind::String)
    }

    /// Whether this property ID returns a 32-bit integer scalar
    /// (every ID except 0, 4, 7).
    pub const fn is_integer(self) -> bool {
        matches!(self.kind(), FlavorPropertyKind::Integer)
    }

    /// The fixed byte length this property writes through `out_len_ptr`,
    /// or `None` for the string cases (whose length is a run-time
    /// `strlen`). Delegates to [`FlavorPropertyKind::fixed_len`].
    pub const fn fixed_len(self) -> Option<u32> {
        self.kind().fixed_len()
    }
}

/// The three flavor-property IDs that return a NUL-terminated string,
/// in ascending order (spec/01 §4.2 / audit #13: *"cases 0, 4, 7"*).
pub const STRING_PROPERTY_IDS: [u8; 3] = [0, 4, 7];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jump_table_has_twenty_one_cases() {
        // spec/01 §4.2: "a 21-entry jump table (cases 0–20, table at
        // RVA 0x1be8)".
        assert_eq!(FLAVOR_PROPERTY_ID_COUNT, 21);
        assert_eq!(MAX_FLAVOR_PROPERTY_ID, 20);
        assert_eq!(FLAVOR_PROPERTY_JUMP_TABLE_RVA, 0x1be8);
    }

    #[test]
    fn jump_table_rva_lies_in_text_section() {
        // spec/01 §1: .text spans RVA 0x1000..0x7c3c.
        assert!((0x1000..0x7c3c).contains(&FLAVOR_PROPERTY_JUMP_TABLE_RVA));
    }

    #[test]
    fn ids_zero_through_twenty_are_constructible() {
        for id in 0..=MAX_FLAVOR_PROPERTY_ID {
            let p = FlavorPropertyId::new(id).expect("0..=20 valid");
            assert_eq!(p.get(), id);
        }
    }

    #[test]
    fn id_twenty_one_and_above_is_rejected() {
        for id in [21u8, 22, 50, 255] {
            assert_eq!(
                FlavorPropertyId::new(id),
                Err(crate::Error::FlavorPropertyIdOutOfRange { got: id })
            );
        }
    }

    #[test]
    fn cases_zero_four_seven_are_strings() {
        // spec/01 §4.2 / audit #13: "cases 0, 4, 7 compute the length
        // with a strlen and return a pointer to a NUL-terminated string".
        for id in STRING_PROPERTY_IDS {
            let p = FlavorPropertyId::new(id).unwrap();
            assert!(p.is_string(), "id {id} should be a string property");
            assert!(!p.is_integer());
            assert_eq!(p.kind(), FlavorPropertyKind::String);
            assert_eq!(p.fixed_len(), None);
        }
    }

    #[test]
    fn every_other_case_is_a_length_four_integer() {
        // spec/01 §4.2: "a returned length of 4 marks a 32-bit integer
        // property; the rest return 32-bit integers".
        for id in 0..=MAX_FLAVOR_PROPERTY_ID {
            if STRING_PROPERTY_IDS.contains(&id) {
                continue;
            }
            let p = FlavorPropertyId::new(id).unwrap();
            assert!(p.is_integer(), "id {id} should be an integer property");
            assert!(!p.is_string());
            assert_eq!(p.kind(), FlavorPropertyKind::Integer);
            assert_eq!(p.fixed_len(), Some(FLAVOR_PROPERTY_INTEGER_LEN));
            assert_eq!(p.fixed_len(), Some(4));
        }
    }

    #[test]
    fn exactly_three_of_the_twenty_one_cases_are_strings() {
        let string_count = (0..=MAX_FLAVOR_PROPERTY_ID)
            .filter(|&id| FlavorPropertyId::new(id).unwrap().is_string())
            .count();
        assert_eq!(string_count, STRING_PROPERTY_IDS.len());
        assert_eq!(string_count, 3);
        // The other 18 are integers.
        let integer_count = (0..=MAX_FLAVOR_PROPERTY_ID)
            .filter(|&id| FlavorPropertyId::new(id).unwrap().is_integer())
            .count();
        assert_eq!(integer_count, 18);
        assert_eq!(
            string_count + integer_count,
            FLAVOR_PROPERTY_ID_COUNT as usize
        );
    }

    #[test]
    fn string_property_ids_are_ascending_and_in_range() {
        let mut prev: Option<u8> = None;
        for id in STRING_PROPERTY_IDS {
            assert!(id <= MAX_FLAVOR_PROPERTY_ID);
            if let Some(p) = prev {
                assert!(id > p, "STRING_PROPERTY_IDS must be ascending");
            }
            prev = Some(id);
        }
    }

    #[test]
    fn kind_fixed_len_is_consistent_with_id_fixed_len() {
        for id in 0..=MAX_FLAVOR_PROPERTY_ID {
            let p = FlavorPropertyId::new(id).unwrap();
            assert_eq!(p.fixed_len(), p.kind().fixed_len());
        }
    }
}
