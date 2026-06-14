//! Round 304 — public Color Map extraction (`parse_tga_color_map`) plus
//! the typed [`TgaColorMap`] view (spec §C.2 / Data Type 1 header bytes
//! 3-7: Color Map Origin, Color Map Length, Color Map Entry Size).
//!
//! A color map is present whenever the header's Color Map Type byte
//! (byte 1) is `1`. The crate has always consumed it internally to decode
//! colour-mapped image types (1 / 9), but never exposed the palette on its
//! own. Two cases this leaves on the table:
//!
//! * a colour-mapped file whose palette a caller wants to inspect without
//!   decoding the full raster, and
//! * a **Data Type 0 (No Image Data)** file — a header + color map (and/or
//!   metadata) with no pixel array — which `parse_tga` rejects ("no image
//!   data") even though the palette is perfectly readable.
//!
//! `parse_tga_color_map` reads only the header + color-map block, so it
//! handles both. Each entry is de-interleaved into straight RGBA exactly
//! as `parse_tga` expands the palette internally (15/16-bit A1R5G5B5,
//! 24-bit BGR, 32-bit BGRA), and the Color Map Origin is recorded so a
//! caller can resolve logical palette indices via `TgaColorMap::get`.

use oxideav_tga::{encode_tga_palette, parse_tga, parse_tga_color_map, TgaColorMap};

/// Parameters for a minimal hand-built TGA header + color map. Lets us
/// produce a Data Type 0 (no-image) palette-only file (`image_type = 0`)
/// or a colour-mapped image; the pixel/index payload, if any, is the
/// `tail`.
struct Build<'a> {
    image_type: u8,
    cmap_first: u16,
    cmap_entry_size: u8,
    cmap_entries: &'a [u8],
    cmap_len: u16,
    width: u16,
    height: u16,
    depth: u8,
    tail: &'a [u8],
}

fn build_with_color_map(b: Build<'_>) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(0); // id_length
    bytes.push(1); // cmap_type = 1 (color map present)
    bytes.push(b.image_type); // image type code
    bytes.extend_from_slice(&b.cmap_first.to_le_bytes()); // color map origin
    bytes.extend_from_slice(&b.cmap_len.to_le_bytes()); // color map length
    bytes.push(b.cmap_entry_size); // color map entry size (bits)
    bytes.extend_from_slice(&0u16.to_le_bytes()); // x origin
    bytes.extend_from_slice(&0u16.to_le_bytes()); // y origin
    bytes.extend_from_slice(&b.width.to_le_bytes()); // width
    bytes.extend_from_slice(&b.height.to_le_bytes()); // height
    bytes.push(b.depth); // pixel depth
    bytes.push(0x20); // descriptor — top-down
    bytes.extend_from_slice(b.cmap_entries); // color map data
    bytes.extend_from_slice(b.tail); // pixel data (if any)
    bytes
}

// ---------------------------------------------------------------------------
// TgaColorMap typed view — pure-data helpers
// ---------------------------------------------------------------------------

#[test]
fn empty_sentinel_and_default() {
    let m = TgaColorMap::empty();
    assert!(m.is_empty());
    assert_eq!(m.len(), 0);
    assert_eq!(m.first_index, 0);
    assert_eq!(m.entry_size, 0);
    assert_eq!(TgaColorMap::default(), m);
    // get on an empty map never resolves.
    assert_eq!(m.get(0), None);
}

#[test]
fn get_honours_color_map_origin() {
    let m = TgaColorMap {
        first_index: 10,
        entry_size: 24,
        entries: vec![[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]],
    };
    assert_eq!(m.len(), 3);
    assert!(!m.is_empty());
    // Logical index 10 -> stored entry 0, 11 -> 1, 12 -> 2.
    assert_eq!(m.get(10), Some([255, 0, 0, 255]));
    assert_eq!(m.get(11), Some([0, 255, 0, 255]));
    assert_eq!(m.get(12), Some([0, 0, 255, 255]));
    // Below the origin and past the stored length both miss.
    assert_eq!(m.get(9), None);
    assert_eq!(m.get(13), None);
}

// ---------------------------------------------------------------------------
// parse_tga_color_map — against real / hand-built files
// ---------------------------------------------------------------------------

#[test]
fn none_when_no_color_map_present() {
    // A true-colour file the crate's own encoder writes has cmap_type 0.
    let rgba: Vec<u8> = (0..8 * 4).flat_map(|i| [i as u8, 0, 0, 255]).collect();
    let base = oxideav_tga::encode_tga_uncompressed(8, 4, &rgba).unwrap();
    assert!(parse_tga_color_map(&base).unwrap().is_none());
}

#[test]
fn reads_color_map_from_encoder_palette_file() {
    // The palette encoder writes a 32-bit BGRA color map. Extract it and
    // confirm every encoded colour resolves through the map.
    let palette_colours: [[u8; 4]; 4] = [
        [200, 10, 20, 255],
        [10, 200, 20, 255],
        [10, 20, 200, 128],
        [50, 50, 50, 255],
    ];
    let rgba: Vec<u8> = palette_colours.iter().flatten().copied().collect();
    let base = encode_tga_palette(4, 1, &rgba).unwrap();

    let map = parse_tga_color_map(&base)
        .unwrap()
        .expect("color map present");
    assert_eq!(map.entry_size, 32);
    // The encoder uses origin 0.
    assert_eq!(map.first_index, 0);
    assert!(!map.is_empty());

    // Decode the whole file and confirm each output pixel equals a map
    // entry the parser surfaced — the public color map agrees with the
    // decoder's internal palette.
    let img = parse_tga(&base).unwrap();
    for px in img.data.chunks_exact(4) {
        let rgba = [px[0], px[1], px[2], px[3]];
        assert!(
            map.entries.contains(&rgba),
            "decoded pixel {rgba:?} not found in extracted color map"
        );
    }
}

#[test]
fn reads_palette_only_type0_file_that_parse_tga_rejects() {
    // Data Type 0 (No Image Data): a header + color map with no pixel
    // array. `parse_tga` rejects it (nothing to decode), but the palette
    // is fully readable.
    // Three 24-bit BGR entries: red, green, blue (on-disk BGR order).
    let cmap = [
        0x00, 0x00, 0xFF, // BGR red
        0x00, 0xFF, 0x00, // BGR green
        0xFF, 0x00, 0x00, // BGR blue
    ];
    let file = build_with_color_map(Build {
        image_type: 0,
        cmap_first: 0,
        cmap_entry_size: 24,
        cmap_entries: &cmap,
        cmap_len: 3,
        width: 0,
        height: 0,
        depth: 0,
        tail: &[],
    });

    // The full decoder refuses — no image data.
    assert!(parse_tga(&file).is_err());

    // The color-map reader recovers the palette anyway.
    let map = parse_tga_color_map(&file)
        .unwrap()
        .expect("palette present");
    assert_eq!(map.entry_size, 24);
    assert_eq!(map.first_index, 0);
    assert_eq!(
        map.entries,
        vec![[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
    );
    assert_eq!(map.get(0), Some([255, 0, 0, 255]));
    assert_eq!(map.get(2), Some([0, 0, 255, 255]));
}

#[test]
fn surfaces_nonzero_color_map_origin() {
    // A colour-mapped (type 1) file with a non-zero Color Map Origin.
    // The two on-disk entries are logical indices 10 and 11.
    let cmap = [
        0x00, 0x00, 0xFF, // index 10 -> red
        0x00, 0xFF, 0x00, // index 11 -> green
    ];
    let pixels = [10u8, 11, 10];
    let file = build_with_color_map(Build {
        image_type: 1,
        cmap_first: 10,
        cmap_entry_size: 24,
        cmap_entries: &cmap,
        cmap_len: 2,
        width: 3,
        height: 1,
        depth: 8,
        tail: &pixels,
    });

    let map = parse_tga_color_map(&file)
        .unwrap()
        .expect("color map present");
    assert_eq!(map.first_index, 10);
    assert_eq!(map.len(), 2);
    // get resolves the logical indices through the recorded origin.
    assert_eq!(map.get(10), Some([255, 0, 0, 255]));
    assert_eq!(map.get(11), Some([0, 255, 0, 255]));
    assert_eq!(map.get(9), None);

    // The full decode of the same file agrees: pixels are red, green, red.
    let img = parse_tga(&file).unwrap();
    assert_eq!(
        img.data,
        [[255, 0, 0, 255], [0, 255, 0, 255], [255, 0, 0, 255]].concat()
    );
}

#[test]
fn err_on_truncated_color_map() {
    // Declares 4 entries × 3 bytes = 12 bytes but only provides 6.
    let cmap = [0u8; 6];
    let file = build_with_color_map(Build {
        image_type: 0,
        cmap_first: 0,
        cmap_entry_size: 24,
        cmap_entries: &cmap,
        cmap_len: 4,
        width: 0,
        height: 0,
        depth: 0,
        tail: &[],
    });
    assert!(parse_tga_color_map(&file).is_err());
}

#[test]
fn err_on_cmap_type_one_with_zero_length() {
    // cmap_type = 1 but cmap_length = 0 is malformed.
    let file = build_with_color_map(Build {
        image_type: 0,
        cmap_first: 0,
        cmap_entry_size: 24,
        cmap_entries: &[],
        cmap_len: 0,
        width: 0,
        height: 0,
        depth: 0,
        tail: &[],
    });
    assert!(parse_tga_color_map(&file).is_err());
}

#[test]
fn err_on_truncated_header() {
    assert!(parse_tga_color_map(&[0u8; 5]).is_err());
}
