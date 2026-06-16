//! Round 323 — §C.6.5 Pixel Aspect Ratio (Field 19) is now *applied*,
//! not just carried.
//!
//! The extension area's pixel-aspect field had only scalar helpers
//! (`corrected_display_width` / `corrected_display_height`). This round
//! adds the resample apply-path that mirrors the established
//! carry-then-apply pattern used by `GammaValue::apply_to_image` and
//! `TgaColourCorrectionTable::apply_to_image`:
//!
//! * `corrected_display_dimensions` — square-display-pixel size derived
//!   by **upscaling the shorter pixel axis only** (no source sample is
//!   dropped).
//! * `resampled` — nearest-neighbour resample to a new `TgaImage`.
//! * `apply_to_image` — in-place companion.
//!
//! Per the TGA 2.0 FFS §C.6.5: SHORT 0 is the pixel width (numerator),
//! SHORT 1 the pixel height (denominator); equal non-zero values mean
//! square pixels and a zero denominator means "not specified".

use oxideav_tga::{PixelAspectRatio, TgaImage, TgaPixelFormat};

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
fn unset_and_square_yield_no_dimensions() {
    // (0, 0) sentinel → unset.
    assert_eq!(
        PixelAspectRatio::UNSET.corrected_display_dimensions(10, 6),
        None
    );
    // Zero denominator → unset per §C.6.5.
    assert_eq!(
        PixelAspectRatio::new(3, 0).corrected_display_dimensions(10, 6),
        None
    );
    // Equal non-zero pair → square pixels → dimensions unchanged.
    assert_eq!(
        PixelAspectRatio::new(2, 2).corrected_display_dimensions(10, 6),
        Some((10, 6))
    );
}

#[test]
fn wide_pixels_stretch_width_only() {
    // Pixel width:height = 2:1 → wide pixels → stretch width ×2.
    let par = PixelAspectRatio::new(2, 1);
    assert_eq!(par.corrected_display_dimensions(4, 3), Some((8, 3)));
}

#[test]
fn tall_pixels_stretch_height_only() {
    // Pixel width:height = 1:2 → tall pixels → stretch height ×2.
    let par = PixelAspectRatio::new(1, 2);
    assert_eq!(par.corrected_display_dimensions(4, 3), Some((4, 6)));
}

#[test]
fn resampled_wide_doubles_columns_nearest_neighbour() {
    // 2x1 RGBA: red, green. Pixel width 2:1 → display width doubles to 4.
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let mut data = Vec::new();
    data.extend_from_slice(&red);
    data.extend_from_slice(&green);
    let img = rgba_image(2, 1, data);

    let par = PixelAspectRatio::new(2, 1);
    let out = par.resampled(&img).expect("wide → resample");
    assert_eq!((out.width, out.height), (4, 1));
    // Nearest-neighbour: sx = dx * 2 / 4 → 0,0,1,1.
    let px = |i: usize| &out.data[i * 4..i * 4 + 4];
    assert_eq!(px(0), &red);
    assert_eq!(px(1), &red);
    assert_eq!(px(2), &green);
    assert_eq!(px(3), &green);
}

#[test]
fn resampled_tall_doubles_rows_nearest_neighbour() {
    // 1x2 RGBA stacked rows. Pixel height 2 → display height doubles to 4.
    let top = [10, 20, 30, 255];
    let bot = [40, 50, 60, 255];
    let mut data = Vec::new();
    data.extend_from_slice(&top);
    data.extend_from_slice(&bot);
    let img = rgba_image(1, 2, data);

    let par = PixelAspectRatio::new(1, 2);
    let out = par.resampled(&img).expect("tall → resample");
    assert_eq!((out.width, out.height), (1, 4));
    let px = |i: usize| &out.data[i * 4..i * 4 + 4];
    assert_eq!(px(0), &top);
    assert_eq!(px(1), &top);
    assert_eq!(px(2), &bot);
    assert_eq!(px(3), &bot);
}

#[test]
fn square_and_unset_resample_returns_none() {
    let img = rgba_image(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(PixelAspectRatio::new(3, 3).resampled(&img).is_none());
    assert!(PixelAspectRatio::UNSET.resampled(&img).is_none());
    assert!(PixelAspectRatio::new(5, 0).resampled(&img).is_none());
}

#[test]
fn empty_image_resample_returns_none() {
    let img = rgba_image(0, 0, Vec::new());
    assert!(PixelAspectRatio::new(2, 1).resampled(&img).is_none());
}

#[test]
fn apply_to_image_mutates_in_place() {
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let mut data = Vec::new();
    data.extend_from_slice(&red);
    data.extend_from_slice(&green);
    let mut img = rgba_image(2, 1, data);

    PixelAspectRatio::new(2, 1).apply_to_image(&mut img);
    assert_eq!((img.width, img.height), (4, 1));
    assert_eq!(img.data.len(), 4 * 4);
}

#[test]
fn apply_to_image_noop_for_square() {
    let mut img = rgba_image(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    let before = img.data.clone();
    PixelAspectRatio::new(4, 4).apply_to_image(&mut img);
    assert_eq!((img.width, img.height), (2, 1));
    assert_eq!(img.data, before);
}

#[test]
fn resample_preserves_pixel_format_gray8() {
    // Gray8: 2 columns, width:height 2:1 → 4 columns.
    let img = TgaImage {
        width: 2,
        height: 1,
        pixel_format: TgaPixelFormat::Gray8,
        data: vec![100, 200],
        pts: Some(7),
    };
    let out = PixelAspectRatio::new(2, 1)
        .resampled(&img)
        .expect("gray resample");
    assert_eq!(out.pixel_format, TgaPixelFormat::Gray8);
    assert_eq!((out.width, out.height), (4, 1));
    assert_eq!(out.data, vec![100, 100, 200, 200]);
    assert_eq!(out.pts, Some(7));
}

#[test]
fn non_integer_ratio_rounds_to_nearest() {
    // 3:2 pixel ratio on a 4-wide frame → round(4 * 1.5) = 6.
    let par = PixelAspectRatio::new(3, 2);
    assert_eq!(par.corrected_display_dimensions(4, 2), Some((6, 2)));
    let img = rgba_image(4, 2, vec![0u8; 4 * 2 * 4]);
    let out = par.resampled(&img).expect("3:2 resample");
    assert_eq!((out.width, out.height), (6, 2));
    assert_eq!(out.data.len(), 6 * 2 * 4);
}
