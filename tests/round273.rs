//! Round 273 — colour-map Color Map Origin (First Entry Index) application.
//!
//! The §C.2 Color Map Specification carries a "Color Map Origin" (header
//! bytes 3-4, "index of first color map entry"). The colour-map data on
//! disk stores `cmap_length` entries that correspond to logical palette
//! indices starting at that origin. An image pixel index `idx` therefore
//! addresses on-disk colour-map entry `idx - cmap_first`.
//!
//! The decoder long parsed `cmap_first` into the header but the palette
//! lookup ignored it, so any colour-mapped file with a non-zero Color Map
//! Origin decoded with an `idx`-vs-`idx - cmap_first` off-by-origin error
//! (or a spurious out-of-range rejection). This round applies the origin
//! in `emit_pixel` for both the uncompressed (type 1) and RLE (type 9)
//! colour-mapped paths, and for the postage-stamp colour-mapped path that
//! shares the parent header.
//!
//! Origin-0 files (everything the crate's own encoder writes) are
//! unaffected: `idx - 0 == idx`.

use oxideav_tga::{parse_tga, TgaPixelFormat};

/// Build a 4×1 type-1 (uncompressed colour-mapped) TGA whose colour map
/// declares a non-zero Color Map Origin. The on-disk palette stores
/// `cmap_length` entries beginning at logical index `cmap_first`; the
/// pixel indices are the *logical* (origin-relative) indices.
fn colour_mapped_with_origin(
    cmap_first: u16,
    palette_bgr: &[[u8; 3]],
    pixel_indices: &[u8],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(0); // id_length
    bytes.push(1); // cmap_type
    bytes.push(1); // image_type = uncompressed colour-mapped
    bytes.extend_from_slice(&cmap_first.to_le_bytes());
    bytes.extend_from_slice(&(palette_bgr.len() as u16).to_le_bytes());
    bytes.push(24); // cmap_entry_size
    bytes.extend_from_slice(&0u16.to_le_bytes()); // x_origin
    bytes.extend_from_slice(&0u16.to_le_bytes()); // y_origin
    bytes.extend_from_slice(&(pixel_indices.len() as u16).to_le_bytes()); // width
    bytes.extend_from_slice(&1u16.to_le_bytes()); // height
    bytes.push(8); // depth (8-bit index)
    bytes.push(0x20); // descriptor — top-down
    for bgr in palette_bgr {
        bytes.extend_from_slice(bgr);
    }
    bytes.extend_from_slice(pixel_indices);
    bytes
}

#[test]
fn origin_zero_is_unchanged() {
    // Pure regression: an origin-0 colour-mapped file decodes exactly as
    // before — `idx - 0 == idx`.
    let pal = [[0x00, 0x00, 0xFF], [0x00, 0xFF, 0x00], [0xFF, 0x00, 0x00]];
    let bytes = colour_mapped_with_origin(0, &pal, &[0, 1, 2, 0]);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    let want: Vec<u8> = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 0, 0, 255],
    ]
    .concat();
    assert_eq!(img.data, want);
}

#[test]
fn nonzero_origin_offsets_palette_lookup() {
    // Origin = 10: the three on-disk palette entries are logical indices
    // 10, 11, 12. Pixel index 10 → entry 0 (red), 11 → entry 1 (green),
    // 12 → entry 2 (blue). Without the fix, index 10 would read past the
    // 3-entry palette and the file would be rejected as out of range.
    let pal = [[0x00, 0x00, 0xFF], [0x00, 0xFF, 0x00], [0xFF, 0x00, 0x00]];
    let bytes = colour_mapped_with_origin(10, &pal, &[10, 11, 12, 10]);
    let img = parse_tga(&bytes).unwrap();
    let want: Vec<u8> = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 0, 0, 255],
    ]
    .concat();
    assert_eq!(img.data, want);
}

#[test]
fn index_below_origin_is_rejected() {
    // Origin = 5 but a pixel carries index 3, which is below the origin —
    // there is no on-disk entry for it, so the decoder must reject rather
    // than wrap/underflow.
    let pal = [[0x00, 0x00, 0xFF], [0x00, 0xFF, 0x00]];
    let bytes = colour_mapped_with_origin(5, &pal, &[5, 3]);
    assert!(parse_tga(&bytes).is_err());
}

#[test]
fn index_at_or_past_origin_plus_length_is_rejected() {
    // Origin = 5, two entries (logical 5, 6). Index 7 (origin + length) is
    // one past the stored palette and must be rejected.
    let pal = [[0x00, 0x00, 0xFF], [0x00, 0xFF, 0x00]];
    let bytes = colour_mapped_with_origin(5, &pal, &[5, 7]);
    assert!(parse_tga(&bytes).is_err());
}

/// Build a 4×1 type-9 (RLE colour-mapped) TGA with a non-zero Color Map
/// Origin and a single run packet covering all four pixels.
fn rle_colour_mapped_run_with_origin(
    cmap_first: u16,
    palette_bgr: &[[u8; 3]],
    index: u8,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(0); // id_length
    bytes.push(1); // cmap_type
    bytes.push(9); // image_type = RLE colour-mapped
    bytes.extend_from_slice(&cmap_first.to_le_bytes());
    bytes.extend_from_slice(&(palette_bgr.len() as u16).to_le_bytes());
    bytes.push(24); // cmap_entry_size
    bytes.extend_from_slice(&0u16.to_le_bytes()); // x_origin
    bytes.extend_from_slice(&0u16.to_le_bytes()); // y_origin
    bytes.extend_from_slice(&4u16.to_le_bytes()); // width
    bytes.extend_from_slice(&1u16.to_le_bytes()); // height
    bytes.push(8); // depth
    bytes.push(0x20); // top-down
    for bgr in palette_bgr {
        bytes.extend_from_slice(bgr);
    }
    // One run packet: count = 4 (0x80 | 3), one index byte.
    bytes.push(0x80 | 3);
    bytes.push(index);
    bytes
}

#[test]
fn rle_colour_mapped_honours_origin() {
    // Origin = 100, three entries (logical 100, 101, 102). A run packet of
    // index 101 fills the row with on-disk entry 1 (green).
    let pal = [[0x00, 0x00, 0xFF], [0x00, 0xFF, 0x00], [0xFF, 0x00, 0x00]];
    let bytes = rle_colour_mapped_run_with_origin(100, &pal, 101);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, 4);
    let want: Vec<u8> = std::iter::repeat([0u8, 255, 0, 255])
        .take(4)
        .flatten()
        .collect();
    assert_eq!(img.data, want);
}

#[test]
fn rle_colour_mapped_index_below_origin_rejected() {
    let pal = [[0x00, 0x00, 0xFF], [0x00, 0xFF, 0x00]];
    let bytes = rle_colour_mapped_run_with_origin(100, &pal, 50);
    assert!(parse_tga(&bytes).is_err());
}

#[test]
fn high_origin_at_short_top_of_index_space() {
    // Origin = 254, two entries (logical 254, 255). 8-bit indices reach
    // 255, so the top of the index space is addressable through the origin.
    let pal = [[0xFF, 0x00, 0x00], [0xFF, 0xFF, 0xFF]];
    let bytes = colour_mapped_with_origin(254, &pal, &[254, 255, 255, 254]);
    let img = parse_tga(&bytes).unwrap();
    let want: Vec<u8> = [
        [0, 0, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [0, 0, 255, 255],
    ]
    .concat();
    assert_eq!(img.data, want);
}
