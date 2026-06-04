//! Round 234 — typed Field 13 (Date/Time Stamp) and Field 15 (Job
//! Time) extension-area accessors plus matching parser conveniences.
//!
//! The TGA 2.0 extension area (§C.6) carries two small numeric
//! sub-fields the decoder has long parsed and the encoder has long
//! written byte-exactly, but which were only ever exposed in raw form
//! on [`TgaExtensionArea`]:
//!
//! * Field 13 — Date/Time Stamp (6 × SHORT: month/day/year/hour/
//!   minute/second). Carried as [`TgaTimestamp`] with raw `u16`
//!   fields only.
//! * Field 15 — Job Time (3 × SHORT: hours/minutes/seconds elapsed).
//!   Carried as the raw `(u16, u16, u16)` tuple
//!   [`TgaExtensionArea::job_time`].
//!
//! This round adds typed views matching the r227 pattern already in
//! place for §C.6.4 / §C.6.5 / §C.6.6 / §C.6.7:
//!
//! * [`TgaTimestamp`] gains [`TgaTimestamp::UNSET`] /
//!   [`TgaTimestamp::is_valid`] / [`TgaTimestamp::as_tuple`] /
//!   [`TgaTimestamp::from_tuple`] / [`TgaTimestamp::iso8601`].
//! * [`JobTime`] is brand-new — `(hours, minutes, seconds)` SHORT
//!   triple with [`JobTime::UNSET`] / [`JobTime::is_unset`] /
//!   [`JobTime::is_valid`] / [`JobTime::total_seconds`] /
//!   [`JobTime::as_f64_hours`] / [`JobTime::hms_string`].
//! * [`TgaExtensionArea::timestamp_typed`] /
//!   [`TgaExtensionArea::job_time_typed`] return the typed views.
//! * Convenience parsers [`parse_tga_timestamp`] /
//!   [`parse_tga_job_time`] walk the footer + extension area in one
//!   call.
//!
//! Tests cover:
//!
//! * Sentinel detection: every "field not set" value the spec calls
//!   out (all-zero sextuple, all-zero triple).
//! * Range checks (Field 13 month/day/year/hour/minute/second; Field
//!   15 minutes/seconds) — only in-range values match `is_valid`.
//! * Tuple round-trip through `as_tuple` / `from_tuple`.
//! * Iso-8601 formatting on a known timestamp; `None` on the unset
//!   sentinel.
//! * `JobTime::total_seconds` / `as_f64_hours` / `hms_string` on
//!   known inputs including the SHORT-cap hours boundary.
//! * `TgaExtensionArea::timestamp_typed` / `job_time_typed` and the
//!   convenience parsers `parse_tga_timestamp` / `parse_tga_job_time`
//!   round-trip through `encode_tga_with_extension`, and return
//!   `None` on a file that has no extension area.

use oxideav_tga::{
    encode_tga_uncompressed, encode_tga_with_extension, parse_tga_extension_area,
    parse_tga_job_time, parse_tga_timestamp, ExtensionAreaInput, JobTime, TgaTimestamp,
};

// ---------------------------------------------------------------------------
// TgaTimestamp (Field 13 — Date/Time Stamp)
// ---------------------------------------------------------------------------

#[test]
fn timestamp_default_matches_unset_constant() {
    let t = TgaTimestamp::default();
    assert_eq!(t, TgaTimestamp::UNSET);
    assert!(t.is_unset());
    // The all-zero sentinel is *not* in-range (month/day/year are
    // outside the spec's 1..=12 / 1..=31 / ≥1 ranges).
    assert!(!t.is_valid());
}

#[test]
fn timestamp_unset_const_is_all_zero_shorts() {
    let t = TgaTimestamp::UNSET;
    assert_eq!(t.as_tuple(), (0, 0, 0, 0, 0, 0));
    assert!(t.iso8601().is_none());
}

#[test]
fn timestamp_is_valid_accepts_spec_in_range_values() {
    let t = TgaTimestamp::from_tuple((1, 1, 1989, 0, 0, 0));
    assert!(t.is_valid());

    let t = TgaTimestamp::from_tuple((12, 31, 9999, 23, 59, 59));
    assert!(t.is_valid());

    let t = TgaTimestamp::from_tuple((6, 15, 2026, 12, 30, 45));
    assert!(t.is_valid());
}

#[test]
fn timestamp_is_valid_rejects_each_out_of_range_field() {
    // month: 0 and 13 are out of 1..=12
    assert!(!TgaTimestamp::from_tuple((0, 1, 2026, 0, 0, 0)).is_valid());
    assert!(!TgaTimestamp::from_tuple((13, 1, 2026, 0, 0, 0)).is_valid());

    // day: 0 and 32 are out of 1..=31
    assert!(!TgaTimestamp::from_tuple((1, 0, 2026, 0, 0, 0)).is_valid());
    assert!(!TgaTimestamp::from_tuple((1, 32, 2026, 0, 0, 0)).is_valid());

    // year: 0 is out of >=1
    assert!(!TgaTimestamp::from_tuple((1, 1, 0, 0, 0, 0)).is_valid());

    // hour: 24 is out of 0..=23
    assert!(!TgaTimestamp::from_tuple((1, 1, 2026, 24, 0, 0)).is_valid());

    // minute: 60 is out of 0..=59
    assert!(!TgaTimestamp::from_tuple((1, 1, 2026, 0, 60, 0)).is_valid());

    // second: 60 is out of 0..=59
    assert!(!TgaTimestamp::from_tuple((1, 1, 2026, 0, 0, 60)).is_valid());
}

#[test]
fn timestamp_tuple_round_trip_preserves_every_short() {
    let t = TgaTimestamp::from_tuple((11, 27, 1989, 14, 7, 33));
    assert_eq!(t.as_tuple(), (11, 27, 1989, 14, 7, 33));
    assert_eq!(t.month, 11);
    assert_eq!(t.day, 27);
    assert_eq!(t.year, 1989);
    assert_eq!(t.hour, 14);
    assert_eq!(t.minute, 7);
    assert_eq!(t.second, 33);
}

#[test]
fn timestamp_iso8601_formats_in_sortable_order() {
    let t = TgaTimestamp::from_tuple((1, 2, 1989, 3, 4, 5));
    assert_eq!(t.iso8601().as_deref(), Some("1989-01-02T03:04:05"));
}

#[test]
fn timestamp_iso8601_pads_each_field() {
    // All single-digit fields get padded to two digits; year stays
    // four wide.
    let t = TgaTimestamp::from_tuple((7, 8, 26, 9, 0, 1));
    assert_eq!(t.iso8601().as_deref(), Some("0026-07-08T09:00:01"));
}

#[test]
fn timestamp_iso8601_on_unset_is_none() {
    assert!(TgaTimestamp::UNSET.iso8601().is_none());
    assert!(TgaTimestamp::default().iso8601().is_none());
}

#[test]
fn timestamp_typed_round_trips_through_extension_area() {
    let ts = TgaTimestamp::from_tuple((6, 4, 2026, 5, 19, 0));
    let input = ExtensionAreaInput {
        timestamp: ts,
        ..Default::default()
    };
    let base = encode_tga_uncompressed(1, 1, &[0x11, 0x22, 0x33, 0xFF]).unwrap();
    let bytes = encode_tga_with_extension(&base, &input).unwrap();
    let parsed = parse_tga_extension_area(&bytes).unwrap();
    assert_eq!(parsed.timestamp, ts);
    assert_eq!(parsed.timestamp_typed(), ts);
    assert_eq!(parse_tga_timestamp(&bytes), Some(ts));
    assert_eq!(
        parsed.timestamp_typed().iso8601().as_deref(),
        Some("2026-06-04T05:19:00")
    );
}

#[test]
fn parse_tga_timestamp_is_none_without_extension_area() {
    let base = encode_tga_uncompressed(1, 1, &[0xAA, 0xBB, 0xCC, 0xFF]).unwrap();
    assert!(parse_tga_timestamp(&base).is_none());
}

#[test]
fn parse_tga_timestamp_returns_unset_when_extension_present_but_field_empty() {
    let base = encode_tga_uncompressed(1, 1, &[0, 0, 0, 0xFF]).unwrap();
    let bytes = encode_tga_with_extension(&base, &ExtensionAreaInput::default()).unwrap();
    let ts = parse_tga_timestamp(&bytes).unwrap();
    assert!(ts.is_unset());
    assert_eq!(ts, TgaTimestamp::UNSET);
}

// ---------------------------------------------------------------------------
// JobTime (Field 15 — Job Time)
// ---------------------------------------------------------------------------

#[test]
fn job_time_default_matches_unset_constant() {
    let j = JobTime::default();
    assert_eq!(j, JobTime::UNSET);
    assert!(j.is_unset());
    // The all-zero sentinel is also valid (zero is in range for every
    // field per the spec).
    assert!(j.is_valid());
    assert_eq!(j.total_seconds(), 0);
    assert!(j.as_f64_hours().abs() < f64::EPSILON);
    assert_eq!(j.hms_string(), "00:00:00");
}

#[test]
fn job_time_new_and_from_tuple_agree() {
    let a = JobTime::new(3, 14, 15);
    let b = JobTime::from_tuple((3, 14, 15));
    assert_eq!(a, b);
    assert_eq!(a.as_tuple(), (3, 14, 15));
    assert!(!a.is_unset());
}

#[test]
fn job_time_is_valid_rejects_out_of_range_minutes_or_seconds() {
    // Minutes >59 invalid
    assert!(!JobTime::new(1, 60, 0).is_valid());
    // Seconds >59 invalid
    assert!(!JobTime::new(1, 0, 60).is_valid());
    // Hours can take the full SHORT range — even 65 535 is valid
    assert!(JobTime::new(65_535, 59, 59).is_valid());
}

#[test]
fn job_time_total_seconds_combines_h_m_s() {
    assert_eq!(JobTime::new(1, 0, 0).total_seconds(), 3600);
    assert_eq!(JobTime::new(0, 1, 0).total_seconds(), 60);
    assert_eq!(JobTime::new(0, 0, 1).total_seconds(), 1);
    assert_eq!(
        JobTime::new(2, 30, 45).total_seconds(),
        2 * 3600 + 30 * 60 + 45
    );
}

#[test]
fn job_time_total_seconds_at_short_cap_fits_u32() {
    // Worst-case hour value carries through without overflow.
    let max = JobTime::new(65_535, 59, 59);
    // 65 535 × 3600 + 59 × 60 + 59 = 235 929 599.
    assert_eq!(max.total_seconds(), 65_535u32 * 3600 + 59 * 60 + 59);
    assert_eq!(max.total_seconds(), 235_929_599);
}

#[test]
fn job_time_as_f64_hours_matches_total_seconds_div_3600() {
    let j = JobTime::new(2, 30, 0);
    assert!((j.as_f64_hours() - 2.5).abs() < 1e-9);

    let j = JobTime::new(1, 1, 1);
    let expected = (3600.0 + 60.0 + 1.0) / 3600.0;
    assert!((j.as_f64_hours() - expected).abs() < 1e-9);
}

#[test]
fn job_time_hms_string_pads_each_field_to_two_digits() {
    assert_eq!(JobTime::new(0, 0, 0).hms_string(), "00:00:00");
    assert_eq!(JobTime::new(1, 23, 45).hms_string(), "01:23:45");
    assert_eq!(JobTime::new(9, 9, 9).hms_string(), "09:09:09");
}

#[test]
fn job_time_hms_string_allows_hours_to_exceed_two_digits() {
    // The hours field can occupy the full SHORT range; the formatter
    // honours that by widening rather than truncating.
    assert_eq!(JobTime::new(100, 5, 6).hms_string(), "100:05:06");
    assert_eq!(JobTime::new(65_535, 59, 59).hms_string(), "65535:59:59");
}

#[test]
fn job_time_typed_round_trips_through_extension_area() {
    let jt = (12u16, 34u16, 56u16);
    let input = ExtensionAreaInput {
        job_time: jt,
        ..Default::default()
    };
    let base = encode_tga_uncompressed(1, 1, &[0x10, 0x20, 0x30, 0xFF]).unwrap();
    let bytes = encode_tga_with_extension(&base, &input).unwrap();
    let parsed = parse_tga_extension_area(&bytes).unwrap();
    assert_eq!(parsed.job_time, jt);
    assert_eq!(parsed.job_time_typed(), JobTime::from_tuple(jt));
    assert_eq!(parse_tga_job_time(&bytes), Some(JobTime::from_tuple(jt)));
    let j = JobTime::from_tuple(jt);
    assert_eq!(j.total_seconds(), 12 * 3600 + 34 * 60 + 56);
    assert_eq!(j.hms_string(), "12:34:56");
    assert!(!j.is_unset());
    assert!(j.is_valid());
}

#[test]
fn parse_tga_job_time_is_none_without_extension_area() {
    let base = encode_tga_uncompressed(1, 1, &[0xAA, 0xBB, 0xCC, 0xFF]).unwrap();
    assert!(parse_tga_job_time(&base).is_none());
}

#[test]
fn parse_tga_job_time_returns_unset_when_extension_present_but_field_empty() {
    let base = encode_tga_uncompressed(1, 1, &[0, 0, 0, 0xFF]).unwrap();
    let bytes = encode_tga_with_extension(&base, &ExtensionAreaInput::default()).unwrap();
    let jt = parse_tga_job_time(&bytes).unwrap();
    assert!(jt.is_unset());
    assert_eq!(jt, JobTime::UNSET);
}

// ---------------------------------------------------------------------------
// Combined: both fields populated in the same file
// ---------------------------------------------------------------------------

#[test]
fn timestamp_and_job_time_coexist_in_the_same_extension_area() {
    let ts = TgaTimestamp::from_tuple((10, 31, 2025, 18, 45, 30));
    let jt = (4u16, 15u16, 7u16);
    let input = ExtensionAreaInput {
        timestamp: ts,
        job_time: jt,
        ..Default::default()
    };
    let base = encode_tga_uncompressed(1, 1, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
    let bytes = encode_tga_with_extension(&base, &input).unwrap();

    let parsed = parse_tga_extension_area(&bytes).unwrap();
    assert_eq!(parsed.timestamp_typed(), ts);
    assert!(parsed.timestamp_typed().is_valid());
    assert_eq!(
        parsed.timestamp_typed().iso8601().as_deref(),
        Some("2025-10-31T18:45:30")
    );
    assert_eq!(parsed.job_time_typed(), JobTime::from_tuple(jt));
    assert_eq!(
        parsed.job_time_typed().total_seconds(),
        4 * 3600 + 15 * 60 + 7
    );
    assert_eq!(parsed.job_time_typed().hms_string(), "04:15:07");

    assert_eq!(parse_tga_timestamp(&bytes), Some(ts));
    assert_eq!(parse_tga_job_time(&bytes), Some(JobTime::from_tuple(jt)));
}
