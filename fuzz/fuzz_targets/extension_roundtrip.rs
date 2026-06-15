#![no_main]

//! Extension-area encode↔decode round-trip target.
//!
//! The [`encode_roundtrip`](encode_roundtrip.rs) target drives the
//! eight base pixel writers; the [`decode_tga`](decode_tga.rs) target
//! drives the read side on arbitrary bytes. Neither exercises
//! [`encode_tga_with_extension`] — the writer that appends a TGA 2.0
//! footer plus a 495-byte extension area, and then back-patches the
//! extension area's internal offset pointers (Field 21 colour-correction
//! offset, Field 22 postage-stamp offset, Field 23 scan-line offset) and
//! the footer's developer-directory offset to wherever each variable-
//! length block actually landed in the output buffer.
//!
//! That offset back-patch arithmetic is the part with the most
//! arithmetic surface and the least test coverage. This target attacks
//! it: from arbitrary fuzz bytes it builds
//!
//! * a small base TGA (via [`encode_tga_uncompressed`]), and
//! * an [`ExtensionAreaInput`] carrying every optional block — gamma
//!   (Field 20), key colour (Field 18), pixel aspect ratio (Field 19),
//!   software version (Field 17), attributes type (Field 24), a
//!   §C.6.8 colour-correction table (Field 27), a §C.6.9 scan-line
//!   table (Field 25), a §C.6.10 postage-stamp thumbnail (Field 26),
//!   and §C.7 developer-area tags —
//!
//! then runs `encode_tga_with_extension`, re-parses the result through
//! the public `parse_tga_*` readers, and asserts the round trip is
//! faithful:
//!
//! 1. The encoder never panics regardless of input shape (length /
//!    palette / oversized-raster failures are documented `Err`s, not
//!    fuzz findings).
//! 2. On `Ok(bytes)`, every reader runs without panicking, and the
//!    footer is recognised as a TGA 2.0 footer with a present extension
//!    area.
//! 3. Lossless blocks round-trip byte- / value-exact: the parsed
//!    colour-correction table equals the one we wrote, the parsed
//!    scan-line offsets equal the ones we wrote, and the verbatim
//!    SHORT-pair fields (gamma, key colour, aspect ratio, software
//!    version) come back with the exact tuples we supplied. These are
//!    stronger oracles than `encode_roundtrip`'s frame-shape check
//!    because the extension-area blocks are stored without any
//!    lossy re-quantisation.
//!
//! # Why no external oracle
//!
//! Same clean-room wall as the sibling targets: there is no trusted
//! third-party TGA implementation in this codebase to cross-check
//! against, so the oracle is the crate's own write→read symmetry.
//!
//! # Raster cap
//!
//! The base image and the postage stamp are both clamped to small side
//! lengths so the harness never allocates a multi-megabyte raster — the
//! logic under test is the offset arithmetic, not the pixel loop, which
//! the other two targets already stress at size.

use libfuzzer_sys::fuzz_target;
use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga_colour_correction_table,
    parse_tga_extension_area, parse_tga_footer, parse_tga_gamma, parse_tga_key_color,
    parse_tga_pixel_aspect_ratio, parse_tga_scan_line_table, parse_tga_software_version,
    DeveloperTagInput, ExtensionAreaInput, TgaColourCorrectionTable, TgaImage, TgaPixelFormat,
    TgaScanLineTable, TGA_COLOUR_CORRECTION_TABLE_ENTRIES,
};

/// Maximum side length for the base image. Keeps the raster tiny — the
/// offset arithmetic under test is independent of frame size.
const MAX_SIDE: u16 = 64;

/// Maximum side length for the optional postage-stamp thumbnail (the
/// spec recommends stamps stay at or below 64 × 64).
const MAX_STAMP_SIDE: u8 = 16;

/// A tiny cursor over the fuzz bytes: each `next_*` consumes from the
/// front and saturates to a default once the input is exhausted, so the
/// harness degrades gracefully on short inputs instead of bailing early.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn next_u8(&mut self) -> u8 {
        let b = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b
    }

    fn next_u16(&mut self) -> u16 {
        u16::from_le_bytes([self.next_u8(), self.next_u8()])
    }

    fn next_bool(&mut self) -> bool {
        self.next_u8() & 1 == 1
    }

    /// Remaining unconsumed bytes (used to tile pixel payloads).
    fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }
}

/// Fill `len` bytes by tiling `seed` (or zeroes when `seed` is empty).
fn tile(seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    if seed.is_empty() {
        out.resize(len, 0);
        return out;
    }
    while out.len() < len {
        let take = (len - out.len()).min(seed.len());
        out.extend_from_slice(&seed[..take]);
    }
    out
}

fuzz_target!(|data: &[u8]| {
    let mut cur = Cursor::new(data);

    // --- Base image -------------------------------------------------
    let w = (cur.next_u16() % MAX_SIDE).max(1);
    let h = (cur.next_u16() % MAX_SIDE).max(1);
    let pixels = tile(cur.rest(), w as usize * h as usize * 4);
    let base = match encode_tga_uncompressed(w, h, &pixels) {
        Ok(b) => b,
        Err(_) => return,
    };

    // --- Extension-area inputs --------------------------------------
    // Verbatim SHORT-pair fields: the encoder stores these untouched so
    // we can assert exact recovery.
    let gamma = (cur.next_u16(), cur.next_u16());
    let key_color = [cur.next_u8(), cur.next_u8(), cur.next_u8(), cur.next_u8()];
    let aspect = (cur.next_u16(), cur.next_u16());
    let sw_version_num = cur.next_u16();
    let sw_version_letter = (b'A' + (cur.next_u8() % 26)) as char;
    let attributes_type = cur.next_u8();

    // Optional colour-correction table (Field 27): 256 × 4 SHORTs. We
    // seed every channel curve from the fuzz tail so the on-disk block
    // is non-trivial and the parse must reconstruct it exactly.
    let want_cct = cur.next_bool();
    let cct = if want_cct {
        let mut t = TgaColourCorrectionTable::default();
        for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
            t.alpha[i] = cur.next_u16();
            t.red[i] = cur.next_u16();
            t.green[i] = cur.next_u16();
            t.blue[i] = cur.next_u16();
        }
        Some(t)
    } else {
        None
    };

    // Optional scan-line table (Field 25): one u32 offset per row. We
    // size it to the parent height (what a conformant writer does) and
    // fill it with arbitrary offsets — the values are opaque to the
    // round trip, only their faithful preservation matters.
    let want_sct = cur.next_bool();
    let sct = if want_sct {
        let mut offsets = Vec::with_capacity(h as usize);
        for _ in 0..h {
            offsets.push(u32::from(cur.next_u16()));
        }
        Some(TgaScanLineTable { offsets })
    } else {
        None
    };

    // Optional postage stamp (Field 26): a tiny uncompressed thumbnail.
    let want_stamp = cur.next_bool();
    let stamp = if want_stamp {
        let sw = (cur.next_u8() % MAX_STAMP_SIDE).max(1);
        let sh = (cur.next_u8() % MAX_STAMP_SIDE).max(1);
        let data = tile(cur.rest(), sw as usize * sh as usize * 4);
        Some(TgaImage {
            width: sw as u32,
            height: sh as u32,
            pixel_format: TgaPixelFormat::Rgba,
            data,
            pts: None,
        })
    } else {
        None
    };

    // Optional developer tags (Field 9 / §C.7): up to a few tags with
    // short payloads, exercising the directory back-patch path.
    let tag_count = (cur.next_u8() % 4) as usize;
    let mut developer_tags = Vec::with_capacity(tag_count);
    for _ in 0..tag_count {
        let tag_id = cur.next_u16();
        let payload_len = (cur.next_u8() % 16) as usize;
        let payload = tile(cur.rest(), payload_len);
        developer_tags.push(DeveloperTagInput { tag_id, payload });
    }

    let ext = ExtensionAreaInput {
        gamma,
        key_color,
        pixel_aspect_ratio: aspect,
        software_version: (sw_version_num, sw_version_letter),
        attributes_type,
        colour_correction_table: cct.clone(),
        scan_line_table: sct.clone(),
        postage_stamp: stamp,
        developer_tags,
        ..ExtensionAreaInput::default()
    };

    let bytes = match encode_tga_with_extension(&base, &ext) {
        Ok(b) => b,
        Err(_) => return,
    };

    // --- Round-trip assertions --------------------------------------
    // A file we just wrote with a footer MUST be recognised as TGA 2.0
    // with a present extension area.
    let footer = parse_tga_footer(&bytes).expect("written footer must parse");
    assert!(
        footer.has_extension_area(),
        "written extension area offset must be non-zero"
    );
    assert!(
        parse_tga_extension_area(&bytes).is_some(),
        "written extension area must parse back"
    );

    // Verbatim SHORT-pair fields round-trip exactly.
    let g = parse_tga_gamma(&bytes).expect("gamma present");
    assert_eq!(g.as_tuple(), gamma, "gamma round-trip mismatch");

    let kc = parse_tga_key_color(&bytes).expect("key colour present");
    assert_eq!(kc.to_argb(), key_color, "key colour round-trip mismatch");

    let ar = parse_tga_pixel_aspect_ratio(&bytes).expect("aspect present");
    assert_eq!(ar.as_tuple(), aspect, "aspect-ratio round-trip mismatch");

    let sv = parse_tga_software_version(&bytes).expect("software version present");
    assert_eq!(
        sv.as_tuple(),
        (sw_version_num, sw_version_letter),
        "software-version round-trip mismatch"
    );

    // Lossless variable-length blocks round-trip byte-/value-exact.
    if let Some(want) = cct {
        let got = parse_tga_colour_correction_table(&bytes)
            .expect("colour-correction table must parse back when written");
        assert_eq!(
            got, want,
            "colour-correction table round-trip mismatch (offset back-patch?)"
        );
    }

    if let Some(want) = sct {
        let got = parse_tga_scan_line_table(&bytes)
            .expect("scan-line table must parse back when written");
        assert_eq!(
            got.offsets, want.offsets,
            "scan-line table round-trip mismatch (offset back-patch?)"
        );
    }
});
