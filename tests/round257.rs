//! Round 257 — typed [`TgaFooter`] accessors + canonical 26-byte
//! serialiser.
//!
//! The TGA 2.0 footer (spec §C.4) is a fixed 26-byte trailer at the
//! end of the file:
//!
//! * bytes 0..4   — `extension_area_offset` (LE u32, 0 = absent)
//! * bytes 4..8   — `developer_directory_offset` (LE u32, 0 = absent)
//! * bytes 8..26  — `"TRUEVISION-XFILE.\0"` (18 ASCII bytes including
//!   the period + binary-zero terminator)
//!
//! The struct itself has long been parsed (`parse_tga_footer`); this
//! round adds the same typed-accessor pattern that landed on KeyColor /
//! PixelAspectRatio / GammaValue / SoftwareVersion / TgaTimestamp /
//! JobTime / TgaAsciiField in r227 / r234 / r252:
//!
//! * [`TgaFooter::UNSET`] — all-zero marker-footer sentinel.
//! * [`TgaFooter::new`] / [`TgaFooter::as_tuple`] /
//!   [`TgaFooter::from_tuple`] — tuple round-trip in
//!   `(extension_area_offset, developer_directory_offset)` order.
//! * [`TgaFooter::has_extension_area`] /
//!   [`TgaFooter::has_developer_area`] — boolean "is this section
//!   present" using the spec's zero-means-absent convention.
//! * [`TgaFooter::is_marker_only`] — both offsets zero, i.e. a TGA 2.0
//!   file flagged as v2 but carrying no metadata sections.
//! * [`TgaFooter::offsets_within`] — sanity-check both offsets fit
//!   inside a buffer of length `input_len`, treating zero offsets as
//!   "section absent" (no constraint).
//! * [`TgaFooter::to_bytes`] — canonical 26-byte serialiser, the
//!   inverse of [`parse_tga_footer`].
//!
//! The encoder already builds the footer inline in
//! `encode_tga_with_extension`; the new `to_bytes` helper exposes the
//! same byte layout for callers that want to construct the trailer
//! independently (test fixtures, manual mux paths, sanity checks).
//! It is not wired into the encoder this round — the public encoder
//! surface is unchanged.

use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga_footer, ExtensionAreaInput,
    TgaFooter, TGA_FOOTER_MAGIC, TGA_FOOTER_SIZE,
};

// ---------------------------------------------------------------------------
// Constants / shape
// ---------------------------------------------------------------------------

#[test]
fn footer_size_constant_matches_spec() {
    // Spec §C.4 — 4 + 4 + 18 = 26.
    assert_eq!(TGA_FOOTER_SIZE, 26);
    assert_eq!(TGA_FOOTER_MAGIC.len(), 18);
    assert_eq!(TGA_FOOTER_MAGIC.as_slice(), b"TRUEVISION-XFILE.\0");
}

#[test]
fn footer_default_is_unset() {
    let f = TgaFooter::default();
    assert_eq!(f, TgaFooter::UNSET);
    assert_eq!(f.extension_area_offset, 0);
    assert_eq!(f.developer_directory_offset, 0);
    assert!(f.is_marker_only());
    assert!(!f.has_extension_area());
    assert!(!f.has_developer_area());
}

// ---------------------------------------------------------------------------
// new / as_tuple / from_tuple
// ---------------------------------------------------------------------------

#[test]
fn new_sets_both_offsets() {
    let f = TgaFooter::new(100, 200);
    assert_eq!(f.extension_area_offset, 100);
    assert_eq!(f.developer_directory_offset, 200);
}

#[test]
fn tuple_round_trip() {
    let f = TgaFooter::new(0xDEAD_BEEF, 0x0123_4567);
    let t = f.as_tuple();
    assert_eq!(t, (0xDEAD_BEEF, 0x0123_4567));
    let back = TgaFooter::from_tuple(t);
    assert_eq!(back, f);
}

#[test]
fn tuple_round_trip_unset() {
    let f = TgaFooter::UNSET;
    let back = TgaFooter::from_tuple(f.as_tuple());
    assert_eq!(back, f);
}

// ---------------------------------------------------------------------------
// has_extension_area / has_developer_area / is_marker_only
// ---------------------------------------------------------------------------

#[test]
fn marker_only_when_both_offsets_zero() {
    assert!(TgaFooter::UNSET.is_marker_only());
    assert!(TgaFooter::new(0, 0).is_marker_only());
    assert!(!TgaFooter::new(1, 0).is_marker_only());
    assert!(!TgaFooter::new(0, 1).is_marker_only());
    assert!(!TgaFooter::new(1, 1).is_marker_only());
}

#[test]
fn has_extension_area_when_offset_nonzero() {
    assert!(!TgaFooter::new(0, 99).has_extension_area());
    assert!(TgaFooter::new(18, 99).has_extension_area());
    assert!(TgaFooter::new(u32::MAX, 0).has_extension_area());
}

#[test]
fn has_developer_area_when_offset_nonzero() {
    assert!(!TgaFooter::new(99, 0).has_developer_area());
    assert!(TgaFooter::new(99, 18).has_developer_area());
    assert!(TgaFooter::new(0, u32::MAX).has_developer_area());
}

// ---------------------------------------------------------------------------
// offsets_within
// ---------------------------------------------------------------------------

#[test]
fn offsets_within_accepts_unset_for_any_size() {
    // Unset offsets contribute no constraint, so a UNSET footer fits
    // any buffer at least 26 bytes long.
    assert!(TgaFooter::UNSET.offsets_within(TGA_FOOTER_SIZE));
    assert!(TgaFooter::UNSET.offsets_within(1024));
    assert!(TgaFooter::UNSET.offsets_within(usize::MAX));
}

#[test]
fn offsets_within_rejects_buffers_smaller_than_footer() {
    // A buffer shorter than the footer can't carry a footer at all.
    assert!(!TgaFooter::UNSET.offsets_within(0));
    assert!(!TgaFooter::UNSET.offsets_within(25));
    assert!(TgaFooter::UNSET.offsets_within(26));
}

#[test]
fn offsets_within_checks_extension_area_offset() {
    let footer = TgaFooter::new(100, 0);
    // 100 + footer (26) = 126; the buffer must be at least 126 bytes.
    assert!(!footer.offsets_within(125));
    assert!(footer.offsets_within(126));
    assert!(footer.offsets_within(2048));
}

#[test]
fn offsets_within_checks_developer_area_offset() {
    let footer = TgaFooter::new(0, 500);
    assert!(!footer.offsets_within(525));
    assert!(footer.offsets_within(526));
    assert!(footer.offsets_within(10_000));
}

#[test]
fn offsets_within_checks_both_offsets() {
    let footer = TgaFooter::new(100, 500);
    assert!(!footer.offsets_within(525));
    assert!(footer.offsets_within(526));
    // The smaller offset (100) is also implicitly fine in a 526-byte
    // buffer; the larger one is the binding constraint.
}

#[test]
fn offsets_within_allows_offset_equal_to_footer_start() {
    // The extension area can start *exactly* at the byte the footer
    // begins on iff its size is zero — which won't be a real file, but
    // the geometry check shouldn't reject the boundary itself.
    let footer = TgaFooter::new(100, 0);
    // input_len = 126 → footer_start = 100; the offset equals
    // footer_start, which is the inclusive boundary.
    assert!(footer.offsets_within(126));
}

#[test]
fn offsets_within_rejects_offset_past_footer_start() {
    let footer = TgaFooter::new(101, 0);
    // input_len = 126 → footer_start = 100; the offset (101) is past
    // it, which would overlap the footer itself.
    assert!(!footer.offsets_within(126));
}

// ---------------------------------------------------------------------------
// to_bytes
// ---------------------------------------------------------------------------

#[test]
fn to_bytes_unset_is_zero_offsets_plus_magic() {
    let bytes = TgaFooter::UNSET.to_bytes();
    assert_eq!(bytes.len(), TGA_FOOTER_SIZE);
    assert_eq!(&bytes[0..4], &[0, 0, 0, 0]);
    assert_eq!(&bytes[4..8], &[0, 0, 0, 0]);
    assert_eq!(&bytes[8..], TGA_FOOTER_MAGIC.as_slice());
}

#[test]
fn to_bytes_encodes_offsets_little_endian() {
    let footer = TgaFooter::new(0x0123_4567, 0x89AB_CDEF);
    let bytes = footer.to_bytes();
    assert_eq!(&bytes[0..4], &[0x67, 0x45, 0x23, 0x01]);
    assert_eq!(&bytes[4..8], &[0xEF, 0xCD, 0xAB, 0x89]);
    assert_eq!(&bytes[8..], TGA_FOOTER_MAGIC.as_slice());
}

#[test]
fn to_bytes_round_trips_through_parse_footer() {
    for &(ext, dev) in &[
        (0u32, 0u32),
        (18, 0),
        (0, 18),
        (100, 200),
        (0xDEAD_BEEF, 0x0123_4567),
        (u32::MAX, u32::MAX),
    ] {
        let f = TgaFooter::new(ext, dev);
        let bytes = f.to_bytes();
        let back = parse_tga_footer(&bytes).expect("footer round-trips");
        assert_eq!(back, f, "round-trip mismatch for ext={ext} dev={dev}");
    }
}

#[test]
fn to_bytes_size_is_exactly_26() {
    // The signature has byte width baked into the type — guard against
    // future spec-divergent changes to TGA_FOOTER_SIZE.
    let bytes = TgaFooter::new(1, 2).to_bytes();
    assert_eq!(bytes.len(), 26);
}

// ---------------------------------------------------------------------------
// Integration — encoder-produced footer parses back to typed accessors
// ---------------------------------------------------------------------------

fn checker_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = (((x ^ y) & 1) * 255) as u8;
            out.extend_from_slice(&[v, v, v, 0xFF]);
        }
    }
    out
}

#[test]
fn encoder_v1_output_has_no_footer() {
    let bytes = encode_tga_uncompressed(4, 4, &checker_rgba(4, 4)).unwrap();
    // Plain uncompressed encoder writes a TGA 1.0 file — no footer.
    assert!(parse_tga_footer(&bytes).is_none());
}

#[test]
fn encoder_v2_footer_round_trips_typed_accessors() {
    // Round-trip through encode_tga_with_extension: a footer must be
    // present, its extension_area_offset must be non-zero (the
    // extension area was written), its developer_directory_offset
    // must be zero (no developer tags supplied), and the parsed footer
    // must equal the offsets advertised in the file.
    let base = encode_tga_uncompressed(8, 8, &checker_rgba(8, 8)).unwrap();
    let ext = ExtensionAreaInput {
        author_name: "Round 257".to_string(),
        ..ExtensionAreaInput::default()
    };
    let full = encode_tga_with_extension(&base, &ext).unwrap();

    let footer = parse_tga_footer(&full).expect("v2 footer present");
    assert!(footer.has_extension_area());
    assert!(!footer.has_developer_area());
    assert!(!footer.is_marker_only());
    assert!(footer.offsets_within(full.len()));
}

#[test]
fn encoder_v2_marker_only_footer_when_no_metadata() {
    // An empty ExtensionAreaInput still produces an extension area
    // (the writer fills it with NUL sentinels), so the extension area
    // offset is non-zero. Use as_tuple to confirm shape.
    let base = encode_tga_uncompressed(4, 4, &checker_rgba(4, 4)).unwrap();
    let ext = ExtensionAreaInput::default();
    let full = encode_tga_with_extension(&base, &ext).unwrap();
    let footer = parse_tga_footer(&full).expect("v2 footer present");
    let (ext_off, dev_off) = footer.as_tuple();
    assert_ne!(ext_off, 0);
    assert_eq!(dev_off, 0);
}

#[test]
fn encoder_v2_developer_tags_set_developer_offset() {
    use oxideav_tga::DeveloperTagInput;
    let base = encode_tga_uncompressed(4, 4, &checker_rgba(4, 4)).unwrap();
    let ext = ExtensionAreaInput {
        developer_tags: vec![DeveloperTagInput {
            tag_id: 12345,
            payload: vec![0xAA, 0xBB, 0xCC],
        }],
        ..ExtensionAreaInput::default()
    };
    let full = encode_tga_with_extension(&base, &ext).unwrap();
    let footer = parse_tga_footer(&full).expect("v2 footer present");
    assert!(footer.has_extension_area());
    assert!(footer.has_developer_area());
    assert!(!footer.is_marker_only());
    assert!(footer.offsets_within(full.len()));
}

#[test]
fn manually_assembled_footer_parses_back() {
    // Lay down a minimal trailer-only buffer: 26 bytes of canonical
    // footer with sentinel offsets. parse_tga_footer should pick up
    // the magic at the tail and recover the exact offsets.
    let mut buf = vec![0u8; 1024];
    let footer = TgaFooter::new(42, 84);
    let bytes = footer.to_bytes();
    let tail = buf.len() - bytes.len();
    buf[tail..].copy_from_slice(&bytes);

    let parsed = parse_tga_footer(&buf).expect("footer parses back");
    assert_eq!(parsed, footer);
    assert_eq!(parsed.as_tuple(), (42, 84));
}

#[test]
fn truncated_buffer_yields_none() {
    // Less than 26 bytes can't carry a footer.
    let bytes = [0u8; 25];
    assert!(parse_tga_footer(&bytes).is_none());
}

#[test]
fn footer_to_bytes_is_pure_constant_layout() {
    // The serialiser must not depend on any global state; calling it
    // twice with the same offsets yields byte-identical output.
    let a = TgaFooter::new(0x1234_5678, 0xABCD_EF01).to_bytes();
    let b = TgaFooter::new(0x1234_5678, 0xABCD_EF01).to_bytes();
    assert_eq!(a, b);
}
