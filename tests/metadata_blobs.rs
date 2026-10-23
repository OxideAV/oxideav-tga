//! The standalone halves of the registry path's metadata carriage: the
//! extension area as a self-describing 495-byte slab
//! (`extension_area_bytes` / `TgaExtensionArea::from_bytes`) and its
//! text fields as `(key, value)` pairs (`metadata_pairs`); the developer
//! area in the self-contained portable form
//! (`TgaDeveloperArea::to_portable_bytes` / `tags_from_portable_bytes`).

use oxideav_tga::{
    DeveloperTagInput, EncodeOptions, ExtensionAreaInput, TGA_EXTENSION_AREA_SIZE,
    TgaDeveloperArea, TgaExtensionArea, TgaImage, TgaTimestamp, encode, encode_tga_with_extension,
    extension_area_bytes, parse_tga_developer_area, parse_tga_extension_area,
};

fn base() -> Vec<u8> {
    encode(
        &TgaImage::from_rgb8(3, 2, vec![0; 18]).unwrap(),
        &EncodeOptions::default().with_rle(false),
    )
    .unwrap()
}

fn rich_extension() -> ExtensionAreaInput {
    ExtensionAreaInput {
        author_name: "Ada".to_string(),
        author_comment: [
            "first line".to_string(),
            String::new(),
            "third line".to_string(),
            String::new(),
        ],
        timestamp: TgaTimestamp {
            month: 10,
            day: 17,
            year: 2026,
            hour: 9,
            minute: 30,
            second: 5,
        },
        job_name: "render-42".to_string(),
        job_time: (1, 2, 3),
        software_id: "oxideav".to_string(),
        software_version: (123, 'b'),
        key_color: [0, 1, 2, 3],
        pixel_aspect_ratio: (4, 3),
        gamma: (220, 100),
        attributes_type: 3,
        ..ExtensionAreaInput::default()
    }
}

#[test]
fn extension_area_bytes_is_the_slab_the_parser_reads() {
    let file = encode_tga_with_extension(&base(), &rich_extension()).unwrap();
    let slab = extension_area_bytes(&file).unwrap();
    assert_eq!(slab.len(), TGA_EXTENSION_AREA_SIZE);
    let parsed = TgaExtensionArea::from_bytes(slab).unwrap();
    assert_eq!(Some(parsed.clone()), parse_tga_extension_area(&file));
    assert_eq!(parsed.author_name, "Ada");
    assert_eq!(parsed.gamma, (220, 100));
    // Short slabs are refused; bytes past the slab are ignored.
    assert!(TgaExtensionArea::from_bytes(&slab[..494]).is_none());
    let mut longer = slab.to_vec();
    longer.extend_from_slice(&[9; 10]);
    assert_eq!(TgaExtensionArea::from_bytes(&longer), Some(parsed));
    // A TGA 1.0 file has no slab.
    assert!(extension_area_bytes(&base()).is_none());
}

#[test]
fn metadata_pairs_carry_the_set_text_fields_only() {
    let file = encode_tga_with_extension(&base(), &rich_extension()).unwrap();
    let area = parse_tga_extension_area(&file).unwrap();
    assert_eq!(
        area.metadata_pairs(),
        vec![
            ("author".to_string(), "Ada".to_string()),
            ("comment".to_string(), "first line\nthird line".to_string()),
            ("date".to_string(), "2026-10-17T09:30:05".to_string()),
            ("job".to_string(), "render-42".to_string()),
            ("job_time".to_string(), "01:02:03".to_string()),
            ("software".to_string(), "oxideav".to_string()),
            ("software_version".to_string(), "1.23b".to_string()),
        ]
    );
    // Nothing set, nothing reported.
    let empty = encode_tga_with_extension(&base(), &ExtensionAreaInput::default()).unwrap();
    assert!(
        parse_tga_extension_area(&empty)
            .unwrap()
            .metadata_pairs()
            .is_empty()
    );
    // A version with no letter.
    let v = ExtensionAreaInput {
        software_version: (100, ' '),
        ..ExtensionAreaInput::default()
    };
    let f = encode_tga_with_extension(&base(), &v).unwrap();
    assert_eq!(
        parse_tga_extension_area(&f).unwrap().metadata_pairs(),
        vec![("software_version".to_string(), "1.00".to_string())]
    );
}

#[test]
fn developer_area_portable_form_round_trips_payloads_and_markers() {
    let tags = vec![
        DeveloperTagInput {
            tag_id: 7,
            payload: b"seven".to_vec(),
        },
        DeveloperTagInput {
            tag_id: 9,
            payload: Vec::new(), // marker
        },
        DeveloperTagInput {
            tag_id: 32768,
            payload: vec![1, 2, 3, 4, 5, 6],
        },
    ];
    let ext = ExtensionAreaInput {
        developer_tags: tags.clone(),
        ..ExtensionAreaInput::default()
    };
    let file = encode_tga_with_extension(&base(), &ext).unwrap();
    let dev = parse_tga_developer_area(&file).unwrap();
    assert_eq!(dev.len(), 3);
    let portable = dev.to_portable_bytes(&file);
    // Directory, then the two payloads.
    assert_eq!(portable.len(), 2 + 3 * 10 + 5 + 6);
    assert_eq!(&portable[32..37], b"seven");
    let back = TgaDeveloperArea::tags_from_portable_bytes(&portable).unwrap();
    assert_eq!(back, tags);
    // The portable form is self-contained: re-emitting it produces the
    // same directory contents in a new file.
    let again = encode_tga_with_extension(
        &base(),
        &ExtensionAreaInput {
            developer_tags: back,
            ..ExtensionAreaInput::default()
        },
    )
    .unwrap();
    let dev2 = parse_tga_developer_area(&again).unwrap();
    assert_eq!(dev2.to_portable_bytes(&again), portable);
}

#[test]
fn hostile_portable_developer_areas_are_refused_not_panics() {
    assert!(TgaDeveloperArea::tags_from_portable_bytes(&[]).is_none());
    assert!(TgaDeveloperArea::tags_from_portable_bytes(&[1]).is_none());
    // Count says one record, none present.
    assert!(TgaDeveloperArea::tags_from_portable_bytes(&[1, 0]).is_none());
    // One record pointing past the end.
    let mut b = vec![1, 0, 1, 0];
    b.extend_from_slice(&100u32.to_le_bytes());
    b.extend_from_slice(&4u32.to_le_bytes());
    assert!(TgaDeveloperArea::tags_from_portable_bytes(&b).is_none());
    // Offset + size overflowing usize.
    let mut b = vec![1, 0, 1, 0];
    b.extend_from_slice(&u32::MAX.to_le_bytes());
    b.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(TgaDeveloperArea::tags_from_portable_bytes(&b).is_none());
    // Zero records is a valid, empty directory.
    assert_eq!(
        TgaDeveloperArea::tags_from_portable_bytes(&[0, 0]),
        Some(Vec::new())
    );
}
