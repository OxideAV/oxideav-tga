//! Round 354 — colour-map **entry-size matrix** on the encode side.
//!
//! Before this round both palette writers (`encode_tga_palette` /
//! `encode_tga_palette_rle`) always emitted a 32-bit BGRA colour map
//! regardless of whether the palette used alpha — the encoder never
//! exercised the 15 / 16 / 24-bit colour-map entry sizes the decoder has
//! always read. The spec (§C.2 "Each color map entry is 2, 3, or 4
//! bytes") defines four canonical widths; this round makes the encoder
//! emit all of them:
//!
//! * `encode_tga_palette` / `encode_tga_palette_rle` now auto-select the
//!   narrowest *lossless* width: 24-bit BGR when the palette is fully
//!   opaque, 32-bit BGRA when any entry uses alpha
//!   (`ColorMapEntrySize::smallest_lossless_for`).
//! * `encode_tga_palette_with_entry_size` writes an explicit
//!   15 / 16 / 24 / 32-bit colour map (image type 1 or 9).
//! * `ColorMapEntrySize` is a typed view of fixed-header byte 7 with
//!   `from_u8` / `bits` / `bytes` / `has_alpha` predicates.
//!
//! Every test round-trips through the existing `parse_tga` decoder, so
//! the assertions pin the encoder's on-disk byte order against the
//! decoder's `decode_palette` expansion (15/16-bit A1R5G5B5, 24-bit BGR,
//! 32-bit BGRA).

use oxideav_tga::{
    encode_tga_palette, encode_tga_palette_rle, encode_tga_palette_with_entry_size, parse_header,
    parse_tga, ColorMapEntrySize, ImageType, TgaPixelFormat,
};

// ---------------------------------------------------------------------------
// ColorMapEntrySize typed view.
// ---------------------------------------------------------------------------

#[test]
fn entry_size_from_u8_only_accepts_spec_widths() {
    assert_eq!(
        ColorMapEntrySize::from_u8(15),
        Some(ColorMapEntrySize::Bits15)
    );
    assert_eq!(
        ColorMapEntrySize::from_u8(16),
        Some(ColorMapEntrySize::Bits16)
    );
    assert_eq!(
        ColorMapEntrySize::from_u8(24),
        Some(ColorMapEntrySize::Bits24)
    );
    assert_eq!(
        ColorMapEntrySize::from_u8(32),
        Some(ColorMapEntrySize::Bits32)
    );
    // Everything else is undefined per spec.
    for bad in [0u8, 1, 8, 12, 17, 23, 31, 33, 48, 64, 255] {
        assert_eq!(ColorMapEntrySize::from_u8(bad), None, "bits {bad}");
    }
}

#[test]
fn entry_size_bits_and_bytes() {
    assert_eq!(ColorMapEntrySize::Bits15.bits(), 15);
    assert_eq!(ColorMapEntrySize::Bits16.bits(), 16);
    assert_eq!(ColorMapEntrySize::Bits24.bits(), 24);
    assert_eq!(ColorMapEntrySize::Bits32.bits(), 32);
    // On-disk byte widths: 2 / 2 / 3 / 4 (spec: "2, 3, or 4 bytes").
    assert_eq!(ColorMapEntrySize::Bits15.bytes(), 2);
    assert_eq!(ColorMapEntrySize::Bits16.bytes(), 2);
    assert_eq!(ColorMapEntrySize::Bits24.bytes(), 3);
    assert_eq!(ColorMapEntrySize::Bits32.bytes(), 4);
    // bytes() == bits.div_ceil(8) for every legal width.
    for e in [
        ColorMapEntrySize::Bits15,
        ColorMapEntrySize::Bits16,
        ColorMapEntrySize::Bits24,
        ColorMapEntrySize::Bits32,
    ] {
        assert_eq!(e.bytes(), (e.bits() as usize).div_ceil(8));
    }
}

#[test]
fn entry_size_has_alpha() {
    assert!(!ColorMapEntrySize::Bits15.has_alpha());
    assert!(ColorMapEntrySize::Bits16.has_alpha());
    assert!(!ColorMapEntrySize::Bits24.has_alpha());
    assert!(ColorMapEntrySize::Bits32.has_alpha());
}

#[test]
fn smallest_lossless_for_picks_24_when_opaque_32_when_alpha() {
    let opaque = [[10, 20, 30, 0xFF], [40, 50, 60, 0xFF]];
    assert_eq!(
        ColorMapEntrySize::smallest_lossless_for(&opaque),
        ColorMapEntrySize::Bits24
    );
    let with_alpha = [[10, 20, 30, 0xFF], [40, 50, 60, 0x7F]];
    assert_eq!(
        ColorMapEntrySize::smallest_lossless_for(&with_alpha),
        ColorMapEntrySize::Bits32
    );
    // Empty palette → 24-bit (no alpha needed).
    assert_eq!(
        ColorMapEntrySize::smallest_lossless_for(&[]),
        ColorMapEntrySize::Bits24
    );
}

// ---------------------------------------------------------------------------
// Auto-selection on the default writers.
// ---------------------------------------------------------------------------

/// A 2×2 opaque palette image → the default writer emits a 24-bit colour
/// map (was 32-bit before this round), and the pixels survive the round
/// trip bit-exact.
#[test]
fn default_writer_emits_24bit_map_when_opaque() {
    // Four distinct opaque colours, one per pixel.
    let rgba = [
        10, 20, 30, 0xFF, //
        40, 50, 60, 0xFF, //
        70, 80, 90, 0xFF, //
        100, 110, 120, 0xFF,
    ];
    let bytes = encode_tga_palette(2, 2, &rgba).unwrap();
    let hdr = parse_header(&bytes).unwrap();
    assert_eq!(hdr.cmap_type, 1);
    assert_eq!(hdr.cmap_entry_size, 24, "opaque palette → 24-bit entries");
    assert_eq!(
        hdr.image_type_raw,
        ImageType::UncompressedColourMapped as u8
    );

    let img = parse_tga(&bytes).unwrap();
    assert_eq!(img.width, 2);
    assert_eq!(img.height, 2);
    assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
    // 24-bit entries are alpha-less → decoder forces alpha 0xFF.
    assert_eq!(&img.data[..], &rgba[..]);
}

/// A palette that uses alpha → the default writer keeps the 32-bit BGRA
/// map and round-trips the alpha channel exactly.
#[test]
fn default_writer_emits_32bit_map_when_alpha_present() {
    let rgba = [
        10, 20, 30, 0xFF, //
        40, 50, 60, 0x80, // translucent
        70, 80, 90, 0x00, // transparent
        100, 110, 120, 0xFF,
    ];
    let bytes = encode_tga_palette(2, 2, &rgba).unwrap();
    let hdr = parse_header(&bytes).unwrap();
    assert_eq!(hdr.cmap_entry_size, 32, "alpha palette → 32-bit entries");

    let img = parse_tga(&bytes).unwrap();
    assert_eq!(&img.data[..], &rgba[..]);
}

/// The RLE colour-mapped writer (type 9) auto-selects the same way.
#[test]
fn rle_writer_auto_selects_entry_size() {
    let opaque = [
        1, 2, 3, 0xFF, 1, 2, 3, 0xFF, 1, 2, 3, 0xFF, 4, 5, 6, 0xFF, //
    ];
    let bytes = encode_tga_palette_rle(4, 1, &opaque).unwrap();
    let hdr = parse_header(&bytes).unwrap();
    assert_eq!(hdr.image_type_raw, ImageType::RleColourMapped as u8);
    assert_eq!(hdr.cmap_entry_size, 24);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(&img.data[..], &opaque[..]);

    let alpha = [
        1, 2, 3, 0x10, 1, 2, 3, 0x10, 1, 2, 3, 0x10, 4, 5, 6, 0xFF, //
    ];
    let bytes = encode_tga_palette_rle(4, 1, &alpha).unwrap();
    let hdr = parse_header(&bytes).unwrap();
    assert_eq!(hdr.cmap_entry_size, 32);
    let img = parse_tga(&bytes).unwrap();
    assert_eq!(&img.data[..], &alpha[..]);
}

// ---------------------------------------------------------------------------
// Explicit entry-size API — 24 / 32-bit lossless round trips.
// ---------------------------------------------------------------------------

#[test]
fn explicit_24bit_lossless_roundtrip_both_types() {
    let rgba = [
        11, 22, 33, 0xFF, //
        44, 55, 66, 0xFF, //
        77, 88, 99, 0xFF, //
        123, 231, 132, 0xFF,
    ];
    for ty in [
        ImageType::UncompressedColourMapped,
        ImageType::RleColourMapped,
    ] {
        let bytes =
            encode_tga_palette_with_entry_size(2, 2, &rgba, ty, ColorMapEntrySize::Bits24).unwrap();
        let hdr = parse_header(&bytes).unwrap();
        assert_eq!(hdr.cmap_entry_size, 24);
        assert_eq!(hdr.image_type_raw, ty as u8);
        let img = parse_tga(&bytes).unwrap();
        assert_eq!(&img.data[..], &rgba[..], "type {:?}", ty);
    }
}

#[test]
fn explicit_32bit_lossless_roundtrip_both_types() {
    let rgba = [
        11, 22, 33, 0x01, //
        44, 55, 66, 0x7F, //
        77, 88, 99, 0xC0, //
        123, 231, 132, 0xFF,
    ];
    for ty in [
        ImageType::UncompressedColourMapped,
        ImageType::RleColourMapped,
    ] {
        let bytes =
            encode_tga_palette_with_entry_size(2, 2, &rgba, ty, ColorMapEntrySize::Bits32).unwrap();
        let hdr = parse_header(&bytes).unwrap();
        assert_eq!(hdr.cmap_entry_size, 32);
        let img = parse_tga(&bytes).unwrap();
        assert_eq!(&img.data[..], &rgba[..], "type {:?}", ty);
    }
}

// ---------------------------------------------------------------------------
// Explicit 15 / 16-bit entries — quantised (5-bit) but predictable.
// ---------------------------------------------------------------------------

/// Helper: expand an 8-bit channel through the encoder's 5-bit quantise
/// then the decoder's 5→8 expansion (top-bits replication). This is the
/// value a colour should reproduce after a 15/16-bit colour-map round
/// trip.
fn quantise_5bit(c: u8) -> u8 {
    let v5 = c >> 3;
    (v5 << 3) | (v5 >> 2)
}

#[test]
fn explicit_16bit_entries_quantise_to_5bits_and_keep_alpha_bit() {
    // Colours chosen so the low 3 bits get dropped by the 5-bit quantise.
    let rgba = [
        0xF8, 0x10, 0x08, 0xFF, // alpha bit set (>= 0x80)
        0x00, 0xFF, 0x80, 0x40, // alpha bit clear (< 0x80)
    ];
    let bytes = encode_tga_palette_with_entry_size(
        2,
        1,
        &rgba,
        ImageType::UncompressedColourMapped,
        ColorMapEntrySize::Bits16,
    )
    .unwrap();
    let hdr = parse_header(&bytes).unwrap();
    assert_eq!(hdr.cmap_entry_size, 16);

    let img = parse_tga(&bytes).unwrap();
    // Pixel 0: colour quantised, alpha bit set → opaque.
    assert_eq!(
        &img.data[0..4],
        &[
            quantise_5bit(0xF8),
            quantise_5bit(0x10),
            quantise_5bit(0x08),
            0xFF
        ]
    );
    // Pixel 1: colour quantised, alpha bit clear → transparent (0x00).
    assert_eq!(
        &img.data[4..8],
        &[
            quantise_5bit(0x00),
            quantise_5bit(0xFF),
            quantise_5bit(0x80),
            0x00
        ]
    );
}

#[test]
fn explicit_15bit_entries_force_opaque() {
    // 15-bit ignores the top bit → alpha always reads back as 0xFF.
    let rgba = [
        0xF8, 0x10, 0x08, 0x00, // input alpha is transparent...
        0x40, 0x80, 0xC0, 0x7F,
    ];
    let bytes = encode_tga_palette_with_entry_size(
        2,
        1,
        &rgba,
        ImageType::RleColourMapped,
        ColorMapEntrySize::Bits15,
    )
    .unwrap();
    let hdr = parse_header(&bytes).unwrap();
    assert_eq!(hdr.cmap_entry_size, 15);

    let img = parse_tga(&bytes).unwrap();
    // ...but 15-bit forces alpha 0xFF for every pixel.
    assert_eq!(
        &img.data[0..4],
        &[
            quantise_5bit(0xF8),
            quantise_5bit(0x10),
            quantise_5bit(0x08),
            0xFF
        ]
    );
    assert_eq!(
        &img.data[4..8],
        &[
            quantise_5bit(0x40),
            quantise_5bit(0x80),
            quantise_5bit(0xC0),
            0xFF
        ]
    );
}

// ---------------------------------------------------------------------------
// Pure-5-bit colours round-trip BIT-EXACT through 16-bit entries.
// ---------------------------------------------------------------------------

/// When every channel is already an exact 5-bit-expanded value (the
/// top-5-bits-replicated pattern), the 16-bit colour map is lossless.
#[test]
fn already_5bit_colours_roundtrip_exact_16bit() {
    // 0x00, 0x08(? no) — use values that equal quantise_5bit of themselves.
    let exact: Vec<u8> = (0u8..32).map(|v| (v << 3) | (v >> 2)).collect();
    // Build a 4×4 image from 16 of these as grayscale-ish opaque colours.
    let mut rgba = Vec::new();
    for i in 0..16 {
        let c = exact[i % exact.len()];
        rgba.extend_from_slice(&[c, c, c, 0xFF]);
    }
    let bytes = encode_tga_palette_with_entry_size(
        4,
        4,
        &rgba,
        ImageType::UncompressedColourMapped,
        ColorMapEntrySize::Bits16,
    )
    .unwrap();
    let img = parse_tga(&bytes).unwrap();
    // Each channel was already a valid 5-bit-expanded value → no loss.
    assert_eq!(&img.data[..], &rgba[..]);
}

// ---------------------------------------------------------------------------
// Error path — non-colour-mapped image types are rejected.
// ---------------------------------------------------------------------------

#[test]
fn explicit_api_rejects_non_colour_mapped_type() {
    let rgba = [1, 2, 3, 0xFF];
    for ty in [
        ImageType::UncompressedTrueColour,
        ImageType::RleTrueColour,
        ImageType::UncompressedGrayscale,
        ImageType::RleGrayscale,
        ImageType::None,
    ] {
        let r = encode_tga_palette_with_entry_size(1, 1, &rgba, ty, ColorMapEntrySize::Bits24);
        assert!(r.is_err(), "type {:?} must be rejected", ty);
    }
}

#[test]
fn explicit_api_still_rejects_over_256_colours() {
    // 257 unique colours → Unsupported regardless of entry size.
    let mut rgba = Vec::new();
    for i in 0..257u32 {
        rgba.extend_from_slice(&[(i & 0xFF) as u8, (i >> 8) as u8, 0, 0xFF]);
    }
    let r = encode_tga_palette_with_entry_size(
        257,
        1,
        &rgba,
        ImageType::UncompressedColourMapped,
        ColorMapEntrySize::Bits32,
    );
    assert!(r.is_err());
}

// ---------------------------------------------------------------------------
// On-disk colour-map byte layout — pins the exact packing per entry size.
// ---------------------------------------------------------------------------

#[test]
fn on_disk_colour_map_byte_layout() {
    // Single-colour image so the colour map has exactly one entry.
    let rgba = [0x12, 0x34, 0x56, 0x78];

    // 24-bit → B, G, R (3 bytes), no alpha.
    let bytes = encode_tga_palette_with_entry_size(
        1,
        1,
        &rgba,
        ImageType::UncompressedColourMapped,
        ColorMapEntrySize::Bits24,
    )
    .unwrap();
    let map = &bytes[18..18 + 3];
    assert_eq!(map, &[0x56, 0x34, 0x12]); // B, G, R

    // 32-bit → B, G, R, A (4 bytes).
    let bytes = encode_tga_palette_with_entry_size(
        1,
        1,
        &rgba,
        ImageType::UncompressedColourMapped,
        ColorMapEntrySize::Bits32,
    )
    .unwrap();
    let map = &bytes[18..18 + 4];
    assert_eq!(map, &[0x56, 0x34, 0x12, 0x78]); // B, G, R, A

    // 16-bit → little-endian u16 of ARRRRRGG GGGBBBBB.
    let bytes = encode_tga_palette_with_entry_size(
        1,
        1,
        &rgba,
        ImageType::UncompressedColourMapped,
        ColorMapEntrySize::Bits16,
    )
    .unwrap();
    let map = &bytes[18..18 + 2];
    let r5 = (0x12u16 >> 3) & 0x1F;
    let g5 = (0x34u16 >> 3) & 0x1F;
    let b5 = (0x56u16 >> 3) & 0x1F;
    // Alpha 0x78 = 120 < 0x80 → top (attribute) bit stays clear.
    let want = (r5 << 10) | (g5 << 5) | b5;
    assert_eq!(map, &want.to_le_bytes());

    // 16-bit with alpha >= 0x80 sets the top bit.
    let opaque_px = [0x12, 0x34, 0x56, 0xFF];
    let bytes = encode_tga_palette_with_entry_size(
        1,
        1,
        &opaque_px,
        ImageType::UncompressedColourMapped,
        ColorMapEntrySize::Bits16,
    )
    .unwrap();
    let map = &bytes[18..18 + 2];
    let want = (r5 << 10) | (g5 << 5) | b5 | 0x8000;
    assert_eq!(map, &want.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Full breadth matrix — every entry size × both colour-mapped image types.
// ---------------------------------------------------------------------------

/// The value an 8-bit channel reproduces after a given entry size's
/// round trip: full 8-bit precision for 24/32-bit, 5-bit quantised for
/// 15/16-bit.
fn channel_after(entry: ColorMapEntrySize, c: u8) -> u8 {
    match entry {
        ColorMapEntrySize::Bits24 | ColorMapEntrySize::Bits32 => c,
        ColorMapEntrySize::Bits15 | ColorMapEntrySize::Bits16 => quantise_5bit(c),
    }
}

/// The alpha an entry reproduces given its declared alpha and the entry
/// size's alpha handling.
fn alpha_after(entry: ColorMapEntrySize, a: u8) -> u8 {
    match entry {
        // Lossless alpha channel.
        ColorMapEntrySize::Bits32 => a,
        // 16-bit: one alpha bit, set when input alpha >= 0x80.
        ColorMapEntrySize::Bits16 => {
            if a >= 0x80 {
                0xFF
            } else {
                0x00
            }
        }
        // 15-bit / 24-bit: no alpha → always opaque.
        ColorMapEntrySize::Bits15 | ColorMapEntrySize::Bits24 => 0xFF,
    }
}

#[test]
fn entry_size_matrix_roundtrip() {
    // A small palette that uses both translucent and opaque entries plus
    // colours with non-trivial low bits (so 5-bit quantise is visible).
    let pixels: [[u8; 4]; 4] = [
        [0xF8, 0x10, 0x08, 0xFF], // opaque
        [0x00, 0xFF, 0x80, 0x40], // translucent (alpha < 0x80)
        [0x12, 0x34, 0x56, 0xC0], // translucent (alpha >= 0x80)
        [0xAB, 0xCD, 0xEF, 0x00], // transparent
    ];
    let mut rgba = Vec::new();
    for p in &pixels {
        rgba.extend_from_slice(p);
    }

    for entry in [
        ColorMapEntrySize::Bits15,
        ColorMapEntrySize::Bits16,
        ColorMapEntrySize::Bits24,
        ColorMapEntrySize::Bits32,
    ] {
        for ty in [
            ImageType::UncompressedColourMapped,
            ImageType::RleColourMapped,
        ] {
            let bytes = encode_tga_palette_with_entry_size(4, 1, &rgba, ty, entry).unwrap();
            let hdr = parse_header(&bytes).unwrap();
            assert_eq!(hdr.cmap_entry_size, entry.bits(), "{entry:?} {ty:?}");
            assert_eq!(hdr.image_type_raw, ty as u8);
            // On-disk colour-map block length = entries × entry bytes.
            let cmap_len = u16::from_le_bytes([bytes[5], bytes[6]]) as usize;
            assert_eq!(cmap_len, 4);

            let img = parse_tga(&bytes).unwrap();
            assert_eq!(img.width, 4);
            assert_eq!(img.height, 1);
            assert_eq!(img.pixel_format, TgaPixelFormat::Rgba);
            for (i, p) in pixels.iter().enumerate() {
                let got = &img.data[i * 4..i * 4 + 4];
                let want = [
                    channel_after(entry, p[0]),
                    channel_after(entry, p[1]),
                    channel_after(entry, p[2]),
                    alpha_after(entry, p[3]),
                ];
                assert_eq!(got, &want, "{entry:?} {ty:?} pixel {i}");
            }
        }
    }
}
