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
    BlobKind, CodecId, CodecParameters, CodecResolver, Error, MediaType, MetadataBlob, Packet,
    Result, StreamInfo, TimeBase,
};
use oxideav_core::{
    ContainerRegistry, Demuxer, MAX_PROBE_SCORE, Muxer, ProbeData, ProbeScore, ReadSeek, WriteSeek,
};

use crate::decoder::{native_format, validate_header};
use crate::types::{TgaDeveloperArea, TgaExtensionArea, extension_area_bytes, parse_footer};

/// The TGA 2.0 structures of a file as the registry path carries them:
/// the extension area as the `tga-extension-area` blob plus its text
/// fields as `(key, value)` pairs, the developer area as the
/// self-contained `tga-developer-area` blob. Empty for a TGA 1.0 file
/// or a marker-only footer. Shared by the demuxer (stream level) and
/// the decoder (frame record).
pub(crate) fn file_metadata(input: &[u8]) -> (Vec<MetadataBlob>, Vec<(String, String)>) {
    let mut blobs = Vec::new();
    let mut pairs = Vec::new();
    if let Some(body) = extension_area_bytes(input) {
        blobs.push(MetadataBlob::new(
            BlobKind::from_static(crate::BLOB_KIND_EXTENSION_AREA),
            body.to_vec(),
        ));
        if let Some(area) = TgaExtensionArea::from_bytes(body) {
            pairs = area.metadata_pairs();
        }
    }
    if let Some(footer) = parse_footer(input)
        && let Some(dev) = TgaDeveloperArea::parse(input, footer.developer_directory_offset)
        && !dev.is_empty()
    {
        blobs.push(MetadataBlob::new(
            BlobKind::from_static(crate::BLOB_KIND_DEVELOPER_AREA),
            dev.to_portable_bytes(input),
        ));
    }
    (blobs, pairs)
}

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
    // Weaker but still useful: a header that validates (an image type
    // in our known set at a depth it allows + non-zero width/height) —
    // the same plausibility test as the standalone `probe`.
    if validate_header(data.buf, false).is_ok() {
        return oxideav_core::PROBE_SCORE_EXTENSION + 5;
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
    let v = validate_header(&buf, false)?;
    let header = v.header;
    let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    params.width = Some(header.width as u32);
    params.height = Some(header.height as u32);
    // The framework decoder emits the native layout (`crate::decode`):
    // the stream declares the same one.
    params.pixel_format = Some(crate::registry::to_core_pixel_format(native_format(
        v.image_type,
        header.depth,
    )));
    // TGA 2.0 structures: the extension area and developer area ride
    // the stream as format-native blobs; the extension area's text
    // fields are the container metadata.
    let (blobs, metadata) = file_metadata(&buf);
    params.blobs = blobs;
    let stream = StreamInfo {
        index: 0,
        params,
        time_base: TimeBase::new(1, 1),
        start_time: Some(0),
        duration: None,
    };
    Ok(Box::new(TgaDemuxer {
        streams: vec![stream],
        metadata,
        data: Some(buf),
    }))
}

struct TgaDemuxer {
    streams: Vec<StreamInfo>,
    /// `author` / `comment` / `date` / `job` / `job_time` / `software` /
    /// `software_version` from the extension area
    /// ([`TgaExtensionArea::metadata_pairs`]).
    metadata: Vec<(String, String)>,
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
    fn metadata(&self) -> &[(String, String)] {
        &self.metadata
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

/// One video stream, one packet: TGA holds exactly one picture. A
/// stream set of any other size — the gateway's mixed-geometry
/// `encode_frames` opens one stream per distinct picture size — is a
/// typed `Error::Unsupported` naming the geometries; so is a second
/// packet.
pub fn open_muxer(output: Box<dyn WriteSeek>, streams: &[StreamInfo]) -> Result<Box<dyn Muxer>> {
    if streams.len() != 1 {
        let geoms: Vec<String> = streams
            .iter()
            .map(|s| {
                format!(
                    "{}x{}",
                    s.params.width.unwrap_or(0),
                    s.params.height.unwrap_or(0)
                )
            })
            .collect();
        return Err(Error::unsupported(format!(
            "TGA muxer: TGA holds one picture; got {} streams ({}) — write one file per picture",
            streams.len(),
            geoms.join(", ")
        )));
    }
    if streams[0].params.media_type != MediaType::Video {
        return Err(Error::invalid("TGA muxer: stream must be video"));
    }
    Ok(Box::new(TgaMuxer {
        output,
        written: false,
    }))
}

struct TgaMuxer {
    output: Box<dyn WriteSeek>,
    /// The one packet has been written.
    written: bool,
}

impl Muxer for TgaMuxer {
    fn format_name(&self) -> &str {
        "tga"
    }
    fn write_header(&mut self) -> Result<()> {
        Ok(())
    }
    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.written {
            return Err(Error::unsupported(
                "TGA muxer: TGA holds one picture; a second packet has no carriage",
            ));
        }
        self.output.write_all(&packet.data)?;
        self.written = true;
        Ok(())
    }
    fn write_trailer(&mut self) -> Result<()> {
        Ok(())
    }
}
