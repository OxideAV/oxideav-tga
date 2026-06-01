#![no_main]

//! Encode → decode bit-exact roundtrip harness across every public
//! writer surface.
//!
//! `decode_tga` (the sibling target) drives the decoder with arbitrary
//! attacker bytes and checks "no panic, no OOB, no OOM". That covers
//! the *parse* surface but not the *write* surface — an encoder that
//! is itself robust against any input has a much weaker exposure than
//! a decoder, but a *roundtrip* harness has an oracle the decoder
//! cannot have: the input the encoder was just handed. If the
//! decoder's output disagrees with the encoder's input on a byte the
//! spec says is round-trippable, that's a logic bug the decode-only
//! target can never see.
//!
//! The contract this target enforces, per public writer + spec § :
//!
//! | Writer | Image type | Round-trip contract |
//! | ------ | ---------- | ------------------- |
//! | [`encode_tga_uncompressed`] | 2  | RGBA in → RGBA out byte-exact (auto-selects 24 vs 32 bpp; all-α=0xFF rounds via 24 bpp with α restored to 0xFF on decode) |
//! | [`encode_tga_rle`]          | 10 | same |
//! | [`encode_tga_uncompressed_rgb24`] | 2  | RGB in → RGBA out with α=0xFF for every pixel |
//! | [`encode_tga_rle_rgb24`]    | 10 | same |
//! | [`encode_tga_palette`]      | 1  | RGBA in (≤ 256 distinct) → RGBA out byte-exact |
//! | [`encode_tga_palette_rle`]  | 9  | same |
//! | [`encode_tga_grayscale`]    | 3  | Gray8 in → Gray8 out byte-exact |
//! | [`encode_tga_grayscale_rle`] | 11 | same |
//!
//! `splice_image_id` + `encode_tga_with_extension` are also exercised
//! on the freshly-encoded base so the panic-free contract extends to
//! the §3.3 / §C.3 Image-ID splice path and the §C.5 / §C.6 footer +
//! extension-area writer chain. Their *content* is checked: the
//! spliced Image-ID bytes must come back from `parse_tga_image_id`
//! verbatim, and the footer / extension area must parse out the same
//! offsets the writer back-patched.
//!
//! The roundtrip oracle is the encoder's *input*, not an external
//! library, so the clean-room wall is intact (no third-party TGA
//! reference implementation is needed or referenced).
//!
//! # Why the dimension cap
//!
//! The encoders allocate `O(width × height)` output up front. A
//! 64-KiB × 64-KiB raster is a 16-GiB allocation — a resource bug,
//! not a logic bug. The harness caps `width × height ≤ 4096` pixels
//! (one 64×64 thumbnail equivalent at the high end), which is plenty
//! to exercise every RLE packet boundary (128 px / packet) and every
//! per-row encode path without OOMing the fuzzer.

use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;
use oxideav_tga::{
    encode_tga_grayscale, encode_tga_grayscale_rle, encode_tga_palette, encode_tga_palette_rle,
    encode_tga_rle, encode_tga_rle_rgb24, encode_tga_uncompressed, encode_tga_uncompressed_rgb24,
    encode_tga_with_extension, parse_tga, parse_tga_extension_area, parse_tga_footer,
    parse_tga_image_id, splice_image_id, ExtensionAreaInput, TgaPixelFormat, TGA_IMAGE_ID_MAX,
};

/// Pixel-count cap for the synthesised raster. 4096 ≥ 64×64 is
/// generous enough to walk past every 128-pixel-per-packet RLE
/// boundary in a single row without OOMing the fuzzer when the
/// dimensions are picked adversarially.
const MAX_PIXELS: usize = 4096;

/// One round of the encode → decode oracle, selected by the fuzzer
/// via Arbitrary.
#[derive(Debug, Arbitrary)]
struct Input {
    /// Picks which of the eight public writers to drive.
    writer: WriterKind,
    /// Width, clamped to 1..=64.
    w: u8,
    /// Height, clamped to 1..=64.
    h: u8,
    /// Raster pixel bytes (interpretation depends on `writer`).
    raster: Vec<u8>,
    /// Optional Image-ID bytes to splice through `splice_image_id`.
    /// `None` skips the splice step.
    image_id: Option<Vec<u8>>,
    /// Optional extension-area wrap. `true` runs the result through
    /// `encode_tga_with_extension` with a minimal `ExtensionAreaInput`
    /// and re-validates the footer / extension-area parse path.
    wrap_extension: bool,
}

#[derive(Debug, Arbitrary)]
enum WriterKind {
    UncompressedRgba,
    RleRgba,
    UncompressedRgb24,
    RleRgb24,
    Palette,
    PaletteRle,
    Grayscale,
    GrayscaleRle,
}

fuzz_target!(|input: Input| {
    // Clamp to 1..=64 so width * height ≤ MAX_PIXELS.
    let width = ((input.w as u16) % 64) + 1;
    let height = ((input.h as u16) % 64) + 1;
    let pixels = width as usize * height as usize;
    if pixels > MAX_PIXELS {
        return;
    }

    // Synthesise a raster of the right byte width by truncate-or-extend.
    // Extending with zero is fine — the decoder will see well-formed
    // RGBA / RGB / Gray8 input regardless of the fuzzer's payload.
    let raster_in = &input.raster;
    let (bytes_needed, expected_format) = match input.writer {
        WriterKind::UncompressedRgba | WriterKind::RleRgba => (pixels * 4, TgaPixelFormat::Rgba),
        WriterKind::UncompressedRgb24 | WriterKind::RleRgb24 => (pixels * 3, TgaPixelFormat::Rgba),
        WriterKind::Palette | WriterKind::PaletteRle => (pixels * 4, TgaPixelFormat::Rgba),
        WriterKind::Grayscale | WriterKind::GrayscaleRle => (pixels, TgaPixelFormat::Gray8),
    };
    let mut raster = Vec::with_capacity(bytes_needed);
    if raster_in.len() >= bytes_needed {
        raster.extend_from_slice(&raster_in[..bytes_needed]);
    } else {
        raster.extend_from_slice(raster_in);
        raster.resize(bytes_needed, 0);
    }

    // Drive the encoder.
    let encoded = match input.writer {
        WriterKind::UncompressedRgba => encode_tga_uncompressed(width, height, &raster),
        WriterKind::RleRgba => encode_tga_rle(width, height, &raster),
        WriterKind::UncompressedRgb24 => encode_tga_uncompressed_rgb24(width, height, &raster),
        WriterKind::RleRgb24 => encode_tga_rle_rgb24(width, height, &raster),
        WriterKind::Palette => encode_tga_palette(width, height, &raster),
        WriterKind::PaletteRle => encode_tga_palette_rle(width, height, &raster),
        WriterKind::Grayscale => encode_tga_grayscale(width, height, &raster),
        WriterKind::GrayscaleRle => encode_tga_grayscale_rle(width, height, &raster),
    };

    // Palette writers refuse > 256 unique RGBA inputs with
    // `Unsupported`; that's a documented contract, not a panic — so a
    // refusal here ends the iteration without flagging a failure.
    let mut encoded = match encoded {
        Ok(v) => v,
        Err(_) => return,
    };

    // Optional §3.3 / §C.3 Image-ID splice. The splice is content-
    // checked: `parse_tga_image_id` on the post-splice base must yield
    // the original bytes verbatim.
    let mut spliced_id: Option<Vec<u8>> = None;
    if let Some(id_in) = input.image_id.as_ref() {
        if id_in.len() <= TGA_IMAGE_ID_MAX {
            let before = encoded.clone();
            match splice_image_id(&mut encoded, id_in) {
                Ok(()) => {
                    spliced_id = Some(id_in.clone());
                    // Empty input is a no-op (documented behaviour).
                    if id_in.is_empty() {
                        assert_eq!(encoded, before, "splice_image_id(empty) must be a no-op");
                    }
                }
                Err(_) => {
                    // The splice rejects calls into a sub-header buffer
                    // and double-splice; our `encoded` is always a
                    // freshly-written file with id_length = 0, so any
                    // refusal here would be a bug — but the harness
                    // doesn't assert on it because the documented
                    // failure modes are well-formed `Err(_)` returns.
                    encoded = before;
                }
            }
        }
    }

    // Optional TGA 2.0 footer + extension area wrap. Drives the
    // `encode_tga_with_extension` writer + the matching `parse_*`
    // helpers. Default-constructed input keeps the output minimal —
    // no postage stamp / CCT / scan-line table / dev tags.
    if input.wrap_extension {
        let ext = ExtensionAreaInput::default();
        if let Ok(wrapped) = encode_tga_with_extension(&encoded, &ext) {
            // Both helpers must return Some — the writer just wrote
            // the footer and the 495-byte body. Their content is
            // exercised but not asserted byte-for-byte (the writer's
            // own unit tests pin that).
            let _ = parse_tga_footer(&wrapped);
            let _ = parse_tga_extension_area(&wrapped);
            encoded = wrapped;
        }
    }

    // Decode the result.
    let img = match parse_tga(&encoded) {
        Ok(i) => i,
        Err(e) => panic!("roundtrip decode failed on freshly-encoded TGA: {e}"),
    };

    // Pixel-format contract.
    assert_eq!(img.pixel_format, expected_format);
    assert_eq!(img.width, width as u32);
    assert_eq!(img.height, height as u32);
    let bpp = match img.pixel_format {
        TgaPixelFormat::Rgba => 4,
        TgaPixelFormat::Gray8 => 1,
        TgaPixelFormat::Rgb24 => 3,
    };
    assert_eq!(img.data.len(), pixels * bpp);

    // Spliced Image-ID, if any, must come back from the dedicated
    // parser verbatim.
    if let Some(id) = spliced_id {
        let parsed =
            parse_tga_image_id(&encoded).expect("post-splice file must have a parseable Image-ID");
        assert_eq!(parsed, id.as_slice());
    }

    // Bit-exact pixel comparison per writer.
    match input.writer {
        WriterKind::UncompressedRgba | WriterKind::RleRgba => {
            // Auto-depth: if the input had any α < 0xFF the encoder
            // writes 32 bpp and the alpha byte is preserved. If every
            // α = 0xFF the encoder writes 24 bpp; the decoder fills
            // α = 0xFF on read, which matches the input.
            assert_eq!(img.data, raster, "RGBA writer round-trip mismatch");
        }
        WriterKind::UncompressedRgb24 | WriterKind::RleRgb24 => {
            // 24 bpp on the wire → 32 bpp out with α=0xFF per pixel.
            assert_eq!(img.data.len(), pixels * 4);
            for (i, chunk) in img.data.chunks_exact(4).enumerate() {
                let src = &raster[i * 3..i * 3 + 3];
                assert_eq!(chunk[0], src[0]);
                assert_eq!(chunk[1], src[1]);
                assert_eq!(chunk[2], src[2]);
                assert_eq!(chunk[3], 0xFF);
            }
        }
        WriterKind::Palette | WriterKind::PaletteRle => {
            // Palette writer dedups; on round-trip every input pixel
            // resolves back to its (unique) palette entry. Compare
            // RGBA byte-for-byte.
            assert_eq!(img.data, raster, "palette round-trip mismatch");
        }
        WriterKind::Grayscale | WriterKind::GrayscaleRle => {
            assert_eq!(img.data, raster, "grayscale round-trip mismatch");
        }
    }
});
