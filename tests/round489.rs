//! Round 489 regressions.
//!
//! * A footer-less file whose pixel data ends in the bytes of the
//!   TGA 2.0 footer signature must not be read back as a TGA 2.0 file
//!   whose offsets are pixel bytes (scheduled fuzz finding, target
//!   `contract_identity`, 2026-11).
//! * The pixel decoder writes every row straight to its place: all
//!   four origins, packets crossing scan lines, every wire layout.
//! * Nothing is allocated for a raster the data cannot fill.
//! * The encoder writes all four origins and a non-zero Color Map
//!   Origin.

use oxideav_tga::{
    ColumnOrder, DecodeOptions, EncodeOptions, Metadata, Palette, PixelFormat, RowOrder, TgaError,
    TgaImage, TrueColourDepth, decode, decode_with, encode, header, info, parse_tga_footer,
};

const SIGNATURE: &[u8; 18] = b"TRUEVISION-XFILE.\0";

/// The fuzz input that tripped the identity target, minimised: a
/// 132 x 271 `Gray8` plane tiled from 22 bytes that end in the footer
/// signature, run-length encoded, 93-byte Image ID, no footer asked
/// for. The last raw packet of the file ended in the signature and the
/// eight bytes before it named an "extension area" inside the pixel
/// data that parsed with a gamma of 0.985.
#[test]
fn rle_gray_plane_tiled_with_the_signature_keeps_its_identity() {
    let tile: Vec<u8> = [0x00, 0x46, 0x0A, 0x0A]
        .iter()
        .chain(SIGNATURE.iter())
        .copied()
        .collect();
    let (w, h) = (132u32, 271u32);
    let plane: Vec<u8> = tile
        .iter()
        .copied()
        .cycle()
        .take((w * h) as usize)
        .collect();
    let mut img = TgaImage::packed(w, h, PixelFormat::Gray8, plane).unwrap();
    img.metadata = Metadata::new();
    let opts = EncodeOptions::default()
        .with_rle(true)
        .with_image_id(vec![0u8; 93]);
    let bytes = encode(&img, &opts).unwrap();

    let back = decode(&bytes).unwrap();
    assert_eq!(back.data(), img.data());
    assert_eq!(back.metadata.gamma, None, "pixel bytes read as a gamma");
    assert!(back.extension.is_none(), "pixel bytes read as an extension");
    // The writer closed the ambiguity with a real, empty footer.
    let footer = parse_tga_footer(&bytes).expect("a real footer was appended");
    assert_eq!(footer.extension_area_offset, 0);
    assert_eq!(footer.developer_directory_offset, 0);
    assert!(info(&bytes).unwrap().has_footer);
}

/// The same hazard on every uncompressed layout: the plane's last 26
/// bytes are `offsets ++ signature` with offsets pointing into the
/// file.
#[test]
fn raw_planes_ending_in_the_signature_gain_an_empty_footer() {
    for format in [PixelFormat::Gray8, PixelFormat::Rgb24, PixelFormat::Rgba] {
        let bpp = format.bytes_per_pixel();
        let (w, h) = (24u32, 40u32);
        let mut plane = vec![0x5Au8; w as usize * h as usize * bpp];
        // Storage order is BGR(A); pick a tail that is the signature
        // *after* the channel swap by encoding, reading the tail, and
        // fixing the source up through the same permutation.
        let n = plane.len();
        let mut tail = Vec::new();
        tail.extend_from_slice(&20u32.to_le_bytes());
        tail.extend_from_slice(&20u32.to_le_bytes());
        tail.extend_from_slice(SIGNATURE);
        // Pad to a whole number of pixels at the front.
        while tail.len() % bpp != 0 {
            tail.insert(0, 0);
        }
        for (dst, src) in plane[n - tail.len()..]
            .chunks_exact_mut(bpp)
            .zip(tail.chunks_exact(bpp))
        {
            match bpp {
                1 => dst[0] = src[0],
                3 => dst.copy_from_slice(&[src[2], src[1], src[0]]),
                _ => dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]),
            }
        }
        let img = TgaImage::packed(w, h, format, plane).unwrap();
        let opts = EncodeOptions::default()
            .with_rle(false)
            .with_row_order(oxideav_tga::RowOrder::TopDown);
        let bytes = encode(&img, &opts).unwrap();
        assert!(bytes.ends_with(SIGNATURE));
        let footer = parse_tga_footer(&bytes).unwrap();
        assert_eq!(
            (
                footer.extension_area_offset,
                footer.developer_directory_offset
            ),
            (0, 0),
            "{format:?}: the footer a reader sees is the writer's, not pixel bytes"
        );
        let back = decode(&bytes).unwrap();
        assert_eq!(back.format, format);
        assert_eq!(back.data(), img.data(), "{format:?}");
        assert!(back.extension.is_none());
    }
}

/// A plane that does not end in the signature still yields a TGA 1.0
/// file: no footer is added.
#[test]
fn ordinary_planes_stay_footerless() {
    let img = TgaImage::packed(8, 8, PixelFormat::Gray8, vec![7u8; 64]).unwrap();
    let bytes = encode(&img, &EncodeOptions::default().with_rle(false)).unwrap();
    assert_eq!(bytes.len(), 18 + 64);
    assert!(parse_tga_footer(&bytes).is_none());
}

// ---------------------------------------------------------------------------
// Decoder: origins, packets, declared sizes
// ---------------------------------------------------------------------------

/// A deterministic byte stream for test planes.
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 8) as u8
        })
        .collect()
}

/// A hand-built file: 18-byte header (no Image ID, no colour map) and
/// the given pixel bytes.
fn file(image_type: u8, w: u16, h: u16, depth: u8, descriptor: u8, pixels: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; 18];
    f[2] = image_type;
    f[12..14].copy_from_slice(&w.to_le_bytes());
    f[14..16].copy_from_slice(&h.to_le_bytes());
    f[16] = depth;
    f[17] = descriptor;
    f.extend_from_slice(pixels);
    f
}

/// The four origins of Table 2, on a hand-built uncompressed file for
/// each wire depth: the decoded picture is the stored one with rows
/// flipped unless bit 5 is set and columns mirrored when bit 4 is.
#[test]
fn all_four_origins_decode_to_the_same_orientation_rule() {
    let (w, h) = (7usize, 5usize);
    for (image_type, depth, wire, native) in [
        (3u8, 8u8, 1usize, 1usize),
        (2, 15, 2, 4),
        (2, 16, 2, 4),
        (2, 24, 3, 3),
        (2, 32, 4, 4),
    ] {
        let stored = noise(w * h * wire, 0xA5A5 + u32::from(depth));
        // The top-left file is the reference: stored order is display
        // order.
        let reference = decode(&file(image_type, w as u16, h as u16, depth, 0x20, &stored))
            .unwrap()
            .data()
            .to_vec();
        assert_eq!(reference.len(), w * h * native);
        for (bits, flip, mirror) in [
            (0x00u8, true, false),
            (0x10, true, true),
            (0x20, false, false),
            (0x30, false, true),
        ] {
            let got = decode(&file(image_type, w as u16, h as u16, depth, bits, &stored)).unwrap();
            let mut want = Vec::with_capacity(reference.len());
            for y in 0..h {
                let sy = if flip { h - 1 - y } else { y };
                let row = &reference[sy * w * native..(sy + 1) * w * native];
                if mirror {
                    for px in row.chunks_exact(native).rev() {
                        want.extend_from_slice(px);
                    }
                } else {
                    want.extend_from_slice(row);
                }
            }
            assert_eq!(
                got.data(),
                &want[..],
                "depth {depth}, descriptor {bits:#04x}"
            );
        }
    }
}

/// Run-length packets that run past the end of a scan line decode (the
/// default mode accepts them) to exactly what the uncompressed file
/// holds, in every origin; strict mode refuses them.
#[test]
fn packets_crossing_scan_lines_land_in_the_right_rows() {
    let (w, h) = (5u16, 4u16);
    for (image_type, depth, wire) in [(3u8, 8u8, 1usize), (2, 16, 2), (2, 24, 3), (2, 32, 4)] {
        let px = |v: u8| -> Vec<u8> { (0..wire).map(|k| v.wrapping_mul(17) ^ k as u8).collect() };
        // 20 pixels: a run of 7 (crosses row 0 → 1), a raw packet of 9
        // (crosses rows 1 → 2 → 3), a run of 4 to the end.
        let mut rle = vec![0x80 | 6];
        rle.extend(px(1));
        rle.push(8);
        for v in 2..11 {
            rle.extend(px(v));
        }
        rle.push(0x80 | 3);
        rle.extend(px(40));
        let mut raw = Vec::new();
        for _ in 0..7 {
            raw.extend(px(1));
        }
        for v in 2..11 {
            raw.extend(px(v));
        }
        for _ in 0..4 {
            raw.extend(px(40));
        }
        for bits in [0x00u8, 0x10, 0x20, 0x30] {
            let from_rle = decode(&file(image_type + 8, w, h, depth, bits, &rle)).unwrap();
            let from_raw = decode(&file(image_type, w, h, depth, bits, &raw)).unwrap();
            assert_eq!(
                from_rle.data(),
                from_raw.data(),
                "depth {depth}, descriptor {bits:#04x}"
            );
        }
        let strict = DecodeOptions::default().with_strict(true);
        assert!(matches!(
            decode_with(&file(image_type + 8, w, h, depth, 0x20, &rle), &strict),
            Err(TgaError::InvalidData(_))
        ));
    }
}

/// A packet that would produce more pixels than the image holds, a
/// packet cut short, and a stream that ends early are all
/// `InvalidData`.
#[test]
fn malformed_packet_streams_are_invalid_data() {
    // 4 x 2 grey: one run of 9 pixels overruns the 8-pixel raster.
    let overrun = file(11, 4, 2, 8, 0x20, &[0x80 | 8, 7]);
    // A raw packet announcing 8 pixels with 3 present.
    let short_raw = file(11, 4, 2, 8, 0x20, &[7, 1, 2, 3]);
    // A run packet header with no pixel after it (24-bit: 3 needed).
    let short_run = file(10, 4, 2, 24, 0x20, &[0x80 | 7, 1, 2]);
    // Half the rows, then nothing.
    let early_end = file(11, 4, 2, 8, 0x20, &[0x80 | 3, 9]);
    for f in [overrun, short_raw, short_run, early_end] {
        assert!(matches!(decode(&f), Err(TgaError::InvalidData(_))));
    }
}

/// A header declaring the largest raster the format can name, over a
/// few bytes of data, is refused from the data it does not have —
/// even with every limit lifted, so the refusal is not the limit
/// check and no plane is allocated for it.
#[test]
fn a_declared_raster_the_data_cannot_fill_is_refused_before_allocation() {
    let unlimited = DecodeOptions::default().unlimited();
    for (image_type, depth) in [(2u8, 32u8), (10, 32), (3, 8), (11, 8), (2, 16), (10, 24)] {
        let f = file(image_type, 0xFFFF, 0xFFFF, depth, 0x00, &[0xFF; 64]);
        let started = std::time::Instant::now();
        assert!(
            matches!(decode_with(&f, &unlimited), Err(TgaError::InvalidData(_))),
            "type {image_type} depth {depth}"
        );
        // Filling a 4 - 17 GiB plane would take seconds; reading 64
        // bytes does not.
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
    // The densest run-length stream is one 128-pixel run packet per
    // 128 pixels: a 256 x 1 grey image needs 4 bytes, and 3 are not
    // enough whatever they say.
    assert!(decode(&file(11, 256, 1, 8, 0x20, &[0xFF, 5, 0xFF, 6])).is_ok());
    assert!(matches!(
        decode(&file(11, 256, 1, 8, 0x20, &[0xFF, 5, 0xFF])),
        Err(TgaError::InvalidData(_))
    ));
}

/// Footer offsets, extension-area offsets and developer-directory
/// offsets at the top of the 32-bit range are simply "not there".
#[test]
fn offsets_pointing_past_everything_read_as_absent() {
    let img = TgaImage::packed(4, 4, PixelFormat::Gray8, vec![1u8; 16]).unwrap();
    let mut f = encode(&img, &EncodeOptions::default().with_footer(true)).unwrap();
    let n = f.len();
    for offset in [
        u32::MAX,
        u32::MAX - 1,
        u32::MAX - 494,
        n as u32,
        n as u32 - 1,
    ] {
        f[n - 26..n - 22].copy_from_slice(&offset.to_le_bytes());
        f[n - 22..n - 18].copy_from_slice(&offset.to_le_bytes());
        let back = decode(&f).unwrap();
        assert_eq!(back.data(), img.data());
        assert!(back.extension.is_none());
        assert!(oxideav_tga::parse_tga_extension_area(&f).is_none());
        assert!(oxideav_tga::parse_tga_developer_area(&f).is_none());
        assert!(oxideav_tga::parse_tga_colour_correction_table(&f).is_none());
        assert!(oxideav_tga::parse_tga_scan_line_table(&f).is_none());
        assert!(matches!(oxideav_tga::parse_tga_postage_stamp(&f), Ok(None)));
    }
}

/// Colour-map indices are checked against the stored map for every
/// origin / length shape, and rebased.
#[test]
fn colour_map_indices_are_range_checked_and_rebased() {
    // Header with a 4-entry 24-bit map whose first entry index is 10.
    let build = |indices: &[u8]| {
        let mut f = vec![0u8; 18];
        f[1] = 1;
        f[2] = 1;
        f[3..5].copy_from_slice(&10u16.to_le_bytes());
        f[5..7].copy_from_slice(&4u16.to_le_bytes());
        f[7] = 24;
        f[12..14].copy_from_slice(&(indices.len() as u16).to_le_bytes());
        f[14..16].copy_from_slice(&1u16.to_le_bytes());
        f[16] = 8;
        f[17] = 0x20;
        f.extend_from_slice(&[0u8; 12]);
        f.extend_from_slice(indices);
        f
    };
    assert_eq!(
        decode(&build(&[10, 13, 11, 12])).unwrap().data(),
        &[0, 3, 1, 2]
    );
    assert!(matches!(
        decode(&build(&[10, 9])),
        Err(TgaError::InvalidData(_))
    ));
    assert!(matches!(
        decode(&build(&[14, 10])),
        Err(TgaError::InvalidData(_))
    ));
    assert!(matches!(
        decode(&build(&[255])),
        Err(TgaError::InvalidData(_))
    ));
}

// ---------------------------------------------------------------------------
// Encoder: four origins, colour-map origin
// ---------------------------------------------------------------------------

/// Every layout x compression x the four origins is an identity, and
/// the descriptor carries the origin bits asked for.
#[test]
fn the_encoder_writes_all_four_origins() {
    let (w, h) = (9u32, 6u32);
    let n = (w * h) as usize;
    let images = [
        TgaImage::packed(w, h, PixelFormat::Gray8, noise(n, 1)).unwrap(),
        TgaImage::packed(w, h, PixelFormat::Rgb24, noise(n * 3, 2)).unwrap(),
        TgaImage::packed(w, h, PixelFormat::Rgba, noise(n * 4, 3)).unwrap(),
        TgaImage::new_indexed(
            w,
            h,
            noise(n, 4).into_iter().map(|i| i % 5).collect(),
            Palette::new(vec![
                [1, 2, 3, 255],
                [4, 5, 6, 255],
                [7, 8, 9, 255],
                [10, 11, 12, 255],
                [13, 14, 15, 255],
            ]),
        )
        .unwrap(),
    ];
    for img in &images {
        for rle in [false, true] {
            for (rows, cols, bits) in [
                (RowOrder::BottomUp, ColumnOrder::LeftToRight, 0x00u8),
                (RowOrder::BottomUp, ColumnOrder::RightToLeft, 0x10),
                (RowOrder::TopDown, ColumnOrder::LeftToRight, 0x20),
                (RowOrder::TopDown, ColumnOrder::RightToLeft, 0x30),
            ] {
                let opts = EncodeOptions::default()
                    .with_rle(rle)
                    .with_row_order(rows)
                    .with_column_order(cols);
                let bytes = encode(img, &opts).unwrap();
                assert_eq!(bytes[17] & 0x30, bits, "{:?} rle {rle}", img.format);
                let described = info(&bytes).unwrap();
                assert_eq!(described.top_down, bits & 0x20 != 0);
                assert_eq!(described.right_to_left, bits & 0x10 != 0);
                let back = decode(&bytes).unwrap();
                assert_eq!(back.format, img.format);
                assert_eq!(
                    back.data(),
                    img.data(),
                    "{:?} rle {rle} {bits:#04x}",
                    img.format
                );
                assert_eq!(back.palette, img.palette);
            }
        }
    }
    // The default stays left-to-right: same bytes as not naming it.
    let img = &images[1];
    assert_eq!(
        encode(img, &EncodeOptions::default()).unwrap(),
        encode(
            img,
            &EncodeOptions::default().with_column_order(ColumnOrder::LeftToRight)
        )
        .unwrap()
    );
}

/// A right-to-left 16-bit file mirrors whole two-byte pixels.
#[test]
fn right_to_left_at_sixteen_bits_mirrors_whole_pixels() {
    let (w, h) = (6u32, 3u32);
    // 5-bit-exact channels, alpha 0 / 255: the 16-bit round trip is
    // exact.
    let mut plane = noise((w * h * 4) as usize, 9);
    for p in plane.as_chunks_mut::<4>().0 {
        for c in &mut p[..3] {
            *c = (*c & 0xF8) | (*c >> 5);
        }
        p[3] = if p[3] >= 0x80 { 0xFF } else { 0 };
    }
    let img = TgaImage::packed(w, h, PixelFormat::Rgba, plane).unwrap();
    for rle in [false, true] {
        let opts = EncodeOptions::default()
            .with_rle(rle)
            .with_column_order(ColumnOrder::RightToLeft)
            .with_true_colour_depth(TrueColourDepth::Bits16);
        let bytes = encode(&img, &opts).unwrap();
        assert_eq!(bytes[16], 16);
        assert_eq!(decode(&bytes).unwrap().data(), img.data());
    }
}

/// A non-zero Color Map Origin offsets the stored indices and the
/// header's First Entry Index; the decoded image is unchanged.
#[test]
fn colour_map_origin_offsets_stored_indices() {
    let pal = Palette::new(vec![[9, 8, 7, 255], [6, 5, 4, 255], [3, 2, 1, 255]]);
    let indices = vec![0u8, 1, 2, 2, 1, 0, 0, 0];
    let img = TgaImage::new_indexed(4, 2, indices.clone(), pal).unwrap();
    for rle in [false, true] {
        let opts = EncodeOptions::default()
            .with_rle(rle)
            .with_color_map_origin(200);
        let bytes = encode(&img, &opts).unwrap();
        let h = header(&bytes).unwrap();
        assert_eq!(h.cmap_first, 200);
        assert_eq!(h.cmap_length, 3);
        if !rle {
            // 18-byte header, 3 x 3-byte map, then the indices + 200.
            let stored: Vec<u8> = indices.iter().map(|i| i + 200).collect();
            assert_eq!(&bytes[18 + 9..], &stored[..]);
        }
        let back = decode(&bytes).unwrap();
        assert_eq!(back.data(), img.data());
        assert_eq!(back.palette, img.palette);
    }
    // 253 + 3 entries = 256: the last index is 255, still a byte.
    assert!(encode(&img, &EncodeOptions::default().with_color_map_origin(253)).is_ok());
    // 254 + 3 would need index 256.
    assert!(matches!(
        encode(&img, &EncodeOptions::default().with_color_map_origin(254)),
        Err(TgaError::InvalidData(_))
    ));
    // Not a colour-mapped layout: the option is ignored.
    let grey = TgaImage::packed(2, 2, PixelFormat::Gray8, vec![1, 2, 3, 4]).unwrap();
    let with = encode(&grey, &EncodeOptions::default().with_color_map_origin(999)).unwrap();
    assert_eq!(with, encode(&grey, &EncodeOptions::default()).unwrap());
}

/// A postage stamp of a colour-mapped file with a non-zero origin
/// shares the offset, so it reads back with the indices it was given.
#[test]
fn postage_stamp_indices_follow_the_colour_map_origin() {
    let pal = Palette::new(vec![[9, 8, 7, 255], [6, 5, 4, 255], [3, 2, 1, 255]]);
    let img = TgaImage::new_indexed(4, 2, vec![0, 1, 2, 2, 1, 0, 0, 0], pal.clone()).unwrap();
    let stamp = TgaImage::new_indexed(2, 1, vec![2, 1], pal).unwrap();
    let ext = oxideav_tga::ExtensionAreaInput {
        postage_stamp: Some(stamp),
        ..Default::default()
    };
    let opts = EncodeOptions::default()
        .with_color_map_origin(100)
        .with_extension(ext);
    let bytes = encode(&img, &opts).unwrap();
    let read = oxideav_tga::parse_tga_postage_stamp(&bytes)
        .unwrap()
        .expect("stamp present");
    // The stamp reader returns packed RGBA: entries 2 and 1.
    assert_eq!(read.data(), &[3, 2, 1, 255, 6, 5, 4, 255]);
}

// ---------------------------------------------------------------------------
// Layout conversion kernels
// ---------------------------------------------------------------------------

/// `to_rgba8` / `to_rgb8` against the per-pixel definition, for every
/// layout — including the documented fallbacks: a palette index the
/// palette does not cover is transparent black, rows a short buffer
/// does not hold are opaque black.
#[test]
fn packed_conversions_match_the_per_pixel_definition() {
    let (w, h) = (7usize, 5usize);
    let pal: Vec<[u8; 4]> = (0..6u8)
        .map(|i| [i * 40, 255 - i * 40, i, 200 + i])
        .collect();
    let layouts = [
        (PixelFormat::Rgba, noise(w * h * 4, 21)),
        (PixelFormat::Rgb24, noise(w * h * 3, 22)),
        (PixelFormat::Gray8, noise(w * h, 23)),
        // Indices 0..=7 over a 6-entry palette: 6 and 7 are uncovered.
        (
            PixelFormat::Pal8,
            noise(w * h, 24).into_iter().map(|i| i % 8).collect(),
        ),
    ];
    for (format, plane) in layouts {
        let bpp = format.bytes_per_pixel();
        for keep_rows in [h, 3] {
            // A caller-assembled image: a plane holding `keep_rows`
            // whole rows and half of the next.
            let mut img = TgaImage::packed(w as u32, h as u32, format, plane.clone()).unwrap();
            if keep_rows < h {
                img.planes[0]
                    .data
                    .truncate(keep_rows * w * bpp + (w / 2) * bpp);
            }
            if format == PixelFormat::Pal8 {
                img.palette = Some(Palette::new(pal.clone()));
            }
            let mut rgba = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    let px = if y < keep_rows {
                        let s = &plane[(y * w + x) * bpp..];
                        match format {
                            PixelFormat::Rgba => [s[0], s[1], s[2], s[3]],
                            PixelFormat::Rgb24 => [s[0], s[1], s[2], 255],
                            PixelFormat::Gray8 => [s[0], s[0], s[0], 255],
                            _ => pal.get(usize::from(s[0])).copied().unwrap_or([0; 4]),
                        }
                    } else {
                        [0, 0, 0, 255]
                    };
                    rgba.extend_from_slice(&px);
                }
            }
            let rgb: Vec<u8> = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            assert_eq!(img.to_rgba8(), rgba, "{format:?} rgba, {keep_rows} rows");
            assert_eq!(img.to_rgb8(), rgb, "{format:?} rgb, {keep_rows} rows");
        }
    }
}
