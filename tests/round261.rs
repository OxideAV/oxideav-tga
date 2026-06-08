//! Round 261 — typed [`TgaScanLineTable`] accessors.
//!
//! The §C.6.9 scan-line table has long been parsed (`parse_tga_scan_line_table`,
//! `TgaScanLineTable::parse`, `TgaScanLineTable::to_bytes`) but the
//! struct itself only exposed the raw `Vec<u32>` of offsets. This round
//! gives it the same typed-accessor surface already on `TgaFooter` (r257),
//! `TgaAsciiField` / `TgaAuthorComments` (r252), `TgaTimestamp` /
//! `JobTime` (r234), and `KeyColor` / `PixelAspectRatio` / `GammaValue` /
//! `SoftwareVersion` (r227):
//!
//! * [`TgaScanLineTable::EMPTY`] — empty-table sentinel.
//! * `Default` — equivalent to [`TgaScanLineTable::EMPTY`].
//! * [`TgaScanLineTable::new`] / [`TgaScanLineTable::with_capacity`] /
//!   `FromIterator<u32>` — construction.
//! * [`TgaScanLineTable::len`] / [`TgaScanLineTable::is_empty`] /
//!   [`TgaScanLineTable::is_unset`] — geometry / spec-shape predicates.
//! * [`TgaScanLineTable::get`] — bounds-checked row-offset accessor.
//! * [`TgaScanLineTable::byte_size`] — `len() * 4`, matches the
//!   `to_bytes()` output length.
//! * [`TgaScanLineTable::is_well_formed_within`] — every recorded
//!   offset addresses a byte inside a buffer of `input_len` bytes.
//! * [`TgaScanLineTable::is_strictly_increasing`] /
//!   [`TgaScanLineTable::is_strictly_decreasing`] — direction-of-save
//!   predicates (top-down vs bottom-up file orderings).
//! * [`TgaScanLineTable::row_range`] /
//!   [`TgaScanLineTable::row_bytes`] — derive `[start, end)` for row
//!   `y` using the next row's start (or a caller-supplied terminal byte
//!   for the last row) as the upper bound, and borrow the row's bytes
//!   straight out of the input.
//! * [`TGA_SCAN_LINE_OFFSET_BYTES`] — on-disk size of one entry (4).
//!
//! No changes to the on-disk wire format, the encoder, or the existing
//! `TgaScanLineTable::parse` / `to_bytes` / `parse_tga_scan_line_table`
//! surface — strictly additive.

use oxideav_tga::{TgaScanLineTable, TGA_SCAN_LINE_OFFSET_BYTES};

// ---------------------------------------------------------------------------
// Constants / sentinel / Default
// ---------------------------------------------------------------------------

#[test]
fn scan_line_offset_bytes_constant_matches_spec() {
    // Spec §C.6.9: "This table should contain a series of 4-byte
    // offsets. Each offset … is a four byte value for each scan line
    // in your image."
    assert_eq!(TGA_SCAN_LINE_OFFSET_BYTES, 4);
}

#[test]
fn empty_sentinel_has_zero_offsets() {
    let t = TgaScanLineTable::EMPTY;
    assert_eq!(t.len(), 0);
    assert!(t.is_empty());
    assert!(t.is_unset());
    assert_eq!(t.byte_size(), 0);
    assert!(t.to_bytes().is_empty());
}

#[test]
fn default_matches_empty_sentinel() {
    let t: TgaScanLineTable = TgaScanLineTable::default();
    assert_eq!(t, TgaScanLineTable::EMPTY);
    assert!(t.is_unset());
}

// ---------------------------------------------------------------------------
// new / with_capacity / FromIterator
// ---------------------------------------------------------------------------

#[test]
fn new_takes_offsets_verbatim() {
    let t = TgaScanLineTable::new(vec![10, 20, 30, 40]);
    assert_eq!(t.len(), 4);
    assert_eq!(t.offsets, vec![10, 20, 30, 40]);
    assert!(!t.is_empty());
    assert!(!t.is_unset());
    assert_eq!(t.byte_size(), 16);
}

#[test]
fn with_capacity_creates_empty_table_with_height_capacity() {
    let t = TgaScanLineTable::with_capacity(768);
    assert!(t.is_empty());
    assert_eq!(t.offsets.capacity(), 768);
}

#[test]
fn with_capacity_zero_height_is_empty() {
    let t = TgaScanLineTable::with_capacity(0);
    assert!(t.is_empty());
    assert_eq!(t.offsets.capacity(), 0);
}

#[test]
fn from_iter_collects_into_table() {
    let t: TgaScanLineTable = (100u32..105).collect();
    assert_eq!(t.offsets, vec![100, 101, 102, 103, 104]);
    assert_eq!(t.len(), 5);
}

#[test]
fn from_iter_empty_is_unset() {
    let t: TgaScanLineTable = std::iter::empty::<u32>().collect();
    assert!(t.is_unset());
}

// ---------------------------------------------------------------------------
// get / byte_size
// ---------------------------------------------------------------------------

#[test]
fn get_returns_some_for_valid_row() {
    let t = TgaScanLineTable::new(vec![100, 200, 300]);
    assert_eq!(t.get(0), Some(100));
    assert_eq!(t.get(1), Some(200));
    assert_eq!(t.get(2), Some(300));
}

#[test]
fn get_returns_none_past_end() {
    let t = TgaScanLineTable::new(vec![100, 200, 300]);
    assert_eq!(t.get(3), None);
    assert_eq!(t.get(99), None);
}

#[test]
fn get_on_empty_table_returns_none() {
    let t = TgaScanLineTable::EMPTY;
    assert_eq!(t.get(0), None);
}

#[test]
fn byte_size_matches_to_bytes_len() {
    let t = TgaScanLineTable::new(vec![10, 20, 30, 40, 50, 60, 70]);
    assert_eq!(t.byte_size(), t.to_bytes().len());
    assert_eq!(t.byte_size(), 7 * 4);
}

// ---------------------------------------------------------------------------
// is_well_formed_within
// ---------------------------------------------------------------------------

#[test]
fn well_formed_when_all_offsets_in_buffer() {
    let t = TgaScanLineTable::new(vec![18, 100, 200]);
    assert!(t.is_well_formed_within(300));
}

#[test]
fn not_well_formed_when_any_offset_at_or_past_end() {
    let t = TgaScanLineTable::new(vec![18, 100, 300]);
    // offset 300 points past the last byte of a 300-byte buffer.
    assert!(!t.is_well_formed_within(300));
    // Even one offset equal to len() fails — offsets address bytes,
    // and a 300-byte buffer's last valid byte index is 299.
    assert!(!t.is_well_formed_within(300));
    assert!(t.is_well_formed_within(301));
}

#[test]
fn well_formed_on_empty_table_is_trivially_true() {
    let t = TgaScanLineTable::EMPTY;
    assert!(t.is_well_formed_within(0));
    assert!(t.is_well_formed_within(100));
}

// ---------------------------------------------------------------------------
// is_strictly_increasing / is_strictly_decreasing
// ---------------------------------------------------------------------------

#[test]
fn empty_table_is_trivially_monotonic() {
    let t = TgaScanLineTable::EMPTY;
    assert!(t.is_strictly_increasing());
    assert!(t.is_strictly_decreasing());
}

#[test]
fn single_entry_is_trivially_monotonic() {
    let t = TgaScanLineTable::new(vec![42]);
    assert!(t.is_strictly_increasing());
    assert!(t.is_strictly_decreasing());
}

#[test]
fn top_down_save_is_strictly_increasing() {
    // Spec §C.6.9: "Each offset … should point to the start of the next
    // scan line, in the order that the image was saved (i.e., top down
    // or bottom up). The offset should be from the start of the file."
    // Top-down: row 0 first → offsets ascend with file position.
    let t = TgaScanLineTable::new(vec![100, 200, 300, 400, 500]);
    assert!(t.is_strictly_increasing());
    assert!(!t.is_strictly_decreasing());
}

#[test]
fn bottom_up_save_is_strictly_decreasing() {
    // Bottom-up: row 0 (the top row) appears *last* in the file, so
    // its offset is the largest; the recorded order (matching the
    // image's saved order) walks toward earlier file positions.
    let t = TgaScanLineTable::new(vec![500, 400, 300, 200, 100]);
    assert!(t.is_strictly_decreasing());
    assert!(!t.is_strictly_increasing());
}

#[test]
fn equal_consecutive_offsets_are_neither_strict_direction() {
    let t = TgaScanLineTable::new(vec![100, 100, 200]);
    assert!(!t.is_strictly_increasing());
    assert!(!t.is_strictly_decreasing());
}

#[test]
fn non_monotonic_table_is_neither_direction() {
    let t = TgaScanLineTable::new(vec![100, 300, 200, 400]);
    assert!(!t.is_strictly_increasing());
    assert!(!t.is_strictly_decreasing());
}

// ---------------------------------------------------------------------------
// row_range / row_bytes
// ---------------------------------------------------------------------------

#[test]
fn row_range_uses_next_row_start_for_upper_bound() {
    let t = TgaScanLineTable::new(vec![18, 100, 200, 300]);
    // Last row's end comes from the caller-supplied terminal.
    assert_eq!(t.row_range(0, 400), Some((18, 100)));
    assert_eq!(t.row_range(1, 400), Some((100, 200)));
    assert_eq!(t.row_range(2, 400), Some((200, 300)));
    assert_eq!(t.row_range(3, 400), Some((300, 400)));
}

#[test]
fn row_range_past_end_returns_none() {
    let t = TgaScanLineTable::new(vec![18, 100, 200, 300]);
    assert_eq!(t.row_range(4, 400), None);
    assert_eq!(t.row_range(99, 400), None);
}

#[test]
fn row_range_on_empty_table_returns_none() {
    let t = TgaScanLineTable::EMPTY;
    assert_eq!(t.row_range(0, 100), None);
}

#[test]
fn row_range_terminal_below_last_row_start_returns_none() {
    // Last row starts at 300 but the caller's terminal is 200 — the
    // range would be ill-defined (end < start), so we return None
    // instead of producing a nonsense slice.
    let t = TgaScanLineTable::new(vec![18, 100, 200, 300]);
    assert_eq!(t.row_range(3, 200), None);
}

#[test]
fn row_range_handles_equal_consecutive_offsets_as_zero_length_row() {
    // Pathological-but-defined: two rows pointing at the same offset
    // is a zero-length row (degenerate but addressable). row_range
    // accepts it; row_bytes returns an empty slice.
    let t = TgaScanLineTable::new(vec![100, 100, 200]);
    assert_eq!(t.row_range(0, 200), Some((100, 100)));
    let buf = vec![0u8; 200];
    assert_eq!(t.row_bytes(&buf, 0, 200), Some(&[][..]));
}

#[test]
fn row_bytes_borrows_slice_for_valid_range() {
    let mut buf = vec![0u8; 60];
    // Lay down per-row sentinel bytes so we can prove the slice is
    // borrowed from the correct file range.
    for (i, b) in buf.iter_mut().enumerate() {
        *b = i as u8;
    }
    let t = TgaScanLineTable::new(vec![10, 20, 30, 40]);
    let r0 = t.row_bytes(&buf, 0, 50).unwrap();
    assert_eq!(r0, &buf[10..20]);
    let r3 = t.row_bytes(&buf, 3, 50).unwrap();
    assert_eq!(r3, &buf[40..50]);
}

#[test]
fn row_bytes_rejects_range_past_buffer_end() {
    let buf = vec![0u8; 60];
    let t = TgaScanLineTable::new(vec![10, 20, 30, 40]);
    // Last-row terminal is 70 but buf is only 60 bytes long — reject.
    assert_eq!(t.row_bytes(&buf, 3, 70), None);
}

#[test]
fn row_bytes_returns_none_when_row_range_is_none() {
    let buf = vec![0u8; 60];
    let t = TgaScanLineTable::new(vec![10, 20, 30, 40]);
    assert_eq!(t.row_bytes(&buf, 4, 50), None);
}

// ---------------------------------------------------------------------------
// Parse / to_bytes round-trip (existing surface, re-pinned for completeness)
// ---------------------------------------------------------------------------

#[test]
fn parse_round_trips_through_to_bytes() {
    // Build a self-contained on-disk table inside a sized buffer at a
    // non-zero file offset (the existing contract rejects offset == 0
    // as the extension-area's "no table" sentinel) and parse it back.
    let table = TgaScanLineTable::new(vec![0x0000_0012, 0xDEAD_BEEF, 0x0123_4567, 0xFFFF_FFFE]);
    let bytes = table.to_bytes();
    assert_eq!(bytes.len(), table.byte_size());
    assert_eq!(bytes.len(), 16);
    let mut on_disk = vec![0u8; 32];
    on_disk[16..32].copy_from_slice(&bytes);
    let parsed = TgaScanLineTable::parse(&on_disk, 16, 4).unwrap();
    assert_eq!(parsed, table);
}

#[test]
fn parse_rejects_zero_height() {
    let buf = vec![0u8; 16];
    assert_eq!(TgaScanLineTable::parse(&buf, 0, 0), None);
}

#[test]
fn parse_rejects_zero_offset() {
    let buf = vec![0u8; 16];
    // Per the existing contract, offset == 0 returns None (the
    // extension area's `scan_line_offset` of 0 means "no table").
    assert_eq!(TgaScanLineTable::parse(&buf, 0, 4), None);
}

#[test]
fn parse_rejects_truncated_buffer() {
    // 3 entries needed, only 8 bytes (2 entries' worth) available.
    let buf = vec![0u8; 8];
    assert_eq!(TgaScanLineTable::parse(&buf, 0, 3), None);
}
