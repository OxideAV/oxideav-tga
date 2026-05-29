//! Self-roundtrip on every (image type, depth) combination round 1
//! ships writers for, and parse-only smoke tests for the read-only
//! types (1 / 3 / 9 / 11 — write paths for those land in round 2).
//!
//! Each test builds a hand-crafted byte stream byte-by-byte from the
//! TGA 2.0 spec rather than re-using the encoder for both sides; that
//! way a bug in the encoder can't mask the same bug in the decoder.

use oxideav_tga::{
    encode_tga_rle, encode_tga_uncompressed, parse_tga, parse_tga_footer, TgaPixelFormat,
};

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

fn checker_rgba(w: u16, h: u16) -> Vec<u8> {
    // 4-colour checker: red / green / blue / white-half-alpha. Each
    // 2×2 block tiles. Hits every primary + an alpha-bearing pixel.
    let palette: [[u8; 4]; 4] = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 200],
        [255, 255, 255, 128],
    ];
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let q = (x & 1) + 2 * (y & 1);
            data.extend_from_slice(&palette[q]);
        }
    }
    data
}

fn opaque_checker_rgba(w: u16, h: u16) -> Vec<u8> {
    // 24-bpp variant: all alpha = 0xFF so the encoder picks 24-bit
    // depth.
    let palette: [[u8; 4]; 4] = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 255],
    ];
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let q = (x & 1) + 2 * (y & 1);
            data.extend_from_slice(&palette[q]);
        }
    }
    data
}

// ---------------------------------------------------------------------------
// Type 2 (uncompressed true-colour) self-roundtrip — 24 bpp + 32 bpp.
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_type2_24bpp_uncompressed() {
    let rgba = opaque_checker_rgba(8, 6);
    let bytes = encode_tga_uncompressed(8, 6, &rgba).unwrap();
    // Header byte 2 = image type, byte 16 = depth.
    assert_eq!(bytes[2], 2, "image type");
    assert_eq!(bytes[16], 24, "depth (no alpha → 24bpp)");
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, 8);
    assert_eq!(img.height, 6);
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    assert_eq!(img.data, rgba);
}

#[test]
fn roundtrip_type2_32bpp_uncompressed() {
    let rgba = checker_rgba(8, 6);
    let bytes = encode_tga_uncompressed(8, 6, &rgba).unwrap();
    assert_eq!(bytes[2], 2);
    assert_eq!(bytes[16], 32, "depth (alpha present → 32bpp)");
    assert_eq!(bytes[17] & 0x0F, 8, "alpha bits");
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
}

// ---------------------------------------------------------------------------
// Type 10 (RLE true-colour) self-roundtrip — 24 bpp + 32 bpp.
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_type10_24bpp_rle() {
    let rgba = opaque_checker_rgba(16, 8);
    let bytes = encode_tga_rle(16, 8, &rgba).unwrap();
    assert_eq!(bytes[2], 10);
    assert_eq!(bytes[16], 24);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba, "RLE roundtrip");
    // 2×2 checker has no per-row runs ≥ 2 so RLE doesn't help — but a
    // banded fixture should compress easily. Verify the encoder picks
    // runs when they're available.
    let mut banded = Vec::with_capacity(16 * 8 * 4);
    for y in 0..8 {
        for _x in 0..16 {
            // Each row is solid colour (16 identical pixels) → RLE
            // emits one packet per row.
            banded.extend_from_slice(&[y * 16, 0, 255 - y * 16, 255]);
        }
    }
    let raw = encode_tga_uncompressed(16, 8, &banded).unwrap();
    let rle = encode_tga_rle(16, 8, &banded).unwrap();
    assert!(
        rle.len() < raw.len(),
        "RLE expected to be smaller than raw on banded fixture (rle={}, raw={})",
        rle.len(),
        raw.len()
    );
    let back = parse_tga(&rle).unwrap();
    assert_eq!(back.data, banded);
}

#[test]
fn roundtrip_type10_32bpp_rle() {
    let rgba = checker_rgba(16, 8);
    let bytes = encode_tga_rle(16, 8, &rgba).unwrap();
    assert_eq!(bytes[2], 10);
    assert_eq!(bytes[16], 32);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn roundtrip_type10_solid_color_compresses_hard() {
    // A solid 100×100 picture should compress to a small handful of
    // run-length packets per row: 100 pixels per row → 1 packet header
    // + 1 pixel per row, max-128 capped runs.
    let mut rgba = vec![0u8; 100 * 100 * 4];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&[123, 45, 67, 255]);
    }
    let bytes = encode_tga_rle(100, 100, &rgba).unwrap();
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
    // Sanity: way smaller than 100*100*3 + header.
    assert!(bytes.len() < 100 * 100 * 3 / 4);
}

#[test]
fn roundtrip_type10_random_uncompressible_data() {
    // No two adjacent pixels equal → all raw packets, max 128 each.
    let w = 40u16;
    let h = 40u16;
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for i in 0..(w as usize * h as usize) {
        // Linear-congruential garbage; deterministic.
        let r = (i.wrapping_mul(31)) as u8;
        let g = (i.wrapping_mul(53)) as u8;
        let b = (i.wrapping_mul(97)) as u8;
        rgba.extend_from_slice(&[r, g, b, 255]);
    }
    let bytes = encode_tga_rle(w, h, &rgba).unwrap();
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
}

// ---------------------------------------------------------------------------
// Type 1 (uncompressed colour-mapped) — read-only smoke test built by
// hand from spec §C.2.
// ---------------------------------------------------------------------------

#[test]
fn parse_type1_uncompressed_colour_mapped_24bit_palette() {
    // 4×2 image, 4-entry palette (BGR), index pattern 0,1,2,3, 3,2,1,0.
    let mut bytes = Vec::new();
    bytes.push(0); // id_length
    bytes.push(1); // cmap_type
    bytes.push(1); // image_type = uncompressed colour-mapped
    bytes.extend_from_slice(&0u16.to_le_bytes()); // cmap_first
    bytes.extend_from_slice(&4u16.to_le_bytes()); // cmap_length
    bytes.push(24); // cmap_entry_size
    bytes.extend_from_slice(&0u16.to_le_bytes()); // x_origin
    bytes.extend_from_slice(&0u16.to_le_bytes()); // y_origin
    bytes.extend_from_slice(&4u16.to_le_bytes()); // width
    bytes.extend_from_slice(&2u16.to_le_bytes()); // height
    bytes.push(8); // depth
    bytes.push(0x20); // descriptor — top-down
                      // Palette: BGR per entry.
    bytes.extend_from_slice(&[0x00, 0x00, 0xFF]); // 0 → red
    bytes.extend_from_slice(&[0x00, 0xFF, 0x00]); // 1 → green
    bytes.extend_from_slice(&[0xFF, 0x00, 0x00]); // 2 → blue
    bytes.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // 3 → white
                                                  // Pixels (top-down because descriptor bit 5 set).
    bytes.extend_from_slice(&[0, 1, 2, 3, 3, 2, 1, 0]);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, 4);
    assert_eq!(img.height, 2);
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    let want: Vec<u8> = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [0, 0, 255, 255],
        [0, 255, 0, 255],
        [255, 0, 0, 255],
    ]
    .concat();
    assert_eq!(img.data, want);
}

// ---------------------------------------------------------------------------
// Type 3 (uncompressed grayscale) — read-only smoke.
// ---------------------------------------------------------------------------

#[test]
fn parse_type3_uncompressed_grayscale() {
    let mut bytes = Vec::new();
    bytes.push(0); // id_length
    bytes.push(0); // cmap_type
    bytes.push(3); // image_type = uncompressed grayscale
    bytes.extend_from_slice(&0u16.to_le_bytes()); // cmap_first
    bytes.extend_from_slice(&0u16.to_le_bytes()); // cmap_length
    bytes.push(0); // cmap_entry_size
    bytes.extend_from_slice(&0u16.to_le_bytes()); // x_origin
    bytes.extend_from_slice(&0u16.to_le_bytes()); // y_origin
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes());
    bytes.push(8);
    bytes.push(0x20); // top-down
    let raw = [
        0x00, 0x55, 0xAA, 0xFF, 0x10, 0x40, 0x80, 0xC0, 0x20, 0x60, 0xA0, 0xE0,
    ];
    bytes.extend_from_slice(&raw);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, 4);
    assert_eq!(img.height, 3);
    assert_eq!(img.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!(img.data, raw.to_vec());
}

// ---------------------------------------------------------------------------
// Type 9 (RLE colour-mapped) — read-only smoke.
// ---------------------------------------------------------------------------

#[test]
fn parse_type9_rle_colour_mapped() {
    // 6×1 picture, 2-entry palette, run-of-3 then raw-of-3.
    let mut bytes = Vec::new();
    bytes.push(0);
    bytes.push(1); // cmap_type
    bytes.push(9); // image_type = RLE colour-mapped
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.push(24);
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&6u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.push(8);
    bytes.push(0x20);
    bytes.extend_from_slice(&[0x00, 0x00, 0xFF]); // 0 → red
    bytes.extend_from_slice(&[0xFF, 0x00, 0x00]); // 1 → blue
                                                  // RLE stream: run of 3 indices `0`, then raw of 3 indices `1,0,1`.
    bytes.push(0x80 | 2); // run, count = 3
    bytes.push(0); // pixel = palette[0] (red)
    bytes.push(2); // raw, count = 3
    bytes.push(1);
    bytes.push(0);
    bytes.push(1);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, 6);
    assert_eq!(
        img.data,
        [
            [255, 0, 0, 255], // run pixel
            [255, 0, 0, 255],
            [255, 0, 0, 255],
            [0, 0, 255, 255], // raw pixels
            [255, 0, 0, 255],
            [0, 0, 255, 255],
        ]
        .concat()
    );
}

// ---------------------------------------------------------------------------
// Type 11 (RLE grayscale) — read-only smoke.
// ---------------------------------------------------------------------------

#[test]
fn parse_type11_rle_grayscale() {
    let mut bytes = Vec::new();
    bytes.push(0);
    bytes.push(0);
    bytes.push(11); // image_type = RLE grayscale
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&5u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.push(8);
    bytes.push(0x20);
    // RLE stream: raw of 2 (10, 20), then run of 3 of 0xAA.
    bytes.push(0x01);
    bytes.push(10);
    bytes.push(20);
    bytes.push(0x80 | 2);
    bytes.push(0xAA);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!(img.data, vec![10, 20, 0xAA, 0xAA, 0xAA]);
}

// ---------------------------------------------------------------------------
// Pixel depth: 15 bpp + 16 bpp readers (type 2) — built by hand.
// ---------------------------------------------------------------------------

fn write_type2_15bit(w: u16, h: u16, depth: u8, pixels16: &[u16]) -> Vec<u8> {
    assert_eq!(pixels16.len(), w as usize * h as usize);
    let mut bytes = Vec::new();
    bytes.push(0);
    bytes.push(0);
    bytes.push(2);
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&w.to_le_bytes());
    bytes.extend_from_slice(&h.to_le_bytes());
    bytes.push(depth);
    bytes.push(0x20); // top-down
    for &v in pixels16 {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

#[test]
fn parse_type2_15bit_a1r5g5b5() {
    // 0x7C00 = pure red (R=31), 0x03E0 = pure green, 0x001F = pure blue.
    let pixels = vec![
        0x7C00, 0x03E0, 0x001F, 0x7FFF, /* white, alpha bit clear → still 0xFF for 15-bit */
    ];
    let bytes = write_type2_15bit(2, 2, 15, &pixels);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data[0..4], [255, 0, 0, 255]);
    assert_eq!(img.data[4..8], [0, 255, 0, 255]);
    assert_eq!(img.data[8..12], [0, 0, 255, 255]);
    assert_eq!(img.data[12..16], [255, 255, 255, 255]);
}

#[test]
fn parse_type2_16bit_a1r5g5b5_alpha_bit() {
    // 16-bit: top bit = alpha. 0x7C00 has alpha bit clear → α = 0.
    // 0xFC00 is the same red with alpha bit set → α = 0xFF.
    let pixels = vec![0x7C00, 0xFC00];
    let bytes = write_type2_15bit(2, 1, 16, &pixels);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data[0..4], [255, 0, 0, 0]);
    assert_eq!(img.data[4..8], [255, 0, 0, 255]);
}

// ---------------------------------------------------------------------------
// Bottom-up vs top-down origin — same image, two on-disk orderings.
// ---------------------------------------------------------------------------

#[test]
fn parse_type2_bottom_up_is_flipped_to_top_down() {
    // 1×2 image: red on top, blue on bottom in the OUT (top-down) sense.
    // Bottom-up on disk means we write blue first then red.
    let mut bytes = Vec::new();
    bytes.push(0);
    bytes.push(0);
    bytes.push(2);
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.push(24);
    bytes.push(0x00); // bottom-up
                      // BGR on disk; bottom row first.
    bytes.extend_from_slice(&[0xFF, 0x00, 0x00]); // blue (BGR)
    bytes.extend_from_slice(&[0x00, 0x00, 0xFF]); // red
    let img = parse_tga(&bytes).unwrap();
    // After flip: top row = red, bottom row = blue.
    assert_eq!(img.data, vec![255, 0, 0, 255, 0, 0, 255, 255]);
}

// ---------------------------------------------------------------------------
// TGA 2.0 footer — recognise + parse extension/developer offsets.
// ---------------------------------------------------------------------------

#[test]
fn footer_parse() {
    // Bare body + 26-byte footer.
    let mut bytes = vec![0u8; 200];
    let off = bytes.len();
    bytes.extend_from_slice(&0x1234_5678u32.to_le_bytes()); // ext area offset
    bytes.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes()); // dev dir offset
    bytes.extend_from_slice(b"TRUEVISION-XFILE.\0");
    let footer = parse_tga_footer(&bytes).unwrap();
    assert_eq!(footer.extension_area_offset, 0x1234_5678);
    assert_eq!(footer.developer_directory_offset, 0xDEAD_BEEF);
    let _ = off;
}

#[test]
fn footer_absent_for_tga_1_0_file() {
    // A real type-2 file with no footer: encoder always omits it.
    let rgba = opaque_checker_rgba(4, 4);
    let bytes = encode_tga_uncompressed(4, 4, &rgba).unwrap();
    assert!(parse_tga_footer(&bytes).is_none());
}

#[test]
fn footer_present_does_not_break_parse() {
    // Append a footer to an encoded file and make sure parse_tga still
    // produces the same image.
    let rgba = opaque_checker_rgba(4, 4);
    let mut bytes = encode_tga_uncompressed(4, 4, &rgba).unwrap();
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(b"TRUEVISION-XFILE.\0");
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
    assert!(parse_tga_footer(&bytes).is_some());
}

// ---------------------------------------------------------------------------
// Error paths.
// ---------------------------------------------------------------------------

#[test]
fn rejects_truncated_header() {
    let bytes = [0u8; 10];
    assert!(parse_tga(&bytes).is_err());
}

#[test]
fn rejects_unknown_image_type() {
    let mut bytes = vec![0u8; 18];
    bytes[2] = 32; // unknown
    bytes[12] = 1; // width = 1
    bytes[14] = 1; // height = 1
    bytes[16] = 8;
    assert!(parse_tga(&bytes).is_err());
}

#[test]
fn rejects_zero_dimension() {
    let rgba = vec![0u8; 4];
    let res = encode_tga_uncompressed(0, 1, &rgba);
    // Encoder will produce *something*; the decoder should reject it.
    if let Ok(bytes) = res {
        assert!(parse_tga(&bytes).is_err());
    }
}

#[test]
fn decodes_right_to_left_columns_by_mirroring() {
    // The encoder writes a top-down, left-to-right file. Flipping
    // image-descriptor bit 4 to 1 relabels the on-disk columns as
    // right-to-left, so the decoder must mirror each row horizontally.
    // The decoded output therefore equals the source pixels with every
    // row reversed left-to-right.
    let (w, h) = (4u16, 3u16);
    let rgba = opaque_checker_rgba(w, h);
    let mut bytes = encode_tga_uncompressed(w, h, &rgba).unwrap();
    assert_eq!(bytes[17] & 0x20, 0x20, "encoder emits top-down (bit 5)");
    bytes[17] |= 0x10; // set bit 4: right-to-left columns

    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, w as u32);
    assert_eq!(img.height, h as u32);

    // Build the expected output: source rows mirrored, alpha forced to
    // 0xFF (24-bpp source has no alpha channel on disk).
    let bpp = 4usize;
    let stride = w as usize * bpp;
    let mut expected = vec![0u8; rgba.len()];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let src = &rgba[y * stride + x * bpp..][..bpp];
            let dst = &mut expected[y * stride + (w as usize - 1 - x) * bpp..][..bpp];
            dst[0] = src[0];
            dst[1] = src[1];
            dst[2] = src[2];
            dst[3] = 0xFF;
        }
    }
    assert_eq!(
        img.data, expected,
        "right-to-left file must decode mirrored"
    );
}

#[test]
fn right_to_left_plus_bottom_up_normalises_both_axes() {
    // bit 4 (right-to-left) and bit 5 clear (bottom-up) combined: the
    // decoder must flip rows AND mirror columns to reach a top-left,
    // left-to-right origin. Result == source rotated 180°.
    let (w, h) = (5u16, 4u16);
    let rgba = opaque_checker_rgba(w, h);
    let mut bytes = encode_tga_uncompressed(w, h, &rgba).unwrap();
    bytes[17] &= !0x20; // clear bit 5: bottom-up rows
    bytes[17] |= 0x10; // set bit 4: right-to-left columns

    let img = parse_tga(&bytes).unwrap();
    let bpp = 4usize;
    let stride = w as usize * bpp;
    let mut expected = vec![0u8; rgba.len()];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let src = &rgba[y * stride + x * bpp..][..bpp];
            let dy = h as usize - 1 - y;
            let dx = w as usize - 1 - x;
            let dst = &mut expected[dy * stride + dx * bpp..][..bpp];
            dst[0] = src[0];
            dst[1] = src[1];
            dst[2] = src[2];
            dst[3] = 0xFF;
        }
    }
    assert_eq!(img.data, expected, "180° rotation expected");
}

// ---------------------------------------------------------------------------
// Random-fixture self-roundtrip on every (type, depth) combo we ship a
// writer for. Hard-asserts the output equals the input byte-for-byte.
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_all_writer_combos() {
    let cases: &[(u16, u16)] = &[(1, 1), (2, 2), (8, 4), (17, 13), (64, 32)];
    for &(w, h) in cases {
        for &has_alpha in &[true, false] {
            let rgba = if has_alpha {
                checker_rgba(w, h)
            } else {
                opaque_checker_rgba(w, h)
            };
            // Type 2 (uncompressed)
            let raw = encode_tga_uncompressed(w, h, &rgba).unwrap();
            let img = parse_tga(&raw).unwrap();
            assert_eq!(
                img.data, rgba,
                "type 2 roundtrip {}×{} alpha={has_alpha}",
                w, h
            );
            // Type 10 (RLE)
            let rle = encode_tga_rle(w, h, &rgba).unwrap();
            let img2 = parse_tga(&rle).unwrap();
            assert_eq!(
                img2.data, rgba,
                "type 10 roundtrip {}×{} alpha={has_alpha}",
                w, h
            );
        }
    }
}
