#![no_main]

//! Bit-exact `decode(encode(img)) == img` identity target for the
//! contract API (`IMAGE_CRATE_API`), across every native layout and
//! every `EncodeOptions` axis that is lossless:
//!
//! * layouts `Rgba` / `Rgb24` / `Gray8` / `Pal8` (palette from the
//!   fuzz bytes, 24- or 32-bit entries — the lossless colour-map
//!   sizes; 15 / 16-bit entries quantise and are excluded),
//! * `rle` on / off, `row_order` top-down / bottom-up, `alpha_bits`
//!   `8` / `0`, an Image ID of 0..=255 bytes, a non-zero screen
//!   origin, a forced footer, and a `metadata.gamma` that must come
//!   back through the extension area.
//!
//! The decoded image must equal the input in `format`, plane bytes,
//! palette and `metadata.gamma` (to the 1/100 the §C.6.6 SHORT pair
//! carries); `info` must agree with the decoded geometry and `probe`
//! must accept the encoder's output. A decode failure on an accepted
//! encode is an encoder/decoder disagreement and trips the `expect`.

use libfuzzer_sys::fuzz_target;
use oxideav_tga::{
    decode, encode, info, probe, EncodeOptions, ImageOrigin, Metadata, Palette, PixelFormat,
    RowOrder, TgaImage,
};

const MAX_PIXELS: usize = 64 * 1024;
const MAX_SIDE: u16 = 512;

fuzz_target!(|data: &[u8]| {
    if data.len() < 12 {
        return;
    }
    let format = match data[0] % 4 {
        0 => PixelFormat::Rgba,
        1 => PixelFormat::Rgb24,
        2 => PixelFormat::Gray8,
        _ => PixelFormat::Pal8,
    };
    let w = (u16::from_le_bytes([data[1], data[2]]) % MAX_SIDE).max(1);
    let h = (u16::from_le_bytes([data[3], data[4]]) % MAX_SIDE).max(1);
    let flags = data[5];
    let id_len = usize::from(data[6]);
    let origin = ImageOrigin::new(
        u16::from_le_bytes([data[7], data[8]]),
        u16::from_le_bytes([data[9], data[10]]),
    );
    let palette_len = usize::from(data[11]).max(1);
    let n = w as usize * h as usize;
    if n > MAX_PIXELS {
        return;
    }
    let tail = &data[12..];
    let take = |len: usize, skip: usize| -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let src = tail.get(skip..).unwrap_or(&[]);
        if src.is_empty() {
            out.resize(len, 0);
            return out;
        }
        while out.len() < len {
            let k = (len - out.len()).min(src.len());
            out.extend_from_slice(&src[..k]);
        }
        out
    };

    let mut img = match format {
        PixelFormat::Pal8 => {
            let entries: Vec<[u8; 4]> = take(palette_len * 4, 0)
                .chunks_exact(4)
                .map(|e| {
                    [
                        e[0],
                        e[1],
                        e[2],
                        if flags & 0x40 != 0 { e[3] } else { 0xFF },
                    ]
                })
                .collect();
            let indices: Vec<u8> = take(n, palette_len * 4)
                .into_iter()
                .map(|i| i % palette_len as u8)
                .collect();
            TgaImage::new_indexed(w as u32, h as u32, indices, Palette::new(entries))
                .expect("valid indexed image")
        }
        other => TgaImage::packed(
            w as u32,
            h as u32,
            other,
            take(n * other.bytes_per_pixel(), 0),
        ),
    };
    let gamma = if flags & 0x20 != 0 {
        Some(f32::from(data[5] % 50 + 1) / 10.0)
    } else {
        None
    };
    img.metadata = Metadata::new().with_gamma(gamma);

    let opts = EncodeOptions::default()
        .with_rle(flags & 1 != 0)
        .with_row_order(if flags & 2 != 0 {
            RowOrder::BottomUp
        } else {
            RowOrder::TopDown
        })
        .with_alpha_bits(if flags & 4 != 0 { Some(0u8) } else { None })
        .with_image_id(take(id_len, n * 4))
        .with_screen_origin(origin)
        .with_footer(flags & 8 != 0);

    let bytes = encode(&img, &opts).expect("every generated image is encodable");
    assert!(probe(&bytes), "probe rejects the encoder's own output");
    let described = info(&bytes).expect("info on encoder output");
    let back = decode(&bytes).expect("encoder produced a file decode rejects");

    assert_eq!(back.format, img.format, "format");
    assert_eq!(
        (back.width, back.height),
        (img.width, img.height),
        "geometry"
    );
    assert_eq!(
        (described.width, described.height, described.format),
        (back.width, back.height, back.format),
        "info vs decode"
    );
    assert_eq!(back.data(), img.data(), "plane bytes");
    assert_eq!(back.palette, img.palette, "palette");
    match gamma {
        Some(g) => {
            let got = back
                .metadata
                .gamma
                .expect("gamma round-trips through the extension area");
            assert!((got - g).abs() < 0.006, "gamma {g} came back as {got}");
            assert!(back.extension.is_some());
        }
        None => assert_eq!(back.metadata.gamma, None),
    }
    assert_eq!(
        described.has_footer,
        gamma.is_some() || flags & 8 != 0,
        "footer presence"
    );
    assert_eq!(
        described.image_id_length as usize, id_len,
        "image id length"
    );
    assert_eq!(
        oxideav_tga::header(&bytes).unwrap().image_origin(),
        origin,
        "screen origin"
    );
});
