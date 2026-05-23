//! Round 5: colour-correction table (CCT) *application* (spec §C.6.8).
//!
//! Round 4 landed CCT parse + write but treated the table as opaque
//! metadata. This round adds the actual LUT-application surface
//! (`correct_rgba16` / `correct_rgba8` / `correct_gray8` /
//! `apply_to_image`) plus locks down the on-disk interleaved
//! `(A,R,G,B)` byte layout the spec mandates ("each set of four
//! contiguous values are the desired A:R:G:B correction for that
//! entry").

use oxideav_tga::{
    encode_tga_grayscale, encode_tga_uncompressed, encode_tga_with_extension, parse_tga,
    parse_tga_colour_correction_table, ExtensionAreaInput, TgaColourCorrectionTable, TgaImage,
    TgaPixelFormat, TgaTimestamp, TGA_COLOUR_CORRECTION_TABLE_ENTRIES,
    TGA_COLOUR_CORRECTION_TABLE_SIZE,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn solid_rgba(width: u16, height: u16, rgba: [u8; 4]) -> Vec<u8> {
    let mut v = Vec::with_capacity(width as usize * height as usize * 4);
    for _ in 0..(width as usize * height as usize) {
        v.extend_from_slice(&rgba);
    }
    v
}

fn baseline_ext() -> ExtensionAreaInput {
    ExtensionAreaInput {
        author_name: String::new(),
        author_comment: [String::new(), String::new(), String::new(), String::new()],
        timestamp: TgaTimestamp::default(),
        job_name: String::new(),
        job_time: (0, 0, 0),
        software_id: "oxideav-tga".to_string(),
        software_version: (100, 'a'),
        key_color: [0; 4],
        pixel_aspect_ratio: (0, 0),
        gamma: (0, 0),
        attributes_type: 3,
        postage_stamp: None,
        colour_correction_table: None,
        scan_line_table: None,
        developer_tags: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// On-disk layout: spec §C.6.8 stores the table INTERLEAVED, not planar.
// "256 x 4 SHORT values, where each set of four contiguous values are the
//  desired A:R:G:B correction for that entry."
// ---------------------------------------------------------------------------

#[test]
fn cct_on_disk_layout_is_interleaved_argb() {
    let mut cct = TgaColourCorrectionTable::default();
    // Distinct sentinels per channel at entry 0 and entry 1 so an
    // interleaved layout is observable in the raw bytes.
    cct.alpha[0] = 0x1111;
    cct.red[0] = 0x2222;
    cct.green[0] = 0x3333;
    cct.blue[0] = 0x4444;
    cct.alpha[1] = 0x5555;
    cct.red[1] = 0x6666;
    cct.green[1] = 0x7777;
    cct.blue[1] = 0x8888;

    let bytes = cct.to_bytes();
    assert_eq!(bytes.len(), TGA_COLOUR_CORRECTION_TABLE_SIZE);

    // Entry 0 occupies bytes 0..8 as A,R,G,B little-endian u16s.
    assert_eq!(&bytes[0..2], &0x1111u16.to_le_bytes(), "entry0 A");
    assert_eq!(&bytes[2..4], &0x2222u16.to_le_bytes(), "entry0 R");
    assert_eq!(&bytes[4..6], &0x3333u16.to_le_bytes(), "entry0 G");
    assert_eq!(&bytes[6..8], &0x4444u16.to_le_bytes(), "entry0 B");
    // Entry 1 occupies bytes 8..16 in the same order.
    assert_eq!(&bytes[8..10], &0x5555u16.to_le_bytes(), "entry1 A");
    assert_eq!(&bytes[10..12], &0x6666u16.to_le_bytes(), "entry1 R");
    assert_eq!(&bytes[12..14], &0x7777u16.to_le_bytes(), "entry1 G");
    assert_eq!(&bytes[14..16], &0x8888u16.to_le_bytes(), "entry1 B");
}

#[test]
fn cct_parse_inverts_interleaved_to_bytes() {
    let mut cct = TgaColourCorrectionTable::default();
    cct.alpha[3] = 0xABCD;
    cct.red[200] = 0x0102;
    cct.green[17] = 0xFEDC;
    cct.blue[255] = 0x9090;

    let bytes = cct.to_bytes();
    // Lay the table at a non-zero offset so the "0 = absent" sentinel
    // path isn't exercised.
    let mut buf = vec![0u8; 8];
    buf.extend_from_slice(&bytes);
    let parsed = TgaColourCorrectionTable::parse(&buf, 8).expect("parse interleaved CCT");
    assert_eq!(parsed, cct);
}

// ---------------------------------------------------------------------------
// correct_rgba16 / correct_rgba8 — per-channel LUT lookup.
// ---------------------------------------------------------------------------

#[test]
fn identity_table_correct_rgba8_is_noop() {
    let cct = TgaColourCorrectionTable::default();
    for &px in &[[0, 0, 0, 0], [255, 255, 255, 255], [40, 80, 120, 200]] {
        assert_eq!(
            cct.correct_rgba8(px),
            px,
            "identity must be a no-op at 8-bit"
        );
    }
}

#[test]
fn identity_table_correct_rgba16_widens_byte() {
    let cct = TgaColourCorrectionTable::default();
    // Identity curve: out[i] = (i << 8) | i.
    assert_eq!(cct.correct_rgba16([0, 0, 0, 0]), [0, 0, 0, 0]);
    assert_eq!(
        cct.correct_rgba16([255, 255, 255, 255]),
        [0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF]
    );
    assert_eq!(
        cct.correct_rgba16([1, 2, 3, 4]),
        [0x0101, 0x0202, 0x0303, 0x0404]
    );
}

#[test]
fn correct_rgba16_picks_per_channel_curve() {
    // Build a table that maps every input to a fixed per-channel output
    // so we can verify the R/G/B/A curves are addressed independently
    // and in the right order.
    let mut cct = TgaColourCorrectionTable::default();
    for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
        cct.red[i] = 0xAAAA;
        cct.green[i] = 0xBBBB;
        cct.blue[i] = 0xCCCC;
        cct.alpha[i] = 0xDDDD;
    }
    // Input order is [R, G, B, A]; output keeps that order.
    let out = cct.correct_rgba16([10, 20, 30, 40]);
    assert_eq!(out, [0xAAAA, 0xBBBB, 0xCCCC, 0xDDDD]);
}

#[test]
fn correct_rgba8_high_byte_narrowing() {
    let mut cct = TgaColourCorrectionTable::default();
    cct.red[100] = 0x1234; // high byte 0x12
    cct.green[100] = 0xFF00; // high byte 0xFF
    cct.blue[100] = 0x00FF; // high byte 0x00
    cct.alpha[100] = 0x80FF; // high byte 0x80
    let out = cct.correct_rgba8([100, 100, 100, 100]);
    assert_eq!(out, [0x12, 0xFF, 0x00, 0x80]);
}

#[test]
fn correct_inversion_applies_curve() {
    // Inversion curve: out[i] = ((255 - i) << 8) | (255 - i).
    let mut cct = TgaColourCorrectionTable::default();
    for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
        let inv = (255 - i) as u16;
        let v = (inv << 8) | inv;
        cct.red[i] = v;
        cct.green[i] = v;
        cct.blue[i] = v;
        cct.alpha[i] = v;
    }
    assert_eq!(cct.correct_rgba8([0, 50, 200, 255]), [255, 205, 55, 0]);
}

// ---------------------------------------------------------------------------
// correct_gray8 — single-channel luma path (uses the green curve).
// ---------------------------------------------------------------------------

#[test]
fn correct_gray8_uses_green_curve() {
    let mut cct = TgaColourCorrectionTable::default();
    cct.green[64] = 0xC0DE; // high byte 0xC0
    cct.red[64] = 0x0000; // must NOT be consulted for grayscale
    assert_eq!(cct.correct_gray8(64), 0xC0);
    // Identity green curve is a no-op.
    let id = TgaColourCorrectionTable::default();
    assert_eq!(id.correct_gray8(123), 123);
}

// ---------------------------------------------------------------------------
// apply_to_image — in-place application across pixel formats.
// ---------------------------------------------------------------------------

fn rgba_image(width: u32, height: u32, data: Vec<u8>) -> TgaImage {
    TgaImage {
        width,
        height,
        pixel_format: TgaPixelFormat::Rgba,
        data,
        pts: None,
    }
}

#[test]
fn apply_identity_leaves_rgba_unchanged() {
    let original = solid_rgba(5, 4, [12, 34, 56, 78]);
    let mut img = rgba_image(5, 4, original.clone());
    TgaColourCorrectionTable::default().apply_to_image(&mut img);
    assert_eq!(img.data, original, "identity apply must be bit-exact");
}

#[test]
fn apply_inversion_to_rgba_image() {
    let mut cct = TgaColourCorrectionTable::default();
    for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
        let inv = (255 - i) as u16;
        let v = (inv << 8) | inv;
        cct.red[i] = v;
        cct.green[i] = v;
        cct.blue[i] = v;
        cct.alpha[i] = v;
    }
    // Two pixels: [10,20,30,40] and [200,100,0,255].
    let data = vec![10, 20, 30, 40, 200, 100, 0, 255];
    let mut img = rgba_image(2, 1, data);
    cct.apply_to_image(&mut img);
    assert_eq!(img.data, vec![245, 235, 225, 215, 55, 155, 255, 0]);
}

#[test]
fn apply_to_gray8_image_uses_green_curve() {
    let mut cct = TgaColourCorrectionTable::default();
    // green[x] = (2x clamped) << 8 — a simple brighten curve.
    for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
        let out = ((i * 2).min(255)) as u16;
        cct.green[i] = (out << 8) | out;
    }
    let mut img = TgaImage {
        width: 3,
        height: 1,
        pixel_format: TgaPixelFormat::Gray8,
        data: vec![10, 100, 200],
        pts: None,
    };
    cct.apply_to_image(&mut img);
    assert_eq!(img.data, vec![20, 200, 255]);
}

#[test]
fn apply_to_rgb24_image_skips_alpha() {
    let mut cct = TgaColourCorrectionTable::default();
    // Distinct constant per colour curve; alpha curve must be ignored.
    for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
        cct.red[i] = 0x1000;
        cct.green[i] = 0x2000;
        cct.blue[i] = 0x3000;
        cct.alpha[i] = 0xFF00; // would corrupt output if (wrongly) used
    }
    let mut img = TgaImage {
        width: 2,
        height: 1,
        pixel_format: TgaPixelFormat::Rgb24,
        data: vec![1, 2, 3, 4, 5, 6],
        pts: None,
    };
    cct.apply_to_image(&mut img);
    assert_eq!(img.data, vec![0x10, 0x20, 0x30, 0x10, 0x20, 0x30]);
}

// ---------------------------------------------------------------------------
// End-to-end: encode with a CCT, decode the image, recover the CCT, apply.
// ---------------------------------------------------------------------------

#[test]
fn end_to_end_decode_then_apply_cct() {
    // Brighten-by-LUT curve: out[i] = min(i + 32, 255).
    let mut cct = TgaColourCorrectionTable::default();
    for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
        let out = ((i + 32).min(255)) as u16;
        cct.red[i] = (out << 8) | out;
        cct.green[i] = (out << 8) | out;
        cct.blue[i] = (out << 8) | out;
        cct.alpha[i] = ((i as u16) << 8) | i as u16; // identity alpha
    }

    let rgba = solid_rgba(4, 4, [10, 20, 30, 255]);
    let base = encode_tga_uncompressed(4, 4, &rgba).unwrap();
    let ext_in = ExtensionAreaInput {
        colour_correction_table: Some(cct.clone()),
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    // Decode the image and recover the table independently.
    let mut img = parse_tga(&full).expect("decode");
    let recovered = parse_tga_colour_correction_table(&full).expect("recover CCT");
    assert_eq!(recovered, cct, "CCT survives the interleaved round-trip");

    recovered.apply_to_image(&mut img);
    // Every pixel was (10,20,30,255) → (42,52,62,255).
    for px in img.data.chunks_exact(4) {
        assert_eq!(px, &[42, 52, 62, 255]);
    }
}

#[test]
fn end_to_end_grayscale_decode_then_apply() {
    // Darken curve on the green (=luma) channel: out[i] = i / 2.
    let mut cct = TgaColourCorrectionTable::default();
    for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
        let out = (i / 2) as u16;
        cct.green[i] = (out << 8) | out;
    }
    let gray = vec![100u8; 9];
    let base = encode_tga_grayscale(3, 3, &gray).unwrap();
    let ext_in = ExtensionAreaInput {
        colour_correction_table: Some(cct.clone()),
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    let mut img = parse_tga(&full).expect("decode gray");
    assert_eq!(img.pixel_format, TgaPixelFormat::Gray8);
    let recovered = parse_tga_colour_correction_table(&full).expect("recover CCT");
    recovered.apply_to_image(&mut img);
    assert!(img.data.iter().all(|&b| b == 50), "100 → 50 via /2 curve");
}
