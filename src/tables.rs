//! Static numeric tables for cook decoding (§9 of the trace doc, plus §4
//! and §5 of `data/cook-vlc-tables.md`). Exclusively functional data —
//! no source-code reproduction.

/// Coefficients per subband (cook's hard-coded grain).
pub const SUBBAND_SIZE: usize = 20;

/// Hard cap on internal subpackets per cook frame (§9.1).
pub const MAX_SUBPACKETS: usize = 5;

/// `nb_bits` for the envelope VLC fast-path.
pub const QUANT_VLC_BITS: u32 = 9;

/// `nb_bits` for the joint-stereo coupling VLC.
pub const COUPLING_VLC_BITS: u32 = 6;

/// Maximum subbands count. (`subbands` in extradata, capped at init.)
pub const MAX_SUBBANDS: usize = 50;

/// Maximum `total_subbands == subbands + js_subband_start`.
pub const MAX_TOTAL_SUBBANDS: usize = 53;

/// Maximum `js_subband_start`. (Strict less-than 51.)
pub const MAX_JS_SUBBAND_START: usize = 50;

/// Quantisation centroids `quant_centroid_tab[7][14]` (§9.3 / §4.1).
/// Slots beyond `kmax_tab[cat] + 1` are zero-padded; the unpacker never
/// reads them.
pub const QUANT_CENTROID_TAB: [[f32; 14]; 7] = [
    [
        0.000, 0.392, 0.761, 1.120, 1.477, 1.832, 2.183, 2.541, 2.893, 3.245, 3.598, 3.942, 4.288,
        4.724,
    ],
    [
        0.000, 0.544, 1.060, 1.563, 2.068, 2.571, 3.072, 3.562, 4.070, 4.620, 0.0, 0.0, 0.0, 0.0,
    ],
    [
        0.000, 0.746, 1.464, 2.180, 2.882, 3.584, 4.316, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    ],
    [
        0.000, 1.006, 2.000, 2.993, 3.985, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    ],
    [
        0.000, 1.321, 2.703, 3.983, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    ],
    [
        0.000, 1.657, 3.491, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    ],
    [
        0.000, 1.964, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    ],
];

/// Dither magnitudes substituted for zero-digit subbands (§9.4 / §4.2).
/// Categories 0..4 substitute zero (silence in noise-flagged bins);
/// 5..7 substitute progressively louder noise; 8 is the
/// post-`expand_category` cap.
///
/// Values transcribed verbatim from the trace doc — `0.176777` is
/// `1/(2√2)·½` and `0.707107` is `1/√2`, both members of the standard
/// noise-substitution magnitude family used in transform-domain speech
/// codecs (G.722.1, AAC PNS).
#[allow(clippy::approx_constant)]
pub const DITHER_TAB: [f32; 9] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.176_777, 0.25, 0.707_107, 1.0];

/// Bit-cost-per-subband estimate for each category (§9.4 / §4.3).
/// Used by the categorise bisection. Monotonically decreasing; the
/// final 0 means "category 7 is free" (residual replaced by dither).
pub const EXPBITS_TAB: [i32; 8] = [52, 47, 43, 37, 29, 22, 16, 0];

/// Per-SQVH-category dimensions (§9.5 / §3.2).
pub const KMAX_TAB: [u32; 7] = [13, 9, 6, 4, 3, 2, 1];
pub const VD_TAB: [u32; 7] = [2, 2, 2, 4, 4, 5, 5];
pub const VPR_TAB: [u32; 7] = [10, 10, 10, 5, 5, 4, 4];

/// `invradix_tab[cat] = round(0x100000 / (kmax+1))` — the integer
/// constant the digit-extraction loop uses instead of division.
pub const INVRADIX_TAB: [u32; 7] = [74_899, 104_858, 149_797, 209_716, 262_144, 349_526, 524_288];

/// `vhvlcsize_tab[cat]` — `nb_bits` argument for SQVH VLC fast path.
pub const VHVLCSIZE_TAB: [u32; 7] = [8, 7, 7, 10, 9, 9, 6];

/// Coupling-band map `cplband[51]` (§9.6 / §5.1). Maps each subband
/// index (0..50) to a coupling-band index (0..19).
pub const CPLBAND: [u8; 51] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, // 0..9
    10, 11, 11, 12, 12, 13, 13, 14, 14, 14, // 10..19
    15, 15, 15, 15, 16, 16, 16, 16, 16, 17, // 20..29
    17, 17, 17, 17, 17, 18, 18, 18, 18, 18, // 30..39
    18, 18, 19, 19, 19, 19, 19, 19, 19, 19, 19, // 40..50
];

/// `cplscale2[5]` — joint-stereo coupling, js_vlc_bits=2 (§5.2).
pub const CPLSCALE2: [f32; 5] = [
    1.000_000_000_000,
    0.953_020_632_267,
    0.707_106_769_085,
    0.302_905_440_331,
    0.000_000_000_000,
];

/// `cplscale3[9]` — joint-stereo coupling, js_vlc_bits=3.
pub const CPLSCALE3: [f32; 9] = [
    1.000_000_000_000,
    0.981_279_790_401,
    0.936_997_592_449,
    0.875_934_481_621,
    0.707_106_769_085,
    0.482_430_040_836,
    0.349_335_819_483,
    0.192_587_479_949,
    0.000_000_000_000,
];

/// `cplscale4[17]` — joint-stereo coupling, js_vlc_bits=4.
pub const CPLSCALE4: [f32; 17] = [
    1.000_000_000_000,
    0.991_486_728_191,
    0.973_249_018_192,
    0.953_020_632_267,
    0.930_133_521_557,
    0.903_453_230_858,
    0.870_746_195_316,
    0.826_180_458_069,
    0.707_106_769_085,
    0.563_405_573_368,
    0.491_732_746_363,
    0.428_686_618_805,
    0.367_221_474_648,
    0.302_905_440_331,
    0.229_752_898_216,
    0.130_207_896_233,
    0.000_000_000_000,
];

/// `cplscale5[33]` — joint-stereo coupling, js_vlc_bits=5.
pub const CPLSCALE5: [f32; 33] = [
    1.000_000_000_000,
    0.995_926_380_157,
    0.987_517_595_291,
    0.978_726_446_629,
    0.969_505_727_291,
    0.959_797_799_587,
    0.949_531_257_153,
    0.938_616_216_183,
    0.926_936_149_597,
    0.914_336_204_529,
    0.900_602_877_140,
    0.885_426_938_534,
    0.868_331_849_575,
    0.848_510_861_397,
    0.824_381_768_703,
    0.791_833_400_726,
    0.707_106_769_085,
    0.610_737_144_947,
    0.566_034_197_807,
    0.529_177_963_734,
    0.495_983_630_419,
    0.464_778_542_519,
    0.434_642_940_760,
    0.404_955_863_953,
    0.375_219_136_477,
    0.344_963_222_742,
    0.313_672_333_956,
    0.280_692_428_350,
    0.245_068_684_220,
    0.205_169_528_723,
    0.157_508_864_999,
    0.090_170_010_924,
    0.000_000_000_000,
];

/// `cplscale6[65]` — joint-stereo coupling, js_vlc_bits=6.
pub const CPLSCALE6: [f32; 65] = [
    1.000_000_000_000,
    0.998_005_926_609,
    0.993_956_744_671,
    0.989_822_506_905,
    0.985_598_564_148,
    0.981_279_790_401,
    0.976_860_702_038,
    0.972_335_040_569,
    0.967_696_130_276,
    0.962_936_460_972,
    0.958_047_747_612,
    0.953_020_632_267,
    0.947_844_684_124,
    0.942_508_161_068,
    0.936_997_592_449,
    0.931_297_719_479,
    0.925_390_899_181,
    0.919_256_627_560,
    0.912_870_943_546,
    0.906_205_296_516,
    0.899_225_592_613,
    0.891_890_347_004,
    0.884_148_240_089,
    0.875_934_481_621,
    0.867_165_684_700,
    0.857_730_865_479,
    0.847_477_376_461,
    0.836_184_680_462,
    0.823_513_329_029,
    0.808_890_223_503,
    0.791_194_140_911,
    0.767_520_070_076,
    0.707_106_769_085,
    0.641_024_887_562,
    0.611_565_053_463,
    0.587_959_706_783,
    0.567_296_981_812,
    0.548_448_026_180,
    0.530_831_515_789,
    0.514_098_942_280,
    0.498_019_754_887,
    0.482_430_040_836,
    0.467_206_478_119,
    0.452_251_672_745,
    0.437_485_188_246,
    0.422_837_972_641,
    0.408_248_275_518,
    0.393_658_757_210,
    0.379_014_074_802,
    0.364_258_885_384,
    0.349_335_819_483,
    0.334_183_186_293,
    0.318_732_559_681,
    0.302_905_440_331,
    0.286_608_695_984,
    0.269_728_302_956,
    0.252_119_421_959,
    0.233_590_632_677,
    0.213_876_649_737,
    0.192_587_479_949,
    0.169_101_938_605,
    0.142_307_326_198,
    0.109_772_264_957,
    0.063_119_828_701,
    0.000_000_000_000,
];

/// Look up the cplscale ladder for a given `js_vlc_bits` (2..=6).
/// Returns the slice for `js_vlc_bits - 2`.
pub fn cplscale(js_vlc_bits: u32) -> &'static [f32] {
    match js_vlc_bits {
        2 => &CPLSCALE2,
        3 => &CPLSCALE3,
        4 => &CPLSCALE4,
        5 => &CPLSCALE5,
        6 => &CPLSCALE6,
        _ => &[],
    }
}

// ───────── Power-of-2 lookup tables (§9.8) ─────────

/// `pow2tab[i + 63] = 2^i` for `i ∈ [-63, +63]`. 127 entries.
pub fn pow2tab() -> &'static [f32; 127] {
    static T: std::sync::OnceLock<[f32; 127]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0.0f32; 127];
        for (i, slot) in t.iter_mut().enumerate() {
            let exp = (i as i32) - 63;
            *slot = 2.0f32.powi(exp);
        }
        t
    })
}

/// `rootpow2tab[i + 63] = 2^(i/2)` for `i ∈ [-63, +63]`. 127 entries.
pub fn rootpow2tab() -> &'static [f32; 127] {
    static T: std::sync::OnceLock<[f32; 127]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0.0f32; 127];
        for (i, slot) in t.iter_mut().enumerate() {
            let exp_half = ((i as i32) - 63) as f32 * 0.5;
            *slot = 2.0f32.powf(exp_half);
        }
        t
    })
}

/// Per-frame gain table (§9.9). Built once per `samples_per_channel`.
/// `gain_table[i] = 2^((i - 15) / gain_size_factor)` for `i ∈ [0, 30]`.
/// `gain_table[15] == 1.0`. Used for intra-slot exponential gain ramps.
pub fn build_gain_table(samples_per_channel: usize) -> [f32; 31] {
    let gsf = (samples_per_channel as f32) / 8.0;
    let mut t = [0.0f32; 31];
    for (i, slot) in t.iter_mut().enumerate() {
        let exp = ((i as f32) - 15.0) / gsf;
        *slot = 2.0f32.powf(exp);
    }
    t
}

/// Cook's selection rule for the envelope-Huffman table given a subband
/// index and `js_subband_start` (§5.3). Returns an index in `[0, 12]`.
///
/// The trace doc gives `vlc_index ∈ [1..13]`, then clips to 13. This
/// crate stores 13 tables at indexes `0..=12`, so we map
/// `vlc_index → vlc_index - 1` clipped to 12.
pub fn envelope_table_index(subband: usize, js_subband_start: usize) -> usize {
    let raw = if subband >= 2 * js_subband_start {
        subband.saturating_sub(js_subband_start)
    } else {
        (subband / 2).max(1)
    };
    raw.min(13).saturating_sub(1).min(12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_invariant() {
        // vd × vpr == SUBBAND_SIZE for every category.
        for cat in 0..7 {
            assert_eq!(
                (VD_TAB[cat] * VPR_TAB[cat]) as usize,
                SUBBAND_SIZE,
                "cat {cat}: vd*vpr != 20"
            );
        }
    }

    #[test]
    fn cplband_max_is_19() {
        assert_eq!(CPLBAND.iter().copied().max().unwrap(), 19);
    }

    #[test]
    fn cplscale_unit_norm_at_midpoint() {
        for &js in &[2u32, 3, 4, 5, 6] {
            let s = cplscale(js);
            let mid = s.len() / 2;
            // s[mid]^2 + s[mid]^2 = 2 * 0.5 = 1.0 (because mid is the
            // equal-energy split point, value ≈ √½).
            let v = s[mid];
            assert!(
                (v * v - 0.5).abs() < 1e-5,
                "midpoint^2 not 0.5 for js_vlc_bits={js}"
            );
        }
    }

    #[test]
    fn pow2tab_indexing() {
        let t = pow2tab();
        assert!((t[63] - 1.0).abs() < 1e-9, "pow2tab[63] = {}", t[63]);
        assert!((t[64] - 2.0).abs() < 1e-9, "pow2tab[64] = {}", t[64]);
        assert!((t[62] - 0.5).abs() < 1e-9, "pow2tab[62] = {}", t[62]);
    }

    #[test]
    fn gain_table_centered() {
        let g = build_gain_table(1024);
        assert!((g[15] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn envelope_table_index_basic() {
        // Below 2*js_subband_start: max(1, subband/2) clipped to [1, 13].
        assert_eq!(envelope_table_index(0, 4), 0); // max(1, 0) = 1 → idx 0
        assert_eq!(envelope_table_index(2, 4), 0); // max(1, 1) = 1 → idx 0
        assert_eq!(envelope_table_index(4, 4), 1); // max(1, 2) = 2 → idx 1
        assert_eq!(envelope_table_index(8, 4), 3); // 8 >= 8 → 8-4 = 4 → idx 3
        assert_eq!(envelope_table_index(20, 4), 12); // 20-4 = 16 → clipped to 13 → idx 12
    }
}
