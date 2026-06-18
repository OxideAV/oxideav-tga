//! Round 337 — §C.6.4 Key Color (Field 18) apply-path: chroma-keying.
//!
//! The Key Color quadruple could be read (`key_color` header field),
//! typed (`KeyColor::from_argb` / `key_color_typed`), and inspected
//! (`as_rgba8` / `is_unset` / `has_alpha`), but — unlike every other
//! extension-area surface (gamma, pixel-aspect-ratio, colour-correction
//! table, attributes-type) — it had no *apply* path. Spec §C.6.4 calls
//! Field 18 the image's "transparent colour" / "the colour the screen
//! would be cleared to". The apply-side of that interpretation is
//! chroma-keying: wherever a decoded pixel equals the key colour, mark
//! it transparent.
//!
//! This round adds:
//! * `KeyColor::matches_rgb` / `matches_rgba` — match predicates.
//! * `KeyColor::key_out_image` — set alpha to 0 for every RGB-matching
//!   pixel of an RGBA image, returning the count keyed out.

use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga, parse_tga_extension_area,
    ExtensionAreaInput, KeyColor, TgaImage, TgaPixelFormat,
};

fn rgba_image(width: u32, height: u32, data: Vec<u8>) -> TgaImage {
    assert_eq!(data.len(), (width * height * 4) as usize);
    TgaImage {
        width,
        height,
        pixel_format: TgaPixelFormat::Rgba,
        data,
        pts: None,
    }
}

#[test]
fn matches_rgb_ignores_alpha_bytes() {
    // Key colour magenta with alpha byte 0x00 (the spec's "no alpha
    // channel" recommendation).
    // On-disk A:R:G:B → R=FF, G=00, B=FF (magenta).
    let key = KeyColor::from_argb([0x00, 0xFF, 0x00, 0xFF]);
    // A pixel that is magenta but fully opaque still matches on RGB.
    assert!(key.matches_rgb([0xFF, 0x00, 0xFF, 0xFF]));
    // Differing alpha does not affect the RGB-only match.
    assert!(key.matches_rgb([0xFF, 0x00, 0xFF, 0x00]));
    // A different colour does not match.
    assert!(!key.matches_rgb([0xFE, 0x00, 0xFF, 0xFF]));
}

#[test]
fn matches_rgba_compares_all_four_channels() {
    // Key colour with a meaningful alpha byte.
    let key = KeyColor::from_argb([0x80, 0x10, 0x20, 0x30]); // A=80,R=10,G=20,B=30
    assert!(key.matches_rgba([0x10, 0x20, 0x30, 0x80]));
    // Same RGB, different alpha: rgba match fails, rgb match holds.
    assert!(!key.matches_rgba([0x10, 0x20, 0x30, 0x7F]));
    assert!(key.matches_rgb([0x10, 0x20, 0x30, 0x7F]));
}

#[test]
fn key_out_image_makes_matching_pixels_transparent() {
    // 2×2 image: top row magenta, bottom row two distinct colours.
    let magenta = [0xFFu8, 0x00, 0xFF, 0xFF];
    let red = [0xFFu8, 0x00, 0x00, 0xFF];
    let blue = [0x00u8, 0x00, 0xFF, 0xFF];
    let mut data = Vec::new();
    data.extend_from_slice(&magenta);
    data.extend_from_slice(&magenta);
    data.extend_from_slice(&red);
    data.extend_from_slice(&blue);
    let mut img = rgba_image(2, 2, data);

    let key = KeyColor::from_argb([0x00, 0xFF, 0x00, 0xFF]); // magenta, A=0
    let keyed = key.key_out_image(&mut img);
    assert_eq!(keyed, 2, "both magenta pixels keyed out");

    // Magenta pixels: alpha zeroed, colour bytes untouched.
    assert_eq!(&img.data[0..4], &[0xFF, 0x00, 0xFF, 0x00]);
    assert_eq!(&img.data[4..8], &[0xFF, 0x00, 0xFF, 0x00]);
    // Non-matching pixels: untouched.
    assert_eq!(&img.data[8..12], &red);
    assert_eq!(&img.data[12..16], &blue);
}

#[test]
fn key_out_image_no_match_is_zero_and_unchanged() {
    let data = vec![0x11u8, 0x22, 0x33, 0xFF, 0x44, 0x55, 0x66, 0xFF];
    let mut img = rgba_image(2, 1, data.clone());
    let key = KeyColor::from_argb([0x00, 0x77, 0x88, 0x99]);
    assert_eq!(key.key_out_image(&mut img), 0);
    assert_eq!(img.data, data, "no pixel touched when nothing matches");
}

#[test]
fn key_out_image_keys_all_pixels_for_solid_match() {
    // A solid key-coloured fill keys every pixel out.
    let key_rgb = [0x12u8, 0x34, 0x56];
    let mut data = Vec::new();
    for _ in 0..6 {
        data.extend_from_slice(&[key_rgb[0], key_rgb[1], key_rgb[2], 0xFF]);
    }
    let mut img = rgba_image(3, 2, data);
    let key = KeyColor::from_argb([0x00, key_rgb[0], key_rgb[1], key_rgb[2]]);
    assert_eq!(key.key_out_image(&mut img), 6);
    assert!(img.data.chunks_exact(4).all(|p| p[3] == 0));
}

#[test]
fn carry_then_apply_roundtrip_through_extension_area() {
    // Encode a 2×2 image that contains the key colour, carry the key
    // colour in the extension area, re-parse, and chroma-key the decoded
    // image with the recovered KeyColor — the carry-then-apply pattern
    // shared with gamma / pixel-aspect-ratio / colour-correction.
    let key_rgb = [0x00u8, 0xFF, 0x00]; // pure green
    let mut rgba = Vec::new();
    rgba.extend_from_slice(&[key_rgb[0], key_rgb[1], key_rgb[2], 0xFF]); // green
    rgba.extend_from_slice(&[0xFF, 0x00, 0x00, 0xFF]); // red
    rgba.extend_from_slice(&[key_rgb[0], key_rgb[1], key_rgb[2], 0xFF]); // green
    rgba.extend_from_slice(&[0x00, 0x00, 0xFF, 0xFF]); // blue
    let base = encode_tga_uncompressed(2, 2, &rgba).expect("base encodes");

    // Carry the key colour (on-disk A:R:G:B); alpha byte 0 per spec
    // recommendation for a no-alpha key.
    let ext = ExtensionAreaInput {
        key_color: [0x00, key_rgb[0], key_rgb[1], key_rgb[2]],
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).expect("extension encodes");

    // Recover the typed key colour from the re-parsed extension area.
    let area = parse_tga_extension_area(&file).expect("extension area present");
    let key = area.key_color_typed();
    assert_eq!(key.as_rgba8(), [key_rgb[0], key_rgb[1], key_rgb[2], 0x00]);

    // Decode the image and apply the recovered key.
    let mut img = parse_tga(&file).expect("decodes");
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    let keyed = key.key_out_image(&mut img);
    assert_eq!(keyed, 2, "two green pixels keyed out");

    // The two green pixels are now transparent; red and blue are opaque.
    let transparent: Vec<bool> = img.data.chunks_exact(4).map(|p| p[3] == 0).collect();
    let green_match: Vec<bool> = img
        .data
        .chunks_exact(4)
        .map(|p| p[0] == key_rgb[0] && p[1] == key_rgb[1] && p[2] == key_rgb[2])
        .collect();
    for (t, g) in transparent.iter().zip(green_match.iter()) {
        assert_eq!(t, g, "transparency matches green-pixel positions");
    }
}

#[test]
fn key_out_image_noop_for_non_rgba_formats() {
    // Gray8 / Rgb24 carry no alpha channel; keying is a no-op (returns 0).
    let mut gray = TgaImage {
        width: 2,
        height: 1,
        pixel_format: TgaPixelFormat::Gray8,
        data: vec![0x10, 0x20],
        pts: None,
    };
    let key = KeyColor::from_argb([0x00, 0x10, 0x10, 0x10]);
    assert_eq!(key.key_out_image(&mut gray), 0);
    assert_eq!(gray.data, vec![0x10, 0x20], "gray data untouched");

    let mut rgb = TgaImage {
        width: 1,
        height: 1,
        pixel_format: TgaPixelFormat::Rgb24,
        data: vec![0x10, 0x10, 0x10],
        pts: None,
    };
    assert_eq!(key.key_out_image(&mut rgb), 0);
    assert_eq!(rgb.data, vec![0x10, 0x10, 0x10], "rgb24 data untouched");
}

#[test]
fn unset_black_key_still_keys_black_pixels() {
    // An all-zero key colour is the spec's "unset" sentinel but also
    // means "key colour = black" — key_out_image treats it literally and
    // keys black RGB pixels (the documented ambiguity is the caller's to
    // resolve, not the apply-path's).
    let key = KeyColor::from_argb([0, 0, 0, 0]);
    assert!(key.is_unset());
    let mut img = rgba_image(2, 1, vec![0x00, 0x00, 0x00, 0xFF, 0x01, 0x00, 0x00, 0xFF]);
    assert_eq!(key.key_out_image(&mut img), 1);
    assert_eq!(&img.data[0..4], &[0x00, 0x00, 0x00, 0x00]);
    assert_eq!(&img.data[4..8], &[0x01, 0x00, 0x00, 0xFF]);
}
