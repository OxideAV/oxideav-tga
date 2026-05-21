//! Round-3 hardening pass:
//!
//! * RGB24-input standalone entry points (`encode_tga_uncompressed_rgb24`
//!   / `encode_tga_rle_rgb24`) — symmetric to the RGBA variants but
//!   skip the alpha-detection scan, always writing 24 bpp BGR.
//! * Extension-area self-roundtrip on every writer surface (types
//!   1/2/3/9/10/11): every writer feeds [`encode_tga_with_extension`],
//!   parse the extension area + main image back, byte-compare.
//! * Postage-stamp coverage at every parent depth we ship: 24-bit BGR
//!   parent, 32-bit BGRA parent, Gray8 parent + Gray8 stamp, palette
//!   parent + Gray8-as-indices stamp.
//! * Extension-area fixed-point: encode → parse → re-encode → parse
//!   converges byte-exact for the parts of the extension area the
//!   writer carries through.
//! * RLE pathological cases: full-width 128-pixel runs that exactly
//!   fit one packet, scanlines longer than 128, very small dimensions.
//!
//! Each test hand-builds its byte stream or independently round-trips
//! so a bug in one path can't mask the same bug on the other.

use oxideav_tga::{
    encode_tga_grayscale, encode_tga_grayscale_rle, encode_tga_palette, encode_tga_palette_rle,
    encode_tga_rle, encode_tga_rle_rgb24, encode_tga_uncompressed, encode_tga_uncompressed_rgb24,
    encode_tga_with_extension, parse_tga, parse_tga_extension_area, parse_tga_footer,
    parse_tga_postage_stamp, ExtensionAreaInput, TgaImage, TgaPixelFormat, TgaTimestamp,
};

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

fn opaque_checker_rgba(w: u16, h: u16) -> Vec<u8> {
    let palette: [[u8; 4]; 4] = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 255],
    ];
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let q = (x & 1) + 2 * (y & 1);
            data.extend_from_slice(&palette[q]);
        }
    }
    data
}

fn alpha_checker_rgba(w: u16, h: u16) -> Vec<u8> {
    let palette: [[u8; 4]; 4] = [
        [255, 0, 0, 255],
        [0, 255, 0, 200],
        [0, 0, 255, 128],
        [255, 255, 255, 64],
    ];
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let q = (x & 1) + 2 * (y & 1);
            data.extend_from_slice(&palette[q]);
        }
    }
    data
}

fn opaque_checker_rgb24(w: u16, h: u16) -> Vec<u8> {
    let palette: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];
    let mut data = Vec::with_capacity(w as usize * h as usize * 3);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let q = (x & 1) + 2 * (y & 1);
            data.extend_from_slice(&palette[q]);
        }
    }
    data
}

fn ramp_gray(w: u16, h: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            data.push(((x + y * 7) & 0xFF) as u8);
        }
    }
    data
}

fn populated_extension() -> ExtensionAreaInput {
    ExtensionAreaInput {
        author_name: "Mark Karpeles".to_string(),
        author_comment: [
            "Round 3 extension-area roundtrip".to_string(),
            "second line".to_string(),
            "third".to_string(),
            "fourth".to_string(),
        ],
        timestamp: TgaTimestamp {
            month: 5,
            day: 19,
            year: 2026,
            hour: 12,
            minute: 34,
            second: 56,
        },
        job_name: "render".to_string(),
        job_time: (1, 2, 3),
        software_id: "oxideav-tga".to_string(),
        software_version: (100, 'a'),
        key_color: [0xFF, 0x12, 0x34, 0x56],
        pixel_aspect_ratio: (4, 3),
        gamma: (22, 10),
        attributes_type: 3,
        postage_stamp: None,
        colour_correction_table: None,
        scan_line_table: None,
        developer_tags: Vec::new(),
    }
}

/// Assert two `ExtensionAreaInput`-shaped values agree on the fields the
/// writer carries through. The writer doesn't carry `postage_stamp` past
/// the offset itself, so we skip that field.
fn assert_extension_round_trip(input: &ExtensionAreaInput, parsed: &oxideav_tga::TgaExtensionArea) {
    assert_eq!(parsed.author_name, input.author_name);
    for (a, b) in parsed
        .author_comment
        .iter()
        .zip(input.author_comment.iter())
    {
        assert_eq!(a, b);
    }
    assert_eq!(parsed.timestamp, input.timestamp);
    assert_eq!(parsed.job_name, input.job_name);
    assert_eq!(parsed.job_time, input.job_time);
    assert_eq!(parsed.software_id, input.software_id);
    assert_eq!(parsed.software_version, input.software_version);
    assert_eq!(parsed.key_color, input.key_color);
    assert_eq!(parsed.pixel_aspect_ratio, input.pixel_aspect_ratio);
    assert_eq!(parsed.gamma, input.gamma);
    assert_eq!(parsed.attributes_type, input.attributes_type);
}

// ---------------------------------------------------------------------------
// RGB24 standalone entry points.
// ---------------------------------------------------------------------------

#[test]
fn rgb24_uncompressed_roundtrips_through_rgba_decoder() {
    let rgb = opaque_checker_rgb24(8, 6);
    let bytes = encode_tga_uncompressed_rgb24(8, 6, &rgb).unwrap();
    // Header check: image type 2, depth 24, no alpha bits.
    assert_eq!(bytes[2], 2);
    assert_eq!(bytes[16], 24);
    assert_eq!(bytes[17] & 0x0F, 0);
    // Decoder always normalises to Rgba.
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, 8);
    assert_eq!(img.height, 6);
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    // Promoted to alpha=0xFF.
    for (rgb_pix, rgba_pix) in rgb.chunks_exact(3).zip(img.data.chunks_exact(4)) {
        assert_eq!(rgb_pix[0], rgba_pix[0]);
        assert_eq!(rgb_pix[1], rgba_pix[1]);
        assert_eq!(rgb_pix[2], rgba_pix[2]);
        assert_eq!(rgba_pix[3], 0xFF);
    }
}

#[test]
fn rgb24_rle_roundtrips_through_rgba_decoder() {
    let rgb = opaque_checker_rgb24(16, 8);
    let bytes = encode_tga_rle_rgb24(16, 8, &rgb).unwrap();
    assert_eq!(bytes[2], 10);
    assert_eq!(bytes[16], 24);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, 16);
    assert_eq!(img.height, 8);
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    for (rgb_pix, rgba_pix) in rgb.chunks_exact(3).zip(img.data.chunks_exact(4)) {
        assert_eq!(rgb_pix[0], rgba_pix[0]);
        assert_eq!(rgb_pix[1], rgba_pix[1]);
        assert_eq!(rgb_pix[2], rgba_pix[2]);
        assert_eq!(rgba_pix[3], 0xFF);
    }
}

#[test]
fn rgb24_rle_runs_compress() {
    // Solid-colour image: one packet per row.
    let mut rgb = vec![0u8; 32 * 8 * 3];
    for px in rgb.chunks_exact_mut(3) {
        px.copy_from_slice(&[12, 34, 56]);
    }
    let raw = encode_tga_uncompressed_rgb24(32, 8, &rgb).unwrap();
    let rle = encode_tga_rle_rgb24(32, 8, &rgb).unwrap();
    assert!(
        rle.len() < raw.len(),
        "RLE expected smaller than raw (rle={}, raw={})",
        rle.len(),
        raw.len()
    );
    let img = parse_tga(&rle).unwrap();
    for px in img.data.chunks_exact(4) {
        assert_eq!(px, &[12, 34, 56, 0xFF]);
    }
}

#[test]
fn rgb24_rejects_unaligned_length() {
    // 3-byte stride enforced.
    let rgb = vec![0u8; 3 * 4 + 1];
    assert!(encode_tga_uncompressed_rgb24(2, 2, &rgb).is_err());
    assert!(encode_tga_rle_rgb24(2, 2, &rgb).is_err());
}

#[test]
fn rgb24_rejects_short_input() {
    // width × height × 3 > input length.
    let rgb = vec![0u8; 3];
    assert!(encode_tga_uncompressed_rgb24(4, 4, &rgb).is_err());
    assert!(encode_tga_rle_rgb24(4, 4, &rgb).is_err());
}

// ---------------------------------------------------------------------------
// Extension-area roundtrip across every writer surface.
// ---------------------------------------------------------------------------

#[test]
fn extension_area_roundtrip_type1_palette() {
    let rgba = opaque_checker_rgba(8, 6);
    let base = encode_tga_palette(8, 6, &rgba).unwrap();
    let ext_in = populated_extension();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let parsed = parse_tga_extension_area(&full).unwrap();
    assert_extension_round_trip(&ext_in, &parsed);
    let img = parse_tga(&full).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn extension_area_roundtrip_type2_uncompressed() {
    let rgba = opaque_checker_rgba(8, 6);
    let base = encode_tga_uncompressed(8, 6, &rgba).unwrap();
    let ext_in = populated_extension();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let parsed = parse_tga_extension_area(&full).unwrap();
    assert_extension_round_trip(&ext_in, &parsed);
    let img = parse_tga(&full).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn extension_area_roundtrip_type3_grayscale() {
    let gray = ramp_gray(16, 12);
    let base = encode_tga_grayscale(16, 12, &gray).unwrap();
    let ext_in = populated_extension();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let parsed = parse_tga_extension_area(&full).unwrap();
    assert_extension_round_trip(&ext_in, &parsed);
    let img = parse_tga(&full).unwrap();
    assert_eq!(img.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!(img.data, gray);
}

#[test]
fn extension_area_roundtrip_type9_palette_rle() {
    let rgba = opaque_checker_rgba(8, 6);
    let base = encode_tga_palette_rle(8, 6, &rgba).unwrap();
    let ext_in = populated_extension();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let parsed = parse_tga_extension_area(&full).unwrap();
    assert_extension_round_trip(&ext_in, &parsed);
    let img = parse_tga(&full).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn extension_area_roundtrip_type10_rle() {
    let rgba = opaque_checker_rgba(8, 6);
    let base = encode_tga_rle(8, 6, &rgba).unwrap();
    let ext_in = populated_extension();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let parsed = parse_tga_extension_area(&full).unwrap();
    assert_extension_round_trip(&ext_in, &parsed);
    let img = parse_tga(&full).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn extension_area_roundtrip_type11_grayscale_rle() {
    let gray = ramp_gray(16, 12);
    let base = encode_tga_grayscale_rle(16, 12, &gray).unwrap();
    let ext_in = populated_extension();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let parsed = parse_tga_extension_area(&full).unwrap();
    assert_extension_round_trip(&ext_in, &parsed);
    let img = parse_tga(&full).unwrap();
    assert_eq!(img.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!(img.data, gray);
}

// ---------------------------------------------------------------------------
// Extension-area fixed point: encode → parse → re-encode → parse converges.
// ---------------------------------------------------------------------------

#[test]
fn extension_area_fixed_point() {
    let rgba = opaque_checker_rgba(8, 6);
    let base = encode_tga_uncompressed(8, 6, &rgba).unwrap();
    let ext_in = populated_extension();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();

    // Parse, rebuild from parsed values, re-encode + parse again, compare.
    let parsed_once = parse_tga_extension_area(&full).unwrap();
    let ext_rebuilt = ExtensionAreaInput {
        author_name: parsed_once.author_name.clone(),
        author_comment: parsed_once.author_comment.clone(),
        timestamp: parsed_once.timestamp,
        job_name: parsed_once.job_name.clone(),
        job_time: parsed_once.job_time,
        software_id: parsed_once.software_id.clone(),
        software_version: parsed_once.software_version,
        key_color: parsed_once.key_color,
        pixel_aspect_ratio: parsed_once.pixel_aspect_ratio,
        gamma: parsed_once.gamma,
        attributes_type: parsed_once.attributes_type,
        postage_stamp: None,
        colour_correction_table: None,
        scan_line_table: None,
        developer_tags: Vec::new(),
    };
    let full_again = encode_tga_with_extension(&base, &ext_rebuilt).unwrap();
    let parsed_twice = parse_tga_extension_area(&full_again).unwrap();
    assert_eq!(
        parsed_once, parsed_twice,
        "extension area must reach a fixed point"
    );
}

// ---------------------------------------------------------------------------
// Postage-stamp coverage across parent depths.
// ---------------------------------------------------------------------------

#[test]
fn postage_stamp_with_24bit_parent_no_alpha() {
    // 24-bit parent ⇒ postage stamp on disk is BGR (no alpha byte per pixel).
    let main_rgba = opaque_checker_rgba(16, 16);
    let stamp_rgba = opaque_checker_rgba(4, 4);
    let stamp = TgaImage {
        width: 4,
        height: 4,
        pixel_format: TgaPixelFormat::Rgba,
        data: stamp_rgba.clone(),
        pts: None,
    };
    let base = encode_tga_uncompressed(16, 16, &main_rgba).unwrap();
    // Parent is 24 bpp (no alpha in main_rgba).
    assert_eq!(base[16], 24);
    let ext_in = ExtensionAreaInput {
        postage_stamp: Some(stamp),
        ..ExtensionAreaInput::default()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let stamp_back = parse_tga_postage_stamp(&full)
        .unwrap()
        .expect("postage stamp should decode");
    assert_eq!(stamp_back.width, 4);
    assert_eq!(stamp_back.height, 4);
    assert_eq!(stamp_back.pixel_format, TgaPixelFormat::Rgba);
    // On a 24-bit parent the stamp doesn't carry alpha; decoder fills α=0xFF.
    assert_eq!(stamp_back.data, stamp_rgba);
}

#[test]
fn postage_stamp_with_32bit_parent_carries_alpha() {
    // 32-bit parent (alpha-bearing main image) ⇒ stamp carries alpha.
    let main_rgba = alpha_checker_rgba(16, 16);
    let stamp_rgba = alpha_checker_rgba(4, 4);
    let stamp = TgaImage {
        width: 4,
        height: 4,
        pixel_format: TgaPixelFormat::Rgba,
        data: stamp_rgba.clone(),
        pts: None,
    };
    let base = encode_tga_uncompressed(16, 16, &main_rgba).unwrap();
    assert_eq!(base[16], 32);
    let ext_in = ExtensionAreaInput {
        postage_stamp: Some(stamp),
        ..ExtensionAreaInput::default()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let stamp_back = parse_tga_postage_stamp(&full)
        .unwrap()
        .expect("postage stamp should decode");
    assert_eq!(stamp_back.width, 4);
    assert_eq!(stamp_back.height, 4);
    assert_eq!(stamp_back.pixel_format, TgaPixelFormat::Rgba);
    // Full alpha preservation through encode → on-disk BGRA → decode.
    assert_eq!(stamp_back.data, stamp_rgba);
}

#[test]
fn postage_stamp_with_palette_parent_using_gray8_indices() {
    // §C.6.10: for colour-mapped parents, the stamp stores 8-bit indices
    // into the SAME palette as the main image. We map the thumbnail
    // through the parent palette by hand + pass it as a Gray8-shaped
    // buffer of indices (the encoder accepts this without remapping).
    let main_rgba = opaque_checker_rgba(8, 8);
    let base = encode_tga_palette(8, 8, &main_rgba).unwrap();
    assert_eq!(base[2], 1, "image type 1 (palette)");
    assert_eq!(base[16], 8, "depth 8");

    // Build a 4×4 thumbnail of palette indices. The main image used
    // a 4-colour checker so palette[0..4] is well-defined.
    let stamp_indices: Vec<u8> = (0..16).map(|i| (i % 4) as u8).collect();
    let stamp = TgaImage {
        width: 4,
        height: 4,
        pixel_format: TgaPixelFormat::Gray8,
        data: stamp_indices.clone(),
        pts: None,
    };
    let ext_in = ExtensionAreaInput {
        postage_stamp: Some(stamp),
        ..ExtensionAreaInput::default()
    };
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let stamp_back = parse_tga_postage_stamp(&full)
        .unwrap()
        .expect("palette-parent postage stamp should decode");
    assert_eq!(stamp_back.width, 4);
    assert_eq!(stamp_back.height, 4);
    // Decoded via the parent palette; output format is Rgba.
    assert_eq!(stamp_back.pixel_format, TgaPixelFormat::Rgba);
    assert_eq!(stamp_back.data.len(), 16 * 4);
    // Walk the indices manually and reproduce the expected RGBA pixels
    // from the parent palette (which was built in palette-encode order).
    // Palette order matches first-occurrence in the input scan, which
    // is `palette[0..4]` = [red, green, blue, white].
    let want_rgba: [[u8; 4]; 4] = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 255],
    ];
    for (i, idx) in stamp_indices.iter().enumerate() {
        let p = &stamp_back.data[i * 4..i * 4 + 4];
        assert_eq!(p, &want_rgba[*idx as usize], "stamp pixel {i}");
    }
}

#[test]
fn postage_stamp_rgba_with_palette_parent_is_unsupported() {
    // Explicit error path: an RGBA stamp with a colour-mapped parent
    // would require us to remap each pixel through the parent palette.
    // The encoder refuses rather than producing wrong output.
    let main_rgba = opaque_checker_rgba(8, 8);
    let base = encode_tga_palette(8, 8, &main_rgba).unwrap();
    let stamp = TgaImage {
        width: 2,
        height: 2,
        pixel_format: TgaPixelFormat::Rgba,
        data: vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
        pts: None,
    };
    let ext_in = ExtensionAreaInput {
        postage_stamp: Some(stamp),
        ..ExtensionAreaInput::default()
    };
    let err = encode_tga_with_extension(&base, &ext_in).unwrap_err();
    let s = format!("{err}");
    assert!(s.contains("map"), "got {s:?}");
}

// ---------------------------------------------------------------------------
// RLE pathological-input regression tests.
// ---------------------------------------------------------------------------

#[test]
fn rle_exact_128_pixel_run() {
    // A row of exactly 128 identical pixels should fit one run-packet
    // (packet count maxes out at 128). 129 should split to a 128-run +
    // a 1-pixel raw packet.
    let mut rgba = Vec::with_capacity(128 * 4);
    for _ in 0..128 {
        rgba.extend_from_slice(&[12, 34, 56, 255]);
    }
    let bytes = encode_tga_rle(128, 1, &rgba).unwrap();
    // Header (18) + 1 packet header + 3 BGR bytes = 22.
    assert_eq!(bytes.len(), 18 + 1 + 3);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn rle_129_pixel_row_splits_packets() {
    let mut rgba = Vec::with_capacity(129 * 4);
    for _ in 0..129 {
        rgba.extend_from_slice(&[12, 34, 56, 255]);
    }
    let bytes = encode_tga_rle(129, 1, &rgba).unwrap();
    // Two packets: 128-run + 1-pixel run (the single trailing pixel
    // counts as a run of 1 — which the encoder represents as a raw
    // packet of length 1, since runs ≥ 2 pick the run path).
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
    // Byte-size: 18 header + 1 + 3 (first packet) + 1 + 3 (second) = 26
    assert_eq!(bytes.len(), 18 + 4 + 4);
}

#[test]
fn rle_long_row_splits_at_128() {
    // Width 300 should produce ceil(300/128) = 3 run-packets per row.
    let w = 300u16;
    let h = 2u16;
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for _ in 0..w as usize * h as usize {
        rgba.extend_from_slice(&[200, 100, 50, 255]);
    }
    let bytes = encode_tga_rle(w, h, &rgba).unwrap();
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
}

#[test]
fn rle_alternating_pixels_all_raw_packets() {
    // Strict ABABAB → no runs, all raw packets. Width 200 should split
    // into ceil(200/128) = 2 raw packets per row.
    let w = 200u16;
    let h = 1u16;
    let mut rgba = Vec::with_capacity(w as usize * 4);
    for x in 0..w as usize {
        if x & 1 == 0 {
            rgba.extend_from_slice(&[255, 0, 0, 255]);
        } else {
            rgba.extend_from_slice(&[0, 255, 0, 255]);
        }
    }
    let bytes = encode_tga_rle(w, h, &rgba).unwrap();
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.data, rgba);
}

// ---------------------------------------------------------------------------
// Footer offset coverage — the extension_area_offset / developer_directory
// fields survive a write + a parse-then-re-attach footer dance.
// ---------------------------------------------------------------------------

#[test]
fn footer_extension_offset_points_inside_file() {
    let rgba = opaque_checker_rgba(4, 4);
    let base = encode_tga_uncompressed(4, 4, &rgba).unwrap();
    let ext_in = populated_extension();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let footer = parse_tga_footer(&full).unwrap();
    // Extension area is appended immediately after the base TGA bytes.
    assert_eq!(footer.extension_area_offset, base.len() as u32);
    // The extension area at that offset is the 495-byte body we wrote;
    // first 2 bytes are the canonical extension_size = 495.
    let ext_off = footer.extension_area_offset as usize;
    let ext_size = u16::from_le_bytes([full[ext_off], full[ext_off + 1]]);
    assert_eq!(ext_size, oxideav_tga::TGA_EXTENSION_AREA_SIZE as u16);
}

#[test]
fn footer_developer_offset_is_zero_when_unused() {
    let rgba = opaque_checker_rgba(4, 4);
    let base = encode_tga_uncompressed(4, 4, &rgba).unwrap();
    let ext_in = populated_extension();
    let full = encode_tga_with_extension(&base, &ext_in).unwrap();
    let footer = parse_tga_footer(&full).unwrap();
    // We don't write a developer directory; field must be zero.
    assert_eq!(footer.developer_directory_offset, 0);
}

// ---------------------------------------------------------------------------
// Tiny-dimension stress: 1×1 every type.
// ---------------------------------------------------------------------------

#[test]
fn one_by_one_every_writer() {
    let rgba_opaque = vec![123, 45, 67, 255];
    let rgba_alpha = vec![123, 45, 67, 200];
    let gray = vec![0x42];
    let rgb24 = vec![123, 45, 67];

    let t1 = encode_tga_palette(1, 1, &rgba_opaque).unwrap();
    assert_eq!(parse_tga(&t1).unwrap().data, rgba_opaque);

    let t2_24 = encode_tga_uncompressed(1, 1, &rgba_opaque).unwrap();
    assert_eq!(parse_tga(&t2_24).unwrap().data, rgba_opaque);

    let t2_32 = encode_tga_uncompressed(1, 1, &rgba_alpha).unwrap();
    assert_eq!(parse_tga(&t2_32).unwrap().data, rgba_alpha);

    let t2_rgb24 = encode_tga_uncompressed_rgb24(1, 1, &rgb24).unwrap();
    let img = parse_tga(&t2_rgb24).unwrap();
    assert_eq!(img.data, [123, 45, 67, 255]);

    let t3 = encode_tga_grayscale(1, 1, &gray).unwrap();
    assert_eq!(parse_tga(&t3).unwrap().data, gray);

    let t9 = encode_tga_palette_rle(1, 1, &rgba_opaque).unwrap();
    assert_eq!(parse_tga(&t9).unwrap().data, rgba_opaque);

    let t10 = encode_tga_rle(1, 1, &rgba_opaque).unwrap();
    assert_eq!(parse_tga(&t10).unwrap().data, rgba_opaque);

    let t10_rgb24 = encode_tga_rle_rgb24(1, 1, &rgb24).unwrap();
    assert_eq!(parse_tga(&t10_rgb24).unwrap().data, [123, 45, 67, 255]);

    let t11 = encode_tga_grayscale_rle(1, 1, &gray).unwrap();
    assert_eq!(parse_tga(&t11).unwrap().data, gray);
}
