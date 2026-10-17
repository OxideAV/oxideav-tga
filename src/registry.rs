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
    BlobKind, CodecCapabilities, CodecId, CodecInfo, CodecParameters, CodecRegistry,
    ColorPrimaries, ColorSignal, ContainerRegistry, Decoder, Encoder, Frame, MatrixCoefficients,
    MetadataBlob, Packet, PixelFormat, RuntimeContext, TimeBase, TransferCharacteristics,
    VideoFrame, VideoPlane,
};

use crate::container;
use crate::encoder::ExtensionAreaInput;
use crate::error::TgaError;
use crate::image::{ColorInfo, ColorRange, Palette, Plane, TgaImage, TgaPixelFormat};
use crate::options::{DecodeOptions, EncodeOptions};
use crate::types::{TgaDeveloperArea, TgaExtensionArea};

/// The `tga-extension-area` blob kind (see [`crate::BLOB_KIND_EXTENSION_AREA`]).
pub fn extension_area_blob_kind() -> BlobKind {
    BlobKind::from_static(crate::BLOB_KIND_EXTENSION_AREA)
}

/// The `tga-developer-area` blob kind (see [`crate::BLOB_KIND_DEVELOPER_AREA`]).
pub fn developer_area_blob_kind() -> BlobKind {
    BlobKind::from_static(crate::BLOB_KIND_DEVELOPER_AREA)
}

/// The TGA 2.0 structures the encoder re-emits from the blobs it was
/// handed: `frame` blobs first (they refine the stream's), then
/// `stream` blobs. An extension-area blob that does not parse, or a
/// developer-area blob that does not unpack, is a typed `InvalidData`.
fn extension_from_blobs(
    stream: &[MetadataBlob],
    frame: &[MetadataBlob],
) -> oxideav_core::Result<Option<ExtensionAreaInput>> {
    let find = |kind: &BlobKind| {
        frame
            .iter()
            .find(|b| b.is(kind))
            .or_else(|| stream.iter().find(|b| b.is(kind)))
    };
    let mut ext = match find(&extension_area_blob_kind()) {
        Some(blob) => {
            let area = TgaExtensionArea::from_bytes(&blob.data).ok_or_else(|| {
                oxideav_core::Error::invalid(format!(
                    "TGA encoder: `{}` blob holds {} bytes; the §C.6 extension area is {} bytes",
                    crate::BLOB_KIND_EXTENSION_AREA,
                    blob.data.len(),
                    crate::types::TGA_EXTENSION_AREA_SIZE
                ))
            })?;
            Some(ExtensionAreaInput::from_area(&area))
        }
        None => None,
    };
    if let Some(blob) = find(&developer_area_blob_kind()) {
        let tags = TgaDeveloperArea::tags_from_portable_bytes(&blob.data).ok_or_else(|| {
            oxideav_core::Error::invalid(format!(
                "TGA encoder: `{}` blob is not a portable developer area (truncated directory or payload)",
                crate::BLOB_KIND_DEVELOPER_AREA
            ))
        })?;
        if !tags.is_empty() {
            ext.get_or_insert_with(ExtensionAreaInput::default)
                .developer_tags = tags;
        }
    }
    Ok(ext)
}

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

/// Every framework layout the registry encoder accepts — what
/// `CodecCapabilities::accepted_pixel_formats` declares, so the
/// gateway's ladder offers exactly these and nothing is converted
/// silently. The RGB-ordered ones map 1:1 onto [`TgaPixelFormat`]; the
/// BGR / alpha-first orders (`Bgra`, `Bgr24`, `Argb`, `Abgr`) are byte
/// swizzles of the same two true-colour layouts and reach the wire at
/// the same depth (`Bgra` / `Bgr24` *are* the on-disk order). Any other
/// layout is a typed `Error::Unsupported` from [`make_encoder`] /
/// `send_frame` — 16-bit, planar, YCbCr, grey + alpha (TGA has no
/// carriage for a grey alpha plane; its grayscale types are 8-bit luma).
pub const ACCEPTED_PIXEL_FORMATS: [PixelFormat; 8] = [
    PixelFormat::Bgra,
    PixelFormat::Rgba,
    PixelFormat::Argb,
    PixelFormat::Abgr,
    PixelFormat::Bgr24,
    PixelFormat::Rgb24,
    PixelFormat::Pal8,
    PixelFormat::Gray8,
];

/// Byte order of a true-colour framework pixel relative to the TGA
/// in-memory order (`Rgb24` / `Rgba`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Swizzle {
    /// Already `R G B [A]`.
    Identity,
    /// `B G R`.
    Bgr,
    /// `B G R A`.
    Bgra,
    /// `A R G B`.
    Argb,
    /// `A B G R`.
    Abgr,
}

/// The TGA layout a framework layout is encoded as, plus the swizzle
/// that turns its pixels into that layout's byte order. Typed
/// `Unsupported` for every layout outside [`ACCEPTED_PIXEL_FORMATS`].
fn accepted_layout(pf: PixelFormat) -> oxideav_core::Result<(TgaPixelFormat, Swizzle)> {
    Ok(match pf {
        PixelFormat::Rgba => (TgaPixelFormat::Rgba, Swizzle::Identity),
        PixelFormat::Bgra => (TgaPixelFormat::Rgba, Swizzle::Bgra),
        PixelFormat::Argb => (TgaPixelFormat::Rgba, Swizzle::Argb),
        PixelFormat::Abgr => (TgaPixelFormat::Rgba, Swizzle::Abgr),
        PixelFormat::Rgb24 => (TgaPixelFormat::Rgb24, Swizzle::Identity),
        PixelFormat::Bgr24 => (TgaPixelFormat::Rgb24, Swizzle::Bgr),
        PixelFormat::Gray8 => (TgaPixelFormat::Gray8, Swizzle::Identity),
        PixelFormat::Pal8 => (TgaPixelFormat::Pal8, Swizzle::Identity),
        other => {
            return Err(oxideav_core::Error::unsupported(format!(
                "TGA encoder: pixel format {other:?} has no TGA carriage; accepted: {}",
                accepted_names()
            )))
        }
    })
}

fn accepted_names() -> String {
    ACCEPTED_PIXEL_FORMATS
        .iter()
        .map(|f| format!("{f:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Re-order the visible pixels of every row of a packed plane into
/// the TGA in-memory order, in place; row padding past the visible
/// width is left alone so the stride is unchanged. `data` must hold
/// `stride × (height − 1) + width × bpp` bytes (validated by the
/// caller).
fn swizzle_plane(data: &mut [u8], stride: usize, width: usize, height: usize, swz: Swizzle) {
    let bpp = match swz {
        Swizzle::Identity => return,
        Swizzle::Bgr => 3,
        _ => 4,
    };
    for y in 0..height {
        let row = &mut data[y * stride..y * stride + width * bpp];
        for px in row.chunks_exact_mut(bpp) {
            match swz {
                Swizzle::Identity => {}
                Swizzle::Bgr | Swizzle::Bgra => px.swap(0, 2),
                Swizzle::Argb => {
                    let a = px[0];
                    px.copy_within(1..4, 0);
                    px[3] = a;
                }
                Swizzle::Abgr => {
                    let (a, b, g, r) = (px[0], px[1], px[2], px[3]);
                    px.copy_from_slice(&[r, g, b, a]);
                }
            }
        }
    }
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

/// Attach the side-channel records: the palette for `Pal8` (plus the
/// palette-alpha record when the palette has a non-opaque entry, or
/// when `alpha_record` says the colour map carried an attribute
/// channel at all) and the colour signal when it says more than TGA's
/// default.
fn stamp_frame_side_channels(frame: &mut VideoFrame, image: &TgaImage, alpha_record: bool) {
    if let (TgaPixelFormat::Pal8, Some(p)) = (image.format, &image.palette) {
        frame.set_palette(p.to_rgb());
        if alpha_record || p.has_alpha() {
            frame.set_palette_alpha(p.to_alpha());
        }
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
/// plane, the palette side-channel for `Pal8` — with the palette-alpha
/// record beside it when any entry is not opaque (an all-opaque palette
/// attaches no record: no record means opaque) — and the colour-signal
/// side-channel when the image signals more than TGA's default.
pub fn image_into_video_frame(image: TgaImage, pts: Option<i64>) -> VideoFrame {
    image_into_video_frame_with(image, pts, false)
}

/// [`image_into_video_frame`] with the palette-alpha record forced on
/// for `Pal8` (`alpha_record`) — what the framework decoder does for a
/// colour map whose entries carry an attribute channel (16 / 32-bit
/// entries, §C.2) even when every entry decodes opaque, so the record
/// states what the file declared and the encoder writes the same
/// entry width back.
pub fn image_into_video_frame_with(
    mut image: TgaImage,
    pts: Option<i64>,
    alpha_record: bool,
) -> VideoFrame {
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
    stamp_frame_side_channels(&mut frame, &image, alpha_record);
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
    /// `pixel_format` defaults to `Rgba`). Every layout in
    /// [`ACCEPTED_PIXEL_FORMATS`] is taken: the BGR / alpha-first
    /// orders are swizzled into `Rgb24` / `Rgba`; anything else is
    /// `Error::Unsupported`. For `Pal8` the palette comes from the
    /// frame's palette side-channel and its alpha from the
    /// palette-alpha record (entries the record does not cover are
    /// opaque; no record means opaque); the frame's colour-signal
    /// side-channel, when attached, becomes `color`. The geometry is
    /// validated.
    pub fn from_video_frame(frame: &VideoFrame, params: &CodecParameters) -> crate::Result<Self> {
        let width = params
            .width
            .ok_or_else(|| TgaError::invalid("TGA: missing width"))?;
        let height = params
            .height
            .ok_or_else(|| TgaError::invalid("TGA: missing height"))?;
        let (pix, swz) = accepted_layout(params.pixel_format.unwrap_or(PixelFormat::Rgba))
            .map_err(|e| TgaError::unsupported(e.to_string()))?;
        let plane = frame
            .image_planes()
            .first()
            .ok_or_else(|| TgaError::invalid("TGA: frame has no planes"))?;
        // `new` validates the geometry first, so a short frame is a
        // typed error before any row is touched.
        let mut img = TgaImage::new(
            width,
            height,
            pix,
            vec![Plane::new(plane.stride, plane.data.clone())],
        )?;
        if swz != Swizzle::Identity {
            let stride = plane.stride;
            swizzle_plane(
                &mut img.planes[0].data,
                stride,
                width as usize,
                height as usize,
                swz,
            );
        }
        if pix == TgaPixelFormat::Pal8 {
            img.palette = frame
                .palette()
                .map(|rgb| Palette::from_rgb(rgb, frame.palette_alpha()));
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
/// The frame carries the decoded samples in the file's native layout,
/// exactly as [`crate::decode`] returns them — `Gray8` for image types
/// 3 / 11, `Pal8` plus the palette side-channel for the colour-mapped
/// types, `Rgb24` for 24-bit true colour, `Rgba` for 32-bit (and the
/// 15 / 16-bit pixels, which have no core layout and expand as
/// `decode` does) — which is what the container's stream parameters
/// declare; nothing is pre-converted to `Rgba` (use `oxideav-pixfmt`,
/// or [`crate::decode_rgba8`] standalone), the colour signal is not
/// stamped (TGA defines no colour space; the sRGB-like default is a
/// documented convention on [`crate::ColorInfo::tga_default`]), and
/// no §C.6 metadata is applied. A consumer
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
    /// the raw samples are emitted in the native layout.
    display: Option<crate::display::TgaDisplayOptions>,
}

impl Decoder for TgaDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn send_packet(&mut self, packet: &Packet) -> oxideav_core::Result<()> {
        let image = match self.display {
            Some(opts) => crate::display::decode_tga_for_display(&packet.data, &opts)?,
            None => crate::decoder::decode_image(&packet.data, &DecodeOptions::default())?,
        };
        // A colour map with 16 / 32-bit entries carries an attribute
        // channel (§C.2); the frame states so through the palette-alpha
        // record even when every entry decodes opaque.
        let map_has_alpha = crate::header(&packet.data)
            .is_ok_and(|h| h.cmap_type == 1 && matches!(h.cmap_entry_size, 16 | 32));
        let mut frame = image_into_video_frame_with(image, packet.pts, map_has_alpha);
        // The picture's own TGA 2.0 structures ride the frame record
        // (the demuxer put the same on the stream; a frame blob refines
        // a stream blob of its kind).
        let (blobs, _) = container::file_metadata(&packet.data);
        if !blobs.is_empty() {
            frame.set_blobs(blobs);
        }
        self.pending = Some(frame);
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

/// Factory registered with the codec registry: one frame in any layout
/// of [`ACCEPTED_PIXEL_FORMATS`] in, one complete TGA file out, written
/// with [`EncodeOptions::default`] (RLE, top-down) through
/// [`crate::encode`]. A `pixel_format` outside the accepted set is a
/// typed `Error::Unsupported` here, before any frame is sent; a `Pal8`
/// frame carrying a palette-alpha record is written with a 32-bit
/// colour map (the record's alpha in the attribute byte), one without
/// a record with a 24-bit map. TGA holds one picture: a second frame
/// is `Error::Unsupported`.
pub fn make_encoder(params: &CodecParameters) -> oxideav_core::Result<Box<dyn Encoder>> {
    let mut out_params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    out_params.width = params.width;
    out_params.height = params.height;
    // The stream decodes to the TGA in-memory layout: the label says
    // what a decoder of the written file emits, not the input's byte
    // order.
    out_params.pixel_format = match params.pixel_format {
        Some(pf) => Some(to_core_pixel_format(accepted_layout(pf)?.0)),
        None => None,
    };
    Ok(Box::new(TgaEncoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        in_params: params.clone(),
        out_params,
        opts: EncodeOptions::default(),
        pending: None,
        sent: false,
        eof: false,
    }))
}

struct TgaEncoder {
    codec_id: CodecId,
    /// The caller's parameters: the input layout (`accepted_layout`
    /// decides the swizzle) and geometry.
    in_params: CodecParameters,
    out_params: CodecParameters,
    opts: EncodeOptions,
    pending: Option<Vec<u8>>,
    /// A frame has been accepted — TGA holds exactly one picture.
    sent: bool,
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
        if self.in_params.pixel_format.is_none() {
            return Err(oxideav_core::Error::invalid(
                "TGA encoder: pixel_format missing in CodecParameters",
            ));
        }
        if self.sent {
            return Err(oxideav_core::Error::unsupported(
                "TGA encoder: TGA holds one picture; a second frame has no carriage \
                 (write one file per picture)",
            ));
        }
        let image = TgaImage::from_video_frame(vf, &self.in_params)?;
        let mut opts = self.opts.clone();
        // Metadata: the extension / developer areas handed as blobs
        // (frame record first, then the stream parameters) are written
        // back; an explicit `EncodeOptions::extension` wins.
        if opts.extension.is_none() {
            opts.extension = extension_from_blobs(&self.in_params.blobs, &vf.blobs())?;
        }
        // The record decides the colour-map entry width when the
        // caller pinned none: a record means the map carries an
        // attribute byte, no record means a 24-bit map.
        if image.format == TgaPixelFormat::Pal8 && opts.palette_entry_size.is_none() {
            opts.palette_entry_size = Some(if vf.palette_alpha().is_some() {
                crate::types::ColorMapEntrySize::Bits32
            } else {
                crate::types::ColorMapEntrySize::Bits24
            });
        }
        self.pending = Some(crate::encoder::encode_image(&image, &opts)?);
        self.sent = true;
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
        .with_pixel_formats(ACCEPTED_PIXEL_FORMATS.to_vec());
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
            TgaImage::from_rgba8(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap(),
            TgaImage::from_rgb8(2, 1, vec![1, 2, 3, 4, 5, 6]).unwrap(),
            TgaImage::from_gray8(2, 1, vec![9, 8]).unwrap(),
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

    /// One packet through `make_decoder`, as the framework sees it.
    fn decode_via_registry(bytes: Vec<u8>) -> VideoFrame {
        let params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
        let mut dec = make_decoder(&params).unwrap();
        dec.send_packet(&Packet::new(0, TimeBase::new(1, 1), bytes))
            .unwrap();
        match dec.receive_frame().unwrap() {
            Frame::Video(v) => v,
            _ => panic!("non-video frame"),
        }
    }

    #[test]
    fn decoder_and_demuxer_use_the_native_layout_and_do_not_stamp_colour() {
        use crate::{encode, EncodeOptions};
        let pal = Palette::new(vec![[1, 2, 3, 255], [4, 5, 6, 255]]);
        let cases = [
            (
                TgaImage::from_rgba8(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap(),
                PixelFormat::Rgba,
                8usize,
            ),
            (
                TgaImage::from_rgb8(2, 1, vec![1, 2, 3, 4, 5, 6]).unwrap(),
                PixelFormat::Rgb24,
                6,
            ),
            (
                TgaImage::from_gray8(2, 1, vec![9, 8]).unwrap(),
                PixelFormat::Gray8,
                2,
            ),
            (
                TgaImage::new_indexed(2, 1, vec![0, 1], pal).unwrap(),
                PixelFormat::Pal8,
                2,
            ),
        ];
        let ctx = oxideav_core::RuntimeContext::new();
        for (img, core, stride) in cases {
            let bytes = encode(&img, &EncodeOptions::default()).unwrap();
            let v = decode_via_registry(bytes.clone());
            assert_eq!(v.image_plane_count(), 1);
            assert_eq!(v.planes[0].stride, stride, "{core:?} native stride");
            assert_eq!(v.planes[0].data, img.data(), "{core:?} native bytes");
            if core == PixelFormat::Pal8 {
                assert_eq!(v.palette(), Some(&[1u8, 2, 3, 4, 5, 6][..]));
            } else {
                assert!(v.palette().is_none());
            }
            assert!(
                v.color_signal().is_none(),
                "TGA colour is a convention, never stamped"
            );
            let dmx = container::open_demuxer(Box::new(std::io::Cursor::new(bytes)), &ctx.codecs)
                .unwrap();
            assert_eq!(dmx.streams()[0].params.pixel_format, Some(core));
        }
    }

    #[test]
    fn unsupported_core_format_is_rejected() {
        assert!(from_core_pixel_format(PixelFormat::Yuv420P).is_err());
        assert!(TgaPixelFormat::try_from(PixelFormat::Nv12).is_err());
        assert_eq!(PixelFormat::from(TgaPixelFormat::Pal8), PixelFormat::Pal8);
    }
}
