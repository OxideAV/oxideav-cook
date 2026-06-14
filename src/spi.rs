//! RealAudio codec **service-provider interface** (SPI) export surface.
//!
//! Source-of-truth: `docs/audio/cook/spec/01-cook-decoder-structure.md`
//! §2 (*"Export surface — the RealAudio codec SPI"*), cross-checked by
//! `docs/audio/cook/provenance/03-cook-audit.md` audit points #1
//! (*"20 named exports, ordinals 1–20, names + RVAs as in spec/01 §2 …
//! all 20 ordinal/name/RVA triples match exactly"*, **CONFIRMED** by a
//! hand-parse of the PE export directory at RVA `0xaa30`), #2
//! (`RAGetNumberOfFlavors` = 15, `RAGetNumberOfFlavors2` = 34, both
//! hardcoded immediates) and #3 (`RASetFlavor` validates index `< 0x22`,
//! stores at context `+0x28`).
//!
//! `cook.dll` exports the standard RealAudio codec SPI: 20 named
//! functions, ordinals 1–20. Each is a thin `stdcall` front-end that
//! takes a codec-context handle as its first argument, validates that it
//! is non-NULL, and dispatches into an internal worker, returning
//! Win32-style `HRESULT` codes. This module gives a typed, exhaustive
//! map of that surface — every ordinal, its export name, its front-end
//! RVA, and the role spec/01 §2 pins — plus the documented `HRESULT`
//! return codes the front-ends produce.
//!
//! ## What this module does *not* cover
//!
//! These are the **export-level** facts: the names, ordinals, front-end
//! RVAs, and the SPI's `HRESULT` contract. It does not model the worker
//! bodies behind each export (the decode pipeline lives in [`crate::driver`]
//! and the other decode modules) nor the encoder exports' internals
//! (recorded only at the export level in spec/01 §2, otherwise a GAP).

/// Number of named exports in the RealAudio codec SPI
/// (spec/01 §2: *"`cook.dll` exports 20 named functions, ordinals
/// 1–20"*).
pub const SPI_EXPORT_COUNT: usize = 20;

/// `HRESULT` `S_OK` — success (spec/01 §2: *"`0` = S_OK"*).
pub const S_OK: u32 = 0x0000_0000;

/// `HRESULT` `E_INVALIDARG` — returned by an export front-end when the
/// codec-context handle argument is NULL (spec/01 §2: *"`0x80070057` =
/// E_INVALIDARG on a NULL handle"*).
pub const E_INVALIDARG: u32 = 0x8007_0057;

/// `HRESULT` `E_NOTIMPL` — returned by the stub exports
/// [`SpiExport::RAGetBackend`], [`SpiExport::RAGetDecoderBackendGUID`]
/// (shared body) and [`SpiExport::RAGetGUID`] (worker `0x1690`) in this
/// build (spec/01 §2: *"Stub: returns `0x80004001` (E_NOTIMPL)"*).
pub const E_NOTIMPL: u32 = 0x8000_4001;

/// `HRESULT` returned by the backend factory `cook.dll!0x1c60` for an
/// unrecognised extradata selector, and by the [`SpiExport::RASetComMode`]
/// export wrapper's NULL-handle path (spec/01 §2 / §3.1:
/// *"Unrecognised selectors return `0x80040005`"*).
pub const HR_UNRECOGNISED_SELECTOR: u32 = 0x8004_0005;

/// Value returned by [`SpiExport::RAGetNumberOfFlavors`] (the legacy
/// flavor count): the worker `cook.dll!0x1620` is literally
/// `mov ax, 0x0f; ret` (audit point #2).
pub const RA_NUMBER_OF_FLAVORS: u16 = 0x0f;

/// Value returned by [`SpiExport::RAGetNumberOfFlavors2`] (the full
/// flavor count): the worker `cook.dll!0x1630` is literally
/// `mov ax, 0x22; ret`, and the same `0x22` immediate is the
/// [`SpiExport::RASetFlavor`] upper bound (audit points #2 / #3).
pub const RA_NUMBER_OF_FLAVORS2: u16 = 0x22;

/// Context offset (`+0x28`) where [`SpiExport::RASetFlavor`] stores the
/// selected flavor index after validating it `< 0x22`
/// (`cook.dll!0x1640`, audit point #3).
pub const RASETFLAVOR_CONTEXT_OFFSET: u32 = 0x28;

/// One of the 20 named exports of the RealAudio codec SPI
/// (`cook.dll`), keyed by its export name. Every variant carries its
/// ordinal, front-end RVA, and role through the inherent accessors.
///
/// Variants are listed in **ordinal order** (1–20), matching the PE
/// export directory at `cook.dll!0xaa30` (audit point #1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpiExport {
    /// Ordinal 1 — destruct + free the codec context.
    RACloseCodec,
    /// Ordinal 2 — encoder factory.
    RACreateEncoderInstance,
    /// Ordinal 3 — decode one input buffer to PCM (six arguments).
    RADecode,
    /// Ordinal 4 — encode one frame.
    RAEncode,
    /// Ordinal 5 — reset/flush the de-interleave buffer state.
    RAFlush,
    /// Ordinal 6 — tear down the decode backend.
    RAFreeDecoder,
    /// Ordinal 7 — encoder teardown.
    RAFreeEncoder,
    /// Ordinal 8 — stub: returns [`E_NOTIMPL`].
    RAGetBackend,
    /// Ordinal 9 — stub: returns [`E_NOTIMPL`] (shares the ordinal-8
    /// body).
    RAGetDecoderBackendGUID,
    /// Ordinal 10 — query a property of a flavor.
    RAGetFlavorProperty,
    /// Ordinal 11 — returns the codec GUID; worker `0x1690` returns
    /// [`E_NOTIMPL`] in this build.
    RAGetGUID,
    /// Ordinal 12 — returns [`RA_NUMBER_OF_FLAVORS2`] (34).
    RAGetNumberOfFlavors2,
    /// Ordinal 13 — returns [`RA_NUMBER_OF_FLAVORS`] (15), the legacy
    /// flavor count.
    RAGetNumberOfFlavors,
    /// Ordinal 14 — consume the decoder open configuration and build the
    /// decode backend.
    RAInitDecoder,
    /// Ordinal 15 — encoder open.
    RAInitEncoder,
    /// Ordinal 16 — variant of [`SpiExport::RAOpenCodec`].
    RAOpenCodec2,
    /// Ordinal 17 — allocate + construct a codec context.
    RAOpenCodec,
    /// Ordinal 18 — set "common mode" (gates the per-buffer XOR
    /// descramble in `RADecode`).
    RASetComMode,
    /// Ordinal 19 — select the active flavor by index (validates
    /// `< 34`, stores at context [`RASETFLAVOR_CONTEXT_OFFSET`]).
    RASetFlavor,
    /// Ordinal 20 — password / DRM hook (decorated `_RASetPwd@8`).
    RASetPwd,
}

impl SpiExport {
    /// All 20 exports, in **ordinal order** (1–20).
    pub const ALL: [SpiExport; SPI_EXPORT_COUNT] = [
        SpiExport::RACloseCodec,
        SpiExport::RACreateEncoderInstance,
        SpiExport::RADecode,
        SpiExport::RAEncode,
        SpiExport::RAFlush,
        SpiExport::RAFreeDecoder,
        SpiExport::RAFreeEncoder,
        SpiExport::RAGetBackend,
        SpiExport::RAGetDecoderBackendGUID,
        SpiExport::RAGetFlavorProperty,
        SpiExport::RAGetGUID,
        SpiExport::RAGetNumberOfFlavors2,
        SpiExport::RAGetNumberOfFlavors,
        SpiExport::RAInitDecoder,
        SpiExport::RAInitEncoder,
        SpiExport::RAOpenCodec2,
        SpiExport::RAOpenCodec,
        SpiExport::RASetComMode,
        SpiExport::RASetFlavor,
        SpiExport::RASetPwd,
    ];

    /// The export ordinal (1–20), as listed in the PE export directory
    /// `cook.dll!0xaa30` (spec/01 §2).
    pub const fn ordinal(self) -> u16 {
        match self {
            SpiExport::RACloseCodec => 1,
            SpiExport::RACreateEncoderInstance => 2,
            SpiExport::RADecode => 3,
            SpiExport::RAEncode => 4,
            SpiExport::RAFlush => 5,
            SpiExport::RAFreeDecoder => 6,
            SpiExport::RAFreeEncoder => 7,
            SpiExport::RAGetBackend => 8,
            SpiExport::RAGetDecoderBackendGUID => 9,
            SpiExport::RAGetFlavorProperty => 10,
            SpiExport::RAGetGUID => 11,
            SpiExport::RAGetNumberOfFlavors2 => 12,
            SpiExport::RAGetNumberOfFlavors => 13,
            SpiExport::RAInitDecoder => 14,
            SpiExport::RAInitEncoder => 15,
            SpiExport::RAOpenCodec2 => 16,
            SpiExport::RAOpenCodec => 17,
            SpiExport::RASetComMode => 18,
            SpiExport::RASetFlavor => 19,
            SpiExport::RASetPwd => 20,
        }
    }

    /// The export name string as it appears in the PE export directory
    /// (spec/01 §2). The DRM hook is exported under its decorated
    /// `stdcall` name `_RASetPwd@8`.
    pub const fn name(self) -> &'static str {
        match self {
            SpiExport::RACloseCodec => "RACloseCodec",
            SpiExport::RACreateEncoderInstance => "RACreateEncoderInstance",
            SpiExport::RADecode => "RADecode",
            SpiExport::RAEncode => "RAEncode",
            SpiExport::RAFlush => "RAFlush",
            SpiExport::RAFreeDecoder => "RAFreeDecoder",
            SpiExport::RAFreeEncoder => "RAFreeEncoder",
            SpiExport::RAGetBackend => "RAGetBackend",
            SpiExport::RAGetDecoderBackendGUID => "RAGetDecoderBackendGUID",
            SpiExport::RAGetFlavorProperty => "RAGetFlavorProperty",
            SpiExport::RAGetGUID => "RAGetGUID",
            SpiExport::RAGetNumberOfFlavors2 => "RAGetNumberOfFlavors2",
            SpiExport::RAGetNumberOfFlavors => "RAGetNumberOfFlavors",
            SpiExport::RAInitDecoder => "RAInitDecoder",
            SpiExport::RAInitEncoder => "RAInitEncoder",
            SpiExport::RAOpenCodec2 => "RAOpenCodec2",
            SpiExport::RAOpenCodec => "RAOpenCodec",
            SpiExport::RASetComMode => "RASetComMode",
            SpiExport::RASetFlavor => "RASetFlavor",
            SpiExport::RASetPwd => "_RASetPwd@8",
        }
    }

    /// The RVA of the export's `stdcall` front-end, as pinned in the
    /// spec/01 §2 export table (relative to the `0x60bd0000` image
    /// base). Ordinals 8 and 9 share the same front-end body
    /// (`0x1210`); the GUID worker behind ordinal 11 is `0x1690`, but
    /// its front-end is at `0x1220`.
    pub const fn front_end_rva(self) -> u32 {
        match self {
            SpiExport::RAOpenCodec => 0x1010,
            SpiExport::RAOpenCodec2 => 0x1060,
            SpiExport::RACloseCodec => 0x10b0,
            SpiExport::RAGetNumberOfFlavors => 0x10e0,
            SpiExport::RAGetNumberOfFlavors2 => 0x1100,
            SpiExport::RAGetFlavorProperty => 0x1120,
            SpiExport::RASetFlavor => 0x1150,
            SpiExport::RAInitDecoder => 0x1170,
            SpiExport::RADecode => 0x1190,
            SpiExport::RAFlush => 0x11c0,
            SpiExport::RAFreeDecoder => 0x11f0,
            SpiExport::RAGetBackend => 0x1210,
            SpiExport::RAGetDecoderBackendGUID => 0x1210,
            SpiExport::RAGetGUID => 0x1220,
            SpiExport::RASetComMode => 0x1240,
            SpiExport::RAInitEncoder => 0x1360,
            SpiExport::RAEncode => 0x1380,
            SpiExport::RAFreeEncoder => 0x13a0,
            SpiExport::RACreateEncoderInstance => 0x6eb0,
            SpiExport::RASetPwd => 0x72f0,
        }
    }

    /// Whether this export is part of the **decode** path (vs. the
    /// encoder factory / open / encode / teardown exports, which
    /// spec/01 §2 records only at the export level — *"This chapter
    /// focuses on the decode path; the encoder is recorded here only at
    /// the export level and is otherwise a GAP"*).
    ///
    /// The four encoder exports are [`SpiExport::RAInitEncoder`],
    /// [`SpiExport::RAEncode`], [`SpiExport::RAFreeEncoder`] and
    /// [`SpiExport::RACreateEncoderInstance`]; the DRM hook
    /// [`SpiExport::RASetPwd`] is neither decode nor encode. Everything
    /// else is on the decode side.
    pub const fn is_decode_path(self) -> bool {
        !matches!(
            self,
            SpiExport::RAInitEncoder
                | SpiExport::RAEncode
                | SpiExport::RAFreeEncoder
                | SpiExport::RACreateEncoderInstance
                | SpiExport::RASetPwd
        )
    }

    /// Whether this export is one of the **encoder** exports.
    ///
    /// Their presence (spec/01 §2) is what shows the binary is the full
    /// encode+decode codec, not a decode-only build; their internals are
    /// a GAP in this clean-room workspace.
    pub const fn is_encoder(self) -> bool {
        matches!(
            self,
            SpiExport::RAInitEncoder
                | SpiExport::RAEncode
                | SpiExport::RAFreeEncoder
                | SpiExport::RACreateEncoderInstance
        )
    }

    /// The fixed `HRESULT` this export returns in this build when it is
    /// an `E_NOTIMPL` stub, else `None`.
    ///
    /// spec/01 §2 pins three such stubs: [`SpiExport::RAGetBackend`]
    /// and [`SpiExport::RAGetDecoderBackendGUID`] (shared body, both
    /// *"returns `0x80004001` (E_NOTIMPL)"*) and [`SpiExport::RAGetGUID`]
    /// (*"worker `0x1690` returns `0x80004001` (E_NOTIMPL in this
    /// build)"*).
    pub const fn notimpl_result(self) -> Option<u32> {
        match self {
            SpiExport::RAGetBackend | SpiExport::RAGetDecoderBackendGUID | SpiExport::RAGetGUID => {
                Some(E_NOTIMPL)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_exactly_twenty_exports() {
        assert_eq!(SPI_EXPORT_COUNT, 20);
        assert_eq!(SpiExport::ALL.len(), SPI_EXPORT_COUNT);
    }

    #[test]
    fn ordinals_are_one_through_twenty_with_no_gaps() {
        // spec/01 §2: "20 named functions (ordinals 1–20)".
        let mut seen = [false; SPI_EXPORT_COUNT];
        for e in SpiExport::ALL {
            let ord = e.ordinal();
            assert!((1..=20).contains(&ord), "ordinal {ord} out of 1..=20");
            assert!(!seen[(ord - 1) as usize], "duplicate ordinal {ord}");
            seen[(ord - 1) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "an ordinal in 1..=20 is missing");
    }

    #[test]
    fn all_array_is_in_ordinal_order() {
        for (i, e) in SpiExport::ALL.iter().enumerate() {
            assert_eq!(e.ordinal() as usize, i + 1);
        }
    }

    #[test]
    fn names_are_unique_and_nonempty() {
        let mut names: Vec<&str> = SpiExport::ALL.iter().map(|e| e.name()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate export name");
        assert!(SpiExport::ALL.iter().all(|e| !e.name().is_empty()));
    }

    #[test]
    fn drm_hook_is_exported_under_its_decorated_name() {
        // spec/01 §2: "_RASetPwd@8 (decorated @8 stdcall name)".
        assert_eq!(SpiExport::RASetPwd.name(), "_RASetPwd@8");
        assert_eq!(SpiExport::RASetPwd.ordinal(), 20);
    }

    #[test]
    fn front_end_rvas_match_spec_table() {
        // Spot-check the spec/01 §2 RVA column for every export.
        let pairs: &[(SpiExport, u32)] = &[
            (SpiExport::RAOpenCodec, 0x1010),
            (SpiExport::RAOpenCodec2, 0x1060),
            (SpiExport::RACloseCodec, 0x10b0),
            (SpiExport::RAGetNumberOfFlavors, 0x10e0),
            (SpiExport::RAGetNumberOfFlavors2, 0x1100),
            (SpiExport::RAGetFlavorProperty, 0x1120),
            (SpiExport::RASetFlavor, 0x1150),
            (SpiExport::RAInitDecoder, 0x1170),
            (SpiExport::RADecode, 0x1190),
            (SpiExport::RAFlush, 0x11c0),
            (SpiExport::RAFreeDecoder, 0x11f0),
            (SpiExport::RAGetBackend, 0x1210),
            (SpiExport::RAGetDecoderBackendGUID, 0x1210),
            (SpiExport::RAGetGUID, 0x1220),
            (SpiExport::RASetComMode, 0x1240),
            (SpiExport::RAInitEncoder, 0x1360),
            (SpiExport::RAEncode, 0x1380),
            (SpiExport::RAFreeEncoder, 0x13a0),
            (SpiExport::RACreateEncoderInstance, 0x6eb0),
            (SpiExport::RASetPwd, 0x72f0),
        ];
        for &(e, rva) in pairs {
            assert_eq!(e.front_end_rva(), rva, "{} RVA", e.name());
        }
    }

    #[test]
    fn get_backend_and_decoder_backend_guid_share_a_front_end() {
        // spec/01 §2: ordinal 9 "Same stub as RAGetBackend (shared
        // body)."
        assert_eq!(
            SpiExport::RAGetBackend.front_end_rva(),
            SpiExport::RAGetDecoderBackendGUID.front_end_rva()
        );
    }

    #[test]
    fn front_end_rvas_lie_in_the_text_section() {
        // spec/01 §1: .text spans RVA 0x1000..(0x1000+0x6c3c)=0x7c3c.
        for e in SpiExport::ALL {
            let rva = e.front_end_rva();
            assert!(
                (0x1000..0x7c3c).contains(&rva),
                "{} front-end RVA {rva:#x} outside .text",
                e.name()
            );
        }
    }

    #[test]
    fn exactly_three_exports_are_notimpl_stubs() {
        let stubs: Vec<SpiExport> = SpiExport::ALL
            .iter()
            .copied()
            .filter(|e| e.notimpl_result().is_some())
            .collect();
        assert_eq!(
            stubs,
            vec![
                SpiExport::RAGetBackend,
                SpiExport::RAGetDecoderBackendGUID,
                SpiExport::RAGetGUID
            ]
        );
        for e in stubs {
            assert_eq!(e.notimpl_result(), Some(E_NOTIMPL));
        }
    }

    #[test]
    fn exactly_four_exports_are_encoder_exports() {
        let encoders: Vec<SpiExport> = SpiExport::ALL
            .iter()
            .copied()
            .filter(|e| e.is_encoder())
            .collect();
        assert_eq!(encoders.len(), 4);
        for e in encoders {
            // Encoder exports are never on the decode path.
            assert!(!e.is_decode_path(), "{} should not be decode", e.name());
        }
    }

    #[test]
    fn decode_path_excludes_encoder_and_drm_hook() {
        assert!(!SpiExport::RASetPwd.is_decode_path());
        assert!(SpiExport::RADecode.is_decode_path());
        assert!(SpiExport::RAInitDecoder.is_decode_path());
        assert!(SpiExport::RAFlush.is_decode_path());
        // The two flavor-count getters are decode-side query helpers.
        assert!(SpiExport::RAGetNumberOfFlavors.is_decode_path());
        assert!(SpiExport::RAGetNumberOfFlavors2.is_decode_path());
    }

    #[test]
    fn hresult_constants_match_spec_values() {
        assert_eq!(S_OK, 0x0000_0000);
        assert_eq!(E_INVALIDARG, 0x8007_0057);
        assert_eq!(E_NOTIMPL, 0x8000_4001);
        assert_eq!(HR_UNRECOGNISED_SELECTOR, 0x8004_0005);
    }

    #[test]
    fn flavor_count_immediates_match_audit_2() {
        // audit #2: 0x1620 = mov ax,0x0f; 0x1630 = mov ax,0x22.
        assert_eq!(RA_NUMBER_OF_FLAVORS, 15);
        assert_eq!(RA_NUMBER_OF_FLAVORS2, 34);
        // The RASetFlavor upper bound is the same 0x22 immediate.
        assert_eq!(RA_NUMBER_OF_FLAVORS2, 0x22);
        // Cross-check the flavor-module constants describe the same SPI.
        assert_eq!(
            RA_NUMBER_OF_FLAVORS,
            crate::flavor::RA_GET_NUMBER_OF_FLAVORS_ADVERTISED as u16
        );
        assert_eq!(
            RA_NUMBER_OF_FLAVORS2,
            crate::flavor::RA_GET_NUMBER_OF_FLAVORS2_ADVERTISED as u16
        );
    }

    #[test]
    fn rasetflavor_context_offset_is_0x28() {
        // audit #3: RASetFlavor stores at ctx +0x28 after validating
        // index < 0x22.
        assert_eq!(RASETFLAVOR_CONTEXT_OFFSET, 0x28);
    }
}
