//! TGA encode.
//!
//! Six write paths, one per image type the spec defines:
//!
//! * [`encode_tga_uncompressed`] — image type 2 (uncompressed BGR /
//!   BGRA), 24 or 32 bpp depending on the input alpha channel.
//! * [`encode_tga_rle`] — image type 10 (RLE BGR / BGRA), 24 or 32 bpp.
//!   The RLE encoder picks runs of ≥ 2 consecutive identical pixels
//!   (max 128 per run-packet) vs raw runs (max 128 per raw-packet) per
//!   spec §C.5.
//! * [`encode_tga_palette`] — image type 1 (uncompressed colour-mapped),
//!   8-bit indices into a colour map whose entry size auto-selects to
//!   the narrowest lossless width (24-bit BGR for an opaque palette,
//!   32-bit BGRA when alpha is used); max 256 unique RGBA colours in the
//!   input. [`encode_tga_palette_with_entry_size`] writes an explicit
//!   15 / 16 / 24 / 32-bit colour map.
//! * [`encode_tga_palette_rle`] — image type 9 (RLE colour-mapped).
//! * [`encode_tga_grayscale`] — image type 3 (uncompressed grayscale),
//!   8 bpp luma.
//! * [`encode_tga_grayscale_rle`] — image type 11 (RLE grayscale).
//!
//! All writers produce a top-down image (image-descriptor bit 5 set) so
//! decoders that don't honour the bit (some old viewers) still display
//! them right-side up. The base output is a TGA 1.0 file: no footer, no
//! extension area; use [`encode_tga_with_extension`] to append a
//! 26-byte TGA 2.0 footer + 495-byte extension-area body authored from
//! [`ExtensionAreaInput`] (optionally with a postage-stamp thumbnail).

use crate::error::{Result, TgaError as Error};
use crate::image::{TgaImage, TgaPixelFormat};
use crate::types::{
    ColorMapEntrySize, ImageOrigin, ImageType, TgaColourCorrectionTable, TgaDeveloperTag,
    TgaScanLineTable, TGA_EXTENSION_AREA_SIZE, TGA_FOOTER_SIZE, TGA_HEADER_SIZE,
};

#[cfg(feature = "registry")]
use oxideav_core::Encoder;
#[cfg(feature = "registry")]
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, PixelFormat, TimeBase};

#[cfg(feature = "registry")]
pub fn make_encoder(params: &CodecParameters) -> oxideav_core::Result<Box<dyn Encoder>> {
    let mut out_params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    out_params.width = params.width;
    out_params.height = params.height;
    out_params.pixel_format = params.pixel_format;
    Ok(Box::new(TgaEncoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        out_params,
        pending: None,
        eof: false,
    }))
}

#[cfg(feature = "registry")]
struct TgaEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    pending: Option<Vec<u8>>,
    eof: bool,
}

#[cfg(feature = "registry")]
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
        let format = self.out_params.pixel_format.ok_or_else(|| {
            oxideav_core::Error::invalid("TGA encoder: pixel_format missing in CodecParameters")
        })?;
        let width = self.out_params.width.ok_or_else(|| {
            oxideav_core::Error::invalid("TGA encoder: width missing in CodecParameters")
        })?;
        let height = self.out_params.height.ok_or_else(|| {
            oxideav_core::Error::invalid("TGA encoder: height missing in CodecParameters")
        })?;
        if vf.planes.is_empty() {
            return Err(oxideav_core::Error::invalid(
                "TGA encoder: empty frame plane",
            ));
        }
        let bpp = match format {
            PixelFormat::Rgba => 4,
            PixelFormat::Rgb24 => 3,
            other => {
                return Err(oxideav_core::Error::invalid(format!(
                    "TGA encoder: unsupported pixel format {other:?}"
                )))
            }
        };
        let want = width as usize * bpp;
        let plane = &vf.planes[0];
        let mut tight = Vec::with_capacity(want * height as usize);
        for y in 0..height as usize {
            let off = y * plane.stride;
            tight.extend_from_slice(&plane.data[off..off + want]);
        }
        let w16: u16 = width
            .try_into()
            .map_err(|_| oxideav_core::Error::invalid("TGA encoder: width exceeds 65535"))?;
        let h16: u16 = height
            .try_into()
            .map_err(|_| oxideav_core::Error::invalid("TGA encoder: height exceeds 65535"))?;
        let bytes = match format {
            PixelFormat::Rgba => encode_tga_rle(w16, h16, &tight)?,
            PixelFormat::Rgb24 => {
                // Promote to RGBA with α=0xFF so the encoder can stay
                // BGRA-only on the wire.
                let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
                for chunk in tight.chunks_exact(3) {
                    rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 0xFF]);
                }
                encode_tga_rle(w16, h16, &rgba)?
            }
            other => {
                return Err(oxideav_core::Error::invalid(format!(
                    "TGA encoder: unsupported pixel format {other:?}"
                )))
            }
        };
        self.pending = Some(bytes);
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

// ---------------------------------------------------------------------------
// Public standalone API
// ---------------------------------------------------------------------------

/// Encode `width × height` RGBA bytes (4 bytes per pixel, top-down,
/// row-major) into an uncompressed TGA file (image type 2). Output
/// depth is 32 bpp if any input alpha byte is `< 0xFF`, otherwise
/// 24 bpp.
pub fn encode_tga_uncompressed(width: u16, height: u16, rgba: &[u8]) -> Result<Vec<u8>> {
    let (depth, alpha) = pick_output_depth(rgba)?;
    if rgba.len() < width as usize * height as usize * 4 {
        return Err(Error::invalid("TGA encoder: input shorter than w*h*4"));
    }
    let mut out = Vec::with_capacity(TGA_HEADER_SIZE + rgba.len());
    write_header(
        &mut out,
        ImageType::UncompressedTrueColour,
        width,
        height,
        depth,
        alpha,
    );
    let bpp = depth as usize / 8;
    for y in 0..height as usize {
        for x in 0..width as usize {
            let off = (y * width as usize + x) * 4;
            // Always write BGR(A).
            out.push(rgba[off + 2]);
            out.push(rgba[off + 1]);
            out.push(rgba[off]);
            if bpp == 4 {
                out.push(rgba[off + 3]);
            }
        }
    }
    Ok(out)
}

/// Encode `width × height` **RGB24** bytes (3 bytes per pixel, top-down,
/// row-major) into an uncompressed TGA file (image type 2) at 24 bpp.
///
/// Symmetric to [`encode_tga_uncompressed`] but skips the alpha-channel
/// detection — the output is always 24 bpp BGR. Use this when the
/// caller already knows the input is fully opaque and wants to avoid
/// the auto-detection scan over the alpha channel.
pub fn encode_tga_uncompressed_rgb24(width: u16, height: u16, rgb: &[u8]) -> Result<Vec<u8>> {
    if rgb.len() % 3 != 0 {
        return Err(Error::invalid(
            "TGA encoder: rgb input length not multiple of 3",
        ));
    }
    if rgb.len() < width as usize * height as usize * 3 {
        return Err(Error::invalid("TGA encoder: rgb input shorter than w*h*3"));
    }
    let mut out = Vec::with_capacity(TGA_HEADER_SIZE + rgb.len());
    write_header(
        &mut out,
        ImageType::UncompressedTrueColour,
        width,
        height,
        24,
        0,
    );
    for y in 0..height as usize {
        for x in 0..width as usize {
            let off = (y * width as usize + x) * 3;
            // BGR on the wire.
            out.push(rgb[off + 2]);
            out.push(rgb[off + 1]);
            out.push(rgb[off]);
        }
    }
    Ok(out)
}

/// Encode `width × height` **RGB24** bytes (3 bytes per pixel, top-down,
/// row-major) into an RLE TGA file (image type 10) at 24 bpp.
///
/// Symmetric to [`encode_tga_rle`] but for RGB24-only inputs — the
/// output is always 24 bpp BGR. RLE packetisation per spec §C.5 (same
/// algorithm as the RGBA variant, just with no alpha channel on the
/// wire).
pub fn encode_tga_rle_rgb24(width: u16, height: u16, rgb: &[u8]) -> Result<Vec<u8>> {
    if rgb.len() % 3 != 0 {
        return Err(Error::invalid(
            "TGA encoder: rgb input length not multiple of 3",
        ));
    }
    if rgb.len() < width as usize * height as usize * 3 {
        return Err(Error::invalid("TGA encoder: rgb input shorter than w*h*3"));
    }
    let mut out = Vec::with_capacity(TGA_HEADER_SIZE + rgb.len() / 2);
    write_header(&mut out, ImageType::RleTrueColour, width, height, 24, 0);

    let mut row_pixels: Vec<[u8; 3]> = Vec::with_capacity(width as usize);
    for y in 0..height as usize {
        row_pixels.clear();
        for x in 0..width as usize {
            let off = (y * width as usize + x) * 3;
            row_pixels.push([rgb[off], rgb[off + 1], rgb[off + 2]]);
        }
        rle_one_row_rgb24(&row_pixels, &mut out);
    }
    Ok(out)
}

/// Encode `width × height` RGBA bytes (4 bytes per pixel, top-down,
/// row-major) into an RLE TGA file (image type 10). Output depth is
/// 32 bpp if any input alpha byte is `< 0xFF`, otherwise 24 bpp.
///
/// RLE packetisation per spec §C.5: a run-length packet of `count`
/// identical pixels (`count` ∈ 1..=128) writes 1 header byte (top bit
/// set, bottom 7 bits = `count - 1`) followed by 1 pixel. A raw packet
/// of `count` literal pixels (`count` ∈ 1..=128) writes 1 header byte
/// (top bit clear, bottom 7 bits = `count - 1`) followed by `count`
/// pixels. Runs are preferred when `count ≥ 2`; isolated single pixels
/// are coalesced into raw packets.
pub fn encode_tga_rle(width: u16, height: u16, rgba: &[u8]) -> Result<Vec<u8>> {
    let (depth, alpha) = pick_output_depth(rgba)?;
    if rgba.len() < width as usize * height as usize * 4 {
        return Err(Error::invalid("TGA encoder: input shorter than w*h*4"));
    }
    let bpp = depth as usize / 8;

    let mut out = Vec::with_capacity(TGA_HEADER_SIZE + rgba.len() / 2);
    write_header(
        &mut out,
        ImageType::RleTrueColour,
        width,
        height,
        depth,
        alpha,
    );

    // RLE is per-row (spec recommends not letting runs cross scanline
    // boundaries; round-trip-safe and is what `magick convert` does).
    let mut row_pixels: Vec<[u8; 4]> = Vec::with_capacity(width as usize);
    for y in 0..height as usize {
        row_pixels.clear();
        for x in 0..width as usize {
            let off = (y * width as usize + x) * 4;
            let p = [rgba[off], rgba[off + 1], rgba[off + 2], rgba[off + 3]];
            row_pixels.push(p);
        }
        rle_one_row(&row_pixels, bpp, &mut out);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// 24 bpp if every input pixel has α = 0xFF, else 32 bpp. `alpha` is
/// the value to put in the descriptor's bottom 4 bits.
fn pick_output_depth(rgba: &[u8]) -> Result<(u8, u8)> {
    if rgba.len() % 4 != 0 {
        return Err(Error::invalid(
            "TGA encoder: input length not multiple of 4",
        ));
    }
    let has_alpha = rgba.chunks_exact(4).any(|p| p[3] != 0xFF);
    if has_alpha {
        Ok((32, 8))
    } else {
        Ok((24, 0))
    }
}

fn write_header(
    out: &mut Vec<u8>,
    image_type: ImageType,
    width: u16,
    height: u16,
    depth: u8,
    alpha_bits: u8,
) {
    out.push(0); // id_length
    out.push(0); // cmap_type — no colour map for true-colour
    out.push(image_type as u8);
    out.extend_from_slice(&0u16.to_le_bytes()); // cmap_first
    out.extend_from_slice(&0u16.to_le_bytes()); // cmap_length
    out.push(0); // cmap_entry_size
    out.extend_from_slice(&0u16.to_le_bytes()); // x_origin
    out.extend_from_slice(&0u16.to_le_bytes()); // y_origin
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.push(depth);
    // Image descriptor: alpha bit count in low 4 bits + bit 5 set
    // (top-down origin) so decoders that don't honour the origin bit
    // still display the image right-way-up.
    out.push((alpha_bits & 0x0F) | 0x20);
}

fn rle_one_row(row: &[[u8; 4]], bpp: usize, out: &mut Vec<u8>) {
    let mut i = 0;
    let n = row.len();
    while i < n {
        // Look at the run-length starting at `i`.
        let mut run = 1;
        while i + run < n && row[i + run] == row[i] && run < 128 {
            run += 1;
        }
        if run >= 2 {
            // Emit run-length packet.
            out.push(0x80 | (run as u8 - 1));
            write_pixel(&row[i], bpp, out);
            i += run;
        } else {
            // Collect a raw run: pixels that don't begin a run of ≥ 2.
            // Stop at length 128 OR right before a position that *does*
            // begin a run of ≥ 2 (so we leave it for the next iteration).
            let raw_start = i;
            let mut raw_end = i + 1;
            while raw_end < n && raw_end - raw_start < 128 {
                // Would `raw_end` start a run of ≥ 2? If so, break out
                // and let the run-packet path pick it up.
                if raw_end + 1 < n && row[raw_end + 1] == row[raw_end] {
                    break;
                }
                raw_end += 1;
            }
            let count = raw_end - raw_start;
            out.push(((count as u8) - 1) & 0x7F);
            for p in &row[raw_start..raw_end] {
                write_pixel(p, bpp, out);
            }
            i = raw_end;
        }
    }
}

fn write_pixel(p: &[u8; 4], bpp: usize, out: &mut Vec<u8>) {
    // Pixel order on the wire: BGR or BGRA.
    out.push(p[2]);
    out.push(p[1]);
    out.push(p[0]);
    if bpp == 4 {
        out.push(p[3]);
    }
}

/// RLE packetisation for `[u8; 3]` (RGB24) pixels. Same algorithm as
/// [`rle_one_row`] but always 24 bpp BGR on the wire and no alpha.
fn rle_one_row_rgb24(row: &[[u8; 3]], out: &mut Vec<u8>) {
    let mut i = 0;
    let n = row.len();
    while i < n {
        let mut run = 1;
        while i + run < n && row[i + run] == row[i] && run < 128 {
            run += 1;
        }
        if run >= 2 {
            out.push(0x80 | (run as u8 - 1));
            // BGR on wire.
            out.push(row[i][2]);
            out.push(row[i][1]);
            out.push(row[i][0]);
            i += run;
        } else {
            let raw_start = i;
            let mut raw_end = i + 1;
            while raw_end < n && raw_end - raw_start < 128 {
                if raw_end + 1 < n && row[raw_end + 1] == row[raw_end] {
                    break;
                }
                raw_end += 1;
            }
            let count = raw_end - raw_start;
            out.push(((count as u8) - 1) & 0x7F);
            for p in &row[raw_start..raw_end] {
                out.push(p[2]);
                out.push(p[1]);
                out.push(p[0]);
            }
            i = raw_end;
        }
    }
}

/// Wrapper so callers with a [`TgaImage`] don't need to flatten by hand.
/// Picks the same depth-selection rule as [`encode_tga_uncompressed`].
pub fn encode_tga_uncompressed_image(image: &TgaImage) -> Result<Vec<u8>> {
    let rgba = to_rgba(image)?;
    encode_tga_uncompressed(image.width as u16, image.height as u16, &rgba)
}

/// Wrapper so callers with a [`TgaImage`] don't need to flatten by hand.
pub fn encode_tga_rle_image(image: &TgaImage) -> Result<Vec<u8>> {
    let rgba = to_rgba(image)?;
    encode_tga_rle(image.width as u16, image.height as u16, &rgba)
}

fn to_rgba(image: &TgaImage) -> Result<Vec<u8>> {
    match image.pixel_format {
        TgaPixelFormat::Rgba => Ok(image.data.clone()),
        TgaPixelFormat::Rgb24 => {
            let mut out = Vec::with_capacity(image.data.len() / 3 * 4);
            for c in image.data.chunks_exact(3) {
                out.extend_from_slice(&[c[0], c[1], c[2], 0xFF]);
            }
            Ok(out)
        }
        TgaPixelFormat::Gray8 => {
            // Promote luma → grey RGBA so RGBA-only encoders accept it.
            let mut out = Vec::with_capacity(image.data.len() * 4);
            for &g in &image.data {
                out.extend_from_slice(&[g, g, g, 0xFF]);
            }
            Ok(out)
        }
    }
}

// ---------------------------------------------------------------------------
// Image type 1 / 9 — uncompressed + RLE colour-mapped (palette indexed).
// ---------------------------------------------------------------------------

/// Encode `width × height` RGBA bytes into an uncompressed colour-mapped
/// TGA file (image type 1) with an 8-bit palette index.
///
/// The palette is built from the unique RGBA colours in the input. The
/// colour-map **entry size** is auto-selected to the narrowest spec-legal
/// width that carries the palette losslessly per
/// [`ColorMapEntrySize::smallest_lossless_for`]: a 24-bit BGR map when
/// every palette entry is fully opaque, a 32-bit BGRA map when any entry
/// uses alpha. (Callers who want a fixed 15/16/24/32-bit entry size — for
/// instance to write a compact Targa-16 palette — use
/// [`encode_tga_palette_with_entry_size`].)
///
/// Returns [`crate::TgaError::Unsupported`] if the input contains more
/// than 256 unique RGBA colours (the indexed palette can't represent
/// them).
pub fn encode_tga_palette(width: u16, height: u16, rgba: &[u8]) -> Result<Vec<u8>> {
    encode_palette_inner(
        width,
        height,
        rgba,
        ImageType::UncompressedColourMapped,
        None,
    )
}

/// Encode `width × height` RGBA bytes into an RLE colour-mapped TGA
/// file (image type 9). Palette construction + entry-size auto-selection
/// match [`encode_tga_palette`]; RLE packetisation runs on the 8-bit
/// indices, per spec §C.5.
pub fn encode_tga_palette_rle(width: u16, height: u16, rgba: &[u8]) -> Result<Vec<u8>> {
    encode_palette_inner(width, height, rgba, ImageType::RleColourMapped, None)
}

/// Encode `width × height` RGBA bytes into a colour-mapped TGA file with
/// an explicit colour-map [`ColorMapEntrySize`].
///
/// `image_type` must be a colour-mapped type ([`ImageType::UncompressedColourMapped`]
/// = 1 or [`ImageType::RleColourMapped`] = 9); any other type is rejected
/// with [`crate::TgaError::InvalidData`].
///
/// The entry size controls how each palette colour is packed on disk
/// (spec §C.2 "Each color map entry is 2, 3, or 4 bytes"):
///
/// * [`ColorMapEntrySize::Bits24`] / [`Bits32`](ColorMapEntrySize::Bits32)
///   store the full 8-bit channels (BGR / BGRA) — lossless.
/// * [`ColorMapEntrySize::Bits15`] / [`Bits16`](ColorMapEntrySize::Bits16)
///   pack each colour into the 5-5-5 `ARRRRRGG GGGBBBBB` layout — the
///   8-bit channels are quantised to 5 bits (top 5 bits kept), so a
///   round trip through the decoder reproduces the **expanded** 5-bit
///   colour, not the original 8-bit colour. 16-bit keeps the top alpha
///   bit (set when an entry's alpha ≥ 0x80); 15-bit writes it `0`.
///
/// Pass [`encode_tga_palette`] / [`encode_tga_palette_rle`] instead to
/// auto-select the narrowest *lossless* (24/32-bit) size.
pub fn encode_tga_palette_with_entry_size(
    width: u16,
    height: u16,
    rgba: &[u8],
    image_type: ImageType,
    entry_size: ColorMapEntrySize,
) -> Result<Vec<u8>> {
    if !image_type.is_colour_mapped() {
        return Err(Error::invalid(
            "TGA palette encoder: image_type must be 1 (uncompressed) or 9 (RLE) colour-mapped",
        ));
    }
    encode_palette_inner(width, height, rgba, image_type, Some(entry_size))
}

fn encode_palette_inner(
    width: u16,
    height: u16,
    rgba: &[u8],
    image_type: ImageType,
    entry_size: Option<ColorMapEntrySize>,
) -> Result<Vec<u8>> {
    if rgba.len() % 4 != 0 {
        return Err(Error::invalid(
            "TGA encoder: palette input length not multiple of 4",
        ));
    }
    if rgba.len() < width as usize * height as usize * 4 {
        return Err(Error::invalid(
            "TGA palette encoder: input shorter than w*h*4",
        ));
    }
    // Build palette + index stream.
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut indices: Vec<u8> = Vec::with_capacity(width as usize * height as usize);
    // Linear scan is fine for the ≤ 256 unique-colour budget.
    for chunk in rgba.chunks_exact(4).take(width as usize * height as usize) {
        let p = [chunk[0], chunk[1], chunk[2], chunk[3]];
        let idx = match palette.iter().position(|q| *q == p) {
            Some(i) => i,
            None => {
                if palette.len() == 256 {
                    return Err(Error::unsupported(
                        "TGA palette encoder: input has > 256 unique RGBA colours \
                         (use encode_tga_uncompressed/encode_tga_rle for true-colour)",
                    ));
                }
                palette.push(p);
                palette.len() - 1
            }
        };
        indices.push(idx as u8);
    }

    // Pick the colour-map entry size: the caller's explicit choice, else
    // the narrowest lossless (24/32-bit) width for this palette.
    let entry = entry_size.unwrap_or_else(|| ColorMapEntrySize::smallest_lossless_for(&palette));
    let entry_bytes = entry.bytes();

    // Header. cmap_type=1, depth=8.
    let palette_len: u16 = palette.len() as u16;
    let mut out =
        Vec::with_capacity(TGA_HEADER_SIZE + (palette_len as usize) * entry_bytes + indices.len());
    out.push(0); // id_length
    out.push(1); // cmap_type
    out.push(image_type as u8);
    out.extend_from_slice(&0u16.to_le_bytes()); // cmap_first
    out.extend_from_slice(&palette_len.to_le_bytes()); // cmap_length
    out.push(entry.bits()); // cmap_entry_size — 15 / 16 / 24 / 32
    out.extend_from_slice(&0u16.to_le_bytes()); // x_origin
    out.extend_from_slice(&0u16.to_le_bytes()); // y_origin
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.push(8); // depth — palette indices are always 8 bpp
                 // Indexed images don't carry an alpha-bit count in the descriptor:
                 // the palette entry holds the alpha channel.
    out.push(0x20); // top-down

    // Colour map: one entry per palette colour, packed per `entry`.
    for p in &palette {
        push_colour_map_entry(&mut out, *p, entry);
    }

    // Pixel data.
    if image_type.is_rle() {
        for y in 0..height as usize {
            let row = &indices[y * width as usize..(y + 1) * width as usize];
            rle_one_row_u8(row, &mut out);
        }
    } else {
        out.extend_from_slice(&indices);
    }
    Ok(out)
}

/// Pack one straight-RGBA palette colour into the on-disk colour-map
/// entry layout for `entry` (spec §C.2). The byte order mirrors
/// `decode_palette`'s expansion exactly so a round trip is faithful:
///
/// * 24-bit → blue, green, red.
/// * 32-bit → blue, green, red, attribute (alpha).
/// * 15/16-bit → little-endian `u16` of `[A]RRRRRGG GGGBBBBB` (top 5 bits
///   of each 8-bit channel; 16-bit sets the top alpha bit when `a ≥ 0x80`,
///   15-bit clears it).
fn push_colour_map_entry(out: &mut Vec<u8>, rgba: [u8; 4], entry: ColorMapEntrySize) {
    match entry {
        ColorMapEntrySize::Bits24 => {
            out.push(rgba[2]); // B
            out.push(rgba[1]); // G
            out.push(rgba[0]); // R
        }
        ColorMapEntrySize::Bits32 => {
            out.push(rgba[2]); // B
            out.push(rgba[1]); // G
            out.push(rgba[0]); // R
            out.push(rgba[3]); // A
        }
        ColorMapEntrySize::Bits15 | ColorMapEntrySize::Bits16 => {
            let r5 = (rgba[0] >> 3) as u16;
            let g5 = (rgba[1] >> 3) as u16;
            let b5 = (rgba[2] >> 3) as u16;
            let mut v = (r5 << 10) | (g5 << 5) | b5;
            if entry == ColorMapEntrySize::Bits16 && rgba[3] >= 0x80 {
                v |= 0x8000;
            }
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

// ---------------------------------------------------------------------------
// Image type 3 / 11 — uncompressed + RLE grayscale.
// ---------------------------------------------------------------------------

/// Encode `width × height` Gray8 bytes (1 byte per pixel, top-down,
/// row-major) into an uncompressed grayscale TGA file (image type 3).
pub fn encode_tga_grayscale(width: u16, height: u16, gray: &[u8]) -> Result<Vec<u8>> {
    encode_grayscale_inner(width, height, gray, ImageType::UncompressedGrayscale)
}

/// Encode `width × height` Gray8 bytes into an RLE grayscale TGA file
/// (image type 11). RLE packetisation per spec §C.5.
pub fn encode_tga_grayscale_rle(width: u16, height: u16, gray: &[u8]) -> Result<Vec<u8>> {
    encode_grayscale_inner(width, height, gray, ImageType::RleGrayscale)
}

fn encode_grayscale_inner(
    width: u16,
    height: u16,
    gray: &[u8],
    image_type: ImageType,
) -> Result<Vec<u8>> {
    if gray.len() < width as usize * height as usize {
        return Err(Error::invalid(
            "TGA grayscale encoder: input shorter than w*h",
        ));
    }
    let mut out = Vec::with_capacity(TGA_HEADER_SIZE + gray.len());
    out.push(0); // id_length
    out.push(0); // cmap_type
    out.push(image_type as u8);
    out.extend_from_slice(&0u16.to_le_bytes()); // cmap_first
    out.extend_from_slice(&0u16.to_le_bytes()); // cmap_length
    out.push(0); // cmap_entry_size
    out.extend_from_slice(&0u16.to_le_bytes()); // x_origin
    out.extend_from_slice(&0u16.to_le_bytes()); // y_origin
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.push(8); // depth
    out.push(0x20); // top-down
    let pixels = &gray[..width as usize * height as usize];
    if image_type.is_rle() {
        for y in 0..height as usize {
            let row = &pixels[y * width as usize..(y + 1) * width as usize];
            rle_one_row_u8(row, &mut out);
        }
    } else {
        out.extend_from_slice(pixels);
    }
    Ok(out)
}

/// Single-byte-per-pixel RLE packetisation (used by both 8-bit
/// palette-indexed and 8-bit grayscale streams). Same algorithm as
/// [`rle_one_row`] but with `bpp=1` and no BGR swap.
fn rle_one_row_u8(row: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    let n = row.len();
    while i < n {
        let mut run = 1;
        while i + run < n && row[i + run] == row[i] && run < 128 {
            run += 1;
        }
        if run >= 2 {
            out.push(0x80 | (run as u8 - 1));
            out.push(row[i]);
            i += run;
        } else {
            let raw_start = i;
            let mut raw_end = i + 1;
            while raw_end < n && raw_end - raw_start < 128 {
                if raw_end + 1 < n && row[raw_end + 1] == row[raw_end] {
                    break;
                }
                raw_end += 1;
            }
            let count = raw_end - raw_start;
            out.push(((count as u8) - 1) & 0x7F);
            out.extend_from_slice(&row[raw_start..raw_end]);
            i = raw_end;
        }
    }
}

// ---------------------------------------------------------------------------
// TGA 2.0 footer + extension area write.
// ---------------------------------------------------------------------------

/// One developer-area tag payload supplied by the caller.
///
/// `encode_tga_with_extension` writes each payload into the developer-
/// area data region, fills in the tag directory at the configured
/// `developer_directory_offset`, and back-patches `tag.offset` /
/// `tag.size` to the on-disk byte range it landed at. Tags with an
/// empty `payload` are emitted as marker tags (directory entry with
/// `offset == 0` and `size == 0` — spec-legal).
#[derive(Debug, Clone)]
pub struct DeveloperTagInput {
    /// Tag identifier. `0..=32767` available for developer use,
    /// `32768..=65535` reserved for Truevision (spec §C.7).
    pub tag_id: u16,
    /// Application-defined payload bytes. Empty = marker tag (no
    /// payload region written).
    pub payload: Vec<u8>,
}

/// Author-supplied fields for the TGA 2.0 extension area body.
///
/// Pass to [`encode_tga_with_extension`]. All fields are optional —
/// leave string fields empty (`String::new()`) and ratio fields at
/// `(0, 0)` to opt out of advertising them. Strings are silently
/// truncated to the spec's per-field limit (40 chars for author/job/
/// software, 80 chars per comment line).
#[derive(Debug, Clone, Default)]
pub struct ExtensionAreaInput {
    pub author_name: String,
    pub author_comment: [String; 4],
    pub timestamp: crate::types::TgaTimestamp,
    pub job_name: String,
    pub job_time: (u16, u16, u16),
    pub software_id: String,
    pub software_version: (u16, char),
    pub key_color: [u8; 4],
    pub pixel_aspect_ratio: (u16, u16),
    pub gamma: (u16, u16),
    pub attributes_type: u8,
    /// Optional postage-stamp / thumbnail. Always emitted in
    /// uncompressed BGR(A) form (per spec §C.6.10) regardless of the
    /// parent image's compression.
    pub postage_stamp: Option<TgaImage>,
    /// Optional colour-correction table (spec §C.6.8). If `Some`, the
    /// table is appended to the file and the extension area's
    /// `colour_correction_offset` field is back-patched to point at
    /// it. Decoders that consult the CCT can recover the table via
    /// [`crate::parse_tga_colour_correction_table`].
    pub colour_correction_table: Option<TgaColourCorrectionTable>,
    /// Optional scan-line table (spec §C.6.9). If `Some`, the table
    /// is appended and the extension area's `scan_line_offset` is
    /// back-patched. The number of entries should match the parent
    /// image height; supplying a different length is permitted (no
    /// hard check) for callers that intentionally write a partial
    /// or padded table.
    pub scan_line_table: Option<TgaScanLineTable>,
    /// Optional developer-area tags (spec §C.7). If non-empty, the
    /// caller-supplied payload bytes are written first, then the tag
    /// directory, and the footer's `developer_directory_offset` is
    /// back-patched.
    pub developer_tags: Vec<DeveloperTagInput>,
}

/// Append a TGA 2.0 footer + extension area body to a TGA byte stream
/// produced by any of the [`encode_tga_uncompressed`] / [`encode_tga_rle`]
/// / [`encode_tga_palette`] / [`encode_tga_palette_rle`] /
/// [`encode_tga_grayscale`] / [`encode_tga_grayscale_rle`] writers.
///
/// Returns a new `Vec<u8>` that is a complete TGA 2.0 file: original
/// pixel data, then the extension area, then the optional postage-stamp
/// (if `ext.postage_stamp` is `Some`), then the 26-byte footer with the
/// `extension_area_offset` pointing at the extension area we just wrote
/// and `developer_directory_offset` set to 0.
///
/// When a postage stamp is supplied it is serialised in the same pixel
/// format the parent TGA uses on disk (spec §C.6.10): 24-bit BGR if the
/// parent header reports `depth = 24`, 32-bit BGRA if it reports
/// `depth = 32`, etc. The supplied `stamp.pixel_format` is treated as
/// the source representation and converted automatically.
pub fn encode_tga_with_extension(base_tga: &[u8], ext: &ExtensionAreaInput) -> Result<Vec<u8>> {
    // We need the parent depth to know how to lay out the postage
    // stamp on disk. Cheap header peek.
    let parent_depth = if base_tga.len() >= TGA_HEADER_SIZE {
        base_tga[16]
    } else {
        return Err(Error::invalid(
            "TGA encoder: base_tga too short to contain a header",
        ));
    };

    let dev_payload_total: usize = ext.developer_tags.iter().map(|t| t.payload.len()).sum();
    let dev_dir_size = if ext.developer_tags.is_empty() {
        0
    } else {
        2 + ext.developer_tags.len() * 10
    };
    let cct_size = if ext.colour_correction_table.is_some() {
        crate::types::TGA_COLOUR_CORRECTION_TABLE_SIZE
    } else {
        0
    };
    let sct_size = ext
        .scan_line_table
        .as_ref()
        .map(|t| t.offsets.len() * 4)
        .unwrap_or(0);
    let stamp_bytes = ext
        .postage_stamp
        .as_ref()
        .map(|s| s.data.len())
        .unwrap_or(0);
    let mut out = Vec::with_capacity(
        base_tga.len()
            + dev_payload_total
            + dev_dir_size
            + TGA_EXTENSION_AREA_SIZE
            + 2
            + stamp_bytes
            + cct_size
            + sct_size
            + TGA_FOOTER_SIZE,
    );
    out.extend_from_slice(base_tga);

    // Developer-area payloads + tag directory go BEFORE the extension
    // area so the directory offset is known at the time we lay down
    // the footer. We write the payloads first (back-patching each
    // tag's offset), then the directory at the end.
    let mut directory_entries: Vec<TgaDeveloperTag> = Vec::with_capacity(ext.developer_tags.len());
    for input_tag in &ext.developer_tags {
        if input_tag.payload.is_empty() {
            directory_entries.push(TgaDeveloperTag {
                tag_id: input_tag.tag_id,
                offset: 0,
                size: 0,
            });
        } else {
            let off = out.len() as u32;
            out.extend_from_slice(&input_tag.payload);
            directory_entries.push(TgaDeveloperTag {
                tag_id: input_tag.tag_id,
                offset: off,
                size: input_tag.payload.len() as u32,
            });
        }
    }
    let developer_directory_offset = if directory_entries.is_empty() {
        0
    } else {
        let off = out.len() as u32;
        out.extend_from_slice(&(directory_entries.len() as u16).to_le_bytes());
        for tag in &directory_entries {
            out.extend_from_slice(&tag.tag_id.to_le_bytes());
            out.extend_from_slice(&tag.offset.to_le_bytes());
            out.extend_from_slice(&tag.size.to_le_bytes());
        }
        off
    };

    let extension_area_offset = out.len() as u32;

    // Write extension area, leaving the 4 pointer fields blank for
    // back-patching once we've appended the postage stamp.
    let ext_area_start = out.len();
    write_extension_area_skeleton(&mut out, ext);

    // Postage stamp (optional).
    let postage_stamp_offset = if let Some(stamp) = &ext.postage_stamp {
        let off = out.len() as u32;
        write_postage_stamp(&mut out, stamp, parent_depth)?;
        off
    } else {
        0
    };

    // Colour-correction table (optional, spec §C.6.8).
    let colour_correction_offset = if let Some(cct) = &ext.colour_correction_table {
        let off = out.len() as u32;
        out.extend_from_slice(&cct.to_bytes());
        off
    } else {
        0
    };

    // Scan-line table (optional, spec §C.6.9).
    let scan_line_offset = if let Some(sct) = &ext.scan_line_table {
        let off = out.len() as u32;
        out.extend_from_slice(&sct.to_bytes());
        off
    } else {
        0
    };

    // Back-patch the 4 pointer fields (colour-correction, postage
    // stamp, scan-line, attributes-type) at offsets 482/486/490/494.
    out[ext_area_start + 482..ext_area_start + 486]
        .copy_from_slice(&colour_correction_offset.to_le_bytes());
    out[ext_area_start + 486..ext_area_start + 490]
        .copy_from_slice(&postage_stamp_offset.to_le_bytes());
    out[ext_area_start + 490..ext_area_start + 494]
        .copy_from_slice(&scan_line_offset.to_le_bytes());
    out[ext_area_start + 494] = ext.attributes_type;

    // Footer.
    out.extend_from_slice(&extension_area_offset.to_le_bytes());
    out.extend_from_slice(&developer_directory_offset.to_le_bytes());
    out.extend_from_slice(crate::types::TGA_FOOTER_MAGIC.as_slice());
    Ok(out)
}

fn write_extension_area_skeleton(out: &mut Vec<u8>, ext: &ExtensionAreaInput) {
    let start = out.len();
    // Pre-fill 495 bytes of zero so we can patch fields by offset.
    out.resize(start + TGA_EXTENSION_AREA_SIZE, 0u8);

    // extension_size — always the canonical 495.
    let sz_bytes = (TGA_EXTENSION_AREA_SIZE as u16).to_le_bytes();
    out[start..start + 2].copy_from_slice(&sz_bytes);

    write_str_field(out, start + 2, &ext.author_name, 41);
    for (i, line) in ext.author_comment.iter().enumerate() {
        write_str_field(out, start + 43 + i * 81, line, 81);
    }
    let t = ext.timestamp;
    out[start + 367..start + 369].copy_from_slice(&t.month.to_le_bytes());
    out[start + 369..start + 371].copy_from_slice(&t.day.to_le_bytes());
    out[start + 371..start + 373].copy_from_slice(&t.year.to_le_bytes());
    out[start + 373..start + 375].copy_from_slice(&t.hour.to_le_bytes());
    out[start + 375..start + 377].copy_from_slice(&t.minute.to_le_bytes());
    out[start + 377..start + 379].copy_from_slice(&t.second.to_le_bytes());
    write_str_field(out, start + 379, &ext.job_name, 41);
    out[start + 420..start + 422].copy_from_slice(&ext.job_time.0.to_le_bytes());
    out[start + 422..start + 424].copy_from_slice(&ext.job_time.1.to_le_bytes());
    out[start + 424..start + 426].copy_from_slice(&ext.job_time.2.to_le_bytes());
    write_str_field(out, start + 426, &ext.software_id, 41);
    out[start + 467..start + 469].copy_from_slice(&ext.software_version.0.to_le_bytes());
    out[start + 469] = ext.software_version.1 as u8;
    out[start + 470..start + 474].copy_from_slice(&ext.key_color);
    out[start + 474..start + 476].copy_from_slice(&ext.pixel_aspect_ratio.0.to_le_bytes());
    out[start + 476..start + 478].copy_from_slice(&ext.pixel_aspect_ratio.1.to_le_bytes());
    out[start + 478..start + 480].copy_from_slice(&ext.gamma.0.to_le_bytes());
    out[start + 480..start + 482].copy_from_slice(&ext.gamma.1.to_le_bytes());
    // 482-485: colour_correction_offset, 486-489: postage_stamp_offset,
    // 490-493: scan_line_offset, 494: attributes_type — all back-patched
    // by the caller after the postage stamp is written.
}

fn write_str_field(out: &mut [u8], start: usize, s: &str, capacity: usize) {
    // Spec convention: NUL-terminated strings inside a fixed-width
    // field, padded with NULs (already pre-filled by `resize(0)`).
    let max = capacity.saturating_sub(1);
    let bytes = s.as_bytes();
    let n = bytes.len().min(max);
    out[start..start + n].copy_from_slice(&bytes[..n]);
    // Trailing NUL is already there from the resize.
}

fn write_postage_stamp(out: &mut Vec<u8>, stamp: &TgaImage, parent_depth: u8) -> Result<()> {
    if stamp.width == 0 || stamp.height == 0 {
        return Err(Error::invalid("TGA encoder: postage stamp zero dimension"));
    }
    if stamp.width > 255 || stamp.height > 255 {
        return Err(Error::invalid(
            "TGA encoder: postage stamp dimensions exceed 255 (the on-disk size header is u8)",
        ));
    }
    out.push(stamp.width as u8);
    out.push(stamp.height as u8);

    // Spec §C.6.10: postage stamp pixel format mirrors the parent's
    // (image type + depth). For colour-mapped parents the stamp stores
    // 8-bit indices into the *same* colour map; for true-colour parents
    // it stores BGR(A) at the parent's depth; for grayscale parents it
    // stores 8 bpp luma.
    match stamp.pixel_format {
        TgaPixelFormat::Rgba => match parent_depth {
            32 => {
                for c in stamp.data.chunks_exact(4) {
                    out.push(c[2]);
                    out.push(c[1]);
                    out.push(c[0]);
                    out.push(c[3]);
                }
            }
            24 => {
                for c in stamp.data.chunks_exact(4) {
                    out.push(c[2]);
                    out.push(c[1]);
                    out.push(c[0]);
                }
            }
            8 => {
                // Parent is colour-mapped: we'd need to map every RGBA
                // pixel through the parent's palette to get an index.
                // That's a separate computation the caller can do; we
                // refuse here to avoid silently producing wrong output.
                return Err(Error::unsupported(
                    "TGA encoder: postage_stamp with Rgba pixel format and a colour-mapped parent — \
                     map the thumbnail through the parent palette and pass it as a Gray8 \
                     pixel buffer of indices instead",
                ));
            }
            other => {
                return Err(Error::unsupported(format!(
                    "TGA encoder: postage stamp with parent depth {other} not supported"
                )));
            }
        },
        TgaPixelFormat::Rgb24 => match parent_depth {
            24 => {
                for c in stamp.data.chunks_exact(3) {
                    out.push(c[2]);
                    out.push(c[1]);
                    out.push(c[0]);
                }
            }
            32 => {
                for c in stamp.data.chunks_exact(3) {
                    out.push(c[2]);
                    out.push(c[1]);
                    out.push(c[0]);
                    out.push(0xFF);
                }
            }
            other => {
                return Err(Error::unsupported(format!(
                    "TGA encoder: postage stamp Rgb24 with parent depth {other} not supported"
                )));
            }
        },
        TgaPixelFormat::Gray8 => {
            if parent_depth != 8 {
                return Err(Error::unsupported(format!(
                    "TGA encoder: postage stamp Gray8 only valid with parent depth 8, got {parent_depth}"
                )));
            }
            out.extend_from_slice(&stamp.data);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Image ID field (spec §3.3 / §C.3): the free-form, up-to-255-byte
// identification block written immediately after the 18-byte header.
// ---------------------------------------------------------------------------

/// Maximum size of the optional Image Identification Field (spec §3.3 /
/// §C.3). The on-disk length is stored as a `u8` at header byte 0, so the
/// field can hold at most 255 bytes.
pub const TGA_IMAGE_ID_MAX: usize = 255;

/// Splice an Image Identification Field (spec §3.3 / §C.3) into a freshly
/// encoded TGA byte stream.
///
/// The Image ID is a free-form, up-to-255-byte block written immediately
/// after the 18-byte header. The base encoders ([`encode_tga_uncompressed`],
/// [`encode_tga_rle`], [`encode_tga_palette`], [`encode_tga_palette_rle`],
/// [`encode_tga_grayscale`], [`encode_tga_grayscale_rle`]) all emit
/// `id_length == 0` (no Image ID). This helper rewrites byte 0 of the
/// header to the new length and inserts the supplied bytes at offset 18,
/// shifting the colour map + pixel payload + any trailing footer /
/// extension area / postage stamp accordingly.
///
/// The Image ID is content-opaque per spec — typically a printable string
/// naming the source camera / scene / artist — and is preserved verbatim
/// (no NUL padding or truncation is applied; the caller controls every
/// byte). To splice an Image ID into a file that already carries a
/// TGA 2.0 footer / extension area, call this **before**
/// [`encode_tga_with_extension`]; the footer is appended at the tail and
/// remains tail-aligned after the splice, so the order
/// "base encoder → `splice_image_id` → `encode_tga_with_extension`"
/// produces a single well-formed file.
///
/// Returns [`crate::TgaError::InvalidData`] if the base file is shorter than
/// the 18-byte header, if byte 0 isn't already `0` (i.e. the file already
/// carries an Image ID — the helper refuses to overwrite an existing one),
/// or if `image_id.len() > 255` (the on-disk length field is a single
/// byte).
pub fn splice_image_id(base_tga: &mut Vec<u8>, image_id: &[u8]) -> Result<()> {
    if base_tga.len() < TGA_HEADER_SIZE {
        return Err(Error::invalid(
            "TGA encoder: base_tga too short to contain a header",
        ));
    }
    if base_tga[0] != 0 {
        return Err(Error::invalid(
            "TGA encoder: base_tga already carries an Image ID (id_length != 0)",
        ));
    }
    if image_id.len() > TGA_IMAGE_ID_MAX {
        return Err(Error::invalid(format!(
            "TGA encoder: image_id length {} exceeds spec maximum {}",
            image_id.len(),
            TGA_IMAGE_ID_MAX,
        )));
    }
    if image_id.is_empty() {
        // No-op — the header already declares id_length == 0.
        return Ok(());
    }
    base_tga[0] = image_id.len() as u8;
    // Splice in the bytes after the header. `Vec::splice` over an empty
    // range is the right primitive here: it shifts the existing tail
    // rightwards by image_id.len() bytes in one go.
    base_tga.splice(TGA_HEADER_SIZE..TGA_HEADER_SIZE, image_id.iter().copied());
    Ok(())
}

/// Set the **§5.1 / §5.2 Image Origin** (fixed-header Fields 5.1 + 5.2,
/// bytes 8-11) on a freshly encoded TGA byte stream.
///
/// The base encoders all write the screen-origin default `(0, 0)` — the
/// lower-left corner of a TARGA display. This helper overwrites the
/// X-origin (bytes 8-9) and Y-origin (bytes 10-11) header fields with the
/// supplied [`ImageOrigin`]'s little-endian coordinates so a file can
/// record a non-default on-screen placement of its lower-left corner.
///
/// Unlike [`splice_image_id`], setting the origin does **not** change the
/// file length: both coordinates live in the fixed 18-byte header, so the
/// helper writes them in place and every downstream offset (colour map /
/// pixel data / extension area / footer) is untouched. It therefore
/// composes freely with [`encode_tga_with_extension`] and
/// [`splice_image_id`] in any order.
///
/// The write round-trips bit-exactly through
/// [`crate::parse_tga_image_origin`] / [`crate::TgaHeader::image_origin`]:
/// reading the origin back from the modified file reproduces the supplied
/// [`ImageOrigin`]. The recorded origin is on-screen placement metadata
/// only — it does not move the raster, and [`crate::parse_tga`] decodes
/// the same pixels regardless of its value.
///
/// Returns [`crate::TgaError::InvalidData`] if the base file is shorter than
/// the 18-byte header.
pub fn set_image_origin(base_tga: &mut [u8], origin: ImageOrigin) -> Result<()> {
    if base_tga.len() < TGA_HEADER_SIZE {
        return Err(Error::invalid(
            "TGA encoder: base_tga too short to contain a header",
        ));
    }
    base_tga[8..12].copy_from_slice(&origin.to_bytes());
    Ok(())
}
