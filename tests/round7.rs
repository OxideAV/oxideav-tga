//! Round 7: panic-free contract for the postage-stamp path
//! (`parse_tga_postage_stamp`).
//!
//! The library's documented contract for every public parser is
//! "returns `Result`, never panics" regardless of how malformed the
//! input is. The round-6 fuzz harness picked up a counter-example: a
//! footer + extension area authored with `postage_stamp_offset` set,
//! against a main header whose `image_type` (true-colour / grayscale)
//! is inconsistent with the `pixel_depth` field. The main-image path
//! (`parse_tga`) called `validate_depth` and returned
//! `Err(TgaError::Unsupported)`; the postage-stamp path skipped the
//! check, drove `decode_raw_pixels` with the bogus depth, and panicked
//! through `emit_pixel`'s `unreachable!()` arm.
//!
//! The fix mirrors `parse_tga`'s pre-decode `validate_depth` call in
//! `parse_tga_postage_stamp`, and replaces both of `emit_pixel`'s
//! `unreachable!()` arms with returned `Err`s (defence in depth — a
//! future caller that forgets to call `validate_depth` still fails
//! closed rather than aborting the process).
//!
//! This file pins both fixes with the corpus crash file + several
//! shaped inputs that exercise the previously-unreachable arms
//! through the public API only.

use oxideav_tga::{
    parse_tga, parse_tga_attributes_type, parse_tga_colour_correction_table,
    parse_tga_developer_area, parse_tga_extension_area, parse_tga_footer, parse_tga_postage_stamp,
    parse_tga_scan_line_table, TgaError,
};

// ---------------------------------------------------------------------------
// Crash-corpus replay
// ---------------------------------------------------------------------------

/// The corpus file `r175-postage-stamp-invalid-depth.bin` was the
/// fuzz crash that motivated this round: a true-colour parent header
/// (`image_type == 2`) whose pixel depth is `4` (unsupported — true-
/// colour only allows 15 / 16 / 24 / 32). The file also carries a
/// footer pointing at an extension area with `postage_stamp_offset`
/// set, so the postage-stamp decoder was driven with the bogus depth.
const FUZZ_CRASH_INPUT: &[u8] =
    include_bytes!("../fuzz/corpus/decode_tga/r175-postage-stamp-invalid-depth.bin");

#[test]
fn fuzz_crash_panics_were_not_panics_anymore() {
    // The contract is "every public parser returns; none panics".
    // This test calls every parser the fuzz harness drives on the
    // same crash bytes; failure mode is a panic, not a wrong return.
    let _ = parse_tga(FUZZ_CRASH_INPUT);
    let _ = parse_tga_footer(FUZZ_CRASH_INPUT);
    let _ = parse_tga_extension_area(FUZZ_CRASH_INPUT);
    let _ = parse_tga_postage_stamp(FUZZ_CRASH_INPUT);
    let _ = parse_tga_colour_correction_table(FUZZ_CRASH_INPUT);
    let _ = parse_tga_scan_line_table(FUZZ_CRASH_INPUT);
    let _ = parse_tga_developer_area(FUZZ_CRASH_INPUT);
    let _ = parse_tga_attributes_type(FUZZ_CRASH_INPUT);
}

#[test]
fn fuzz_crash_returns_err_from_main_decoder() {
    // The crash file's main header is degenerate (zero width). The
    // main decoder rejects it with `InvalidData` before reaching the
    // depth dispatch; the postage-stamp helper picks up the depth
    // dispatch separately, so the previously-panicking arm was only
    // reachable through `parse_tga_postage_stamp`, not `parse_tga`.
    // What matters here is that *both* surfaces return `Err` rather
    // than panicking.
    assert!(parse_tga(FUZZ_CRASH_INPUT).is_err());
}

#[test]
fn fuzz_crash_returns_err_from_postage_stamp() {
    // Same crash file through the postage-stamp helper: pre-fix this
    // panicked; post-fix it returns `Err(Unsupported)` because the
    // synthetic stamp header inherits the parent's bogus depth and
    // is now run through `validate_depth`.
    match parse_tga_postage_stamp(FUZZ_CRASH_INPUT) {
        Err(TgaError::Unsupported(_)) => {}
        Err(other) => panic!("expected Unsupported, got {other:?}"),
        Ok(_) => panic!("expected Err, got Ok"),
    }
}

// ---------------------------------------------------------------------------
// Shaped postage-stamp invalid-depth inputs
// ---------------------------------------------------------------------------
//
// The corpus crash file is opaque bytes; these tests exercise the
// same panic-class through the public API with minimal, hand-shaped
// inputs so the failure mode (returned `Unsupported`, no panic) is
// pinned for every (image_type, bad_depth) pair that previously hit
// `emit_pixel`'s `unreachable!()` arm.

/// Build a minimal TGA byte stream with the given `image_type` and
/// `pixel_depth`, a 1×1 frame, no image-ID / no colour-map, then
/// append a TGA 2.0 footer pointing at an extension area whose
/// `postage_stamp_offset` is just past the extension area itself.
/// The stamp header inside the file is a 1×1 stamp so the decoder
/// is forced through `decode_raw_pixels` with the bogus depth.
fn synth_with_postage(image_type: u8, depth: u8) -> Vec<u8> {
    let mut out = Vec::new();

    // 18-byte header. Width / height = 1, descriptor = 0x20 (top-down).
    out.push(0); // id_length
    out.push(0); // cmap_type
    out.push(image_type);
    out.extend_from_slice(&0u16.to_le_bytes()); // cmap_first
    out.extend_from_slice(&0u16.to_le_bytes()); // cmap_length
    out.push(0); // cmap_entry_size
    out.extend_from_slice(&0u16.to_le_bytes()); // x_origin
    out.extend_from_slice(&0u16.to_le_bytes()); // y_origin
    out.extend_from_slice(&1u16.to_le_bytes()); // width
    out.extend_from_slice(&1u16.to_le_bytes()); // height
    out.push(depth);
    out.push(0x20); // descriptor: top-down

    // One pixel of "data" — give the body 4 bytes so even the bogus
    // depth doesn't fall off the end (we want to reach the depth
    // dispatch, not the truncation check).
    out.extend_from_slice(&[0, 0, 0, 0]);

    // Extension area at offset = current cursor. Build a 495-byte
    // block, leave most fields zero, set postage_stamp_offset to
    // point at the postage-stamp dimensions we'll append after it.
    let ext_offset = out.len() as u32;
    let stamp_offset = ext_offset + 495;
    let mut ext = vec![0u8; 495];
    // extension_size = 495 at bytes 0..2
    ext[0..2].copy_from_slice(&495u16.to_le_bytes());
    // postage_stamp_offset = stamp_offset at bytes 486..490
    ext[486..490].copy_from_slice(&stamp_offset.to_le_bytes());
    out.extend_from_slice(&ext);

    // Postage stamp header: width=1, height=1.
    out.push(1);
    out.push(1);
    // Stamp body: 4 bytes (room for up to a 32-bit pixel; the bogus
    // depth path may want fewer, that's fine).
    out.extend_from_slice(&[0, 0, 0, 0]);

    // Footer: ext_offset, dev_offset = 0, signature.
    out.extend_from_slice(&ext_offset.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(b"TRUEVISION-XFILE.\0");

    out
}

#[test]
fn postage_stamp_truecolour_depth_4_returns_err_not_panic() {
    // True-colour (image_type 2) accepts 15 / 16 / 24 / 32 only.
    // Pre-fix this panicked through `emit_pixel`'s `unreachable!()`;
    // post-fix it returns `Unsupported` via `validate_depth`.
    let bytes = synth_with_postage(2, 4);
    match parse_tga_postage_stamp(&bytes) {
        Err(TgaError::Unsupported(_)) => {}
        Err(other) => panic!("expected Unsupported, got {other:?}"),
        Ok(_) => panic!("expected Err, got Ok"),
    }
}

#[test]
fn postage_stamp_truecolour_depth_0_returns_err_not_panic() {
    // Depth 0 is the most-pathological value: `in_bpp = ceil(0/8) = 0`,
    // and the depth dispatch in `emit_pixel` previously fell into
    // `_ => unreachable!()`.
    let bytes = synth_with_postage(2, 0);
    match parse_tga_postage_stamp(&bytes) {
        Err(TgaError::Unsupported(_)) => {}
        Err(other) => panic!("expected Unsupported, got {other:?}"),
        Ok(_) => panic!("expected Err, got Ok"),
    }
}

#[test]
fn postage_stamp_grayscale_depth_4_returns_err_not_panic() {
    // Grayscale (image_type 3) accepts depth 8 only.
    let bytes = synth_with_postage(3, 4);
    match parse_tga_postage_stamp(&bytes) {
        Err(TgaError::Unsupported(_)) => {}
        Err(other) => panic!("expected Unsupported, got {other:?}"),
        Ok(_) => panic!("expected Err, got Ok"),
    }
}

#[test]
fn postage_stamp_rle_truecolour_depth_4_returns_err_not_panic() {
    // RLE true-colour (image_type 10): the synthetic stamp header
    // remaps RLE → uncompressed for the stamp itself, but the depth
    // it inherits from the parent is still 4 (invalid). The same
    // `validate_depth` guard fires.
    let bytes = synth_with_postage(10, 4);
    match parse_tga_postage_stamp(&bytes) {
        Err(TgaError::Unsupported(_)) => {}
        Err(other) => panic!("expected Unsupported, got {other:?}"),
        Ok(_) => panic!("expected Err, got Ok"),
    }
}

#[test]
fn postage_stamp_valid_depth_24_still_succeeds() {
    // The fix doesn't regress the success path: depth 24 is valid
    // for image_type 2 and the postage stamp decodes cleanly.
    let bytes = synth_with_postage(2, 24);
    let stamp = parse_tga_postage_stamp(&bytes)
        .expect("postage stamp parse returns Ok for valid depth")
        .expect("Some(stamp) when offset != 0");
    assert_eq!(stamp.width, 1);
    assert_eq!(stamp.height, 1);
}

#[test]
fn postage_stamp_valid_depth_8_grayscale_still_succeeds() {
    let bytes = synth_with_postage(3, 8);
    let stamp = parse_tga_postage_stamp(&bytes)
        .expect("postage stamp parse returns Ok for valid depth")
        .expect("Some(stamp) when offset != 0");
    assert_eq!(stamp.width, 1);
    assert_eq!(stamp.height, 1);
}
