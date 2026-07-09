//! Round 403 — hand-built on-disk files pinning the decoder's storage-order
//! normalisation, 15/16-bit A1R5G5B5 expansion, and origin-aware colour-map
//! index resolution.
//!
//! The identity matrix (`round403.rs`) only ever exercises what the crate's
//! own encoder writes: top-down origin, 24/32-bpp true-colour, origin-0
//! colour maps. These tests reach the decode-only paths the encoder never
//! produces by assembling the on-disk bytes directly:
//!   * image-descriptor bit 5 (bottom-up) / bit 4 (right-to-left) row and
//!     column flips per the TGA 2.0 FFS Table 2 Image Origin field;
//!   * 16-bpp A1R5G5B5 (top bit alpha) and 15-bpp (alpha forced opaque)
//!     true-colour expansion (§C.2);
//!   * a colour-mapped file whose §C.2 Color Map Origin is non-zero, so a
//!     logical index `idx` addresses on-disk entry `idx - first`, and
//!     indices below the origin or past the stored length are rejected.

use oxideav_tga::*;

fn base_header(image_type: u8, w: u16, h: u16, depth: u8, desc: u8) -> Vec<u8> {
    let mut v = vec![0u8; 18];
    v[2] = image_type;
    v[12] = (w & 0xFF) as u8;
    v[13] = (w >> 8) as u8;
    v[14] = (h & 0xFF) as u8;
    v[15] = (h >> 8) as u8;
    v[16] = depth;
    v[17] = desc;
    v
}

/// 2x2 24-bpp uncompressed file with a given descriptor and four on-disk
/// BGR pixels laid out in on-disk (storage) order.
fn file_2x2_24(desc: u8, disk_bgr: &[[u8; 3]; 4]) -> Vec<u8> {
    let mut v = base_header(2, 2, 2, 24, desc);
    for px in disk_bgr {
        v.extend_from_slice(px);
    }
    v
}

#[test]
fn storage_order_flips_normalise_to_topdown_left_right() {
    // Logical display image (top-down, left-right):
    //   (0,0)=red   (1,0)=green
    //   (0,1)=blue  (1,1)=white
    // BGR on disk.
    let red = [0x00, 0x00, 0xFF];
    let green = [0x00, 0xFF, 0x00];
    let blue = [0xFF, 0x00, 0x00];
    let white = [0xFF, 0xFF, 0xFF];
    let want = [
        [0xFF, 0x00, 0x00, 0xFF],
        [0x00, 0xFF, 0x00, 0xFF],
        [0x00, 0x00, 0xFF, 0xFF],
        [0xFF, 0xFF, 0xFF, 0xFF],
    ];

    // top-down (bit5 set = 0x20): disk row0 == display row0.
    let td = file_2x2_24(0x20, &[red, green, blue, white]);
    // bottom-up (bit5 clear = 0x00): disk row0 == display *last* row.
    let bu = file_2x2_24(0x00, &[blue, white, red, green]);
    // top-down right-to-left (bit5+bit4 = 0x30): each row reversed on disk.
    let rl = file_2x2_24(0x30, &[green, red, white, blue]);
    // bottom-up right-to-left (bit4 = 0x10): both axes flipped == 180deg.
    let both = file_2x2_24(0x10, &[white, blue, green, red]);

    for (label, file) in [("td", td), ("bu", bu), ("rl", rl), ("both", both)] {
        let img = parse_tga(&file).unwrap();
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        for (i, expect) in want.iter().enumerate() {
            assert_eq!(&img.data[i * 4..i * 4 + 4], expect, "{label} px{i}");
        }
    }
}

#[test]
fn expand_16bpp_a1r5g5b5() {
    // 16-bpp: top bit = alpha. 0xFC00 = A1 R11111 G0 B0 -> opaque red.
    // 0x001F = A0 R0 G0 B11111 -> blue, alpha 0.
    let mut v = base_header(2, 2, 1, 16, 0x20);
    v.extend_from_slice(&0xFC00u16.to_le_bytes());
    v.extend_from_slice(&0x001Fu16.to_le_bytes());
    let img = parse_tga(&v).unwrap();
    assert_eq!(&img.data[0..4], &[0xFF, 0x00, 0x00, 0xFF], "opaque red");
    assert_eq!(&img.data[4..8], &[0x00, 0x00, 0xFF, 0x00], "blue alpha0");
}

#[test]
fn expand_15bpp_forces_opaque() {
    // 15-bpp: top bit reserved; alpha always 0xFF even when it is clear.
    let mut v = base_header(2, 1, 1, 15, 0x20);
    v.extend_from_slice(&0x001Fu16.to_le_bytes());
    let img = parse_tga(&v).unwrap();
    assert_eq!(&img.data[0..4], &[0x00, 0x00, 0xFF, 0xFF]);
}

#[test]
fn expand5_replicates_top_bits() {
    // 5-bit 0b11111 must expand to 0xFF (bit replication), not 0xF8.
    // 16-bpp value with all colour bits set + alpha bit set.
    let mut v = base_header(2, 1, 1, 16, 0x20);
    v.extend_from_slice(&0xFFFFu16.to_le_bytes());
    let img = parse_tga(&v).unwrap();
    assert_eq!(
        &img.data[0..4],
        &[0xFF, 0xFF, 0xFF, 0xFF],
        "0b11111 -> 0xFF"
    );
    // 0b10000 (16) must expand to 0x84 = (16<<3)|(16>>2).
    let mut v2 = base_header(2, 1, 1, 15, 0x20);
    // R = 0b10000, G/B = 0.
    v2.extend_from_slice(&(0x10u16 << 10).to_le_bytes());
    let img2 = parse_tga(&v2).unwrap();
    assert_eq!(img2.data[0], 0x84, "0b10000 -> 0x84");
}

#[test]
fn colour_map_origin_resolves_and_rejects_out_of_range() {
    // Type 1, 8-bpp index. Color Map Origin = 10, length = 3, 24-bpp BGR.
    let mut v = vec![0u8; 18];
    v[1] = 1; // colour map present
    v[2] = 1; // uncompressed colour-mapped
    v[3] = 10; // first index lo
    v[5] = 3; // length lo
    v[7] = 24; // entry size
    v[12] = 1; // width
    v[14] = 3; // height
    v[16] = 8; // index depth
    v[17] = 0x20; // top-down
    v.extend_from_slice(&[0x00, 0x00, 0xFF]); // entry@10 -> red
    v.extend_from_slice(&[0x00, 0xFF, 0x00]); // entry@11 -> green
    v.extend_from_slice(&[0xFF, 0x00, 0x00]); // entry@12 -> blue
    v.extend_from_slice(&[10, 11, 12]); // pixel indices

    let img = parse_tga(&v).unwrap();
    assert_eq!(&img.data[0..4], &[0xFF, 0x00, 0x00, 0xFF]);
    assert_eq!(&img.data[4..8], &[0x00, 0xFF, 0x00, 0xFF]);
    assert_eq!(&img.data[8..12], &[0x00, 0x00, 0xFF, 0xFF]);

    // index below the origin -> out of range.
    let mut below = v.clone();
    let n = below.len();
    below[n - 3] = 9;
    assert!(parse_tga(&below).is_err(), "index 9 < origin 10");

    // index past origin + length -> out of range.
    let mut past = v.clone();
    let m = past.len();
    past[m - 1] = 13; // 13 >= 10 + 3
    assert!(parse_tga(&past).is_err(), "index 13 past stored length");

    // parse_tga_color_map surfaces the origin-aware palette too.
    let cmap = parse_tga_color_map(&v).unwrap().expect("map present");
    assert_eq!(cmap.get(10), Some([0xFF, 0x00, 0x00, 0xFF]));
    assert_eq!(cmap.get(12), Some([0x00, 0x00, 0xFF, 0xFF]));
    assert_eq!(cmap.get(9), None, "below origin");
    assert_eq!(cmap.get(13), None, "past length");
}
