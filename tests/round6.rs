//! Round 6: attributes-type (spec §C.6.13) alpha-channel *interpretation*.
//!
//! Round 4 landed the typed `AttributesType` view + the
//! `has_meaningful_alpha` / `is_premultiplied` predicates, but treated
//! the byte as opaque metadata. This round makes the attribute
//! *actionable*: `AttributesType::normalize_rgba8` maps a single decoded
//! pixel to straight (non-premultiplied) alpha, and
//! `AttributesType::apply_to_image` rewrites a decoded `TgaImage` in
//! place. The decoder side surfaces the typed attribute directly with
//! `parse_tga_attributes_type`.
//!
//! The pre-multiplied path follows the spec example verbatim: straight
//! `(a,r,g,b) = (0.5, 1, 0, 0)` is stored pre-multiplied as
//! `(0.5, 0.5, 0, 0)`; un-premultiply recovers the straight colour by
//! dividing each channel by the normalised alpha.

use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga, parse_tga_attributes_type,
    AttributesType, ExtensionAreaInput, TgaImage, TgaPixelFormat, TgaTimestamp,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rgba_image(width: u32, height: u32, data: Vec<u8>) -> TgaImage {
    TgaImage {
        width,
        height,
        pixel_format: TgaPixelFormat::Rgba,
        data,
        pts: None,
    }
}

fn baseline_ext() -> ExtensionAreaInput {
    ExtensionAreaInput {
        author_name: String::new(),
        author_comment: [String::new(), String::new(), String::new(), String::new()],
        timestamp: TgaTimestamp::default(),
        job_name: String::new(),
        job_time: (0, 0, 0),
        software_id: "oxideav-tga".to_string(),
        software_version: (100, 'a'),
        key_color: [0; 4],
        pixel_aspect_ratio: (0, 0),
        gamma: (0, 0),
        attributes_type: 3,
        postage_stamp: None,
        colour_correction_table: None,
        scan_line_table: None,
        developer_tags: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// normalize_rgba8 — per-pixel attribute interpretation.
// ---------------------------------------------------------------------------

#[test]
fn no_alpha_forces_opaque() {
    // Value 0: "no Alpha data included" — the byte is not meaningful, so
    // the pixel is forced opaque while the colour channels are kept.
    let px = [10, 20, 30, 0];
    assert_eq!(
        AttributesType::NoAlpha.normalize_rgba8(px),
        [10, 20, 30, 255]
    );
    assert_eq!(
        AttributesType::NoAlpha.normalize_rgba8([1, 2, 3, 128]),
        [1, 2, 3, 255]
    );
}

#[test]
fn undefined_ignore_forces_opaque() {
    // Value 1: "undefined data in the Alpha field, can be ignored".
    let px = [200, 100, 50, 17];
    assert_eq!(
        AttributesType::UndefinedIgnore.normalize_rgba8(px),
        [200, 100, 50, 255]
    );
}

#[test]
fn undefined_retain_passes_through() {
    // Value 2: "undefined data … but should be retained" — verbatim.
    let px = [200, 100, 50, 17];
    assert_eq!(AttributesType::UndefinedRetain.normalize_rgba8(px), px);
}

#[test]
fn useful_alpha_passes_through() {
    // Value 3: already-straight alpha — verbatim.
    let px = [200, 100, 50, 128];
    assert_eq!(AttributesType::UsefulAlpha.normalize_rgba8(px), px);
}

#[test]
fn reserved_passes_through_unchanged() {
    // Unknown attributes byte: no defined semantics, so pass through so
    // nothing is silently corrupted.
    let px = [10, 20, 30, 40];
    assert_eq!(AttributesType::Reserved(99).normalize_rgba8(px), px);
}

#[test]
fn premultiplied_spec_example_unmultiplies() {
    // Spec §C.6.13 example (component values normalised 0..1):
    //   straight (a,r,g,b) = (0.5, 1, 0, 0)
    //   stored  pre-mult   = (0.5, 0.5, 0, 0)
    // In 8-bit terms: A = 128 (~0.5), stored R = 128 (0.5), G = B = 0.
    // Un-premultiply: straight R = 128 * 255 / 128 = 255 (= 1.0).
    let stored = [128, 0, 0, 128];
    let straight = AttributesType::PremultipliedAlpha.normalize_rgba8(stored);
    assert_eq!(straight, [255, 0, 0, 128]);
}

#[test]
fn premultiplied_opaque_pixel_is_identity() {
    // A fully-opaque pre-multiplied pixel (A = 255) is its own straight
    // form: stored = straight * 1.0.
    let px = [200, 100, 50, 255];
    assert_eq!(AttributesType::PremultipliedAlpha.normalize_rgba8(px), px);
}

#[test]
fn premultiplied_transparent_pixel_is_black() {
    // A == 0 carries no recoverable colour (division by zero); the spec's
    // model has every channel scaled to zero, so straight is transparent
    // black.
    assert_eq!(
        AttributesType::PremultipliedAlpha.normalize_rgba8([0, 0, 0, 0]),
        [0, 0, 0, 0]
    );
    // Even if a malformed file left non-zero colour under A == 0, it is
    // unrecoverable and clamped to transparent black.
    assert_eq!(
        AttributesType::PremultipliedAlpha.normalize_rgba8([60, 60, 60, 0]),
        [0, 0, 0, 0]
    );
}

#[test]
fn premultiplied_clamps_overshoot() {
    // A malformed file may store a colour channel that exceeds its alpha
    // (impossible for a true pre-multiplied source). Division would push
    // the straight value past 255; it must clamp.
    let stored = [200, 100, 0, 100];
    let out = AttributesType::PremultipliedAlpha.normalize_rgba8(stored);
    // R: 200*255/100 = 510 -> clamp 255; G: 100*255/100 = 255; B: 0.
    assert_eq!(out, [255, 255, 0, 100]);
}

#[test]
fn premultiplied_rounds_to_nearest() {
    // A = 100, stored R = 39 -> 39*255/100 = 99.45 -> nearest 99.
    // (round-half-up: (39*255 + 50) / 100 = (9945 + 50)/100 = 99).
    let out = AttributesType::PremultipliedAlpha.normalize_rgba8([39, 0, 0, 100]);
    assert_eq!(out[0], 99);
    // A = 2, stored R = 1 -> 1*255/2 = 127.5 -> round-half-up 128.
    let out2 = AttributesType::PremultipliedAlpha.normalize_rgba8([1, 0, 0, 2]);
    assert_eq!(out2[0], 128);
}

// ---------------------------------------------------------------------------
// apply_to_image — in-place across a decoded image.
// ---------------------------------------------------------------------------

#[test]
fn apply_useful_alpha_is_noop() {
    let original = vec![10, 20, 30, 40, 200, 100, 0, 128];
    let mut img = rgba_image(2, 1, original.clone());
    AttributesType::UsefulAlpha.apply_to_image(&mut img);
    assert_eq!(img.data, original, "useful alpha must be bit-exact");
}

#[test]
fn apply_undefined_retain_is_noop() {
    let original = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let mut img = rgba_image(2, 1, original.clone());
    AttributesType::UndefinedRetain.apply_to_image(&mut img);
    assert_eq!(img.data, original);
}

#[test]
fn apply_no_alpha_forces_every_pixel_opaque() {
    let mut img = rgba_image(2, 1, vec![10, 20, 30, 0, 40, 50, 60, 99]);
    AttributesType::NoAlpha.apply_to_image(&mut img);
    assert_eq!(img.data, vec![10, 20, 30, 255, 40, 50, 60, 255]);
}

#[test]
fn apply_premultiplied_unmultiplies_every_pixel() {
    // Two pixels: spec example (128,0,0,128) and an opaque (200,100,50,255).
    let mut img = rgba_image(2, 1, vec![128, 0, 0, 128, 200, 100, 50, 255]);
    AttributesType::PremultipliedAlpha.apply_to_image(&mut img);
    assert_eq!(img.data, vec![255, 0, 0, 128, 200, 100, 50, 255]);
}

#[test]
fn apply_leaves_gray8_untouched() {
    let mut img = TgaImage {
        width: 3,
        height: 1,
        pixel_format: TgaPixelFormat::Gray8,
        data: vec![10, 100, 200],
        pts: None,
    };
    // No alpha channel to interpret — every variant is a no-op.
    AttributesType::NoAlpha.apply_to_image(&mut img);
    assert_eq!(img.data, vec![10, 100, 200]);
    AttributesType::PremultipliedAlpha.apply_to_image(&mut img);
    assert_eq!(img.data, vec![10, 100, 200]);
}

#[test]
fn apply_leaves_rgb24_untouched() {
    let mut img = TgaImage {
        width: 2,
        height: 1,
        pixel_format: TgaPixelFormat::Rgb24,
        data: vec![1, 2, 3, 4, 5, 6],
        pts: None,
    };
    AttributesType::PremultipliedAlpha.apply_to_image(&mut img);
    assert_eq!(img.data, vec![1, 2, 3, 4, 5, 6]);
}

// ---------------------------------------------------------------------------
// parse_tga_attributes_type — decoder-side surface.
// ---------------------------------------------------------------------------

#[test]
fn parse_attributes_type_absent_without_extension() {
    // A bare TGA 1.0 file (no footer / extension area) declares no
    // attributes type.
    let base = encode_tga_uncompressed(2, 2, &[255u8; 16]).unwrap();
    assert_eq!(parse_tga_attributes_type(&base), None);
}

#[test]
fn parse_attributes_type_recovers_premultiplied() {
    let base = encode_tga_uncompressed(2, 2, &[128u8; 16]).unwrap();
    let ext_in = ExtensionAreaInput {
        attributes_type: 4,
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    assert_eq!(
        parse_tga_attributes_type(&full),
        Some(AttributesType::PremultipliedAlpha)
    );
}

#[test]
fn parse_attributes_type_recovers_reserved() {
    let base = encode_tga_uncompressed(1, 1, &[1, 2, 3, 4]).unwrap();
    let ext_in = ExtensionAreaInput {
        attributes_type: 200,
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    assert_eq!(
        parse_tga_attributes_type(&full),
        Some(AttributesType::Reserved(200))
    );
}

// ---------------------------------------------------------------------------
// End-to-end: encode pre-multiplied, decode, read the attribute, normalise.
// ---------------------------------------------------------------------------

#[test]
fn end_to_end_decode_then_normalize_premultiplied() {
    // Build a 32-bit RGBA image whose colour is already pre-multiplied
    // (spec example pixel repeated): stored (128, 0, 0, 128).
    let mut stored = Vec::new();
    for _ in 0..(3 * 3) {
        stored.extend_from_slice(&[128, 0, 0, 128]);
    }
    let base = encode_tga_uncompressed(3, 3, &stored).unwrap();
    let ext_in = ExtensionAreaInput {
        attributes_type: 4,
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    let mut img = parse_tga(&full).expect("decode");
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    let attr = parse_tga_attributes_type(&full).expect("recover attributes type");
    assert_eq!(attr, AttributesType::PremultipliedAlpha);

    attr.apply_to_image(&mut img);
    // Every stored (128,0,0,128) un-premultiplies to straight (255,0,0,128).
    for px in img.data.chunks_exact(4) {
        assert_eq!(px, &[255, 0, 0, 128]);
    }
}

#[test]
fn end_to_end_decode_then_normalize_no_alpha() {
    // A 32-bit file written with attributes_type = 0: the alpha bytes are
    // not meaningful and must be forced opaque on normalisation.
    let mut stored = Vec::new();
    for _ in 0..(2 * 2) {
        stored.extend_from_slice(&[40, 80, 120, 7]);
    }
    let base = encode_tga_uncompressed(2, 2, &stored).unwrap();
    let ext_in = ExtensionAreaInput {
        attributes_type: 0,
        ..baseline_ext()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    let mut img = parse_tga(&full).expect("decode");
    let attr = parse_tga_attributes_type(&full).expect("recover attributes type");
    assert_eq!(attr, AttributesType::NoAlpha);
    attr.apply_to_image(&mut img);
    for px in img.data.chunks_exact(4) {
        assert_eq!(px, &[40, 80, 120, 255]);
    }
}
