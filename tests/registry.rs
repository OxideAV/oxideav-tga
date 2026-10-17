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

// ---- metadata: extension area + developer area as blobs, text fields as pairs ----

mod metadata {
    use super::*;
    use oxideav_core::{BlobKind, MetadataBlob};
    use oxideav_tga::{
        encode_tga_with_extension, extension_area_bytes, parse_tga_developer_area,
        parse_tga_extension_area, DeveloperTagInput, ExtensionAreaInput, RowOrder,
        TgaDeveloperArea, TgaTimestamp, BLOB_KIND_DEVELOPER_AREA, BLOB_KIND_EXTENSION_AREA,
    };

    fn ext_kind() -> BlobKind {
        BlobKind::from_static(BLOB_KIND_EXTENSION_AREA)
    }
    fn dev_kind() -> BlobKind {
        BlobKind::from_static(BLOB_KIND_DEVELOPER_AREA)
    }

    fn picture() -> TgaImage {
        TgaImage::from_rgb8(3, 2, (0..18).collect()).unwrap()
    }

    fn rich() -> ExtensionAreaInput {
        ExtensionAreaInput {
            author_name: "Ada".to_string(),
            author_comment: [
                "one".to_string(),
                "two".to_string(),
                String::new(),
                String::new(),
            ],
            timestamp: TgaTimestamp {
                month: 10,
                day: 17,
                year: 2026,
                hour: 9,
                minute: 30,
                second: 5,
            },
            job_name: "job".to_string(),
            job_time: (0, 5, 0),
            software_id: "oxideav".to_string(),
            software_version: (10, 'a'),
            key_color: [255, 1, 2, 3],
            pixel_aspect_ratio: (1, 1),
            gamma: (220, 100),
            attributes_type: 3,
            developer_tags: vec![
                DeveloperTagInput {
                    tag_id: 1,
                    payload: b"payload-one".to_vec(),
                },
                DeveloperTagInput {
                    tag_id: 2,
                    payload: Vec::new(),
                },
            ],
            ..ExtensionAreaInput::default()
        }
    }

    fn rich_file(order: RowOrder) -> Vec<u8> {
        let base = encode(
            &picture(),
            &EncodeOptions::default()
                .with_rle(false)
                .with_row_order(order),
        )
        .unwrap();
        encode_tga_with_extension(&base, &rich()).unwrap()
    }

    fn open(bytes: &[u8]) -> Box<dyn oxideav_core::Demuxer> {
        let ctx = oxideav_core::RuntimeContext::new();
        oxideav_tga::container::open_demuxer(Box::new(Cursor::new(bytes.to_vec())), &ctx.codecs)
            .unwrap()
    }

    #[test]
    fn the_demuxer_pushes_both_blobs_and_the_text_pairs() {
        let file = rich_file(RowOrder::TopDown);
        let dmx = open(&file);
        let p = &dmx.streams()[0].params;
        assert_eq!(p.blobs.len(), 2);
        let ext = p.blob(&ext_kind()).unwrap();
        assert_eq!(ext.data.as_slice(), extension_area_bytes(&file).unwrap());
        assert!(!ext.kind.is_well_known());
        let dev = p.blob(&dev_kind()).unwrap();
        assert_eq!(
            TgaDeveloperArea::tags_from_portable_bytes(&dev.data).unwrap(),
            rich().developer_tags
        );
        assert_eq!(
            dmx.metadata(),
            &[
                ("author".to_string(), "Ada".to_string()),
                ("comment".to_string(), "one\ntwo".to_string()),
                ("date".to_string(), "2026-10-17T09:30:05".to_string()),
                ("job".to_string(), "job".to_string()),
                ("job_time".to_string(), "00:05:00".to_string()),
                ("software".to_string(), "oxideav".to_string()),
                ("software_version".to_string(), "0.10a".to_string()),
            ]
        );
        // A TGA 1.0 file carries neither.
        let plain = encode(&picture(), &EncodeOptions::default()).unwrap();
        let dmx = open(&plain);
        assert!(dmx.streams()[0].params.blobs.is_empty());
        assert!(dmx.metadata().is_empty());
    }

    #[test]
    fn the_decoder_attaches_the_same_blobs_to_the_frame_record() {
        let file = rich_file(RowOrder::TopDown);
        let v = decode_registry(&file);
        let blobs = v.blobs();
        assert_eq!(blobs.len(), 2);
        assert_eq!(
            v.blob(&ext_kind()).unwrap().data.as_slice(),
            extension_area_bytes(&file).unwrap()
        );
        assert!(v.blob(&dev_kind()).is_some());
        assert_eq!(
            v.image_plane_count(),
            1,
            "blobs are a side-channel record, not a plane"
        );
        assert_eq!(v.planes[0].data, picture().data());
        let plain = encode(&picture(), &EncodeOptions::default()).unwrap();
        assert!(decode_registry(&plain).blobs().is_empty());
    }

    #[test]
    fn the_encoder_writes_the_blobs_back_from_the_stream_parameters() {
        let file = rich_file(RowOrder::TopDown);
        let dmx = open(&file);
        let v = decode_registry(&file);
        let frame_without_record = VideoFrame {
            pts: None,
            planes: vec![v.planes[0].clone()],
        };
        // Stream-level only (the frame carries no record).
        let mut p = params(3, 2, PixelFormat::Rgb24);
        p.blobs = dmx.streams()[0].params.blobs.clone();
        let out = encode_registry(&frame_without_record, &p).unwrap();
        let area = parse_tga_extension_area(&out).unwrap();
        let want = parse_tga_extension_area(&file).unwrap();
        assert_eq!(area.author_name, want.author_name);
        assert_eq!(area.author_comment, want.author_comment);
        assert_eq!(area.timestamp, want.timestamp);
        assert_eq!(area.job_name, want.job_name);
        assert_eq!(area.job_time, want.job_time);
        assert_eq!(area.software_id, want.software_id);
        assert_eq!(area.software_version, want.software_version);
        assert_eq!(area.key_color, want.key_color);
        assert_eq!(area.pixel_aspect_ratio, want.pixel_aspect_ratio);
        assert_eq!(area.gamma, want.gamma);
        assert_eq!(area.attributes_type, want.attributes_type);
        let dev = parse_tga_developer_area(&out).unwrap();
        assert_eq!(
            TgaDeveloperArea::tags_from_portable_bytes(&dev.to_portable_bytes(&out)).unwrap(),
            rich().developer_tags
        );
        // The re-read file reports the same pairs.
        assert_eq!(open(&out).metadata(), dmx.metadata());
        assert_eq!(decode(&out).unwrap().data(), picture().data());
    }

    #[test]
    fn frame_blobs_refine_stream_blobs_and_the_frame_record_alone_suffices() {
        let file = rich_file(RowOrder::TopDown);
        let v = decode_registry(&file);
        // Frame record alone.
        let out = encode_registry(&v, &params(3, 2, PixelFormat::Rgb24)).unwrap();
        assert_eq!(parse_tga_extension_area(&out).unwrap().author_name, "Ada");
        assert_eq!(parse_tga_developer_area(&out).unwrap().len(), 2);
        // A frame blob overrides a stream blob of the same kind.
        let other = encode_tga_with_extension(
            &encode(&picture(), &EncodeOptions::default()).unwrap(),
            &ExtensionAreaInput {
                author_name: "Bob".to_string(),
                ..ExtensionAreaInput::default()
            },
        )
        .unwrap();
        let mut p = params(3, 2, PixelFormat::Rgb24);
        p.blobs = vec![MetadataBlob::new(
            ext_kind(),
            extension_area_bytes(&other).unwrap().to_vec(),
        )];
        let out = encode_registry(&v, &p).unwrap();
        assert_eq!(
            parse_tga_extension_area(&out).unwrap().author_name,
            "Ada",
            "the frame's own area wins"
        );
        // Stream blob alone with a frame carrying none → Bob.
        let bare = VideoFrame {
            pts: None,
            planes: vec![v.planes[0].clone()],
        };
        let out = encode_registry(&bare, &p).unwrap();
        assert_eq!(parse_tga_extension_area(&out).unwrap().author_name, "Bob");
    }

    #[test]
    fn malformed_blobs_are_typed_invalid_data() {
        let frame = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 1,
                data: vec![0],
            }],
        }
        .with_blob(MetadataBlob::new(ext_kind(), vec![0; 10]));
        match encode_registry(&frame, &params(1, 1, PixelFormat::Gray8)) {
            Err(oxideav_core::Error::InvalidData(msg)) => assert!(msg.contains("495"), "{msg}"),
            other => panic!("expected InvalidData, got {other:?}"),
        }
        let frame = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 1,
                data: vec![0],
            }],
        }
        .with_blob(MetadataBlob::new(dev_kind(), vec![1, 0, 1]));
        assert!(matches!(
            encode_registry(&frame, &params(1, 1, PixelFormat::Gray8)),
            Err(oxideav_core::Error::InvalidData(_))
        ));
        // Blobs of other kinds are ignored (TGA has no carrier for them).
        let frame = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 1,
                data: vec![0],
            }],
        }
        .with_blob(MetadataBlob::new(BlobKind::EXIF, vec![1, 2, 3]));
        let out = encode_registry(&frame, &params(1, 1, PixelFormat::Gray8)).unwrap();
        assert!(parse_tga_extension_area(&out).is_none());
    }

    #[test]
    fn the_descriptor_origin_never_leaks_into_pixel_order() {
        let top = rich_file(RowOrder::TopDown);
        let bottom = rich_file(RowOrder::BottomUp);
        assert_ne!(top, bottom, "the files differ in storage order");
        let vt = decode_registry(&top);
        let vb = decode_registry(&bottom);
        assert_eq!(vt.planes[0].data, vb.planes[0].data);
        assert_eq!(vt.planes[0].data, picture().data());
        assert_eq!(vt.blob(&ext_kind()), vb.blob(&ext_kind()));
        // Through the encoder (top-down default) and back: same picture.
        for v in [&vt, &vb] {
            let out = encode_registry(v, &params(3, 2, PixelFormat::Rgb24)).unwrap();
            assert!(oxideav_tga::info(&out).unwrap().top_down);
            assert_eq!(decode_registry(&out).planes[0].data, picture().data());
        }
    }
}

// ---- option schemas: every lossless axis by name, no `quality` ----

mod options {
    use super::*;
    use oxideav_core::CodecOptions;
    use oxideav_tga::{info, ImageType, TGA_HEADER_SIZE};

    fn with(opts: &[(&str, &str)], w: u32, h: u32, pf: PixelFormat) -> CodecParameters {
        let mut p = params(w, h, pf);
        for (k, v) in opts {
            p.options = p.options.clone().set(*k, *v);
        }
        p
    }

    fn gray_frame() -> VideoFrame {
        VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 4,
                data: vec![1, 1, 1, 1, 2, 3, 4, 5],
            }],
        }
    }

    #[test]
    fn quality_is_undeclared_and_refused() {
        let mut ctx = oxideav_core::RuntimeContext::new();
        oxideav_tga::register(&mut ctx);
        let enc = ctx
            .codecs
            .encoder_options_schema(&id())
            .expect("encoder schema declared");
        assert!(
            enc.iter().all(|f| f.name != "quality"),
            "lossless: no quality knob"
        );
        assert_eq!(enc.len(), 10);
        let dec = ctx
            .codecs
            .decoder_options_schema(&id())
            .expect("decoder schema declared");
        assert_eq!(dec.len(), 4);
        match oxideav_tga::make_encoder(&with(&[("quality", "50")], 4, 2, PixelFormat::Gray8)) {
            Err(oxideav_core::Error::InvalidData(msg)) => assert!(msg.contains("quality"), "{msg}"),
            Err(other) => panic!("expected InvalidData, got {other:?}"),
            Ok(_) => panic!("accepted a quality option"),
        }
        assert!(matches!(
            oxideav_tga::make_decoder(&with(&[("quality", "50")], 4, 2, PixelFormat::Gray8)),
            Err(oxideav_core::Error::InvalidData(_))
        ));
        // Identical bytes whatever a caller might think quality means.
        let a = encode_registry(&gray_frame(), &params(4, 2, PixelFormat::Gray8)).unwrap();
        let b = encode_registry(&gray_frame(), &params(4, 2, PixelFormat::Gray8)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn every_encoder_option_reaches_the_wire() {
        let f = gray_frame();
        let p = with(
            &[
                ("rle", "false"),
                ("row_order", "bottom-up"),
                ("image_id", "hello"),
                ("screen_origin_x", "7"),
                ("screen_origin_y", "9"),
                ("footer", "true"),
            ],
            4,
            2,
            PixelFormat::Gray8,
        );
        let bytes = encode_registry(&f, &p).unwrap();
        let i = info(&bytes).unwrap();
        assert_eq!(i.image_type, ImageType::UncompressedGrayscale);
        assert!(!i.top_down);
        assert_eq!(i.image_id_length, 5);
        assert!(i.has_footer && !i.has_extension_area);
        assert_eq!(&bytes[TGA_HEADER_SIZE..TGA_HEADER_SIZE + 5], b"hello");
        let hdr = oxideav_tga::header(&bytes).unwrap();
        assert_eq!(hdr.image_origin(), oxideav_tga::ImageOrigin::new(7, 9));
        // Bottom-up storage: the first stored row is the picture's last.
        assert_eq!(
            &bytes[TGA_HEADER_SIZE + 5..TGA_HEADER_SIZE + 9],
            &[2, 3, 4, 5]
        );
        assert_eq!(decode(&bytes).unwrap().data(), f.planes[0].data.as_slice());

        // Defaults: RLE, top-down, no id, no footer.
        let bytes = encode_registry(&f, &params(4, 2, PixelFormat::Gray8)).unwrap();
        let i = info(&bytes).unwrap();
        assert_eq!(i.image_type, ImageType::RleGrayscale);
        assert!(i.top_down && !i.has_footer && i.image_id_length == 0);
    }

    #[test]
    fn true_colour_depth_and_alpha_bits_options() {
        let rgba = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 8,
                data: vec![0xF8, 0x08, 0x10, 0xFF, 0x20, 0x28, 0x30, 0x00],
            }],
        };
        let p = with(&[("true_colour_depth", "16")], 2, 1, PixelFormat::Rgba);
        let bytes = encode_registry(&rgba, &p).unwrap();
        let i = info(&bytes).unwrap();
        assert_eq!((i.depth, i.attribute_bits), (16, 1));
        let p = with(&[("true_colour_depth", "15")], 2, 1, PixelFormat::Rgba);
        let i = info(&encode_registry(&rgba, &p).unwrap()).unwrap();
        assert_eq!((i.depth, i.attribute_bits), (15, 0));
        let p = with(
            &[("true_colour_depth", "native"), ("alpha_bits", "0")],
            2,
            1,
            PixelFormat::Rgba,
        );
        let i = info(&encode_registry(&rgba, &p).unwrap()).unwrap();
        assert_eq!((i.depth, i.attribute_bits), (32, 0));
        let p = with(&[("drop_opaque_alpha", "true")], 2, 1, PixelFormat::Rgba);
        let opaque = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 8,
                data: vec![1, 2, 3, 255, 4, 5, 6, 255],
            }],
        };
        assert_eq!(
            info(&encode_registry(&opaque, &p).unwrap()).unwrap().depth,
            24
        );
        // Out-of-range / unknown values are typed refusals at construction.
        for (k, v) in [
            ("alpha_bits", "16"),
            ("row_order", "sideways"),
            ("true_colour_depth", "24"),
            ("screen_origin_x", "65536"),
            ("rle", "maybe"),
        ] {
            assert!(
                matches!(
                    oxideav_tga::make_encoder(&with(&[(k, v)], 2, 1, PixelFormat::Rgba)),
                    Err(oxideav_core::Error::InvalidData(_))
                ),
                "{k}={v}"
            );
        }
        let long = "x".repeat(256);
        assert!(matches!(
            oxideav_tga::make_encoder(&with(&[("image_id", &long)], 2, 1, PixelFormat::Rgba)),
            Err(oxideav_core::Error::InvalidData(_))
        ));
    }

    #[test]
    fn palette_entry_size_option_pins_the_map_width() {
        let mut frame = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 2,
                data: vec![0, 1],
            }],
        };
        frame.set_palette(vec![0xFF, 0, 0, 0, 0xFF, 0]);
        // No record: auto = 24; pinned 32 / 16 / 15 are honoured.
        for (v, want) in [("auto", 24), ("32", 32), ("16", 16), ("15", 15), ("24", 24)] {
            let p = with(&[("palette_entry_size", v)], 2, 1, PixelFormat::Pal8);
            let bytes = encode_registry(&frame, &p).unwrap();
            assert_eq!(info(&bytes).unwrap().color_map_entry_size, want, "{v}");
            assert_eq!(
                decode(&bytes).unwrap().palette.unwrap().entries,
                vec![[0xFF, 0, 0, 255], [0, 0xFF, 0, 255]],
                "{v}: 5-bit-exact entries survive every width"
            );
        }
        // With a record, a pinned 24-bit map drops the alpha (explicit choice).
        frame.set_palette_alpha(vec![0, 255]);
        let p = with(&[("palette_entry_size", "24")], 2, 1, PixelFormat::Pal8);
        let bytes = encode_registry(&frame, &p).unwrap();
        assert_eq!(info(&bytes).unwrap().color_map_entry_size, 24);
        assert!(decode_registry(&bytes).palette_alpha().is_none());
    }

    #[test]
    fn decoder_options_limits_strict_and_display() {
        let img = TgaImage::from_rgba8(3, 2, vec![0; 24]).unwrap();
        let bytes = encode(&img, &EncodeOptions::default()).unwrap();
        let pkt = Packet::new(0, TimeBase::new(1, 1), bytes.clone());
        // max_pixels below the picture: ResourceExhausted before allocation.
        let mut dec =
            oxideav_tga::make_decoder(&with(&[("max_pixels", "5")], 3, 2, PixelFormat::Rgba))
                .unwrap();
        assert!(matches!(
            dec.send_packet(&pkt),
            Err(oxideav_core::Error::ResourceExhausted(_))
        ));
        let mut dec =
            oxideav_tga::make_decoder(&with(&[("max_bytes", "23")], 3, 2, PixelFormat::Rgba))
                .unwrap();
        assert!(matches!(
            dec.send_packet(&pkt),
            Err(oxideav_core::Error::ResourceExhausted(_))
        ));
        // 0 lifts the option's cap (the stream limits still apply).
        let mut dec =
            oxideav_tga::make_decoder(&with(&[("max_bytes", "0")], 3, 2, PixelFormat::Rgba))
                .unwrap();
        dec.send_packet(&pkt).unwrap();
        // The stream's DecoderLimits cap the geometry too.
        let mut p = params(3, 2, PixelFormat::Rgba);
        p.limits.max_pixels_per_frame = 4;
        let mut dec = oxideav_tga::make_decoder(&p).unwrap();
        assert!(matches!(
            dec.send_packet(&pkt),
            Err(oxideav_core::Error::ResourceExhausted(_))
        ));
        let mut p = params(3, 2, PixelFormat::Rgba);
        p.limits.max_alloc_bytes_per_frame = 16;
        let mut dec = oxideav_tga::make_decoder(&p).unwrap();
        assert!(matches!(
            dec.send_packet(&pkt),
            Err(oxideav_core::Error::ResourceExhausted(_))
        ));

        // strict: an RLE run spanning two rows is refused; the lenient
        // default takes it (2×2 Gray8, one run of 4).
        let mut spanning = vec![0u8; TGA_HEADER_SIZE];
        spanning[2] = 11;
        spanning[12] = 2;
        spanning[14] = 2;
        spanning[16] = 8;
        spanning[17] = 0x20;
        spanning.extend_from_slice(&[0x83, 9]);
        let pkt = Packet::new(0, TimeBase::new(1, 1), spanning);
        let mut lenient = oxideav_tga::make_decoder(&params(2, 2, PixelFormat::Gray8)).unwrap();
        lenient.send_packet(&pkt).unwrap();
        let mut strict =
            oxideav_tga::make_decoder(&with(&[("strict", "true")], 2, 2, PixelFormat::Gray8))
                .unwrap();
        assert!(matches!(
            strict.send_packet(&pkt),
            Err(oxideav_core::Error::InvalidData(_))
        ));

        // display: a 32-bit TGA 1.0 file whose descriptor declares no
        // attribute bits (the "TARGA-32" convention) is shown opaque —
        // the header's alpha bits honoured; the raw path keeps the
        // stored fourth byte.
        let targa32 = encode(&img, &EncodeOptions::default().with_alpha_bits(Some(0u8))).unwrap();
        assert_eq!(info(&targa32).unwrap().attribute_bits, 0);
        let raw = decode_registry(&targa32);
        assert!(raw.planes[0].data.chunks_exact(4).all(|p| p[3] == 0));
        let mut shown =
            oxideav_tga::make_decoder(&with(&[("display", "true")], 3, 2, PixelFormat::Rgba))
                .unwrap();
        shown
            .send_packet(&Packet::new(0, TimeBase::new(1, 1), targa32))
            .unwrap();
        let v = match shown.receive_frame().unwrap() {
            Frame::Video(v) => v,
            _ => panic!(),
        };
        assert!(v.planes[0].data.chunks_exact(4).all(|p| p[3] == 0xFF));
    }

    #[test]
    fn typed_option_structs_convert() {
        let o = oxideav_tga::TgaEncoderOptions::new();
        assert_eq!(o.to_encode_options().unwrap(), EncodeOptions::default());
        let d = oxideav_tga::TgaDecoderOptions::new()
            .to_decode_options(&oxideav_core::DecoderLimits::default());
        assert_eq!(d.max_bytes, Some(1 << 30));
        assert!(!d.strict);
        let _ = CodecOptions::new(); // the bag type the factories parse
    }
}
