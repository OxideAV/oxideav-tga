//! TGA encode.
//!
//! Round 1 ships two write paths, both true-colour:
//!
//! * [`encode_tga_uncompressed`] — image type 2 (uncompressed BGR /
//!   BGRA), 24 or 32 bpp depending on the input alpha channel.
//! * [`encode_tga_rle`] — image type 10 (RLE BGR / BGRA), 24 or 32 bpp.
//!   The RLE encoder picks runs of ≥ 2 consecutive identical pixels
//!   (max 128 per run-packet) vs raw runs (max 128 per raw-packet) per
//!   spec §C.5.
//!
//! Both write a top-down image (image-descriptor bit 5 set) so decoders
//! that don't honour the bit (some old viewers) still display them
//! right-side up. The output is a TGA 1.0 file: no footer, no extension
//! area. Round 2 will optionally append the 26-byte footer + extension
//! area.

use crate::error::{Result, TgaError as Error};
use crate::image::{TgaImage, TgaPixelFormat};
use crate::types::*;

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
        TgaPixelFormat::Gray8 => Err(Error::unsupported(
            "TGA encoder: Gray8 input not supported in round 1 (use Rgba/Rgb24)",
        )),
    }
}
