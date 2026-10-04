//! The root vocabulary of the image-crate contract (`IMAGE_CRATE_API`):
//! `probe` / `info` / `decode*` / `encode*`, all framework-free, plus
//! the typed [`header`] depth accessor.

use std::io::{Read, Write};

use crate::decoder::{decode_image, header_info, probe_bytes, validate_header};
use crate::encoder::encode_image;
use crate::error::{Result, TgaError as Error};
use crate::image::{ImageInfo, RgbImage, RgbaImage, TgaImage};
use crate::options::{DecodeOptions, EncodeOptions};
use crate::types::TgaHeader;

/// `true` when `bytes` looks like a TGA file. TGA has no leading magic,
/// so this is a plausibility test: the TGA 2.0 `TRUEVISION-XFILE.\0`
/// footer at the tail is a definite yes; otherwise the 18-byte header
/// must carry a known image type (1 / 2 / 3 / 9 / 10 / 11), a depth
/// that type allows and non-zero dimensions. Allocation-free; `false`
/// on short input.
pub fn probe(bytes: &[u8]) -> bool {
    probe_bytes(bytes)
}

/// Describe a TGA from its header (and the optional footer) without
/// decoding pixels: dimensions, the native layout [`decode`] would
/// return, alpha, and which TGA 2.0 structures are present. Fails with
/// the same header errors [`decode`] would.
pub fn info(bytes: &[u8]) -> Result<ImageInfo> {
    header_info(bytes)
}

/// The typed 18-byte header (§C.2), validated as [`decode`] validates
/// it (known image type, allowed depth, non-zero geometry). The depth
/// accessor behind [`info`]; see [`TgaHeader`] for the typed views
/// (attribute bits, interleaving, screen origin, colour-map type,
/// storage order).
pub fn header(bytes: &[u8]) -> Result<TgaHeader> {
    validate_header(bytes, false).map(|v| v.header)
}

/// Decode a TGA into its native layout with [`DecodeOptions::default`].
pub fn decode(bytes: &[u8]) -> Result<TgaImage> {
    decode_image(bytes, &DecodeOptions::default())
}

/// [`decode`] under explicit limits / strictness.
pub fn decode_with(bytes: &[u8], opts: &DecodeOptions) -> Result<TgaImage> {
    decode_image(bytes, opts)
}

/// Decode straight to tightly packed 8-bit RGB (alpha dropped). See
/// [`TgaImage::to_rgb8`] for the per-layout kernels.
pub fn decode_rgb8(bytes: &[u8]) -> Result<RgbImage> {
    let img = decode(bytes)?;
    Ok(RgbImage::new(img.width, img.height, img.to_rgb8()))
}

/// Decode straight to tightly packed 8-bit RGBA (alpha `255` where the
/// source has none; palette alpha applied). See [`TgaImage::to_rgba8`]
/// for the per-layout kernels.
pub fn decode_rgba8(bytes: &[u8]) -> Result<RgbaImage> {
    let img = decode(bytes)?;
    Ok(RgbaImage::new(img.width, img.height, img.to_rgba8()))
}

/// Read `r` to its end and [`decode`] the bytes.
pub fn decode_from<R: Read>(mut r: R) -> Result<TgaImage> {
    let mut buf = Vec::new();
    r.read_to_end(&mut buf)?;
    decode(&buf)
}

/// Encode `image` as a TGA file. See [`EncodeOptions`] for the layout →
/// wire mapping; the layout is written as given (`Rgba` at 32 bits,
/// `Rgb24` at 24, `Gray8` as type 3 / 11, `Pal8` + palette as type
/// 1 / 9) — never a silent conversion. A `Pal8` image without a
/// palette, or with an index outside it, is [`Error::InvalidData`];
/// dimensions over 65535 are [`Error::Unsupported`].
pub fn encode(image: &TgaImage, opts: &EncodeOptions) -> Result<Vec<u8>> {
    encode_image(image, opts)
}

/// Encode tightly packed 8-bit RGB (`3 × width × height` bytes) as a
/// 24-bit true-colour TGA (image type 10, or 2 with `rle = false`).
pub fn encode_rgb8(width: u32, height: u32, rgb: &[u8], opts: &EncodeOptions) -> Result<Vec<u8>> {
    check_raw_len(width, height, 3, rgb.len())?;
    encode_image(&TgaImage::from_rgb8(width, height, rgb.to_vec())?, opts)
}

/// Encode tightly packed 8-bit RGBA (`4 × width × height` bytes) as a
/// 32-bit true-colour TGA (image type 10, or 2 with `rle = false`) with
/// 8 descriptor alpha bits.
pub fn encode_rgba8(width: u32, height: u32, rgba: &[u8], opts: &EncodeOptions) -> Result<Vec<u8>> {
    check_raw_len(width, height, 4, rgba.len())?;
    encode_image(&TgaImage::from_rgba8(width, height, rgba.to_vec())?, opts)
}

/// [`encode`] straight into a writer.
pub fn encode_to<W: Write>(image: &TgaImage, opts: &EncodeOptions, mut w: W) -> Result<()> {
    let bytes = encode_image(image, opts)?;
    w.write_all(&bytes)?;
    Ok(())
}

fn check_raw_len(width: u32, height: u32, bpp: usize, len: usize) -> Result<()> {
    let need = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(bpp))
        .ok_or_else(|| Error::invalid("TGA encoder: dimensions overflow"))?;
    if len < need {
        return Err(Error::invalid(format!(
            "TGA encoder: {width}x{height} at {bpp} bytes/pixel needs {need} bytes, got {len}"
        )));
    }
    Ok(())
}
