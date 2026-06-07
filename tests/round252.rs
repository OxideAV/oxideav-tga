//! Round 252 — typed Field 11 / 12 / 14 / 16 (Author Name / Author
//! Comments / Job Name / Software ID) extension-area accessors plus
//! matching parser conveniences.
//!
//! The TGA 2.0 extension area (§C.6) reserves four ASCII slots for
//! free-form bookkeeping that the decoder has long parsed and the
//! encoder has long written byte-exactly, but which were only ever
//! exposed in raw form on [`TgaExtensionArea`]:
//!
//! * Field 11 — Author Name (41 bytes, 40 ASCII chars + binary-zero
//!   terminator). Carried as a raw `String`.
//! * Field 12 — Author Comments (324 bytes, 4 lines × 81 bytes each,
//!   80 ASCII chars + binary-zero terminator per line). Carried as a
//!   raw `[String; 4]`.
//! * Field 14 — Job Name/ID (41 bytes, same shape as Field 11).
//!   Carried as a raw `String`.
//! * Field 16 — Software ID (41 bytes, same shape as Field 11).
//!   Carried as a raw `String`.
//!
//! This round adds typed views matching the r227 / r234 pattern
//! already in place for the numeric extension-area fields:
//!
//! * [`TgaAsciiField`] is new — wraps the parsed `String` for all
//!   three 41-byte fields with [`TgaAsciiField::is_unset`] /
//!   [`TgaAsciiField::is_valid_ascii`] / [`TgaAsciiField::fits_capacity`]
//!   / [`TgaAsciiField::trimmed`] / [`TgaAsciiField::char_len`] /
//!   [`TgaAsciiField::as_str`] / [`TgaAsciiField::into_inner`].
//! * [`TgaAuthorComments`] is new — wraps the four parsed comment
//!   lines with [`TgaAuthorComments::is_unset`] /
//!   [`TgaAuthorComments::is_valid_ascii`] /
//!   [`TgaAuthorComments::fits_capacity`] / [`TgaAuthorComments::line`]
//!   / [`TgaAuthorComments::joined`] /
//!   [`TgaAuthorComments::empty`].
//! * [`TgaExtensionArea::author_name_typed`] /
//!   [`TgaExtensionArea::author_comments_typed`] /
//!   [`TgaExtensionArea::job_name_typed`] /
//!   [`TgaExtensionArea::software_id_typed`] return the typed views.
//! * Convenience parsers [`parse_tga_author_name`] /
//!   [`parse_tga_author_comments`] / [`parse_tga_job_name`] /
//!   [`parse_tga_software_id`] walk the footer + extension area in
//!   one call.
//!
//! Tests cover:
//!
//! * Sentinel detection for `is_unset`: empty / NUL-only / blanks /
//!   blanks-terminated-by-null all return `true`; a meaningful
//!   payload returns `false`.
//! * `is_valid_ascii` for printable / non-printable / high-bit
//!   inputs.
//! * `fits_capacity` for the 40-character and 80-character caps.
//! * `trimmed` strips leading + trailing ASCII whitespace.
//! * `joined` skips trailing blank lines on the comment block.
//! * The four `_typed` accessors and the four convenience parsers
//!   round-trip through `encode_tga_with_extension`, and return
//!   `None` on a file that has no extension area.

use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga_author_comments,
    parse_tga_author_name, parse_tga_job_name, parse_tga_software_id, ExtensionAreaInput,
    TgaAsciiField, TgaAuthorComments, TGA_ASCII_FIELD_MAX_CHARS, TGA_AUTHOR_COMMENT_LINES,
    TGA_AUTHOR_COMMENT_LINE_BYTES, TGA_AUTHOR_COMMENT_LINE_MAX_CHARS,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

#[test]
fn ascii_field_constants_match_spec() {
    // Spec §C.6 fixed-width ASCII fields are 41 bytes — 40 character
    // slots plus a binary-zero terminator.
    assert_eq!(TGA_ASCII_FIELD_MAX_CHARS, 40);
    // Author Comments is 4 lines × 81 bytes (80 chars + NUL per line).
    assert_eq!(TGA_AUTHOR_COMMENT_LINES, 4);
    assert_eq!(TGA_AUTHOR_COMMENT_LINE_BYTES, 81);
    assert_eq!(TGA_AUTHOR_COMMENT_LINE_MAX_CHARS, 80);
}

// ---------------------------------------------------------------------------
// TgaAsciiField — used by Author Name / Job Name / Software ID
// ---------------------------------------------------------------------------

#[test]
fn ascii_field_default_is_empty_and_unset() {
    let f = TgaAsciiField::default();
    assert!(f.value.is_empty());
    assert!(f.is_unset());
    assert!(f.is_valid_ascii());
    assert!(f.fits_capacity());
    assert_eq!(f.char_len(), 0);
}

#[test]
fn ascii_field_new_and_borrowed_constructors_match() {
    let a = TgaAsciiField::new("hello".to_string());
    let b = TgaAsciiField::from_borrowed("hello");
    assert_eq!(a, b);
    assert_eq!(a.as_str(), "hello");
    assert_eq!(a.into_inner(), "hello".to_string());
}

#[test]
fn ascii_field_is_unset_treats_blanks_and_nuls_as_unused() {
    // Empty string.
    assert!(TgaAsciiField::from_borrowed("").is_unset());
    // The spec recommends "a null terminated series of blanks
    // (spaces)" as the not-being-used form.
    assert!(TgaAsciiField::from_borrowed("    ").is_unset());
    // A NUL-only payload (the parser strips trailing NULs so this is
    // also legitimate as an in-memory unset state).
    let nul_payload = TgaAsciiField::new("\0\0\0".to_string());
    assert!(nul_payload.is_unset());
}

#[test]
fn ascii_field_is_unset_false_for_payload() {
    assert!(!TgaAsciiField::from_borrowed("Alice").is_unset());
    // A leading blank with a non-blank tail is NOT the spec sentinel.
    assert!(!TgaAsciiField::from_borrowed(" Bob").is_unset());
}

#[test]
fn ascii_field_is_valid_ascii_accepts_printable_only() {
    // Printable ASCII.
    assert!(TgaAsciiField::from_borrowed("Alice Smith").is_valid_ascii());
    assert!(TgaAsciiField::from_borrowed("!@#$%^&*()").is_valid_ascii());
    // Empty is trivially valid.
    assert!(TgaAsciiField::from_borrowed("").is_valid_ascii());
    // Control character — invalid.
    assert!(!TgaAsciiField::new("Alice\nSmith".to_string()).is_valid_ascii());
    // Tab is control — invalid.
    assert!(!TgaAsciiField::new("Alice\tSmith".to_string()).is_valid_ascii());
    // High-bit non-ASCII — invalid.
    assert!(!TgaAsciiField::from_borrowed("Café").is_valid_ascii());
    // A bare NUL is also non-printable.
    assert!(!TgaAsciiField::new("A\0B".to_string()).is_valid_ascii());
}

#[test]
fn ascii_field_fits_capacity_respects_40_char_cap() {
    let exactly_40 = "0123456789012345678901234567890123456789";
    assert_eq!(exactly_40.len(), TGA_ASCII_FIELD_MAX_CHARS);
    assert!(TgaAsciiField::from_borrowed(exactly_40).fits_capacity());

    let too_long = "0123456789012345678901234567890123456789X";
    assert_eq!(too_long.len(), TGA_ASCII_FIELD_MAX_CHARS + 1);
    assert!(!TgaAsciiField::from_borrowed(too_long).fits_capacity());

    // Empty fits trivially.
    assert!(TgaAsciiField::from_borrowed("").fits_capacity());
}

#[test]
fn ascii_field_trimmed_strips_ascii_whitespace() {
    assert_eq!(TgaAsciiField::from_borrowed("  Alice  ").trimmed(), "Alice");
    assert_eq!(TgaAsciiField::from_borrowed("").trimmed(), "");
    // Whitespace-only trims to empty.
    assert_eq!(TgaAsciiField::from_borrowed("    ").trimmed(), "");
    // Preserves interior whitespace.
    assert_eq!(
        TgaAsciiField::from_borrowed(" Alice Smith ").trimmed(),
        "Alice Smith"
    );
}

#[test]
fn ascii_field_char_len_matches_byte_len() {
    assert_eq!(TgaAsciiField::from_borrowed("Alice").char_len(), 5);
    assert_eq!(TgaAsciiField::from_borrowed("").char_len(), 0);
    let f = TgaAsciiField::from_borrowed("0123456789");
    assert_eq!(f.char_len(), 10);
}

// ---------------------------------------------------------------------------
// TgaAuthorComments
// ---------------------------------------------------------------------------

#[test]
fn author_comments_empty_constant_has_four_empty_lines() {
    let c = TgaAuthorComments::empty();
    assert_eq!(c.lines.len(), TGA_AUTHOR_COMMENT_LINES);
    for line in &c.lines {
        assert!(line.is_empty());
    }
    assert!(c.is_unset());
    assert!(c.is_valid_ascii());
    assert!(c.fits_capacity());
}

#[test]
fn author_comments_default_matches_empty_constant() {
    assert_eq!(TgaAuthorComments::default(), TgaAuthorComments::empty());
}

#[test]
fn author_comments_from_strs_constructor() {
    let c = TgaAuthorComments::from_strs(["line one", "line two", "", ""]);
    assert_eq!(c.line(0), Some("line one"));
    assert_eq!(c.line(1), Some("line two"));
    assert_eq!(c.line(2), Some(""));
    assert_eq!(c.line(3), Some(""));
    assert_eq!(c.line(4), None);
}

#[test]
fn author_comments_is_unset_for_blanks_and_nuls() {
    assert!(TgaAuthorComments::empty().is_unset());
    // Blanks-only is unset.
    let blanks = TgaAuthorComments::from_strs(["   ", "   ", "   ", "   "]);
    assert!(blanks.is_unset());
    // NUL-only is unset.
    let nuls = TgaAuthorComments::new([
        "\0\0".to_string(),
        "\0".to_string(),
        String::new(),
        String::new(),
    ]);
    assert!(nuls.is_unset());
}

#[test]
fn author_comments_is_unset_false_when_any_line_has_payload() {
    let c = TgaAuthorComments::from_strs(["", "", "", "trailing payload"]);
    assert!(!c.is_unset());
}

#[test]
fn author_comments_is_valid_ascii_classification() {
    // All printable lines.
    assert!(TgaAuthorComments::from_strs(["hello", "world", "", ""]).is_valid_ascii());
    // Empty is trivially valid.
    assert!(TgaAuthorComments::empty().is_valid_ascii());
    // One non-printable line spoils the whole block.
    let bad = TgaAuthorComments::new([
        "hello".to_string(),
        "wor\tld".to_string(),
        String::new(),
        String::new(),
    ]);
    assert!(!bad.is_valid_ascii());
}

#[test]
fn author_comments_fits_capacity_respects_80_char_cap() {
    let exactly_80 = "0".repeat(TGA_AUTHOR_COMMENT_LINE_MAX_CHARS);
    let too_long = "0".repeat(TGA_AUTHOR_COMMENT_LINE_MAX_CHARS + 1);
    let ok = TgaAuthorComments::new([
        exactly_80.clone(),
        exactly_80.clone(),
        exactly_80.clone(),
        exactly_80.clone(),
    ]);
    assert!(ok.fits_capacity());
    let bad = TgaAuthorComments::new([
        too_long.clone(),
        String::new(),
        String::new(),
        String::new(),
    ]);
    assert!(!bad.fits_capacity());
}

#[test]
fn author_comments_joined_drops_trailing_blank_lines() {
    let c = TgaAuthorComments::from_strs(["a", "b", "c", "d"]);
    assert_eq!(c.joined(), "a\nb\nc\nd");
    // Drops trailing blanks.
    let c = TgaAuthorComments::from_strs(["a", "b", "", ""]);
    assert_eq!(c.joined(), "a\nb");
    // Drops trailing blanks-only.
    let c = TgaAuthorComments::from_strs(["a", "  ", "", ""]);
    assert_eq!(c.joined(), "a");
    // Empty in the middle is preserved.
    let c = TgaAuthorComments::from_strs(["a", "", "c", ""]);
    assert_eq!(c.joined(), "a\n\nc");
    // All blank — joined is empty.
    assert_eq!(TgaAuthorComments::empty().joined(), "");
}

// ---------------------------------------------------------------------------
// TgaExtensionArea::*_typed accessors and parser conveniences round-trip
// ---------------------------------------------------------------------------

fn one_pixel_base() -> Vec<u8> {
    // 1×1 RGBA opaque red.
    encode_tga_uncompressed(1, 1, &[0xFF, 0x00, 0x00, 0xFF]).expect("encode 1x1 base")
}

#[test]
fn typed_accessors_round_trip_through_encoder() {
    let ext = ExtensionAreaInput {
        author_name: "Ada Lovelace".to_string(),
        author_comment: [
            "First Comment Line".to_string(),
            "Second Line".to_string(),
            String::new(),
            String::new(),
        ],
        job_name: "Analytic Engine Render".to_string(),
        software_id: "OxideAV TGA".to_string(),
        ..Default::default()
    };

    let base = one_pixel_base();
    let full = encode_tga_with_extension(&base, &ext).expect("with-extension");
    let parsed = oxideav_tga::parse_tga_extension_area(&full).expect("ext parsed");

    let author = parsed.author_name_typed();
    assert_eq!(author.as_str(), "Ada Lovelace");
    assert!(!author.is_unset());
    assert!(author.is_valid_ascii());
    assert!(author.fits_capacity());

    let comments = parsed.author_comments_typed();
    assert_eq!(comments.line(0), Some("First Comment Line"));
    assert_eq!(comments.line(1), Some("Second Line"));
    assert_eq!(comments.line(2), Some(""));
    assert_eq!(comments.line(3), Some(""));
    assert!(!comments.is_unset());
    assert!(comments.is_valid_ascii());
    assert!(comments.fits_capacity());
    assert_eq!(comments.joined(), "First Comment Line\nSecond Line");

    let job = parsed.job_name_typed();
    assert_eq!(job.as_str(), "Analytic Engine Render");
    assert!(!job.is_unset());

    let sw = parsed.software_id_typed();
    assert_eq!(sw.as_str(), "OxideAV TGA");
    assert!(!sw.is_unset());
}

#[test]
fn parse_helpers_round_trip_through_encoder() {
    let ext = ExtensionAreaInput {
        author_name: "Charles Babbage".to_string(),
        author_comment: [
            "Difference Engine #2".to_string(),
            String::new(),
            String::new(),
            String::new(),
        ],
        job_name: "JOB-001".to_string(),
        software_id: "TGA-Writer".to_string(),
        ..Default::default()
    };
    let base = one_pixel_base();
    let full = encode_tga_with_extension(&base, &ext).expect("with-extension");

    let author = parse_tga_author_name(&full).expect("author");
    assert_eq!(author.as_str(), "Charles Babbage");

    let comments = parse_tga_author_comments(&full).expect("comments");
    assert_eq!(comments.line(0), Some("Difference Engine #2"));
    assert_eq!(comments.joined(), "Difference Engine #2");

    let job = parse_tga_job_name(&full).expect("job");
    assert_eq!(job.as_str(), "JOB-001");

    let sw = parse_tga_software_id(&full).expect("sw");
    assert_eq!(sw.as_str(), "TGA-Writer");
}

#[test]
fn parse_helpers_return_none_for_tga_without_extension_area() {
    // Plain TGA 1.0 — no footer, no extension area.
    let plain = one_pixel_base();
    assert!(parse_tga_author_name(&plain).is_none());
    assert!(parse_tga_author_comments(&plain).is_none());
    assert!(parse_tga_job_name(&plain).is_none());
    assert!(parse_tga_software_id(&plain).is_none());
}

#[test]
fn typed_accessors_handle_empty_default_ext() {
    // Default ExtensionAreaInput leaves every ASCII field empty.
    let ext = ExtensionAreaInput::default();
    let base = one_pixel_base();
    let full = encode_tga_with_extension(&base, &ext).expect("with-extension");
    let parsed = oxideav_tga::parse_tga_extension_area(&full).expect("ext parsed");

    assert!(parsed.author_name_typed().is_unset());
    assert!(parsed.author_comments_typed().is_unset());
    assert!(parsed.job_name_typed().is_unset());
    assert!(parsed.software_id_typed().is_unset());
}

#[test]
fn typed_accessors_round_trip_at_capacity_cap() {
    // Fill each 40-char field exactly to its on-disk capacity, and
    // each 80-char comment line exactly to its on-disk capacity.
    let forty = "0123456789".repeat(4);
    assert_eq!(forty.len(), TGA_ASCII_FIELD_MAX_CHARS);
    let eighty = "0123456789".repeat(8);
    assert_eq!(eighty.len(), TGA_AUTHOR_COMMENT_LINE_MAX_CHARS);

    let ext = ExtensionAreaInput {
        author_name: forty.clone(),
        author_comment: [
            eighty.clone(),
            eighty.clone(),
            eighty.clone(),
            eighty.clone(),
        ],
        job_name: forty.clone(),
        software_id: forty.clone(),
        ..Default::default()
    };
    let base = one_pixel_base();
    let full = encode_tga_with_extension(&base, &ext).expect("with-extension");
    let parsed = oxideav_tga::parse_tga_extension_area(&full).expect("ext parsed");

    assert_eq!(parsed.author_name_typed().as_str(), forty);
    assert_eq!(parsed.job_name_typed().as_str(), forty);
    assert_eq!(parsed.software_id_typed().as_str(), forty);
    let c = parsed.author_comments_typed();
    for i in 0..TGA_AUTHOR_COMMENT_LINES {
        assert_eq!(c.line(i), Some(eighty.as_str()));
    }
    assert!(c.fits_capacity());
    assert!(parsed.author_name_typed().fits_capacity());
}
