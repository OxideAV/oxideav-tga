//! Round 489 regressions.
//!
//! * A footer-less file whose pixel data ends in the bytes of the
//!   TGA 2.0 footer signature must not be read back as a TGA 2.0 file
//!   whose offsets are pixel bytes (scheduled fuzz finding, target
//!   `contract_identity`, 2026-11).

use oxideav_tga::{
    EncodeOptions, Metadata, PixelFormat, TgaImage, decode, encode, info, parse_tga_footer,
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
