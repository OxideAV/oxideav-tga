//! Round-2 feature self-roundtrip:
//!
//! * Encoders for image types 1 (uncompressed colour-mapped), 3
//!   (uncompressed grayscale), 9 (RLE colour-mapped), 11 (RLE
//!   grayscale).
//! * TGA 2.0 extension area body parse + write.
//! * Postage-stamp / thumbnail extraction.
//!
//! Each test independently round-trips a hand-crafted byte stream so
//! that a bug in one side can't mask the same bug on the other.

use oxideav_tga::{
    encode_tga_grayscale, encode_tga_grayscale_rle, encode_tga_palette, encode_tga_palette_rle,
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga, parse_tga_extension_area,
    parse_tga_footer, parse_tga_postage_stamp, ExtensionAreaInput, TgaImage, TgaPixelFormat,
    TgaTimestamp,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn palette_image(w: u16, h: u16) -> Vec<u8> {
    // 4-colour checker pattern.
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

fn opaque_palette_image(w: u16, h: u16) -> Vec<u8> {
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

fn ramp_gray(w: u16, h: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            data.push(((x + y * 7) & 0xFF) as u8);
        }
    }
    data
}

// ---------------------------------------------------------------------------
// Type 1 (uncompressed colour-mapped) — write + read-back.
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_type1_palette() {
    let rgba = palette_image(8, 6);
    let bytes = encode_tga_palette(8, 6, &rgba).unwrap();
    // Header byte 2 = image type, byte 1 = cmap_type, byte 16 = depth.
    assert_eq!(bytes[2], 1, "image type 1");
    assert_eq!(bytes[1], 1, "cmap_type");
    assert_eq!(bytes[16], 8, "depth = 8");
    // cmap_length should equal the number of unique colours (4).
    let cmap_len = u16::from_le_bytes([bytes[5], bytes[6]]);
    assert_eq!(cmap_len, 4);
    let entry_size = bytes[7];
    assert_eq!(entry_size, 32);

    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, 8);
    assert_eq!(img.height, 6);
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    assert_eq!(img.data, rgba);
}

#[test]
fn roundtrip_type1_palette_opaque() {
    // No alpha-bearing colours → the writer auto-selects the narrowest
    // lossless colour-map entry size, a compact 24-bit BGR map (see
    // round354). The round trip is still bit-exact: 24-bit entries are
    // alpha-less, so every decoded pixel comes back opaque (0xFF), which
    // matches the all-opaque input.
    let rgba = opaque_palette_image(4, 4);
    let bytes = encode_tga_palette(4, 4, &rgba).unwrap();
    assert_eq!(bytes[2], 1);
    assert_eq!(bytes[7], 24, "opaque palette → 24-bit colour-map entries");
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn type1_palette_overflows_at_257_colors() {
    // 257 unique RGBA colours: by-design rejection (TGA palette is u8).
    let mut rgba = Vec::with_capacity(257 * 4);
    for i in 0..257u32 {
        rgba.extend_from_slice(&[(i & 0xFF) as u8, ((i >> 8) & 0xFF) as u8, 0, 0xFF]);
    }
    // Place 257 unique pixels into a 257×1 image.
    let err = encode_tga_palette(257, 1, &rgba).unwrap_err();
    let s = format!("{err}");
    assert!(s.contains("256") || s.contains("unique"), "got {s:?}");
}

// ---------------------------------------------------------------------------
// Type 3 (uncompressed grayscale) — write + read-back.
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_type3_grayscale() {
    let gray = ramp_gray(16, 8);
    let bytes = encode_tga_grayscale(16, 8, &gray).unwrap();
    assert_eq!(bytes[2], 3);
    assert_eq!(bytes[16], 8);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, 16);
    assert_eq!(img.height, 8);
    assert_eq!(img.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!(img.data, gray);
}

// ---------------------------------------------------------------------------
// Type 9 (RLE colour-mapped) — write + read-back.
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_type9_palette_rle() {
    let rgba = palette_image(16, 8);
    let bytes = encode_tga_palette_rle(16, 8, &rgba).unwrap();
    assert_eq!(bytes[2], 9);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn type9_compresses_better_than_type1_on_banded() {
    // Banded RGBA palette image: each row is a single colour. RLE
    // should beat raw decisively.
    let mut rgba = Vec::with_capacity(32 * 8 * 4);
    let palette: [[u8; 4]; 8] = [
        [10, 20, 30, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [128, 128, 0, 255],
        [128, 0, 128, 255],
        [0, 128, 128, 255],
        [255, 255, 255, 255],
    ];
    for y in 0..8 {
        for _x in 0..32 {
            rgba.extend_from_slice(&palette[y as usize]);
        }
    }
    let raw = encode_tga_palette(32, 8, &rgba).unwrap();
    let rle = encode_tga_palette_rle(32, 8, &rgba).unwrap();
    assert!(
        rle.len() < raw.len(),
        "RLE expected smaller than raw on banded fixture (rle={}, raw={})",
        rle.len(),
        raw.len()
    );
    let back = parse_tga(&rle).unwrap();
    assert_eq!(back.data, rgba);
}

// ---------------------------------------------------------------------------
// Type 11 (RLE grayscale) — write + read-back.
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_type11_grayscale_rle() {
    let gray = ramp_gray(16, 8);
    let bytes = encode_tga_grayscale_rle(16, 8, &gray).unwrap();
    assert_eq!(bytes[2], 11);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!(img.data, gray);
}

#[test]
fn type11_compresses_constant_image_hard() {
    let gray = vec![0xAAu8; 100 * 100];
    let bytes = encode_tga_grayscale_rle(100, 100, &gray).unwrap();
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, gray);
    // 100×100 = 10000 bytes raw; RLE should pack each row into 1
    // header + 1 pixel ≈ 200 bytes plus header + footer-less.
    assert!(
        bytes.len() < 1000,
        "RLE didn't compress: got {}",
        bytes.len()
    );
}

// ---------------------------------------------------------------------------
// All-writer-combos sweep — every type at multiple sizes round-trips.
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_all_round2_writer_combos() {
    let cases: &[(u16, u16)] = &[(1, 1), (2, 2), (8, 4), (17, 13), (64, 32)];
    for &(w, h) in cases {
        // Type 1 + 9 (palette).
        let rgba = palette_image(w, h);
        let bytes = encode_tga_palette(w, h, &rgba).unwrap();
        let img = parse_tga(&bytes).unwrap();
        assert_eq!(img.data, rgba, "type 1 roundtrip {}×{}", w, h);
        let bytes = encode_tga_palette_rle(w, h, &rgba).unwrap();
        let img = parse_tga(&bytes).unwrap();
        assert_eq!(img.data, rgba, "type 9 roundtrip {}×{}", w, h);
        // Type 3 + 11 (grayscale).
        let gray = ramp_gray(w, h);
        let bytes = encode_tga_grayscale(w, h, &gray).unwrap();
        let img = parse_tga(&bytes).unwrap();
        assert_eq!(img.data, gray, "type 3 roundtrip {}×{}", w, h);
        let bytes = encode_tga_grayscale_rle(w, h, &gray).unwrap();
        let img = parse_tga(&bytes).unwrap();
        assert_eq!(img.data, gray, "type 11 roundtrip {}×{}", w, h);
    }
}

// ---------------------------------------------------------------------------
// TGA 2.0 extension area body — read-only smoke from a hand-crafted
// byte stream.
// ---------------------------------------------------------------------------

#[test]
fn parse_extension_area_hand_crafted() {
    // Build a minimal type-2 image, then append an extension area.
    let rgba = opaque_palette_image(2, 2);
    let mut bytes = encode_tga_uncompressed(2, 2, &rgba).unwrap();

    let ext_offset = bytes.len() as u32;

    // 495-byte extension area, hand-built per spec §C.6.
    let mut ext = vec![0u8; 495];
    ext[0..2].copy_from_slice(&495u16.to_le_bytes()); // extension_size
                                                      // author_name @ 2 (41 bytes incl. terminator)
    ext[2..7].copy_from_slice(b"Mark\0");
    // author_comment[0] @ 43 (81 bytes incl. terminator)
    ext[43..50].copy_from_slice(b"Hello!\0");
    // timestamp @ 367
    ext[367..369].copy_from_slice(&5u16.to_le_bytes()); // month
    ext[369..371].copy_from_slice(&5u16.to_le_bytes()); // day
    ext[371..373].copy_from_slice(&2026u16.to_le_bytes()); // year
    ext[373..375].copy_from_slice(&12u16.to_le_bytes()); // hour
    ext[375..377].copy_from_slice(&34u16.to_le_bytes()); // minute
    ext[377..379].copy_from_slice(&56u16.to_le_bytes()); // second
                                                         // job_name @ 379
    ext[379..386].copy_from_slice(b"render\0");
    // job_time @ 420
    ext[420..422].copy_from_slice(&1u16.to_le_bytes());
    ext[422..424].copy_from_slice(&2u16.to_le_bytes());
    ext[424..426].copy_from_slice(&3u16.to_le_bytes());
    // software_id @ 426
    ext[426..437].copy_from_slice(b"oxideav-tga");
    // software_version @ 467 (u16 + ASCII letter)
    ext[467..469].copy_from_slice(&100u16.to_le_bytes()); // 1.00
    ext[469] = b'a';
    // key_color @ 470 (ARGB)
    ext[470..474].copy_from_slice(&[0xFF, 0x12, 0x34, 0x56]);
    // pixel_aspect_ratio @ 474
    ext[474..476].copy_from_slice(&4u16.to_le_bytes());
    ext[476..478].copy_from_slice(&3u16.to_le_bytes());
    // gamma @ 478 (2.2)
    ext[478..480].copy_from_slice(&22u16.to_le_bytes());
    ext[480..482].copy_from_slice(&10u16.to_le_bytes());
    // colour_correction_offset @ 482, postage_stamp_offset @ 486,
    // scan_line_offset @ 490 — leave at 0
    // attributes_type @ 494 (3 = useful alpha data)
    ext[494] = 3;

    bytes.extend_from_slice(&ext);
    // Footer pointing at the extension area.
    bytes.extend_from_slice(&ext_offset.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(b"TRUEVISION-XFILE.\0");

    // Footer + extension area parse independently.
    let footer = parse_tga_footer(&bytes).unwrap();
    assert_eq!(footer.extension_area_offset, ext_offset);
    let parsed = parse_tga_extension_area(&bytes).unwrap();
    assert_eq!(parsed.extension_size, 495);
    assert_eq!(parsed.author_name, "Mark");
    assert_eq!(parsed.author_comment[0], "Hello!");
    assert_eq!(parsed.author_comment[1], "");
    assert_eq!(parsed.timestamp.month, 5);
    assert_eq!(parsed.timestamp.day, 5);
    assert_eq!(parsed.timestamp.year, 2026);
    assert_eq!(parsed.timestamp.hour, 12);
    assert_eq!(parsed.timestamp.minute, 34);
    assert_eq!(parsed.timestamp.second, 56);
    assert_eq!(parsed.job_name, "render");
    assert_eq!(parsed.job_time, (1, 2, 3));
    assert_eq!(parsed.software_id, "oxideav-tga");
    assert_eq!(parsed.software_version, (100, 'a'));
    assert_eq!(parsed.key_color, [0xFF, 0x12, 0x34, 0x56]);
    assert_eq!(parsed.pixel_aspect_ratio, (4, 3));
    assert_eq!(parsed.gamma, (22, 10));
    assert_eq!(parsed.colour_correction_offset, 0);
    assert_eq!(parsed.postage_stamp_offset, 0);
    assert_eq!(parsed.scan_line_offset, 0);
    assert_eq!(parsed.attributes_type, 3);

    // Main image still parses unchanged.
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
}

// ---------------------------------------------------------------------------
// TGA 2.0 extension area write + read-back (encode_tga_with_extension).
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_extension_area_write_then_parse() {
    let rgba = opaque_palette_image(8, 6);
    let base = encode_tga_uncompressed(8, 6, &rgba).unwrap();
    let ext_in = ExtensionAreaInput {
        author_name: "Mark Karpeles".to_string(),
        author_comment: [
            "Round 2 extension area write smoke test".to_string(),
            "second line".to_string(),
            String::new(),
            String::new(),
        ],
        timestamp: TgaTimestamp {
            month: 5,
            day: 5,
            year: 2026,
            hour: 10,
            minute: 30,
            second: 0,
        },
        job_name: "ci".to_string(),
        job_time: (0, 1, 30),
        software_id: "oxideav-tga".to_string(),
        software_version: (10, 'a'),
        key_color: [0xFF, 0, 0, 0],
        pixel_aspect_ratio: (1, 1),
        gamma: (22, 10),
        attributes_type: 3,
        postage_stamp: None,
        colour_correction_table: None,
        scan_line_table: None,
        developer_tags: Vec::new(),
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    // Footer survives the append.
    let footer = parse_tga_footer(&full).unwrap();
    assert_ne!(footer.extension_area_offset, 0);
    assert_eq!(footer.developer_directory_offset, 0);

    // Extension area parses + carries the values we wrote.
    let parsed = parse_tga_extension_area(&full).unwrap();
    assert_eq!(parsed.author_name, "Mark Karpeles");
    assert_eq!(
        parsed.author_comment[0],
        "Round 2 extension area write smoke test"
    );
    assert_eq!(parsed.author_comment[1], "second line");
    assert_eq!(parsed.author_comment[2], "");
    assert_eq!(parsed.timestamp.year, 2026);
    assert_eq!(parsed.timestamp.month, 5);
    assert_eq!(parsed.timestamp.minute, 30);
    assert_eq!(parsed.job_name, "ci");
    assert_eq!(parsed.job_time, (0, 1, 30));
    assert_eq!(parsed.software_id, "oxideav-tga");
    assert_eq!(parsed.software_version, (10, 'a'));
    assert_eq!(parsed.key_color, [0xFF, 0, 0, 0]);
    assert_eq!(parsed.pixel_aspect_ratio, (1, 1));
    assert_eq!(parsed.gamma, (22, 10));
    assert_eq!(parsed.attributes_type, 3);
    assert_eq!(parsed.postage_stamp_offset, 0);

    // Main image still parses + matches the input.
    let img = parse_tga(&full).unwrap();
    assert_eq!(img.data, rgba);
}

// ---------------------------------------------------------------------------
// Postage-stamp / thumbnail extraction.
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_postage_stamp_rgba() {
    // Main image: 16×16 colour checker. Thumbnail: 4×4 same checker.
    let main_rgba = opaque_palette_image(16, 16);
    let stamp_rgba = opaque_palette_image(4, 4);
    let stamp = TgaImage {
        width: 4,
        height: 4,
        pixel_format: TgaPixelFormat::Rgba,
        data: stamp_rgba.clone(),
        pts: None,
    };
    let base = encode_tga_uncompressed(16, 16, &main_rgba).unwrap();
    let ext_in = ExtensionAreaInput {
        postage_stamp: Some(stamp),
        ..ExtensionAreaInput::default()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    let parsed = parse_tga_extension_area(&full).unwrap();
    assert_ne!(
        parsed.postage_stamp_offset, 0,
        "extension area must point at the postage stamp we wrote"
    );

    let stamp_back = parse_tga_postage_stamp(&full)
        .unwrap()
        .expect("postage stamp should decode");
    assert_eq!(stamp_back.width, 4);
    assert_eq!(stamp_back.height, 4);
    assert_eq!(stamp_back.pixel_format, TgaPixelFormat::Rgba);
    assert_eq!(stamp_back.data, stamp_rgba);
}

#[test]
fn roundtrip_postage_stamp_grayscale_parent() {
    // Grayscale main image + grayscale thumbnail: postage-stamp pixel
    // format follows the parent's image type.
    let main_gray = ramp_gray(32, 32);
    let stamp_gray = ramp_gray(8, 8);
    let stamp = TgaImage {
        width: 8,
        height: 8,
        pixel_format: TgaPixelFormat::Gray8,
        data: stamp_gray.clone(),
        pts: None,
    };
    let base = encode_tga_grayscale(32, 32, &main_gray).unwrap();
    let ext_in = ExtensionAreaInput {
        postage_stamp: Some(stamp),
        ..ExtensionAreaInput::default()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    let stamp_back = parse_tga_postage_stamp(&full)
        .unwrap()
        .expect("postage stamp should decode");
    assert_eq!(stamp_back.width, 8);
    assert_eq!(stamp_back.height, 8);
    assert_eq!(stamp_back.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!(stamp_back.data, stamp_gray);
}

#[test]
fn no_postage_stamp_returns_none() {
    let rgba = opaque_palette_image(4, 4);
    let base = encode_tga_uncompressed(4, 4, &rgba).unwrap();
    let ext_in = ExtensionAreaInput::default();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let stamp = parse_tga_postage_stamp(&full).unwrap();
    assert!(stamp.is_none());
}

#[test]
fn no_extension_area_returns_none() {
    let rgba = opaque_palette_image(4, 4);
    let base = encode_tga_uncompressed(4, 4, &rgba).unwrap();
    // Plain TGA 1.0 file (no footer).
    assert!(parse_tga_extension_area(&base).is_none());
    assert!(parse_tga_postage_stamp(&base).unwrap().is_none());
}

#[test]
fn postage_stamp_dimension_overflow_rejected() {
    // The stamp's on-disk size header is u8 — anything > 255 must be
    // rejected at encode time rather than silently truncated.
    let rgba = opaque_palette_image(4, 4);
    let base = encode_tga_uncompressed(4, 4, &rgba).unwrap();
    let stamp = TgaImage {
        width: 256,
        height: 4,
        pixel_format: TgaPixelFormat::Rgba,
        data: vec![0u8; 256 * 4 * 4],
        pts: None,
    };
    let ext_in = ExtensionAreaInput {
        postage_stamp: Some(stamp),
        ..ExtensionAreaInput::default()
    };
    let err = encode_tga_with_extension(&base, &ext_in).unwrap_err();
    let s = format!("{err}");
    assert!(s.contains("255") || s.contains("dimensions"), "got {s:?}");
}
