#![no_main]

//! The registry path as an identity: arbitrary bytes through the
//! framework decoder (`make_decoder` → one `VideoFrame` in the native
//! layout with the palette record, the palette-alpha record for 16 /
//! 32-bit colour maps, and the `tga-extension-area` /
//! `tga-developer-area` blobs), then — whenever the bytes decoded — the
//! frame back through `make_encoder` with the stream parameters
//! `open_demuxer` derives from the same bytes, and the encoder's packet
//! decoded again. Because TGA is lossless and the registry path is a
//! thin adapter over `decode` / `encode`, the second frame must equal
//! the first in plane bytes, palette, palette-alpha record and the
//! typed extension-area / developer-area contents (the three table /
//! stamp pointers excepted: they describe the source file's layout).
//!
//! The true-colour frames are also re-labelled as every swizzled input
//! order the encoder declares (`Bgra` / `Argb` / `Abgr`, `Bgr24`) and
//! must reach the same file bytes as the `Rgba` / `Rgb24` path; and the
//! option schemas are driven with values derived from the input (the
//! lossless axes must keep the identity, a `quality` key must be
//! refused).
//!
//! Undecodable bytes exercise the decoder's typed-error paths only. The
//! 16 MiB raster cap of `decode_tga` applies (the stream's
//! `DecoderLimits` are set to it, so an oversized header is a typed
//! `ResourceExhausted` before any allocation).

use libfuzzer_sys::fuzz_target;
use oxideav_core::{
    CodecId, CodecParameters, DecoderLimits, Frame, Packet, PixelFormat, ReadSeek, TimeBase,
    VideoFrame, VideoPlane,
};
use oxideav_tga::{
    BLOB_KIND_DEVELOPER_AREA, BLOB_KIND_EXTENSION_AREA, CODEC_ID_STR, TgaDeveloperArea,
    TgaExtensionArea, make_decoder, make_encoder, parse_tga_developer_area,
    parse_tga_extension_area,
};

const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;

fn decode_frame(bytes: &[u8], params: &CodecParameters) -> Option<VideoFrame> {
    let mut dec = make_decoder(params).ok()?;
    dec.send_packet(&Packet::new(0, TimeBase::new(1, 1), bytes.to_vec()))
        .ok()?;
    match dec.receive_frame() {
        Ok(Frame::Video(v)) => Some(v),
        Ok(_) => panic!("TGA decoder emitted a non-video frame"),
        Err(_) => None,
    }
}

fn encode_frame(frame: &VideoFrame, params: &CodecParameters) -> oxideav_core::Result<Vec<u8>> {
    let mut enc = make_encoder(params)?;
    enc.send_frame(&Frame::Video(frame.clone()))?;
    Ok(enc.receive_packet()?.data)
}

/// The typed extension-area fields the encoder re-emits (pointers excluded).
fn ext_fields(a: &TgaExtensionArea) -> impl PartialEq + std::fmt::Debug + use<> {
    (
        a.author_name.clone(),
        a.author_comment.clone(),
        a.timestamp,
        a.job_name.clone(),
        a.job_time,
        a.software_id.clone(),
        a.software_version,
        a.key_color,
        a.pixel_aspect_ratio,
        a.gamma,
        a.attributes_type,
    )
}

fn dev_tags(bytes: &[u8]) -> Option<Vec<(u16, Vec<u8>)>> {
    let dev = parse_tga_developer_area(bytes)?;
    let portable = dev.to_portable_bytes(bytes);
    let tags = TgaDeveloperArea::tags_from_portable_bytes(&portable)
        .expect("a portable form the crate wrote reads back");
    Some(tags.into_iter().map(|t| (t.tag_id, t.payload)).collect())
}

fuzz_target!(|data: &[u8]| {
    let mut params = CodecParameters::video(CodecId::new(CODEC_ID_STR));
    let mut limits = DecoderLimits::default();
    limits.max_pixels_per_frame = MAX_OUTPUT_BYTES;
    limits.max_alloc_bytes_per_frame = MAX_OUTPUT_BYTES;
    params.limits = limits;
    let Some(frame) = decode_frame(data, &params) else {
        return;
    };

    // The demuxer on the same bytes: its stream parameters are what a
    // framework consumer hands the encoder (geometry, label, blobs).
    let ctx = oxideav_core::RuntimeContext::new();
    let input: Box<dyn ReadSeek> = Box::new(std::io::Cursor::new(data.to_vec()));
    let dmx = oxideav_tga::container::open_demuxer(input, &ctx.codecs)
        .expect("a file the decoder accepted opens through the demuxer");
    let stream = &dmx.streams()[0].params;
    let label = stream.pixel_format.expect("the demuxer labels the stream");
    let (w, h) = (stream.width.unwrap(), stream.height.unwrap());
    assert_eq!(frame.image_plane_count(), 1, "one packed plane");
    let bpp = oxideav_tga::registry::from_core_pixel_format(label)
        .expect("the demuxer labels a TGA layout")
        .bytes_per_pixel();
    assert_eq!(
        frame.planes[0].data.len(),
        (w as usize) * (h as usize) * bpp,
        "the plane matches the label"
    );
    // Blobs: the frame record and the stream carry the same set.
    let frame_blobs = frame.blobs();
    assert_eq!(frame_blobs.len(), stream.blobs.len());
    for b in &stream.blobs {
        assert!(
            frame_blobs
                .iter()
                .any(|f| f.kind == b.kind && f.data == b.data)
        );
    }
    let ext_kind = oxideav_core::BlobKind::from_static(BLOB_KIND_EXTENSION_AREA);
    let dev_kind = oxideav_core::BlobKind::from_static(BLOB_KIND_DEVELOPER_AREA);
    let source_ext = parse_tga_extension_area(data);
    assert_eq!(
        frame.blob(&ext_kind).is_some(),
        source_ext.is_some(),
        "the extension blob is present exactly when the file has an extension area"
    );
    if let Some(b) = frame.blob(&ext_kind) {
        assert_eq!(
            TgaExtensionArea::from_bytes(&b.data)
                .as_ref()
                .map(ext_fields),
            source_ext.as_ref().map(ext_fields)
        );
    }
    let source_dev = dev_tags(data).filter(|t| !t.is_empty());
    assert_eq!(frame.blob(&dev_kind).is_some(), source_dev.is_some());

    // Palette record rules: Pal8 carries a palette; the alpha record is
    // present exactly for 16 / 32-bit colour maps.
    let hdr = oxideav_tga::header(data).unwrap();
    if label == PixelFormat::Pal8 {
        assert!(frame.palette().is_some());
        let map_alpha = hdr.cmap_type == 1 && matches!(hdr.cmap_entry_size, 16 | 32);
        assert_eq!(frame.palette_alpha().is_some(), map_alpha);
    } else {
        assert!(frame.palette().is_none());
    }

    // Encode with the stream's parameters, decode again: identity.
    let mut enc_params = stream.clone();
    enc_params.limits = params.limits;
    let bytes = encode_frame(&frame, &enc_params)
        .expect("a frame the decoder produced encodes through the registry");
    let back = decode_frame(&bytes, &params).expect("the encoder's file decodes");
    assert_eq!(back.planes[0].data, frame.planes[0].data, "plane bytes");
    assert_eq!(back.planes[0].stride, frame.planes[0].stride, "stride");
    assert_eq!(back.palette(), frame.palette(), "palette");
    assert_eq!(back.palette_alpha(), frame.palette_alpha(), "palette alpha");
    // The extension area comes back field for field (its text bytes
    // verbatim). A developer area alone is written with an extension
    // area whose every field is unset (the writer emits the two
    // together).
    let expect_ext = source_ext.clone().or_else(|| {
        source_dev.as_ref().map(|_| TgaExtensionArea {
            // An all-zero area reads its version letter as NUL.
            software_version: (0, '\0'),
            ..TgaExtensionArea::default()
        })
    });
    assert_eq!(
        parse_tga_extension_area(&bytes).as_ref().map(ext_fields),
        expect_ext.as_ref().map(ext_fields),
        "extension-area fields"
    );
    assert_eq!(
        dev_tags(&bytes).filter(|t| !t.is_empty()),
        source_dev,
        "developer tags"
    );
    // Row order never leaks: the written file is top-down whatever the
    // source stored.
    assert!(oxideav_tga::info(&bytes).unwrap().top_down);

    // Swizzled input orders reach the same file.
    type Swizzle = fn(&[u8]) -> Vec<u8>;
    let swizzles: &[(PixelFormat, Swizzle)] = match label {
        PixelFormat::Rgba => &[
            (PixelFormat::Bgra, |p| vec![p[2], p[1], p[0], p[3]]),
            (PixelFormat::Argb, |p| vec![p[3], p[0], p[1], p[2]]),
            (PixelFormat::Abgr, |p| vec![p[3], p[2], p[1], p[0]]),
        ],
        PixelFormat::Rgb24 => &[(PixelFormat::Bgr24, |p| vec![p[2], p[1], p[0]])],
        _ => &[],
    };
    for (pf, swz) in swizzles {
        let data: Vec<u8> = frame.planes[0]
            .data
            .chunks_exact(bpp)
            .flat_map(swz)
            .collect();
        let other = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: frame.planes[0].stride,
                data,
            }],
        };
        let mut p = enc_params.clone();
        p.pixel_format = Some(*pf);
        p.blobs.clear();
        let mut plain = enc_params.clone();
        plain.blobs.clear();
        let bare = VideoFrame {
            pts: None,
            planes: vec![frame.planes[0].clone()],
        };
        assert_eq!(
            encode_frame(&other, &p).expect("declared order"),
            encode_frame(&bare, &plain).unwrap(),
            "{pf:?} reaches the Rgba / Rgb24 file"
        );
    }

    // Option schemas: the lossless axes keep the identity; `quality` is refused.
    let flags = data.last().copied().unwrap_or(0);
    let mut p = enc_params.clone();
    p.options = p
        .options
        .set("rle", if flags & 1 != 0 { "true" } else { "false" })
        .set(
            "row_order",
            if flags & 2 != 0 {
                "bottom-up"
            } else {
                "top-down"
            },
        )
        .set("footer", if flags & 4 != 0 { "true" } else { "false" })
        .set("image_id", "fuzz");
    if label == PixelFormat::Pal8 {
        p.options = p.options.set(
            "palette_entry_size",
            if flags & 8 != 0 { "32" } else { "auto" },
        );
    }
    let bytes = encode_frame(&frame, &p).expect("lossless options encode");
    let back = decode_frame(&bytes, &params).expect("decodes");
    assert_eq!(
        back.planes[0].data, frame.planes[0].data,
        "plane bytes under options"
    );
    assert_eq!(back.palette(), frame.palette(), "palette under options");
    let mut q = enc_params.clone();
    q.options = q.options.set("quality", "90");
    assert!(
        matches!(make_encoder(&q), Err(oxideav_core::Error::InvalidData(_))),
        "quality is not a TGA option"
    );
});
