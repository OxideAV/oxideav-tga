//! Round 280 — §C.6.9 scan-line tables: *computing* them from an
//! encoded stream and *using* them for random-access row decode.
//!
//! The scan-line table (extension-area Field 25) exists "to make
//! random access of compressed images easy" and "to allow 'giant
//! picture' access in smaller 'chunks'": one 4-byte from-file-start
//! offset per scan line, in the order the image was saved. The §C.5
//! packet rule — RLE packets "should never encode pixels from more
//! than one scan line" — is what makes the table well-defined for
//! compressed files. Prior rounds could parse / serialise / sanity-
//! check the table ([`TgaScanLineTable`], r261) but could neither
//! build one from a file's own pixel data nor jump to a single row
//! through one. This round adds both:
//!
//! * [`compute_tga_scan_line_table`] — derive the table from any
//!   supported file: arithmetic for uncompressed types, a single
//!   packet walk for RLE types. RLE files whose packets span a
//!   scan-line boundary (spec-discouraged, but [`parse_tga`] accepts
//!   them) are rejected with `Unsupported` — their rows don't start
//!   on packet boundaries, so no table can describe them.
//! * [`parse_tga_scan_line`] — decode exactly one row (saved-order
//!   index, per the spec's table ordering) through a table, returning
//!   a 1-pixel-tall [`TgaImage`] in the same normalised format
//!   `parse_tga` produces (columns mirrored per descriptor bit 4).
//!
//! The oracle throughout is `parse_tga`'s whole-image decode: every
//! random-access row must be byte-identical to the corresponding row
//! of the full raster, across all six writer-covered image types.

use oxideav_tga::{
    compute_tga_scan_line_table, encode_tga_grayscale, encode_tga_grayscale_rle,
    encode_tga_palette, encode_tga_palette_rle, encode_tga_rle, encode_tga_uncompressed,
    encode_tga_with_extension, parse_tga, parse_tga_scan_line, parse_tga_scan_line_table,
    ExtensionAreaInput, TgaError, TgaImage, TgaPixelFormat, TgaScanLineTable, TGA_HEADER_SIZE,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Every-pixel-unique opaque RGBA gradient (encoder auto-selects 24 bpp).
fn gradient_rgba(w: u16, h: u16) -> Vec<u8> {
    let mut v = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            v.extend_from_slice(&[
                (x * 7 + 3) as u8,
                (y * 11 + 5) as u8,
                (x as usize * y as usize % 251) as u8,
                0xFF,
            ]);
        }
    }
    v
}

/// Banded RGBA (long runs per row) with non-opaque alpha so the RLE
/// writer emits 32 bpp run packets.
fn banded_rgba(w: u16, h: u16) -> Vec<u8> {
    let mut v = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let band = (x / 5) as u8;
            v.extend_from_slice(&[band.wrapping_mul(40), (y as u8).wrapping_mul(30), 200, 0x80]);
        }
    }
    v
}

/// Assert every saved-order row fetched through `table` matches the
/// whole-image decode. The crate's writers are top-down, so saved
/// index == display row here.
fn assert_rows_match(input: &[u8], table: &TgaScanLineTable, full: &TgaImage) {
    let bpp = match full.pixel_format {
        TgaPixelFormat::Gray8 => 1usize,
        _ => 4,
    };
    let stride = full.width as usize * bpp;
    assert_eq!(table.len(), full.height as usize);
    for y in 0..full.height as usize {
        let row = parse_tga_scan_line(input, table, y).expect("row decode");
        assert_eq!(row.width, full.width);
        assert_eq!(row.height, 1);
        assert_eq!(row.pixel_format, full.pixel_format);
        assert_eq!(
            row.data,
            &full.data[y * stride..(y + 1) * stride],
            "row {y} mismatch"
        );
    }
}

// ---------------------------------------------------------------------------
// compute + random access across all six writer-covered image types
// ---------------------------------------------------------------------------

#[test]
fn uncompressed_truecolour_table_is_arithmetic_and_rows_match() {
    let (w, h) = (7u16, 5u16);
    let tga = encode_tga_uncompressed(w, h, &gradient_rgba(w, h)).unwrap();
    let table = compute_tga_scan_line_table(&tga).unwrap();
    assert_eq!(table.len(), h as usize);
    // No image ID / colour map: row 0 starts right after the header,
    // and rows are width × 3 bytes apart (opaque input → 24 bpp).
    assert_eq!(table.get(0), Some(TGA_HEADER_SIZE as u32));
    assert_eq!(
        table.get(1),
        Some((TGA_HEADER_SIZE + w as usize * 3) as u32)
    );
    assert!(table.is_strictly_increasing());
    assert!(table.is_well_formed_within(tga.len()));
    assert_rows_match(&tga, &table, &parse_tga(&tga).unwrap());
}

#[test]
fn rle_truecolour_run_and_raw_rows_match() {
    let (w, h) = (23u16, 9u16);
    // Banded → run packets dominate.
    let tga = encode_tga_rle(w, h, &banded_rgba(w, h)).unwrap();
    let table = compute_tga_scan_line_table(&tga).unwrap();
    assert!(table.is_strictly_increasing());
    assert_rows_match(&tga, &table, &parse_tga(&tga).unwrap());

    // Gradient → raw packets dominate.
    let tga = encode_tga_rle(w, h, &gradient_rgba(w, h)).unwrap();
    let table = compute_tga_scan_line_table(&tga).unwrap();
    assert_rows_match(&tga, &table, &parse_tga(&tga).unwrap());
}

#[test]
fn palette_uncompressed_and_rle_rows_match() {
    let (w, h) = (16u16, 6u16);
    // ≤ 256 unique colours so the palette writers accept it.
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let c = ((x + y * 3) % 13) as u8;
            rgba.extend_from_slice(&[c * 19, c * 7, 255 - c * 10, 0xFF]);
        }
    }
    for encode in [encode_tga_palette, encode_tga_palette_rle] {
        let tga = encode(w, h, &rgba).unwrap();
        let table = compute_tga_scan_line_table(&tga).unwrap();
        // Row 0 starts after header + 256-max colour map — i.e. past
        // the pixel-start the palette walk computed, never at 18.
        assert!(table.get(0).unwrap() > TGA_HEADER_SIZE as u32);
        assert_rows_match(&tga, &table, &parse_tga(&tga).unwrap());
    }
}

#[test]
fn grayscale_uncompressed_and_rle_rows_match() {
    let (w, h) = (21u16, 7u16);
    let gray: Vec<u8> = (0..w as usize * h as usize)
        .map(|i| ((i / 4) % 256) as u8)
        .collect();
    for encode in [encode_tga_grayscale, encode_tga_grayscale_rle] {
        let tga = encode(w, h, &gray).unwrap();
        let table = compute_tga_scan_line_table(&tga).unwrap();
        let full = parse_tga(&tga).unwrap();
        assert_eq!(full.pixel_format, TgaPixelFormat::Gray8);
        assert_rows_match(&tga, &table, &full);
    }
}

// ---------------------------------------------------------------------------
// Saved order (spec: "in the order that the image was saved") +
// column normalisation
// ---------------------------------------------------------------------------

/// Hand-built 2×2 uncompressed 24 bpp file with bottom-up row order
/// (descriptor bit 5 clear): table entry 0 must be the *first saved*
/// row, which displays at the bottom.
#[test]
fn bottom_up_file_table_is_in_saved_order() {
    let mut tga = vec![
        0, 0, 2, // no ID, no cmap, type 2
        0, 0, 0, 0, 0, // cmap spec
        0, 0, 0, 0, // x/y origin
        2, 0, 2, 0,    // 2×2
        24,   // depth
        0x00, // bottom-up, left-to-right
    ];
    // Saved first (bottom display row): red, green. Then blue, white.
    // 24 bpp is BGR on disk.
    tga.extend_from_slice(&[0, 0, 255, 0, 255, 0]); // red, green
    tga.extend_from_slice(&[255, 0, 0, 255, 255, 255]); // blue, white

    let full = parse_tga(&tga).unwrap(); // normalised top-down
    let table = compute_tga_scan_line_table(&tga).unwrap();
    assert_eq!(table.get(0), Some(TGA_HEADER_SIZE as u32));

    // Saved index 0 = display bottom row (red, green).
    let saved0 = parse_tga_scan_line(&tga, &table, 0).unwrap();
    assert_eq!(saved0.data, &full.data[8..16]);
    assert_eq!(
        saved0.data,
        [255, 0, 0, 255, 0, 255, 0, 255] // red, green in RGBA
    );
    // Saved index 1 = display top row (blue, white).
    let saved1 = parse_tga_scan_line(&tga, &table, 1).unwrap();
    assert_eq!(saved1.data, &full.data[0..8]);
}

/// Hand-built 3×1 uncompressed 24 bpp file with right-to-left columns
/// (descriptor bit 4 set): the scan-line decode must mirror columns
/// exactly as the whole-image decoder does.
#[test]
fn right_to_left_columns_are_mirrored() {
    let mut tga = vec![
        0, 0, 2, //
        0, 0, 0, 0, 0, //
        0, 0, 0, 0, //
        3, 0, 1, 0,    //
        24,   //
        0x30, // top-down + right-to-left
    ];
    // On-disk order (BGR): red, green, blue — stored right-to-left.
    tga.extend_from_slice(&[0, 0, 255, 0, 255, 0, 255, 0, 0]);

    let full = parse_tga(&tga).unwrap();
    let table = compute_tga_scan_line_table(&tga).unwrap();
    let row = parse_tga_scan_line(&tga, &table, 0).unwrap();
    assert_eq!(row.data, full.data);
    // Left-to-right after mirroring: blue, green, red.
    assert_eq!(row.data, [0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255]);
}

// ---------------------------------------------------------------------------
// The §C.5 boundary rule: a packet spanning two scan lines defeats the
// table. parse_tga still decodes such a file; compute refuses it.
// ---------------------------------------------------------------------------

#[test]
fn rle_packet_spanning_rows_is_rejected_for_table_but_decodes_whole() {
    let mut tga = vec![
        0, 0, 10, // type 10 (RLE true-colour)
        0, 0, 0, 0, 0, //
        0, 0, 0, 0, //
        2, 0, 2, 0,    // 2×2
        24,   //
        0x20, // top-down
    ];
    // One run packet covering all four pixels — crosses the row
    // boundary between rows 0 and 1.
    tga.extend_from_slice(&[0x83, 10, 20, 30]);

    let full = parse_tga(&tga).expect("whole-image decode accepts the file");
    assert_eq!(full.data.len(), 2 * 2 * 4);

    let err = compute_tga_scan_line_table(&tga).unwrap_err();
    assert!(
        matches!(err, TgaError::Unsupported(_)),
        "expected Unsupported, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Hostile / out-of-range inputs to the row decoder
// ---------------------------------------------------------------------------

#[test]
fn row_decoder_rejects_bad_indices_and_offsets() {
    let (w, h) = (4u16, 3u16);
    let tga = encode_tga_uncompressed(w, h, &gradient_rgba(w, h)).unwrap();
    let table = compute_tga_scan_line_table(&tga).unwrap();

    // Index past the table.
    let err = parse_tga_scan_line(&tga, &table, h as usize).unwrap_err();
    assert!(matches!(err, TgaError::InvalidData(_)));

    // Offset past the end of the file.
    let bogus = TgaScanLineTable::new(vec![u32::MAX]);
    let err = parse_tga_scan_line(&tga, &bogus, 0).unwrap_err();
    assert!(matches!(err, TgaError::InvalidData(_)));

    // Offset inside the file but with fewer than a full row of bytes
    // remaining → the §C.4 truncation guard fires.
    let late = TgaScanLineTable::new(vec![(tga.len() - 1) as u32]);
    let err = parse_tga_scan_line(&tga, &late, 0).unwrap_err();
    assert!(matches!(err, TgaError::InvalidData(_)));
}

// ---------------------------------------------------------------------------
// End-to-end: compute → embed via encode_tga_with_extension → re-parse
// from the file → random access through the *parsed* table.
// ---------------------------------------------------------------------------

#[test]
fn computed_table_embeds_and_round_trips_through_extension_area() {
    let (w, h) = (19u16, 11u16);
    let base = encode_tga_rle(w, h, &banded_rgba(w, h)).unwrap();
    // Compute against the base file; encode_tga_with_extension only
    // appends after the pixel data, so the offsets remain valid in
    // the extended file.
    let table = compute_tga_scan_line_table(&base).unwrap();
    let ext = ExtensionAreaInput {
        scan_line_table: Some(table.clone()),
        ..Default::default()
    };
    let full_file = encode_tga_with_extension(&base, &ext).unwrap();

    let parsed = parse_tga_scan_line_table(&full_file).expect("table present in file");
    assert_eq!(parsed, table);
    assert!(parsed.is_well_formed_within(full_file.len()));

    assert_rows_match(&full_file, &parsed, &parse_tga(&full_file).unwrap());
}
