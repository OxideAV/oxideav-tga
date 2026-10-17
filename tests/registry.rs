//! The registry path of `oxideav-tga` against the IMAGE_CRATE_API
//! rulings of rounds 475 / 476 (core 0.1.40): the palette-alpha record
//! both ways, the declared accepted layouts with byte-order swizzles
//! and typed refusals, truthful output labels, and the single-picture
//! rule on the encoder and the muxer.

#![cfg(feature = "registry")]

use std::io::Cursor;

use oxideav_core::{
    CodecId, CodecParameters, Frame, Packet, PixelFormat, StreamInfo, TimeBase, VideoFrame,
    VideoPlane,
};
use oxideav_tga::{
    decode, encode, ColorMapEntrySize, EncodeOptions, Palette, TgaImage, TgaPixelFormat,
    ACCEPTED_PIXEL_FORMATS, CODEC_ID_STR,
};

fn id() -> CodecId {
    CodecId::new(CODEC_ID_STR)
}

fn params(w: u32, h: u32, pf: PixelFormat) -> CodecParameters {
    let mut p = CodecParameters::video(id());
    p.width = Some(w);
    p.height = Some(h);
    p.pixel_format = Some(pf);
    p
}

fn decode_registry(bytes: &[u8]) -> VideoFrame {
    let mut dec = oxideav_tga::make_decoder(&CodecParameters::video(id())).unwrap();
    dec.send_packet(&Packet::new(0, TimeBase::new(1, 1), bytes.to_vec()))
        .unwrap();
    match dec.receive_frame().unwrap() {
        Frame::Video(v) => v,
        _ => panic!("non-video frame"),
    }
}

fn encode_registry(frame: &VideoFrame, p: &CodecParameters) -> oxideav_core::Result<Vec<u8>> {
    let mut enc = oxideav_tga::make_encoder(p)?;
    enc.send_frame(&Frame::Video(frame.clone()))?;
    Ok(enc.receive_packet()?.data)
}

fn stream(w: u32, h: u32) -> StreamInfo {
    StreamInfo {
        index: 0,
        params: params(w, h, PixelFormat::Rgb24),
        time_base: TimeBase::new(1, 1),
        start_time: Some(0),
        duration: None,
    }
}

fn translucent_palette() -> Palette {
    Palette::new(vec![
        [10, 20, 30, 0],
        [40, 50, 60, 128],
        [70, 80, 90, 255],
        [1, 2, 3, 7],
    ])
}

fn indexed(pal: Palette) -> TgaImage {
    TgaImage::new_indexed(4, 2, vec![0, 1, 2, 3, 3, 2, 1, 0], pal).unwrap()
}

// ---- palette alpha record: decoder side ----

#[test]
fn colour_map_with_32bit_entries_attaches_the_alpha_record() {
    let img = indexed(translucent_palette());
    let bytes = encode(&img, &EncodeOptions::default()).unwrap();
    assert_eq!(oxideav_tga::info(&bytes).unwrap().color_map_entry_size, 32);
    let v = decode_registry(&bytes);
    assert_eq!(
        v.palette(),
        Some(&[10u8, 20, 30, 40, 50, 60, 70, 80, 90, 1, 2, 3][..])
    );
    assert_eq!(v.palette_alpha(), Some(&[0u8, 128, 255, 7][..]));
    assert_eq!(v.palette_rgba(0), Some([10, 20, 30, 0]));
    assert_eq!(v.palette_rgba(1), Some([40, 50, 60, 128]));
    assert_eq!(v.palette_rgba(3), Some([1, 2, 3, 7]));
    assert_eq!(v.planes[0].data, img.data());
}

#[test]
fn opaque_32bit_colour_map_still_declares_its_attribute_channel() {
    let pal = Palette::new(vec![[1, 2, 3, 255], [4, 5, 6, 255]]);
    let img = TgaImage::new_indexed(2, 1, vec![0, 1], pal).unwrap();
    let bytes = encode(
        &img,
        &EncodeOptions::default().with_palette_entry_size(ColorMapEntrySize::Bits32),
    )
    .unwrap();
    let v = decode_registry(&bytes);
    assert_eq!(
        v.palette_alpha(),
        Some(&[255u8, 255][..]),
        "a 32-bit map carries an attribute byte: the record says so even when opaque"
    );
    // ... and the encoder writes the same entry width back.
    let out = encode_registry(&v, &params(2, 1, PixelFormat::Pal8)).unwrap();
    assert_eq!(oxideav_tga::info(&out).unwrap().color_map_entry_size, 32);
}

#[test]
fn a_24bit_colour_map_attaches_no_alpha_record() {
    let pal = Palette::new(vec![[1, 2, 3, 255], [4, 5, 6, 255]]);
    let img = TgaImage::new_indexed(2, 1, vec![0, 1], pal).unwrap();
    let bytes = encode(&img, &EncodeOptions::default()).unwrap();
    assert_eq!(oxideav_tga::info(&bytes).unwrap().color_map_entry_size, 24);
    let v = decode_registry(&bytes);
    assert_eq!(v.palette(), Some(&[1u8, 2, 3, 4, 5, 6][..]));
    assert!(v.palette_alpha().is_none(), "no record means opaque");
    assert_eq!(v.palette_rgba(1), Some([4, 5, 6, 255]));
    let out = encode_registry(&v, &params(2, 1, PixelFormat::Pal8)).unwrap();
    assert_eq!(oxideav_tga::info(&out).unwrap().color_map_entry_size, 24);
}

#[test]
fn a_16bit_colour_map_attribute_bit_becomes_the_record() {
    // 5-bit-exact channels so the 16-bit map is lossless on colour.
    let pal = Palette::new(vec![[0xF8, 0x00, 0x08, 0], [0x10, 0xF8, 0x80, 255]]);
    let img = TgaImage::new_indexed(2, 1, vec![0, 1], pal).unwrap();
    let bytes = encode(
        &img,
        &EncodeOptions::default().with_palette_entry_size(ColorMapEntrySize::Bits16),
    )
    .unwrap();
    assert_eq!(oxideav_tga::info(&bytes).unwrap().color_map_entry_size, 16);
    let v = decode_registry(&bytes);
    assert_eq!(v.palette_alpha(), Some(&[0u8, 255][..]));
    // 0xF8 expands back to 0xFF (top bits repeated), 0x08 to 0x08, 0x80 to 0x84.
    assert_eq!(v.palette_rgba(0), Some([0xFF, 0x00, 0x08, 0]));
    assert_eq!(v.palette_rgba(1), Some([0x10, 0xFF, 0x84, 255]));
    // A 15-bit map has no attribute bit: no record.
    let bytes15 = encode(
        &img,
        &EncodeOptions::default().with_palette_entry_size(ColorMapEntrySize::Bits15),
    )
    .unwrap();
    assert!(decode_registry(&bytes15).palette_alpha().is_none());
}

// ---- palette alpha record: encoder side ----

#[test]
fn encoder_writes_a_32bit_colour_map_from_the_record() {
    let mut frame = VideoFrame {
        pts: None,
        planes: vec![VideoPlane {
            stride: 4,
            data: vec![0, 1, 2, 3, 3, 2, 1, 0],
        }],
    };
    frame.set_palette_rgba(&translucent_palette().entries);
    let bytes = encode_registry(&frame, &params(4, 2, PixelFormat::Pal8)).unwrap();
    let info = oxideav_tga::info(&bytes).unwrap();
    assert_eq!(info.color_map_entry_size, 32);
    assert_eq!(
        info.attribute_bits, 0,
        "index pixels declare no attribute bits"
    );
    let back = decode(&bytes).unwrap();
    assert_eq!(back.format, TgaPixelFormat::Pal8);
    assert_eq!(back.palette, Some(translucent_palette()));
    assert_eq!(back.data(), &[0, 1, 2, 3, 3, 2, 1, 0]);
}

#[test]
fn a_short_alpha_record_leaves_uncovered_entries_opaque() {
    let mut frame = VideoFrame {
        pts: None,
        planes: vec![VideoPlane {
            stride: 3,
            data: vec![0, 1, 2],
        }],
    };
    frame.set_palette(vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
    frame.set_palette_alpha(vec![0]); // only entry 0 is covered
    let bytes = encode_registry(&frame, &params(3, 1, PixelFormat::Pal8)).unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(
        back.palette.unwrap().entries,
        vec![[1, 2, 3, 0], [4, 5, 6, 255], [7, 8, 9, 255]]
    );
}

#[test]
fn registry_round_trip_keeps_the_record_exactly() {
    let img = indexed(translucent_palette());
    let bytes = encode(&img, &EncodeOptions::default()).unwrap();
    let v = decode_registry(&bytes);
    let out = encode_registry(&v, &params(4, 2, PixelFormat::Pal8)).unwrap();
    let v2 = decode_registry(&out);
    assert_eq!(v2.planes[0].data, v.planes[0].data);
    assert_eq!(v2.palette(), v.palette());
    assert_eq!(v2.palette_alpha(), v.palette_alpha());
    assert_eq!(out, bytes, "same picture, same record: same bytes");
}

// ---- declared layouts ----

#[test]
fn the_registry_declares_exactly_the_accepted_layouts() {
    let mut ctx = oxideav_core::RuntimeContext::new();
    oxideav_tga::register(&mut ctx);
    let impls = ctx.codecs.implementations(&id());
    assert_eq!(impls.len(), 1);
    let caps = &impls[0].caps;
    assert_eq!(caps.accepted_pixel_formats, ACCEPTED_PIXEL_FORMATS.to_vec());
    assert!(caps.lossless && !caps.lossy && caps.intra_only);
    assert_eq!(ACCEPTED_PIXEL_FORMATS.len(), 8);
    for pf in ACCEPTED_PIXEL_FORMATS {
        assert!(
            oxideav_tga::make_encoder(&params(2, 1, pf)).is_ok(),
            "{pf:?} declared but refused"
        );
    }
}

#[test]
fn every_declared_true_colour_order_reaches_the_same_pixels() {
    // One 2×1 picture: pixel 0 = (R 1, G 2, B 3, A 4), pixel 1 = (5, 6, 7, 8).
    let cases: [(PixelFormat, Vec<u8>); 6] = [
        (PixelFormat::Rgba, vec![1, 2, 3, 4, 5, 6, 7, 8]),
        (PixelFormat::Bgra, vec![3, 2, 1, 4, 7, 6, 5, 8]),
        (PixelFormat::Argb, vec![4, 1, 2, 3, 8, 5, 6, 7]),
        (PixelFormat::Abgr, vec![4, 3, 2, 1, 8, 7, 6, 5]),
        (PixelFormat::Rgb24, vec![1, 2, 3, 5, 6, 7]),
        (PixelFormat::Bgr24, vec![3, 2, 1, 7, 6, 5]),
    ];
    for (pf, data) in cases {
        let stride = data.len(); // one row of two pixels
        let bpp = stride / 2;
        let frame = VideoFrame {
            pts: None,
            planes: vec![VideoPlane { stride, data }],
        };
        let p = params(2, 1, pf);
        let enc = oxideav_tga::make_encoder(&p).unwrap();
        let expect_label = if bpp == 4 {
            PixelFormat::Rgba
        } else {
            PixelFormat::Rgb24
        };
        assert_eq!(
            enc.output_params().pixel_format,
            Some(expect_label),
            "{pf:?}: the stream label is what the file decodes to"
        );
        let bytes = encode_registry(&frame, &p).unwrap();
        let back = decode(&bytes).unwrap();
        if bpp == 4 {
            assert_eq!(back.format, TgaPixelFormat::Rgba, "{pf:?}");
            assert_eq!(back.data(), &[1, 2, 3, 4, 5, 6, 7, 8], "{pf:?}");
        } else {
            assert_eq!(back.format, TgaPixelFormat::Rgb24, "{pf:?}");
            assert_eq!(back.data(), &[1, 2, 3, 5, 6, 7], "{pf:?}");
        }
        // The decoder's label is truthful: it emits what the demuxer declares.
        let v = decode_registry(&bytes);
        assert_eq!(v.planes[0].data, back.data());
        let ctx = oxideav_core::RuntimeContext::new();
        let dmx = oxideav_tga::container::open_demuxer(Box::new(Cursor::new(bytes)), &ctx.codecs)
            .unwrap();
        assert_eq!(dmx.streams()[0].params.pixel_format, Some(expect_label));
    }
}

#[test]
fn swizzled_frames_honour_row_padding() {
    // Bgr24 with a 5-byte stride for a 1-pixel row (2 bytes of padding).
    let frame = VideoFrame {
        pts: None,
        planes: vec![VideoPlane {
            stride: 5,
            data: vec![3, 2, 1, 99, 99, 6, 5, 4, 99, 99],
        }],
    };
    let bytes = encode_registry(&frame, &params(1, 2, PixelFormat::Bgr24)).unwrap();
    assert_eq!(decode(&bytes).unwrap().data(), &[1, 2, 3, 4, 5, 6]);
}

#[test]
fn undeclared_layouts_are_typed_refusals_not_conversions() {
    for pf in [
        PixelFormat::Yuv420P,
        PixelFormat::Yuv444P,
        PixelFormat::Ya8,
        PixelFormat::Gray16Le,
        PixelFormat::Rgb48Le,
        PixelFormat::Rgba64Le,
        PixelFormat::Nv12,
    ] {
        match oxideav_tga::make_encoder(&params(2, 1, pf)) {
            Err(oxideav_core::Error::Unsupported(msg)) => {
                assert!(msg.contains("no TGA carriage"), "{pf:?}: {msg}");
                assert!(
                    msg.contains("Bgra"),
                    "{pf:?}: the refusal names the accepted set"
                );
            }
            Err(other) => panic!("{pf:?}: expected Unsupported, got {other:?}"),
            Ok(_) => panic!("{pf:?}: accepted an undeclared layout"),
        }
    }
    // The frame bridge refuses the same way.
    let frame = VideoFrame {
        pts: None,
        planes: vec![VideoPlane {
            stride: 2,
            data: vec![0; 4],
        }],
    };
    assert!(matches!(
        TgaImage::from_video_frame(&frame, &params(2, 1, PixelFormat::Ya8)),
        Err(oxideav_tga::TgaError::Unsupported(_))
    ));
}

#[test]
fn a_short_frame_is_invalid_data_not_a_panic() {
    let frame = VideoFrame {
        pts: None,
        planes: vec![VideoPlane {
            stride: 8,
            data: vec![0; 8],
        }],
    };
    for pf in [PixelFormat::Bgra, PixelFormat::Rgba] {
        let mut enc = oxideav_tga::make_encoder(&params(2, 2, pf)).unwrap();
        assert!(matches!(
            enc.send_frame(&Frame::Video(frame.clone())),
            Err(oxideav_core::Error::InvalidData(_))
        ));
    }
}

// ---- single picture ----

#[test]
fn the_encoder_refuses_a_second_frame() {
    let frame = VideoFrame {
        pts: None,
        planes: vec![VideoPlane {
            stride: 2,
            data: vec![1, 2],
        }],
    };
    let mut enc = oxideav_tga::make_encoder(&params(2, 1, PixelFormat::Gray8)).unwrap();
    enc.send_frame(&Frame::Video(frame.clone())).unwrap();
    let pkt = enc.receive_packet().unwrap();
    assert!(pkt.flags.keyframe);
    match enc.send_frame(&Frame::Video(frame)) {
        Err(oxideav_core::Error::Unsupported(msg)) => assert!(msg.contains("one picture")),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn the_muxer_refuses_mixed_geometry_and_a_second_packet() {
    let out: Box<dyn oxideav_core::WriteSeek> = Box::new(Cursor::new(Vec::new()));
    match oxideav_tga::container::open_muxer(out, &[stream(4, 4), stream(8, 2)]) {
        Err(oxideav_core::Error::Unsupported(msg)) => {
            assert!(msg.contains("4x4") && msg.contains("8x2"), "{msg}");
        }
        Err(other) => panic!("expected Unsupported, got {other:?}"),
        Ok(_) => panic!("accepted two streams"),
    }
    let out: Box<dyn oxideav_core::WriteSeek> = Box::new(Cursor::new(Vec::new()));
    let mut mux = oxideav_tga::container::open_muxer(out, &[stream(2, 1)]).unwrap();
    mux.write_header().unwrap();
    let bytes = encode(
        &TgaImage::from_rgb8(2, 1, vec![0; 6]).unwrap(),
        &EncodeOptions::default(),
    )
    .unwrap();
    let pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
    mux.write_packet(&pkt).unwrap();
    assert!(matches!(
        mux.write_packet(&pkt),
        Err(oxideav_core::Error::Unsupported(_))
    ));
    mux.write_trailer().unwrap();
}

// ---- the frame bridge on its own ----

#[test]
fn from_tga_image_attaches_the_record_only_for_translucent_palettes() {
    let opaque = TgaImage::new_indexed(1, 1, vec![0], Palette::new(vec![[1, 2, 3, 255]])).unwrap();
    let v: VideoFrame = opaque.into();
    assert!(v.palette_alpha().is_none());
    let v2: VideoFrame = indexed(translucent_palette()).into();
    assert_eq!(v2.palette_alpha(), Some(&[0u8, 128, 255, 7][..]));
    let forced = oxideav_tga::image_into_video_frame_with(
        TgaImage::new_indexed(1, 1, vec![0], Palette::new(vec![[1, 2, 3, 255]])).unwrap(),
        Some(3),
        true,
    );
    assert_eq!(forced.palette_alpha(), Some(&[255u8][..]));
    assert_eq!(forced.pts, Some(3));
}
