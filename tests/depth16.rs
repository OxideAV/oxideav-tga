//! The 15 / 16-bit true-colour encode path (`EncodeOptions::
//! true_colour_depth`): the 5-bit quantiser is the exact inverse of
//! the decoder's expansion, so 15 / 16-bit files round-trip bit-exactly
//! through `decode` → `encode`, and 5-bit-exact pictures through
//! `encode` → `decode`.

use oxideav_tga::{
    decode, encode, info, EncodeOptions, ImageType, Palette, RowOrder, TgaImage, TgaPixelFormat,
    TrueColourDepth, TGA_HEADER_SIZE,
};

/// 5 → 8 expansion the decoder documents: repeat the top three bits.
fn expand5(v: u8) -> u8 {
    (v << 3) | (v >> 2)
}

/// A picture whose channels are all 5-bit-exact and whose alpha is 0 / 255.
fn five_bit_rgba(w: u32, h: u32, seed: u8) -> TgaImage {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for i in 0..(w * h) as usize {
        let k = (i as u8).wrapping_mul(7).wrapping_add(seed);
        data.extend_from_slice(&[
            expand5(k & 0x1F),
            expand5((k >> 2) & 0x1F),
            expand5((k >> 3) & 0x1F),
            if k & 1 == 0 { 0xFF } else { 0 },
        ]);
    }
    TgaImage::from_rgba8(w, h, data).unwrap()
}

/// A hand-built uncompressed 16-bit type-2 file, bottom-up, 2×2.
fn hand_built_16bit() -> Vec<u8> {
    let mut f = vec![0u8; TGA_HEADER_SIZE];
    f[2] = 2;
    f[12] = 2;
    f[14] = 2;
    f[16] = 16;
    f[17] = 0x01; // bottom-up, 1 attribute bit
                  // Pixels in storage order (bottom row first): A R G B as 5-5-5 + bit.
    let px = |a: bool, r: u16, g: u16, b: u16| -> [u8; 2] {
        (((a as u16) << 15) | (r << 10) | (g << 5) | b).to_le_bytes()
    };
    f.extend_from_slice(&px(true, 31, 0, 0));
    f.extend_from_slice(&px(false, 0, 31, 0));
    f.extend_from_slice(&px(true, 0, 0, 31));
    f.extend_from_slice(&px(false, 16, 8, 4));
    f
}

#[test]
fn a_16bit_file_decodes_to_the_documented_expansion_and_re_encodes_byte_exactly() {
    let bytes = hand_built_16bit();
    let img = decode(&bytes).unwrap();
    assert_eq!(img.format, TgaPixelFormat::Rgba);
    // Top row of the picture is the second stored row.
    assert_eq!(
        img.data(),
        &[
            0,
            0,
            255,
            255, //
            expand5(16),
            expand5(8),
            expand5(4),
            0, //
            255,
            0,
            0,
            255, //
            0,
            255,
            0,
            0,
        ]
    );
    let info16 = info(&bytes).unwrap();
    assert_eq!((info16.depth, info16.attribute_bits), (16, 1));
    assert!(info16.has_alpha);

    let out = encode(
        &img,
        &EncodeOptions::default()
            .with_rle(false)
            .with_row_order(RowOrder::BottomUp)
            .with_true_colour_depth(TrueColourDepth::Bits16),
    )
    .unwrap();
    assert_eq!(
        out, bytes,
        "decode → encode at 16 bits is the identity on the file"
    );
}

#[test]
fn five_bit_exact_pictures_round_trip_at_16_bits_on_every_axis() {
    let img = five_bit_rgba(37, 11, 3);
    for rle in [false, true] {
        for order in [RowOrder::TopDown, RowOrder::BottomUp] {
            let opts = EncodeOptions::default()
                .with_rle(rle)
                .with_row_order(order)
                .with_true_colour_depth(TrueColourDepth::Bits16);
            let bytes = encode(&img, &opts).unwrap();
            let i = info(&bytes).unwrap();
            assert_eq!(i.depth, 16);
            assert_eq!(i.attribute_bits, 1);
            assert_eq!(i.top_down, order == RowOrder::TopDown);
            assert_eq!(
                i.image_type,
                if rle {
                    ImageType::RleTrueColour
                } else {
                    ImageType::UncompressedTrueColour
                }
            );
            let back = decode(&bytes).unwrap();
            assert_eq!(back.format, TgaPixelFormat::Rgba);
            assert_eq!(back.data(), img.data(), "rle={rle} {order:?}");
            // Uncompressed: exactly 2 bytes per pixel on the wire.
            if !rle {
                assert_eq!(bytes.len(), TGA_HEADER_SIZE + 37 * 11 * 2);
            }
        }
    }
}

#[test]
fn bits15_has_no_attribute_bit_and_decodes_opaque() {
    let img = five_bit_rgba(5, 3, 9);
    let bytes = encode(
        &img,
        &EncodeOptions::default().with_true_colour_depth(TrueColourDepth::Bits15),
    )
    .unwrap();
    let i = info(&bytes).unwrap();
    assert_eq!((i.depth, i.attribute_bits), (15, 0));
    assert!(!i.has_alpha);
    let back = decode(&bytes).unwrap();
    let mut expect = img.clone();
    expect.force_opaque();
    assert_eq!(back.data(), expect.data());
    // The top bit is clear in every stored pixel.
    let uncompressed = encode(
        &img,
        &EncodeOptions::default()
            .with_rle(false)
            .with_true_colour_depth(TrueColourDepth::Bits15),
    )
    .unwrap();
    for px in uncompressed[TGA_HEADER_SIZE..].chunks_exact(2) {
        assert_eq!(px[1] & 0x80, 0);
    }
}

#[test]
fn rgb24_at_16_bits_declares_no_attribute_bits_and_decodes_opaque_rgba() {
    let img = TgaImage::from_rgb8(
        2,
        1,
        vec![
            expand5(1),
            expand5(2),
            expand5(3),
            expand5(31),
            0,
            expand5(15),
        ],
    )
    .unwrap();
    let bytes = encode(
        &img,
        &EncodeOptions::default().with_true_colour_depth(TrueColourDepth::Bits16),
    )
    .unwrap();
    let i = info(&bytes).unwrap();
    assert_eq!((i.depth, i.attribute_bits), (16, 0));
    let back = decode(&bytes).unwrap();
    assert_eq!(
        back.format,
        TgaPixelFormat::Rgba,
        "16-bit files decode as Rgba"
    );
    assert_eq!(back.to_rgb8(), img.data());
    // An opaque source sets the stored attribute bit (a reader that
    // honours the bit sees the picture opaque), even though the header
    // declares no attribute bits for an `Rgb24` source.
    assert!(back.data().chunks_exact(4).all(|p| p[3] == 0xFF));
    // ... and `alpha_bits` can override the declared count for an Rgba source.
    let rgba = TgaImage::from_rgba8(1, 1, vec![0, 0, 0, 255]).unwrap();
    let bytes = encode(
        &rgba,
        &EncodeOptions::default()
            .with_true_colour_depth(TrueColourDepth::Bits16)
            .with_alpha_bits(Some(0u8)),
    )
    .unwrap();
    assert_eq!(info(&bytes).unwrap().attribute_bits, 0);
}

#[test]
fn quantisation_keeps_the_top_five_bits() {
    // Not 5-bit-exact: the low three bits are dropped, the top five kept.
    let img =
        TgaImage::from_rgba8(2, 1, vec![0xFF, 0x07, 0x88, 0x80, 0x00, 0xF9, 0x10, 0x7F]).unwrap();
    let bytes = encode(
        &img,
        &EncodeOptions::default().with_true_colour_depth(TrueColourDepth::Bits16),
    )
    .unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(
        back.data(),
        &[
            expand5(0xFF >> 3),
            expand5(0x07 >> 3),
            expand5(0x88 >> 3),
            0xFF, // a = 0x80 → bit set
            expand5(0),
            expand5(0xF9 >> 3),
            expand5(0x10 >> 3),
            0x00, // a = 0x7F → bit clear
        ]
    );
    // Re-encoding the decoded picture is now exact (idempotent quantiser).
    let again = encode(
        &back,
        &EncodeOptions::default().with_true_colour_depth(TrueColourDepth::Bits16),
    )
    .unwrap();
    assert_eq!(again, bytes);
}

#[test]
fn gray8_and_pal8_ignore_the_true_colour_depth() {
    let g = TgaImage::from_gray8(3, 1, vec![1, 2, 3]).unwrap();
    let opts = EncodeOptions::default().with_true_colour_depth(TrueColourDepth::Bits16);
    let bytes = encode(&g, &opts).unwrap();
    assert_eq!(info(&bytes).unwrap().depth, 8);
    assert_eq!(decode(&bytes).unwrap(), g);
    let p = TgaImage::new_indexed(
        2,
        1,
        vec![0, 1],
        Palette::new(vec![[1, 2, 3, 255], [4, 5, 6, 255]]),
    )
    .unwrap();
    let bytes = encode(&p, &opts).unwrap();
    assert_eq!(info(&bytes).unwrap().depth, 8);
    assert_eq!(decode(&bytes).unwrap(), p);
}

#[test]
fn drop_opaque_alpha_composes_with_16_bits() {
    let mut img = five_bit_rgba(4, 4, 1);
    img.force_opaque();
    let bytes = encode(
        &img,
        &EncodeOptions::default()
            .with_true_colour_depth(TrueColourDepth::Bits16)
            .with_drop_opaque_alpha(true),
    )
    .unwrap();
    // Opaque Rgba narrows to Rgb24 first, so no attribute bit is declared.
    let i = info(&bytes).unwrap();
    assert_eq!((i.depth, i.attribute_bits), (16, 0));
    assert_eq!(decode(&bytes).unwrap().to_rgb8(), img.to_rgb8());
}

#[test]
fn a_16bit_postage_stamp_follows_the_parent_depth() {
    use oxideav_tga::{encode_tga_with_extension, parse_tga_postage_stamp, ExtensionAreaInput};
    let img = five_bit_rgba(8, 8, 5);
    let base = encode(
        &img,
        &EncodeOptions::default()
            .with_rle(false)
            .with_true_colour_depth(TrueColourDepth::Bits16),
    )
    .unwrap();
    let stamp = five_bit_rgba(2, 2, 6);
    let ext = ExtensionAreaInput {
        postage_stamp: Some(stamp.clone()),
        ..ExtensionAreaInput::default()
    };
    let full = encode_tga_with_extension(&base, &ext).unwrap();
    let back = parse_tga_postage_stamp(&full).unwrap().unwrap();
    assert_eq!(back.to_rgba8(), stamp.to_rgba8());
}
