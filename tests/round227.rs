//! Round 227 — typed extension-area accessors + gamma application.
//!
//! The TGA 2.0 extension area (§C.6) carries five small numeric fields
//! that the decoder has long parsed and the encoder has long re-written
//! byte-exactly, but which were only ever exposed as raw tuples /
//! arrays on [`TgaExtensionArea`]:
//!
//! * §C.6.4 — Key Color (`[A, R, G, B]`)
//! * §C.6.5 — Pixel Aspect Ratio (`(numerator, denominator)`)
//! * §C.6.6 — Gamma Value (`(numerator, denominator)`)
//! * §C.6.7 — Software Version (`(SHORT, BYTE-as-char)`)
//!
//! This round adds typed views — [`KeyColor`], [`PixelAspectRatio`],
//! [`GammaValue`], [`SoftwareVersion`] — with the same sentinel /
//! identity / apply semantics already established for
//! `AttributesType` (§C.6.13) and `TgaColourCorrectionTable` (§C.6.8).
//!
//! Tests cover:
//!
//! * Sentinel detection: every "field not set" value the spec calls
//!   out (all-zero quadruple, denominator == 0 for ratios, ASCII space
//!   release-letter) maps to `is_unset() == true`.
//! * Round-trip: the typed view re-emits the on-disk tuple byte-exactly,
//!   and the typed view round-trips through `TgaExtensionArea`.
//! * `GammaValue::apply_to_channel8` / `apply_to_rgba8` /
//!   `apply_to_image`: identity gamma is a bit-exact no-op; non-unity
//!   gamma maps a known channel + image; alpha is preserved; non-RGBA
//!   image formats apply through every byte; pathological gammas (zero
//!   denominator, malformed `f32`) are graceful no-ops.
//! * `PixelAspectRatio::corrected_display_*`: rationalised display
//!   dimensions for square-display-pixel resampling; sentinel cases.
//! * Parser convenience helpers: `parse_tga_key_color` /
//!   `parse_tga_pixel_aspect_ratio` / `parse_tga_gamma` /
//!   `parse_tga_software_version` return matching typed views straight
//!   from a freshly-encoded extension-bearing file, and `None` when no
//!   extension area is present.

use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga, parse_tga_extension_area,
    parse_tga_gamma, parse_tga_key_color, parse_tga_pixel_aspect_ratio, parse_tga_software_version,
    ExtensionAreaInput, GammaValue, KeyColor, PixelAspectRatio, SoftwareVersion, TgaImage,
    TgaPixelFormat,
};

// ---------------------------------------------------------------------------
// KeyColor (§C.6.4)
// ---------------------------------------------------------------------------

#[test]
fn key_color_default_is_unset() {
    let kc = KeyColor::default();
    assert!(kc.is_unset());
    assert!(!kc.has_alpha());
    assert_eq!(kc.to_argb(), [0; 4]);
}

#[test]
fn key_color_argb_layout_is_msb_first_per_spec_c64() {
    // Spec §C.6.4: "format is in A:R:G:B where 'A' (most significant
    // byte) is the alpha channel key color".
    let kc = KeyColor::from_argb([0xFE, 0x12, 0x34, 0x56]);
    assert_eq!(kc.a, 0xFE);
    assert_eq!(kc.r, 0x12);
    assert_eq!(kc.g, 0x34);
    assert_eq!(kc.b, 0x56);
    assert_eq!(kc.to_argb(), [0xFE, 0x12, 0x34, 0x56]);
    assert!(!kc.is_unset());
    assert!(kc.has_alpha());
}

#[test]
fn key_color_as_rgba8_reorders_for_framework() {
    // Decoded pixels are returned in `[R, G, B, A]` order — the typed
    // view exposes a matching helper so a caller chroma-keying a
    // decoded buffer doesn't have to swap bytes by hand.
    let kc = KeyColor::from_argb([0xFF, 0x10, 0x20, 0x30]);
    assert_eq!(kc.as_rgba8(), [0x10, 0x20, 0x30, 0xFF]);
}

#[test]
fn key_color_unset_sentinel_overlaps_intentional_black() {
    // Per spec §C.6.4: "Setting the field to all zeros is the same as
    // selecting a key color of black." The `is_unset` predicate is
    // therefore a textual is-all-zero check — distinguishing intent
    // requires out-of-band context.
    let kc = KeyColor::from_argb([0, 0, 0, 0]);
    assert!(kc.is_unset());
    assert!(!kc.has_alpha());
}

#[test]
fn key_color_round_trips_through_extension_area() {
    let input = ExtensionAreaInput {
        key_color: [0xCA, 0xFE, 0xBA, 0xBE],
        ..Default::default()
    };
    let base = encode_tga_uncompressed(1, 1, &[0x12, 0x34, 0x56, 0xFF]).unwrap();
    let bytes = encode_tga_with_extension(&base, &input).unwrap();
    let parsed = parse_tga_extension_area(&bytes).unwrap();
    assert_eq!(parsed.key_color, [0xCA, 0xFE, 0xBA, 0xBE]);
    assert_eq!(
        parsed.key_color_typed(),
        KeyColor::from_argb([0xCA, 0xFE, 0xBA, 0xBE])
    );
    // The convenience parser helper returns the same typed view.
    assert_eq!(parse_tga_key_color(&bytes), Some(parsed.key_color_typed()));
}

#[test]
fn parse_tga_key_color_is_none_without_extension_area() {
    let base = encode_tga_uncompressed(1, 1, &[0xAA, 0xBB, 0xCC, 0xFF]).unwrap();
    assert!(parse_tga_key_color(&base).is_none());
}

// ---------------------------------------------------------------------------
// PixelAspectRatio (§C.6.5)
// ---------------------------------------------------------------------------

#[test]
fn pixel_aspect_unset_sentinel_is_zero_zero() {
    let par = PixelAspectRatio::UNSET;
    assert_eq!(par, PixelAspectRatio::default());
    assert!(par.is_unset());
    assert!(par.is_square()); // 0 == 0 per the spec convention
    assert_eq!(par.as_f32(), None);
    assert_eq!(par.as_tuple(), (0, 0));
}

#[test]
fn pixel_aspect_square_non_zero_is_one() {
    let par = PixelAspectRatio::new(7, 7);
    assert!(!par.is_unset());
    assert!(par.is_square());
    assert_eq!(par.as_f32(), Some(1.0));
}

#[test]
fn pixel_aspect_non_square_ratio_returns_quotient() {
    // 4:3 NTSC-style sample (numerator > denominator → wide pixels).
    let par = PixelAspectRatio::new(4, 3);
    assert!(!par.is_square());
    let r = par.as_f32().unwrap();
    assert!((r - 4.0 / 3.0).abs() < 1e-6);
}

#[test]
fn pixel_aspect_zero_denominator_is_unset_per_spec_c65() {
    // Spec §C.6.5: "A zero in the second sub-field (denominator)
    // indicates that no pixel aspect ratio is specified." Even if the
    // numerator is non-zero, denominator==0 marks the field unused.
    let par = PixelAspectRatio::new(5, 0);
    assert_eq!(par.as_f32(), None);
}

#[test]
fn pixel_aspect_corrected_display_dimensions_for_non_square() {
    // A 4:3 pixel aspect ratio (wide pixels) stored on a 640×480
    // frame displays correctly at 640 × (480 × 3/4) = 640 × 360.
    let par = PixelAspectRatio::new(4, 3);
    assert_eq!(par.corrected_display_height(480), Some(360));
    // Symmetric helper: 640 × 4/3 = 853 (rounded).
    assert_eq!(par.corrected_display_width(640), Some(853));
}

#[test]
fn pixel_aspect_corrected_display_unset_returns_none() {
    let par = PixelAspectRatio::UNSET;
    assert_eq!(par.corrected_display_height(480), None);
    assert_eq!(par.corrected_display_width(640), None);
}

#[test]
fn pixel_aspect_corrected_display_square_returns_input() {
    // is_square non-zero → ratio == 1 → no resampling needed.
    let par = PixelAspectRatio::new(7, 7);
    assert_eq!(par.corrected_display_height(123), Some(123));
    assert_eq!(par.corrected_display_width(456), Some(456));
}

#[test]
fn pixel_aspect_round_trips_through_extension_area() {
    let input = ExtensionAreaInput {
        pixel_aspect_ratio: (16, 9),
        ..Default::default()
    };
    let base = encode_tga_uncompressed(1, 1, &[0, 0, 0, 0xFF]).unwrap();
    let bytes = encode_tga_with_extension(&base, &input).unwrap();
    let parsed = parse_tga_extension_area(&bytes).unwrap();
    assert_eq!(parsed.pixel_aspect_ratio, (16, 9));
    assert_eq!(
        parsed.pixel_aspect_ratio_typed(),
        PixelAspectRatio::new(16, 9)
    );
    assert_eq!(
        parse_tga_pixel_aspect_ratio(&bytes),
        Some(PixelAspectRatio::new(16, 9))
    );
}

// ---------------------------------------------------------------------------
// GammaValue (§C.6.6)
// ---------------------------------------------------------------------------

#[test]
fn gamma_unset_sentinel_is_zero_denominator() {
    let g = GammaValue::UNSET;
    assert_eq!(g, GammaValue::default());
    assert!(g.is_unset());
    assert!(!g.is_identity());
    assert_eq!(g.as_f32(), None);
}

#[test]
fn gamma_zero_denominator_is_unset_per_spec_c66() {
    // Spec §C.6.6: "If you decide to totally ignore this field, please
    // set the denominator (the second SHORT) to the value zero." Any
    // numerator with denominator=0 maps to UNSET.
    assert!(GammaValue::new(22, 0).is_unset());
    assert!(GammaValue::new(0, 0).is_unset());
}

#[test]
fn gamma_one_is_identity_per_spec_c66() {
    // Spec §C.6.6: "An uncorrected image (an image with no gamma)
    // should have the value 1.0 as the result. This may be accomplished
    // by placing the same, non-zero values in both positions (i.e., 1/1)."
    let g = GammaValue::ONE;
    assert_eq!(g, GammaValue::new(1, 1));
    assert!(!g.is_unset());
    assert!(g.is_identity());
    assert_eq!(g.as_f32(), Some(1.0));
}

#[test]
fn gamma_identity_is_any_equal_non_zero_pair() {
    let g = GammaValue::new(13, 13);
    assert!(g.is_identity());
    assert_eq!(g.as_f32(), Some(1.0));
}

#[test]
fn gamma_22_over_10_decodes_to_2_2() {
    // Standard CRT gamma.
    let g = GammaValue::new(22, 10);
    let v = g.as_f32().unwrap();
    assert!((v - 2.2).abs() < 1e-6);
    assert!(!g.is_identity());
}

#[test]
fn gamma_apply_to_channel8_is_noop_for_unset_and_identity() {
    let unset = GammaValue::UNSET;
    let id = GammaValue::ONE;
    for s in 0u8..=255 {
        assert_eq!(unset.apply_to_channel8(s), s, "unset must not modify {s}");
        assert_eq!(id.apply_to_channel8(s), s, "identity must not modify {s}");
    }
}

#[test]
fn gamma_apply_to_channel8_known_values() {
    // gamma == 2.0, x_in = 128 (0.50196…), x_in^2 ≈ 0.252 → 64.
    let g = GammaValue::new(2, 1);
    assert_eq!(g.apply_to_channel8(0), 0);
    assert_eq!(g.apply_to_channel8(255), 255);
    let mid = g.apply_to_channel8(128);
    // 128/255 = 0.50196..; 0.50196² = 0.25196..; ×255 = 64.249 → 64.
    assert_eq!(mid, 64);
}

#[test]
fn gamma_apply_to_rgba8_preserves_alpha() {
    let g = GammaValue::new(2, 1);
    let out = g.apply_to_rgba8([128, 128, 128, 200]);
    assert_eq!(out[0], 64);
    assert_eq!(out[1], 64);
    assert_eq!(out[2], 64);
    assert_eq!(out[3], 200, "alpha must be untouched");
}

#[test]
fn gamma_apply_to_image_rgba_walks_every_pixel() {
    let g = GammaValue::new(2, 1);
    let mut image = TgaImage {
        width: 2,
        height: 1,
        pixel_format: TgaPixelFormat::Rgba,
        data: vec![128, 128, 128, 200, 64, 64, 64, 100],
        pts: None,
    };
    g.apply_to_image(&mut image);
    // First pixel: 128 → 64; alpha preserved.
    assert_eq!(&image.data[..4], &[64, 64, 64, 200]);
    // Second pixel: 64/255 = 0.251; squared = 0.063; ×255 = 16.04 → 16.
    assert_eq!(&image.data[4..], &[16, 16, 16, 100]);
}

#[test]
fn gamma_apply_to_image_gray8_walks_every_byte() {
    let g = GammaValue::new(2, 1);
    let mut image = TgaImage {
        width: 2,
        height: 1,
        pixel_format: TgaPixelFormat::Gray8,
        data: vec![128, 255],
        pts: None,
    };
    g.apply_to_image(&mut image);
    assert_eq!(image.data, vec![64, 255]);
}

#[test]
fn gamma_apply_to_image_rgb24_walks_every_byte() {
    // Rgb24 has no alpha, so every byte is a colour channel.
    let g = GammaValue::new(2, 1);
    let mut image = TgaImage {
        width: 1,
        height: 1,
        pixel_format: TgaPixelFormat::Rgb24,
        data: vec![128, 128, 128],
        pts: None,
    };
    g.apply_to_image(&mut image);
    assert_eq!(image.data, vec![64, 64, 64]);
}

#[test]
fn gamma_apply_to_image_unset_and_identity_are_noops() {
    let original = vec![10u8, 20, 30, 200];
    for g in [GammaValue::UNSET, GammaValue::ONE, GammaValue::new(99, 99)] {
        let mut image = TgaImage {
            width: 1,
            height: 1,
            pixel_format: TgaPixelFormat::Rgba,
            data: original.clone(),
            pts: None,
        };
        g.apply_to_image(&mut image);
        assert_eq!(image.data, original, "{g:?} must be a no-op");
    }
}

#[test]
fn gamma_apply_to_image_malformed_is_noop() {
    // Negative or zero gamma can't be encoded by the (u16, u16)
    // representation, but huge / overflow combinations can — both
    // sides should be a no-op so a hostile file never blows up.
    let original = vec![10u8, 20, 30, 200];
    let mut image = TgaImage {
        width: 1,
        height: 1,
        pixel_format: TgaPixelFormat::Rgba,
        data: original.clone(),
        pts: None,
    };
    // denominator == 0 → unset → no-op.
    GammaValue::new(65535, 0).apply_to_image(&mut image);
    assert_eq!(image.data, original);
}

#[test]
fn gamma_round_trips_through_extension_area() {
    let input = ExtensionAreaInput {
        gamma: (22, 10),
        ..Default::default()
    };
    let base = encode_tga_uncompressed(1, 1, &[0, 0, 0, 0xFF]).unwrap();
    let bytes = encode_tga_with_extension(&base, &input).unwrap();
    let parsed = parse_tga_extension_area(&bytes).unwrap();
    assert_eq!(parsed.gamma, (22, 10));
    assert_eq!(parsed.gamma_typed(), GammaValue::new(22, 10));
    assert_eq!(parse_tga_gamma(&bytes), Some(GammaValue::new(22, 10)));
}

// ---------------------------------------------------------------------------
// SoftwareVersion (§C.6.7)
// ---------------------------------------------------------------------------

#[test]
fn software_version_unset_sentinel_is_zero_space() {
    let v = SoftwareVersion::UNSET;
    assert_eq!(v, SoftwareVersion::default());
    assert!(v.is_unset());
    assert_eq!(v.as_f32(), None);
    assert_eq!(v.as_tuple(), (0, ' '));
}

#[test]
fn software_version_117b_decodes_to_one_one_seven() {
    // Spec §C.6.7 example: "software version 4.17 would be the
    // integer value 417"; the round-trip works the same for any
    // (version, letter) pair.
    let v = SoftwareVersion::new(117, 'b');
    assert!(!v.is_unset());
    let n = v.as_f32().unwrap();
    assert!((n - 1.17).abs() < 1e-6);
}

#[test]
fn software_version_round_trips_through_extension_area() {
    let input = ExtensionAreaInput {
        software_version: (417, 'a'),
        ..Default::default()
    };
    let base = encode_tga_uncompressed(1, 1, &[0, 0, 0, 0xFF]).unwrap();
    let bytes = encode_tga_with_extension(&base, &input).unwrap();
    let parsed = parse_tga_extension_area(&bytes).unwrap();
    assert_eq!(parsed.software_version, (417, 'a'));
    assert_eq!(
        parsed.software_version_typed(),
        SoftwareVersion::new(417, 'a')
    );
    assert_eq!(
        parse_tga_software_version(&bytes),
        Some(SoftwareVersion::new(417, 'a'))
    );
}

// ---------------------------------------------------------------------------
// Convenience parser helpers — None without extension area.
// ---------------------------------------------------------------------------

#[test]
fn parser_helpers_return_none_on_tga_v1_file() {
    // No footer / extension area → every helper returns None.
    let base = encode_tga_uncompressed(2, 1, &[0, 0, 0, 0xFF, 0, 0, 0, 0xFF]).unwrap();
    assert!(parse_tga_key_color(&base).is_none());
    assert!(parse_tga_pixel_aspect_ratio(&base).is_none());
    assert!(parse_tga_gamma(&base).is_none());
    assert!(parse_tga_software_version(&base).is_none());
}

#[test]
fn parser_helpers_round_trip_against_full_extension_area() {
    let input = ExtensionAreaInput {
        key_color: [0xFF, 0x80, 0x40, 0x20],
        pixel_aspect_ratio: (8, 9),
        gamma: (18, 10),
        software_version: (842, 'c'),
        ..Default::default()
    };
    let base = encode_tga_uncompressed(1, 1, &[0xAA, 0xBB, 0xCC, 0xDD]).unwrap();
    let bytes = encode_tga_with_extension(&base, &input).unwrap();

    assert_eq!(
        parse_tga_key_color(&bytes),
        Some(KeyColor::from_argb([0xFF, 0x80, 0x40, 0x20]))
    );
    assert_eq!(
        parse_tga_pixel_aspect_ratio(&bytes),
        Some(PixelAspectRatio::new(8, 9))
    );
    assert_eq!(parse_tga_gamma(&bytes), Some(GammaValue::new(18, 10)));
    assert_eq!(
        parse_tga_software_version(&bytes),
        Some(SoftwareVersion::new(842, 'c'))
    );

    // Re-decoding the base image still works — the helpers are
    // strictly additive on the extension-area surface.
    let image = parse_tga(&bytes).unwrap();
    assert_eq!(image.width, 1);
    assert_eq!(image.height, 1);
    assert_eq!(image.pixel_format, TgaPixelFormat::Rgba);
}
