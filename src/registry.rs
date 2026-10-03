//! `oxideav-core` integration layer for `oxideav-tga`.
//!
//! Gated behind the default-on `registry` feature so image-library
//! consumers can depend on `oxideav-tga` with `default-features = false`
//! and skip the `oxideav-core` dependency entirely.
//!
//! The module exposes:
//! * [`register`] (the fleet `RuntimeContext` entry point, also what
//!   the `oxideav_core::register!` macro dispatches),
//!   [`register_codecs`] / [`register_containers`] /
//!   [`register_registries`] for callers holding the sub-registries.
//! * [`make_decoder`] / [`make_encoder`] — the codec factories; the
//!   framework `Decoder` / `Encoder` are thin adapters over
//!   [`crate::decode`] / [`crate::encode`] (one implementation).
//! * The frame bridge: `From<TgaImage> for VideoFrame`,
//!   [`TgaImage::from_video_frame`] and
//!   `TryFrom<(&VideoFrame, &CodecParameters)>`, plus the 1:1
//!   [`TgaPixelFormat`] ↔ `oxideav_core::PixelFormat` name mapping.
//! * The `From<TgaError> for oxideav_core::Error` conversion.

use oxideav_core::{
    CodecCapabilities, CodecId, CodecInfo, CodecParameters, CodecRegistry, ColorPrimaries,
    ColorSignal, ContainerRegistry, Decoder, Encoder, Frame, MatrixCoefficients, Packet,
    PixelFormat, RuntimeContext, TimeBase, TransferCharacteristics, VideoFrame, VideoPlane,
};

use crate::container;
use crate::error::TgaError;
use crate::image::{ColorInfo, ColorRange, Palette, Plane, TgaImage, TgaPixelFormat};
use crate::options::EncodeOptions;

/// Convert a [`TgaError`] into the framework-shared `oxideav_core::Error`
/// so trait impls in this crate can use `?` on errors returned by the
/// framework-free decode/encode functions.
impl From<TgaError> for oxideav_core::Error {
    fn from(e: TgaError) -> Self {
        match e {
            TgaError::InvalidData(s) => oxideav_core::Error::InvalidData(s),
            TgaError::Unsupported(s) => oxideav_core::Error::Unsupported(s),
            TgaError::LimitExceeded(s) => oxideav_core::Error::InvalidData(s),
            TgaError::Io(e) => oxideav_core::Error::Io(e),
        }
    }
}

// ---- Pixel-format and colour mapping (1:1 by name) ----

/// The 1:1 name mapping from the framework enum to [`TgaPixelFormat`].
pub fn from_core_pixel_format(pf: PixelFormat) -> oxideav_core::Result<TgaPixelFormat> {
    Ok(match pf {
        PixelFormat::Rgba => TgaPixelFormat::Rgba,
        PixelFormat::Rgb24 => TgaPixelFormat::Rgb24,
        PixelFormat::Gray8 => TgaPixelFormat::Gray8,
        PixelFormat::Pal8 => TgaPixelFormat::Pal8,
        other => {
            return Err(oxideav_core::Error::unsupported(format!(
                "TGA: pixel format {other:?} not supported"
            )))
        }
    })
}

/// The 1:1 name mapping from [`TgaPixelFormat`] to the framework enum.
pub fn to_core_pixel_format(pf: TgaPixelFormat) -> PixelFormat {
    match pf {
        TgaPixelFormat::Rgba => PixelFormat::Rgba,
        TgaPixelFormat::Rgb24 => PixelFormat::Rgb24,
        TgaPixelFormat::Gray8 => PixelFormat::Gray8,
        TgaPixelFormat::Pal8 => PixelFormat::Pal8,
    }
}

impl From<TgaPixelFormat> for PixelFormat {
    fn from(pf: TgaPixelFormat) -> Self {
        to_core_pixel_format(pf)
    }
}

impl TryFrom<PixelFormat> for TgaPixelFormat {
    type Error = oxideav_core::Error;
    fn try_from(pf: PixelFormat) -> oxideav_core::Result<Self> {
        from_core_pixel_format(pf)
    }
}

/// [`ColorInfo`] as the framework's [`ColorSignal`] (code points map
/// 1:1; `Unspecified` range stays unspecified).
pub fn to_color_signal(c: &ColorInfo) -> ColorSignal {
    let range = match c.range {
        ColorRange::Unspecified => oxideav_core::ColorRange::Unspecified,
        ColorRange::Limited => oxideav_core::ColorRange::Limited,
        ColorRange::Full => oxideav_core::ColorRange::Full,
    };
    ColorSignal::new(
        range,
        ColorPrimaries(c.primaries),
        TransferCharacteristics(c.transfer),
        MatrixCoefficients(c.matrix),
    )
}

/// The inverse of [`to_color_signal`].
pub fn from_color_signal(s: &ColorSignal) -> ColorInfo {
    let range = match s.range {
        oxideav_core::ColorRange::Limited => ColorRange::Limited,
        oxideav_core::ColorRange::Full => ColorRange::Full,
        _ => ColorRange::Unspecified,
    };
    ColorInfo::new(range, s.primaries.0, s.transfer.0, s.matrix.0)
}

// ---- Frame bridge ----

fn stamp_frame_side_channels(frame: &mut VideoFrame, image: &TgaImage) {
    if let (TgaPixelFormat::Pal8, Some(p)) = (image.format, &image.palette) {
        frame.set_palette(p.to_rgb());
    }
    let c = image.color;
    if c.primaries != ColorInfo::UNSPECIFIED
        || c.transfer != ColorInfo::UNSPECIFIED
        || c.range == ColorRange::Limited
    {
        frame.set_color_signal(to_color_signal(&c));
    }
}

/// [`TgaImage`] → `VideoFrame` with `pts` stamped: the single packed
/// plane, the palette side-channel for `Pal8` (RGB only — the
/// framework's palette record has no alpha), and the colour-signal
/// side-channel when the image signals more than TGA's default.
pub fn image_into_video_frame(mut image: TgaImage, pts: Option<i64>) -> VideoFrame {
    let stride = image.stride();
    let data = if image.planes.is_empty() {
        Vec::new()
    } else {
        std::mem::take(&mut image.planes[0].data)
    };
    let mut frame = VideoFrame {
        pts,
        planes: vec![VideoPlane { stride, data }],
    };
    stamp_frame_side_channels(&mut frame, &image);
    frame
}

impl From<TgaImage> for VideoFrame {
    /// The pixel plane (`pts` `None`) plus the side-channels; see
    /// [`image_into_video_frame`].
    fn from(image: TgaImage) -> Self {
        image_into_video_frame(image, None)
    }
}

impl From<&TgaImage> for VideoFrame {
    fn from(image: &TgaImage) -> Self {
        image_into_video_frame(image.clone(), None)
    }
}

impl TgaImage {
    /// Rebuild an image from a framework frame and the stream
    /// parameters that describe it (`width`, `height` required;
    /// `pixel_format` defaults to `Rgba`). For `Pal8` the palette comes
    /// from the frame's palette side-channel (opaque entries); the
    /// frame's colour-signal side-channel, when attached, becomes
    /// `color`. The geometry is validated.
    pub fn from_video_frame(frame: &VideoFrame, params: &CodecParameters) -> crate::Result<Self> {
        let width = params
            .width
            .ok_or_else(|| TgaError::invalid("TGA: missing width"))?;
        let height = params
            .height
            .ok_or_else(|| TgaError::invalid("TGA: missing height"))?;
        let pix = from_core_pixel_format(params.pixel_format.unwrap_or(PixelFormat::Rgba))
            .map_err(|e| TgaError::unsupported(e.to_string()))?;
        let plane = frame
            .image_planes()
            .first()
            .ok_or_else(|| TgaError::invalid("TGA: frame has no planes"))?;
        let mut img = TgaImage::new(
            width,
            height,
            pix,
            vec![Plane::new(plane.stride, plane.data.clone())],
        )?;
        if pix == TgaPixelFormat::Pal8 {
            img.palette = frame.palette().map(|rgb| Palette::from_rgb(rgb, None));
            img.validate()?;
        }
        if let Some(sig) = frame.color_signal() {
            img.color = from_color_signal(&sig);
        }
        Ok(img)
    }
}

impl TryFrom<(&VideoFrame, &CodecParameters)> for TgaImage {
    type Error = TgaError;
    fn try_from((frame, params): (&VideoFrame, &CodecParameters)) -> crate::Result<Self> {
        TgaImage::from_video_frame(frame, params)
    }
}

// ---- Decoder trait impl + factory ----

/// Factory registered with the codec registry. Consumes one packet per
/// whole TGA file and produces one frame.
///
/// The frame carries the decoded samples in the pre-contract framework
/// layout — `Gray8` for image types 3 / 11, packed `Rgba` for every
/// other type (palette and 15 / 16 / 24-bit pixels expanded through
/// [`TgaImage::to_rgba8`]) — which is what the container's stream
/// parameters declare, and no §C.6 metadata is applied. A consumer
/// that wants the file's own gamma / colour-correction /
/// attributes-type / key-colour / pixel-aspect metadata applied builds
/// the decoder with [`make_decoder_with_display_options`] (or runs
/// [`crate::decode_tga_for_display`] on the packet bytes).
pub fn make_decoder(_params: &CodecParameters) -> oxideav_core::Result<Box<dyn Decoder>> {
    Ok(Box::new(TgaDecoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        pending: None,
        eof: false,
        display: None,
    }))
}

/// Factory variant that finalizes each frame through the composed
/// [`crate::decode_tga_for_display`] pipeline using the supplied
/// [`crate::TgaDisplayOptions`].
///
/// Use [`crate::TgaDisplayOptions::default`] for the spec-faithful
/// "display this file" behaviour (alpha resolution → tone curve → key
/// colour → pixel-aspect resample). Unlike the raw [`make_decoder`],
/// the produced frame may differ in geometry (pixel-aspect resampling)
/// and alpha from the on-disk samples; this is the intended display
/// behaviour, kept behind an explicit factory so the default codec
/// path stays a raw passthrough.
pub fn make_decoder_with_display_options(
    options: crate::display::TgaDisplayOptions,
) -> oxideav_core::Result<Box<dyn Decoder>> {
    Ok(Box::new(TgaDecoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        pending: None,
        eof: false,
        display: Some(options),
    }))
}

struct TgaDecoder {
    codec_id: CodecId,
    pending: Option<VideoFrame>,
    eof: bool,
    /// When `Some`, each decoded frame is finalized through
    /// [`crate::decode_tga_for_display`] with these options; when `None`
    /// the raw samples are emitted in the legacy layout.
    display: Option<crate::display::TgaDisplayOptions>,
}

impl Decoder for TgaDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn send_packet(&mut self, packet: &Packet) -> oxideav_core::Result<()> {
        let image = match self.display {
            Some(opts) => crate::display::decode_tga_for_display(&packet.data, &opts)?,
            None => crate::decoder::decode_legacy(&packet.data)?,
        };
        self.pending = Some(image_into_video_frame(image, packet.pts));
        Ok(())
    }
    fn receive_frame(&mut self) -> oxideav_core::Result<Frame> {
        match self.pending.take() {
            Some(f) => Ok(Frame::Video(f)),
            None => {
                if self.eof {
                    Err(oxideav_core::Error::Eof)
                } else {
                    Err(oxideav_core::Error::NeedMore)
                }
            }
        }
    }
    fn flush(&mut self) -> oxideav_core::Result<()> {
        self.eof = true;
        Ok(())
    }
}

// ---- Encoder trait impl + factory ----

/// Factory registered with the codec registry: one `Rgba` / `Rgb24` /
/// `Gray8` / `Pal8` frame in, one complete TGA file out, written with
/// [`EncodeOptions::default`] (RLE, top-down) through [`crate::encode`].
pub fn make_encoder(params: &CodecParameters) -> oxideav_core::Result<Box<dyn Encoder>> {
    let mut out_params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    out_params.width = params.width;
    out_params.height = params.height;
    out_params.pixel_format = params.pixel_format;
    Ok(Box::new(TgaEncoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        out_params,
        opts: EncodeOptions::default(),
        pending: None,
        eof: false,
    }))
}

struct TgaEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    opts: EncodeOptions,
    pending: Option<Vec<u8>>,
    eof: bool,
}

impl Encoder for TgaEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn output_params(&self) -> &CodecParameters {
        &self.out_params
    }
    fn send_frame(&mut self, frame: &Frame) -> oxideav_core::Result<()> {
        let vf = match frame {
            Frame::Video(v) => v,
            _ => {
                return Err(oxideav_core::Error::invalid(
                    "TGA encoder: expected video frame",
                ))
            }
        };
        if self.out_params.pixel_format.is_none() {
            return Err(oxideav_core::Error::invalid(
                "TGA encoder: pixel_format missing in CodecParameters",
            ));
        }
        let image = TgaImage::from_video_frame(vf, &self.out_params)?;
        self.pending = Some(crate::encoder::encode_image(&image, &self.opts)?);
        Ok(())
    }
    fn receive_packet(&mut self) -> oxideav_core::Result<Packet> {
        match self.pending.take() {
            Some(bytes) => {
                let mut pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
                pkt.flags.keyframe = true;
                Ok(pkt)
            }
            None => {
                if self.eof {
                    Err(oxideav_core::Error::Eof)
                } else {
                    Err(oxideav_core::Error::NeedMore)
                }
            }
        }
    }
    fn flush(&mut self) -> oxideav_core::Result<()> {
        self.eof = true;
        Ok(())
    }
}

// ---- Registration ----

/// Register the TGA codec into the supplied [`CodecRegistry`].
pub fn register_codecs(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("tga_sw")
        .with_intra_only(true)
        .with_lossless(true)
        .with_max_size(65535, 65535)
        .with_pixel_formats(vec![
            PixelFormat::Rgba,
            PixelFormat::Rgb24,
            PixelFormat::Gray8,
            PixelFormat::Pal8,
        ]);
    reg.register(
        CodecInfo::new(CodecId::new(crate::CODEC_ID_STR))
            .capabilities(caps)
            .decoder(make_decoder)
            .encoder(make_encoder),
    );
}

/// Register the TGA container demuxer + muxer + extension + probe
/// into the supplied [`ContainerRegistry`].
pub fn register_containers(reg: &mut ContainerRegistry) {
    container::register(reg);
}

/// Combined registration for callers holding the two sub-registries
/// rather than a [`RuntimeContext`] (the pre-contract two-argument
/// `register`).
pub fn register_registries(codecs: &mut CodecRegistry, containers: &mut ContainerRegistry) {
    register_codecs(codecs);
    register_containers(containers);
}

/// Unified registration entry point — installs the TGA codec into the
/// codec sub-registry and the TGA container into the container
/// sub-registry of the supplied [`RuntimeContext`]. This is the form
/// `oxideav_meta::register_all` dispatches via the
/// [`oxideav_core::register!`] macro.
pub fn register(ctx: &mut RuntimeContext) {
    register_registries(&mut ctx.codecs, &mut ctx.containers);
}

/// The pre-contract name of [`register`].
#[deprecated(note = "use oxideav_tga::register(&mut RuntimeContext) (IMAGE_CRATE_API)")]
pub fn register_runtime(ctx: &mut RuntimeContext) {
    register(ctx);
}

oxideav_core::register!("tga", register);

#[cfg(test)]
mod runtime_entry_tests {
    use super::*;

    #[test]
    fn oxideav_entry_installs_codec_and_container() {
        let mut ctx = oxideav_core::RuntimeContext::new();
        __oxideav_entry(&mut ctx);
        assert!(
            ctx.codecs.decoder_ids().next().is_some(),
            "__oxideav_entry should install codec decoder factories"
        );
        assert_eq!(
            ctx.containers.container_for_extension("tga"),
            Some("tga"),
            "__oxideav_entry should install the .tga extension hint"
        );
    }

    #[test]
    fn frame_bridge_round_trips_every_layout() {
        let pal = Palette::new(vec![[1, 2, 3, 255], [4, 5, 6, 255]]);
        let imgs = vec![
            TgaImage::from_rgba8(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]),
            TgaImage::from_rgb8(2, 1, vec![1, 2, 3, 4, 5, 6]),
            TgaImage::from_gray8(2, 1, vec![9, 8]),
            TgaImage::new_indexed(2, 1, vec![0, 1], pal).unwrap(),
        ];
        for img in imgs {
            let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
            params.width = Some(img.width);
            params.height = Some(img.height);
            params.pixel_format = Some(img.format.into());
            let frame: VideoFrame = img.clone().into();
            let back = TgaImage::try_from((&frame, &params)).unwrap();
            assert_eq!(back.format, img.format);
            assert_eq!(back.data(), img.data());
            assert_eq!(back.to_rgba8(), img.to_rgba8());
        }
    }

    #[test]
    fn unsupported_core_format_is_rejected() {
        assert!(from_core_pixel_format(PixelFormat::Yuv420P).is_err());
        assert!(TgaPixelFormat::try_from(PixelFormat::Nv12).is_err());
        assert_eq!(PixelFormat::from(TgaPixelFormat::Pal8), PixelFormat::Pal8);
    }
}
