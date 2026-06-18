//! Round 332 — one-call §C.7 developer-payload lookup by tag id.
//!
//! The developer area's tag directory could be parsed
//! (`parse_tga_developer_area`), looked up by id (`TgaDeveloperArea::find`
//! / `contains`, r269), and its payload borrowed by record
//! (`TgaDeveloperArea::payload`). A caller that knows only the tag id —
//! not its directory position — still had to compose the two by hand:
//! `area.find(id).and_then(|t| area.payload(input, t))`. Spec §C.7 lets
//! the directory be unsorted ("The TAGS may appear in any order in the
//! directory"), so by-id retrieval is the natural caller-facing
//! primitive; this round adds the missing one-call composition.
//!
//! * `TgaDeveloperArea::payload_by_id(input, tag_id)` — the first record
//!   carrying `tag_id`'s payload bytes, or `None` for a missing id, a
//!   marker record (offset 0, no payload), or an out-of-buffer range.

use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga_developer_area,
    DeveloperTagInput, ExtensionAreaInput, TgaDeveloperArea, TgaDeveloperTag,
};

#[test]
fn payload_by_id_matches_manual_find_then_payload() {
    // payload first, directory after — the on-disk layout a real file uses.
    let payload_off = 8u32;
    let d = TgaDeveloperArea::new(vec![
        TgaDeveloperTag::new(7, payload_off, 4),
        TgaDeveloperTag::new(40000, 0, 0), // marker (no payload)
    ]);
    let mut buf = vec![0u8; 12];
    buf[8..12].copy_from_slice(b"DATA");
    let dir_off = buf.len() as u32;
    buf.extend_from_slice(&d.to_bytes());

    let parsed = TgaDeveloperArea::parse(&buf, dir_off).expect("directory parses");

    // One-call lookup == the manual find()+payload() composition.
    assert_eq!(parsed.payload_by_id(&buf, 7), Some(&b"DATA"[..]));
    assert_eq!(
        parsed.payload_by_id(&buf, 7),
        parsed.payload(&buf, parsed.find(7).unwrap())
    );

    // Marker record: present in the directory but yields no payload.
    assert!(parsed.contains(40000));
    assert_eq!(parsed.payload_by_id(&buf, 40000), None);

    // Absent id: None (and never panics).
    assert!(!parsed.contains(999));
    assert_eq!(parsed.payload_by_id(&buf, 999), None);
}

#[test]
fn payload_by_id_takes_first_match_in_directory_order() {
    // Two records share id 5; find() (and thus payload_by_id) take the
    // first in directory order. Two distinct 3-byte payloads at [8,11)
    // and [11,14).
    let d = TgaDeveloperArea::new(vec![
        TgaDeveloperTag::new(5, 8, 3),
        TgaDeveloperTag::new(5, 11, 3),
    ]);
    let mut buf = vec![0u8; 14];
    buf[8..11].copy_from_slice(b"AAA");
    buf[11..14].copy_from_slice(b"BBB");
    let dir_off = buf.len() as u32;
    buf.extend_from_slice(&d.to_bytes());

    let parsed = TgaDeveloperArea::parse(&buf, dir_off).expect("parses");
    assert_eq!(parsed.payload_by_id(&buf, 5), Some(&b"AAA"[..]));
}

#[test]
fn empty_directory_payload_by_id_is_none() {
    assert_eq!(TgaDeveloperArea::EMPTY.payload_by_id(&[], 0), None);
    assert_eq!(
        TgaDeveloperArea::EMPTY.payload_by_id(b"anything", 123),
        None
    );
}

#[test]
fn encoded_developer_area_one_call_lookup() {
    // 2×2 opaque RGBA → type-2 base, attach two payload tags + one marker.
    let mut rgba: Vec<u8> = (0..2 * 2 * 4).map(|i| (i * 7) as u8).collect();
    for px in rgba.chunks_exact_mut(4) {
        px[3] = 0xFF;
    }
    let base = encode_tga_uncompressed(2, 2, &rgba).expect("base encodes");
    let ext = ExtensionAreaInput {
        developer_tags: vec![
            DeveloperTagInput {
                tag_id: 100,
                payload: b"hello".to_vec(),
            },
            DeveloperTagInput {
                tag_id: 40000,
                payload: b"world!".to_vec(),
            },
            DeveloperTagInput {
                tag_id: 200,
                payload: Vec::new(), // marker
            },
        ],
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).expect("extension encodes");

    let dev = parse_tga_developer_area(&file).expect("developer area present");

    // Both payload tags resolve in a single call; the marker yields None.
    assert_eq!(dev.payload_by_id(&file, 100), Some(&b"hello"[..]));
    assert_eq!(dev.payload_by_id(&file, 40000), Some(&b"world!"[..]));
    assert_eq!(dev.payload_by_id(&file, 200), None); // marker
    assert_eq!(dev.payload_by_id(&file, 101), None); // absent

    // Equivalent to the explicit find()+payload() pair for every id.
    for id in [100u16, 40000, 200] {
        let manual = dev.find(id).and_then(|t| dev.payload(&file, t));
        assert_eq!(dev.payload_by_id(&file, id), manual);
    }
}
