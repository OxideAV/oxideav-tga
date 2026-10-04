//! TGA encode: the one writer behind [`crate::encode`] /
//! [`crate::encode_rgb8`] / [`crate::encode_rgba8`] /
//! [`crate::encode_to`], plus the byte-level TGA 2.0 helpers
//! ([`encode_tga_with_extension`], [`splice_image_id`],
//! [`set_image_origin`]).
//!
//! Every [`crate::EncodeOptions`] field is a behaviour the pre-contract
//! function family selected by name: `rle` picks image types 9 / 10 /
//! 11 over 1 / 2 / 3, `row_order` the descriptor origin bit, the
//! image's layout the colour model —
//!
//! | Layout | Image type | Depth on the wire |
//! |---|---|---|
//! | `Pal8` + palette | 1 / 9 | 8-bit indices + 15 / 16 / 24 / 32-bit colour map |
//! | `Rgb24` | 2 / 10 | 24 (BGR) |
//! | `Rgba` | 2 / 10 | 32 (BGRA), descriptor alpha bits 8 |
//! | `Gray8` | 3 / 11 | 8 |
//!
//! The RLE packetiser (§C.5) runs per row — a run packet for ≥ 2
//! consecutive identical pixels (max 128), raw packets otherwise (max
//! 128) — and never lets a packet cross a scan line, so a §C.6.9
//! scan-line table can always describe the output. The base output is
//! a TGA 1.0 file; the TGA 2.0 footer + 495-byte extension area are
//! appended when the options or the image's metadata ask for them.
//!
//! The pre-contract writers (`encode_tga_uncompressed`, `encode_tga_rle`,
//! `…_rgb24`, `…_image`, `encode_tga_palette*`, `encode_tga_grayscale*`)
//! remain as deprecated wrappers that produce byte-identical files.

use crate::error::{Result, TgaError as Error};
use crate::image::{Palette, TgaImage, TgaPixelFormat};
use crate::options::{EncodeOptions, RowOrder};
use crate::types::{
    ColorMapEntrySize, ImageOrigin, ImageType, TgaColourCorrectionTable, TgaDeveloperTag,
    TgaScanLineTable, TGA_EXTENSION_AREA_SIZE, TGA_FOOTER_SIZE, TGA_HEADER_SIZE,
};

/// The framework factory moved to [`crate::registry`]; this path stays
/// so `oxideav_tga::encoder::make_encoder` keeps resolving.
#[cfg(feature = "registry")]
pub use crate::registry::make_encoder;

// ---------------------------------------------------------------------------
// The one encoder
// ---------------------------------------------------------------------------

/// Encode `image` as a TGA file under `opts`. See the module docs for
/// the layout → wire mapping; [`crate::encode`] is the public name.
pub(crate) fn encode_image(image: &TgaImage, opts: &EncodeOptions) -> Result<Vec<u8>> {
    image.validate()?;
    let width: u16 = image
        .width
        .try_into()
        .map_err(|_| Error::unsupported("TGA encoder: width exceeds 65535"))?;
    let height: u16 = image
        .height
        .try_into()
        .map_err(|_| Error::unsupported("TGA encoder: height exceeds 65535"))?;
    if opts.image_id.len() > TGA_IMAGE_ID_MAX {
        return Err(Error::invalid(format!(
            "TGA encoder: image_id length {} exceeds spec maximum {}",
            opts.image_id.len(),
            TGA_IMAGE_ID_MAX,
        )));
    }

    // The wire layout: image type, depth, descriptor alpha bits, the
    // bytes per source pixel the row writer reads, and the colour map.
    let mut format = image.format;
    if format == TgaPixelFormat::Rgba
        && opts.drop_opaque_alpha
        && image.data().chunks_exact(4).all(|p| p[3] == 0xFF)
    {
        format = TgaPixelFormat::Rgb24;
    }
    let image_type = format.image_type(opts.rle);
    let (depth, alpha_bits) = match format {
        TgaPixelFormat::Rgba => (32u8, opts.alpha_bits.unwrap_or(8) & 0x0F),
        TgaPixelFormat::Rgb24 => (24, 0),
        TgaPixelFormat::Gray8 | TgaPixelFormat::Pal8 => (8, 0),
    };
    let palette: Option<(&Palette, ColorMapEntrySize)> = match format {
        TgaPixelFormat::Pal8 => {
            let pal = image
                .palette
                .as_ref()
                .ok_or_else(|| Error::invalid("TGA encoder: Pal8 image without a palette"))?;
            let entry = opts
                .palette_entry_size
                .unwrap_or_else(|| ColorMapEntrySize::smallest_lossless_for(&pal.entries));
            Some((pal, entry))
        }
        _ => None,
    };

    let w = width as usize;
    let h = height as usize;
    let src_bpp = image.bytes_per_pixel();
    let out_bpp = depth as usize / 8;
    let map_bytes = palette.map(|(p, e)| p.len() * e.bytes()).unwrap_or(0);
    let mut out = Vec::with_capacity(
        TGA_HEADER_SIZE
            + opts.image_id.len()
            + map_bytes
            + w * h * out_bpp / if opts.rle { 2 } else { 1 },
    );

    // Header (§C.2).
    out.push(opts.image_id.len() as u8);
    out.push(u8::from(palette.is_some()));
    out.push(image_type as u8);
    out.extend_from_slice(&0u16.to_le_bytes()); // cmap_first
    out.extend_from_slice(&(palette.map(|(p, _)| p.len() as u16).unwrap_or(0)).to_le_bytes());
    out.push(palette.map(|(_, e)| e.bits()).unwrap_or(0));
    out.extend_from_slice(&opts.screen_origin.to_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.push(depth);
    let origin_bit = match opts.row_order {
        RowOrder::TopDown => 0x20,
        RowOrder::BottomUp => 0x00,
    };
    out.push((alpha_bits & 0x0F) | origin_bit);

    // Image ID (§C.3), colour map (§C.4).
    out.extend_from_slice(&opts.image_id);
    if let Some((pal, entry)) = palette {
        for e in &pal.entries {
            push_colour_map_entry(&mut out, *e, entry);
        }
    }

    // Pixel array (§C.5), one row at a time in storage order.
    let stride = image.stride();
    let data = image.data();
    let row_bytes = w * src_bpp;
    let mut wire_row: Vec<u8> = Vec::with_capacity(w * out_bpp);
    for i in 0..h {
        let y = match opts.row_order {
            RowOrder::TopDown => i,
            RowOrder::BottomUp => h - 1 - i,
        };
        let row = &data[y * stride..y * stride + row_bytes];
        wire_row.clear();
        match (image.format, format) {
            // RGBA source written at 32 bits: BGRA.
            (TgaPixelFormat::Rgba, TgaPixelFormat::Rgba) => {
                for p in row.chunks_exact(4) {
                    wire_row.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
                }
            }
            // RGBA source with uniformly opaque alpha, dropped to 24 bits.
            (TgaPixelFormat::Rgba, TgaPixelFormat::Rgb24) => {
                for p in row.chunks_exact(4) {
                    wire_row.extend_from_slice(&[p[2], p[1], p[0]]);
                }
            }
            (TgaPixelFormat::Rgb24, _) => {
                for p in row.chunks_exact(3) {
                    wire_row.extend_from_slice(&[p[2], p[1], p[0]]);
                }
            }
            // Indices and luma go out as they are.
            (TgaPixelFormat::Gray8 | TgaPixelFormat::Pal8, _) => wire_row.extend_from_slice(row),
            (TgaPixelFormat::Rgba, _) => unreachable!("Rgba only narrows to Rgb24"),
        }
        if opts.rle {
            rle_one_row(&wire_row, out_bpp, &mut out);
        } else {
            out.extend_from_slice(&wire_row);
        }
    }

    // TGA 2.0 tail (§C.6 - §C.8): explicit extension, else the image's
    // own record, else a gamma-only area, else a bare footer on request.
    let gamma_ratio = image.metadata.gamma.map(gamma_to_ratio);
    let ext = match (&opts.extension, &image.extension) {
        (Some(e), _) => Some(sync_gamma(e.clone(), gamma_ratio)),
        (None, Some(rec)) => Some(sync_gamma(ExtensionAreaInput::from_area(rec), gamma_ratio)),
        (None, None) => gamma_ratio.map(|g| ExtensionAreaInput {
            gamma: g,
            ..ExtensionAreaInput::default()
        }),
    };
    match ext {
        Some(ext) => encode_tga_with_extension(&out, &ext),
        None if opts.footer => {
            out.extend_from_slice(&crate::types::TgaFooter::default().to_bytes());
            Ok(out)
        }
        None => Ok(out),
    }
}

/// A float gamma as the §C.6.6 SHORT pair: `(round(g × 100), 100)`
/// — one decimal place is what the spec promises ("gamma value with
/// one decimal place of useful precision"); two are kept.
fn gamma_to_ratio(g: f32) -> (u16, u16) {
    if !(g.is_finite() && g > 0.0) {
        return (0, 0);
    }
    let num = (g * 100.0).round().clamp(1.0, u16::MAX as f32) as u16;
    (num, 100)
}

/// Fill an unset extension gamma from the image's `metadata.gamma`.
fn sync_gamma(mut ext: ExtensionAreaInput, gamma: Option<(u16, u16)>) -> ExtensionAreaInput {
    if ext.gamma.1 == 0 {
        if let Some(g) = gamma {
            ext.gamma = g;
        }
    }
    ext
}

/// §C.5 RLE packetisation of one wire-order row of `bpp`-byte pixels:
/// a run packet for ≥ 2 consecutive identical pixels (max 128), raw
/// packets otherwise (max 128); packets never span rows.
fn rle_one_row(row: &[u8], bpp: usize, out: &mut Vec<u8>) {
    let n = row.len() / bpp;
    let px = |i: usize| &row[i * bpp..i * bpp + bpp];
    let mut i = 0;
    while i < n {
        // Look at the run-length starting at `i`.
        let mut run = 1;
        while i + run < n && px(i + run) == px(i) && run < 128 {
            run += 1;
        }
        if run >= 2 {
            // Emit run-length packet.
            out.push(0x80 | (run as u8 - 1));
            out.extend_from_slice(px(i));
            i += run;
        } else {
            // Collect a raw run: pixels that don't begin a run of ≥ 2.
            // Stop at length 128 OR right before a position that *does*
            // begin a run of ≥ 2 (so we leave it for the next iteration).
            let raw_start = i;
            let mut raw_end = i + 1;
            while raw_end < n && raw_end - raw_start < 128 {
                if raw_end + 1 < n && px(raw_end + 1) == px(raw_end) {
                    break;
                }
                raw_end += 1;
            }
            let count = raw_end - raw_start;
            out.push(((count as u8) - 1) & 0x7F);
            out.extend_from_slice(&row[raw_start * bpp..raw_end * bpp]);
            i = raw_end;
        }
    }
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
// Pre-contract writers — deprecated wrappers, byte-identical output
// ---------------------------------------------------------------------------

/// Options reproducing the pre-contract true-colour writers: the
/// auto-depth rule (24 bpp when every alpha is `0xFF`), top-down, no
/// footer.
fn legacy_opts(rle: bool) -> EncodeOptions {
    EncodeOptions::default()
        .with_rle(rle)
        .with_drop_opaque_alpha(true)
}

fn check_raw_len(width: u16, height: u16, bpp: usize, len: usize, what: &str) -> Result<()> {
    if len % bpp != 0 {
        return Err(Error::invalid(format!(
            "TGA encoder: {what} input length not multiple of {bpp}"
        )));
    }
    if len < width as usize * height as usize * bpp {
        return Err(Error::invalid(format!(
            "TGA encoder: {what} input shorter than w*h*{bpp}"
        )));
    }
    Ok(())
}

/// Encode `width × height` RGBA bytes (4 bytes per pixel, top-down,
/// row-major) into an uncompressed TGA file (image type 2). Output
/// depth is 32 bpp if any input alpha byte is `< 0xFF`, otherwise
/// 24 bpp.
#[deprecated(
    note = "use oxideav_tga::encode_rgba8 with EncodeOptions::default().with_rle(false) (IMAGE_CRATE_API)"
)]
pub fn encode_tga_uncompressed(width: u16, height: u16, rgba: &[u8]) -> Result<Vec<u8>> {
    check_raw_len(width, height, 4, rgba.len(), "RGBA")?;
    let img = TgaImage::from_rgba8(width as u32, height as u32, rgba.to_vec())?;
    encode_image(&img, &legacy_opts(false))
}

/// Encode `width × height` **RGB24** bytes (3 bytes per pixel, top-down,
/// row-major) into an uncompressed TGA file (image type 2) at 24 bpp.
#[deprecated(
    note = "use oxideav_tga::encode_rgb8 with EncodeOptions::default().with_rle(false) (IMAGE_CRATE_API)"
)]
pub fn encode_tga_uncompressed_rgb24(width: u16, height: u16, rgb: &[u8]) -> Result<Vec<u8>> {
    check_raw_len(width, height, 3, rgb.len(), "rgb")?;
    let img = TgaImage::from_rgb8(width as u32, height as u32, rgb.to_vec())?;
    encode_image(&img, &legacy_opts(false))
}

/// Encode `width × height` **RGB24** bytes (3 bytes per pixel, top-down,
/// row-major) into an RLE TGA file (image type 10) at 24 bpp.
#[deprecated(note = "use oxideav_tga::encode_rgb8 (IMAGE_CRATE_API)")]
pub fn encode_tga_rle_rgb24(width: u16, height: u16, rgb: &[u8]) -> Result<Vec<u8>> {
    check_raw_len(width, height, 3, rgb.len(), "rgb")?;
    let img = TgaImage::from_rgb8(width as u32, height as u32, rgb.to_vec())?;
    encode_image(&img, &legacy_opts(true))
}

/// Encode `width × height` RGBA bytes (4 bytes per pixel, top-down,
/// row-major) into an RLE TGA file (image type 10). Output depth is
/// 32 bpp if any input alpha byte is `< 0xFF`, otherwise 24 bpp.
#[deprecated(note = "use oxideav_tga::encode_rgba8 (IMAGE_CRATE_API)")]
pub fn encode_tga_rle(width: u16, height: u16, rgba: &[u8]) -> Result<Vec<u8>> {
    check_raw_len(width, height, 4, rgba.len(), "RGBA")?;
    let img = TgaImage::from_rgba8(width as u32, height as u32, rgba.to_vec())?;
    encode_image(&img, &legacy_opts(true))
}

/// The pre-contract image writers promoted every layout to RGBA (a
/// `Gray8` image became true-colour grey) before applying the
/// auto-depth rule.
fn legacy_image(image: &TgaImage) -> Result<TgaImage> {
    image.validate()?;
    TgaImage::from_rgba8(image.width, image.height, image.to_rgba8())
}

/// Wrapper so callers with a [`TgaImage`] don't need to flatten by hand.
/// Picks the same depth-selection rule as [`encode_tga_uncompressed`].
#[deprecated(
    note = "use oxideav_tga::encode with EncodeOptions::default().with_rle(false) (IMAGE_CRATE_API)"
)]
pub fn encode_tga_uncompressed_image(image: &TgaImage) -> Result<Vec<u8>> {
    encode_image(&legacy_image(image)?, &legacy_opts(false))
}

/// Wrapper so callers with a [`TgaImage`] don't need to flatten by hand.
#[deprecated(note = "use oxideav_tga::encode (IMAGE_CRATE_API)")]
pub fn encode_tga_rle_image(image: &TgaImage) -> Result<Vec<u8>> {
    encode_image(&legacy_image(image)?, &legacy_opts(true))
}

/// Encode `width × height` RGBA bytes into an uncompressed colour-mapped
/// TGA file (image type 1) with an 8-bit palette index.
///
/// The palette is built from the unique RGBA colours in the input
/// ([`TgaImage::to_indexed`]); the colour-map entry size auto-selects
/// to the narrowest lossless width. Returns
/// [`crate::TgaError::Unsupported`] if the input contains more than 256
/// unique RGBA colours.
#[deprecated(
    note = "use TgaImage::from_rgba8(..).to_indexed() + oxideav_tga::encode with EncodeOptions::default().with_rle(false) (IMAGE_CRATE_API)"
)]
pub fn encode_tga_palette(width: u16, height: u16, rgba: &[u8]) -> Result<Vec<u8>> {
    encode_palette_inner(width, height, rgba, false, None)
}

/// Encode `width × height` RGBA bytes into an RLE colour-mapped TGA
/// file (image type 9).
#[deprecated(
    note = "use TgaImage::from_rgba8(..).to_indexed() + oxideav_tga::encode (IMAGE_CRATE_API)"
)]
pub fn encode_tga_palette_rle(width: u16, height: u16, rgba: &[u8]) -> Result<Vec<u8>> {
    encode_palette_inner(width, height, rgba, true, None)
}

/// Encode `width × height` RGBA bytes into a colour-mapped TGA file with
/// an explicit colour-map [`ColorMapEntrySize`].
///
/// `image_type` must be a colour-mapped type ([`ImageType::UncompressedColourMapped`]
/// = 1 or [`ImageType::RleColourMapped`] = 9); any other type is rejected
/// with [`crate::TgaError::InvalidData`]. 15 / 16-bit entries quantise
/// each channel to 5 bits (see [`ColorMapEntrySize`]).
#[deprecated(
    note = "use TgaImage::to_indexed() + oxideav_tga::encode with EncodeOptions::with_palette_entry_size (IMAGE_CRATE_API)"
)]
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
    encode_palette_inner(width, height, rgba, image_type.is_rle(), Some(entry_size))
}

fn encode_palette_inner(
    width: u16,
    height: u16,
    rgba: &[u8],
    rle: bool,
    entry_size: Option<ColorMapEntrySize>,
) -> Result<Vec<u8>> {
    check_raw_len(width, height, 4, rgba.len(), "palette")?;
    let n = width as usize * height as usize * 4;
    let img =
        TgaImage::from_rgba8(width as u32, height as u32, rgba[..n].to_vec())?.to_indexed()?;
    encode_image(
        &img,
        &EncodeOptions::default()
            .with_rle(rle)
            .with_palette_entry_size(entry_size),
    )
}

/// Encode `width × height` Gray8 bytes (1 byte per pixel, top-down,
/// row-major) into an uncompressed grayscale TGA file (image type 3).
#[deprecated(
    note = "use TgaImage::from_gray8 + oxideav_tga::encode with EncodeOptions::default().with_rle(false) (IMAGE_CRATE_API)"
)]
pub fn encode_tga_grayscale(width: u16, height: u16, gray: &[u8]) -> Result<Vec<u8>> {
    encode_grayscale_inner(width, height, gray, false)
}

/// Encode `width × height` Gray8 bytes into an RLE grayscale TGA file
/// (image type 11).
#[deprecated(note = "use TgaImage::from_gray8 + oxideav_tga::encode (IMAGE_CRATE_API)")]
pub fn encode_tga_grayscale_rle(width: u16, height: u16, gray: &[u8]) -> Result<Vec<u8>> {
    encode_grayscale_inner(width, height, gray, true)
}

fn encode_grayscale_inner(width: u16, height: u16, gray: &[u8], rle: bool) -> Result<Vec<u8>> {
    if gray.len() < width as usize * height as usize {
        return Err(Error::invalid(
            "TGA grayscale encoder: input shorter than w*h",
        ));
    }
    let n = width as usize * height as usize;
    let img = TgaImage::from_gray8(width as u32, height as u32, gray[..n].to_vec())?;
    encode_image(&img, &EncodeOptions::default().with_rle(rle))
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, Default, PartialEq)]
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

impl ExtensionAreaInput {
    /// Author an input from a decoded [`TgaExtensionArea`](crate::types::TgaExtensionArea) record (as
    /// [`crate::decode`] places on [`TgaImage::extension`]): every
    /// scalar / string field is copied; the three table / stamp
    /// pointers are not (they refer to the source file's byte layout —
    /// supply [`Self::postage_stamp`] / [`Self::colour_correction_table`]
    /// / [`Self::scan_line_table`] / [`Self::developer_tags`] explicitly
    /// to re-emit those).
    pub fn from_area(area: &crate::types::TgaExtensionArea) -> Self {
        Self {
            author_name: area.author_name.clone(),
            author_comment: area.author_comment.clone(),
            timestamp: area.timestamp,
            job_name: area.job_name.clone(),
            job_time: area.job_time,
            software_id: area.software_id.clone(),
            software_version: area.software_version,
            key_color: area.key_color,
            pixel_aspect_ratio: area.pixel_aspect_ratio,
            gamma: area.gamma,
            attributes_type: area.attributes_type,
            postage_stamp: None,
            colour_correction_table: None,
            scan_line_table: None,
            developer_tags: Vec::new(),
        }
    }
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
/// `depth = 32`, etc. The supplied `stamp.format` is treated as
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
        .map(|s| s.data().len())
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
    let data = stamp.to_rgba8();
    match stamp.format {
        TgaPixelFormat::Rgba | TgaPixelFormat::Rgb24 => match parent_depth {
            32 => {
                for c in data.chunks_exact(4) {
                    out.push(c[2]);
                    out.push(c[1]);
                    out.push(c[0]);
                    out.push(c[3]);
                }
            }
            24 => {
                for c in data.chunks_exact(4) {
                    out.push(c[2]);
                    out.push(c[1]);
                    out.push(c[0]);
                }
            }
            8 if stamp.format == TgaPixelFormat::Rgba => {
                // Parent is colour-mapped: we'd need to map every RGBA
                // pixel through the parent's palette to get an index.
                // That's a separate computation the caller can do; we
                // refuse here to avoid silently producing wrong output.
                return Err(Error::unsupported(
                    "TGA encoder: postage_stamp with Rgba pixel format and a colour-mapped parent — \
                     map the thumbnail through the parent palette and pass it as a Pal8 / Gray8 \
                     pixel buffer of indices instead",
                ));
            }
            other => {
                return Err(Error::unsupported(format!(
                    "TGA encoder: postage stamp {:?} with parent depth {other} not supported",
                    stamp.format
                )));
            }
        },
        // Luma for a grayscale parent, or indices for a colour-mapped
        // parent (the stamp shares the parent's colour map, §C.6.10).
        TgaPixelFormat::Gray8 | TgaPixelFormat::Pal8 => {
            if parent_depth != 8 {
                return Err(Error::unsupported(format!(
                    "TGA encoder: postage stamp {:?} only valid with parent depth 8, got {parent_depth}",
                    stamp.format
                )));
            }
            let w = stamp.width as usize;
            let stride = stamp.stride();
            let src = stamp.data();
            for y in 0..stamp.height as usize {
                out.extend_from_slice(&src[y * stride..y * stride + w]);
            }
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
