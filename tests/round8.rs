//! Round 8: Image Identification Field round-trip (spec §3.3 / §C.3).
//!
//! The Image ID is a free-form, up-to-255-byte block written immediately
//! after the 18-byte header. Its length is the `id_length` byte at header
//! offset 0; content is unconstrained ("free-form" per spec, typically a
//! printable ASCII string identifying the source camera / scene / artist).
//!
//! Until this round the writer hard-coded `id_length = 0` (no Image ID)
//! and the decoder silently stepped over whatever bytes the header
//! declared, never surfacing them. Both halves now have a public route:
//!
//! * [`oxideav_tga::splice_image_id`] inserts an ID into a freshly-encoded
//!   base TGA (writing byte 0 and shifting the colour-map + pixel + footer
//!   payload by `image_id.len()` bytes).
//! * [`oxideav_tga::parse_tga_image_id`] borrows the ID slice straight out
//!   of an input buffer.
//!
//! This file pins:
//!
//! * Round-trip across every base writer (true-colour 24/32, RLE, palette
//!   uncompressed/RLE, grayscale uncompressed/RLE) — every produced file
//!   still decodes pixel-exact through `parse_tga`, and the spliced ID
//!   round-trips byte-for-byte through `parse_tga_image_id`.
//! * Interaction with `encode_tga_with_extension`: an Image ID spliced
//!   *before* extension wrapping survives because the footer is appended
//!   at the tail; the footer / postage-stamp / colour-correction-table /
//!   scan-line-table / developer-area offsets back-patched by the
//!   extension writer all land on the post-splice file's bytes.
//! * Edge cases: empty ID is a no-op, 255-byte ID is the maximum spec
//!   length, > 255 returns [`oxideav_tga::TgaError::Invalid`], and a
//!   double splice (trying to overwrite an existing ID) is rejected.
//! * Decoder: a file truncated mid-Image-ID returns `None` from
//!   `parse_tga_image_id` (panic-free contract — same as every other
//!   `parse_tga_*` helper).

use oxideav_tga::{
    encode_tga_grayscale, encode_tga_grayscale_rle, encode_tga_palette, encode_tga_palette_rle,
    encode_tga_rle, encode_tga_rle_rgb24, encode_tga_uncompressed, encode_tga_uncompressed_rgb24,
    encode_tga_with_extension, parse_tga, parse_tga_extension_area, parse_tga_footer,
    parse_tga_image_id, parse_tga_postage_stamp, splice_image_id, ExtensionAreaInput, TgaError,
    TgaImage, TgaPixelFormat, TGA_IMAGE_ID_MAX,
};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn rgba_diag(width: u32, height: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let on_diag = x == y;
            let r = (x as u8).wrapping_mul(13);
            let g = (y as u8).wrapping_mul(17);
            let b = if on_diag { 0xFF } else { 0x10 };
            let a = if on_diag { 0xC0 } else { 0xFF };
            buf.extend_from_slice(&[r, g, b, a]);
        }
    }
    buf
}

fn rgb24_ramp(width: u32, height: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            buf.extend_from_slice(&[
                (x as u8).wrapping_mul(7),
                (y as u8).wrapping_mul(11),
                (x.wrapping_add(y) as u8).wrapping_mul(19),
            ]);
        }
    }
    buf
}

fn gray_ramp(width: u32, height: u32) -> Vec<u8> {
    (0..(width * height) as usize)
        .map(|i| (i as u8).wrapping_mul(23))
        .collect()
}

fn palette_pixels(width: u32, height: u32) -> Vec<u8> {
    // Eight unique colours, picked so the palette stays well below 256.
    let palette: [[u8; 4]; 8] = [
        [0x00, 0x00, 0x00, 0xFF],
        [0xFF, 0x00, 0x00, 0xFF],
        [0x00, 0xFF, 0x00, 0xFF],
        [0x00, 0x00, 0xFF, 0xFF],
        [0xFF, 0xFF, 0x00, 0xFF],
        [0xFF, 0x00, 0xFF, 0xFF],
        [0x00, 0xFF, 0xFF, 0xFF],
        [0xFF, 0xFF, 0xFF, 0xFF],
    ];
    let mut buf = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let i = ((x + y) as usize) & 7;
            buf.extend_from_slice(&palette[i]);
        }
    }
    buf
}

// ---------------------------------------------------------------------------
// Round-trip across every base writer
// ---------------------------------------------------------------------------

const SAMPLE_ID: &[u8] = b"oxideav-tga round8 sample image";

fn assert_image_id_round_trip(bytes: &[u8], expect: &[u8]) {
    let id = parse_tga_image_id(bytes).expect("image-id slice should be readable");
    assert_eq!(id, expect, "image-id bytes round-trip exactly");
}

#[test]
fn image_id_roundtrip_uncompressed_truecolor() {
    let rgba = rgba_diag(8, 4);
    let mut base = encode_tga_uncompressed(8, 4, &rgba).unwrap();
    splice_image_id(&mut base, SAMPLE_ID).unwrap();
    assert_image_id_round_trip(&base, SAMPLE_ID);
    let img = parse_tga(&base).unwrap();
    assert_eq!(img.width, 8);
    assert_eq!(img.height, 4);
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    assert_eq!(img.data, rgba);
}

#[test]
fn image_id_roundtrip_uncompressed_rgb24() {
    let rgb = rgb24_ramp(6, 3);
    let mut base = encode_tga_uncompressed_rgb24(6, 3, &rgb).unwrap();
    splice_image_id(&mut base, SAMPLE_ID).unwrap();
    assert_image_id_round_trip(&base, SAMPLE_ID);
    let img = parse_tga(&base).unwrap();
    assert_eq!(img.width, 6);
    assert_eq!(img.height, 3);
    // RGB24 input is decoded as RGBA (alpha forced to 0xFF).
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    for (i, px) in img.data.chunks_exact(4).enumerate() {
        let src = &rgb[i * 3..i * 3 + 3];
        assert_eq!(&px[..3], src);
        assert_eq!(px[3], 0xFF);
    }
}

#[test]
fn image_id_roundtrip_rle_truecolor() {
    let rgba = rgba_diag(8, 4);
    let mut base = encode_tga_rle(8, 4, &rgba).unwrap();
    splice_image_id(&mut base, SAMPLE_ID).unwrap();
    assert_image_id_round_trip(&base, SAMPLE_ID);
    let img = parse_tga(&base).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn image_id_roundtrip_rle_rgb24() {
    let rgb = rgb24_ramp(6, 3);
    let mut base = encode_tga_rle_rgb24(6, 3, &rgb).unwrap();
    splice_image_id(&mut base, SAMPLE_ID).unwrap();
    assert_image_id_round_trip(&base, SAMPLE_ID);
    let img = parse_tga(&base).unwrap();
    for (i, px) in img.data.chunks_exact(4).enumerate() {
        let src = &rgb[i * 3..i * 3 + 3];
        assert_eq!(&px[..3], src);
        assert_eq!(px[3], 0xFF);
    }
}

#[test]
fn image_id_roundtrip_palette_uncompressed() {
    let rgba = palette_pixels(8, 4);
    let mut base = encode_tga_palette(8, 4, &rgba).unwrap();
    splice_image_id(&mut base, SAMPLE_ID).unwrap();
    assert_image_id_round_trip(&base, SAMPLE_ID);
    let img = parse_tga(&base).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn image_id_roundtrip_palette_rle() {
    let rgba = palette_pixels(8, 4);
    let mut base = encode_tga_palette_rle(8, 4, &rgba).unwrap();
    splice_image_id(&mut base, SAMPLE_ID).unwrap();
    assert_image_id_round_trip(&base, SAMPLE_ID);
    let img = parse_tga(&base).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn image_id_roundtrip_grayscale_uncompressed() {
    let gray = gray_ramp(10, 2);
    let mut base = encode_tga_grayscale(10, 2, &gray).unwrap();
    splice_image_id(&mut base, SAMPLE_ID).unwrap();
    assert_image_id_round_trip(&base, SAMPLE_ID);
    let img = parse_tga(&base).unwrap();
    assert_eq!(img.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!(img.data, gray);
}

#[test]
fn image_id_roundtrip_grayscale_rle() {
    let gray = gray_ramp(10, 2);
    let mut base = encode_tga_grayscale_rle(10, 2, &gray).unwrap();
    splice_image_id(&mut base, SAMPLE_ID).unwrap();
    assert_image_id_round_trip(&base, SAMPLE_ID);
    let img = parse_tga(&base).unwrap();
    assert_eq!(img.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!(img.data, gray);
}

// ---------------------------------------------------------------------------
// Splice composes cleanly with the TGA 2.0 footer / extension area writer
// ---------------------------------------------------------------------------

#[test]
fn splice_then_extension_area_preserves_both() {
    let rgba = rgba_diag(8, 4);
    let mut base = encode_tga_rle(8, 4, &rgba).unwrap();
    splice_image_id(&mut base, SAMPLE_ID).unwrap();

    let ext_input = ExtensionAreaInput {
        author_name: "oxideav-tga".to_string(),
        software_id: "round8".to_string(),
        software_version: (100, 'a'),
        attributes_type: 3,
        postage_stamp: Some(TgaImage {
            width: 4,
            height: 2,
            pixel_format: TgaPixelFormat::Rgba,
            data: rgba_diag(4, 2),
            pts: None,
        }),
        ..Default::default()
    };
    let full = encode_tga_with_extension(&base, &ext_input).unwrap();

    // Image ID still parses.
    assert_image_id_round_trip(&full, SAMPLE_ID);
    // Footer + extension area still parse and point at consistent offsets.
    let footer = parse_tga_footer(&full).expect("footer present");
    assert_ne!(footer.extension_area_offset, 0);
    let ext = parse_tga_extension_area(&full).expect("extension area present");
    assert_eq!(ext.author_name, "oxideav-tga");
    assert_eq!(ext.software_id, "round8");
    assert_eq!(ext.software_version, (100, 'a'));
    assert_ne!(ext.postage_stamp_offset, 0);
    // Postage stamp decodes pixel-exact.
    let stamp = parse_tga_postage_stamp(&full).unwrap().expect("stamp");
    assert_eq!(stamp.width, 4);
    assert_eq!(stamp.height, 2);
    assert_eq!(stamp.data, rgba_diag(4, 2));
    // Main image still decodes pixel-exact.
    let img = parse_tga(&full).unwrap();
    assert_eq!(img.data, rgba);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_image_id_is_noop() {
    let rgba = rgba_diag(2, 2);
    let base_pristine = encode_tga_uncompressed(2, 2, &rgba).unwrap();
    let mut base = base_pristine.clone();
    splice_image_id(&mut base, &[]).unwrap();
    // No bytes added, byte 0 still zero.
    assert_eq!(base, base_pristine);
    assert_eq!(base[0], 0);
    // parse_tga_image_id returns an empty slice for id_length == 0.
    let id = parse_tga_image_id(&base).expect("zero-length slice should be Some(&[])");
    assert!(id.is_empty());
}

#[test]
fn maximum_length_image_id_is_accepted() {
    let rgba = rgba_diag(2, 2);
    let mut base = encode_tga_uncompressed(2, 2, &rgba).unwrap();
    let big = vec![0xAAu8; TGA_IMAGE_ID_MAX];
    splice_image_id(&mut base, &big).unwrap();
    assert_eq!(base[0], 255);
    let id = parse_tga_image_id(&base).unwrap();
    assert_eq!(id.len(), TGA_IMAGE_ID_MAX);
    assert!(id.iter().all(|&b| b == 0xAA));
    let img = parse_tga(&base).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn oversized_image_id_rejected() {
    let rgba = rgba_diag(2, 2);
    let mut base = encode_tga_uncompressed(2, 2, &rgba).unwrap();
    let too_big = vec![0u8; TGA_IMAGE_ID_MAX + 1];
    let err = splice_image_id(&mut base, &too_big).unwrap_err();
    match err {
        TgaError::InvalidData(_) => {}
        other => panic!("expected Invalid, got {:?}", other),
    }
    // Original file unchanged.
    assert_eq!(base[0], 0);
}

#[test]
fn double_splice_rejected() {
    let rgba = rgba_diag(2, 2);
    let mut base = encode_tga_uncompressed(2, 2, &rgba).unwrap();
    splice_image_id(&mut base, SAMPLE_ID).unwrap();
    let err = splice_image_id(&mut base, b"different").unwrap_err();
    match err {
        TgaError::InvalidData(_) => {}
        other => panic!("expected Invalid, got {:?}", other),
    }
    // First splice is preserved.
    assert_image_id_round_trip(&base, SAMPLE_ID);
}

#[test]
fn splice_into_short_buffer_rejected() {
    let mut tiny = vec![0u8; 5];
    let err = splice_image_id(&mut tiny, SAMPLE_ID).unwrap_err();
    match err {
        TgaError::InvalidData(_) => {}
        other => panic!("expected Invalid, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Decoder is panic-free for hostile inputs (matches every other parser)
// ---------------------------------------------------------------------------

#[test]
fn parse_tga_image_id_returns_none_on_short_buffer() {
    assert!(parse_tga_image_id(&[]).is_none());
    assert!(parse_tga_image_id(&[0u8; 5]).is_none());
    // Header declares id_length=200 but only 18 bytes total.
    let mut buf = vec![0u8; 18];
    buf[0] = 200;
    assert!(parse_tga_image_id(&buf).is_none());
}

#[test]
fn parse_tga_image_id_handles_zero_length() {
    let rgba = rgba_diag(2, 2);
    let base = encode_tga_uncompressed(2, 2, &rgba).unwrap();
    let id = parse_tga_image_id(&base).unwrap();
    assert!(id.is_empty());
}

#[test]
fn parse_tga_image_id_arbitrary_bytes_panic_free() {
    // Mimics the panic-free contract every public parser obeys.
    for n in 0..=64u16 {
        let mut buf = vec![0u8; n as usize];
        let _ = parse_tga_image_id(&buf);
        if buf.len() >= 18 {
            buf[0] = 250;
            let _ = parse_tga_image_id(&buf);
        }
    }
}
