//! Round 316 — §C.2 Image Descriptor data-storage interleaving flag
//! (Field 5.6, bits 7-6): typed [`Interleaving`] view + the one-call
//! [`parse_tga_interleaving`] header reader.
//!
//! The image-descriptor byte's top two bits carry the data-storage
//! interleaving flag. The two source documents differ on its meaning, and
//! the view surfaces both:
//!
//! * The **TGA 2.0 File Format Specification** (the PDF, this crate's
//!   primary source) states bits 7 & 6 "Must be zero to insure future
//!   compatibility" — TGA 2.0 abandoned interleaving.
//! * The earlier Truevision header layout (Gamers.org plain-text mirror)
//!   defined the field as `00` non-interleaved, `01` two-way (even/odd),
//!   `10` four-way, `11` reserved.
//!
//! This crate surfaces and classifies the field but does not de-interleave
//! (the 2.0 FFS removed it and never documents the reorder, and no
//! in-the-wild fixtures set a non-zero value). The view lets a caller
//! recognise — and choose to reject — a legacy interleaved file. Mirrors
//! the surface-but-don't-guess approach taken for `AttributeBits`
//! (round 311).

use oxideav_tga::{parse_tga, parse_tga_interleaving, Interleaving, TGA_INTERLEAVING_MASK};

/// Build a minimal 1×1 uncompressed 24-bpp true-colour (type 2) TGA with
/// a caller-chosen descriptor byte and one BGR pixel.
fn build_bgr_1px(descriptor: u8, bgr: [u8; 3]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(0); // id_length
    bytes.push(0); // cmap_type = 0
    bytes.push(2); // image type 2 (uncompressed true-colour)
    bytes.extend_from_slice(&0u16.to_le_bytes()); // cmap first
    bytes.extend_from_slice(&0u16.to_le_bytes()); // cmap len
    bytes.push(0); // cmap entry size
    bytes.extend_from_slice(&0u16.to_le_bytes()); // x origin
    bytes.extend_from_slice(&0u16.to_le_bytes()); // y origin
    bytes.extend_from_slice(&1u16.to_le_bytes()); // width
    bytes.extend_from_slice(&1u16.to_le_bytes()); // height
    bytes.push(24); // depth
    bytes.push(descriptor); // image descriptor
    bytes.extend_from_slice(&bgr); // one BGR pixel
    bytes
}

#[test]
fn interleaving_extracted_from_descriptor() {
    // 0x00..=0xC0 sweep over bits 7-6, low bits arbitrary.
    assert_eq!(
        Interleaving::from_descriptor(0x20),
        Interleaving::NonInterleaved
    );
    assert_eq!(Interleaving::from_descriptor(0x40), Interleaving::TwoWay);
    assert_eq!(Interleaving::from_descriptor(0x80), Interleaving::FourWay);
    assert_eq!(Interleaving::from_descriptor(0xC0), Interleaving::Reserved);
}

#[test]
fn interleaving_ignores_low_bits() {
    // Low six bits (attribute count, origin bits) must not leak into the
    // interleaving classification.
    assert_eq!(
        Interleaving::from_descriptor(0x3F),
        Interleaving::NonInterleaved
    );
    assert_eq!(Interleaving::from_descriptor(0x7F), Interleaving::TwoWay);
    assert_eq!(Interleaving::from_descriptor(0xBF), Interleaving::FourWay);
    assert_eq!(Interleaving::from_descriptor(0xFF), Interleaving::Reserved);
}

#[test]
fn interleaving_default_is_non_interleaved() {
    assert_eq!(Interleaving::default(), Interleaving::NonInterleaved);
    assert!(Interleaving::default().is_non_interleaved());
    assert!(Interleaving::default().is_tga2_compliant());
}

#[test]
fn interleaving_raw_values() {
    assert_eq!(Interleaving::NonInterleaved.raw(), 0);
    assert_eq!(Interleaving::TwoWay.raw(), 1);
    assert_eq!(Interleaving::FourWay.raw(), 2);
    assert_eq!(Interleaving::Reserved.raw(), 3);
}

#[test]
fn interleaving_to_descriptor_bits_round_trips() {
    for v in [
        Interleaving::NonInterleaved,
        Interleaving::TwoWay,
        Interleaving::FourWay,
        Interleaving::Reserved,
    ] {
        let bits = v.to_descriptor_bits();
        // Only bits 7-6 are populated.
        assert_eq!(bits & !TGA_INTERLEAVING_MASK, 0);
        // And re-extracting recovers the same variant, regardless of any
        // low-bit content OR'd alongside it.
        assert_eq!(Interleaving::from_descriptor(bits), v);
        assert_eq!(Interleaving::from_descriptor(bits | 0x2A), v);
    }
}

#[test]
fn interleaving_predicates() {
    assert!(Interleaving::NonInterleaved.is_non_interleaved());
    assert!(!Interleaving::NonInterleaved.is_two_way());
    assert!(!Interleaving::NonInterleaved.is_four_way());
    assert!(!Interleaving::NonInterleaved.is_reserved());

    assert!(Interleaving::TwoWay.is_two_way());
    assert!(Interleaving::FourWay.is_four_way());
    assert!(Interleaving::Reserved.is_reserved());

    // Only the 2.0-conformant zero value is "compliant".
    assert!(Interleaving::NonInterleaved.is_tga2_compliant());
    assert!(!Interleaving::TwoWay.is_tga2_compliant());
    assert!(!Interleaving::FourWay.is_tga2_compliant());
    assert!(!Interleaving::Reserved.is_tga2_compliant());
}

#[test]
fn interleaving_mask_constant() {
    assert_eq!(TGA_INTERLEAVING_MASK, 0xC0);
}

#[test]
fn parse_tga_interleaving_reads_header() {
    // Top-down (bit 5) + non-interleaved.
    let f = build_bgr_1px(0x20, [1, 2, 3]);
    assert_eq!(
        parse_tga_interleaving(&f),
        Some(Interleaving::NonInterleaved)
    );

    // Two-way interleaving flag set alongside top-down.
    let f = build_bgr_1px(0x60, [1, 2, 3]);
    assert_eq!(parse_tga_interleaving(&f), Some(Interleaving::TwoWay));

    // Four-way.
    let f = build_bgr_1px(0xA0, [1, 2, 3]);
    assert_eq!(parse_tga_interleaving(&f), Some(Interleaving::FourWay));

    // Reserved.
    let f = build_bgr_1px(0xE0, [1, 2, 3]);
    assert_eq!(parse_tga_interleaving(&f), Some(Interleaving::Reserved));
}

#[test]
fn parse_tga_interleaving_none_on_truncated() {
    // Anything shorter than the 18-byte header yields None.
    assert_eq!(parse_tga_interleaving(&[]), None);
    assert_eq!(parse_tga_interleaving(&[0u8; 17]), None);
    // Exactly 18 bytes is enough to read the descriptor.
    assert!(parse_tga_interleaving(&[0u8; 18]).is_some());
}

#[test]
fn header_interleaving_matches_helper() {
    // The TgaHeader::interleaving() accessor and the standalone reader
    // agree, and decode still succeeds (the field is informational; the
    // decoder treats the raster as non-interleaved regardless).
    let f = build_bgr_1px(0x60, [10, 20, 30]);
    let via_reader = parse_tga_interleaving(&f).unwrap();
    assert_eq!(via_reader, Interleaving::TwoWay);

    // The non-zero interleaving flag does not break decode: a 1×1 raster
    // decodes to the single RGBA pixel (BGR 10,20,30 -> R30 G20 B10).
    let img = parse_tga(&f).expect("decode succeeds despite interleaving flag");
    assert_eq!(img.width, 1);
    assert_eq!(img.height, 1);
    assert_eq!(&img.data[0..3], &[30, 20, 10]);
}
