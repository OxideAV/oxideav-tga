//! Round 378 — composed, spec-ordered TGA metadata-application pipeline.
//!
//! Every §C.6 metadata apply-path already existed as an *isolated* opt-in
//! helper (`GammaValue::apply_to_image`, `KeyColor::key_out_image`,
//! `AttributesType::apply_to_image`, `TgaColourCorrectionTable::apply_to_image`,
//! `PixelAspectRatio::apply_to_image`, `resolve_alpha_*`). `parse_tga`
//! deliberately applies *none* of them — it returns the raw decoded
//! samples. This round adds the composed counterpart:
//! `decode_tga_for_display(input, &TgaDisplayOptions)` runs `parse_tga`
//! then applies the file's own metadata in the spec-faithful order
//! (alpha resolution → tone curve → key colour → pixel aspect), with each
//! pass independently gated.
//!
//! The tests pin: (a) `parse_tga` stays raw, (b) the composed path applies
//! each pass, (c) order-dependence is observable (premultiplied alpha ×
//! gamma decode differently in the wrong order), (d) each toggle gates its
//! pass, (e) defaults are spec-correct, (f) all three pixel formats no-op
//! correctly where they carry no alpha, (g) a metadata-free TGA 1.0 file
//! is byte-identical between `parse_tga` and the composed default path.

use oxideav_tga::{
    decode_tga_for_display, decode_tga_for_display_reported, decode_tga_frame,
    encode_tga_grayscale, encode_tga_uncompressed, encode_tga_uncompressed_rgb24,
    encode_tga_with_extension, parse_tga, set_image_origin, AlphaResolution, AttributesType,
    DeveloperTagInput, ExtensionAreaInput, GammaValue, ImageOrigin, TgaDisplayOptions,
    TgaPixelFormat, ToneApplied,
};

/// Gamma channel reference: `round((x/255)^g * 255)`, matching
/// `GammaValue::apply_to_channel8`.
fn gamma_channel(x: u8, g: f32) -> u8 {
    let y = (x as f32 / 255.0).powf(g) * 255.0;
    y.round().clamp(0.0, 255.0) as u8
}

// ---------------------------------------------------------------------------
// (a) parse_tga stays raw — composed NONE == parse_tga, bit for bit
// ---------------------------------------------------------------------------

#[test]
fn passthrough_options_equal_parse_tga() {
    // A 32-bpp file with a gamma + key colour in its extension area. Under
    // NONE options the composed path must return exactly what parse_tga does.
    let rgba: Vec<u8> = (0..4 * 4).map(|i| (i * 7 % 250) as u8).collect::<Vec<u8>>();
    let base = encode_tga_uncompressed(2, 2, &rgba).unwrap();
    let ext = ExtensionAreaInput {
        gamma: (1, 2), // gamma 0.5 — a real, non-identity curve
        key_color: [0x00, 0x10, 0x20, 0x30],
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    let raw = parse_tga(&file).unwrap();
    let pass = decode_tga_for_display(&file, &TgaDisplayOptions::NONE).unwrap();
    assert_eq!(raw.data, pass.data, "NONE options must not touch pixels");
    assert_eq!(raw.width, pass.width);
    assert_eq!(raw.height, pass.height);
    assert_eq!(raw.pixel_format, pass.pixel_format);
}

#[test]
fn passthrough_is_default_is_all_constants() {
    assert!(TgaDisplayOptions::NONE.is_passthrough());
    assert!(!TgaDisplayOptions::ALL.is_passthrough());
    assert_eq!(TgaDisplayOptions::default(), TgaDisplayOptions::ALL);
}

// ---------------------------------------------------------------------------
// (g) metadata-free TGA 1.0 file: raw == composed-default, byte identical
// ---------------------------------------------------------------------------

#[test]
fn metadata_free_file_default_equals_raw() {
    // A plain 24-bpp TGA 1.0 file (no footer, no extension area). The
    // default (ALL) composed path must reproduce parse_tga exactly: no
    // attributes type, no gamma, no key colour, square pixels.
    let rgb: Vec<u8> = (0..3 * 9).map(|i| (i * 5 % 240) as u8).collect();
    let file = encode_tga_uncompressed_rgb24(3, 3, &rgb).unwrap();
    let raw = parse_tga(&file).unwrap();
    let disp = decode_tga_for_display(&file, &TgaDisplayOptions::default()).unwrap();
    assert_eq!(raw.data, disp.data);
    assert_eq!((raw.width, raw.height), (disp.width, disp.height));

    let (_, report) =
        decode_tga_for_display_reported(&file, &TgaDisplayOptions::default()).unwrap();
    // No extension area → alpha resolution falls back to the header bits.
    assert!(matches!(report.alpha, AlphaResolution::HeaderBits(_)));
    assert!(matches!(report.tone, ToneApplied::None));
    assert_eq!(report.keyed_pixels, 0);
    assert!(!report.resampled);
    assert!(report.is_colour_geometry_noop());
}

// ---------------------------------------------------------------------------
// (b) each pass actually applies: gamma
// ---------------------------------------------------------------------------

#[test]
fn tone_pass_applies_gamma() {
    // Opaque 24→24bpp file so alpha resolution is a no-op; gamma is the
    // only colour change. Use a grey ramp.
    let rgb: Vec<u8> = vec![10, 80, 200, 30, 120, 240, 5, 250, 128];
    let file_base = encode_tga_uncompressed_rgb24(3, 1, &rgb).unwrap();
    let ext = ExtensionAreaInput {
        gamma: (1, 2), // 0.5
        ..Default::default()
    };
    let file = encode_tga_with_extension(&file_base, &ext).unwrap();

    let raw = parse_tga(&file).unwrap();
    let (disp, report) =
        decode_tga_for_display_reported(&file, &TgaDisplayOptions::default()).unwrap();

    // RGB24 input is decoded to RGBA (alpha forced 0xFF), so each pixel is 4 bytes.
    assert_eq!(disp.pixel_format, TgaPixelFormat::Rgba);
    assert!(matches!(report.tone, ToneApplied::Gamma(_)));
    for (r, d) in raw.data.chunks_exact(4).zip(disp.data.chunks_exact(4)) {
        assert_eq!(d[0], gamma_channel(r[0], 0.5));
        assert_eq!(d[1], gamma_channel(r[1], 0.5));
        assert_eq!(d[2], gamma_channel(r[2], 0.5));
        assert_eq!(d[3], r[3]); // alpha untouched
    }
}

// ---------------------------------------------------------------------------
// (b) each pass actually applies: key colour
// ---------------------------------------------------------------------------

#[test]
fn key_color_pass_keys_out_matching_pixels() {
    // 2×2: two magenta pixels + two others. Key colour magenta.
    let magenta = [0xFFu8, 0x00, 0xFF, 0xFF];
    let red = [0xFFu8, 0x00, 0x00, 0xFF];
    let blue = [0x00u8, 0x00, 0xFF, 0xFF];
    let mut data = Vec::new();
    data.extend_from_slice(&magenta);
    data.extend_from_slice(&magenta);
    data.extend_from_slice(&red);
    data.extend_from_slice(&blue);
    let base = encode_tga_uncompressed(2, 2, &data).unwrap();
    let ext = ExtensionAreaInput {
        key_color: [0x00, 0xFF, 0x00, 0xFF], // A:R:G:B → magenta
        attributes_type: 3,                  // useful alpha so resolution leaves alpha alone
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    let (disp, report) =
        decode_tga_for_display_reported(&file, &TgaDisplayOptions::default()).unwrap();
    assert_eq!(report.keyed_pixels, 2);
    assert!(report.applied_key_color());
    // Magenta pixels now transparent; colour bytes untouched.
    assert_eq!(&disp.data[0..4], &[0xFF, 0x00, 0xFF, 0x00]);
    assert_eq!(&disp.data[4..8], &[0xFF, 0x00, 0xFF, 0x00]);
    // Red / blue keep alpha 0xFF.
    assert_eq!(disp.data[11], 0xFF);
    assert_eq!(disp.data[15], 0xFF);
}

// ---------------------------------------------------------------------------
// (b) each pass actually applies: pixel aspect resample (geometry)
// ---------------------------------------------------------------------------

#[test]
fn pixel_aspect_pass_resamples_geometry() {
    // 2×2 RGBA, wide pixels (2:1) → display width doubles to 4.
    let data: Vec<u8> = vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255];
    let base = encode_tga_uncompressed(2, 2, &data).unwrap();
    let ext = ExtensionAreaInput {
        pixel_aspect_ratio: (2, 1),
        attributes_type: 3, // useful alpha — leave pixels alone
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    let raw = parse_tga(&file).unwrap();
    assert_eq!((raw.width, raw.height), (2, 2));
    let (disp, report) =
        decode_tga_for_display_reported(&file, &TgaDisplayOptions::default()).unwrap();
    assert!(report.resampled);
    assert!(disp.width > raw.width, "wide pixels stretch width");
    assert_eq!(disp.height, raw.height);
}

// ---------------------------------------------------------------------------
// (c) ORDER-DEPENDENCE: premultiplied alpha × gamma differs by order
// ---------------------------------------------------------------------------

#[test]
fn premultiplied_then_gamma_order_is_observable() {
    // A 32-bpp file with attributes_type = 4 (pre-multiplied alpha) AND a
    // non-identity gamma. The pipeline un-premultiplies FIRST (recovering
    // straight colour) THEN applies gamma. Doing gamma first then
    // un-premultiplying would yield different bytes — we prove the
    // difference by computing both orders by hand and confirming the
    // pipeline matches the spec order, not the reverse.
    //
    // Pick a pixel where the stored (pre-multiplied) red sits well below
    // the alpha so neither order saturates — making the two orders diverge.
    // A=200, stored R=50: un-premultiply→64, gamma(0.5)→128; the reverse
    // (gamma then un-premultiply) lands on a different value.
    let a = 200u8;
    let r_stored = 50u8; // already premultiplied
    let pixel = [r_stored, 0, 0, a];
    let base = encode_tga_uncompressed(1, 1, &pixel).unwrap();
    let ext = ExtensionAreaInput {
        attributes_type: 4, // premultiplied
        gamma: (1, 2),      // 0.5
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    let (disp, report) =
        decode_tga_for_display_reported(&file, &TgaDisplayOptions::default()).unwrap();
    assert!(matches!(
        report.alpha,
        AlphaResolution::AttributesType(AttributesType::PremultipliedAlpha)
    ));
    assert!(matches!(report.tone, ToneApplied::Gamma(_)));

    // Spec order (pipeline): un-premultiply THEN gamma.
    let straight = AttributesType::PremultipliedAlpha.normalize_rgba8(pixel);
    let mut spec_order = straight;
    spec_order[0] = gamma_channel(straight[0], 0.5);
    spec_order[1] = gamma_channel(straight[1], 0.5);
    spec_order[2] = gamma_channel(straight[2], 0.5);

    // Reversed order: gamma THEN un-premultiply.
    let mut gamma_first = pixel;
    gamma_first[0] = gamma_channel(pixel[0], 0.5);
    gamma_first[1] = gamma_channel(pixel[1], 0.5);
    gamma_first[2] = gamma_channel(pixel[2], 0.5);
    let rev_order = AttributesType::PremultipliedAlpha.normalize_rgba8(gamma_first);

    assert_eq!(
        &disp.data[0..4],
        &spec_order,
        "pipeline must follow spec order (un-premultiply then gamma)"
    );
    assert_ne!(
        spec_order, rev_order,
        "the two orders must differ for this fixture (proves order matters)"
    );
}

// ---------------------------------------------------------------------------
// (b/spec) tone alternatives: colour-correction wins over gamma (never both)
// ---------------------------------------------------------------------------

#[test]
fn colour_correction_takes_precedence_over_gamma() {
    use oxideav_tga::TgaColourCorrectionTable;
    // Build a non-identity CCT: invert the red curve. Also set a gamma. The
    // pipeline must apply the CCT and report ColourCorrection, NOT Gamma —
    // the spec treats them as alternatives.
    let rgb: Vec<u8> = vec![40, 80, 120];
    let base = encode_tga_uncompressed_rgb24(1, 1, &rgb).unwrap();

    let mut cct = TgaColourCorrectionTable::default(); // identity
                                                       // Invert red channel: entry i maps to (255-i) scaled to u16 high byte.
    for i in 0..256 {
        cct.red[i] = ((255 - i) as u16) << 8 | ((255 - i) as u16);
    }
    let ext = ExtensionAreaInput {
        gamma: (1, 2),
        colour_correction_table: Some(cct),
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    let (disp, report) =
        decode_tga_for_display_reported(&file, &TgaDisplayOptions::default()).unwrap();
    assert!(
        matches!(report.tone, ToneApplied::ColourCorrection),
        "CCT must win over gamma; got {:?}",
        report.tone
    );
    // Red inverted (255-40 = 215), green/blue identity (high byte of dup).
    assert_eq!(disp.data[0], 215);
    assert_eq!(disp.data[1], 80);
    assert_eq!(disp.data[2], 120);
}

// ---------------------------------------------------------------------------
// (d) each toggle independently gates its pass
// ---------------------------------------------------------------------------

#[test]
fn tone_toggle_gates_gamma() {
    let rgb: Vec<u8> = vec![10, 80, 200];
    let base = encode_tga_uncompressed_rgb24(1, 1, &rgb).unwrap();
    let ext = ExtensionAreaInput {
        gamma: (1, 2),
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    let raw = parse_tga(&file).unwrap();
    // tone OFF, alpha ON: pixels equal raw (gamma skipped).
    let opts = TgaDisplayOptions::NONE.with_resolve_alpha(true);
    let (disp, report) = decode_tga_for_display_reported(&file, &opts).unwrap();
    assert!(matches!(report.tone, ToneApplied::None));
    assert_eq!(disp.data, raw.data);

    // tone ON: gamma applies.
    let opts2 = opts.with_apply_tone(true);
    let (disp2, report2) = decode_tga_for_display_reported(&file, &opts2).unwrap();
    assert!(matches!(report2.tone, ToneApplied::Gamma(_)));
    assert_ne!(disp2.data, raw.data);
}

#[test]
fn key_color_toggle_gates_keying() {
    let magenta = [0xFFu8, 0x00, 0xFF, 0xFF];
    let base = encode_tga_uncompressed(1, 1, &magenta).unwrap();
    let ext = ExtensionAreaInput {
        key_color: [0x00, 0xFF, 0x00, 0xFF],
        attributes_type: 3,
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    // key OFF
    let opts = TgaDisplayOptions::NONE;
    let (disp, report) = decode_tga_for_display_reported(&file, &opts).unwrap();
    assert_eq!(report.keyed_pixels, 0);
    assert_eq!(disp.data[3], 0xFF);

    // key ON
    let opts2 = TgaDisplayOptions::NONE.with_apply_key_color(true);
    let (disp2, report2) = decode_tga_for_display_reported(&file, &opts2).unwrap();
    assert_eq!(report2.keyed_pixels, 1);
    assert_eq!(disp2.data[3], 0x00);
}

#[test]
fn pixel_aspect_toggle_gates_resample() {
    let data: Vec<u8> = vec![1, 2, 3, 255, 4, 5, 6, 255];
    let base = encode_tga_uncompressed(2, 1, &data).unwrap();
    let ext = ExtensionAreaInput {
        pixel_aspect_ratio: (2, 1),
        attributes_type: 3,
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();
    let raw = parse_tga(&file).unwrap();

    // aspect OFF
    let opts = TgaDisplayOptions::NONE;
    let (disp, report) = decode_tga_for_display_reported(&file, &opts).unwrap();
    assert!(!report.resampled);
    assert_eq!(disp.width, raw.width);

    // aspect ON
    let opts2 = TgaDisplayOptions::NONE.with_apply_pixel_aspect(true);
    let (disp2, report2) = decode_tga_for_display_reported(&file, &opts2).unwrap();
    assert!(report2.resampled);
    assert!(disp2.width > raw.width);
}

#[test]
fn alpha_toggle_gates_resolution() {
    // 32-bpp file, attributes_type = 0 (no alpha) → resolution forces opaque.
    // Make the stored alpha non-0xFF so the force is observable.
    let pixel = [10u8, 20, 30, 0x40];
    let base = encode_tga_uncompressed(1, 1, &pixel).unwrap();
    let ext = ExtensionAreaInput {
        attributes_type: 0, // no alpha → force opaque
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    // alpha OFF: stored alpha passes through.
    let opts = TgaDisplayOptions::NONE;
    let (disp, report) = decode_tga_for_display_reported(&file, &opts).unwrap();
    assert!(matches!(report.alpha, AlphaResolution::Skipped));
    assert_eq!(disp.data[3], 0x40);

    // alpha ON: forced opaque.
    let opts2 = TgaDisplayOptions::NONE.with_resolve_alpha(true);
    let (disp2, report2) = decode_tga_for_display_reported(&file, &opts2).unwrap();
    assert!(matches!(
        report2.alpha,
        AlphaResolution::AttributesType(AttributesType::NoAlpha)
    ));
    assert_eq!(disp2.data[3], 0xFF);
}

// ---------------------------------------------------------------------------
// (f) alpha-less pixel formats: passes that touch alpha are no-ops
// ---------------------------------------------------------------------------

#[test]
fn grayscale_key_color_is_noop() {
    // Gray8 file with a key colour — keying has no alpha to set, so 0 keyed.
    let gray: Vec<u8> = vec![10, 20, 30, 40];
    let base = encode_tga_grayscale(2, 2, &gray).unwrap();
    let ext = ExtensionAreaInput {
        key_color: [0x00, 10, 10, 10],
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();
    let (disp, report) =
        decode_tga_for_display_reported(&file, &TgaDisplayOptions::default()).unwrap();
    assert_eq!(disp.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!(report.keyed_pixels, 0);
}

#[test]
fn grayscale_gamma_still_applies() {
    // Gamma is a colour-channel transform that DOES apply to Gray8 luma.
    let gray: Vec<u8> = vec![10, 80, 200, 250];
    let base = encode_tga_grayscale(2, 2, &gray).unwrap();
    let ext = ExtensionAreaInput {
        gamma: (1, 2),
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();
    let raw = parse_tga(&file).unwrap();
    let (disp, report) =
        decode_tga_for_display_reported(&file, &TgaDisplayOptions::default()).unwrap();
    assert!(matches!(report.tone, ToneApplied::Gamma(_)));
    for (r, d) in raw.data.iter().zip(disp.data.iter()) {
        assert_eq!(*d, gamma_channel(*r, 0.5));
    }
}

// ---------------------------------------------------------------------------
// (e) defaults are spec-correct: full pipeline on a rich file
// ---------------------------------------------------------------------------

#[test]
fn full_default_pipeline_runs_every_pass() {
    // 32-bpp file with premultiplied alpha + gamma + key colour + 2:1 aspect.
    // Default options run all four passes in order.
    let pixels: Vec<u8> = vec![
        128, 0, 0, 128, // premultiplied half-red
        200, 100, 50, 255, // opaque
    ];
    let base = encode_tga_uncompressed(2, 1, &pixels).unwrap();
    let ext = ExtensionAreaInput {
        attributes_type: 4,
        gamma: (1, 2),
        key_color: [0x00, 255, 255, 255], // white — won't match either pixel
        pixel_aspect_ratio: (2, 1),
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    let (disp, report) =
        decode_tga_for_display_reported(&file, &TgaDisplayOptions::default()).unwrap();
    assert!(matches!(
        report.alpha,
        AlphaResolution::AttributesType(AttributesType::PremultipliedAlpha)
    ));
    assert!(matches!(report.tone, ToneApplied::Gamma(_)));
    assert_eq!(report.keyed_pixels, 0); // white key matches nothing
    assert!(report.resampled);
    // 2:1 aspect doubles width from 2 → 4.
    assert_eq!(disp.width, 4);
    assert_eq!(disp.height, 1);
    assert!(!report.is_colour_geometry_noop());
}

// ---------------------------------------------------------------------------
// builder ergonomics
// ---------------------------------------------------------------------------

#[test]
fn builder_composes_individual_toggles() {
    let opts = TgaDisplayOptions::NONE
        .with_resolve_alpha(true)
        .with_apply_tone(true);
    assert!(opts.resolve_alpha);
    assert!(opts.apply_tone);
    assert!(!opts.apply_key_color);
    assert!(!opts.apply_pixel_aspect);
    assert!(!opts.is_passthrough());
}

#[test]
fn gamma_unset_is_no_tone() {
    // Extension area present but gamma is the unset sentinel and no CCT.
    let rgb: Vec<u8> = vec![10, 80, 200];
    let base = encode_tga_uncompressed_rgb24(1, 1, &rgb).unwrap();
    let ext = ExtensionAreaInput {
        gamma: GammaValue::UNSET.as_tuple(),
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();
    let (_disp, report) =
        decode_tga_for_display_reported(&file, &TgaDisplayOptions::default()).unwrap();
    assert!(matches!(report.tone, ToneApplied::None));
}

// ---------------------------------------------------------------------------
// TgaDecodedFrame bundle: finalized pixels + placement / preview / dev tags
// ---------------------------------------------------------------------------

#[test]
fn decoded_frame_bundles_image_matching_display_path() {
    // The bundle's image must equal decode_tga_for_display for the same opts.
    let pixel = [10u8, 20, 30, 0x40];
    let base = encode_tga_uncompressed(1, 1, &pixel).unwrap();
    let ext = ExtensionAreaInput {
        attributes_type: 0, // force opaque under defaults
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    let want = decode_tga_for_display(&file, &TgaDisplayOptions::default()).unwrap();
    let frame = decode_tga_frame(&file, &TgaDisplayOptions::default()).unwrap();
    assert_eq!(frame.image.data, want.data);
    assert_eq!(frame.image.data[3], 0xFF); // resolved opaque
                                           // No placement / stamp / dev tags in this file.
    assert_eq!(frame.screen_origin, ImageOrigin::ORIGIN);
    assert!(!frame.has_screen_offset());
    assert!(!frame.has_postage_stamp());
    assert!(frame.developer_area.is_none() || frame.developer_area.as_ref().unwrap().is_empty());
}

#[test]
fn decoded_frame_surfaces_screen_origin() {
    // Encode a base, then stamp a non-zero screen origin into the header.
    let rgb: Vec<u8> = vec![1, 2, 3];
    let mut base = encode_tga_uncompressed_rgb24(1, 1, &rgb).unwrap();
    set_image_origin(&mut base, ImageOrigin::new(100, 200)).unwrap();
    let frame = decode_tga_frame(&base, &TgaDisplayOptions::default()).unwrap();
    assert_eq!(frame.screen_origin, ImageOrigin::new(100, 200));
    assert!(frame.has_screen_offset());
}

#[test]
fn decoded_frame_surfaces_postage_stamp_and_dev_tags() {
    // 2×2 RGBA base + a tiny postage stamp + a developer tag.
    let data: Vec<u8> = vec![9, 9, 9, 255, 8, 8, 8, 255, 7, 7, 7, 255, 6, 6, 6, 255];
    let base = encode_tga_uncompressed(2, 2, &data).unwrap();
    let stamp = parse_tga(&base).unwrap(); // reuse the decoded 2×2 as the thumbnail
    let ext = ExtensionAreaInput {
        attributes_type: 3,
        postage_stamp: Some(stamp),
        developer_tags: vec![DeveloperTagInput {
            tag_id: 100,
            payload: vec![0xDE, 0xAD, 0xBE, 0xEF],
        }],
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();

    let frame = decode_tga_frame(&file, &TgaDisplayOptions::default()).unwrap();
    assert!(frame.has_postage_stamp());
    let ps = frame.postage_stamp.as_ref().unwrap();
    assert_eq!((ps.width, ps.height), (2, 2));

    let dev = frame
        .developer_area
        .as_ref()
        .expect("developer area present");
    assert!(dev.contains(100));
    let payload = dev.payload_by_id(&file, 100).expect("tag 100 payload");
    assert_eq!(payload, &[0xDE, 0xAD, 0xBE, 0xEF]);
}

#[test]
fn decoded_frame_passthrough_options_match_raw() {
    // NONE options → bundle image equals parse_tga.
    let pixel = [10u8, 20, 30, 0x40];
    let base = encode_tga_uncompressed(1, 1, &pixel).unwrap();
    let ext = ExtensionAreaInput {
        attributes_type: 0,
        ..Default::default()
    };
    let file = encode_tga_with_extension(&base, &ext).unwrap();
    let raw = parse_tga(&file).unwrap();
    let frame = decode_tga_frame(&file, &TgaDisplayOptions::NONE).unwrap();
    assert_eq!(frame.image.data, raw.data);
}
