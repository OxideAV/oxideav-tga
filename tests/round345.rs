//! Round 345 — §C.2 Color Map Type typed view + the TIPS border-colour
//! feature.
//!
//! The fixed-header **Color Map Type** byte (offset 1) was carried only
//! as the raw `TgaHeader::cmap_type: u8`. The spec constrains it to two
//! values — *"0 means no color map is included. 1 means a color map is
//! included, but since this is an unmapped image it is usually ignored.
//! TIPS (a Targa paint system) will set the border color [to] the first
//! map color if it is present."*
//!
//! This round adds:
//! * `ColorMapType` typed view (`Absent` / `Present` / `Reserved(u8)`)
//!   reachable via `TgaHeader::color_map_type()` and the one-call
//!   `parse_tga_color_map_type`, mirroring `Interleaving` /
//!   `AttributeBits`.
//! * `parse_tga_border_color` — the apply-side of the TIPS reading: the
//!   first colour-map entry of an *unmapped* image that carries a present
//!   map, surfaced as straight RGBA. Returns `None` for a colour-mapped
//!   image type (there the map is the working palette, not a border
//!   colour) and for a map-absent file.

use oxideav_tga::{
    encode_tga_palette, encode_tga_uncompressed, parse_header, parse_tga_border_color,
    parse_tga_color_map, parse_tga_color_map_type, ColorMapType,
};

// Tiny local helper so the test exercises the `TgaHeader::color_map_type()`
// accessor directly (the public re-export path), not just the one-call
// parser.
fn parse_tga_header_color_map_type_via_header(input: &[u8]) -> Option<ColorMapType> {
    parse_header(input).map(|h| h.color_map_type())
}

/// Build a true-colour (type 2, 24 bpp) TGA whose header declares a
/// **present** colour map (`cmap_type = 1`) carrying `entries` 24-bit
/// BGR border-colour entries, followed by `width × height` BGR pixels.
///
/// This is the TIPS-border layout: an unmapped image type that
/// nonetheless stores a vestigial colour map. The crate's encoder never
/// writes such a file, so we lay the bytes down by hand per the §C.2
/// header layout.
fn truecolour_with_border_map(
    width: u16,
    height: u16,
    map_bgr: &[[u8; 3]],
    pixels_bgr: &[[u8; 3]],
) -> Vec<u8> {
    assert_eq!(pixels_bgr.len(), width as usize * height as usize);
    let mut out = Vec::new();
    out.push(0); // id_length
    out.push(1); // cmap_type = 1 (present)
    out.push(2); // image_type = 2 (uncompressed true-colour)
    out.extend_from_slice(&0u16.to_le_bytes()); // cmap_first
    out.extend_from_slice(&(map_bgr.len() as u16).to_le_bytes()); // cmap_length
    out.push(24); // cmap_entry_size (24-bit BGR entries)
    out.extend_from_slice(&0u16.to_le_bytes()); // x_origin
    out.extend_from_slice(&0u16.to_le_bytes()); // y_origin
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.push(24); // pixel depth
    out.push(0x20); // descriptor: top-down
    for bgr in map_bgr {
        out.extend_from_slice(bgr);
    }
    for bgr in pixels_bgr {
        out.extend_from_slice(bgr);
    }
    out
}

#[test]
fn color_map_type_classifies_absent_present_reserved() {
    assert_eq!(ColorMapType::from_u8(0), ColorMapType::Absent);
    assert_eq!(ColorMapType::from_u8(1), ColorMapType::Present);
    assert_eq!(ColorMapType::from_u8(2), ColorMapType::Reserved(2));
    assert_eq!(ColorMapType::from_u8(255), ColorMapType::Reserved(255));

    assert!(ColorMapType::Absent.is_absent());
    assert!(!ColorMapType::Absent.has_map_data());
    assert!(ColorMapType::Absent.is_conformant());

    assert!(ColorMapType::Present.is_present());
    assert!(ColorMapType::Present.has_map_data());
    assert!(ColorMapType::Present.is_conformant());
    assert!(!ColorMapType::Present.is_reserved());

    let r = ColorMapType::Reserved(7);
    assert!(r.is_reserved());
    assert!(!r.is_conformant());
    // The decoder's lenient "non-zero means map data follows" reading:
    assert!(r.has_map_data());
    assert!(!r.is_present());
}

#[test]
fn color_map_type_round_trips_to_u8() {
    for byte in 0u8..=255 {
        assert_eq!(ColorMapType::from_u8(byte).to_u8(), byte);
    }
}

#[test]
fn color_map_type_default_is_absent() {
    assert_eq!(ColorMapType::default(), ColorMapType::Absent);
}

#[test]
fn parse_color_map_type_reads_header_byte() {
    // True-colour file written by the encoder declares cmap_type = 0.
    let pixels = vec![10u8, 20, 30, 255, 40, 50, 60, 255];
    let bytes = encode_tga_uncompressed(2, 1, &pixels).unwrap();
    assert_eq!(
        parse_tga_color_map_type(&bytes).unwrap(),
        ColorMapType::Absent
    );
    assert_eq!(
        parse_tga_header_color_map_type_via_header(&bytes),
        Some(ColorMapType::Absent)
    );

    // Palette file declares cmap_type = 1.
    let pal_rgba = vec![1u8, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 1, 2, 3, 255];
    let pal_bytes = encode_tga_palette(2, 2, &pal_rgba).unwrap();
    assert_eq!(
        parse_tga_color_map_type(&pal_bytes).unwrap(),
        ColorMapType::Present
    );
}

#[test]
fn parse_color_map_type_truncated_header_errs() {
    assert!(parse_tga_color_map_type(&[0u8; 4]).is_err());
}

#[test]
fn border_color_reads_first_map_entry_on_unmapped_image() {
    // Border colour = red (BGR 0x00,0x00,0xFF), a second map entry green.
    let map = [[0x00, 0x00, 0xFF], [0x00, 0xFF, 0x00]];
    // 2×1 white image.
    let pixels = [[0xFF, 0xFF, 0xFF], [0xFF, 0xFF, 0xFF]];
    let bytes = truecolour_with_border_map(2, 1, &map, &pixels);

    // The border colour is the FIRST map entry, de-interleaved BGR→RGBA.
    let border = parse_tga_border_color(&bytes).unwrap();
    assert_eq!(border, Some([0xFF, 0x00, 0x00, 0xFF]));

    // And the header reports a present map.
    assert_eq!(
        parse_tga_color_map_type(&bytes).unwrap(),
        ColorMapType::Present
    );

    // `parse_tga_color_map` surfaces the whole vestigial map; the border
    // colour is its first entry.
    let full = parse_tga_color_map(&bytes).unwrap().unwrap();
    assert_eq!(full.len(), 2);
    assert_eq!(full.entries[0], [0xFF, 0x00, 0x00, 0xFF]);
    assert_eq!(full.entries[1], [0x00, 0xFF, 0x00, 0xFF]);
}

#[test]
fn border_color_none_for_unmapped_without_map() {
    // Encoder-written true-colour file has no colour map.
    let pixels = vec![10u8, 20, 30, 255];
    let bytes = encode_tga_uncompressed(1, 1, &pixels).unwrap();
    assert_eq!(parse_tga_border_color(&bytes).unwrap(), None);
}

#[test]
fn border_color_none_for_colour_mapped_image() {
    // A genuinely colour-mapped image (type 1) has its map as the working
    // palette, not a border colour — `parse_tga_border_color` returns
    // None and steers the caller to `parse_tga_color_map`.
    let pal_rgba = vec![9u8, 8, 7, 255, 6, 5, 4, 255, 3, 2, 1, 255, 9, 8, 7, 255];
    let bytes = encode_tga_palette(2, 2, &pal_rgba).unwrap();
    assert_eq!(parse_tga_border_color(&bytes).unwrap(), None);
    // But the palette itself is still reachable via the colour-map API.
    assert!(parse_tga_color_map(&bytes).unwrap().is_some());
}
