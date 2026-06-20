#![no_main]

//! Encode-then-decode round-trip target for every public TGA writer.
//!
//! The decoder-only [`decode_tga`](decode_tga.rs) target drives
//! attacker-controlled bytes through [`parse_tga`] and friends; the
//! contract there is purely panic-free. This target attacks the
//! complement: take arbitrary fuzz bytes, divide them into a small
//! header (writer selector + width / height + pixel-data tail), feed
//! that pixel data through one of the eight standalone encoders
//! (`encode_tga_uncompressed`, `encode_tga_uncompressed_rgb24`,
//! `encode_tga_rle`, `encode_tga_rle_rgb24`, `encode_tga_grayscale`,
//! `encode_tga_grayscale_rle`, `encode_tga_palette`,
//! `encode_tga_palette_rle`), and verify that:
//!
//! 1. The encoder never panics, regardless of input shape. Length
//!    mismatches / palette-overflow inputs / oversized rasters are
//!    expected to return `Err(TgaError::…)` — that's a documented
//!    failure mode, not a fuzz finding.
//! 2. When the encoder returns `Ok(bytes)`, those bytes parse back
//!    through [`parse_tga`] without panicking. The decoder may
//!    legitimately return `Err` (e.g. a bug we haven't found yet) but
//!    must never abort or overflow.
//! 3. On a successful decode, the round-tripped image's `width` /
//!    `height` match the requested dimensions and the payload length
//!    matches `width × height × bytes_per_pixel(pixel_format)`. Pixel
//!    content equality is *not* asserted at this layer — the
//!    RGBA→24/32 depth auto-selection drops alpha when every input
//!    pixel is opaque, palette encoders re-index and so re-order
//!    colours, and RGB24-input writers always emit 24 bpp regardless
//!    of alpha — all of which are documented behaviours of those
//!    writers. Frame-shape integrity is the strongest assertion
//!    that's valid across every writer.
//!
//! # Raster cap
//!
//! Same logic as `decode_tga`: a hostile fuzz input could declare
//! `width = height = 65535`, which is a legal but ~16 GiB allocation
//! that would mask the real logic bugs we're hunting for. The
//! harness clamps `width × height` to 16 MiB before driving any
//! encoder, mirroring what a real demuxer's sanity limits would do.
//! The library itself keeps no policy cap; the cap lives in the
//! harness only.
//!
//! # Why no encoder-side oracle
//!
//! The same clean-room wall that bars libtga / image-rs's tga
//! submodule / ffmpeg's targa.c as a decode oracle in `decode_tga`
//! applies here. We only have ourselves; the strongest assertion we
//! can make without an oracle is "encode-then-decode preserves frame
//! shape," and that's what this target enforces.

use libfuzzer_sys::fuzz_target;
use oxideav_tga::{
    encode_tga_grayscale, encode_tga_grayscale_rle, encode_tga_palette, encode_tga_palette_rle,
    encode_tga_palette_with_entry_size, encode_tga_rle, encode_tga_rle_rgb24,
    encode_tga_uncompressed, encode_tga_uncompressed_rgb24, parse_tga, ColorMapEntrySize, ImageType,
    TgaPixelFormat,
};

/// Upper bound on the declared raster (16 MiB worth of RGBA = 4 M
/// pixels). Anything larger is a resource request, not a logic path.
const MAX_PIXELS: usize = 4 * 1024 * 1024;

/// Per-writer maximum side length. The TGA on-disk header carries
/// `width` and `height` as `u16` so the format max is 65535; the
/// product cap above is the actual constraint, so the side cap only
/// needs to keep small inputs from skewing too far towards
/// degenerate 1-pixel-wide rasters.
const MAX_SIDE: u16 = 4096;

/// Encoder selectors, one per public writer entry point. The last four
/// drive `encode_tga_palette_with_entry_size` so the explicit
/// 16 / 24-bit colour-map writer paths (image types 1 and 9) get the
/// same encode-then-decode frame-shape oracle. (15- and 32-bit are
/// shape-equivalent to 16- and the default writers respectively; the two
/// here exercise the 2-byte-packed and 3-byte-packed colour-map layouts
/// the default writers don't always pick.)
#[derive(Clone, Copy)]
enum Writer {
    UncompressedRgba,
    UncompressedRgb24,
    RleRgba,
    RleRgb24,
    Grayscale,
    GrayscaleRle,
    Palette,
    PaletteRle,
    PaletteEntry16Type1,
    PaletteEntry24Type1,
    PaletteEntry16Type9,
    PaletteEntry24Type9,
}

impl Writer {
    fn from_byte(b: u8) -> Self {
        match b % 12 {
            0 => Writer::UncompressedRgba,
            1 => Writer::UncompressedRgb24,
            2 => Writer::RleRgba,
            3 => Writer::RleRgb24,
            4 => Writer::Grayscale,
            5 => Writer::GrayscaleRle,
            6 => Writer::Palette,
            7 => Writer::PaletteRle,
            8 => Writer::PaletteEntry16Type1,
            9 => Writer::PaletteEntry24Type1,
            10 => Writer::PaletteEntry16Type9,
            _ => Writer::PaletteEntry24Type9,
        }
    }

    /// Bytes per pixel the writer wants on input.
    fn input_bpp(self) -> usize {
        match self {
            Writer::UncompressedRgba
            | Writer::RleRgba
            | Writer::Palette
            | Writer::PaletteRle
            | Writer::PaletteEntry16Type1
            | Writer::PaletteEntry24Type1
            | Writer::PaletteEntry16Type9
            | Writer::PaletteEntry24Type9 => 4,
            Writer::UncompressedRgb24 | Writer::RleRgb24 => 3,
            Writer::Grayscale | Writer::GrayscaleRle => 1,
        }
    }

    /// Pixel format the decoder will produce after a successful
    /// round-trip. Type 1 / 9 (palette) and type 2 / 10 (true-colour)
    /// both decode to RGBA; type 3 / 11 (grayscale) decode to Gray8.
    fn decoded_format(self) -> TgaPixelFormat {
        match self {
            Writer::Grayscale | Writer::GrayscaleRle => TgaPixelFormat::Gray8,
            _ => TgaPixelFormat::Rgba,
        }
    }

    fn encode(self, w: u16, h: u16, pixels: &[u8]) -> oxideav_tga::Result<Vec<u8>> {
        match self {
            Writer::UncompressedRgba => encode_tga_uncompressed(w, h, pixels),
            Writer::UncompressedRgb24 => encode_tga_uncompressed_rgb24(w, h, pixels),
            Writer::RleRgba => encode_tga_rle(w, h, pixels),
            Writer::RleRgb24 => encode_tga_rle_rgb24(w, h, pixels),
            Writer::Grayscale => encode_tga_grayscale(w, h, pixels),
            Writer::GrayscaleRle => encode_tga_grayscale_rle(w, h, pixels),
            Writer::Palette => encode_tga_palette(w, h, pixels),
            Writer::PaletteRle => encode_tga_palette_rle(w, h, pixels),
            Writer::PaletteEntry16Type1 => encode_tga_palette_with_entry_size(
                w,
                h,
                pixels,
                ImageType::UncompressedColourMapped,
                ColorMapEntrySize::Bits16,
            ),
            Writer::PaletteEntry24Type1 => encode_tga_palette_with_entry_size(
                w,
                h,
                pixels,
                ImageType::UncompressedColourMapped,
                ColorMapEntrySize::Bits24,
            ),
            Writer::PaletteEntry16Type9 => encode_tga_palette_with_entry_size(
                w,
                h,
                pixels,
                ImageType::RleColourMapped,
                ColorMapEntrySize::Bits16,
            ),
            Writer::PaletteEntry24Type9 => encode_tga_palette_with_entry_size(
                w,
                h,
                pixels,
                ImageType::RleColourMapped,
                ColorMapEntrySize::Bits24,
            ),
        }
    }
}

fuzz_target!(|data: &[u8]| {
    // Minimum useful input: 1 selector byte + 4 dimension bytes + at
    // least 1 pixel byte.
    if data.len() < 6 {
        return;
    }

    let writer = Writer::from_byte(data[0]);
    // Two u16 dimensions, taken from the next four bytes, clamped to
    // `MAX_SIDE`. `max(1)` keeps the encoders out of zero-pixel
    // branches the standalone API already rejects with `Err`.
    let raw_w = u16::from_le_bytes([data[1], data[2]]);
    let raw_h = u16::from_le_bytes([data[3], data[4]]);
    let w = (raw_w % MAX_SIDE).max(1);
    let h = (raw_h % MAX_SIDE).max(1);

    let bpp = writer.input_bpp();
    let need_pixels = w as usize * h as usize;
    // Clamp the raster so the encoder doesn't have to allocate >
    // 16 MiB of pixel buffer.
    if need_pixels > MAX_PIXELS {
        return;
    }
    let need_bytes = need_pixels * bpp;

    // Tile the fuzz tail to fill the input buffer. Tiling (vs
    // zero-padding) keeps the RLE encoder honest: a zero-padded
    // input would collapse to one giant run packet and skip the
    // raw-packet code path. The tail starts after the 5 header
    // bytes; we already know `data.len() >= 6` so the tail has at
    // least one byte.
    let tail = &data[5..];
    let mut pixels = Vec::with_capacity(need_bytes);
    while pixels.len() < need_bytes {
        let take = (need_bytes - pixels.len()).min(tail.len());
        pixels.extend_from_slice(&tail[..take]);
    }

    let bytes = match writer.encode(w, h, &pixels) {
        Ok(b) => b,
        Err(_) => return,
    };

    // Decode the freshly-encoded file. The decoder is allowed to
    // return `Err` here only if there's a real producer bug — but in
    // either case it must not panic, abort, or overflow.
    let img = match parse_tga(&bytes) {
        Ok(img) => img,
        Err(_) => return,
    };

    // Frame shape must round-trip exactly: dimensions match what we
    // asked for, payload length matches dimensions × bpp.
    assert_eq!(img.width, w as u32, "round-trip width mismatch");
    assert_eq!(img.height, h as u32, "round-trip height mismatch");
    assert_eq!(
        img.pixel_format,
        writer.decoded_format(),
        "round-trip pixel format mismatch"
    );
    let expected_len = img.width as usize * img.height as usize * img.bytes_per_pixel();
    assert_eq!(
        img.data.len(),
        expected_len,
        "round-trip payload length doesn't match w×h×bpp"
    );
});
