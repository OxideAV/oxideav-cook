# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.3](https://github.com/OxideAV/oxideav-cook/compare/v0.0.2...v0.0.3) - 2026-05-06

### Other

- prepend retirement notice (docs audit 2026-05-06)

## [0.0.2](https://github.com/OxideAV/oxideav-cook/compare/v0.0.1...v0.0.2) - 2026-05-03

### Other

- drop duplicate semver_check key
- remove unused Decoder import + use div_ceil
- replace never-match regex with semver_check = false
- cargo fmt: fix rustfmt --check CI gate

### Added

- Initial release: pure-Rust RealAudio **Cook** (G2 / "Cooker") decoder
  for the oxideav framework. Implements the full per-subpacket pipeline
  documented in `docs/audio/cook/cook-trace-reverse-engineering.md`:
  XOR descrambler (0x37C511F2 rotated), gain-profile RLE, differential
  scale-factor envelope (13 envelope Huffman tables), bit-budget
  category bisection, SQVH residual decode (7 per-category Huffman
  tables, base-(kmax+1) digit unpacking), per-band scalar dequantization
  with dither for high-category bands, joint-stereo matrix decoupling
  (5 cplscale ladders, 51-entry cplband map), MDCT with sine window,
  overlap-add lapping, and per-slot gain ramping.
- Modes covered: MONO (cookversion 0x01000001), STEREO (0x01000002),
  JOINT_STEREO (0x01000003) and MULTI_CHANNEL / MC_COOK (0x02000000)
  with chained sub-blobs (5.1 / 7.1).
- Codec id `cook` registered in the oxideav `CodecRegistry`.
