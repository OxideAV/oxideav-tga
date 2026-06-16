//! Round 327 — §C.6.10 Postage Stamp Image (Field 26) is now *generated*,
//! not just carried.
//!
//! The extension area's postage-stamp thumbnail could be parsed
//! (`parse_tga_postage_stamp`) and its dimension header inspected
//! (`PostageStamp` typed view), and the encoder accepts a caller-supplied
//! `ExtensionAreaInput::postage_stamp`. This round adds the missing
//! producer side so a caller can derive a spec-conformant thumbnail from a
//! decoded image, mirroring the carry-then-apply pattern of
//! `PixelAspectRatio::resampled` (round 323):
//!
//! * `PostageStamp::recommended_for(w, h)` — thumbnail dimensions for a
//!   source frame: the longer edge scaled down to the recommended
//!   64-pixel cap (aspect-ratio preserving, downscale-only, ≥1 per axis).
//! * `PostageStamp::subsample(&image)` — point sub-sampled thumbnail in
//!   the source's pixel format, feedable straight to
//!   `ExtensionAreaInput::postage_stamp`.
//!
//! Per the TGA 2.0 FFS Field 26: the stamp is "a smaller representation of
//! the original image … create one using sub-sampling techniques … must be
//! stored in the same format as the normal image … If the original image
//! is color mapped, DO NOT average the postage stamp, as you will create
//! new colors not in your map." Point sub-sampling copies whole source
//! pixels and never averages, satisfying the colour-mapped caveat for
//! every format.

use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga_postage_stamp,
    ExtensionAreaInput, PostageStamp, TgaImage, TgaPixelFormat, TGA_POSTAGE_STAMP_RECOMMENDED_MAX,
};

fn rgba_image(width: u32, height: u32, data: Vec<u8>) -> TgaImage {
    assert_eq!(data.len(), width as usize * height as usize * 4);
    TgaImage {
        width,
        height,
        pixel_format: TgaPixelFormat::Rgba,
        data,
        pts: None,
    }
}

#[test]
fn recommended_for_keeps_small_images_unchanged() {
    // A source already within 64 x 64 is its own best stamp.
    assert_eq!(
        PostageStamp::recommended_for(16, 16),
        PostageStamp::new(16, 16)
    );
    assert_eq!(
        PostageStamp::recommended_for(64, 64),
        PostageStamp::new(64, 64)
    );
    assert_eq!(PostageStamp::recommended_for(1, 1), PostageStamp::new(1, 1));
    assert_eq!(
        PostageStamp::recommended_for(64, 10),
        PostageStamp::new(64, 10)
    );
}

#[test]
fn recommended_for_caps_longer_edge_at_64() {
    // 256 x 128 → longer edge (width) scaled to 64, height by same ratio.
    assert_eq!(
        PostageStamp::recommended_for(256, 128),
        PostageStamp::new(64, 32)
    );
    // 128 x 256 → longer edge (height) scaled to 64.
    assert_eq!(
        PostageStamp::recommended_for(128, 256),
        PostageStamp::new(32, 64)
    );
    // Square large image → 64 x 64.
    assert_eq!(
        PostageStamp::recommended_for(512, 512),
        PostageStamp::new(64, 64)
    );
    // Both edges within the recommended box for the output.
    let s = PostageStamp::recommended_for(1000, 700);
    assert!(s.within_recommended_size());
    assert!(s.width <= TGA_POSTAGE_STAMP_RECOMMENDED_MAX);
    assert!(s.height <= TGA_POSTAGE_STAMP_RECOMMENDED_MAX);
}

#[test]
fn recommended_for_floors_minor_axis_at_one() {
    // 1000 x 3 → height ratio 3 * 64 / 1000 = 0.192 → floored to 1, not 0.
    assert_eq!(
        PostageStamp::recommended_for(1000, 3),
        PostageStamp::new(64, 1)
    );
    assert_eq!(
        PostageStamp::recommended_for(3, 1000),
        PostageStamp::new(1, 64)
    );
}

#[test]
fn recommended_for_degenerate_source_is_unset() {
    assert_eq!(PostageStamp::recommended_for(0, 10), PostageStamp::UNSET);
    assert_eq!(PostageStamp::recommended_for(10, 0), PostageStamp::UNSET);
    assert!(PostageStamp::recommended_for(0, 0).is_unset());
}

#[test]
fn subsample_small_image_returns_none() {
    // Already within the recommended box → no separate thumbnail.
    let img = rgba_image(4, 4, vec![0u8; 4 * 4 * 4]);
    assert!(PostageStamp::subsample(&img).is_none());
    let img = rgba_image(64, 64, vec![7u8; 64 * 64 * 4]);
    assert!(PostageStamp::subsample(&img).is_none());
}

#[test]
fn subsample_empty_image_returns_none() {
    let img = rgba_image(0, 0, Vec::new());
    assert!(PostageStamp::subsample(&img).is_none());
}

#[test]
fn subsample_downscales_to_recommended_size_and_format() {
    // 128 x 128 RGBA → 64 x 64 thumbnail, same format.
    let mut data = Vec::with_capacity(128 * 128 * 4);
    for y in 0..128u32 {
        for x in 0..128u32 {
            data.extend_from_slice(&[x as u8, y as u8, 0, 255]);
        }
    }
    let img = rgba_image(128, 128, data);
    let stamp = PostageStamp::subsample(&img).expect("downscale");
    assert_eq!((stamp.width, stamp.height), (64, 64));
    assert_eq!(stamp.pixel_format, TgaPixelFormat::Rgba);
    assert_eq!(stamp.data.len(), 64 * 64 * 4);

    // Nearest-neighbour: thumbnail pixel (dx,dy) samples source
    // (dx*128/64, dy*128/64) = (2*dx, 2*dy).
    let tpx = |x: usize, y: usize| {
        let i = (y * 64 + x) * 4;
        &stamp.data[i..i + 4]
    };
    assert_eq!(tpx(0, 0), &[0, 0, 0, 255]);
    assert_eq!(tpx(1, 0), &[2, 0, 0, 255]);
    assert_eq!(tpx(0, 1), &[0, 2, 0, 255]);
    assert_eq!(tpx(10, 20), &[20, 40, 0, 255]);
}

#[test]
fn subsample_only_copies_source_colours_never_averages() {
    // Two-colour image: the spec's "DO NOT average … create new colors"
    // rule. Point sub-sampling must only ever emit one of the two source
    // colours, never a blended third.
    let a = [200u8, 10, 30, 255];
    let b = [5u8, 220, 90, 255];
    let mut data = Vec::with_capacity(200 * 100 * 4);
    for y in 0..100u32 {
        for x in 0..200u32 {
            data.extend_from_slice(if (x + y) % 2 == 0 { &a } else { &b });
        }
    }
    let img = rgba_image(200, 100, data);
    let stamp = PostageStamp::subsample(&img).expect("downscale");
    assert_eq!((stamp.width, stamp.height), (64, 32));
    for px in stamp.data.chunks_exact(4) {
        assert!(
            px == a || px == b,
            "thumbnail pixel {px:?} is not one of the two source colours"
        );
    }
}

#[test]
fn subsample_preserves_gray8_and_pts() {
    let mut data = Vec::with_capacity(130 * 10);
    for y in 0..10u32 {
        for x in 0..130u32 {
            data.push((x ^ y) as u8);
        }
    }
    let img = TgaImage {
        width: 130,
        height: 10,
        pixel_format: TgaPixelFormat::Gray8,
        data,
        pts: Some(42),
    };
    let stamp = PostageStamp::subsample(&img).expect("downscale gray");
    assert_eq!(stamp.pixel_format, TgaPixelFormat::Gray8);
    // 130 x 10: longer edge 130 → 64; height 10*64/130 = 4.
    assert_eq!((stamp.width, stamp.height), (64, 4));
    assert_eq!(stamp.data.len(), 64 * 4);
    assert_eq!(stamp.pts, Some(42));
}

#[test]
fn generated_stamp_round_trips_through_extension_writer() {
    // End-to-end: encode a base image, generate a thumbnail from the same
    // pixel buffer, append it via the extension writer, then recover it.
    let w = 100u16;
    let h = 80u16;
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as u32 {
        for x in 0..w as u32 {
            rgba.extend_from_slice(&[(x * 2) as u8, (y * 3) as u8, 64, 255]);
        }
    }
    let base = encode_tga_uncompressed(w, h, &rgba).expect("encode base");

    let source = rgba_image(w as u32, h as u32, rgba);
    let stamp = PostageStamp::subsample(&source).expect("generate stamp");
    let expected = stamp.clone();

    let ext = ExtensionAreaInput {
        postage_stamp: Some(stamp),
        ..Default::default()
    };
    let extended = encode_tga_with_extension(&base, &ext).expect("encode with stamp");

    let recovered = parse_tga_postage_stamp(&extended)
        .expect("parse stamp ok")
        .expect("stamp present");
    // 100 x 80: longer edge 100 → 64; height 80 * 64 / 100 = 51.
    assert_eq!((recovered.width, recovered.height), (64, 51));
    assert_eq!((expected.width, expected.height), (64, 51));
    assert_eq!(recovered.pixel_format, expected.pixel_format);
    // The base image's RGBA carried only opaque pixels, so the writer
    // auto-selected 24 bpp; the recovered thumbnail therefore comes back
    // with alpha forced to 0xFF. Compare the colour channels exactly.
    for (got, want) in recovered
        .data
        .chunks_exact(4)
        .zip(expected.data.chunks_exact(4))
    {
        assert_eq!(&got[0..3], &want[0..3]);
    }
}
