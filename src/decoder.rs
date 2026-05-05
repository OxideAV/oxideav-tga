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
//! Image-descriptor bit 5 (bottom-up vs top-down) is honoured. Bit 4
//! (right-to-left columns) is rejected as `Unsupported` rather than
//! producing silently mirrored output — no real-world encoder sets it.
//!
//! With the default `registry` feature on, the gated `TgaDecoder` trait
//! impl wraps [`parse_tga`] for the `oxideav_core::Decoder` surface.

use crate::error::{Result, TgaError as Error};
use crate::image::{TgaImage, TgaPixelFormat};
use crate::types::{
    parse_extension_area, parse_footer, parse_header, ImageType, TgaExtensionArea, TgaFooter,
    TgaHeader, TGA_HEADER_SIZE,
};

#[cfg(feature = "registry")]
use oxideav_core::Decoder;
#[cfg(feature = "registry")]
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, VideoFrame, VideoPlane};

/// Factory registered with the codec registry. Consumes one packet per
/// whole TGA file and produces one frame.
#[cfg(feature = "registry")]
pub fn make_decoder(_params: &CodecParameters) -> oxideav_core::Result<Box<dyn Decoder>> {
    Ok(Box::new(TgaDecoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        pending: None,
        eof: false,
    }))
}

#[cfg(feature = "registry")]
struct TgaDecoder {
    codec_id: CodecId,
    pending: Option<VideoFrame>,
    eof: bool,
}

#[cfg(feature = "registry")]
impl Decoder for TgaDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn send_packet(&mut self, packet: &Packet) -> oxideav_core::Result<()> {
        let image = parse_tga(&packet.data)?;
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
    if header.is_right_to_left() {
        return Err(Error::unsupported(
            "TGA: image-descriptor bit 4 (right-to-left columns) not supported",
        ));
    }
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

    // Skip optional Image ID.
    let mut cursor = TGA_HEADER_SIZE + header.id_length as usize;
    if input.len() < cursor {
        return Err(Error::invalid("TGA: truncated past image ID"));
    }

    // Read colour map if present.
    let palette = if header.cmap_type == 1 {
        if !image_type.is_colour_mapped() {
            // The spec allows a colour map to be present even for
            // true-colour images (some encoders do this), so we just
            // read past it instead of erroring out.
        }
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

    // Normalise to top-down. On-disk rows are top-down iff the
    // descriptor bit-5 is set; otherwise we flip.
    let bpp = match pixel_format {
        TgaPixelFormat::Rgba => 4,
        TgaPixelFormat::Gray8 => 1,
        TgaPixelFormat::Rgb24 => 3,
    };
    let stride = header.width as usize * bpp;
    let data = if header.is_top_down() {
        pixels
    } else {
        let mut flipped = vec![0u8; pixels.len()];
        for y in 0..header.height as usize {
            let src = &pixels[(header.height as usize - 1 - y) * stride..][..stride];
            flipped[y * stride..y * stride + stride].copy_from_slice(src);
        }
        flipped
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
    // colour map for palette lookups.
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
        let _ = cursor;
        Some(decode_palette(cmap, header.cmap_entry_size, entry_bytes)?)
    } else {
        None
    };

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

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

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
        emit_pixel(image_type, header.depth, src, palette, &mut out)?;
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
            // Run-length packet: 1 pixel × `count`.
            if cursor + in_bpp > input.len() {
                return Err(Error::invalid("TGA: RLE run packet truncated"));
            }
            let src = &input[cursor..cursor + in_bpp];
            for _ in 0..count {
                emit_pixel(image_type, header.depth, src, palette, &mut out)?;
            }
            cursor += in_bpp;
        } else {
            // Raw packet: `count` literal pixels.
            if cursor + count * in_bpp > input.len() {
                return Err(Error::invalid("TGA: RLE raw packet truncated"));
            }
            for i in 0..count {
                let src = &input[cursor + i * in_bpp..cursor + i * in_bpp + in_bpp];
                emit_pixel(image_type, header.depth, src, palette, &mut out)?;
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
    src: &[u8],
    palette: Option<&[[u8; 4]]>,
    out: &mut Vec<u8>,
) -> Result<()> {
    match image_type {
        ImageType::UncompressedColourMapped | ImageType::RleColourMapped => {
            // 8-bit palette index.
            let idx = src[0] as usize;
            let p = palette.ok_or_else(|| Error::invalid("TGA: colour-mapped without palette"))?;
            let rgba = *p
                .get(idx)
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
            _ => unreachable!("validate_depth gates this"),
        },
        ImageType::UncompressedGrayscale | ImageType::RleGrayscale => {
            // 8 bpp luma, single byte.
            out.push(src[0]);
        }
        ImageType::None => unreachable!("rejected at parse_tga()"),
    }
    Ok(())
}
