//! Round 221: TARGA-32 vs ARGB-32 alpha-channel fallback.
//!
//! Some legacy paint applications wrote 32-bpp TGA files without setting
//! the extension-area `attributes_type` byte to a meaningful value while
//! leaving the alpha channel at all-zero. A naïve decoder renders every
//! pixel as fully-transparent black, even though the original intent was
//! "opaque, alpha ignored". The convention documented in
//! `docs/image/tga/README.md` for these files is:
//!
//! > if alpha values are all 0, treat the file as opaque, not as
//! > fully-transparent black.
//!
//! This round exposes the convention as opt-in helpers — the default
//! decode path is unchanged:
//!
//! * [`TgaImage::all_alpha_zero`] — predicate: RGBA, non-empty, every
//!   alpha byte is `0`.
//! * [`TgaImage::force_opaque`] — write `A = 0xFF` to every pixel.
//! * [`oxideav_tga::resolve_alpha_with_targa32_fallback`] — composed
//!   resolver: prefer the file's declared `AttributesType` when present;
//!   fall back to the all-zero-alpha heuristic otherwise.
//!
//! This file pins:
//!
//! * `all_alpha_zero` semantics across pixel formats and edge cases.
//! * `force_opaque` semantics across pixel formats.
//! * `resolve_alpha_with_targa32_fallback` precedence:
//!   - File with extension area + declared `AttributesType` → apply that
//!     type, return `Some(attrs)`.
//!   - File with no extension area + all-zero alpha → force opaque,
//!     return `None`.
//!   - File with no extension area + some non-zero alpha → leave
//!     untouched, return `None`.
//!   - Non-RGBA image → no-op (still surfaces declared attributes when
//!     present).

use oxideav_tga::{
    encode_tga_grayscale, encode_tga_rle, encode_tga_uncompressed, encode_tga_with_extension,
    parse_tga, resolve_alpha_with_targa32_fallback, AttributesType, ExtensionAreaInput, TgaImage,
    TgaPixelFormat,
};

// ---------------------------------------------------------------------------
// TgaImage::all_alpha_zero
// ---------------------------------------------------------------------------

#[test]
fn all_alpha_zero_true_for_rgba_with_every_alpha_byte_zero() {
    // 2 × 1 RGBA, both alphas 0.
    let img = TgaImage {
        width: 2,
        height: 1,
        pixel_format: TgaPixelFormat::Rgba,
        data: vec![0xAA, 0xBB, 0xCC, 0x00, 0x11, 0x22, 0x33, 0x00],
        pts: None,
    };
    assert!(img.all_alpha_zero());
}

#[test]
fn all_alpha_zero_false_when_any_alpha_is_nonzero() {
    let img = TgaImage {
        width: 2,
        height: 1,
        pixel_format: TgaPixelFormat::Rgba,
        data: vec![0xAA, 0xBB, 0xCC, 0x00, 0x11, 0x22, 0x33, 0x01],
        pts: None,
    };
    assert!(!img.all_alpha_zero());
}

#[test]
fn all_alpha_zero_false_for_rgb24() {
    let img = TgaImage {
        width: 1,
        height: 1,
        pixel_format: TgaPixelFormat::Rgb24,
        data: vec![0xAA, 0xBB, 0xCC],
        pts: None,
    };
    assert!(!img.all_alpha_zero());
}

#[test]
fn all_alpha_zero_false_for_gray8() {
    let img = TgaImage {
        width: 1,
        height: 1,
        pixel_format: TgaPixelFormat::Gray8,
        data: vec![0x42],
        pts: None,
    };
    assert!(!img.all_alpha_zero());
}

#[test]
fn all_alpha_zero_false_for_empty_rgba() {
    let img = TgaImage {
        width: 0,
        height: 0,
        pixel_format: TgaPixelFormat::Rgba,
        data: vec![],
        pts: None,
    };
    assert!(!img.all_alpha_zero());
}

#[test]
fn all_alpha_zero_true_for_fully_opaque_then_zeroed_alpha() {
    // RGB unchanged from a real decode; alpha bytes wiped to 0.
    let img = TgaImage {
        width: 3,
        height: 1,
        pixel_format: TgaPixelFormat::Rgba,
        data: vec![
            0xFF, 0x00, 0x00, 0x00, // red, A=0
            0x00, 0xFF, 0x00, 0x00, // green, A=0
            0x00, 0x00, 0xFF, 0x00, // blue, A=0
        ],
        pts: None,
    };
    assert!(img.all_alpha_zero());
}

// ---------------------------------------------------------------------------
// TgaImage::force_opaque
// ---------------------------------------------------------------------------

#[test]
fn force_opaque_writes_ff_to_every_alpha_byte() {
    let mut img = TgaImage {
        width: 2,
        height: 1,
        pixel_format: TgaPixelFormat::Rgba,
        data: vec![0xAA, 0xBB, 0xCC, 0x00, 0x11, 0x22, 0x33, 0x80],
        pts: None,
    };
    img.force_opaque();
    assert_eq!(
        img.data,
        vec![0xAA, 0xBB, 0xCC, 0xFF, 0x11, 0x22, 0x33, 0xFF],
        "colour channels untouched, alpha → 0xFF"
    );
}

#[test]
fn force_opaque_is_noop_on_rgb24() {
    let mut img = TgaImage {
        width: 1,
        height: 1,
        pixel_format: TgaPixelFormat::Rgb24,
        data: vec![0xAA, 0xBB, 0xCC],
        pts: None,
    };
    let before = img.data.clone();
    img.force_opaque();
    assert_eq!(img.data, before);
}

#[test]
fn force_opaque_is_noop_on_gray8() {
    let mut img = TgaImage {
        width: 1,
        height: 1,
        pixel_format: TgaPixelFormat::Gray8,
        data: vec![0x42],
        pts: None,
    };
    let before = img.data.clone();
    img.force_opaque();
    assert_eq!(img.data, before);
}

#[test]
fn force_opaque_then_all_alpha_zero_is_false() {
    let mut img = TgaImage {
        width: 2,
        height: 1,
        pixel_format: TgaPixelFormat::Rgba,
        data: vec![0x10, 0x20, 0x30, 0x00, 0x40, 0x50, 0x60, 0x00],
        pts: None,
    };
    assert!(img.all_alpha_zero());
    img.force_opaque();
    assert!(!img.all_alpha_zero());
}

// ---------------------------------------------------------------------------
// resolve_alpha_with_targa32_fallback
// ---------------------------------------------------------------------------

fn build_all_zero_alpha_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            buf.push((x & 0xFF) as u8);
            buf.push((y & 0xFF) as u8);
            buf.push(((x + y) & 0xFF) as u8);
            buf.push(0x00); // alpha left zero on purpose
        }
    }
    buf
}

#[test]
fn resolver_no_extension_area_all_zero_alpha_forces_opaque() {
    // Encode a 4×4 RGBA file whose alpha is all-zero so the encoder picks
    // 32 bpp on the wire. Decode it through `parse_tga` (no fallback) →
    // every alpha is 0. Re-run with the resolver → every alpha is 0xFF
    // and the resolver reports `None` (no `AttributesType` was declared).
    let rgba = build_all_zero_alpha_rgba(4, 4);
    let file = encode_tga_uncompressed(4, 4, &rgba).expect("encode");

    let mut img = parse_tga(&file).expect("decode");
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    assert!(img.all_alpha_zero(), "decoded as all-zero alpha");

    let declared = resolve_alpha_with_targa32_fallback(&file, &mut img);
    assert!(
        declared.is_none(),
        "file has no extension area → no declared AttributesType"
    );
    assert!(img.data.chunks_exact(4).all(|p| p[3] == 0xFF));
    // Colour channels unchanged.
    for (idx, p) in img.data.chunks_exact(4).enumerate() {
        let x = (idx % 4) as u8;
        let y = (idx / 4) as u8;
        assert_eq!(p[0], x);
        assert_eq!(p[1], y);
        assert_eq!(p[2], x.wrapping_add(y));
    }
}

#[test]
fn resolver_no_extension_area_some_nonzero_alpha_leaves_image_unchanged() {
    // A file with at least one nonzero alpha byte must not be silently
    // forced opaque — only the all-zero case triggers the fallback.
    let mut rgba = build_all_zero_alpha_rgba(2, 2);
    rgba[3] = 0x40; // first pixel: A = 0x40
    let file = encode_tga_uncompressed(2, 2, &rgba).expect("encode");

    let mut img = parse_tga(&file).expect("decode");
    let snapshot = img.data.clone();
    let declared = resolve_alpha_with_targa32_fallback(&file, &mut img);
    assert!(declared.is_none(), "no extension area → None");
    assert_eq!(img.data, snapshot, "leave the image untouched");
}

#[test]
fn resolver_declared_attributes_type_takes_precedence_over_fallback() {
    // Build a TGA 2.0 file whose extension area declares `NoAlpha` (0).
    // The decoded buffer has all-zero alpha, but with a declared type
    // the resolver applies that — same observable result, but it must
    // return `Some(NoAlpha)` to disambiguate the path that ran.
    let rgba = build_all_zero_alpha_rgba(3, 3);
    let base = encode_tga_uncompressed(3, 3, &rgba).expect("encode");
    let file = encode_tga_with_extension(
        &base,
        &ExtensionAreaInput {
            attributes_type: AttributesType::NoAlpha.as_u8(),
            ..ExtensionAreaInput::default()
        },
    )
    .expect("wrap");

    let mut img = parse_tga(&file).expect("decode");
    assert!(img.all_alpha_zero());

    let declared = resolve_alpha_with_targa32_fallback(&file, &mut img);
    assert_eq!(declared, Some(AttributesType::NoAlpha));
    assert!(img.data.chunks_exact(4).all(|p| p[3] == 0xFF));
}

#[test]
fn resolver_declared_useful_alpha_passes_through() {
    // Declared `UsefulAlpha` (3): apply_to_image is a no-op for that
    // variant, so an all-zero-alpha buffer stays at A=0. Confirms the
    // declared-type path wins over the fallback — even when the
    // fallback *would* have changed the image.
    let rgba = build_all_zero_alpha_rgba(2, 2);
    let base = encode_tga_uncompressed(2, 2, &rgba).expect("encode");
    let file = encode_tga_with_extension(
        &base,
        &ExtensionAreaInput {
            attributes_type: AttributesType::UsefulAlpha.as_u8(),
            ..ExtensionAreaInput::default()
        },
    )
    .expect("wrap");

    let mut img = parse_tga(&file).expect("decode");
    assert!(img.all_alpha_zero());

    let declared = resolve_alpha_with_targa32_fallback(&file, &mut img);
    assert_eq!(declared, Some(AttributesType::UsefulAlpha));
    assert!(img.all_alpha_zero(), "UsefulAlpha pass-through");
}

#[test]
fn resolver_declared_premultiplied_unmultiplies_even_with_zero_alpha() {
    // PremultipliedAlpha with A=0 → spec says transparent black; this
    // also exercises the "declared type beats fallback" precedence.
    let rgba = build_all_zero_alpha_rgba(2, 2);
    let base = encode_tga_uncompressed(2, 2, &rgba).expect("encode");
    let file = encode_tga_with_extension(
        &base,
        &ExtensionAreaInput {
            attributes_type: AttributesType::PremultipliedAlpha.as_u8(),
            ..ExtensionAreaInput::default()
        },
    )
    .expect("wrap");

    let mut img = parse_tga(&file).expect("decode");
    let declared = resolve_alpha_with_targa32_fallback(&file, &mut img);
    assert_eq!(declared, Some(AttributesType::PremultipliedAlpha));
    assert!(
        img.data.chunks_exact(4).all(|p| p == [0, 0, 0, 0]),
        "A=0 pixels become transparent black per spec",
    );
}

#[test]
fn resolver_rgb24_image_is_left_untouched_no_extension() {
    // RGB24 carries no alpha — the resolver is a strict no-op on the
    // pixels and surfaces whatever attributes the file declares.
    // Standalone (no extension area) → returns `None`.
    let mut img = TgaImage {
        width: 1,
        height: 1,
        pixel_format: TgaPixelFormat::Rgb24,
        data: vec![0x10, 0x20, 0x30],
        pts: None,
    };
    let before = img.data.clone();
    // We can pass any input bytes — the function returns early on the
    // pixel-format check before touching the buffer.
    let declared = resolve_alpha_with_targa32_fallback(&[], &mut img);
    assert!(declared.is_none());
    assert_eq!(img.data, before);
}

#[test]
fn resolver_gray8_image_with_declared_extension_surfaces_attributes() {
    // Gray8 likewise has no alpha; resolver still returns the declared
    // type when the file has one, so callers can inspect it without
    // running a second `parse_tga_attributes_type` call.
    let gray = vec![0x10, 0x20, 0x30, 0x40];
    let base = encode_tga_grayscale(2, 2, &gray).expect("encode");
    let file = encode_tga_with_extension(
        &base,
        &ExtensionAreaInput {
            attributes_type: AttributesType::UndefinedRetain.as_u8(),
            ..ExtensionAreaInput::default()
        },
    )
    .expect("wrap");
    let mut img = parse_tga(&file).expect("decode");
    assert_eq!(img.pixel_format, TgaPixelFormat::Gray8);
    let declared = resolve_alpha_with_targa32_fallback(&file, &mut img);
    assert_eq!(declared, Some(AttributesType::UndefinedRetain));
    // Gray8 buffer untouched.
    assert_eq!(img.data, vec![0x10, 0x20, 0x30, 0x40]);
}

#[test]
fn resolver_rle_file_with_zero_alpha_force_opaque() {
    // Same fallback rule on an RLE (type 10) file — the resolver doesn't
    // care which writer produced the bytes; only the file's metadata
    // matters.
    let rgba = build_all_zero_alpha_rgba(4, 4);
    let file = encode_tga_rle(4, 4, &rgba).expect("encode");
    let mut img = parse_tga(&file).expect("decode");
    assert!(img.all_alpha_zero());
    let declared = resolve_alpha_with_targa32_fallback(&file, &mut img);
    assert!(declared.is_none());
    assert!(img.data.chunks_exact(4).all(|p| p[3] == 0xFF));
}
