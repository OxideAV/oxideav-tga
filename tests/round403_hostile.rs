//! Round 403 — hostile-input hardening: the public decoder surface must
//! never panic and must fail *closed* with a typed error.
//!
//! The three fuzz harnesses under `fuzz/` already assert panic-freedom, but
//! they run only on nightly under `cargo fuzz`. These in-tree tests pin the
//! same contract in the CI `cargo test` matrix and additionally classify
//! the returned error: a malformed header returns `TgaError` (either
//! `InvalidData` or `Unsupported`), never a silently-wrong `Ok`.

use oxideav_tga::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

fn build_header(image_type: u8, depth: u8, cmap_type: u8, w: u16, h: u16, desc: u8) -> Vec<u8> {
    let mut v = vec![0u8; 18];
    v[1] = cmap_type;
    v[2] = image_type;
    if cmap_type != 0 {
        v[5] = 0;
        v[6] = 1; // length hi -> 256 entries
        v[7] = 24; // entry size
    }
    v[12] = (w & 0xFF) as u8;
    v[13] = (w >> 8) as u8;
    v[14] = (h & 0xFF) as u8;
    v[15] = (h >> 8) as u8;
    v[16] = depth;
    v[17] = desc;
    v
}

/// Exhaustive image_type × depth × cmap_type × descriptor sweep: every
/// public parser call must return without unwinding.
#[test]
fn hostile_header_sweep_never_panics() {
    let mut panicked = Vec::new();
    for image_type in [0u8, 1, 2, 3, 9, 10, 11, 32, 33, 128, 200, 255] {
        for depth in [0u8, 1, 4, 8, 15, 16, 24, 32, 40, 64, 255] {
            for cmap_type in [0u8, 1, 2, 255] {
                for desc in [0x00u8, 0x20, 0x10, 0x30, 0xC0, 0xFF] {
                    let mut buf = build_header(image_type, depth, cmap_type, 4, 4, desc);
                    if cmap_type != 0 {
                        buf.extend(std::iter::repeat(0x55).take(256 * 3));
                    }
                    buf.extend(std::iter::repeat(0x80).take(4 * 4 * 4 + 64));
                    let r = catch_unwind(AssertUnwindSafe(|| {
                        let _ = parse_tga(&buf);
                        let _ = parse_tga_color_map(&buf);
                        let _ = parse_tga_color_map_type(&buf);
                        let _ = parse_tga_footer(&buf);
                        let _ = parse_tga_extension_area(&buf);
                        let _ = parse_tga_attribute_bits(&buf);
                        let _ = parse_tga_attributes_type(&buf);
                        let _ = parse_tga_interleaving(&buf);
                        let _ = parse_tga_image_origin(&buf);
                        let _ = parse_tga_image_id(&buf);
                        let _ = parse_tga_border_color(&buf);
                        let _ = decode_tga_for_display(&buf, &TgaDisplayOptions::ALL);
                        let _ = decode_tga_for_display_reported(&buf, &TgaDisplayOptions::ALL);
                    }));
                    if r.is_err() {
                        panicked.push((image_type, depth, cmap_type, desc));
                    }
                }
            }
        }
    }
    assert!(panicked.is_empty(), "panics: {panicked:?}");
}

/// Truncating a valid file at every byte length must never panic — the
/// decoder handles a cut at any header / palette / pixel / footer boundary.
#[test]
fn hostile_truncation_never_panics() {
    let rgba: Vec<u8> = (0..(8 * 6 * 4)).map(|i| (i % 256) as u8).collect();
    // include an extension area + footer so the trailing-block parsers get
    // truncated mid-structure too.
    let base = encode_tga_rle(8, 6, &rgba).unwrap();
    let ext = ExtensionAreaInput {
        gamma: (45, 100),
        ..Default::default()
    };
    let full = encode_tga_with_extension(&base, &ext).unwrap();
    for len in 0..=full.len() {
        let slice = &full[..len];
        let r = catch_unwind(AssertUnwindSafe(|| {
            let _ = parse_tga(slice);
            let _ = parse_tga_footer(slice);
            let _ = parse_tga_extension_area(slice);
            let _ = parse_tga_color_map(slice);
            let _ = parse_tga_gamma(slice);
            let _ = decode_tga_for_display(slice, &TgaDisplayOptions::ALL);
        }));
        assert!(r.is_ok(), "panic at truncation len {len}");
    }
}

/// A representative set of clearly-malformed inputs must each return a typed
/// `TgaError` (fail closed), never a wrong `Ok`.
#[test]
fn malformed_inputs_return_typed_errors() {
    let cases: &[(&str, Vec<u8>)] = &[
        ("empty", vec![]),
        ("one byte", vec![2]),
        ("truncated header", vec![0u8; 10]),
        // unsupported image type 5
        ("image type 5", build_header(5, 24, 0, 2, 2, 0x20)),
        // true-colour with an impossible depth (7)
        ("depth 7", {
            let mut v = build_header(2, 7, 0, 2, 2, 0x20);
            v.extend(std::iter::repeat(0).take(64));
            v
        }),
        // grayscale declared 24 bpp (must be 8)
        ("gray depth 24", {
            let mut v = build_header(3, 24, 0, 2, 2, 0x20);
            v.extend(std::iter::repeat(0).take(64));
            v
        }),
        // header declares a 4×4 image but no pixel bytes follow
        ("no pixels", build_header(2, 24, 0, 4, 4, 0x20)),
    ];
    for (label, buf) in cases {
        let r = parse_tga(buf);
        assert!(
            matches!(
                r,
                Err(TgaError::InvalidData(_)) | Err(TgaError::Unsupported(_))
            ),
            "{label}: expected typed error, got {r:?}"
        );
    }
}

/// Data Type 0 (No Image Data) — a header + colour map with no pixel array
/// — is rejected by `parse_tga` (no image) yet its palette is still
/// surfaced by `parse_tga_color_map`.
#[test]
fn no_image_data_type0_palette_only() {
    let mut v = vec![0u8; 18];
    v[1] = 1; // colour map present
    v[2] = 0; // No Image Data
    v[5] = 2; // length = 2
    v[7] = 24; // 24-bpp entries
    v.extend_from_slice(&[0x00, 0x00, 0xFF]); // red
    v.extend_from_slice(&[0xFF, 0x00, 0x00]); // blue
    assert!(parse_tga(&v).is_err(), "type 0 has no image");
    let cmap = parse_tga_color_map(&v).unwrap().expect("palette present");
    assert_eq!(cmap.len(), 2);
    assert_eq!(cmap.get(0), Some([0xFF, 0x00, 0x00, 0xFF]));
    assert_eq!(cmap.get(1), Some([0x00, 0x00, 0xFF, 0xFF]));
}
