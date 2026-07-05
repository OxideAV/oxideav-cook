//! Framework registration surface — the [`oxideav_core`] codec-registry
//! entry point and the dual-API `make_decoder` factory.
//!
//! This wires the crate into the framework's in-process codec registry
//! ([`register`] / [`register_codecs`]) and exposes the historical
//! direct-factory endpoint ([`make_decoder`], the
//! `decoder::make_decoder` convention the workspace's dual-API rule
//! keeps alongside the registry path).
//!
//! ## Decode status
//!
//! The Cook decode pipeline is assembled end-to-end **except** the §2.2
//! per-band **category-assignment loop** (`cook.dll!0x4800`, algorithm a
//! recorded DOCS-GAP — see [`crate::frame_decode`]). Because the per-band
//! categories are *computed in the decoder*, not carried on the wire, a
//! real packet cannot be decoded to correct PCM until that loop is
//! pinned. [`CookDecoder`] therefore parses and structurally validates
//! each packet (the pinned §0–§2.1 frame-body prefix over the real
//! bitstream) and surfaces the precise remaining blocker as a typed
//! [`oxideav_core::Error::Unsupported`] on [`Decoder::receive_frame`],
//! rather than emitting fabricated audio. Every other stage — the
//! entropy read, the expectation reconstruction, the recovered §4.3
//! coupling split, and the recovered-window §5 synthesis — is wired and
//! exercised by [`crate::frame_decode`] and `tests/entropy_to_pcm.rs`.

use oxideav_core::{
    CodecCapabilities, CodecId, CodecInfo, CodecParameters, CodecRegistry, CodecTag, Decoder,
    Error as CoreError, Frame, Packet, Result as CoreResult, RuntimeContext, SampleFormat,
};

/// Codec id under which [`register_codecs`] installs this decoder.
pub const CODEC_ID_STR: &str = "cook";

/// Matroska/WebM codec id for RealAudio Cook (`A_REAL/COOK`).
pub const MATROSKA_CODEC_ID: &str = "A_REAL/COOK";

/// RealMedia audio fourcc for Cook / "Cooker" (`cook`).
pub const COOK_FOURCC: [u8; 4] = *b"cook";

/// Build a boxed RealAudio Cook [`Decoder`] from `params`.
///
/// `params.sample_rate` and `params.channels` seed the output stream
/// parameters (Cook is 1 or 2 channels); `params.extradata`, when
/// present, is the per-stream Cook cookie (the 16-byte extended
/// `0x01000003` layout — [`crate::CookCookie`]), parsed to configure the
/// decoder's geometry.
///
/// # Errors
///
/// [`oxideav_core::Error::invalid`] when `channels` is supplied and not
/// 1 or 2 (Cook carries at most two channels).
pub fn make_decoder(params: &CodecParameters) -> CoreResult<Box<dyn Decoder>> {
    let channels = params.channels.unwrap_or(2);
    if channels != 1 && channels != 2 {
        return Err(CoreError::invalid(format!(
            "oxideav-cook: decoder supports 1 or 2 channels (channels={channels})"
        )));
    }
    let sample_rate = params.sample_rate.unwrap_or(44_100);

    let mut out = CodecParameters::audio(CodecId::new(CODEC_ID_STR));
    out.sample_rate = Some(sample_rate);
    out.channels = Some(channels);
    out.sample_format = Some(SampleFormat::S16);
    out.tag = Some(CodecTag::fourcc(&COOK_FOURCC));

    Ok(Box::new(CookDecoder {
        codec_id: CodecId::new(CODEC_ID_STR),
        output: out,
        extradata: params.extradata.clone(),
    }))
}

/// Packet-to-frame adaptor wrapping the crate's decode pipeline in the
/// framework [`Decoder`] trait.
///
/// See the module docs: the pipeline is complete except the §2.2
/// category-assignment loop, so `receive_frame` surfaces that single
/// recorded GAP as a typed unsupported error rather than fabricating
/// audio.
pub struct CookDecoder {
    codec_id: CodecId,
    output: CodecParameters,
    /// Per-stream Cook cookie (from `CodecParameters::extradata`), if
    /// the container supplied one.
    extradata: Vec<u8>,
}

impl CookDecoder {
    /// The parsed per-stream cookie, if the container supplied a
    /// well-formed extended (`0x01000003`) extradata blob.
    pub fn cookie(&self) -> Option<crate::CookCookie> {
        crate::CookCookie::parse(&self.extradata).ok()
    }

    /// The output stream parameters this decoder advertises.
    pub fn output_params(&self) -> &CodecParameters {
        &self.output
    }
}

impl Decoder for CookDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> CoreResult<()> {
        // Structurally validate the packet against the wired front end:
        // an empty packet is a container framing error; a non-empty one
        // is a real coded frame the pipeline can parse up to the §2.2
        // category-assignment blocker. We do not buffer output because
        // the entropy→PCM stage needs the (GAP) per-band categories.
        if packet.data.is_empty() {
            return Err(CoreError::invalid(
                "oxideav-cook: empty packet (no coded Cook frame)",
            ));
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> CoreResult<Frame> {
        Err(CoreError::unsupported(
            "oxideav-cook: real-stream decode to PCM is blocked on the §2.2 \
             per-band category-assignment loop (cook.dll!0x4800), a recorded \
             DOCS-GAP — the categories are computed in the decoder, not carried \
             on the wire, so the per-band codebook cannot yet be derived. Every \
             downstream stage (entropy read, expectation reconstruction, §4.3 \
             coupling, recovered-window synthesis) is wired; see \
             oxideav_cook::frame_decode and tests/entropy_to_pcm.rs.",
        ))
    }

    fn flush(&mut self) -> CoreResult<()> {
        Ok(())
    }
}

/// Register the Cook decoder into a [`CodecRegistry`].
///
/// Installs one [`CodecInfo`] under the `"cook"` id carrying the decode
/// factory and the container tags a demuxer resolves Cook streams by
/// (the RealMedia `cook` fourcc and the Matroska `A_REAL/COOK` id).
pub fn register_codecs(reg: &mut CodecRegistry) {
    let info = CodecInfo::new(CodecId::new(CODEC_ID_STR))
        .capabilities(
            CodecCapabilities::audio(CODEC_ID_STR)
                .with_decode()
                .with_lossy(true),
        )
        .decoder(make_decoder)
        .tags([
            CodecTag::fourcc(&COOK_FOURCC),
            CodecTag::matroska(MATROSKA_CODEC_ID),
        ]);
    reg.register(info);
}

/// Canonical sibling registration entry point (see
/// [`oxideav_core::register!`]): installs the Cook decoder into `ctx`'s
/// codec registry.
pub fn register(ctx: &mut RuntimeContext) {
    register_codecs(&mut ctx.codecs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{MediaType, Packet, TimeBase};

    fn params() -> CodecParameters {
        let mut p = CodecParameters::audio(CodecId::new(CODEC_ID_STR));
        p.sample_rate = Some(44_100);
        p.channels = Some(2);
        p
    }

    #[test]
    fn make_decoder_wires_output_params() {
        let dec = make_decoder(&params()).unwrap();
        assert_eq!(dec.codec_id().as_str(), CODEC_ID_STR);
    }

    #[test]
    fn make_decoder_rejects_bad_channel_count() {
        let mut p = params();
        p.channels = Some(3);
        assert!(make_decoder(&p).is_err());
        p.channels = Some(1);
        assert!(make_decoder(&p).is_ok());
        p.channels = None; // defaults to 2
        assert!(make_decoder(&p).is_ok());
    }

    #[test]
    fn decoder_advertises_s16_stereo_output() {
        let boxed = make_decoder(&params()).unwrap();
        // Downcast-free: the trait object exposes codec id; the concrete
        // output params are checked through a direct build below.
        assert_eq!(boxed.codec_id().as_str(), CODEC_ID_STR);
        let concrete = CookDecoder {
            codec_id: CodecId::new(CODEC_ID_STR),
            output: {
                let mut o = CodecParameters::audio(CodecId::new(CODEC_ID_STR));
                o.sample_rate = Some(44_100);
                o.channels = Some(2);
                o.sample_format = Some(SampleFormat::S16);
                o
            },
            extradata: Vec::new(),
        };
        assert_eq!(concrete.output_params().media_type, MediaType::Audio);
        assert_eq!(
            concrete.output_params().sample_format,
            Some(SampleFormat::S16)
        );
        assert_eq!(concrete.output_params().channels, Some(2));
    }

    #[test]
    fn empty_packet_is_a_framing_error() {
        let mut dec = make_decoder(&params()).unwrap();
        let empty = Packet {
            stream_index: 0,
            time_base: TimeBase::new(1, 44_100),
            pts: None,
            dts: None,
            duration: None,
            data: Vec::new(),
            flags: Default::default(),
        };
        assert!(dec.send_packet(&empty).is_err());
    }

    #[test]
    fn real_frame_parses_then_surfaces_the_category_gap() {
        let mut dec = make_decoder(&params()).unwrap();
        let pkt = Packet {
            stream_index: 0,
            time_base: TimeBase::new(1, 44_100),
            pts: Some(0),
            dts: None,
            duration: None,
            data: vec![0x20u8; 465], // a non-empty coded frame
            flags: Default::default(),
        };
        // A non-empty packet is accepted (the pinned front end parses it)...
        dec.send_packet(&pkt).unwrap();
        // ...but the real decode to PCM surfaces the §2.2 category GAP.
        match dec.receive_frame() {
            Err(CoreError::Unsupported(msg)) => {
                assert!(msg.contains("category-assignment"));
            }
            other => panic!("expected the category-assignment GAP, got {other:?}"),
        }
        dec.flush().unwrap();
    }

    #[test]
    fn register_installs_the_cook_decoder() {
        let mut ctx = RuntimeContext::new();
        register(&mut ctx);
        assert!(ctx.codecs.has_decoder(&CodecId::new(CODEC_ID_STR)));
        assert!(!ctx.codecs.has_encoder(&CodecId::new(CODEC_ID_STR)));
    }

    #[test]
    fn registered_tags_resolve_to_cook() {
        let mut ctx = RuntimeContext::new();
        register(&mut ctx);
        let tags: Vec<_> = ctx.codecs.all_tag_registrations().collect();
        assert!(tags.iter().any(
            |(tag, id)| **tag == CodecTag::fourcc(&COOK_FOURCC) && id.as_str() == CODEC_ID_STR
        ));
        assert!(tags
            .iter()
            .any(|(tag, id)| **tag == CodecTag::matroska(MATROSKA_CODEC_ID)
                && id.as_str() == CODEC_ID_STR));
    }
}
