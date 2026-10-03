//! The image-crate contract (`IMAGE_CRATE_API`) on `oxideav-tga`: root
//! vocabulary, native layouts, `to_rgb8` / `to_rgba8` exactness,
//! `DecodeOptions` limits before allocation, lossless
//! `decode(encode(img)) == img` for every layout, `EncodeOptions` as
//! fields, metadata / extension round trip, error shape, and the
//! pre-contract wrappers staying byte-identical to the new paths.

use std::io::Cursor;

use oxideav_tga::{
    decode, decode_from, decode_rgb8, decode_rgba8, decode_with, encode, encode_rgb8, encode_rgba8,
    encode_to, header, info, probe, ColorInfo, ColorMapEntrySize, ColorRange, DecodeOptions,
    EncodeOptions, Error, ImageOrigin, ImageType, Metadata, Palette, PixelFormat, Plane, RowOrder,
    TgaError, TgaImage, TgaPixelFormat, TGA_FOOTER_SIZE, TGA_HEADER_SIZE,
};

fn seeded(len: usize, seed: u32) -> Vec<u8> {
    let mut s = seed;
    (0..len)
        .map(|i| {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            // runs every few pixels so RLE has both packet kinds
            if (i / 7) % 3 == 0 {
                (i / 7) as u8
            } else {
                (s >> 16) as u8
            }
        })
        .collect()
}

fn rgba(w: u32, h: u32) -> TgaImage {
    TgaImage::from_rgba8(w, h, seeded(w as usize * h as usize * 4, 1))
}

fn rgb(w: u32, h: u32) -> TgaImage {
    TgaImage::from_rgb8(w, h, seeded(w as usize * h as usize * 3, 2))
}

fn gray(w: u32, h: u32) -> TgaImage {
    TgaImage::from_gray8(w, h, seeded(w as usize * h as usize, 3))
}

fn pal(w: u32, h: u32, alpha: bool) -> TgaImage {
    let entries: Vec<[u8; 4]> = (0..19u8)
        .map(|i| {
            [
                i * 13,
                255 - i * 9,
                i * 5,
                if alpha && i % 4 == 0 { 0x80 } else { 0xFF },
            ]
        })
        .collect();
    let idx: Vec<u8> = seeded(w as usize * h as usize, 4)
        .into_iter()
        .map(|v| v % 19)
        .collect();
    TgaImage::new_indexed(w, h, idx, Palette::new(entries)).unwrap()
}

// ---- probe / info / header -------------------------------------------------

#[test]
fn probe_is_total_and_discriminating() {
    assert!(!probe(&[]));
    assert!(!probe(&[0u8; 17]));
    assert!(!probe(&[0u8; 18])); // image type 0, zero geometry
    assert!(!probe(b"GIF89a\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"));
    let f = encode(&rgb(3, 2), &EncodeOptions::default()).unwrap();
    assert!(probe(&f));
    assert!(probe(&f[..TGA_HEADER_SIZE])); // header plausibility alone
                                           // A footer alone is the strong signal, even on an otherwise bogus body.
    let mut junk = vec![0xFFu8; 40];
    junk.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
    junk.extend_from_slice(b"TRUEVISION-XFILE.\0");
    assert!(probe(&junk));
    assert!(decode(&junk).is_err());
}

#[test]
fn info_describes_every_layout_without_decoding() {
    let cases: Vec<(TgaImage, PixelFormat, bool, u8, ImageType)> = vec![
        (
            rgba(5, 4),
            PixelFormat::Rgba,
            true,
            32,
            ImageType::RleTrueColour,
        ),
        (
            rgb(5, 4),
            PixelFormat::Rgb24,
            false,
            24,
            ImageType::RleTrueColour,
        ),
        (
            gray(5, 4),
            PixelFormat::Gray8,
            false,
            8,
            ImageType::RleGrayscale,
        ),
        (
            pal(5, 4, true),
            PixelFormat::Pal8,
            true,
            8,
            ImageType::RleColourMapped,
        ),
        (
            pal(5, 4, false),
            PixelFormat::Pal8,
            false,
            8,
            ImageType::RleColourMapped,
        ),
    ];
    for (img, fmt, alpha, depth, ty) in cases {
        let f = encode(&img, &EncodeOptions::default()).unwrap();
        // Truncate the pixel data: info must still succeed (header only).
        let truncated = &f[..TGA_HEADER_SIZE + 2];
        let i = info(truncated).unwrap();
        assert_eq!((i.width, i.height), (5, 4), "{fmt:?}");
        assert_eq!(i.format, fmt);
        assert_eq!(i.frames, 1);
        assert_eq!(i.has_alpha, alpha, "{fmt:?}");
        assert_eq!(i.depth, depth);
        assert_eq!(i.image_type, ty);
        assert!(i.top_down);
        assert!(!i.right_to_left);
        assert!(!i.has_icc && !i.has_exif && !i.has_xmp);
        assert_eq!(i.color, ColorInfo::tga_default());
        assert!(!i.has_footer);
        assert_eq!(
            i.color_map_length,
            if fmt == PixelFormat::Pal8 { 19 } else { 0 }
        );
        let h = header(&f).unwrap();
        assert_eq!((h.width, h.height, h.depth), (5, 4, depth));
    }
}

#[test]
fn info_reports_footer_and_extension_area() {
    let mut img = rgba(4, 4);
    img.metadata = Metadata::new().with_gamma(1.8);
    let f = encode(&img, &EncodeOptions::default()).unwrap();
    let i = info(&f).unwrap();
    assert!(i.has_footer && i.has_extension_area && !i.has_developer_area);
    let bare = encode(&rgba(4, 4), &EncodeOptions::default().with_footer(true)).unwrap();
    let i = info(&bare).unwrap();
    assert!(i.has_footer && !i.has_extension_area);
    assert_eq!(bare.len(), f.len() - 495);
    assert_eq!(&bare[bare.len() - 18..], b"TRUEVISION-XFILE.\0");
    let _ = TGA_FOOTER_SIZE;
}

// ---- decode: native layouts + conversions -----------------------------------

#[test]
fn decode_returns_native_layouts_and_conversions_are_exact() {
    for img in [rgba(7, 3), rgb(7, 3), gray(7, 3), pal(7, 3, true)] {
        let f = encode(&img, &EncodeOptions::default()).unwrap();
        let back = decode(&f).unwrap();
        assert_eq!(back.format, img.format);
        assert_eq!(back.planes.len(), 1);
        assert_eq!(back.stride(), img.width as usize * img.bytes_per_pixel());
        assert_eq!(back.as_bytes().unwrap(), img.data());
        assert_eq!(back.color, ColorInfo::tga_default());
        assert_eq!(back.color.range, ColorRange::Full);
        assert!(back.metadata.is_empty());
        assert!(back.extension.is_none());
        assert_eq!(back.palette, img.palette);
        // raw paths agree with the methods
        let rgb8 = decode_rgb8(&f).unwrap();
        let rgba8 = decode_rgba8(&f).unwrap();
        assert_eq!(rgb8.data, back.to_rgb8());
        assert_eq!(rgba8.data, back.to_rgba8());
        assert_eq!(rgb8.data.len(), 7 * 3 * 3);
        assert_eq!(rgba8.data.len(), 7 * 3 * 4);
        assert_eq!(rgb8.stride(), 21);
        assert_eq!(rgba8.stride(), 28);
        // rgb8 is rgba8 minus alpha
        for (a, b) in rgba8.data.chunks_exact(4).zip(rgb8.data.chunks_exact(3)) {
            assert_eq!(&a[..3], b);
        }
        assert_eq!(back.try_to_rgba8().unwrap(), back.to_rgba8());
        assert_eq!(back.clone().into_raw(), img.data().to_vec());
    }
}

#[test]
fn gray_and_palette_expansion_kernels() {
    let g = TgaImage::from_gray8(2, 1, vec![0, 200]);
    assert_eq!(g.to_rgba8(), vec![0, 0, 0, 255, 200, 200, 200, 255]);
    let p = pal(3, 1, true);
    let pal_entries = &p.palette.as_ref().unwrap().entries;
    let expect: Vec<u8> = p
        .data()
        .iter()
        .flat_map(|&i| pal_entries[usize::from(i)])
        .collect();
    assert_eq!(p.to_rgba8(), expect);
}

#[test]
fn sixteen_bit_truecolour_expands_as_documented() {
    // 2 x 1, 16 bpp, type 2, top-down, 1 attribute bit.
    let mut f = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 16, 0x21];
    // pixel 0: A=1 R=31 G=0 B=16 ; pixel 1: A=0 R=1 G=31 B=0
    let p0: u16 = 0x8000 | (31 << 10) | 16;
    let p1: u16 = (1 << 10) | (31 << 5);
    f.extend_from_slice(&p0.to_le_bytes());
    f.extend_from_slice(&p1.to_le_bytes());
    let img = decode(&f).unwrap();
    assert_eq!(img.format, PixelFormat::Rgba);
    assert_eq!(img.data(), &[255, 0, 0x84, 255, 0x08, 255, 0, 0]);
    assert_eq!(info(&f).unwrap().depth, 16);
    assert!(info(&f).unwrap().has_alpha);
    // 15-bit: attribute bit reserved, always opaque.
    f[16] = 15;
    f[17] = 0x20;
    let img = decode(&f).unwrap();
    assert_eq!(img.data(), &[255, 0, 0x84, 255, 0x08, 255, 0, 255]);
    assert!(!info(&f).unwrap().has_alpha);
}

#[test]
fn colour_map_origin_is_folded_into_indices() {
    // 4 x 1 palette image whose colour map starts at index 4.
    let mut f = vec![0, 1, 1];
    f.extend_from_slice(&4u16.to_le_bytes());
    f.extend_from_slice(&3u16.to_le_bytes());
    f.push(24);
    f.extend_from_slice(&[0, 0, 0, 0, 4, 0, 1, 0, 8, 0x20]);
    f.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9]); // BGR x3
    f.extend_from_slice(&[4, 5, 6, 6]);
    let img = decode(&f).unwrap();
    assert_eq!(img.format, PixelFormat::Pal8);
    assert_eq!(img.data(), &[0, 1, 2, 2]);
    let pal = img.palette.as_ref().unwrap();
    assert_eq!(
        pal.entries,
        vec![[3, 2, 1, 255], [6, 5, 4, 255], [9, 8, 7, 255]]
    );
    assert_eq!(&img.to_rgb8()[..3], &[3, 2, 1]);
    // An index below the origin, or past the map, is InvalidData.
    let n = f.len();
    f[n - 1] = 3;
    assert!(matches!(decode(&f), Err(TgaError::InvalidData(_))));
    f[n - 1] = 7;
    assert!(matches!(decode(&f), Err(TgaError::InvalidData(_))));
}

#[test]
fn bottom_up_and_right_to_left_files_normalise() {
    let img = rgb(5, 3);
    let bu = encode(
        &img,
        &EncodeOptions::default().with_row_order(RowOrder::BottomUp),
    )
    .unwrap();
    assert_eq!(bu[17] & 0x20, 0, "descriptor bit 5 clear");
    assert!(!info(&bu).unwrap().top_down);
    assert_eq!(
        decode(&bu).unwrap(),
        decode(&encode(&img, &EncodeOptions::default()).unwrap()).unwrap()
    );
    // Mirror columns by hand (bit 4) on an uncompressed file.
    let mut rtl = encode(&img, &EncodeOptions::default().with_rle(false)).unwrap();
    rtl[17] |= 0x10;
    let m = decode(&rtl).unwrap();
    for y in 0..3 {
        for x in 0..5 {
            assert_eq!(
                &m.data()[(y * 5 + x) * 3..][..3],
                &img.data()[(y * 5 + (4 - x)) * 3..][..3]
            );
        }
    }
}

#[test]
fn decode_from_reads_a_stream() {
    let f = encode(&gray(3, 3), &EncodeOptions::default()).unwrap();
    let img = decode_from(Cursor::new(f.clone())).unwrap();
    assert_eq!(img, decode(&f).unwrap());
}

// ---- DecodeOptions ---------------------------------------------------------

#[test]
fn limits_fail_closed_before_allocation() {
    // 60000 x 60000 x 4 declared, no pixel data at all: the default
    // 1 GiB cap rejects it as LimitExceeded, not as truncated data.
    let mut f = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    f.extend_from_slice(&60000u16.to_le_bytes());
    f.extend_from_slice(&60000u16.to_le_bytes());
    f.extend_from_slice(&[32, 0x28]);
    assert!(matches!(decode(&f), Err(TgaError::LimitExceeded(_))));
    assert!(matches!(
        decode_with(&f, &DecodeOptions::default().unlimited()),
        Err(TgaError::InvalidData(_))
    ));
    // info / header / probe do not care about size.
    assert!(info(&f).is_ok());
    assert!(probe(&f));

    let small = encode(&rgba(8, 8), &EncodeOptions::default()).unwrap();
    assert!(decode_with(&small, &DecodeOptions::default().with_max_width(7u32)).is_err());
    assert!(decode_with(&small, &DecodeOptions::default().with_max_height(7u32)).is_err());
    assert!(decode_with(&small, &DecodeOptions::default().with_max_pixels(63u64)).is_err());
    assert!(decode_with(&small, &DecodeOptions::default().with_max_bytes(255u64)).is_err());
    assert!(decode_with(&small, &DecodeOptions::default().with_max_bytes(256u64)).is_ok());
    let err = decode_with(&small, &DecodeOptions::default().with_max_pixels(1u64)).unwrap_err();
    assert!(err.to_string().starts_with("limit exceeded:"), "{err}");
    assert_eq!(DecodeOptions::default().max_bytes, Some(1 << 30));
    assert!(!DecodeOptions::default().strict);
}

#[test]
fn strict_mode_rejects_row_spanning_rle_and_interleaving() {
    // 2 x 2 gray RLE: one run packet of 4 pixels crosses the row boundary.
    let mut f = vec![0, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 8, 0x20];
    f.extend_from_slice(&[0x83, 7]);
    assert_eq!(decode(&f).unwrap().data(), &[7, 7, 7, 7]);
    assert!(matches!(
        decode_with(&f, &DecodeOptions::default().with_strict(true)),
        Err(TgaError::InvalidData(_))
    ));
    // Interleaving flag set.
    let mut g = encode(&gray(2, 2), &EncodeOptions::default()).unwrap();
    g[17] |= 0x40;
    assert!(decode(&g).is_ok());
    assert!(decode_with(&g, &DecodeOptions::default().with_strict(true)).is_err());
}

// ---- encode: layouts as given, options as fields --------------------------

#[test]
fn lossless_round_trip_every_layout_every_axis() {
    for img in [
        rgba(9, 5),
        rgb(9, 5),
        gray(9, 5),
        pal(9, 5, true),
        pal(9, 5, false),
    ] {
        for rle in [false, true] {
            for order in [RowOrder::TopDown, RowOrder::BottomUp] {
                let opts = EncodeOptions::default().with_rle(rle).with_row_order(order);
                let f = encode(&img, &opts).unwrap();
                let back = decode(&f).unwrap();
                assert_eq!(back, img, "rle={rle} order={order:?} {:?}", img.format);
                // encode_to streams the same bytes
                let mut buf = Vec::new();
                encode_to(&img, &opts, &mut buf).unwrap();
                assert_eq!(buf, f);
            }
        }
    }
}

#[test]
fn rgba_is_written_at_32_bits_even_when_opaque() {
    let mut img = rgba(4, 2);
    for p in img.data_mut().chunks_exact_mut(4) {
        p[3] = 0xFF;
    }
    let f = encode(&img, &EncodeOptions::default()).unwrap();
    assert_eq!(f[16], 32);
    assert_eq!(f[17] & 0x0F, 8);
    assert_eq!(decode(&f).unwrap(), img);
    // The pre-contract auto-depth rule is an explicit field.
    let f24 = encode(&img, &EncodeOptions::default().with_drop_opaque_alpha(true)).unwrap();
    assert_eq!(f24[16], 24);
    let back = decode(&f24).unwrap();
    assert_eq!(back.format, PixelFormat::Rgb24);
    assert_eq!(back.to_rgba8(), img.to_rgba8());
    // alpha_bits override keeps 32 bits but declares none.
    let f0 = encode(&img, &EncodeOptions::default().with_alpha_bits(0u8)).unwrap();
    assert_eq!((f0[16], f0[17] & 0x0F), (32, 0));
    assert_eq!(info(&f0).unwrap().attribute_bits, 0);
}

#[test]
fn encode_rgb8_and_rgba8_match_encode_of_from_constructors() {
    let w = 6;
    let h = 2;
    let rgb_px = seeded(w * h * 3, 9);
    let rgba_px = seeded(w * h * 4, 10);
    let opts = EncodeOptions::default().with_rle(false);
    assert_eq!(
        encode_rgb8(w as u32, h as u32, &rgb_px, &opts).unwrap(),
        encode(
            &TgaImage::from_rgb8(w as u32, h as u32, rgb_px.clone()),
            &opts
        )
        .unwrap()
    );
    assert_eq!(
        encode_rgba8(w as u32, h as u32, &rgba_px, &opts).unwrap(),
        encode(
            &TgaImage::from_rgba8(w as u32, h as u32, rgba_px.clone()),
            &opts
        )
        .unwrap()
    );
    assert!(matches!(
        encode_rgb8(w as u32, h as u32, &rgb_px[..5], &opts),
        Err(TgaError::InvalidData(_))
    ));
    assert!(matches!(
        encode(&TgaImage::from_rgba8(3, 3, vec![0; 4]), &opts),
        Err(TgaError::InvalidData(_))
    ));
}

#[test]
fn palette_entry_size_field_and_unsupported_inputs() {
    let img = pal(6, 2, true);
    let f32b = encode(&img, &EncodeOptions::default()).unwrap();
    assert_eq!(f32b[7], 32, "alpha palette -> 32-bit entries");
    let opaque = pal(6, 2, false);
    let f24 = encode(&opaque, &EncodeOptions::default()).unwrap();
    assert_eq!(f24[7], 24, "opaque palette -> 24-bit entries");
    let f16 = encode(
        &img,
        &EncodeOptions::default().with_palette_entry_size(ColorMapEntrySize::Bits16),
    )
    .unwrap();
    assert_eq!(f16[7], 16);
    let back = decode(&f16).unwrap();
    assert_eq!(back.format, PixelFormat::Pal8);
    assert_eq!(back.data(), img.data(), "indices survive; colours quantise");
    // Pal8 without a palette, or > 256 colours, cannot be represented.
    let no_pal = TgaImage::packed(2, 1, PixelFormat::Pal8, vec![0, 1]);
    assert!(matches!(
        encode(&no_pal, &EncodeOptions::default()),
        Err(TgaError::InvalidData(_))
    ));
    let many: Vec<u8> = (0..300u32)
        .flat_map(|i| [(i % 256) as u8, (i / 256) as u8, 0, 255])
        .collect();
    assert!(matches!(
        TgaImage::from_rgba8(300, 1, many).to_indexed(),
        Err(TgaError::Unsupported(_))
    ));
    assert!(matches!(
        encode(
            &TgaImage::from_gray8(70000, 1, vec![0; 70000]),
            &EncodeOptions::default()
        ),
        Err(TgaError::Unsupported(_))
    ));
}

#[test]
fn image_id_screen_origin_and_footer_fields() {
    let img = gray(3, 2);
    let opts = EncodeOptions::default()
        .with_image_id(b"hello".to_vec())
        .with_screen_origin(ImageOrigin::new(11, 22))
        .with_footer(true);
    let f = encode(&img, &opts).unwrap();
    assert_eq!(f[0], 5);
    assert_eq!(&f[18..23], b"hello");
    let h = header(&f).unwrap();
    assert_eq!(h.image_origin(), ImageOrigin::new(11, 22));
    assert_eq!(oxideav_tga::parse_tga_image_id(&f), Some(&b"hello"[..]));
    let i = info(&f).unwrap();
    assert!(i.has_footer && !i.has_extension_area);
    assert_eq!(i.image_id_length, 5);
    assert_eq!(decode(&f).unwrap(), img);
    assert!(matches!(
        encode(
            &img,
            &EncodeOptions::default().with_image_id(vec![0u8; 256])
        ),
        Err(TgaError::InvalidData(_))
    ));
}

#[test]
fn metadata_gamma_and_extension_round_trip() {
    let mut img = rgba(4, 3);
    img.metadata = Metadata::new().with_gamma(2.2);
    let f = encode(&img, &EncodeOptions::default()).unwrap();
    let back = decode(&f).unwrap();
    assert_eq!(back.metadata.gamma, Some(2.2));
    let ext = back.extension.clone().expect("extension area");
    assert_eq!(ext.gamma, (220, 100));
    assert_eq!(back.data(), img.data());
    // The extension record re-emits with its other fields intact.
    let mut edited = back.clone();
    edited.extension.as_mut().unwrap().author_name = "contract".into();
    let f2 = encode(&edited, &EncodeOptions::default()).unwrap();
    let back2 = decode(&f2).unwrap();
    assert_eq!(back2.extension.as_ref().unwrap().author_name, "contract");
    assert_eq!(back2.metadata.gamma, Some(2.2));
    // An explicit ExtensionAreaInput wins, but an unset gamma is filled in.
    let input = oxideav_tga::ExtensionAreaInput {
        software_id: "oxideav".into(),
        ..Default::default()
    };
    let f3 = encode(&img, &EncodeOptions::default().with_extension(input)).unwrap();
    let back3 = decode(&f3).unwrap();
    assert_eq!(back3.extension.as_ref().unwrap().software_id, "oxideav");
    assert_eq!(back3.metadata.gamma, Some(2.2));
    // Without gamma and without a record, no footer at all.
    let plain = encode(&rgba(4, 3), &EncodeOptions::default()).unwrap();
    assert!(!info(&plain).unwrap().has_footer);
    assert_eq!(decode(&plain).unwrap().metadata, Metadata::default());
}

// ---- TgaImage constructors ----------------------------------------------------

#[test]
fn new_validates_and_from_constructors_are_infallible() {
    assert!(TgaImage::new(2, 2, PixelFormat::Gray8, vec![Plane::new(2, vec![0; 3])]).is_err());
    assert!(TgaImage::new(2, 2, PixelFormat::Gray8, vec![Plane::new(2, vec![0; 4])]).is_ok());
    assert!(TgaImage::new(2, 2, PixelFormat::Gray8, vec![]).is_err());
    let short = TgaImage::from_rgb8(2, 2, vec![1, 2, 3]);
    assert_eq!(short.width(), 2);
    assert_eq!(short.height(), 2);
    assert_eq!(short.format(), PixelFormat::Rgb24);
    assert!(short.validate().is_err());
    assert!(short.try_to_rgb8().is_err());
    assert_eq!(short.to_rgba8().len(), 16, "padded, never panics");
    assert!(TgaImage::new_indexed(2, 1, vec![0, 5], Palette::new(vec![[0; 4]])).is_err());
    let built = TgaImage::new(
        1,
        1,
        PixelFormat::Rgba,
        vec![Plane::new(4, vec![1, 2, 3, 4])],
    )
    .unwrap()
    .with_color(ColorInfo::srgb())
    .with_metadata(Metadata::new().with_gamma(1.0));
    assert!(built.color.is_specified());
    assert!(built.has_alpha());
    assert!(!rgb(1, 1).has_alpha());
    assert!(pal(1, 1, true).has_alpha());
    assert!(!pal(1, 1, false).has_alpha());
}

// ---- Error shape --------------------------------------------------------------

#[test]
fn error_shape() {
    let e: Error = std::io::Error::other("boom").into();
    assert!(matches!(e, TgaError::Io(_)));
    assert!(std::error::Error::source(&e).is_some());
    assert!(e.to_string().starts_with("io: "));
    struct Failing;
    impl std::io::Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("nope"))
        }
    }
    assert!(matches!(decode_from(Failing), Err(TgaError::Io(_))));
    struct Full;
    impl std::io::Write for Full {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(matches!(
        encode_to(&gray(1, 1), &EncodeOptions::default(), Full),
        Err(TgaError::Io(_))
    ));
    assert!(matches!(decode(&[]), Err(TgaError::InvalidData(_))));
    assert!(matches!(
        decode(&[0, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 8, 0]),
        Err(TgaError::Unsupported(_))
    ));
    assert_eq!(std::mem::size_of::<TgaPixelFormat>(), 1);
}

// ---- Pre-contract wrappers stay byte-identical ---------------------------------

#[test]
#[allow(deprecated)]
fn deprecated_writers_match_the_contract_paths() {
    let img = rgba(11, 4);
    let legacy = EncodeOptions::default().with_drop_opaque_alpha(true);
    assert_eq!(
        oxideav_tga::encode_tga_rle(11, 4, img.data()).unwrap(),
        encode(&img, &legacy).unwrap()
    );
    assert_eq!(
        oxideav_tga::encode_tga_uncompressed(11, 4, img.data()).unwrap(),
        encode(&img, &legacy.clone().with_rle(false)).unwrap()
    );
    let g = gray(11, 4);
    assert_eq!(
        oxideav_tga::encode_tga_grayscale_rle(11, 4, g.data()).unwrap(),
        encode(&g, &EncodeOptions::default()).unwrap()
    );
    let p = pal(11, 4, true);
    let via_rgba = oxideav_tga::encode_tga_palette_rle(11, 4, &p.to_rgba8()).unwrap();
    let back = decode(&via_rgba).unwrap();
    assert_eq!(back.to_rgba8(), p.to_rgba8());
    // parse_tga is decode + the legacy expansion.
    let f = encode(&p, &EncodeOptions::default()).unwrap();
    let old = oxideav_tga::parse_tga(&f).unwrap();
    assert_eq!(old.format, PixelFormat::Rgba);
    assert_eq!(old.data(), p.to_rgba8());
    assert_eq!(old, decode(&f).unwrap().into_legacy_layout());
    let fg = encode(&g, &EncodeOptions::default()).unwrap();
    assert_eq!(
        oxideav_tga::parse_tga(&fg).unwrap().format,
        PixelFormat::Gray8
    );
    assert_eq!(
        oxideav_tga::parse_header(&fg).map(|h| h.width),
        Some(header(&fg).unwrap().width)
    );
}
