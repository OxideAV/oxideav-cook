//! Cook codec registration.

use oxideav_core::{
    CodecCapabilities, CodecId, CodecInfo, CodecParameters, CodecRegistry, Decoder, Result,
};

pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::audio("cook_sw")
        .with_lossy(true)
        .with_max_channels(8)
        .with_max_sample_rate(48_000);
    reg.register(
        CodecInfo::new(CodecId::new(super::CODEC_ID_STR))
            .capabilities(caps)
            .decoder(make_decoder),
    );
}

fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    super::decoder::make_decoder(params)
}
