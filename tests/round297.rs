//! Round 297: typed view of the extension area's **Field 26 (Postage
//! Stamp Image)** dimension header (spec §C.6.10).
//!
//! Field 26's payload is prefixed by two single-byte size fields: "The
//! first byte of the postage stamp image specifies the X size of the
//! stamp in pixels, the second byte … the Y size". Truevision adds the
//! recommendation that "stamps larger than 64 x 64 pixels … be clipped".
//!
//! The crate already decodes the full thumbnail (`parse_tga_postage_stamp`);
//! this round adds the lightweight typed view matching the rest of the
//! extension-area `_typed` family:
//!
//! * `PostageStamp` — wraps the `(width, height)` byte pair with `UNSET`
//!   / `new` / `as_tuple` / `from_tuple` / `is_unset` / `pixel_count` /
//!   `within_recommended_size` / `clipped_to_recommended`.
//! * `parse_tga_postage_stamp_dimensions` — reads just the two size bytes
//!   from a real file without decoding the pixels.
//! * `TGA_POSTAGE_STAMP_RECOMMENDED_MAX` (64) and `TGA_POSTAGE_STAMP_MAX`
//!   (255) constants.

use oxideav_tga::{
    encode_tga_rle, encode_tga_with_extension, parse_tga_postage_stamp,
    parse_tga_postage_stamp_dimensions, ExtensionAreaInput, PostageStamp, TgaImage, TgaPixelFormat,
    TGA_POSTAGE_STAMP_MAX, TGA_POSTAGE_STAMP_RECOMMENDED_MAX,
};

fn rgba_diag(width: u32, height: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let on_diag = x == y;
            let r = (x as u8).wrapping_mul(13);
            let g = (y as u8).wrapping_mul(17);
            let b = if on_diag { 0xFF } else { 0x10 };
            let a = if on_diag { 0xC0 } else { 0xFF };
            buf.extend_from_slice(&[r, g, b, a]);
        }
    }
    buf
}

// ---------------------------------------------------------------------------
// PostageStamp typed view — pure-data helpers
// ---------------------------------------------------------------------------

#[test]
fn unset_sentinel_is_zero_pair() {
    let s = PostageStamp::UNSET;
    assert_eq!(s.as_tuple(), (0, 0));
    assert!(s.is_unset());
    assert_eq!(PostageStamp::default(), PostageStamp::UNSET);
}

#[test]
fn new_and_tuple_round_trip() {
    let s = PostageStamp::new(40, 30);
    assert_eq!(s.width, 40);
    assert_eq!(s.height, 30);
    assert_eq!(s.as_tuple(), (40, 30));
    assert_eq!(PostageStamp::from_tuple((40, 30)), s);
    assert_eq!(PostageStamp::from_tuple(s.as_tuple()), s);
}

#[test]
fn is_unset_when_either_axis_zero() {
    assert!(PostageStamp::new(0, 0).is_unset());
    assert!(PostageStamp::new(0, 16).is_unset());
    assert!(PostageStamp::new(16, 0).is_unset());
    assert!(!PostageStamp::new(1, 1).is_unset());
    assert!(!PostageStamp::new(64, 64).is_unset());
}

#[test]
fn pixel_count_does_not_overflow_at_max() {
    assert_eq!(PostageStamp::new(0, 0).pixel_count(), 0);
    assert_eq!(PostageStamp::new(64, 64).pixel_count(), 4096);
    // 255 × 255 = 65025 — would overflow a u16 product; helper promotes
    // to u32.
    assert_eq!(
        PostageStamp::new(TGA_POSTAGE_STAMP_MAX, TGA_POSTAGE_STAMP_MAX).pixel_count(),
        65025
    );
}

#[test]
fn recommended_size_predicate_matches_spec_64_cap() {
    assert_eq!(TGA_POSTAGE_STAMP_RECOMMENDED_MAX, 64);
    assert!(PostageStamp::new(64, 64).within_recommended_size());
    assert!(PostageStamp::new(1, 1).within_recommended_size());
    assert!(PostageStamp::new(64, 1).within_recommended_size());
    // One pixel over on either axis is "larger than recommended".
    assert!(!PostageStamp::new(65, 64).within_recommended_size());
    assert!(!PostageStamp::new(64, 65).within_recommended_size());
    assert!(!PostageStamp::new(128, 96).within_recommended_size());
}

#[test]
fn clip_caps_each_axis_independently() {
    // Already within recommendation → unchanged.
    let small = PostageStamp::new(40, 50);
    assert_eq!(small.clipped_to_recommended(), small);
    assert!(small.clipped_to_recommended().within_recommended_size());

    // Over on both axes → both clamped to 64.
    assert_eq!(
        PostageStamp::new(200, 100).clipped_to_recommended(),
        PostageStamp::new(64, 64)
    );
    // Over on one axis only → that axis clamped, other preserved.
    assert_eq!(
        PostageStamp::new(80, 32).clipped_to_recommended(),
        PostageStamp::new(64, 32)
    );
    // Clipping is idempotent and always lands within the recommendation.
    let clipped = PostageStamp::new(255, 255).clipped_to_recommended();
    assert!(clipped.within_recommended_size());
    assert_eq!(clipped.clipped_to_recommended(), clipped);
}

// ---------------------------------------------------------------------------
// parse_tga_postage_stamp_dimensions — against real encoded files
// ---------------------------------------------------------------------------

#[test]
fn dimensions_none_for_plain_v1_and_no_stamp_files() {
    // A bare RLE file has no footer / extension area at all.
    let base = encode_tga_rle(8, 4, &rgba_diag(8, 4)).unwrap();
    assert!(parse_tga_postage_stamp_dimensions(&base).unwrap().is_none());

    // A file with an extension area but NO postage stamp (offset 0).
    let ext = ExtensionAreaInput {
        author_name: "round297".to_string(),
        ..Default::default()
    };
    let with_ext = encode_tga_with_extension(&base, &ext).unwrap();
    assert!(parse_tga_postage_stamp_dimensions(&with_ext)
        .unwrap()
        .is_none());
}

#[test]
fn dimensions_match_embedded_stamp_and_skip_decode() {
    let base = encode_tga_rle(8, 4, &rgba_diag(8, 4)).unwrap();
    let ext = ExtensionAreaInput {
        attributes_type: 3,
        postage_stamp: Some(TgaImage {
            width: 4,
            height: 2,
            pixel_format: TgaPixelFormat::Rgba,
            data: rgba_diag(4, 2),
            pts: None,
        }),
        ..Default::default()
    };
    let full = encode_tga_with_extension(&base, &ext).unwrap();

    // The lightweight dimension reader sees the 4×2 stamp.
    let dims = parse_tga_postage_stamp_dimensions(&full)
        .unwrap()
        .expect("stamp dimensions present");
    assert_eq!(dims.as_tuple(), (4, 2));
    assert!(!dims.is_unset());
    assert_eq!(dims.pixel_count(), 8);
    assert!(dims.within_recommended_size());

    // ...and it agrees with the full decoder's reported geometry.
    let stamp = parse_tga_postage_stamp(&full).unwrap().expect("stamp");
    assert_eq!(dims.width as u32, stamp.width);
    assert_eq!(dims.height as u32, stamp.height);
}

#[test]
fn dimensions_flag_oversized_stamp_per_recommendation() {
    // Embed a 100×70 stamp — spec-legal (each axis < 256) but past the
    // 64×64 recommendation.
    let base = encode_tga_rle(128, 96, &rgba_diag(128, 96)).unwrap();
    let ext = ExtensionAreaInput {
        attributes_type: 3,
        postage_stamp: Some(TgaImage {
            width: 100,
            height: 70,
            pixel_format: TgaPixelFormat::Rgba,
            data: rgba_diag(100, 70),
            pts: None,
        }),
        ..Default::default()
    };
    let full = encode_tga_with_extension(&base, &ext).unwrap();

    let dims = parse_tga_postage_stamp_dimensions(&full)
        .unwrap()
        .expect("oversized stamp dimensions present");
    assert_eq!(dims.as_tuple(), (100, 70));
    assert!(!dims.within_recommended_size());
    assert_eq!(dims.clipped_to_recommended(), PostageStamp::new(64, 64));
    // The full decoder still reads the oversized stamp unchanged — the
    // recommendation is advisory, not a hard limit.
    let stamp = parse_tga_postage_stamp(&full).unwrap().expect("stamp");
    assert_eq!((stamp.width, stamp.height), (100, 70));
}

#[test]
fn dimensions_err_on_out_of_range_offset() {
    // Build a real extension-area file (no stamp), then patch the
    // postage-stamp offset (extension-area byte 486) to point one byte
    // past the buffer. The footer + extension area still parse cleanly
    // via their own (in-bounds) offsets, so the helper reaches the
    // size-header read and must return Err — not panic — when the
    // pointed-at header lies outside the buffer.
    let base = encode_tga_rle(8, 4, &rgba_diag(8, 4)).unwrap();
    let ext = ExtensionAreaInput {
        author_name: "round297".to_string(),
        ..Default::default()
    };
    let mut full = encode_tga_with_extension(&base, &ext).unwrap();

    // The footer's last 26 bytes carry the extension-area offset in its
    // first 4 bytes (LE u32).
    let footer_start = full.len() - 26;
    let ext_off = u32::from_le_bytes([
        full[footer_start],
        full[footer_start + 1],
        full[footer_start + 2],
        full[footer_start + 3],
    ]) as usize;

    // Point the postage-stamp offset (ext byte 486) just past the end.
    let bad = (full.len() as u32) + 1;
    full[ext_off + 486..ext_off + 490].copy_from_slice(&bad.to_le_bytes());

    match parse_tga_postage_stamp_dimensions(&full) {
        Err(_) => {}
        other => panic!("expected Err for out-of-range stamp offset, got {other:?}"),
    }
}
