# Changelog

All notable changes to this crate are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.2](https://github.com/OxideAV/oxideav-cook/releases/tag/v0.0.2) - 2026-05-30

### Other

- Round 2: vendor 8 extracted DSP tables with self-validating loaders
- Round 1: flavor geometry table loader + extradata cookie parser
- Round 0 — clean-room rebuild scaffold (orphan master)

### Added

- Flavor geometry table loader: `flavor_record(index) -> Option<FlavorRecord>`
  reads the 31 well-formed per-flavor records from the vendored facts
  table `tables/flavor-geometry-table.csv` (parsed on demand, never
  retyped into source).
- Extradata cookie parser: `CookCookie::parse` reads the big-endian
  per-stream cookie for the extended (`>= 0x01000003`) selector, recovers
  samples-per-frame, and cross-checks the cookie against its named flavor
  record (`matches_flavor`). Pinned against the real `FUN_RM_32.rm`
  stream (flavor 21) in `tests/cookie_realstream.rs`.
- `Error::CookieTooShort` and `Error::UnsupportedSelector` variants.
- `tables` module vendoring the remaining eight extracted DSP parameter
  tables (two 127-entry power-of-two ladders, per-category gain-step /
  gain-bias / level-count triples, the 11-entry reciprocal averaging
  table, the 51-entry monotone category-index LUT, and the five
  Princen-Bradley MDCT half-windows of lengths 3 / 7 / 15 / 31 / 64),
  each `OnceLock`-cached and self-validating against the constraint
  stated in its clean-room `.meta` (f32-exact equality with `2^k`, TDAC
  identity, monotonicity).

### Changed

- Clean-room rebuild from a fresh orphan `master`. The previous
  implementation was retired by the OxideAV docs audit dated
  2026-05-06; the prior history is preserved on the `old` branch.
  See `README.md` for the rebuild scope and the strict-isolation
  workspace the Implementer rounds will draw from.
