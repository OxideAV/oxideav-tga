//! TGA container: one single-image file becomes one [`Packet`] on
//! stream `0`. Matches how other single-image codecs in the workspace
//! (`oxideav-bmp`, `oxideav-png` for non-APNG, `oxideav-webp` for
//! static WEBP) plug into the container pipeline.
//!
//! Probe scoring is conservative: TGA has no magic bytes at the start
//! of the file (the format pre-dates the practice), so we can only
//! probe the optional 26-byte 2.0 footer at end-of-file or fall back to
//! the file extension.

use std::io::{Read, SeekFrom, Write};

use oxideav_core::{
    CodecId, CodecParameters, CodecResolver, Error, MediaType, Packet, PixelFormat, Result,
    StreamInfo, TimeBase,
};
use oxideav_core::{
    ContainerRegistry, Demuxer, Muxer, ProbeData, ProbeScore, ReadSeek, WriteSeek, MAX_PROBE_SCORE,
};

use crate::types::{parse_footer, parse_header};

pub fn register(reg: &mut ContainerRegistry) {
    reg.register_demuxer("tga", open_demuxer);
    reg.register_muxer("tga", open_muxer);
    reg.register_extension("tga", "tga");
    reg.register_extension("targa", "tga");
    reg.register_extension("tpic", "tga");
    reg.register_extension("vda", "tga");
    reg.register_extension("icb", "tga");
    reg.register_extension("vst", "tga");
    reg.register_probe("tga", probe);
}

fn probe(data: &ProbeData) -> ProbeScore {
    // Strongest signal: TGA 2.0 footer magic at end of file.
    if parse_footer(data.buf).is_some() {
        return MAX_PROBE_SCORE;
    }
    // Weaker but still useful: a header that parses, with an image
    // type in our known set + a non-zero width/height + a sane depth.
    if let Some(h) = parse_header(data.buf) {
        let plausible_type = matches!(h.image_type_raw, 0 | 1 | 2 | 3 | 9 | 10 | 11);
        let plausible_depth = matches!(h.depth, 8 | 15 | 16 | 24 | 32);
        if plausible_type && plausible_depth && h.width != 0 && h.height != 0 {
            return oxideav_core::PROBE_SCORE_EXTENSION + 5;
        }
    }
    if matches!(
        data.ext,
        Some("tga") | Some("targa") | Some("tpic") | Some("vda") | Some("icb") | Some("vst")
    ) {
        oxideav_core::PROBE_SCORE_EXTENSION
    } else {
        0
    }
}

pub fn open_demuxer(
    mut input: Box<dyn ReadSeek>,
    _codecs: &dyn CodecResolver,
) -> Result<Box<dyn Demuxer>> {
    input.seek(SeekFrom::Start(0))?;
    let mut buf = Vec::new();
    input.read_to_end(&mut buf)?;
    let header = parse_header(&buf).ok_or_else(|| Error::invalid("TGA: header truncated"))?;
    let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    params.width = Some(header.width as u32);
    params.height = Some(header.height as u32);
    params.pixel_format = Some(PixelFormat::Rgba);
    let stream = StreamInfo {
        index: 0,
        params,
        time_base: TimeBase::new(1, 1),
        start_time: Some(0),
        duration: None,
    };
    Ok(Box::new(TgaDemuxer {
        streams: vec![stream],
        data: Some(buf),
    }))
}

struct TgaDemuxer {
    streams: Vec<StreamInfo>,
    /// `None` once the sole packet has been emitted.
    data: Option<Vec<u8>>,
}

impl Demuxer for TgaDemuxer {
    fn format_name(&self) -> &str {
        "tga"
    }
    fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }
    fn next_packet(&mut self) -> Result<Packet> {
        match self.data.take() {
            Some(bytes) => {
                let mut pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
                pkt.pts = Some(0);
                pkt.dts = Some(0);
                pkt.flags.keyframe = true;
                Ok(pkt)
            }
            None => Err(Error::Eof),
        }
    }
}

pub fn open_muxer(output: Box<dyn WriteSeek>, streams: &[StreamInfo]) -> Result<Box<dyn Muxer>> {
    if streams.len() != 1 {
        return Err(Error::invalid(
            "TGA muxer: expected exactly one video stream",
        ));
    }
    if streams[0].params.media_type != MediaType::Video {
        return Err(Error::invalid("TGA muxer: stream must be video"));
    }
    Ok(Box::new(TgaMuxer { output }))
}

struct TgaMuxer {
    output: Box<dyn WriteSeek>,
}

impl Muxer for TgaMuxer {
    fn format_name(&self) -> &str {
        "tga"
    }
    fn write_header(&mut self) -> Result<()> {
        Ok(())
    }
    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        self.output.write_all(&packet.data)?;
        Ok(())
    }
    fn write_trailer(&mut self) -> Result<()> {
        Ok(())
    }
}
