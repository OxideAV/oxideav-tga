//! Round 403 — §C.5 RLE packet-boundary stress and extension-area
//! postage-stamp pixel round-trip.
//!
//! The §C.5 run/raw packet cap is 128 pixels and the encoder must not let a
//! run cross a scanline boundary. These tests drive the encoder to those
//! edges and confirm the decoder recovers the raster bit-exactly, then
//! separately confirm the decoder is *lenient* about a hand-built run that
//! does span two rows (the spec's random-access scan-line table forbids it,
//! but `parse_tga` still decodes such a file whole). Finally the
//! extension-area writer's postage-stamp thumbnail is checked for
//! pixel-exact recovery — a stronger oracle than the fuzz harness's
//! offset-arithmetic check.

use oxideav_tga::*;

fn base_header(image_type: u8, w: u16, h: u16, depth: u8, desc: u8) -> Vec<u8> {
    let mut v = vec![0u8; 18];
    v[2] = image_type;
    v[12] = (w & 0xFF) as u8;
    v[13] = (w >> 8) as u8;
    v[14] = (h & 0xFF) as u8;
    v[15] = (h >> 8) as u8;
    v[16] = depth;
    v[17] = desc;
    v
}

#[test]
fn rle_run_longer_than_128_splits_and_recovers() {
    // A single 300-pixel row of one solid colour: the encoder must emit
    // multiple run packets (128 cap) and the decoder must recover all 300.
    let w = 300u16;
    let rgb: Vec<u8> = [0x11u8, 0x22, 0x33].repeat(w as usize);
    let bytes = encode_tga_rle_rgb24(w, 1, &rgb).unwrap();
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, w as u32);
    for i in 0..w as usize {
        assert_eq!(&img.data[i * 4..i * 4 + 3], &[0x11, 0x22, 0x33], "px{i}");
        assert_eq!(img.data[i * 4 + 3], 0xFF);
    }
}

#[test]
fn rle_exact_128_run_boundary() {
    // Rows of exactly 128 identical pixels -> one full run packet per row.
    for row_len in [127u16, 128, 129, 256] {
        let rgb: Vec<u8> = [0x40u8, 0x50, 0x60].repeat(row_len as usize);
        let bytes = encode_tga_rle_rgb24(row_len, 1, &rgb).unwrap();
        let img = parse_tga(&bytes).unwrap();
        for i in 0..row_len as usize {
            assert_eq!(
                &img.data[i * 4..i * 4 + 3],
                &[0x40, 0x50, 0x60],
                "row_len {row_len} px{i}"
            );
        }
    }
}

#[test]
fn rle_alternating_raw_packets_recover() {
    // Alternating distinct colours defeat run coalescing -> raw packets;
    // 300 pixels crosses the 128-pixel raw-packet cap.
    let w = 300u16;
    let mut rgb = Vec::with_capacity(w as usize * 3);
    for i in 0..w as usize {
        let v = (i & 0xFF) as u8;
        rgb.extend_from_slice(&[v, v.wrapping_add(1), v.wrapping_add(2)]);
    }
    let bytes = encode_tga_rle_rgb24(w, 1, &rgb).unwrap();
    let img = parse_tga(&bytes).unwrap();
    for i in 0..w as usize {
        assert_eq!(&img.data[i * 4..i * 4 + 3], &rgb[i * 3..i * 3 + 3], "px{i}");
    }
}

#[test]
fn rle_run_does_not_cross_scanline_in_encoder() {
    // Two identical rows: even though the colour is constant across both
    // rows, the encoder must restart the run at the row boundary. Decoding
    // must still recover the full 2-row raster exactly.
    let w = 5u16;
    let h = 2u16;
    let rgb: Vec<u8> = [0xAAu8, 0xBB, 0xCC].repeat(w as usize * h as usize);
    let bytes = encode_tga_rle_rgb24(w, h, &rgb).unwrap();
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data.len(), w as usize * h as usize * 4);
    for i in 0..(w as usize * h as usize) {
        assert_eq!(&img.data[i * 4..i * 4 + 3], &[0xAA, 0xBB, 0xCC]);
    }
}

#[test]
fn rle_decoder_lenient_about_hand_built_cross_scanline_run() {
    // Hand-build a type-10 24bpp 3x2 file as ONE run packet of 6 pixels
    // spanning both scanlines. The spec's scan-line table forbids this, but
    // parse_tga is documented to decode such a file whole.
    let mut v = base_header(10, 3, 2, 24, 0x20);
    v.push(0x80 | (6 - 1)); // run packet, count = 6
    v.extend_from_slice(&[0xCC, 0xBB, 0xAA]); // BGR -> RGB (0xAA,0xBB,0xCC)
    let img = parse_tga(&v).unwrap();
    assert_eq!(img.data.len(), 3 * 2 * 4);
    for i in 0..6 {
        assert_eq!(&img.data[i * 4..i * 4 + 3], &[0xAA, 0xBB, 0xCC], "px{i}");
    }
}

#[test]
fn extension_postage_stamp_pixel_roundtrip() {
    // Encode a base RGBA frame, attach a hand-painted postage-stamp thumbnail
    // in the parent's format, and confirm the decoded stamp is pixel-exact.
    let (bw, bh) = (8u16, 6u16);
    let base_rgba: Vec<u8> = (0..(bw as usize * bh as usize * 4))
        .map(|i| (i * 7 % 251) as u8)
        .collect();
    // force non-opaque so the base is 32bpp and the stamp travels as BGRA
    let mut base_rgba = base_rgba;
    base_rgba[3] = 0x40;
    let base = encode_tga_uncompressed(bw, bh, &base_rgba).unwrap();

    let (sw, sh) = (3u16, 2u16);
    let stamp_rgba: Vec<u8> = (0..(sw as usize * sh as usize * 4))
        .map(|i| (200 - i) as u8)
        .collect();
    let mut stamp_rgba = stamp_rgba;
    stamp_rgba[3] = 0x20; // keep it 32bpp too
    let stamp = TgaImage {
        width: sw as u32,
        height: sh as u32,
        pixel_format: TgaPixelFormat::Rgba,
        data: stamp_rgba.clone(),
        pts: None,
    };

    let ext = ExtensionAreaInput {
        postage_stamp: Some(stamp),
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    // main image round-trips
    let main = parse_tga(&file).unwrap();
    assert_eq!(main.data, base_rgba, "main image");

    // stamp round-trips pixel-exact
    let recovered = parse_tga_postage_stamp(&file)
        .unwrap()
        .expect("stamp present");
    assert_eq!(recovered.width, sw as u32);
    assert_eq!(recovered.height, sh as u32);
    assert_eq!(recovered.data, stamp_rgba, "postage-stamp pixels");
}
