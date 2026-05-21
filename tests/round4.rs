//! Round 4: developer area + colour-correction table + scan-line table
//! + typed attributes-type roundtrips.
//!
//! Covers the four lacking surfaces flagged in the round-3 README:
//! `parse_tga_developer_area`, `parse_tga_colour_correction_table`,
//! `parse_tga_scan_line_table`, and the `AttributesType` typed view.
//!
//! Each test exercises a write-then-parse cycle end-to-end via
//! `encode_tga_with_extension` so the offset back-patching machinery
//! is verified, plus targeted parser tests on hand-crafted byte
//! buffers to nail truncation behaviour.

use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga_colour_correction_table,
    parse_tga_developer_area, parse_tga_extension_area, parse_tga_footer,
    parse_tga_scan_line_table, AttributesType, DeveloperTagInput, ExtensionAreaInput,
    TgaColourCorrectionTable, TgaDeveloperArea, TgaScanLineTable, TgaTimestamp,
    TGA_COLOUR_CORRECTION_TABLE_ENTRIES, TGA_COLOUR_CORRECTION_TABLE_SIZE,
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
        author_name: "Mark Karpeles".to_string(),
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
// AttributesType — typed view over §C.6.13 byte.
// ---------------------------------------------------------------------------

#[test]
fn attributes_type_named_variants_roundtrip() {
    for &v in &[0u8, 1, 2, 3, 4] {
        let typed = AttributesType::from_u8(v);
        assert_eq!(typed.as_u8(), v, "named variant byte={v}");
    }
}

#[test]
fn attributes_type_reserved_byte_preserved() {
    for v in 5u8..=200 {
        let typed = AttributesType::from_u8(v);
        assert!(matches!(typed, AttributesType::Reserved(_)));
        assert_eq!(typed.as_u8(), v);
    }
}

#[test]
fn attributes_type_has_meaningful_alpha_predicates() {
    assert!(!AttributesType::NoAlpha.has_meaningful_alpha());
    assert!(!AttributesType::UndefinedIgnore.has_meaningful_alpha());
    assert!(!AttributesType::UndefinedRetain.has_meaningful_alpha());
    assert!(AttributesType::UsefulAlpha.has_meaningful_alpha());
    assert!(AttributesType::PremultipliedAlpha.has_meaningful_alpha());
    assert!(!AttributesType::Reserved(42).has_meaningful_alpha());
    assert!(AttributesType::PremultipliedAlpha.is_premultiplied());
    assert!(!AttributesType::UsefulAlpha.is_premultiplied());
}

#[test]
fn attributes_type_default_is_no_alpha() {
    assert_eq!(AttributesType::default(), AttributesType::NoAlpha);
}

#[test]
fn extension_area_attributes_accessor_matches_byte() {
    let rgba = solid_rgba(2, 2, [10, 20, 30, 128]);
    let base = encode_tga_uncompressed(2, 2, &rgba).unwrap();
    let ext_in = ExtensionAreaInput {
        attributes_type: 4,
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let parsed = parse_tga_extension_area(&full).unwrap();
    assert_eq!(parsed.attributes_type, 4);
    assert_eq!(parsed.attributes(), AttributesType::PremultipliedAlpha);
}

#[test]
fn extension_area_attributes_accessor_reserved_byte() {
    // Forge a buffer with a non-standard attributes byte and confirm
    // it round-trips through the typed accessor.
    let rgba = solid_rgba(1, 1, [255, 255, 255, 255]);
    let base = encode_tga_uncompressed(1, 1, &rgba).unwrap();
    let ext_in = ExtensionAreaInput {
        attributes_type: 99,
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let parsed = parse_tga_extension_area(&full).unwrap();
    assert_eq!(parsed.attributes(), AttributesType::Reserved(99));
}

// ---------------------------------------------------------------------------
// Colour-correction table (spec §C.6.8).
// ---------------------------------------------------------------------------

#[test]
fn colour_correction_table_default_is_identity_curve() {
    let cct = TgaColourCorrectionTable::default();
    for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
        let expected = ((i as u16) << 8) | (i as u16);
        assert_eq!(cct.alpha[i], expected, "alpha[{i}]");
        assert_eq!(cct.red[i], expected, "red[{i}]");
        assert_eq!(cct.green[i], expected, "green[{i}]");
        assert_eq!(cct.blue[i], expected, "blue[{i}]");
    }
}

#[test]
fn colour_correction_table_to_bytes_size_matches_spec() {
    let cct = TgaColourCorrectionTable::default();
    let bytes = cct.to_bytes();
    assert_eq!(bytes.len(), TGA_COLOUR_CORRECTION_TABLE_SIZE);
    assert_eq!(bytes.len(), 4 * 256 * 2);
}

#[test]
fn colour_correction_table_roundtrip_through_extension_area() {
    let rgba = solid_rgba(4, 3, [40, 80, 120, 255]);
    let base = encode_tga_uncompressed(4, 3, &rgba).unwrap();
    let mut cct = TgaColourCorrectionTable::default();
    // Tweak a handful of entries so we can confirm not just identity.
    cct.red[0] = 0x0001;
    cct.green[255] = 0x1234;
    cct.blue[128] = 0xCAFE;
    cct.alpha[42] = 0xBEEF;
    let ext_in = ExtensionAreaInput {
        colour_correction_table: Some(cct.clone()),
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    // Footer must carry the extension-area offset (already covered in
    // round 2) and parsing the extension area should now reveal a
    // non-zero `colour_correction_offset`.
    let ext = parse_tga_extension_area(&full).unwrap();
    assert_ne!(
        ext.colour_correction_offset, 0,
        "colour_correction_offset back-patched"
    );

    let parsed = parse_tga_colour_correction_table(&full).expect("CCT roundtrips");
    assert_eq!(parsed, cct);
}

#[test]
fn colour_correction_table_absent_when_not_supplied() {
    let rgba = solid_rgba(2, 2, [0, 0, 0, 255]);
    let base = encode_tga_uncompressed(2, 2, &rgba).unwrap();
    let ext_in = baseline_ext();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let ext = parse_tga_extension_area(&full).unwrap();
    assert_eq!(ext.colour_correction_offset, 0);
    assert!(parse_tga_colour_correction_table(&full).is_none());
}

#[test]
fn colour_correction_table_parse_rejects_truncated_buffer() {
    let mut input = vec![0u8; 100];
    // Offset 50 + 2048 bytes won't fit in a 100-byte buffer.
    let parsed = TgaColourCorrectionTable::parse(&input, 50);
    assert!(parsed.is_none(), "truncated CCT rejected");
    // Zero offset is the documented "absent" sentinel.
    input.resize(TGA_COLOUR_CORRECTION_TABLE_SIZE + 10, 0);
    assert!(TgaColourCorrectionTable::parse(&input, 0).is_none());
    // Now place the CCT at offset 0 (skip the sentinel) — offset 1
    // should succeed when the buffer is long enough.
    let big = vec![0u8; TGA_COLOUR_CORRECTION_TABLE_SIZE + 8];
    assert!(TgaColourCorrectionTable::parse(&big, 4).is_some());
}

// ---------------------------------------------------------------------------
// Scan-line table (spec §C.6.9).
// ---------------------------------------------------------------------------

#[test]
fn scan_line_table_to_bytes_layout_is_le_u32() {
    let sct = TgaScanLineTable {
        offsets: vec![0x0102_0304, 0x0506_0708, 0x090A_0B0C],
    };
    let bytes = sct.to_bytes();
    assert_eq!(
        bytes,
        vec![
            0x04, 0x03, 0x02, 0x01, // 0x01020304 LE
            0x08, 0x07, 0x06, 0x05, // 0x05060708 LE
            0x0C, 0x0B, 0x0A, 0x09, // 0x090A0B0C LE
        ]
    );
}

#[test]
fn scan_line_table_roundtrip_through_extension_area() {
    // 5-row image so the scan-line table has 5 entries.
    let width = 4u16;
    let height = 5u16;
    let rgba = solid_rgba(width, height, [50, 60, 70, 255]);
    let base = encode_tga_uncompressed(width, height, &rgba).unwrap();
    let sct = TgaScanLineTable {
        offsets: vec![18, 30, 42, 54, 66],
    };
    let ext_in = ExtensionAreaInput {
        scan_line_table: Some(sct.clone()),
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let ext = parse_tga_extension_area(&full).unwrap();
    assert_ne!(ext.scan_line_offset, 0);

    let parsed = parse_tga_scan_line_table(&full).expect("SCT roundtrips");
    assert_eq!(parsed, sct);
    assert_eq!(parsed.offsets.len(), height as usize);
}

#[test]
fn scan_line_table_parse_zero_offset_returns_none() {
    let buf = vec![0u8; 200];
    assert!(TgaScanLineTable::parse(&buf, 0, 10).is_none());
}

#[test]
fn scan_line_table_parse_zero_height_returns_none() {
    let buf = vec![0u8; 200];
    assert!(TgaScanLineTable::parse(&buf, 10, 0).is_none());
}

#[test]
fn scan_line_table_parse_truncated_returns_none() {
    // 4 entries × 4 bytes = 16, but only 8 bytes after offset 100.
    let buf = vec![0u8; 108];
    assert!(TgaScanLineTable::parse(&buf, 100, 4).is_none());
}

// ---------------------------------------------------------------------------
// Developer area (spec §C.7).
// ---------------------------------------------------------------------------

#[test]
fn developer_area_roundtrip_three_tags() {
    let rgba = solid_rgba(3, 3, [11, 22, 33, 255]);
    let base = encode_tga_uncompressed(3, 3, &rgba).unwrap();
    let tags = vec![
        DeveloperTagInput {
            tag_id: 32768, // user-defined
            payload: vec![1, 2, 3, 4, 5, 6, 7, 8],
        },
        DeveloperTagInput {
            tag_id: 40000,
            payload: vec![0xAA; 16],
        },
        DeveloperTagInput {
            tag_id: 65535,
            payload: b"oxideav-tga-round-4".to_vec(),
        },
    ];
    let ext_in = ExtensionAreaInput {
        developer_tags: tags.clone(),
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    let footer = parse_tga_footer(&full).unwrap();
    assert_ne!(footer.developer_directory_offset, 0);

    let dev = parse_tga_developer_area(&full).expect("dev area roundtrips");
    assert_eq!(dev.tags.len(), tags.len());
    for (parsed, original) in dev.tags.iter().zip(tags.iter()) {
        assert_eq!(parsed.tag_id, original.tag_id);
        assert_eq!(parsed.size as usize, original.payload.len());
        let payload = dev
            .payload(&full, parsed)
            .expect("non-empty payloads borrowable");
        assert_eq!(payload, original.payload.as_slice());
    }
}

#[test]
fn developer_area_marker_tag_offset_zero() {
    let rgba = solid_rgba(2, 2, [0, 0, 0, 255]);
    let base = encode_tga_uncompressed(2, 2, &rgba).unwrap();
    let tags = vec![
        DeveloperTagInput {
            tag_id: 50000,
            payload: Vec::new(), // marker
        },
        DeveloperTagInput {
            tag_id: 50001,
            payload: vec![0xDE, 0xAD, 0xBE, 0xEF],
        },
    ];
    let ext_in = ExtensionAreaInput {
        developer_tags: tags,
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let dev = parse_tga_developer_area(&full).unwrap();
    assert_eq!(dev.tags.len(), 2);
    assert_eq!(dev.tags[0].offset, 0, "marker tag has offset 0");
    assert_eq!(dev.tags[0].size, 0);
    assert!(
        dev.payload(&full, &dev.tags[0]).is_none(),
        "marker tag has no payload"
    );
    assert_eq!(dev.tags[1].size, 4);
    assert_eq!(
        dev.payload(&full, &dev.tags[1]).unwrap(),
        &[0xDE, 0xAD, 0xBE, 0xEF]
    );
}

#[test]
fn developer_area_absent_returns_none() {
    let rgba = solid_rgba(2, 2, [0, 0, 0, 255]);
    let base = encode_tga_uncompressed(2, 2, &rgba).unwrap();
    let ext_in = baseline_ext();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    assert!(parse_tga_developer_area(&full).is_none());
}

#[test]
fn developer_area_parse_rejects_payload_overrun() {
    // Hand-craft a directory pointing at an out-of-bounds payload.
    // 1 directory entry, tag 1, offset = 1000, size = 100, but the
    // buffer is only 100 bytes long.
    let mut buf = vec![0u8; 100];
    // Directory at offset 80: u16 count + 1 record (10 bytes) = 12.
    buf[80] = 1; // n = 1
    buf[81] = 0;
    // tag_id = 0x0001
    buf[82] = 0x01;
    buf[83] = 0x00;
    // offset = 1000 (out of bounds)
    buf[84..88].copy_from_slice(&1000u32.to_le_bytes());
    // size = 100
    buf[88..92].copy_from_slice(&100u32.to_le_bytes());
    assert!(TgaDeveloperArea::parse(&buf, 80).is_none());
}

#[test]
fn developer_area_parse_zero_offset_returns_none() {
    let buf = vec![0u8; 200];
    assert!(TgaDeveloperArea::parse(&buf, 0).is_none());
}

#[test]
fn developer_area_parse_truncated_count_word_returns_none() {
    let buf = vec![0u8; 50];
    assert!(TgaDeveloperArea::parse(&buf, 49).is_none());
}

#[test]
fn developer_area_parse_truncated_directory_returns_none() {
    // n = 3 declared but only space for 2 entries.
    let mut buf = vec![0u8; 30];
    buf[10] = 3; // n = 3 → needs 2 + 30 = 32 bytes from offset 10
    buf[11] = 0;
    assert!(TgaDeveloperArea::parse(&buf, 10).is_none());
}

// ---------------------------------------------------------------------------
// All-four-extensions combined roundtrip.
// ---------------------------------------------------------------------------

#[test]
fn full_extension_area_with_all_four_optional_sections() {
    let width = 8u16;
    let height = 4u16;
    let rgba = solid_rgba(width, height, [200, 150, 100, 255]);
    let base = encode_tga_uncompressed(width, height, &rgba).unwrap();

    let mut cct = TgaColourCorrectionTable::default();
    cct.red[0] = 0xFFFF;
    cct.green[0] = 0x0000;
    let sct = TgaScanLineTable {
        offsets: vec![18, 42, 66, 90],
    };
    let tags = vec![
        DeveloperTagInput {
            tag_id: 32770,
            payload: b"hello world".to_vec(),
        },
        DeveloperTagInput {
            tag_id: 32771,
            payload: vec![0; 256],
        },
    ];

    let ext_in = ExtensionAreaInput {
        attributes_type: 4,
        colour_correction_table: Some(cct.clone()),
        scan_line_table: Some(sct.clone()),
        developer_tags: tags.clone(),
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    // Footer.
    let footer = parse_tga_footer(&full).unwrap();
    assert_ne!(footer.extension_area_offset, 0);
    assert_ne!(footer.developer_directory_offset, 0);

    // Extension area + typed attributes.
    let ext = parse_tga_extension_area(&full).unwrap();
    assert_eq!(ext.attributes(), AttributesType::PremultipliedAlpha);
    assert_ne!(ext.colour_correction_offset, 0);
    assert_ne!(ext.scan_line_offset, 0);

    // CCT.
    let parsed_cct = parse_tga_colour_correction_table(&full).unwrap();
    assert_eq!(parsed_cct, cct);

    // SCT.
    let parsed_sct = parse_tga_scan_line_table(&full).unwrap();
    assert_eq!(parsed_sct, sct);

    // Developer area + per-tag payloads.
    let parsed_dev = parse_tga_developer_area(&full).unwrap();
    assert_eq!(parsed_dev.tags.len(), tags.len());
    for (parsed, original) in parsed_dev.tags.iter().zip(tags.iter()) {
        assert_eq!(parsed.tag_id, original.tag_id);
        assert_eq!(
            parsed_dev.payload(&full, parsed).unwrap(),
            original.payload.as_slice()
        );
    }
}
