# oxideav-cook test fixtures

Real-stream RealMedia fixtures used by the integration tests.

| File | Size (B) | SHA-256 |
|------|---------:|---------|
| `FUN_RM_32.rm` | 69765 | `ae7804ce179f7d8d907f67ac3e17c0da560e05c7730e1c45a04c1d19a2e45d5c` |

## `FUN_RM_32.rm`

A small (~68 KB, ~16.7 s) RealMedia `.rm` file with a single RealAudio
**Cook** (FourCC `cook`) stream at flavor 21 (stereo 44 100 Hz,
1024-sample frames, 32 subbands, `coded_frame_size = 465`,
`sub_packet_size = 93`). The file is test data, not decoder source; it is
the same fixture the upstream clean-room workspace uses to validate the
binary-derived decoder model end-to-end (see the validator chapter cited
from this crate's `tests/realstream_fixture.rs`).

The fixture's per-stream parameters and packet framing are pinned by
that validator; this crate's integration test parses the wire bytes
directly and feeds the resulting `(cookie, descriptor, flavor,
frame_bytes)` tuple into `DecodeConfig::from_inputs` to check that the
crate's decode-config layer agrees with every measured number.
