//! Round 269 — typed [`TgaDeveloperArea`] / [`TgaDeveloperTag`] accessors.
//!
//! The §C.7 developer area has long been parsed (`parse_tga_developer_area`,
//! `TgaDeveloperArea::parse`, `TgaDeveloperArea::payload`) and written
//! (`ExtensionAreaInput::developer_tags`) but the structs themselves only
//! exposed raw fields. This round gives them the same typed-accessor
//! surface already on `TgaScanLineTable` (r261), `TgaFooter` (r257),
//! `TgaAsciiField` / `TgaAuthorComments` (r252), `TgaTimestamp` /
//! `JobTime` (r234), and the numeric extension-area fields (r227):
//!
//! * [`TgaDeveloperTag::new`] / `as_tuple` / `from_tuple` — construction
//!   + tuple round-trip in on-disk field order (TAG, OFFSET, FIELD SIZE).
//! * [`TgaDeveloperTag::is_developer_use`] /
//!   [`TgaDeveloperTag::is_truevision_reserved`] — spec §C.7 tag-id range
//!   classification ("Values from 0 - 32767 are available for developer
//!   use, while values from 32768 - 65535 are reserved for Truevision").
//! * [`TgaDeveloperTag::is_marker`] — offset-0 / no-payload record shape.
//! * [`TgaDeveloperTag::is_well_formed_within`] — per-record payload
//!   bound, mirroring `TgaDeveloperArea::parse`'s rejection rule.
//! * [`TgaDeveloperTag::to_bytes`] — the 10-byte on-disk record.
//! * [`TgaDeveloperArea::EMPTY`] / `Default` / `new` /
//!   `FromIterator<TgaDeveloperTag>` — sentinel + construction.
//! * [`TgaDeveloperArea::len`] / `is_empty` / `is_unset` /
//!   [`TgaDeveloperArea::directory_byte_size`] — geometry per the spec's
//!   `(NUMBER_OF_TAGS_IN_THE_DIRECTORY * 10) + 2` formula.
//! * [`TgaDeveloperArea::get`] / [`TgaDeveloperArea::find`] /
//!   [`TgaDeveloperArea::contains`] — positional + by-id lookup (spec
//!   §C.7: "The TAGS may appear in any order in the directory").
//! * [`TgaDeveloperArea::is_well_formed_within`] — whole-directory bound.
//! * [`TgaDeveloperArea::to_bytes`] — the on-disk directory, bit-exact
//!   inverse of the directory portion of `TgaDeveloperArea::parse`.
//! * [`TGA_DEVELOPER_TAG_BYTES`] (10) /
//!   [`TGA_DEVELOPER_DIRECTORY_HEADER_BYTES`] (2) — on-disk dimensions.
//!
//! Also pinned: the spec §C.7 tag-id reservation direction (developer
//! use is the LOW range; Truevision-reserved is the HIGH range).
//!
//! No changes to the on-disk wire format, the encoder, or the existing
//! `TgaDeveloperArea::parse` / `payload` / `parse_tga_developer_area`
//! surface — strictly additive.

use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga_developer_area,
    DeveloperTagInput, ExtensionAreaInput, TgaDeveloperArea, TgaDeveloperTag,
    TGA_DEVELOPER_DIRECTORY_HEADER_BYTES, TGA_DEVELOPER_TAG_BYTES,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

#[test]
fn developer_tag_bytes_constant_matches_spec() {
    // Spec §C.7: "each set is 10 bytes in size (1 short, 2 longs)".
    assert_eq!(TGA_DEVELOPER_TAG_BYTES, 10);
}

#[test]
fn developer_directory_header_bytes_constant_matches_spec() {
    // Spec §C.7: "The '+ 2' includes the 2 bytes for the SHORT value
    // specifying the number of tags in the directory."
    assert_eq!(TGA_DEVELOPER_DIRECTORY_HEADER_BYTES, 2);
}

// ---------------------------------------------------------------------------
// TgaDeveloperTag — construction + tuple round-trip
// ---------------------------------------------------------------------------

#[test]
fn tag_new_takes_fields_verbatim() {
    let t = TgaDeveloperTag::new(7, 100, 32);
    assert_eq!(t.tag_id, 7);
    assert_eq!(t.offset, 100);
    assert_eq!(t.size, 32);
}

#[test]
fn tag_tuple_round_trip_in_on_disk_order() {
    // On-disk field order per spec §C.7: TAG, then OFFSET, then
    // FIELD SIZE.
    let t = TgaDeveloperTag::new(513, 0x0102_0304, 0x0A0B_0C0D);
    assert_eq!(t.as_tuple(), (513, 0x0102_0304, 0x0A0B_0C0D));
    assert_eq!(TgaDeveloperTag::from_tuple(t.as_tuple()), t);
}

// ---------------------------------------------------------------------------
// TgaDeveloperTag — spec §C.7 tag-id range classification
// ---------------------------------------------------------------------------

#[test]
fn tag_id_zero_is_developer_use() {
    // Spec §C.7: "Values from 0 - 32767 are available for developer
    // use".
    let t = TgaDeveloperTag::new(0, 0, 0);
    assert!(t.is_developer_use());
    assert!(!t.is_truevision_reserved());
}

#[test]
fn tag_id_32767_is_developer_use_upper_bound() {
    let t = TgaDeveloperTag::new(32767, 0, 0);
    assert!(t.is_developer_use());
    assert!(!t.is_truevision_reserved());
}

#[test]
fn tag_id_32768_is_truevision_reserved_lower_bound() {
    // Spec §C.7: "values from 32768 - 65535 are reserved for
    // Truevision".
    let t = TgaDeveloperTag::new(32768, 0, 0);
    assert!(!t.is_developer_use());
    assert!(t.is_truevision_reserved());
}

#[test]
fn tag_id_65535_is_truevision_reserved() {
    let t = TgaDeveloperTag::new(65535, 0, 0);
    assert!(!t.is_developer_use());
    assert!(t.is_truevision_reserved());
}

#[test]
fn every_tag_id_is_in_exactly_one_range() {
    // The two §C.7 ranges partition the full SHORT domain.
    for id in [0u16, 1, 1000, 32766, 32767, 32768, 32769, 65534, 65535] {
        let t = TgaDeveloperTag::new(id, 0, 0);
        assert_ne!(
            t.is_developer_use(),
            t.is_truevision_reserved(),
            "tag id {id} must be in exactly one range"
        );
    }
}

// ---------------------------------------------------------------------------
// TgaDeveloperTag — marker + well-formedness
// ---------------------------------------------------------------------------

#[test]
fn offset_zero_is_marker() {
    assert!(TgaDeveloperTag::new(1, 0, 0).is_marker());
    // A marker keeps its shape even with a (meaningless) size.
    assert!(TgaDeveloperTag::new(1, 0, 99).is_marker());
}

#[test]
fn non_zero_offset_is_not_marker() {
    assert!(!TgaDeveloperTag::new(1, 18, 4).is_marker());
}

#[test]
fn marker_tag_is_trivially_well_formed() {
    assert!(TgaDeveloperTag::new(1, 0, 0).is_well_formed_within(0));
    assert!(TgaDeveloperTag::new(1, 0, u32::MAX).is_well_formed_within(1));
}

#[test]
fn payload_inside_buffer_is_well_formed() {
    // [100, 132) inside a 132-byte buffer: exactly fits.
    let t = TgaDeveloperTag::new(1, 100, 32);
    assert!(t.is_well_formed_within(132));
    assert!(t.is_well_formed_within(200));
}

#[test]
fn payload_past_buffer_end_is_rejected() {
    let t = TgaDeveloperTag::new(1, 100, 33);
    assert!(!t.is_well_formed_within(132));
}

#[test]
fn payload_at_field_extremes_is_rejected_for_real_buffers() {
    // offset + size is computed without wrapping, so the extreme
    // (u32::MAX, u32::MAX) record can never pass against any buffer
    // a u32 offset could address.
    let t = TgaDeveloperTag::new(1, u32::MAX, u32::MAX);
    assert!(!t.is_well_formed_within(u32::MAX as usize));
}

// ---------------------------------------------------------------------------
// TgaDeveloperTag — on-disk record serialisation
// ---------------------------------------------------------------------------

#[test]
fn tag_to_bytes_is_little_endian_tag_offset_size() {
    // Spec §C.7 Figure 2 - Developer Directory: SHORT tag, LONG
    // offset, LONG size — all little-endian.
    let t = TgaDeveloperTag::new(0x0201, 0x0605_0403, 0x0A09_0807);
    assert_eq!(
        t.to_bytes(),
        [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A]
    );
}

#[test]
fn tag_to_bytes_is_exactly_ten_bytes() {
    assert_eq!(
        TgaDeveloperTag::new(0, 0, 0).to_bytes().len(),
        TGA_DEVELOPER_TAG_BYTES
    );
}

// ---------------------------------------------------------------------------
// TgaDeveloperArea — sentinel / Default / construction
// ---------------------------------------------------------------------------

#[test]
fn empty_sentinel_has_zero_tags() {
    let d = TgaDeveloperArea::EMPTY;
    assert_eq!(d.len(), 0);
    assert!(d.is_empty());
    assert!(d.is_unset());
    assert_eq!(
        d.directory_byte_size(),
        TGA_DEVELOPER_DIRECTORY_HEADER_BYTES
    );
}

#[test]
fn default_matches_empty_sentinel() {
    let d = TgaDeveloperArea::default();
    assert_eq!(d, TgaDeveloperArea::EMPTY);
    assert!(d.is_unset());
}

#[test]
fn new_takes_tags_verbatim() {
    let tags = vec![
        TgaDeveloperTag::new(1, 18, 4),
        TgaDeveloperTag::new(2, 0, 0),
    ];
    let d = TgaDeveloperArea::new(tags.clone());
    assert_eq!(d.tags, tags);
    assert_eq!(d.len(), 2);
    assert!(!d.is_empty());
    assert!(!d.is_unset());
}

#[test]
fn from_iterator_collects_tags_in_order() {
    let d: TgaDeveloperArea = (1u16..=3)
        .map(|i| TgaDeveloperTag::new(i, u32::from(i) * 10, 5))
        .collect();
    assert_eq!(d.len(), 3);
    assert_eq!(d.tags[0].tag_id, 1);
    assert_eq!(d.tags[2].offset, 30);
}

// ---------------------------------------------------------------------------
// TgaDeveloperArea — geometry / lookup
// ---------------------------------------------------------------------------

#[test]
fn directory_byte_size_matches_spec_formula() {
    // Spec §C.7: "(NUMBER_OF_TAGS_IN_THE_DIRECTORY * 10) + 2".
    let d: TgaDeveloperArea = (0u16..3).map(|i| TgaDeveloperTag::new(i, 0, 0)).collect();
    assert_eq!(d.directory_byte_size(), 3 * 10 + 2);
    assert_eq!(d.directory_byte_size(), d.to_bytes().len());
}

#[test]
fn get_is_bounds_checked() {
    let d = TgaDeveloperArea::new(vec![TgaDeveloperTag::new(9, 18, 1)]);
    assert_eq!(d.get(0), Some(&TgaDeveloperTag::new(9, 18, 1)));
    assert_eq!(d.get(1), None);
    assert_eq!(TgaDeveloperArea::EMPTY.get(0), None);
}

#[test]
fn find_returns_first_match_in_directory_order() {
    // Spec §C.7: "The TAGS may appear in any order in the directory
    // (i.e., they do not need to be sorted)" — so lookup is by id,
    // and a duplicated id resolves to the earliest record.
    let d = TgaDeveloperArea::new(vec![
        TgaDeveloperTag::new(7, 100, 1),
        TgaDeveloperTag::new(3, 200, 2),
        TgaDeveloperTag::new(7, 300, 3),
    ]);
    assert_eq!(d.find(3), Some(&TgaDeveloperTag::new(3, 200, 2)));
    assert_eq!(d.find(7), Some(&TgaDeveloperTag::new(7, 100, 1)));
    assert_eq!(d.find(8), None);
}

#[test]
fn contains_reports_presence() {
    let d = TgaDeveloperArea::new(vec![TgaDeveloperTag::new(42, 0, 0)]);
    assert!(d.contains(42));
    assert!(!d.contains(43));
    assert!(!TgaDeveloperArea::EMPTY.contains(42));
}

// ---------------------------------------------------------------------------
// TgaDeveloperArea — well-formedness
// ---------------------------------------------------------------------------

#[test]
fn empty_directory_is_trivially_well_formed() {
    assert!(TgaDeveloperArea::EMPTY.is_well_formed_within(0));
}

#[test]
fn directory_well_formed_when_every_payload_fits() {
    let d = TgaDeveloperArea::new(vec![
        TgaDeveloperTag::new(1, 18, 10),
        TgaDeveloperTag::new(2, 0, 0), // marker — trivially OK
        TgaDeveloperTag::new(3, 28, 4),
    ]);
    assert!(d.is_well_formed_within(32));
}

#[test]
fn directory_rejected_when_any_payload_overruns() {
    let d = TgaDeveloperArea::new(vec![
        TgaDeveloperTag::new(1, 18, 10),
        TgaDeveloperTag::new(3, 28, 5), // [28, 33) past a 32-byte buffer
    ]);
    assert!(!d.is_well_formed_within(32));
}

// ---------------------------------------------------------------------------
// TgaDeveloperArea — on-disk directory serialisation + parse round-trip
// ---------------------------------------------------------------------------

#[test]
fn to_bytes_emits_count_then_records() {
    let d = TgaDeveloperArea::new(vec![
        TgaDeveloperTag::new(0x0201, 0x0605_0403, 0x0A09_0807),
        TgaDeveloperTag::new(0xBBAA, 0, 0),
    ]);
    let bytes = d.to_bytes();
    assert_eq!(bytes.len(), 2 + 2 * 10);
    // Leading SHORT count, little-endian.
    assert_eq!(&bytes[0..2], &[0x02, 0x00]);
    // First record.
    assert_eq!(
        &bytes[2..12],
        &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A]
    );
    // Second record (marker).
    assert_eq!(&bytes[12..22], &[0xAA, 0xBB, 0, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn empty_directory_to_bytes_is_count_only() {
    assert_eq!(TgaDeveloperArea::EMPTY.to_bytes(), vec![0x00, 0x00]);
}

#[test]
fn to_bytes_round_trips_through_parse() {
    // Lay a 4-byte payload + the directory into a buffer the way a
    // real file would (payload first, directory after), then parse
    // the directory back from its offset.
    let payload_off = 8u32; // arbitrary non-zero spot inside the buffer
    let d = TgaDeveloperArea::new(vec![
        TgaDeveloperTag::new(7, payload_off, 4),
        TgaDeveloperTag::new(40000, 0, 0),
    ]);
    let mut buf = vec![0u8; 12]; // room for the 4 payload bytes at [8, 12)
    buf[8..12].copy_from_slice(b"DATA");
    let dir_off = buf.len() as u32;
    buf.extend_from_slice(&d.to_bytes());

    let parsed = TgaDeveloperArea::parse(&buf, dir_off).expect("directory parses");
    assert_eq!(parsed, d);
    assert_eq!(
        parsed.payload(&buf, parsed.find(7).unwrap()),
        Some(&b"DATA"[..])
    );
    assert_eq!(parsed.payload(&buf, parsed.find(40000).unwrap()), None);
    assert!(parsed.is_well_formed_within(buf.len()));
}

// ---------------------------------------------------------------------------
// End-to-end: encoder-written developer area through the typed surface
// ---------------------------------------------------------------------------

#[test]
fn encoded_developer_area_supports_typed_lookup() {
    // 2×2 opaque RGBA → type-2 base, then attach two developer tags
    // (one in each §C.7 id range) + one marker.
    let rgba: Vec<u8> = (0..2 * 2 * 4).map(|i| (i * 7) as u8).collect();
    let mut rgba = rgba;
    for px in rgba.chunks_exact_mut(4) {
        px[3] = 0xFF;
    }
    let base = encode_tga_uncompressed(2, 2, &rgba).expect("base encodes");
    let ext = ExtensionAreaInput {
        developer_tags: vec![
            DeveloperTagInput {
                tag_id: 100, // developer-use range
                payload: b"hello".to_vec(),
            },
            DeveloperTagInput {
                tag_id: 40000, // Truevision-reserved range
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
    assert_eq!(dev.len(), 3);
    assert!(!dev.is_unset());
    assert!(dev.is_well_formed_within(file.len()));
    assert_eq!(dev.directory_byte_size(), 3 * 10 + 2);

    let t100 = dev.find(100).expect("tag 100 present");
    assert!(t100.is_developer_use());
    assert!(!t100.is_marker());
    assert_eq!(dev.payload(&file, t100), Some(&b"hello"[..]));

    let t40000 = dev.find(40000).expect("tag 40000 present");
    assert!(t40000.is_truevision_reserved());
    assert_eq!(dev.payload(&file, t40000), Some(&b"world!"[..]));

    let t200 = dev.find(200).expect("tag 200 present");
    assert!(t200.is_marker());
    assert!(t200.is_well_formed_within(file.len()));
    assert_eq!(dev.payload(&file, t200), None);

    assert!(!dev.contains(101));
    assert_eq!(dev.get(0).unwrap().tag_id, 100);
    assert_eq!(dev.get(3), None);

    // The directory bytes the encoder wrote are bit-exact with the
    // typed serialiser's output for the parsed directory.
    let footer = oxideav_tga::parse_tga_footer(&file).expect("v2 footer");
    let dir_off = footer.developer_directory_offset as usize;
    let dir_len = dev.directory_byte_size();
    assert_eq!(&file[dir_off..dir_off + dir_len], &dev.to_bytes()[..]);
}
