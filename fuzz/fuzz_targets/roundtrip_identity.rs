#![no_main]

//! Bit-exact encode→decode **identity** target for the lossless writers.
//!
//! The sibling [`encode_roundtrip`](encode_roundtrip.rs) target only
//! asserts *frame shape* (`width` / `height` / `pixel_format` / byte
//! length) because it drives every writer, including the lossy ones
//! (15/16-bit colour-map quantisation, the RGB24-input writers that always
//! drop to 24 bpp). This target attacks the complement: it drives only the
//! writers whose output the decoder recovers **byte-for-byte**, and asserts
//! the decoded raster equals a locally-computed expected buffer.
//!
//! Which writers are lossless, and what "expected" is for each:
//!
//! * `encode_tga_uncompressed` / `encode_tga_rle` (RGBA input) — always
//!   lossless. If any input alpha is `< 0xFF` the writer emits 32 bpp and
//!   the full quad round-trips; if every alpha is `0xFF` it drops to 24 bpp
//!   and the decoder forces alpha back to `0xFF` — identical to the input.
//!   So `expected == input`.
//! * `encode_tga_uncompressed_rgb24` / `encode_tga_rle_rgb24` (RGB input) —
//!   lossless on colour; the decoder appends `0xFF` alpha. `expected` is
//!   the RGB buffer widened to RGBA with `0xFF` alpha.
//! * `encode_tga_grayscale` / `encode_tga_grayscale_rle` (Gray8 input) —
//!   `expected == input`.
//! * `encode_tga_palette` / `encode_tga_palette_rle` (RGBA, ≤256 colours) —
//!   the default writers pick the narrowest *lossless* colour-map entry
//!   size, so a successful encode round-trips exactly: `expected == input`.
//! * `encode_tga_palette_with_entry_size(…, Bits32)` — lossless, so
//!   `expected == input`. `Bits24` drops alpha (forced `0xFF` on decode);
//!   `expected` is the RGB channels with `0xFF` alpha.
//!
//! The lossy 15/16-bit colour-map entry sizes are intentionally excluded —
//! they have no clean-room-derivable expected output without re-implementing
//! the 5-bit quantiser in the harness, which would just be testing the test.
//!
//! Same 16-MiB raster cap and fuzz-tail tiling as the sibling targets. A
//! `parse_tga` failure on an accepted encode is a decoder bug (the encoder
//! produced a file it can't read back) and trips the identity assertion via
//! the `expect`.

use libfuzzer_sys::fuzz_target;
use oxideav_tga::{
    encode_tga_grayscale, encode_tga_grayscale_rle, encode_tga_palette, encode_tga_palette_rle,
    encode_tga_palette_with_entry_size, encode_tga_rle, encode_tga_rle_rgb24,
    encode_tga_uncompressed, encode_tga_uncompressed_rgb24, parse_tga, ColorMapEntrySize,
    ImageType, TgaPixelFormat,
};

const MAX_PIXELS: usize = 4 * 1024 * 1024;
const MAX_SIDE: u16 = 4096;

#[derive(Clone, Copy)]
enum Writer {
    UncompressedRgba,
    RleRgba,
    UncompressedRgb24,
    RleRgb24,
    Grayscale,
    GrayscaleRle,
    Palette,
    PaletteRle,
    PaletteEntry32Type1,
    PaletteEntry24Type1,
    PaletteEntry32Type9,
    PaletteEntry24Type9,
}

impl Writer {
    fn from_byte(b: u8) -> Self {
        match b % 12 {
            0 => Writer::UncompressedRgba,
            1 => Writer::RleRgba,
            2 => Writer::UncompressedRgb24,
            3 => Writer::RleRgb24,
            4 => Writer::Grayscale,
            5 => Writer::GrayscaleRle,
            6 => Writer::Palette,
            7 => Writer::PaletteRle,
            8 => Writer::PaletteEntry32Type1,
            9 => Writer::PaletteEntry24Type1,
            10 => Writer::PaletteEntry32Type9,
            _ => Writer::PaletteEntry24Type9,
        }
    }

    fn input_bpp(self) -> usize {
        match self {
            Writer::UncompressedRgb24 | Writer::RleRgb24 => 3,
            Writer::Grayscale | Writer::GrayscaleRle => 1,
            _ => 4,
        }
    }

    fn decoded_format(self) -> TgaPixelFormat {
        match self {
            Writer::Grayscale | Writer::GrayscaleRle => TgaPixelFormat::Gray8,
            _ => TgaPixelFormat::Rgba,
        }
    }

    /// True when the writer forces alpha to `0xFF` on decode (RGB-input and
    /// 24-bit colour-map writers), so the expected RGBA buffer must widen
    /// the colour channels with an opaque alpha rather than copy input.
    fn forces_opaque(self) -> bool {
        matches!(
            self,
            Writer::UncompressedRgb24
                | Writer::RleRgb24
                | Writer::PaletteEntry24Type1
                | Writer::PaletteEntry24Type9
        )
    }

    fn encode(self, w: u16, h: u16, px: &[u8]) -> oxideav_tga::Result<Vec<u8>> {
        match self {
            Writer::UncompressedRgba => encode_tga_uncompressed(w, h, px),
            Writer::RleRgba => encode_tga_rle(w, h, px),
            Writer::UncompressedRgb24 => encode_tga_uncompressed_rgb24(w, h, px),
            Writer::RleRgb24 => encode_tga_rle_rgb24(w, h, px),
            Writer::Grayscale => encode_tga_grayscale(w, h, px),
            Writer::GrayscaleRle => encode_tga_grayscale_rle(w, h, px),
            Writer::Palette => encode_tga_palette(w, h, px),
            Writer::PaletteRle => encode_tga_palette_rle(w, h, px),
            Writer::PaletteEntry32Type1 => encode_tga_palette_with_entry_size(
                w,
                h,
                px,
                ImageType::UncompressedColourMapped,
                ColorMapEntrySize::Bits32,
            ),
            Writer::PaletteEntry24Type1 => encode_tga_palette_with_entry_size(
                w,
                h,
                px,
                ImageType::UncompressedColourMapped,
                ColorMapEntrySize::Bits24,
            ),
            Writer::PaletteEntry32Type9 => encode_tga_palette_with_entry_size(
                w,
                h,
                px,
                ImageType::RleColourMapped,
                ColorMapEntrySize::Bits32,
            ),
            Writer::PaletteEntry24Type9 => encode_tga_palette_with_entry_size(
                w,
                h,
                px,
                ImageType::RleColourMapped,
                ColorMapEntrySize::Bits24,
            ),
        }
    }
}

/// Widen an RGB (3-byte) buffer to RGBA with `0xFF` alpha.
fn widen_rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for chunk in rgb.chunks_exact(3) {
        out.extend_from_slice(chunk);
        out.push(0xFF);
    }
    out
}

/// Force the alpha byte of every RGBA pixel to `0xFF`.
fn force_opaque_rgba(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        px[3] = 0xFF;
    }
    out
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 6 {
        return;
    }
    let writer = Writer::from_byte(data[0]);
    let raw_w = u16::from_le_bytes([data[1], data[2]]);
    let raw_h = u16::from_le_bytes([data[3], data[4]]);
    let w = (raw_w % MAX_SIDE).max(1);
    let h = (raw_h % MAX_SIDE).max(1);

    let bpp = writer.input_bpp();
    let need_pixels = w as usize * h as usize;
    if need_pixels > MAX_PIXELS {
        return;
    }
    let need_bytes = need_pixels * bpp;

    let tail = &data[5..];
    let mut pixels = Vec::with_capacity(need_bytes);
    while pixels.len() < need_bytes {
        let take = (need_bytes - pixels.len()).min(tail.len());
        pixels.extend_from_slice(&tail[..take]);
    }

    let bytes = match writer.encode(w, h, &pixels) {
        Ok(b) => b,
        // Length mismatch / >256-colour palette overflow are documented
        // Err returns, not identity findings.
        Err(_) => return,
    };

    // The encoder accepted the input, so the decoder MUST read it back.
    let img = parse_tga(&bytes).expect("encoder produced a file parse_tga rejects");
    assert_eq!(img.width, w as u32, "width");
    assert_eq!(img.height, h as u32, "height");
    assert_eq!(img.pixel_format, writer.decoded_format(), "pixel format");

    let expected = match (bpp, writer.forces_opaque()) {
        // Gray8: exact copy.
        (1, _) => pixels.clone(),
        // RGB input: widen to opaque RGBA.
        (3, _) => widen_rgb_to_rgba(&pixels),
        // RGBA input, alpha-forced writer (Bits24 colour map): opaque RGBA.
        (4, true) => force_opaque_rgba(&pixels),
        // RGBA input, lossless writer: exact copy.
        (4, false) => pixels.clone(),
        _ => unreachable!(),
    };
    assert_eq!(img.data, expected, "decoded raster != expected identity");
});
