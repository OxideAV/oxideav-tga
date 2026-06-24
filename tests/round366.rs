//! Round 366 — §5.1 / §5.2 Image Origin typed view.
//!
//! The fixed-header **X-origin / Y-origin of Image** fields (Field 5.1,
//! bytes 8-9, and Field 5.2, bytes 10-11) record "the absolute
//! horizontal / vertical coordinate for the lower left corner of the
//! image as it is positioned on a display device having an origin at the
//! lower left of the screen (e.g., the TARGA series)". The decoder has
//! always parsed them into `TgaHeader::x_origin` / `y_origin` but they
//! were the last fixed-header field without a typed view — every other
//! header field (attribute-bit count, interleaving flag, colour-map type)
//! already mirrors the `_typed` accessor pattern.
//!
//! This round adds the `ImageOrigin` typed view reachable via
//! `TgaHeader::image_origin()` and the one-call `parse_tga_image_origin`,
//! plus the `to_bytes` / `from_header` round-trip primitives, mirroring
//! `AttributeBits` / `Interleaving` / `ColorMapType`.

use oxideav_tga::{
    encode_tga_uncompressed, parse_header, parse_tga_image_origin, ImageOrigin, TGA_HEADER_SIZE,
};

/// Tiny RGBA frame for the encoder so the header reader has a real file
/// to walk (every encoder writes the screen-origin `(0, 0)`).
fn small_rgba() -> Vec<u8> {
    // 2×2 opaque pixels.
    vec![
        10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
    ]
}

/// Lay an `(x_origin, y_origin)` pair into a freshly-encoded base file's
/// header bytes 8-11 by hand so we can exercise the *non-zero* origin
/// read path (the crate's encoder always writes `0`).
fn set_origin(base: &mut [u8], x: u16, y: u16) {
    base[8..10].copy_from_slice(&x.to_le_bytes());
    base[10..12].copy_from_slice(&y.to_le_bytes());
}

#[test]
fn origin_sentinel_and_constructors() {
    assert_eq!(ImageOrigin::ORIGIN, ImageOrigin::default());
    assert_eq!(ImageOrigin::ORIGIN, ImageOrigin::new(0, 0));
    assert!(ImageOrigin::ORIGIN.is_origin());
    assert!(!ImageOrigin::ORIGIN.is_offset());

    let o = ImageOrigin::new(640, 480);
    assert_eq!(o.x, 640);
    assert_eq!(o.y, 480);
    assert!(o.is_offset());
    assert!(!o.is_origin());
}

#[test]
fn tuple_round_trip() {
    let o = ImageOrigin::new(123, 65535);
    assert_eq!(o.as_tuple(), (123, 65535));
    assert_eq!(ImageOrigin::from_tuple((123, 65535)), o);
    // Full inverse across the SHORT domain edges.
    for (x, y) in [(0, 0), (0, 1), (1, 0), (65535, 65535), (32768, 100)] {
        let o = ImageOrigin::new(x, y);
        assert_eq!(ImageOrigin::from_tuple(o.as_tuple()), o);
    }
}

#[test]
fn to_bytes_is_le_x_then_y() {
    // 0x1234 = 4660, 0xABCD = 43981.
    let o = ImageOrigin::new(0x1234, 0xABCD);
    assert_eq!(o.to_bytes(), [0x34, 0x12, 0xCD, 0xAB]);
    // origin sentinel is all-zero on disk.
    assert_eq!(ImageOrigin::ORIGIN.to_bytes(), [0, 0, 0, 0]);
}

#[test]
fn from_header_reads_bytes_8_to_11() {
    let mut hdr = [0u8; TGA_HEADER_SIZE];
    // x = 0x0102 at 8-9, y = 0x0304 at 10-11.
    hdr[8] = 0x02;
    hdr[9] = 0x01;
    hdr[10] = 0x04;
    hdr[11] = 0x03;
    let o = ImageOrigin::from_header(&hdr).unwrap();
    assert_eq!(o, ImageOrigin::new(0x0102, 0x0304));
    // to_bytes is the inverse of from_header's coordinate read.
    assert_eq!(&o.to_bytes()[..], &hdr[8..12]);
}

#[test]
fn from_header_rejects_short_input() {
    assert_eq!(ImageOrigin::from_header(&[0u8; 17]), None);
    assert!(ImageOrigin::from_header(&[0u8; 18]).is_some());
}

#[test]
fn encoder_writes_screen_origin() {
    // Every writer emits (0, 0) — the screen-origin default.
    let bytes = encode_tga_uncompressed(2, 2, &small_rgba()).unwrap();
    let o = parse_tga_image_origin(&bytes).unwrap();
    assert_eq!(o, ImageOrigin::ORIGIN);
    assert!(o.is_origin());
    // Agrees with the raw header fields and the accessor.
    let h = parse_header(&bytes).unwrap();
    assert_eq!(h.x_origin, 0);
    assert_eq!(h.y_origin, 0);
    assert_eq!(h.image_origin(), o);
}

#[test]
fn non_zero_origin_round_trips_through_reader() {
    let mut bytes = encode_tga_uncompressed(2, 2, &small_rgba()).unwrap();
    set_origin(&mut bytes, 1920, 1080);
    let o = parse_tga_image_origin(&bytes).unwrap();
    assert_eq!(o, ImageOrigin::new(1920, 1080));
    assert!(o.is_offset());
    // The one-call reader agrees with the header accessor.
    let h = parse_header(&bytes).unwrap();
    assert_eq!(h.image_origin(), o);
    assert_eq!(h.x_origin, 1920);
    assert_eq!(h.y_origin, 1080);
    // The decoded pixels are unchanged — the origin does not relocate
    // the raster, only records on-screen placement.
    let img = oxideav_tga::parse_tga(&bytes).unwrap();
    assert_eq!((img.width, img.height), (2, 2));
}

#[test]
fn reader_returns_none_on_truncated_header() {
    assert_eq!(parse_tga_image_origin(&[0u8; 10]), None);
}

#[test]
fn origin_is_orthogonal_to_descriptor_storage_bits() {
    // A top-down file (descriptor bit 5 set — what the encoder writes)
    // can still carry a non-zero on-screen origin. The two are
    // independent header concepts.
    let mut bytes = encode_tga_uncompressed(2, 2, &small_rgba()).unwrap();
    set_origin(&mut bytes, 50, 60);
    let h = parse_header(&bytes).unwrap();
    assert!(h.is_top_down(), "encoder writes top-down storage order");
    assert_eq!(h.image_origin(), ImageOrigin::new(50, 60));
    assert!(h.image_origin().is_offset());
}
