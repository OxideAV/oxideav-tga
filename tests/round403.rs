//! Round 403 — deterministic property-style bit-exact round-trip identity
//! across the full writer/type matrix.
//!
//! The crate's existing `roundtrip.rs` pins the lossless encode→decode
//! invariants on a handful of small fixed inputs. This round generalises
//! that into a fuzz-shaped-but-deterministic sweep: a seeded xorshift PRNG
//! paints every supported dimension shape (1×1, thin strips, wide rows, the
//! 255-index edge, a 64×64 tile) with random content, feeds each frame
//! through the lossless writers, and asserts the decoded raster is
//! **bit-exact** to what was written.
//!
//! Which writers are lossless (and therefore identity-testable) depends on
//! the pixel domain:
//!   * `encode_tga_uncompressed_rgb24` / `encode_tga_rle_rgb24` — every RGB
//!     input is preserved (decoder forces alpha to 0xFF).
//!   * `encode_tga_uncompressed` / `encode_tga_rle` — preserved *when* the
//!     RGBA input carries at least one non-opaque alpha byte (else the
//!     writer drops to 24 bpp, which is still lossless on the RGB channels
//!     but forces alpha to 0xFF). We force a non-opaque pixel so the 32-bpp
//!     path is taken and the whole RGBA quad round-trips.
//!   * `encode_tga_grayscale` / `encode_tga_grayscale_rle` — Gray8 is
//!     preserved exactly.
//!   * `encode_tga_palette` / `encode_tga_palette_rle` — the default
//!     writers pick the narrowest *lossless* colour-map entry size (24 bpp
//!     opaque, 32 bpp with alpha), so a ≤256-colour source round-trips
//!     exactly (colours re-index but decode back to the same RGBA).
//!   * `encode_tga_palette_with_entry_size(…, Bits24 / Bits32)` — explicit
//!     lossless entry sizes, both colour-mapped types (1 / 9).
//!
//! No content is asserted for lossy paths (15/16-bit colour-map entries):
//! those are covered by the idempotence test in this file instead.

use oxideav_tga::*;

/// Small deterministic xorshift64 PRNG — reproducible across hosts, no
/// external dependency, seeded per-test so a failure is bisectable.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }
    fn byte(&mut self) -> u8 {
        (self.next_u32() & 0xFF) as u8
    }
    fn range(&mut self, n: u32) -> u32 {
        self.next_u32() % n.max(1)
    }
}

/// Dimension shapes that stress the encoders' scanline / packet logic:
/// degenerate 1×1, single row / column, the 255 colour-map-index edge,
/// a run-length-boundary-crossing width, and a square tile.
fn dims() -> &'static [(u16, u16)] {
    &[
        (1, 1),
        (1, 7),
        (7, 1),
        (2, 2),
        (3, 5),
        (13, 4),
        (16, 16),
        (255, 1),
        (1, 255),
        (129, 3),
        (64, 64),
    ]
}

#[test]
fn identity_rgb24_matrix() {
    let mut rng = Rng::new(0xC0FF_EE01);
    for &(w, h) in dims() {
        let px = w as usize * h as usize;
        let rgb: Vec<u8> = (0..px * 3).map(|_| rng.byte()).collect();
        for (name, bytes) in [
            (
                "uncompressed_rgb24",
                encode_tga_uncompressed_rgb24(w, h, &rgb).unwrap(),
            ),
            ("rle_rgb24", encode_tga_rle_rgb24(w, h, &rgb).unwrap()),
        ] {
            let img = parse_tga(&bytes).unwrap();
            assert_eq!(img.pixel_format, TgaPixelFormat::Rgba, "{name} {w}x{h}");
            assert_eq!(img.width, w as u32);
            assert_eq!(img.height, h as u32);
            for i in 0..px {
                assert_eq!(
                    &img.data[i * 4..i * 4 + 3],
                    &rgb[i * 3..i * 3 + 3],
                    "{name} rgb px{i} {w}x{h}"
                );
                assert_eq!(img.data[i * 4 + 3], 0xFF, "{name} alpha px{i} {w}x{h}");
            }
        }
    }
}

#[test]
fn identity_rgba32_matrix() {
    let mut rng = Rng::new(0x1234_ABCD);
    for &(w, h) in dims() {
        let px = w as usize * h as usize;
        let mut rgba: Vec<u8> = (0..px * 4).map(|_| rng.byte()).collect();
        // Force a non-opaque alpha so the writer selects the 32-bpp path
        // and the full RGBA quad round-trips (not just the RGB channels).
        rgba[3] = 0x7F;
        for (name, bytes) in [
            (
                "uncompressed",
                encode_tga_uncompressed(w, h, &rgba).unwrap(),
            ),
            ("rle", encode_tga_rle(w, h, &rgba).unwrap()),
        ] {
            let img = parse_tga(&bytes).unwrap();
            assert_eq!(img.pixel_format, TgaPixelFormat::Rgba, "{name} {w}x{h}");
            assert_eq!(img.data, rgba, "{name} {w}x{h}");
        }
    }
}

#[test]
fn identity_gray_matrix() {
    let mut rng = Rng::new(0x0BAD_F00D);
    for &(w, h) in dims() {
        let px = w as usize * h as usize;
        let g: Vec<u8> = (0..px).map(|_| rng.byte()).collect();
        for (name, bytes) in [
            ("grayscale", encode_tga_grayscale(w, h, &g).unwrap()),
            ("grayscale_rle", encode_tga_grayscale_rle(w, h, &g).unwrap()),
        ] {
            let img = parse_tga(&bytes).unwrap();
            assert_eq!(img.pixel_format, TgaPixelFormat::Gray8, "{name} {w}x{h}");
            assert_eq!(img.data, g, "{name} {w}x{h}");
        }
    }
}

/// Paint `px` pixels from a random ≤256-entry palette; return `(rgba,
/// expected)` where both are identical (the palette is lossless here).
fn painted_palette(rng: &mut Rng, px: usize, with_alpha: bool) -> Vec<u8> {
    let ncol = 1 + rng.range(200) as usize;
    let mut pal: Vec<[u8; 4]> = Vec::with_capacity(ncol);
    for _ in 0..ncol {
        let a = if with_alpha && rng.range(2) == 0 {
            rng.byte()
        } else {
            0xFF
        };
        pal.push([rng.byte(), rng.byte(), rng.byte(), a]);
    }
    let mut rgba = Vec::with_capacity(px * 4);
    for _ in 0..px {
        rgba.extend_from_slice(&pal[rng.range(ncol as u32) as usize]);
    }
    rgba
}

#[test]
fn identity_palette_default_writers() {
    let mut rng = Rng::new(0xFACE_1357);
    for &(w, h) in dims() {
        let px = w as usize * h as usize;
        // opaque-only palette so the default writer picks a 24-bit map;
        // and an alpha-carrying palette so it picks a 32-bit map — both
        // lossless.
        for with_alpha in [false, true] {
            let rgba = painted_palette(&mut rng, px, with_alpha);
            for (name, bytes) in [
                ("palette", encode_tga_palette(w, h, &rgba).unwrap()),
                ("palette_rle", encode_tga_palette_rle(w, h, &rgba).unwrap()),
            ] {
                let img = parse_tga(&bytes).unwrap();
                assert_eq!(img.data, rgba, "{name} {w}x{h} alpha={with_alpha}");
            }
        }
    }
}

#[test]
fn identity_palette_explicit_lossless_entry_sizes() {
    let mut rng = Rng::new(0x5A5A_9696);
    for &(w, h) in dims() {
        let px = w as usize * h as usize;
        let rgba = painted_palette(&mut rng, px, true);
        // Bits24 (opaque-forced on decode) and Bits32 (alpha preserved),
        // for both colour-mapped image types (1 uncompressed, 9 RLE).
        for entry in [ColorMapEntrySize::Bits24, ColorMapEntrySize::Bits32] {
            for image_type in [
                ImageType::UncompressedColourMapped,
                ImageType::RleColourMapped,
            ] {
                let bytes =
                    encode_tga_palette_with_entry_size(w, h, &rgba, image_type, entry).unwrap();
                let img = parse_tga(&bytes).unwrap();
                if entry == ColorMapEntrySize::Bits32 {
                    assert_eq!(img.data, rgba, "type{image_type:?} Bits32 {w}x{h}");
                } else {
                    // Bits24 drops alpha -> RGB preserved, alpha 0xFF.
                    for i in 0..px {
                        assert_eq!(
                            &img.data[i * 4..i * 4 + 3],
                            &rgba[i * 4..i * 4 + 3],
                            "type{image_type:?} Bits24 rgb px{i} {w}x{h}"
                        );
                        assert_eq!(
                            img.data[i * 4 + 3],
                            0xFF,
                            "type{image_type:?} Bits24 alpha px{i} {w}x{h}"
                        );
                    }
                }
            }
        }
    }
}

/// The lossy 15/16-bit colour-map entry sizes quantise 8-bit channels to
/// 5 bits. A single encode is lossy, but the *decoded* image must be a
/// fixed point: re-encoding the already-quantised colours at the same
/// entry size and decoding again must reproduce the identical raster
/// (the 5-bit round is idempotent under `expand5`).
#[test]
fn palette_lossy_entry_sizes_are_idempotent() {
    let mut rng = Rng::new(0x7E57_1DEA);
    for &(w, h) in dims() {
        let px = w as usize * h as usize;
        let rgba = painted_palette(&mut rng, px, true);
        for entry in [ColorMapEntrySize::Bits15, ColorMapEntrySize::Bits16] {
            for image_type in [
                ImageType::UncompressedColourMapped,
                ImageType::RleColourMapped,
            ] {
                let b1 =
                    encode_tga_palette_with_entry_size(w, h, &rgba, image_type, entry).unwrap();
                let img1 = parse_tga(&b1).unwrap();
                // Re-encode the decoded (already-quantised) pixels.
                let b2 = encode_tga_palette_with_entry_size(w, h, &img1.data, image_type, entry)
                    .unwrap();
                let img2 = parse_tga(&b2).unwrap();
                assert_eq!(
                    img1.data, img2.data,
                    "type{image_type:?} {entry:?} not idempotent {w}x{h}"
                );
            }
        }
    }
}
