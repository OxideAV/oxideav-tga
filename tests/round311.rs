//! Round 311 — §C.2 Image Descriptor attribute-bit count (Field 5.6,
//! bits 3-0): typed [`AttributeBits`] view + the header-local alpha
//! resolver [`resolve_alpha_from_descriptor`].
//!
//! The image-descriptor byte's low four bits "specify the number of
//! attribute bits per pixel … designated as Alpha Channel bits". The spec
//! ties this to the extension-area Attributes Type (Field 24): a value of
//! `0` there ("no Alpha data included") means "bits 3-0 of field 5.6
//! should also be set to zero". A header that declares zero attribute
//! bits is therefore the header-local (and TGA-1.0-compatible) signal
//! that a 32-bpp pixel's fourth byte carries no meaningful alpha and the
//! image should display opaque — the descriptor-only counterpart to the
//! extension-area `AttributesType` path and the all-alpha-zero heuristic.
//!
//! The crate already exposed `TgaHeader::alpha_bits()` (the raw count) but
//! never *applied* it at decode time and offered no typed view. This round
//! adds `AttributeBits` (typed view + spec predicates),
//! `TgaHeader::attribute_bits()`, the public `parse_tga_attribute_bits`
//! reader, and the `resolve_alpha_from_descriptor` apply-side resolver.

use oxideav_tga::{
    parse_tga, parse_tga_attribute_bits, resolve_alpha_from_descriptor, AttributeBits,
    TgaPixelFormat, TGA_ATTRIBUTE_BITS_MAX,
};

/// Build a minimal 1×1 uncompressed 32-bpp true-colour (type 2) TGA with
/// a caller-chosen descriptor byte and a single BGRA pixel. The pixel is
/// `(B, G, R, A)` on disk.
fn build_bgra_1px(descriptor: u8, bgra: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(0); // id_length
    bytes.push(0); // cmap_type = 0 (no color map)
    bytes.push(2); // image type = 2 (uncompressed true-colour)
    bytes.extend_from_slice(&0u16.to_le_bytes()); // cmap first
    bytes.extend_from_slice(&0u16.to_le_bytes()); // cmap len
    bytes.push(0); // cmap entry size
    bytes.extend_from_slice(&0u16.to_le_bytes()); // x origin
    bytes.extend_from_slice(&0u16.to_le_bytes()); // y origin
    bytes.extend_from_slice(&1u16.to_le_bytes()); // width
    bytes.extend_from_slice(&1u16.to_le_bytes()); // height
    bytes.push(32); // depth
    bytes.push(descriptor); // image descriptor
    bytes.extend_from_slice(&bgra); // one BGRA pixel
    bytes
}

#[test]
fn attribute_bits_extracted_from_descriptor() {
    // Bit 5 set (top-down) + 8 attribute bits => low nibble 0x08.
    let bits = AttributeBits::from_descriptor(0x28);
    assert_eq!(bits.count, 8);
    assert!(bits.has_alpha());
    assert!(!bits.is_none());

    // Bit 5 set only, no attribute bits => low nibble 0.
    let none = AttributeBits::from_descriptor(0x20);
    assert_eq!(none.count, 0);
    assert!(none.is_none());
    assert!(!none.has_alpha());
    assert_eq!(none, AttributeBits::NONE);
}

#[test]
fn attribute_bits_new_masks_to_four_bits() {
    // 0xFF must clamp to the four-bit field width.
    assert_eq!(AttributeBits::new(0xFF).count, TGA_ATTRIBUTE_BITS_MAX);
    assert_eq!(AttributeBits::new(8).count, 8);
    assert_eq!(AttributeBits::new(0), AttributeBits::NONE);
}

#[test]
fn canonical_for_depth_matches_spec_depths() {
    // 32 bpp => 8 alpha bits is canonical; 16 bpp => 1; alpha-less depths => 0.
    assert!(AttributeBits::new(8).is_canonical_for_depth(32));
    assert!(AttributeBits::new(1).is_canonical_for_depth(16));
    // A zero count is always consistent ("no meaningful alpha").
    assert!(AttributeBits::NONE.is_canonical_for_depth(32));
    assert!(AttributeBits::NONE.is_canonical_for_depth(24));
    assert!(AttributeBits::NONE.is_canonical_for_depth(16));
    // Non-canonical mismatches.
    assert!(!AttributeBits::new(4).is_canonical_for_depth(32));
    assert!(!AttributeBits::new(8).is_canonical_for_depth(16));
    assert!(!AttributeBits::new(1).is_canonical_for_depth(24));
}

#[test]
fn parse_attribute_bits_reads_header() {
    let file = build_bgra_1px(0x28, [0x10, 0x20, 0x30, 0x40]);
    let bits = parse_tga_attribute_bits(&file).expect("header present");
    assert_eq!(bits.count, 8);
    assert!(bits.has_alpha());

    let file_none = build_bgra_1px(0x20, [0x10, 0x20, 0x30, 0x40]);
    assert!(parse_tga_attribute_bits(&file_none)
        .expect("header present")
        .is_none());
}

#[test]
fn parse_attribute_bits_rejects_truncated_header() {
    assert!(parse_tga_attribute_bits(&[0u8; 4]).is_none());
}

#[test]
fn header_typed_accessor_agrees_with_raw() {
    use oxideav_tga::parse_header;
    let file = build_bgra_1px(0x29, [1, 2, 3, 4]); // 9 attribute bits (non-canonical)
    let h = parse_header(&file).expect("header");
    assert_eq!(h.alpha_bits(), 9);
    assert_eq!(h.attribute_bits().count, 9);
    assert!(!h.attribute_bits().is_canonical_for_depth(32));
}

#[test]
fn resolver_forces_opaque_when_no_attribute_bits() {
    // 32-bpp file declaring 0 attribute bits, on-disk alpha byte 0x00.
    // Decode keeps the raw alpha (0), then the descriptor resolver forces
    // it opaque because the header declares no meaningful alpha.
    let file = build_bgra_1px(0x20, [0x10, 0x20, 0x30, 0x00]);
    let mut img = parse_tga(&file).expect("decode");
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    assert_eq!(img.data[3], 0x00, "raw decode preserves the on-disk alpha");

    let bits = resolve_alpha_from_descriptor(&file, &mut img).expect("header");
    assert!(bits.is_none());
    assert_eq!(img.data[3], 0xFF, "no attribute bits => forced opaque");
    // Colour channels (BGR -> RGB) untouched: B=0x10 G=0x20 R=0x30.
    assert_eq!(&img.data[0..3], &[0x30, 0x20, 0x10]);
}

#[test]
fn resolver_honours_declared_alpha() {
    // 32-bpp file declaring 8 attribute bits, on-disk alpha 0x40 — the
    // resolver leaves the decoded alpha untouched.
    let file = build_bgra_1px(0x28, [0x10, 0x20, 0x30, 0x40]);
    let mut img = parse_tga(&file).expect("decode");
    let bits = resolve_alpha_from_descriptor(&file, &mut img).expect("header");
    assert_eq!(bits.count, 8);
    assert!(bits.has_alpha());
    assert_eq!(img.data[3], 0x40, "declared alpha is preserved");
}

#[test]
fn resolver_acts_even_when_alpha_not_all_zero() {
    // Distinguishes the descriptor resolver from the all-alpha-zero
    // heuristic: here the alpha byte is non-zero (0x7F) but the header
    // declares zero attribute bits, so the descriptor resolver still
    // forces opaque (the heuristic would not have fired).
    let file = build_bgra_1px(0x20, [0x10, 0x20, 0x30, 0x7F]);
    let mut img = parse_tga(&file).expect("decode");
    assert_eq!(img.data[3], 0x7F);
    let bits = resolve_alpha_from_descriptor(&file, &mut img).expect("header");
    assert!(bits.is_none());
    assert_eq!(img.data[3], 0xFF);
}

#[test]
fn resolver_noop_on_truncated_header() {
    let mut img = parse_tga(&build_bgra_1px(0x20, [1, 2, 3, 0])).expect("decode");
    img.data[3] = 0x00; // sentinel
                        // A buffer too short to hold a header yields None and no mutation.
    assert!(resolve_alpha_from_descriptor(&[0u8; 4], &mut img).is_none());
    assert_eq!(img.data[3], 0x00, "no-op when header can't be read");
}
