//! TGA decode. Always normalises to top-left origin and produces
//! either [`TgaPixelFormat::Rgba`] (types 1 / 2 / 9 / 10) or
//! [`TgaPixelFormat::Gray8`] (types 3 / 11) — palette lookup, BGR→RGB
//! swapping, and 5-5-5 expansion all happen at decode time so consumers
//! don't have to know the on-disk quirks.
//!
//! Supported (clean-room from TGA 2.0 spec §C):
//!
//! * Type 1 — uncompressed colour-mapped (palette indices, 8 bpp).
//!   Palette entry size 15 / 16 / 24 / 32 bits.
//! * Type 2 — uncompressed true-colour, 15 / 16 / 24 / 32 bpp.
//! * Type 3 — uncompressed grayscale, 8 bpp.
//! * Type 9 — RLE colour-mapped (RLE on 8-bit indices).
//! * Type 10 — RLE true-colour, 15 / 16 / 24 / 32 bpp.
//! * Type 11 — RLE grayscale, 8 bpp.
//!
//! Optional 26-byte TGA 2.0 footer is recognised. The extension area
//! (gamma, software ID + version, postage-stamp thumbnail, attributes
//! type, …) is parsed by [`parse_tga_extension_area`]; the embedded
//! thumbnail is decoded by [`parse_tga_postage_stamp`].
//!
//! Both image-descriptor ordering bits are honoured per the TGA 2.0 FFS
//! image-descriptor field (Table 2 - Image Origin): bit 5 selects
//! top-to-bottom vs bottom-to-top row order and bit 4 selects
//! left-to-right vs right-to-left column order. The decoder normalises
//! every combination to a top-down, left-to-right output, flipping rows
//! and/or mirroring columns as the descriptor dictates.
//!
//! With the default `registry` feature on, the gated `TgaDecoder` trait
//! impl wraps [`parse_tga`] for the `oxideav_core::Decoder` surface.

use crate::error::{Result, TgaError as Error};
use crate::image::{TgaImage, TgaPixelFormat};
use crate::types::{
    parse_extension_area, parse_footer, parse_header, AttributeBits, AttributesType, ColorMapType,
    GammaValue, ImageOrigin, ImageType, Interleaving, JobTime, KeyColor, PixelAspectRatio,
    PostageStamp, SoftwareVersion, TgaAsciiField, TgaAuthorComments, TgaColorMap,
    TgaColourCorrectionTable, TgaDeveloperArea, TgaExtensionArea, TgaFooter, TgaHeader,
    TgaScanLineTable, TgaTimestamp, TGA_HEADER_SIZE,
};

#[cfg(feature = "registry")]
use oxideav_core::Decoder;
#[cfg(feature = "registry")]
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, VideoFrame, VideoPlane};

/// Factory registered with the codec registry. Consumes one packet per
/// whole TGA file and produces one frame.
///
/// The frame carries the **raw** decoded samples (the [`parse_tga`]
/// contract): no §C.6 metadata is applied. A consumer that wants the
/// file's own gamma / colour-correction / attributes-type / key-colour /
/// pixel-aspect metadata applied should build the decoder with
/// [`make_decoder_with_display_options`] (or run
/// [`crate::decode_tga_for_display`] directly on the packet bytes).
#[cfg(feature = "registry")]
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
/// Use [`crate::TgaDisplayOptions::default`] for the spec-faithful "display
/// this file" behaviour (alpha resolution → tone curve → key colour →
/// pixel-aspect resample). Unlike the raw [`make_decoder`], the produced
/// frame may differ in geometry (pixel-aspect resampling) and alpha from
/// the on-disk samples; this is the intended display behaviour, kept
/// behind an explicit factory so the default codec path stays a raw
/// passthrough.
#[cfg(feature = "registry")]
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

#[cfg(feature = "registry")]
struct TgaDecoder {
    codec_id: CodecId,
    pending: Option<VideoFrame>,
    eof: bool,
    /// When `Some`, each decoded frame is finalized through
    /// [`crate::decode_tga_for_display`] with these options; when `None`
    /// the raw [`parse_tga`] samples are emitted.
    display: Option<crate::display::TgaDisplayOptions>,
}

#[cfg(feature = "registry")]
impl Decoder for TgaDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn send_packet(&mut self, packet: &Packet) -> oxideav_core::Result<()> {
        let mut image = match self.display {
            Some(opts) => crate::display::decode_tga_for_display(&packet.data, &opts)?,
            None => parse_tga(&packet.data)?,
        };
        image.pts = packet.pts;
        self.pending = Some(image_to_video_frame(image));
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

#[cfg(feature = "registry")]
fn image_to_video_frame(image: TgaImage) -> VideoFrame {
    let stride = image.stride();
    VideoFrame {
        pts: image.pts,
        planes: vec![VideoPlane {
            stride,
            data: image.data,
        }],
    }
}

// ---------------------------------------------------------------------------
// Public standalone API
// ---------------------------------------------------------------------------

/// Decode a complete TGA file into a [`TgaImage`].
///
/// Always produces a top-down image; bottom-up files (the common case)
/// are flipped at decode time. Output pixel format is `Rgba` for types
/// 1/2/9/10 and `Gray8` for types 3/11.
pub fn parse_tga(input: &[u8]) -> Result<TgaImage> {
    let header = parse_header(input).ok_or_else(|| Error::invalid("TGA: header truncated"))?;
    let image_type = ImageType::from_u8(header.image_type_raw).ok_or_else(|| {
        Error::unsupported(format!(
            "TGA: image type {} not supported",
            header.image_type_raw
        ))
    })?;
    if image_type == ImageType::None {
        return Err(Error::invalid("TGA: image_type == 0 (no image data)"));
    }

    if header.width == 0 || header.height == 0 {
        return Err(Error::invalid("TGA: zero dimension"));
    }

    // Skip optional Image ID, then read the colour map if present.
    let (palette, cursor) = read_palette(input, &header, image_type)?;

    // Validate pixel depth for the chosen image type.
    validate_depth(image_type, header.depth)?;

    // Decode the pixel array.
    let pixels = if image_type.is_rle() {
        decode_rle_pixels(&header, image_type, &input[cursor..], palette.as_deref())?
    } else {
        decode_raw_pixels(&header, image_type, &input[cursor..], palette.as_deref())?
    };

    // Decide output pixel format.
    let pixel_format = if image_type.is_grayscale() {
        TgaPixelFormat::Gray8
    } else {
        TgaPixelFormat::Rgba
    };

    // Normalise to a top-left origin. Per the TGA 2.0 FFS image-descriptor
    // field (Table 2 - Image Origin), bit 5 carries the top-to-bottom row
    // ordering and bit 4 the left-to-right column ordering. We always emit
    // top-down, left-to-right, flipping rows when bit 5 is clear (the common
    // bottom-up case) and mirroring columns when bit 4 is set (rare, but
    // spec-legal — some Truevision tooling wrote right-to-left files).
    let bpp = match pixel_format {
        TgaPixelFormat::Rgba => 4,
        TgaPixelFormat::Gray8 => 1,
        TgaPixelFormat::Rgb24 => 3,
    };
    let width = header.width as usize;
    let height = header.height as usize;
    let stride = width * bpp;
    let flip_rows = !header.is_top_down();
    let mirror_cols = header.is_right_to_left();
    let data = if !flip_rows && !mirror_cols {
        pixels
    } else {
        let mut out = vec![0u8; pixels.len()];
        for y in 0..height {
            let src_y = if flip_rows { height - 1 - y } else { y };
            let src_row = &pixels[src_y * stride..][..stride];
            let dst_row = &mut out[y * stride..][..stride];
            if mirror_cols {
                for x in 0..width {
                    let src_px = &src_row[x * bpp..][..bpp];
                    let dst_px = &mut dst_row[(width - 1 - x) * bpp..][..bpp];
                    dst_px.copy_from_slice(src_px);
                }
            } else {
                dst_row.copy_from_slice(src_row);
            }
        }
        out
    };

    Ok(TgaImage {
        width: header.width as u32,
        height: header.height as u32,
        pixel_format,
        data,
        pts: None,
    })
}

/// Recognise + parse the optional TGA 2.0 footer. Returns `None` for
/// TGA 1.0 files (no footer) or files truncated before the footer.
pub fn parse_tga_footer(input: &[u8]) -> Option<TgaFooter> {
    parse_footer(input)
}

/// Borrow the Image Identification Field bytes (spec §3.3 / §C.3) from a
/// TGA file.
///
/// The Image ID is a free-form, up-to-255-byte block written immediately
/// after the 18-byte header. Its length is the `id_length` byte at
/// header offset 0; the spec describes it as a free-form identification
/// field (typically a printable ASCII string naming the source camera /
/// scene / artist) and declines to constrain the content further, so this
/// helper returns the raw bytes verbatim — no trimming, no decoding to
/// `String`, no NUL-stripping. Callers that want a string can run
/// `String::from_utf8_lossy` over the returned slice.
///
/// Returns:
///
/// * `Some(&[])` when the header declares `id_length == 0` (the common
///   case — most encoders, including this crate's, write no Image ID).
/// * `Some(bytes)` with the borrowed slice when an Image ID is present.
/// * `None` when the input is shorter than the 18-byte header, or
///   truncated before the Image ID's full length.
pub fn parse_tga_image_id(input: &[u8]) -> Option<&[u8]> {
    let header = parse_header(input)?;
    let n = header.id_length as usize;
    let end = TGA_HEADER_SIZE + n;
    if input.len() < end {
        return None;
    }
    Some(&input[TGA_HEADER_SIZE..end])
}

/// Parse the TGA 2.0 extension-area body if the file has one.
///
/// Locates the footer + follows the `extension_area_offset` it carries,
/// returning the parsed [`TgaExtensionArea`]. Returns `None` when the
/// file has no footer, no extension area (`offset == 0`), or the
/// extension area is truncated.
pub fn parse_tga_extension_area(input: &[u8]) -> Option<TgaExtensionArea> {
    let footer = parse_footer(input)?;
    parse_extension_area(input, footer.extension_area_offset)
}

/// Parse the colour-correction table referenced by the extension area
/// (spec §C.6.8) if the file has one.
///
/// Returns `None` when:
/// * the file has no TGA 2.0 footer,
/// * the extension area is absent or truncated,
/// * the extension area's `colour_correction_offset` is `0`, or
/// * the colour-correction table itself is truncated before the full
///   `4 × 256 × 2 = 2048` bytes.
pub fn parse_tga_colour_correction_table(input: &[u8]) -> Option<TgaColourCorrectionTable> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    TgaColourCorrectionTable::parse(input, ext.colour_correction_offset)
}

/// Parse the scan-line table referenced by the extension area (spec
/// §C.6.9) if the file has one.
///
/// The number of entries equals the main image's height — this routine
/// reads the header to determine the height, then walks `height` u32
/// row-offset entries. Returns `None` when the file has no footer, no
/// extension area, no scan-line table (offset `0`), or the table is
/// truncated.
pub fn parse_tga_scan_line_table(input: &[u8]) -> Option<TgaScanLineTable> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    let header = parse_header(input)?;
    TgaScanLineTable::parse(input, ext.scan_line_offset, header.height)
}

/// Build the §C.6.9 scan-line table for an encoded TGA byte stream.
///
/// The spec provides the scan-line table (extension-area Field 25)
/// "to make random access of compressed images easy" and "to allow
/// 'giant picture' access in smaller 'chunks'": one 4-byte offset per
/// scan line, measured from the start of the file, pointing at the
/// first byte of each line **in the order the image was saved** (top
/// down or bottom up — i.e. entry order is stream order, not display
/// order). This helper derives that table from the file's own pixel
/// data:
///
/// * Uncompressed types (1 / 2 / 3): pure arithmetic —
///   `pixel_start + y × width × bytes_per_pixel`.
/// * RLE types (9 / 10 / 11): the §C.5 packet stream is walked once,
///   recording the byte position at which each row's first packet
///   begins.
///
/// The result can be handed to
/// [`crate::encoder::ExtensionAreaInput::scan_line_table`] (computed
/// against the base file *before* [`crate::encode_tga_with_extension`]
/// appends anything — the pixel data doesn't move, so the offsets stay
/// valid in the extended file), or used directly with
/// [`parse_tga_scan_line`] for random row access.
///
/// # Errors
///
/// * [`crate::TgaError::InvalidData`] for a malformed file (truncated
///   header / colour map / pixel data, zero dimension, or a file so
///   large a row offset exceeds the table's 4-byte field).
/// * [`crate::TgaError::Unsupported`] for an unsupported image type /
///   depth, **or** for an RLE file in which a packet spans a scan-line
///   boundary. The spec's packet rule — packets "should never encode
///   pixels from more than one scan line", precisely because that
///   "allows software to create and use a scan line table for rapid,
///   random access of individual lines" — is what makes the table
///   well-defined; a file that breaks the rule has rows that don't
///   start on packet boundaries, so no table can describe it. (The
///   whole-image decoder [`parse_tga`] still accepts such files.)
pub fn compute_tga_scan_line_table(input: &[u8]) -> Result<TgaScanLineTable> {
    let header = parse_header(input).ok_or_else(|| Error::invalid("TGA: header truncated"))?;
    let image_type = ImageType::from_u8(header.image_type_raw).ok_or_else(|| {
        Error::unsupported(format!(
            "TGA: image type {} not supported",
            header.image_type_raw
        ))
    })?;
    if image_type == ImageType::None {
        return Err(Error::invalid("TGA: image_type == 0 (no image data)"));
    }
    if header.width == 0 || header.height == 0 {
        return Err(Error::invalid("TGA: zero dimension"));
    }
    let (_palette, pixel_start) = read_palette(input, &header, image_type)?;
    validate_depth(image_type, header.depth)?;

    let width = header.width as usize;
    let height = header.height as usize;
    let in_bpp = header.depth.div_ceil(8) as usize;
    let mut table = TgaScanLineTable::with_capacity(header.height);
    if image_type.is_rle() {
        let mut pos = pixel_start;
        for _ in 0..height {
            table.offsets.push(scan_line_offset_u32(pos)?);
            let mut remaining = width;
            while remaining > 0 {
                if pos >= input.len() {
                    return Err(Error::invalid("TGA: RLE stream truncated (no packet hdr)"));
                }
                let packet = input[pos];
                pos += 1;
                let count = (packet & 0x7F) as usize + 1;
                if count > remaining {
                    return Err(Error::unsupported(format!(
                        "TGA: RLE packet spans a scan-line boundary (row has {remaining} \
                         pixel(s) left, packet encodes {count}); rows don't start on packet \
                         boundaries, so no §C.6.9 scan-line table can describe this file"
                    )));
                }
                let body = if (packet & 0x80) != 0 {
                    // Run-length packet: one pixel value.
                    in_bpp
                } else {
                    // Raw packet: `count` literal pixel values.
                    count * in_bpp
                };
                if pos + body > input.len() {
                    return Err(Error::invalid("TGA: RLE packet truncated"));
                }
                pos += body;
                remaining -= count;
            }
        }
    } else {
        let row_bytes = width * in_bpp;
        if input.len() < pixel_start + row_bytes * height {
            return Err(Error::invalid("TGA: pixel data truncated"));
        }
        for y in 0..height {
            table
                .offsets
                .push(scan_line_offset_u32(pixel_start + y * row_bytes)?);
        }
    }
    Ok(table)
}

/// Random-access decode of a single scan line through a §C.6.9
/// scan-line table — the "rapid, random access of individual lines"
/// the spec built the table for, without decoding (or, for RLE files,
/// even walking) any preceding row.
///
/// `index` is in **saved order**, matching the table itself (the spec
/// records offsets "in the order that the image was saved (i.e., top
/// down or bottom up)"): for a top-down file (descriptor bit 5 set)
/// saved index equals display row; for a bottom-up file entry 0 is the
/// *bottom* display row, so display row `y` lives at saved index
/// `height − 1 − y`. Use [`crate::TgaHeader::is_top_down`] to pick.
/// Columns are normalised to left-to-right exactly as [`parse_tga`]
/// does (descriptor bit 4 mirroring).
///
/// `table` is typically [`parse_tga_scan_line_table`]'s output for a
/// file that ships one, or [`compute_tga_scan_line_table`]'s for one
/// that doesn't.
///
/// Returns a 1-pixel-tall [`TgaImage`] in the same normalised pixel
/// format the whole-image decoder would produce (`Rgba` for types
/// 1/2/9/10, `Gray8` for 3/11). Errors mirror [`parse_tga`]'s, plus
/// [`crate::TgaError::InvalidData`] when `index` is outside the table
/// or the recorded offset points past the end of `input`; an RLE
/// packet at the recorded offset that encodes more pixels than the row
/// holds is rejected (the §C.5 overrun guard — a row that doesn't end
/// on a packet boundary can't be randomly accessed).
pub fn parse_tga_scan_line(
    input: &[u8],
    table: &TgaScanLineTable,
    index: usize,
) -> Result<TgaImage> {
    let header = parse_header(input).ok_or_else(|| Error::invalid("TGA: header truncated"))?;
    let image_type = ImageType::from_u8(header.image_type_raw).ok_or_else(|| {
        Error::unsupported(format!(
            "TGA: image type {} not supported",
            header.image_type_raw
        ))
    })?;
    if image_type == ImageType::None {
        return Err(Error::invalid("TGA: image_type == 0 (no image data)"));
    }
    if header.width == 0 || header.height == 0 {
        return Err(Error::invalid("TGA: zero dimension"));
    }
    let (palette, _pixel_start) = read_palette(input, &header, image_type)?;
    validate_depth(image_type, header.depth)?;

    let offset = table.get(index).ok_or_else(|| {
        Error::invalid(format!(
            "TGA: scan-line index {index} out of range (table has {} entries)",
            table.len()
        ))
    })? as usize;
    if offset > input.len() {
        return Err(Error::invalid(
            "TGA: scan-line table offset points past the end of the input",
        ));
    }

    // Decode exactly one row's worth of pixels starting at the
    // recorded offset, reusing the §C.5 packet / raw-pixel machinery
    // with a synthetic single-row header.
    let row_header = TgaHeader {
        height: 1,
        ..header
    };
    let row_input = &input[offset..];
    let mut pixels = if image_type.is_rle() {
        decode_rle_pixels(&row_header, image_type, row_input, palette.as_deref())?
    } else {
        decode_raw_pixels(&row_header, image_type, row_input, palette.as_deref())?
    };

    let pixel_format = if image_type.is_grayscale() {
        TgaPixelFormat::Gray8
    } else {
        TgaPixelFormat::Rgba
    };
    // Normalise column order (descriptor bit 4), same as parse_tga.
    // Row order (bit 5) needs no per-row work — the caller's `index`
    // already addresses the saved order.
    if header.is_right_to_left() {
        let bpp = match pixel_format {
            TgaPixelFormat::Gray8 => 1,
            _ => 4,
        };
        let w = header.width as usize;
        for x in 0..w / 2 {
            for b in 0..bpp {
                pixels.swap(x * bpp + b, (w - 1 - x) * bpp + b);
            }
        }
    }

    Ok(TgaImage {
        width: header.width as u32,
        height: 1,
        pixel_format,
        data: pixels,
        pts: None,
    })
}

/// Parse the TGA 2.0 developer-area tag directory if the file has one
/// (spec §C.7).
///
/// Returns `None` when the file has no footer, when the footer's
/// `developer_directory_offset` is `0`, or when the tag directory or
/// any of its referenced tag payloads is truncated.
///
/// Use [`TgaDeveloperArea::payload`] to borrow each tag's body bytes
/// from the same input buffer; tag-payload format is application-
/// defined and not interpreted by this parser.
pub fn parse_tga_developer_area(input: &[u8]) -> Option<TgaDeveloperArea> {
    let footer = parse_footer(input)?;
    TgaDeveloperArea::parse(input, footer.developer_directory_offset)
}

/// Read the typed alpha-channel attributes (spec §C.6.13) declared by a
/// file's extension area.
///
/// Returns the [`AttributesType`] decoded from the extension area's
/// `attributes_type` byte. A file with no TGA 2.0 footer or no extension
/// area carries no declared attributes type, so this returns `None`
/// (the caller should fall back to its own alpha convention — typically
/// treating the channel as straight/useful).
///
/// Pair this with [`AttributesType::apply_to_image`] to normalise a
/// decoded RGBA image to straight alpha: a `PremultipliedAlpha` file is
/// un-premultiplied, a `NoAlpha` / `UndefinedIgnore` file is forced
/// opaque, and the remaining types pass through unchanged.
pub fn parse_tga_attributes_type(input: &[u8]) -> Option<AttributesType> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.attributes())
}

/// Convenience helper: read the §C.6.4 [`KeyColor`] straight from the
/// extension area without separately parsing the footer + extension
/// area body. Returns `None` when the file has no TGA 2.0 footer or no
/// extension area.
pub fn parse_tga_key_color(input: &[u8]) -> Option<KeyColor> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.key_color_typed())
}

/// Convenience helper: read the §C.6.5 [`PixelAspectRatio`] straight
/// from the extension area. Returns `None` when the file has no TGA 2.0
/// footer or no extension area.
pub fn parse_tga_pixel_aspect_ratio(input: &[u8]) -> Option<PixelAspectRatio> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.pixel_aspect_ratio_typed())
}

/// Convenience helper: read the §C.6.6 [`GammaValue`] straight from the
/// extension area. Returns `None` when the file has no TGA 2.0 footer
/// or no extension area.
pub fn parse_tga_gamma(input: &[u8]) -> Option<GammaValue> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.gamma_typed())
}

/// Convenience helper: read the §C.6.7 [`SoftwareVersion`] straight
/// from the extension area. Returns `None` when the file has no TGA 2.0
/// footer or no extension area.
pub fn parse_tga_software_version(input: &[u8]) -> Option<SoftwareVersion> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.software_version_typed())
}

/// Convenience helper: read the Field 13 [`TgaTimestamp`] (Date/Time
/// Stamp) straight from the extension area. Returns `None` when the
/// file has no TGA 2.0 footer or no extension area.
///
/// The returned struct still needs [`TgaTimestamp::is_unset`] /
/// [`TgaTimestamp::is_valid`] to distinguish "no timestamp recorded"
/// from "valid datetime" — `parse_tga_timestamp` itself does no
/// validity filtering so a caller can preserve every byte the writer
/// laid down.
pub fn parse_tga_timestamp(input: &[u8]) -> Option<TgaTimestamp> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.timestamp_typed())
}

/// Convenience helper: read the Field 15 [`JobTime`] (elapsed-time
/// triple) straight from the extension area. Returns `None` when the
/// file has no TGA 2.0 footer or no extension area.
///
/// The returned struct still needs [`JobTime::is_unset`] /
/// [`JobTime::is_valid`] to distinguish the spec sentinel from a
/// recorded zero-elapsed job; the parser does no validity filtering.
pub fn parse_tga_job_time(input: &[u8]) -> Option<JobTime> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.job_time_typed())
}

/// Convenience helper: read the Field 11 [`TgaAsciiField`] (Author
/// Name) straight from the extension area. Returns `None` when the
/// file has no TGA 2.0 footer or no extension area.
///
/// The returned struct still needs [`TgaAsciiField::is_unset`] /
/// [`TgaAsciiField::is_valid_ascii`] to distinguish the spec
/// "blanks-terminated-by-null" sentinel from a meaningful payload —
/// the parser does no validity filtering.
pub fn parse_tga_author_name(input: &[u8]) -> Option<TgaAsciiField> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.author_name_typed())
}

/// Convenience helper: read the Field 12 [`TgaAuthorComments`]
/// (four-line author-comment block) straight from the extension area.
/// Returns `None` when the file has no TGA 2.0 footer or no extension
/// area.
pub fn parse_tga_author_comments(input: &[u8]) -> Option<TgaAuthorComments> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.author_comments_typed())
}

/// Convenience helper: read the Field 14 [`TgaAsciiField`] (Job
/// Name/ID) straight from the extension area. Returns `None` when the
/// file has no TGA 2.0 footer or no extension area.
pub fn parse_tga_job_name(input: &[u8]) -> Option<TgaAsciiField> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.job_name_typed())
}

/// Convenience helper: read the Field 16 [`TgaAsciiField`] (Software
/// ID) straight from the extension area. Returns `None` when the file
/// has no TGA 2.0 footer or no extension area.
pub fn parse_tga_software_id(input: &[u8]) -> Option<TgaAsciiField> {
    let footer = parse_footer(input)?;
    let ext = parse_extension_area(input, footer.extension_area_offset)?;
    Some(ext.software_id_typed())
}

/// Resolve a decoded RGBA image's alpha channel to *straight* alpha, applying
/// the documented TARGA-32 vs ARGB-32 fallback when the file carries no
/// authoritative [`AttributesType`].
///
/// Resolution rules:
///
/// 1. If the file declares an attributes type (TGA 2.0 footer + extension
///    area present, [`parse_tga_attributes_type`] returns `Some`), apply
///    [`AttributesType::apply_to_image`] in place: `PremultipliedAlpha` is
///    un-premultiplied, `NoAlpha` / `UndefinedIgnore` is forced opaque, and
///    the remaining types pass through.
/// 2. Otherwise (TGA 1.0 file, or a TGA 2.0 file with no extension area)
///    apply the convention documented in `docs/image/tga/README.md`: if every
///    alpha byte is `0`, treat the file as opaque (force every alpha byte to
///    `0xFF`); leave the image untouched otherwise. The convention reflects
///    real-world files written by paint applications that left
///    `attributes_type` unset and the alpha channel uninitialised — a naïve
///    decoder would render them as fully-transparent black even though the
///    intended interpretation is opaque.
///
/// `image.pixel_format != Rgba` is a no-op: neither
/// [`TgaPixelFormat::Rgb24`] nor [`TgaPixelFormat::Gray8`] carries an alpha
/// channel to interpret.
///
/// Returns the [`AttributesType`] applied in case (1), or `None` in
/// case (2), so the caller can tell which branch ran.
pub fn resolve_alpha_with_targa32_fallback(
    input: &[u8],
    image: &mut TgaImage,
) -> Option<AttributesType> {
    if image.pixel_format != TgaPixelFormat::Rgba {
        return parse_tga_attributes_type(input);
    }
    if let Some(attrs) = parse_tga_attributes_type(input) {
        attrs.apply_to_image(image);
        return Some(attrs);
    }
    if image.all_alpha_zero() {
        image.force_opaque();
    }
    None
}

/// Read the **§C.2 Image Descriptor** attribute-bit count (Field 5.6,
/// bits 3-0) straight from a TGA file's 18-byte header.
///
/// "These bits specify the number of attribute bits per pixel … these
/// bits indicate the number of bits per pixel which are designated as
/// Alpha Channel bits." Unlike the extension-area
/// [`AttributesType`](crate::AttributesType) (Field 24, TGA 2.0 only),
/// this declaration lives in the fixed header, so it is present in
/// **every** TGA file including TGA 1.0 — making it the header-local way
/// to learn whether a 32-bpp pixel's fourth byte (or a 16-bpp pixel's top
/// bit) carries meaningful alpha.
///
/// Returns the typed [`AttributeBits`] view, or `None` when the input is
/// shorter than the 18-byte header.
pub fn parse_tga_attribute_bits(input: &[u8]) -> Option<AttributeBits> {
    parse_header(input).map(|h| h.attribute_bits())
}

/// Read the **§C.2 Image Descriptor** data-storage interleaving flag
/// (Field 5.6, bits 7-6) straight from a TGA file's 18-byte header.
///
/// The TGA 2.0 FFS requires this field to be zero "to insure future
/// compatibility" — TGA 2.0 abandoned the earlier interleaving scheme —
/// so a conformant 2.0 file reports
/// [`Interleaving::NonInterleaved`](crate::Interleaving::NonInterleaved).
/// The earlier Truevision layout defined the field as a data-storage
/// interleaving flag (`00` non-interleaved, `01` two-way even/odd, `10`
/// four-way, `11` reserved); a legacy file that set a non-zero value is
/// surfaced verbatim by the returned [`Interleaving`] so a caller can
/// detect it. Like the field-5.6 attribute-bit count, this declaration
/// lives in the fixed header and is present in **every** TGA file
/// including TGA 1.0.
///
/// This crate does not perform a de-interleaving row reorder: the 2.0 FFS
/// removed interleaving and never documents the reorder algorithm, and no
/// in-the-wild fixtures set a non-zero value — the decoder treats the
/// pixel array as non-interleaved regardless. The view exists so a caller
/// can recognise (and choose to reject) a legacy interleaved file.
///
/// Returns the typed [`Interleaving`] view, or `None` when the input is
/// shorter than the 18-byte header.
pub fn parse_tga_interleaving(input: &[u8]) -> Option<Interleaving> {
    parse_header(input).map(|h| h.interleaving())
}

/// Read the **§5.1 / §5.2 Image Origin** (fixed-header Fields 5.1 + 5.2,
/// bytes 8-11) straight from a TGA file's 18-byte header as a typed
/// [`ImageOrigin`] view.
///
/// The two fields record "the absolute horizontal / vertical coordinate
/// for the lower left corner of the image as it is positioned on a display
/// device having an origin at the lower left of the screen (e.g., the
/// TARGA series)". Like the field-5.6 attribute-bit count and
/// interleaving flag, this declaration lives in the fixed header and is
/// present in **every** TGA file including TGA 1.0.
///
/// This is the on-screen placement coordinate, orthogonal to the
/// image-descriptor origin bits (Field 5.6 bits 4-5) the decoder uses to
/// normalise pixel storage order: a file can be stored top-down yet
/// declare a non-zero screen origin. The decoder does not relocate pixels
/// (it always emits a single normalised raster); this view lets a caller
/// that does its own on-screen compositing read and honour the requested
/// placement.
///
/// Returns the typed [`ImageOrigin`] view, or `None` when the input is
/// shorter than the 18-byte header.
pub fn parse_tga_image_origin(input: &[u8]) -> Option<ImageOrigin> {
    parse_header(input).map(|h| h.image_origin())
}

/// Resolve a decoded RGBA image's alpha channel using only the
/// **§C.2 Image Descriptor** attribute-bit count (Field 5.6, bits 3-0) —
/// the header-local counterpart to
/// [`resolve_alpha_with_targa32_fallback`].
///
/// The spec states that an extension-area Field 24 value of `0` ("no
/// Alpha data included") means "bits 3-0 of field 5.6 should also be set
/// to zero", so a header that declares **zero** attribute bits is the
/// standalone (TGA-1.0-compatible) signal that the on-disk alpha data is
/// not meaningful and the picture should display opaque. This resolver
/// reads that count from the header and, when it is zero on an RGBA
/// image, forces every alpha byte to `0xFF` ([`TgaImage::force_opaque`]).
/// A non-zero count is honoured as "the file declares useful alpha" and
/// the decoded alpha bytes are left untouched.
///
/// Distinct from [`resolve_alpha_with_targa32_fallback`], which prefers
/// the TGA 2.0 extension-area [`AttributesType`] and only falls back to
/// the *all-alpha-zero* heuristic: this resolver consults the header bits
/// alone, never reads the extension area, and never inspects pixel values
/// — so it acts even on a non-zero-but-uninitialised alpha plane when the
/// header says "no attribute bits". A caller wanting the descriptor to
/// win can run this first; one wanting the extension area to win runs the
/// other.
///
/// `image.pixel_format != Rgba` is a no-op (no alpha channel to resolve).
/// Returns the [`AttributeBits`] read from the header (`None` only when
/// the input is shorter than the 18-byte header), so the caller can tell
/// which branch ran.
pub fn resolve_alpha_from_descriptor(input: &[u8], image: &mut TgaImage) -> Option<AttributeBits> {
    let bits = parse_tga_attribute_bits(input)?;
    if image.pixel_format == TgaPixelFormat::Rgba && bits.is_none() {
        image.force_opaque();
    }
    Some(bits)
}

/// Decode the TGA 2.0 postage-stamp (thumbnail) image from a file with
/// an extension area.
///
/// The postage stamp is a tiny preview image (typically 64×64) embedded
/// at the byte offset given by [`TgaExtensionArea::postage_stamp_offset`].
/// On disk it is two `u8` size bytes (width, height) followed by
/// uncompressed pixel data using the *same* image type / pixel
/// depth / palette as the main image (spec §C.6.10): never RLE,
/// regardless of the parent image's compression.
///
/// Returns `Ok(None)` when the file has no extension area or no
/// postage stamp; `Err` for truncated / malformed thumbnails.
pub fn parse_tga_postage_stamp(input: &[u8]) -> Result<Option<TgaImage>> {
    let Some(footer) = parse_footer(input) else {
        return Ok(None);
    };
    let Some(ext) = parse_extension_area(input, footer.extension_area_offset) else {
        return Ok(None);
    };
    if ext.postage_stamp_offset == 0 {
        return Ok(None);
    }

    // Re-parse the main header so we know the original pixel format
    // and (if applicable) the colour map. The postage stamp shares
    // both with the main image.
    let header = parse_header(input).ok_or_else(|| Error::invalid("TGA: header truncated"))?;
    let image_type = ImageType::from_u8(header.image_type_raw).ok_or_else(|| {
        Error::unsupported(format!(
            "TGA: image type {} not supported (postage stamp)",
            header.image_type_raw
        ))
    })?;
    if image_type == ImageType::None {
        return Err(Error::invalid(
            "TGA: image_type == 0 (postage stamp parent has no main image)",
        ));
    }

    // Walk past the header + image ID + colour map to locate the
    // colour map for palette lookups (the stamp shares the parent's
    // palette).
    let (palette, _pixel_start) = read_palette(input, &header, image_type)?;

    let pp_off = ext.postage_stamp_offset as usize;
    if input.len() < pp_off + 2 {
        return Err(Error::invalid("TGA: postage stamp size header truncated"));
    }
    let stamp_w = input[pp_off] as u16;
    let stamp_h = input[pp_off + 1] as u16;
    if stamp_w == 0 || stamp_h == 0 {
        return Err(Error::invalid("TGA: postage stamp has zero dimension"));
    }

    // Postage stamps are always uncompressed regardless of the parent
    // image's compression. Construct a synthetic header so we can reuse
    // decode_raw_pixels().
    let stamp_header = TgaHeader {
        id_length: 0,
        cmap_type: header.cmap_type,
        image_type_raw: match image_type {
            ImageType::RleColourMapped => ImageType::UncompressedColourMapped as u8,
            ImageType::RleTrueColour => ImageType::UncompressedTrueColour as u8,
            ImageType::RleGrayscale => ImageType::UncompressedGrayscale as u8,
            other => other as u8,
        },
        cmap_first: header.cmap_first,
        cmap_length: header.cmap_length,
        cmap_entry_size: header.cmap_entry_size,
        x_origin: 0,
        y_origin: 0,
        width: stamp_w,
        height: stamp_h,
        depth: header.depth,
        descriptor: 0x20, // top-down — postage stamps don't carry their own descriptor
    };
    let stamp_image_type =
        ImageType::from_u8(stamp_header.image_type_raw).expect("converted from validated parent");

    // Validate the parent's pixel depth against the (now-derived)
    // postage-stamp image type. The parent's depth field is u8 straight
    // off the wire (the §C.2 header allows any value), and an extension
    // area can be authored with a `postage_stamp_offset` even when the
    // parent depth is nonsensical (e.g. `8` for type 2 / 10, `0` for type
    // 3 / 11). Without this guard a hostile file reaches `emit_pixel`'s
    // depth `match` with an unsupported value and trips the internal
    // `unreachable!()` — a panic, not a returned `Err`. The main-image
    // path (`parse_tga`) already calls `validate_depth` for the same
    // reason; mirror that here so every public decoder surface returns
    // `Err(TgaError::Unsupported)` for a malformed depth rather than
    // aborting the process.
    validate_depth(stamp_image_type, stamp_header.depth)?;

    let pixels_in = &input[pp_off + 2..];
    let pixels = decode_raw_pixels(
        &stamp_header,
        stamp_image_type,
        pixels_in,
        palette.as_deref(),
    )?;

    let pixel_format = if stamp_image_type.is_grayscale() {
        TgaPixelFormat::Gray8
    } else {
        TgaPixelFormat::Rgba
    };

    Ok(Some(TgaImage {
        width: stamp_w as u32,
        height: stamp_h as u32,
        pixel_format,
        data: pixels,
        pts: None,
    }))
}

/// Read just the **Field 26 (Postage Stamp Image)** dimension header
/// (spec §C.6.10) — the two leading `u8` size bytes — without decoding
/// the thumbnail's pixels.
///
/// "The first byte of the postage stamp image specifies the X size of
/// the stamp in pixels, the second byte … the Y size". This helper seeks
/// to [`TgaExtensionArea::postage_stamp_offset`] and returns those two
/// bytes wrapped in a [`PostageStamp`] so a caller can inspect the stamp
/// geometry — including Truevision's 64 × 64 recommendation via
/// [`PostageStamp::within_recommended_size`] — cheaply, e.g. to decide
/// whether to call the heavier [`parse_tga_postage_stamp`].
///
/// Returns `Ok(None)` when the file has no extension area or no postage
/// stamp (Field 22 offset zero); `Err` when the offset points past the
/// end of the buffer (size header truncated). A `(0, _)` or `(_, 0)`
/// degenerate header is returned as-is (the caller's
/// [`PostageStamp::is_unset`] surfaces it); the full decoder is the path
/// that rejects a zero-dimension stamp.
pub fn parse_tga_postage_stamp_dimensions(input: &[u8]) -> Result<Option<PostageStamp>> {
    let Some(footer) = parse_footer(input) else {
        return Ok(None);
    };
    let Some(ext) = parse_extension_area(input, footer.extension_area_offset) else {
        return Ok(None);
    };
    if ext.postage_stamp_offset == 0 {
        return Ok(None);
    }
    let off = ext.postage_stamp_offset as usize;
    if input.len() < off + 2 {
        return Err(Error::invalid("TGA: postage stamp size header truncated"));
    }
    Ok(Some(PostageStamp::new(input[off], input[off + 1])))
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Walk past the header + Image ID, decode the colour map if
/// `cmap_type == 1`, and return `(palette, pixel_data_start)` where
/// `pixel_data_start` is the absolute byte offset of the first pixel-
/// data byte (the file layout is header, then Image ID, then colour
/// map, then the pixel array).
///
/// The spec allows a colour map to be present even for non-colour-
/// mapped image types (the §C.2 Color Map Type byte only declares
/// whether map data follows), so those still read — and skip — the
/// map rather than erroring out; a colour-mapped image type with
/// `cmap_type == 0` is rejected.
fn read_palette(
    input: &[u8],
    header: &TgaHeader,
    image_type: ImageType,
) -> Result<(Option<Vec<[u8; 4]>>, usize)> {
    let mut cursor = TGA_HEADER_SIZE + header.id_length as usize;
    if input.len() < cursor {
        return Err(Error::invalid("TGA: truncated past image ID"));
    }
    let palette = if header.cmap_type == 1 {
        if header.cmap_entry_size == 0 || header.cmap_length == 0 {
            return Err(Error::invalid(
                "TGA: cmap_type == 1 but cmap_entry_size / cmap_length == 0",
            ));
        }
        let entry_bytes = header.cmap_entry_size.div_ceil(8) as usize;
        let cmap_bytes = entry_bytes * header.cmap_length as usize;
        if input.len() < cursor + cmap_bytes {
            return Err(Error::invalid("TGA: colour map truncated"));
        }
        let cmap = &input[cursor..cursor + cmap_bytes];
        cursor += cmap_bytes;
        Some(decode_palette(cmap, header.cmap_entry_size, entry_bytes)?)
    } else {
        if image_type.is_colour_mapped() {
            return Err(Error::invalid(
                "TGA: colour-mapped image type but cmap_type == 0",
            ));
        }
        None
    };
    Ok((palette, cursor))
}

/// Parse a TGA file's **Color Map** (the on-disk palette declared by the
/// header's Color Map Specification, spec §C.2 / Data Type 1 header
/// bytes 3-7) into a typed [`TgaColorMap`].
///
/// Unlike [`parse_tga`] — which decodes the full raster and so rejects
/// **Data Type 0 (No Image Data)** files — this helper only reads the
/// header and the color-map block, so it works on a palette-only type-0
/// TGA (a header + color map with no pixel array) as well as on a
/// colour-mapped image whose palette a caller wants to inspect directly.
///
/// Returns:
/// * `Ok(None)` when the header's Color Map Type byte is `0` (no color
///   map present) — including a true-colour or grayscale file.
/// * `Ok(Some(map))` with the de-interleaved palette (15/16-bit
///   A1R5G5B5, 24-bit BGR, or 32-bit BGRA expanded to straight RGBA,
///   matching [`parse_tga`]'s internal palette expansion) and the
///   Color Map Origin recorded in [`TgaColorMap::first_index`].
/// * `Err` for a truncated header / color map, a Color Map Type byte
///   of `1` with a zero entry-size or length, or an unsupported entry
///   bit depth.
///
/// The color-map block sits after the Image Identification Field, so the
/// header's `id_length` is honoured when locating it.
pub fn parse_tga_color_map(input: &[u8]) -> Result<Option<TgaColorMap>> {
    let header = parse_header(input).ok_or_else(|| Error::invalid("TGA: header truncated"))?;
    if header.cmap_type != 1 {
        return Ok(None);
    }
    if header.cmap_entry_size == 0 || header.cmap_length == 0 {
        return Err(Error::invalid(
            "TGA: cmap_type == 1 but cmap_entry_size / cmap_length == 0",
        ));
    }
    let cursor = TGA_HEADER_SIZE + header.id_length as usize;
    let entry_bytes = header.cmap_entry_size.div_ceil(8) as usize;
    let cmap_bytes = entry_bytes * header.cmap_length as usize;
    if input.len() < cursor + cmap_bytes {
        return Err(Error::invalid("TGA: colour map truncated"));
    }
    let cmap = &input[cursor..cursor + cmap_bytes];
    let entries = decode_palette(cmap, header.cmap_entry_size, entry_bytes)?;
    Ok(Some(TgaColorMap {
        first_index: header.cmap_first,
        entry_size: header.cmap_entry_size,
        entries,
    }))
}

/// Read the §C.2 **TIPS border / background colour** — the first colour
/// map entry of a file whose header declares a Color Map Type of `1`
/// (spec, Color Map Type byte: *"1 means a color map is included, but
/// since this is an unmapped image it is usually ignored. TIPS (a Targa
/// paint system) will set the border color [to] the first map color if
/// it is present"*).
///
/// This is the meaningful interpretation of the vestigial colour map an
/// **un**mapped image (image type 2 / 3 / 10 / 11) may carry: a single
/// (or first) palette entry that records the paint-system's border /
/// background colour. The main decode path reads and skips that map; this
/// helper surfaces its first entry as straight RGBA.
///
/// Returns:
/// * `Ok(None)` when the header's Color Map Type byte is `0` (no map at
///   all) **or** when the file's image type is itself colour-mapped
///   (1 / 9) — for a colour-mapped image the map is the working palette,
///   not a border colour, so the "border colour" reading does not apply
///   and the caller should use [`parse_tga_color_map`] instead.
/// * `Ok(Some(rgba))` with the first colour-map entry (de-interleaved
///   exactly as [`parse_tga_color_map`] expands the palette).
/// * `Err` for a truncated header / colour map, a Color Map Type byte of
///   `1` with a zero entry-size or length, or an unsupported entry depth
///   — the same rejection rules [`parse_tga_color_map`] enforces.
///
/// A stored map with zero entries cannot occur (the parser rejects a
/// `cmap_type == 1` map of length 0), so a successful return always
/// carries a colour.
pub fn parse_tga_border_color(input: &[u8]) -> Result<Option<[u8; 4]>> {
    let header = parse_header(input).ok_or_else(|| Error::invalid("TGA: header truncated"))?;
    // Only an *unmapped* image type with a present map carries a TIPS
    // border colour; for a colour-mapped type the map is the palette.
    if let Some(image_type) = ImageType::from_u8(header.image_type_raw) {
        if image_type.is_colour_mapped() {
            return Ok(None);
        }
    }
    match parse_tga_color_map(input)? {
        Some(map) => Ok(map.entries.first().copied()),
        None => Ok(None),
    }
}

/// Read the §C.2 **Color Map Type** byte (fixed-header byte 1) as a typed
/// [`ColorMapType`] view straight from a file's bytes, without decoding
/// the colour map or the raster.
///
/// Returns `Err` only when the input is too short to hold the 18-byte
/// header; every byte value `0..=255` otherwise classifies
/// ([`ColorMapType::Absent`] / [`ColorMapType::Present`] /
/// [`ColorMapType::Reserved`]).
pub fn parse_tga_color_map_type(input: &[u8]) -> Result<ColorMapType> {
    let header = parse_header(input).ok_or_else(|| Error::invalid("TGA: header truncated"))?;
    Ok(header.color_map_type())
}

/// Narrow a file position to the scan-line table's 4-byte on-disk
/// offset width (§C.6.9 entries are 4-byte values).
fn scan_line_offset_u32(pos: usize) -> Result<u32> {
    u32::try_from(pos)
        .map_err(|_| Error::invalid("TGA: scan-line offset exceeds the table's 4-byte field"))
}

fn validate_depth(image_type: ImageType, depth: u8) -> Result<()> {
    let ok = match image_type {
        ImageType::UncompressedColourMapped | ImageType::RleColourMapped => depth == 8,
        ImageType::UncompressedTrueColour | ImageType::RleTrueColour => {
            matches!(depth, 15 | 16 | 24 | 32)
        }
        ImageType::UncompressedGrayscale | ImageType::RleGrayscale => depth == 8,
        ImageType::None => false,
    };
    if !ok {
        return Err(Error::unsupported(format!(
            "TGA: image type {:?} with depth {} not supported",
            image_type, depth
        )));
    }
    Ok(())
}

/// Expand a colour map to a flat `Vec<[u8; 4]>` palette in `Rgba`.
fn decode_palette(cmap: &[u8], entry_size: u8, entry_bytes: usize) -> Result<Vec<[u8; 4]>> {
    let entries = cmap.len() / entry_bytes;
    let mut palette = Vec::with_capacity(entries);
    for i in 0..entries {
        let off = i * entry_bytes;
        let rgba = match entry_size {
            15 | 16 => {
                let v = u16::from_le_bytes([cmap[off], cmap[off + 1]]);
                expand_a1r5g5b5(v, entry_size)
            }
            24 => {
                // BGR
                [cmap[off + 2], cmap[off + 1], cmap[off], 0xFF]
            }
            32 => {
                // BGRA
                [cmap[off + 2], cmap[off + 1], cmap[off], cmap[off + 3]]
            }
            other => {
                return Err(Error::unsupported(format!(
                    "TGA: palette entry size {} bits not supported",
                    other
                )))
            }
        };
        palette.push(rgba);
    }
    Ok(palette)
}

/// 16-bit pixel = A1R5G5B5 (top bit alpha) per spec §C.2; 15-bit pixel
/// = the same layout with the top bit reserved (alpha forced to 0xFF).
fn expand_a1r5g5b5(v: u16, entry_size: u8) -> [u8; 4] {
    let r5 = ((v >> 10) & 0x1F) as u8;
    let g5 = ((v >> 5) & 0x1F) as u8;
    let b5 = (v & 0x1F) as u8;
    let a = if entry_size == 16 {
        if (v & 0x8000) != 0 {
            0xFF
        } else {
            0
        }
    } else {
        0xFF
    };
    [expand5(r5), expand5(g5), expand5(b5), a]
}

/// 5-bit value to 8-bit by repeating the top bits (so `0b11111` →
/// `0xFF`, not `0xF8`).
fn expand5(v: u8) -> u8 {
    (v << 3) | (v >> 2)
}

fn decode_raw_pixels(
    header: &TgaHeader,
    image_type: ImageType,
    input: &[u8],
    palette: Option<&[[u8; 4]]>,
) -> Result<Vec<u8>> {
    let pixel_count = header.width as usize * header.height as usize;
    let in_bpp = header.depth.div_ceil(8) as usize;
    if input.len() < pixel_count * in_bpp {
        return Err(Error::invalid("TGA: pixel data truncated"));
    }
    let mut out = match image_type {
        ImageType::UncompressedGrayscale | ImageType::RleGrayscale => {
            Vec::with_capacity(pixel_count)
        }
        _ => Vec::with_capacity(pixel_count * 4),
    };
    for i in 0..pixel_count {
        let src = &input[i * in_bpp..i * in_bpp + in_bpp];
        emit_pixel(
            image_type,
            header.depth,
            header.cmap_first,
            src,
            palette,
            &mut out,
        )?;
    }
    Ok(out)
}

fn decode_rle_pixels(
    header: &TgaHeader,
    image_type: ImageType,
    input: &[u8],
    palette: Option<&[[u8; 4]]>,
) -> Result<Vec<u8>> {
    let pixel_count = header.width as usize * header.height as usize;
    let in_bpp = header.depth.div_ceil(8) as usize;
    let mut out = match image_type {
        ImageType::RleGrayscale => Vec::with_capacity(pixel_count),
        _ => Vec::with_capacity(pixel_count * 4),
    };
    let mut cursor = 0usize;
    let mut produced = 0usize;
    while produced < pixel_count {
        if cursor >= input.len() {
            return Err(Error::invalid("TGA: RLE stream truncated (no packet hdr)"));
        }
        let packet = input[cursor];
        cursor += 1;
        let count = (packet & 0x7F) as usize + 1;
        let is_run = (packet & 0x80) != 0;
        if produced + count > pixel_count {
            return Err(Error::invalid(format!(
                "TGA: RLE packet overruns pixel array (produced={produced}, packet count={count}, total={pixel_count})"
            )));
        }
        if is_run {
            // Run-length packet: 1 pixel × `count`. Every output pixel
            // in the run is byte-identical, so expand the source pixel
            // exactly once through `emit_pixel`, then replicate the
            // resulting output bytes for the remaining `count - 1`
            // pixels. This is bit-identical to calling `emit_pixel`
            // `count` times (same bytes, same order) but skips the
            // per-pixel image_type/depth match dispatch on the run.
            if cursor + in_bpp > input.len() {
                return Err(Error::invalid("TGA: RLE run packet truncated"));
            }
            let src = &input[cursor..cursor + in_bpp];
            let before = out.len();
            emit_pixel(
                image_type,
                header.depth,
                header.cmap_first,
                src,
                palette,
                &mut out,
            )?;
            let unit = out.len() - before;
            // Replicate the just-emitted pixel's bytes `count - 1` more
            // times by doubling the freshly-written tail in place.
            out.reserve(unit * (count - 1));
            let mut filled = unit;
            let target = unit * count;
            while filled < target {
                let chunk = filled.min(target - filled);
                out.extend_from_within(before..before + chunk);
                filled += chunk;
            }
            cursor += in_bpp;
        } else {
            // Raw packet: `count` literal pixels.
            if cursor + count * in_bpp > input.len() {
                return Err(Error::invalid("TGA: RLE raw packet truncated"));
            }
            for i in 0..count {
                let src = &input[cursor + i * in_bpp..cursor + i * in_bpp + in_bpp];
                emit_pixel(
                    image_type,
                    header.depth,
                    header.cmap_first,
                    src,
                    palette,
                    &mut out,
                )?;
            }
            cursor += count * in_bpp;
        }
        produced += count;
    }
    Ok(out)
}

fn emit_pixel(
    image_type: ImageType,
    depth: u8,
    cmap_first: u16,
    src: &[u8],
    palette: Option<&[[u8; 4]]>,
    out: &mut Vec<u8>,
) -> Result<()> {
    match image_type {
        ImageType::UncompressedColourMapped | ImageType::RleColourMapped => {
            // 8-bit palette index. The colour map on disk stores entries
            // starting at the Color Map Origin (the §C.2 "index of first
            // color map entry", header bytes 3-4). An image index `idx`
            // therefore addresses on-disk entry `idx - cmap_first`; indices
            // below the origin (or past the stored length) are out of range.
            let idx = src[0] as usize;
            let p = palette.ok_or_else(|| Error::invalid("TGA: colour-mapped without palette"))?;
            let entry = idx.checked_sub(cmap_first as usize).ok_or_else(|| {
                Error::invalid(format!(
                    "TGA: palette index {idx} below colour-map origin {cmap_first}"
                ))
            })?;
            let rgba = *p
                .get(entry)
                .ok_or_else(|| Error::invalid(format!("TGA: palette index {idx} out of range")))?;
            out.extend_from_slice(&rgba);
        }
        ImageType::UncompressedTrueColour | ImageType::RleTrueColour => match depth {
            15 | 16 => {
                let v = u16::from_le_bytes([src[0], src[1]]);
                let rgba = expand_a1r5g5b5(v, depth);
                out.extend_from_slice(&rgba);
            }
            24 => {
                // BGR on disk → RGBA in memory.
                out.extend_from_slice(&[src[2], src[1], src[0], 0xFF]);
            }
            32 => {
                // BGRA on disk → RGBA in memory.
                out.extend_from_slice(&[src[2], src[1], src[0], src[3]]);
            }
            // Defensive: every caller now runs `validate_depth` before
            // driving this routine, but a returned `Err` is the
            // documented contract for an unsupported depth — never a
            // panic — so a future caller that forgets the validation
            // still fails closed.
            other => {
                return Err(Error::unsupported(format!(
                    "TGA: true-colour pixel depth {other} not supported"
                )))
            }
        },
        ImageType::UncompressedGrayscale | ImageType::RleGrayscale => {
            // 8 bpp luma, single byte.
            out.push(src[0]);
        }
        // Defensive: `parse_tga` already rejects `image_type == 0`,
        // but as with the depth `match` above the contract is
        // returned-error, not panic.
        ImageType::None => {
            return Err(Error::invalid("TGA: image_type == 0 reached pixel decoder"))
        }
    }
    Ok(())
}
