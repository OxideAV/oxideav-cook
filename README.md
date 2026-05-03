# oxideav-cook

Pure-Rust **RealAudio Cook** (RealNetworks G2 / "Cooker") audio decoder
for the [oxideav](https://github.com/OxideAV/oxideav-workspace) media
framework.

Cook is the audio codec used inside RealMedia (`*.rm` / `*.ra` /
`*.rmvb`) containers — sub-band MDCT-based, lossy, with a fixed XOR
obfuscation layer, a differentially-coded subband-envelope scheme, and
seven Huffman-coded scalar-quantized vector tables that pack 20
coefficients per subband. JOINT_STEREO mode adds a unit-norm matrix
decoupling step; MC_COOK chains independent single-stream cooks for
multichannel layouts (5.1 / 7.1).

## Coverage

- All four `cookversion` modes:
  `0x01000001` MONO, `0x01000002` STEREO, `0x01000003` JOINT_STEREO,
  `0x02000000` MC_COOK (5.1 / 7.1 layouts as chained 20-byte sub-blobs).
- All three sample-per-channel block sizes (256 / 512 / 1024).
- The full categoriser bisection (binary search + two-cursor expand /
  contract) with the 8-element `expbits_tab` cost vector.
- All seven SQVH tables (categories 0..6) with verbatim symbol orderings
  taken from the clean-room VLC tables sidecar at
  `docs/audio/cook/data/cook-vlc-tables.md`.
- All thirteen envelope Huffman tables and the five joint-stereo
  coupling tables.
- Sine-window MDCT lapping, per-slot gain ramps with the
  `2^((i-15)/gain_size_factor)` gain table, deterministic AVLFG-derived
  dither (MD5-seeded, seed = 0).

## Spec / docs

This decoder was implemented strictly from the clean-room
behavioural-trace document at
[`docs/audio/cook/cook-trace-reverse-engineering.md`](../../docs/audio/cook/cook-trace-reverse-engineering.md)
and its accompanying VLC tables at
[`docs/audio/cook/data/cook-vlc-tables.md`](../../docs/audio/cook/data/cook-vlc-tables.md).
**No external library source code was consulted** — `ffmpeg(1)` is used
only as a black-box validator for fixture decoding.

## Quick use

```rust,no_run
use oxideav_core::{CodecId, CodecParameters, Decoder, Packet, SampleFormat};

let mut params = CodecParameters::audio(CodecId::new(oxideav_cook::CODEC_ID_STR));
params.sample_rate = Some(44_100);
params.channels = Some(2);
params.sample_format = Some(SampleFormat::F32);
// extradata = the 8 / 16 / 80-byte cook-specific blob from the
// RealMedia MDPR chunk (see §4 of the trace doc).
params.extradata = vec![/* ... */];

let mut dec = oxideav_cook::decoder::make_decoder(&params).unwrap();
// dec.send_packet(&Packet { data: ..., pts: ..., .. })?;
// dec.receive_frame()?;
```

## License

MIT. See `LICENSE`.
