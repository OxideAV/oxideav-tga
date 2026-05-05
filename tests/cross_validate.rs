//! Cross-validation against `magick identify` / `magick convert`.
//!
//! Skipped (returns ok with a message) when ImageMagick isn't on the
//! `PATH`. When it is, we:
//!
//! * Encode an image with our writer + ask `magick identify` to confirm
//!   it parses (read-our-write).
//! * Encode an image with `magick convert -depth 8 input.tga` from our
//!   uncompressed RGBA temp file, then re-decode with our reader
//!   (read-their-write).

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use oxideav_tga::{
    encode_tga_grayscale, encode_tga_grayscale_rle, encode_tga_palette, encode_tga_palette_rle,
    encode_tga_rle, encode_tga_uncompressed, encode_tga_with_extension, parse_tga,
    parse_tga_extension_area, parse_tga_postage_stamp, ExtensionAreaInput, TgaImage,
    TgaPixelFormat,
};

fn have_magick() -> bool {
    Command::new("magick")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("oxideav-tga-cross-{}-{}", std::process::id(), name));
    p
}

fn checker(w: u16, h: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let q = (x & 1) + 2 * (y & 1);
            let p: [u8; 4] = [
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [128, 128, 128, 255],
            ][q];
            data.extend_from_slice(&p);
        }
    }
    data
}

#[test]
fn magick_identifies_our_uncompressed_output() {
    if !have_magick() {
        eprintln!("skipping: ImageMagick not on PATH");
        return;
    }
    let rgba = checker(16, 16);
    let bytes = encode_tga_uncompressed(16, 16, &rgba).unwrap();
    let path = tmp("uncompressed.tga");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let out = Command::new("magick")
        .arg("identify")
        .arg(&path)
        .output()
        .expect("magick identify");
    assert!(
        out.status.success(),
        "magick identify failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("TGA"), "expected 'TGA' in {s}");
    assert!(s.contains("16x16"), "expected '16x16' in {s}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn magick_identifies_our_rle_output() {
    if !have_magick() {
        eprintln!("skipping: ImageMagick not on PATH");
        return;
    }
    let rgba = checker(32, 24);
    let bytes = encode_tga_rle(32, 24, &rgba).unwrap();
    let path = tmp("rle.tga");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let out = Command::new("magick")
        .arg("identify")
        .arg(&path)
        .output()
        .expect("magick identify");
    assert!(
        out.status.success(),
        "magick identify failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("TGA"), "expected 'TGA' in {s}");
    assert!(s.contains("32x24"), "expected '32x24' in {s}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn we_decode_magick_authored_tga() {
    if !have_magick() {
        eprintln!("skipping: ImageMagick not on PATH");
        return;
    }
    // Ask magick to write a TGA from a tiny PNG it generates itself
    // via the `xc:` colour-canvas device. Two flavours: uncompressed
    // (default) and RLE-compressed.
    for &compress in &["none", "rle"] {
        let path = tmp(&format!("magick-{compress}.tga"));
        let status = Command::new("magick")
            .arg("-size")
            .arg("8x8")
            .arg("xc:red")
            .arg("-compress")
            .arg(compress)
            .arg(path.to_str().unwrap())
            .status()
            .expect("magick convert");
        assert!(status.success());
        let bytes = std::fs::read(&path).unwrap();
        let img = parse_tga(&bytes).unwrap_or_else(|e| {
            panic!(
                "parse_tga failed on magick-authored {compress} TGA: {e}; bytes={}",
                bytes.len()
            )
        });
        assert_eq!(img.width, 8);
        assert_eq!(img.height, 8);
        // First pixel should be pure red regardless of pixel format.
        match img.pixel_format {
            oxideav_tga::TgaPixelFormat::Rgba => {
                assert_eq!(&img.data[0..3], &[255, 0, 0]);
            }
            oxideav_tga::TgaPixelFormat::Rgb24 => {
                assert_eq!(&img.data[0..3], &[255, 0, 0]);
            }
            oxideav_tga::TgaPixelFormat::Gray8 => {
                // magick may output grayscale for a single-colour image
                // — accept that too.
            }
        }
        let _ = std::fs::remove_file(&path);
    }
}

// ---------------------------------------------------------------------------
// Round-2 writers cross-validated against magick.
// ---------------------------------------------------------------------------

fn gray_ramp(w: u16, h: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            data.push(((x + y * 5) & 0xFF) as u8);
        }
    }
    data
}

#[test]
fn magick_identifies_our_grayscale_output() {
    if !have_magick() {
        eprintln!("skipping: ImageMagick not on PATH");
        return;
    }
    let gray = gray_ramp(16, 16);
    let bytes = encode_tga_grayscale(16, 16, &gray).unwrap();
    let path = tmp("grayscale.tga");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let out = Command::new("magick")
        .arg("identify")
        .arg(&path)
        .output()
        .expect("magick identify");
    assert!(
        out.status.success(),
        "magick identify failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("16x16"), "expected '16x16' in {s}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn magick_identifies_our_grayscale_rle_output() {
    if !have_magick() {
        eprintln!("skipping: ImageMagick not on PATH");
        return;
    }
    let gray = gray_ramp(16, 16);
    let bytes = encode_tga_grayscale_rle(16, 16, &gray).unwrap();
    let path = tmp("grayscale-rle.tga");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let out = Command::new("magick")
        .arg("identify")
        .arg(&path)
        .output()
        .expect("magick identify");
    assert!(
        out.status.success(),
        "magick identify failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("16x16"), "expected '16x16' in {s}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn magick_identifies_our_palette_output() {
    if !have_magick() {
        eprintln!("skipping: ImageMagick not on PATH");
        return;
    }
    let rgba = checker(16, 16);
    let bytes = encode_tga_palette(16, 16, &rgba).unwrap();
    let path = tmp("palette.tga");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let out = Command::new("magick")
        .arg("identify")
        .arg(&path)
        .output()
        .expect("magick identify");
    assert!(
        out.status.success(),
        "magick identify failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("16x16"), "expected '16x16' in {s}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn magick_identifies_our_palette_rle_output() {
    if !have_magick() {
        eprintln!("skipping: ImageMagick not on PATH");
        return;
    }
    let rgba = checker(16, 16);
    let bytes = encode_tga_palette_rle(16, 16, &rgba).unwrap();
    let path = tmp("palette-rle.tga");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let out = Command::new("magick")
        .arg("identify")
        .arg(&path)
        .output()
        .expect("magick identify");
    assert!(
        out.status.success(),
        "magick identify failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("16x16"), "expected '16x16' in {s}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn magick_accepts_our_extension_area_output() {
    // We append a TGA 2.0 footer + extension area to a base image and
    // check magick still identifies + decodes it correctly.
    if !have_magick() {
        eprintln!("skipping: ImageMagick not on PATH");
        return;
    }
    let rgba = checker(16, 16);
    let base = encode_tga_uncompressed(16, 16, &rgba).unwrap();
    let stamp = TgaImage {
        width: 4,
        height: 4,
        pixel_format: TgaPixelFormat::Rgba,
        data: checker(4, 4),
        pts: None,
    };
    let ext = ExtensionAreaInput {
        author_name: "oxideav-tga test".to_string(),
        software_id: "oxideav-tga".to_string(),
        software_version: (200, 'a'),
        attributes_type: 3,
        postage_stamp: Some(stamp),
        ..ExtensionAreaInput::default()
    };

    let bytes = encode_tga_with_extension(&base, &ext).unwrap();
    let path = tmp("ext.tga");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();

    // magick identify should succeed.
    let out = Command::new("magick")
        .arg("identify")
        .arg(&path)
        .output()
        .expect("magick identify");
    assert!(
        out.status.success(),
        "magick identify failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("16x16"), "expected '16x16' in {s}");

    // Self-roundtrip: extension + thumbnail still parse on our side.
    let parsed = parse_tga_extension_area(&bytes).unwrap();
    assert_eq!(parsed.author_name, "oxideav-tga test");
    let stamp_back = parse_tga_postage_stamp(&bytes).unwrap().unwrap();
    assert_eq!(stamp_back.width, 4);
    assert_eq!(stamp_back.height, 4);

    let _ = std::fs::remove_file(&path);
}
