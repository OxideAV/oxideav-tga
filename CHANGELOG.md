# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Round 316: §C.2 Image Descriptor data-storage interleaving flag (Field 5.6,
  bits 7-6) typed `Interleaving` view + `parse_tga_interleaving` reader

## [0.0.2](https://github.com/OxideAV/oxideav-tga/compare/v0.0.1...v0.0.2) - 2026-06-15

### Fixed

- *(decode)* apply §C.2 Color Map Origin (First Entry Index) in palette lookup

### Other

- Round 311: §C.2 attribute-bit count typed view + header-local alpha resolver
- Round 304: public §C.2 Color Map extraction (parse_tga_color_map + TgaColorMap)
- tga r297: §C.6.10 Postage Stamp Image (Field 26) typed dimension view
- Round 289: bit-identical §C.5 RLE run-packet decode optimization
- Round 280: compute + random-access §C.6.9 scan-line tables
- Round 269: typed TgaDeveloperArea / TgaDeveloperTag accessors (spec §C.7)
- Round 261: typed TgaScanLineTable accessors
- Round 257: typed TgaFooter accessors + canonical to_bytes serialiser
- Round 252: typed Field 11/12/14/16 ASCII extension-area accessors
- drop release-plz.toml — use release-plz defaults across the workspace
- Round 234: typed Field 13 timestamp + Field 15 job-time accessors
- Round 227: typed §C.6.4/§C.6.5/§C.6.6/§C.6.7 accessors + gamma application
- Round 221: TARGA-32 vs ARGB-32 alpha-channel fallback (opt-in resolver)
- Round 213: criterion benchmark harness for decode + encode hot paths
- Round 207: cargo-fuzz encode_roundtrip target
- round 8 part 2: Image Identification Field round-trip (spec §3.3 / §C.3)
- honour image-descriptor bit 4 (right-to-left columns)
- Round 7 fix: postage-stamp validate_depth gate
- Round 7: cargo-fuzz decode_tga harness + daily 30-min CI run
- apply attributes-type alpha interpretation (spec §C.6.13)
- Round 5: colour-correction table application + interleaved on-disk layout fix
- round 4: developer area + CCT + scan-line table + typed AttributesType
- round 3: RGB24-input standalone entry points + hardening tests
- fix rustdoc broken intra-doc link to TgaError
- clean round-2 [Unreleased] above 0.0.1 release notes
- Round 2: type 1/3/9/11 writers + TGA 2.0 extension area + thumbnail
- release v0.0.1

### Added

- Round 311 (§C.2 Image Descriptor attribute-bit count — typed view +
  header-local alpha resolver): the image-descriptor byte's low four bits
  (Field 5.6, bits 3-0) "specify the number of attribute bits per pixel …
  designated as Alpha Channel bits". The crate exposed the raw count
  (`TgaHeader::alpha_bits`) but never gave it a typed view and never
  *applied* it at decode time. New `AttributeBits` struct wraps the count
  with `NONE` / `from_descriptor` / `new` (four-bit masked) / `is_none` /
  `has_alpha` / `is_canonical_for_depth` (8 for 32 bpp, 1 for 16 bpp,
  0 for the alpha-less depths), reachable via the new
  `TgaHeader::attribute_bits()` accessor and the public
  `parse_tga_attribute_bits(input)` reader (works on TGA 1.0 — the field
  is in the fixed header, unlike the extension-area `AttributesType`).
  New `resolve_alpha_from_descriptor(input, &mut image)` is the
  apply-side: the spec states a Field 24 value of `0` ("no Alpha data
  included") means "bits 3-0 of field 5.6 should also be set to zero", so
  for an RGBA image whose header declares zero attribute bits the resolver
  forces every alpha byte to `0xFF` (`force_opaque`) and otherwise leaves
  the decoded alpha untouched, returning the `AttributeBits` read. It is
  the header-only counterpart to `resolve_alpha_with_targa32_fallback`:
  it consults the header bits alone, never reads the extension area, and
  never inspects pixel values, so it acts even on a non-zero-but-
  uninitialised alpha plane. New `TGA_ATTRIBUTE_BITS_MAX` (15) constant.
  The `decode_tga` fuzz harness now drives `parse_tga_attribute_bits` and
  `resolve_alpha_from_descriptor`. New `tests/round311.rs` (10 tests)
  covers the typed view + spec predicates, the header reader (incl.
  truncation), the raw-vs-typed agreement, and the resolver's
  force-opaque / honour-declared-alpha / acts-on-nonzero-alpha /
  truncated-no-op branches.

- Round 304 (§C.2 Color Map Specification — public palette extraction):
  new `parse_tga_color_map` reads a TGA file's on-disk color map (header
  bytes 3-7 — Color Map Origin, Color Map Length, Color Map Entry Size)
  into a typed `TgaColorMap` (`first_index` / `entry_size` / `entries`
  plus `empty` / `len` / `is_empty` / origin-aware `get(idx)`). It reads
  only the header + color-map block, so it surfaces the palette of a
  colour-mapped file **and** of a **Data Type 0 (No Image Data)**
  palette-only file (a header + color map with no pixel array) that
  `parse_tga` rejects as having no image. Entries are de-interleaved into
  straight RGBA exactly as the decoder expands the palette internally
  (15/16-bit A1R5G5B5, 24-bit BGR, 32-bit BGRA); the Color Map Origin is
  recorded so a logical pixel index `idx` resolves to
  `entries[idx - first_index]`. Returns `None` when the Color Map Type
  byte is `0`. The `decode_tga` fuzz harness now drives the new helper.
  New `tests/round304.rs` (9 tests) covers the typed view, the
  encoder-palette round-trip, the type-0 palette-only case, a non-zero
  Color Map Origin, and the truncation / malformed-length / truncated-
  header error paths.

### Performance

- Round 289 (§C.5 RLE run-packet decode — bit-identical optimization):
  profiling the `decode/rle_24bpp_runs` criterion scenario showed a
  run-heavy RLE file decoding no faster than a same-size noise file —
  the run-packet path called `emit_pixel` once per output pixel, paying
  the full `image_type` / pixel-depth match dispatch for every pixel in
  a run even though all pixels in a run are byte-identical.
  `decode_rle_pixels` now expands the source pixel exactly once through
  `emit_pixel`, then replicates the emitted output bytes for the
  remaining `count - 1` pixels (by doubling the freshly-written tail in
  place via `Vec::extend_from_within`). The decoded output is unchanged:
  same bytes, same order. New `tests/round289.rs` pins the bit-identical
  contract against a deliberately naive pixel-by-pixel reference across
  all six RLE image-type/depth combinations — true-colour 24/32/16 bpp,
  8-bit grayscale, colour-mapped (type 9), and an alternating
  run/raw-packet stress case — with runs spanning the 1-pixel minimum
  and 128-pixel maximum. Measured on the run-heavy decode benchmark the
  change is roughly a 70 % wall-clock reduction (≈123 µs → ≈37 µs on the
  development host, 256×256 frame); raw-packet (count == 1) decode is
  unaffected. No wire-format, public-API, or encoder change. Suite
  357 → 363 tests; standalone + default-feature builds both green.

### Added

- Round 297 (§C.6.10 Postage Stamp Image / Field 26 — typed dimension
  view): the embedded-thumbnail decoder (`parse_tga_postage_stamp`,
  r7) already read the stamp's two leading on-disk size bytes ("The
  first byte of the postage stamp image specifies the X size of the
  stamp in pixels, the second byte … the Y size") while decoding the
  full pixel payload, but those dimensions had no typed view of their
  own — the only field in the extension area's `_typed` family without
  one. New `PostageStamp` struct wraps the `(width, height)` byte pair
  with `UNSET` / `new` / `as_tuple` / `from_tuple` / `is_unset` (either
  axis zero) / `pixel_count` (promoted to `u32` so the 255×255 worst
  case can't overflow) / `within_recommended_size` (both edges ≤ 64,
  Truevision's "does not recommend stamps larger than 64 x 64 pixels"
  guidance) / `clipped_to_recommended` (clamps each axis to the 64-pixel
  cap — the recommendation is advisory, not a hard limit; the full
  decoder still reads oversized stamps unchanged). New
  `parse_tga_postage_stamp_dimensions(input)` reads just the two size
  bytes from a real file (`None` for a TGA 1.0 file / no-stamp file,
  `Err` for an out-of-range offset, never a panic) so a caller can
  inspect the stamp geometry without decoding its pixels; its reported
  geometry agrees with `parse_tga_postage_stamp`'s. New
  `TGA_POSTAGE_STAMP_RECOMMENDED_MAX` (64) and `TGA_POSTAGE_STAMP_MAX`
  (255, the single-byte axis ceiling) constants expose the spec's
  dimension bounds. New `tests/round297.rs` (10 tests) pins the
  pure-data helpers, the against-real-files dimension reader (no-stamp /
  embedded / oversized / out-of-range-offset), and the agreement with
  the full decoder. No wire-format or encoder change; purely additive.
- Round 280 (§C.6.9 scan-line tables — compute + random row access): the
  scan-line table could be parsed (`parse_tga_scan_line_table`), typed
  (`TgaScanLineTable`, r261), serialised, and embedded on encode, but the
  crate could neither *build* one from a file's own pixel data nor *use*
  one for the random access the spec created it for ("to make random
  access of compressed images easy", "to allow 'giant picture' access in
  smaller 'chunks'"). Two new public functions close that:
  - `compute_tga_scan_line_table(input)` derives the table from any
    supported file — pure offset arithmetic for uncompressed types
    1/2/3, a single §C.5 packet walk for RLE types 9/10/11. Entries are
    in saved order (top-down or bottom-up), from-file-start, one per
    row, exactly the Field 25 shape. An RLE file in which a packet
    spans a scan-line boundary is rejected with
    `TgaError::Unsupported`: the spec's packet rule (packets "should
    never encode pixels from more than one scan line") is precisely
    what makes the table well-defined, and a file that breaks it has
    rows that don't start on packet boundaries — `parse_tga` still
    decodes such files whole. Offsets that would not fit the table's
    4-byte field are rejected rather than truncated.
  - `parse_tga_scan_line(input, &table, index)` decodes exactly one row
    through a table (file-supplied or computed) without touching any
    preceding row, returning a 1-pixel-tall `TgaImage` in the same
    normalised format `parse_tga` produces (palette lookup, BGR→RGBA
    swap, A1R5G5B5 expansion, descriptor-bit-4 column mirroring all
    applied; `index` is in the table's saved order, so bottom-up files
    address display row `height − 1 − y` at entry `y`). Out-of-range
    indices, offsets past the end of the input, and rows whose packets
    overrun the row are rejected with returned errors.
  The duplicated header→Image-ID→colour-map walk in `parse_tga` /
  `parse_tga_postage_stamp` was factored into a shared internal helper
  (also used by both new functions); as a side effect the postage-stamp
  path now rejects a colour-mapped parent with `cmap_type == 0` up
  front instead of failing later at pixel-emit time. New
  `tests/round280.rs` pins 9 cases: arithmetic table shape +
  row-vs-whole-decode equality across all six writer-covered image
  types (uncompressed/RLE × true-colour/palette/grayscale, run-heavy
  and raw-heavy RLE), saved-order semantics on a hand-built bottom-up
  file, column mirroring on a right-to-left file, boundary-spanning
  packet rejection (whole-file decode still accepted), hostile
  index/offset rejection, and an end-to-end compute → embed via
  `encode_tga_with_extension` → re-parse → random-access round trip.
  The `decode_tga` fuzz harness now also drives both new functions
  (computed table + attacker-controlled file-supplied table) on every
  input.

### Fixed

- Round 273 (Color Map Origin / First Entry Index application): the §C.2
  Color Map Specification's "index of first color map entry" (header
  bytes 3-4, parsed into `TgaHeader::cmap_first` since round 1) was read
  but never applied during palette lookup. The on-disk colour map stores
  `cmap_length` entries that correspond to *logical* palette indices
  starting at the origin, so an image index `idx` addresses on-disk entry
  `idx - cmap_first`. The decoder previously indexed the palette directly
  by `idx`, so any colour-mapped file with a non-zero Color Map Origin
  decoded with an off-by-origin palette error (or was spuriously rejected
  as out of range when `idx >= cmap_length`). `emit_pixel` now subtracts
  the origin for both the uncompressed (type 1) and RLE (type 9)
  colour-mapped paths, and for the postage-stamp colour-mapped path that
  shares the parent header; indices below the origin (`idx < cmap_first`)
  are rejected via a checked subtraction rather than wrapping. Origin-0
  files — everything this crate's own encoder writes — are bit-exact
  unchanged (`idx - 0 == idx`), so no existing round-trip is affected.
  New `tests/round273.rs` pins 7 cases: origin-0 regression, non-zero
  origin offset on type 1, below-origin + past-(origin+length) rejection
  on type 1, RLE (type 9) origin application + below-origin rejection,
  and a high-origin case at the top of the 8-bit index space. Suite
  341 → 348 tests; standalone (no `registry` feature) + default-feature
  builds both green. No on-disk wire-format or encoder change.

### Added

- Round 269 (typed `TgaDeveloperArea` / `TgaDeveloperTag` accessors):
  the §C.7 developer-area tag directory the decoder has long parsed and
  the encoder has long written now carries typed surfaces matching the
  r227 / r234 / r252 / r257 / r261 pattern — the last parsed TGA 2.0
  structure without one. `TgaDeveloperTag` gains `new` / `as_tuple` /
  `from_tuple` (tuple round-trip in on-disk TAG, OFFSET, FIELD SIZE
  order); the §C.7 tag-id range classifiers `is_developer_use`
  (`0..=32767`) and `is_truevision_reserved` (`32768..=65535`);
  `is_marker` (offset-0 / no-payload record — the shape
  `DeveloperTagInput` writes for an empty payload and for which
  `payload` returns `None`); `is_well_formed_within(input_len)`
  (non-marker payload range fits the buffer, mirroring `parse`'s
  per-record rejection rule); and `to_bytes()` emitting the on-disk
  10-byte record (LE `u16` tag, LE `u32` offset, LE `u32` size).
  `TgaDeveloperArea` gains `EMPTY` (empty-directory sentinel — the
  in-memory shape for the spec's "If the Offset is ZERO (binary zero)
  no directory and no Developer Area fields exist") + `Default` (==
  `EMPTY`); construction via `new(tags)` /
  `FromIterator<TgaDeveloperTag>`; geometry / sentinel via `len` /
  `is_empty` / `is_unset` / `directory_byte_size` (the spec's
  `(NUMBER_OF_TAGS_IN_THE_DIRECTORY * 10) + 2` formula, ==
  `to_bytes().len()`); positional + by-id lookup via `get(i)` /
  `find(tag_id)` (first match in directory order — spec §C.7: "The
  TAGS may appear in any order in the directory") / `contains(tag_id)`;
  whole-directory `is_well_formed_within(input_len)`; and `to_bytes()`
  serialising the count-prefixed directory bit-exactly as `parse`
  reads it (and bit-exactly as `encode_tga_with_extension` writes it).
  New constants `TGA_DEVELOPER_TAG_BYTES` (= 10, spec: "each set is 10
  bytes in size (1 short, 2 longs)") and
  `TGA_DEVELOPER_DIRECTORY_HEADER_BYTES` (= 2). New `tests/round269.rs`
  pins 32 cases (constants / construction / tuple round-trip / range
  classification at both boundaries + full-domain partition / marker
  shapes / per-record + whole-directory well-formedness accept +
  reject / 10-byte LE record layout / count-prefixed directory layout
  + empty directory / `to_bytes` ↔ `parse` round-trip with a real
  payload / end-to-end encoder-written developer area through the
  typed lookup surface including bit-exact directory-byte comparison).
  Suite 309 → 341 tests; standalone (no `registry` feature) +
  default-feature builds both green. No changes to the on-disk wire
  format, the encoder, the registered decoder, or the existing
  `TgaDeveloperArea::parse` / `payload` / `parse_tga_developer_area`
  surface — strictly additive.

### Fixed

- Round 269: the §C.7 tag-id reservation ranges were documented
  *inverted* in two doc comments (`TgaDeveloperTag` in `types.rs`,
  `DeveloperTagInput::tag_id` in `encoder.rs`), claiming the low range
  was Truevision-reserved. The spec says the opposite: "Values from
  0 - 32767 are available for developer use, while values from
  32768 - 65535 are reserved for Truevision." Doc-only fix — no code
  ever branched on the ranges; the new `is_developer_use` /
  `is_truevision_reserved` classifiers and their boundary tests pin
  the correct direction.

### Added

- Round 261 (typed `TgaScanLineTable` accessors): the §C.6.9 scan-line
  table the decoder has long parsed and the encoder has long re-written
  byte-exactly now carries typed surfaces matching the r227 / r234 /
  r252 / r257 pattern already in place for `TgaFooter`,
  `TgaAsciiField` / `TgaAuthorComments`, `TgaTimestamp` / `JobTime`,
  and the numeric extension-area fields. `TgaScanLineTable::EMPTY` is
  the empty-table sentinel (`offsets.len() == 0`) and `Default` is
  equivalent. Construction: `new(offsets)` /
  `with_capacity(height: u16)` / `FromIterator<u32>`. Geometry +
  sentinel predicates: `len` / `is_empty` / `is_unset` (matches the
  spec's "no scan-line table present" shape — same byte-for-byte as
  `is_empty`, named for symmetry with the other typed accessors) /
  `byte_size` (`len() * 4`, matches `to_bytes().len()`). Bounds-checked
  row-offset accessor `get(y) -> Option<u32>`. Buffer-bounds sanity
  check `is_well_formed_within(input_len: usize) -> bool` — every
  recorded offset addresses a byte strictly inside an `input_len`-byte
  buffer; empty table trivially satisfies. Direction-of-save predicates
  `is_strictly_increasing` (top-down save — row 0 appears first in the
  file, offsets ascend) and `is_strictly_decreasing` (bottom-up save —
  the spec §C.6.9 alternative). Range / byte borrow helpers
  `row_range(y, terminal: u32) -> Option<(u32, u32)>` (derives `[start,
  end)` where `end` is the next row's recorded offset or, for the last
  row, the caller-supplied terminal) and `row_bytes(input, y, terminal)
  -> Option<&[u8]>` (borrows the row's bytes straight out of the input).
  New constant `TGA_SCAN_LINE_OFFSET_BYTES` (= 4) exposes the on-disk
  per-entry size. New `tests/round261.rs` pins 33 cases (sentinel /
  default / construction / geometry / `get` bounds / `byte_size`
  ↔ `to_bytes` length / `is_well_formed_within` true and false /
  monotonic predicates on empty / single-entry / top-down / bottom-up /
  equal-consecutive / non-monotonic shapes / `row_range` on valid +
  past-end + empty + below-terminal + zero-length-row inputs /
  `row_bytes` slice borrow + past-buffer rejection + invalid-range
  rejection / parse round-trip through `to_bytes` / parse rejects
  zero-height / zero-offset / truncated). Suite 276 → 309 tests;
  standalone (no `registry` feature) + default-feature builds both
  green. No changes to the on-disk wire format, the encoder, the
  registered decoder, or the existing `TgaScanLineTable::parse` /
  `to_bytes` / `parse_tga_scan_line_table` surface — strictly additive.

- Round 257 (typed `TgaFooter` accessors + canonical serialiser): the
  26-byte TGA 2.0 trailer (spec §C.4) gains the same typed-accessor
  pattern already in place for the extension-area numeric / ASCII
  fields. `TgaFooter::UNSET` is the all-zero sentinel (marker-only
  v2 footer with no metadata sections); `TgaFooter::new(ext, dev)` /
  `as_tuple` / `from_tuple` provide tuple round-trip in
  `(extension_area_offset, developer_directory_offset)` order;
  `has_extension_area` / `has_developer_area` apply the spec's
  zero-means-absent convention; `is_marker_only` recognises a v2 file
  flagged with no metadata; `offsets_within(input_len)` is a
  geometry-only sanity check that both non-zero offsets fit inside a
  buffer of `input_len` bytes leaving at least the trailing 26-byte
  footer unconsumed; `to_bytes` emits the canonical 26-byte trailer
  (LE `u32` extension offset, LE `u32` developer offset, 18-byte
  `"TRUEVISION-XFILE.\0"` signature) and is the bit-exact inverse of
  `parse_tga_footer`. `TgaFooter` also gains `Default` (== `UNSET`).
  Suite 250 → 276 tests; standalone (no `registry` feature) +
  default-feature builds both green. No changes to the on-disk wire
  format, the existing encoder, or the registered decoder; the typed
  accessors are additive surface on the already-published struct, and
  the serialiser is independent of the encoder's existing inline
  footer emission in `encode_tga_with_extension`.

- Round 252 (typed Field 11 / 12 / 14 / 16 extension-area accessors):
  the four ASCII fields the decoder has long parsed and the encoder
  has long re-written byte-exactly now carry typed surfaces matching
  the r227 / r234 pattern already in place for the numeric
  extension-area fields. `TgaAsciiField` is new — wraps the parsed
  payload of Author Name (Field 11, 41 bytes), Job Name/ID
  (Field 14, 41 bytes), and Software ID (Field 16, 41 bytes) with
  `new` / `from_borrowed` / `as_str` / `into_inner` / `char_len` /
  `is_unset` (empty / NUL-only / blanks-and-NULs sentinel per the
  spec's "fill with NULs or a series of blanks terminated by a null"
  recommendation) / `is_valid_ascii` (every byte printable ASCII
  `0x20..=0x7E` per the spec's "printable ASCII only" guidance) /
  `fits_capacity` (40-character on-disk slot) / `trimmed` (leading +
  trailing ASCII whitespace stripped). `TgaAuthorComments` is new —
  wraps the four 81-byte lines of Author Comments (Field 12,
  324 bytes total) with `new` / `from_strs` / `empty` / `line(i)` /
  `is_unset` / `is_valid_ascii` / `fits_capacity` (80-character
  per-line cap) / `joined` (newline-separated, trailing blank lines
  dropped). `TgaExtensionArea::author_name_typed` /
  `author_comments_typed` / `job_name_typed` / `software_id_typed`
  return the typed views from existing raw fields. Convenience
  parsers `parse_tga_author_name` / `parse_tga_author_comments` /
  `parse_tga_job_name` / `parse_tga_software_id` walk the footer +
  extension area in one call and return `None` on TGA 1.0 files.
  New per-field constants `TGA_ASCII_FIELD_MAX_CHARS` (40),
  `TGA_AUTHOR_COMMENT_LINES` (4), `TGA_AUTHOR_COMMENT_LINE_BYTES`
  (81), `TGA_AUTHOR_COMMENT_LINE_MAX_CHARS` (80). Suite 228 → 250
  tests; standalone (no `registry` feature) + default-feature builds
  both green. No changes to the on-disk wire format, the encoder, or
  any pre-existing decoder return type — strictly additive.

- Round 234 (typed Field 13 + Field 15 extension-area accessors):
  the Date/Time Stamp (Field 13) and Job Time (Field 15) sub-fields
  the decoder has long parsed and the encoder has long re-written
  byte-exactly now carry typed surfaces matching the r227 pattern
  already in place for §C.6.4 / §C.6.5 / §C.6.6 / §C.6.7. `TgaTimestamp`
  gains a `UNSET` constant (all-zero sentinel), `is_valid` (month
  ∈ 1..=12, day ∈ 1..=31, year ≥ 1, hour ∈ 0..=23, minute ∈ 0..=59,
  second ∈ 0..=59 per the spec's named ranges; the unset sentinel
  is not in-range so callers can split "unset vs valid"), `as_tuple`
  / `from_tuple` (six-SHORT round-trip in on-disk order), and
  `iso8601` returning `Some("YYYY-MM-DDTHH:MM:SS")` on a set
  timestamp, `None` on the all-zero sentinel. `JobTime` is a new
  typed wrap of the on-disk `(hours, minutes, seconds)` SHORT
  triple (the encoder's existing `ExtensionAreaInput::job_time`
  shape): `UNSET` / `new` / `from_tuple` / `as_tuple` / `is_unset`
  / `is_valid` (minutes and seconds ∈ 0..=59; hours covers the full
  SHORT range per the spec's "0 - 65535") / `total_seconds` (`u32`,
  the SHORT-cap maximum 65 535 × 3600 + 59 × 60 + 59 = 235 929 599
  fits with room) / `as_f64_hours` (total seconds divided by 3600,
  useful for billing reports) / `hms_string` (zero-padded
  `"HH:MM:SS"` with hours widening past two digits at the SHORT
  cap). `TgaExtensionArea::timestamp_typed` and `job_time_typed`
  return the typed views from existing raw fields. Convenience
  parsers `parse_tga_timestamp` / `parse_tga_job_time` walk the
  footer + extension area in one call and return `None` on TGA 1.0
  files. Suite 205 → 228 tests; standalone (no `registry` feature)
  + default-feature builds both green. No changes to the on-disk
  wire format, the encoder, or any pre-existing decoder return
  type — strictly additive.

- Round 227 (typed §C.6.4 / §C.6.5 / §C.6.6 / §C.6.7 accessors + gamma
  application): the four small numeric extension-area fields that the
  decoder has long parsed and the encoder has long re-written
  byte-exactly now carry typed views in the same shape as the
  pre-existing `AttributesType` (§C.6.13) and `TgaColourCorrectionTable`
  (§C.6.8) surfaces. `KeyColor` wraps the §C.6.4 `[A, R, G, B]` quadruple
  (`from_argb` / `to_argb` / `as_rgba8` for framework-order chroma-key
  comparison / `is_unset` / `has_alpha`). `PixelAspectRatio` wraps the
  §C.6.5 SHORT pair (`new` / `as_tuple` / `is_unset` for `(0, 0)` or
  zero-denominator / `is_square` / `as_f32` / `corrected_display_height`
  / `corrected_display_width` — the last two helpers rationalise the
  square-display-pixel resampling dimensions documented inline in the
  spec). `GammaValue` wraps the §C.6.6 SHORT pair with the same sentinel
  / identity / apply semantics as `AttributesType`: `is_unset` covers any
  zero-denominator pair (the spec's "totally ignore" signal), `is_identity`
  covers any equal non-zero pair (the spec's "uncorrected image" signal),
  `as_f32` is `None` on unset, and `apply_to_channel8` / `apply_to_rgba8`
  / `apply_to_image` raise each colour channel through the `y = (x/255)
  ^ gamma × 255` power curve (rounded to nearest, clamped to 0..=255).
  Alpha bytes are never touched; RGBA / Rgb24 / Gray8 all walk through
  consistently; unset / identity / malformed gammas (non-finite, ≤ 0)
  are bit-exact no-ops so an arbitrary file never blows up downstream
  compositing. `SoftwareVersion` wraps the §C.6.7 `(SHORT, BYTE-as-char)`
  pair (`is_unset` for the `(0, ' ')` spec sentinel; `as_f32` is
  `number_times_100 / 100.0`, e.g. `Some(1.17)` for the spec's `(117,
  'b')` example). `TgaExtensionArea` gains
  `{key_color_typed, pixel_aspect_ratio_typed, gamma_typed,
  software_version_typed}` accessors that consume the existing raw
  fields. Four new convenience parser helpers — `parse_tga_key_color` /
  `parse_tga_pixel_aspect_ratio` / `parse_tga_gamma` /
  `parse_tga_software_version` — read the typed view straight from a
  full TGA file (footer + extension-area walk in one call) and return
  `None` when the file has no TGA 2.0 footer or no extension area.
  Tests added in `tests/round227.rs`: 33 cases pinning the spec
  sentinels, the round-trip through `TgaExtensionArea`, every
  `GammaValue::apply_*` branch (identity preserves bytes; gamma=2.0
  maps mid-gray 128 → 64 within ½-LSB; alpha preserved; pathological
  values are no-ops), the corrected-display dimension arithmetic for
  4:3 / unset / square cases, and all four parser convenience helpers
  on both extension-bearing and TGA-v1 files. Suite 172 → 205 tests,
  green standalone and with the default `registry` feature on. No
  changes to the on-disk wire format, the encoder, or any pre-existing
  decoder return type.

- Round 221 (TARGA-32 vs ARGB-32 fallback): the long-standing real-world
  ambiguity around 32-bpp TGA files written by legacy paint applications
  that left `attributes_type` unset and the alpha channel uninitialised
  now has an opt-in resolver. The default decode path is unchanged — a
  caller that decodes such a file via `parse_tga` still gets verbatim
  on-disk bytes, alpha included. The new
  `resolve_alpha_with_targa32_fallback(input: &[u8], &mut TgaImage)`
  entry point composes the two existing pieces of state — the file's
  declared `AttributesType` (if any) and the decoded buffer — and:
  (1) if the file declares an `AttributesType` (TGA 2.0 footer +
  extension area), apply it (`AttributesType::apply_to_image` semantics:
  `PremultipliedAlpha` un-premultiplied, `NoAlpha` / `UndefinedIgnore`
  forced opaque, the rest pass through) and return `Some(attrs)`;
  (2) otherwise check `image.all_alpha_zero()` and, when every alpha
  byte is `0`, call `image.force_opaque()` so the picture displays as
  opaque rather than fully-transparent black — returning `None` to
  disambiguate which branch ran. The convention is the one documented
  in `docs/image/tga/README.md` ("if alpha values are all 0, treat the
  file as opaque, not as fully-transparent black"). `TgaImage` gains
  two standalone primitives: `all_alpha_zero(&self) -> bool` (RGBA,
  non-empty, every `A == 0`) and `force_opaque(&mut self)` (writes
  `A = 0xFF` to every pixel; no-op on `Rgb24` / `Gray8`). Non-RGBA
  images are a pixel-buffer no-op for the resolver but still surface
  the file's declared `AttributesType` so callers can inspect it without
  a second `parse_tga_attributes_type` call. New `tests/round221.rs`
  pins 18 cases: `all_alpha_zero` true/false across pixel formats and
  the empty-image edge case, `force_opaque` write semantics + no-op on
  non-RGBA + idempotency-after-zero, resolver precedence (declared type
  beats fallback for `NoAlpha` / `UsefulAlpha` / `PremultipliedAlpha`),
  resolver fallback (no extension area + all-zero alpha → force opaque
  + return `None`; no extension area + some non-zero alpha → unchanged
  + return `None`), resolver no-op on `Rgb24` / `Gray8`, and the
  fallback on an RLE-encoded file. Suite 153 → 171 tests, green
  standalone and with the `registry` feature on. No changes to the
  on-disk wire format or the encoder.

- Round 213 (criterion benchmark harness): a new `benches/codec.rs`
  driven by `criterion` (added as a dev-dep with `default-features =
  false` and the `cargo_bench_support` feature only — no impact on
  runtime consumers) characterises the decode + encode hot paths.
  Ten scenarios isolate one decode-or-encode call apiece on
  procedurally-generated 256×256 inputs: decode `uncompressed_24bpp`
  (type 2 / 24 bpp gradient), `uncompressed_32bpp` (type 2 / 32 bpp
  alternating-alpha), `rle_24bpp_runs` (type 10 / banded so each row
  packs into ~8 §C.5 run packets), `rle_24bpp_noise` (type 10 /
  gradient so §C.5 falls back to raw packets), `grayscale_8bpp`
  (type 3), `palette_8bpp` (type 1 + 256-colour palette); encode
  `uncompressed_rgba` (`encode_tga_uncompressed` auto-24/32-bpp),
  `uncompressed_rgb24` (`encode_tga_uncompressed_rgb24` fixed 24 bpp,
  no alpha scan), `rle_rgba_runs` (exercises §C.5 run packets),
  `rle_rgba_noise` (exercises §C.5 raw packets). No fixtures are
  committed; every input is generated inside the bench file. The
  harness uses only the crate's published standalone API
  (`parse_tga`, `encode_tga_uncompressed`,
  `encode_tga_uncompressed_rgb24`, `encode_tga_rle`,
  `encode_tga_grayscale`, `encode_tga_palette`). `Cargo.toml` gains
  a `[[bench]]` entry with `harness = false`; README §"Benchmarks"
  documents the scenario matrix and the run command. No source-of-
  truth crate changes; numeric headline figures are deliberately
  omitted from this commit (they drift with CI hardware). Default
  `cargo bench` run completes in roughly a minute on a laptop.

- Round 207 (encode-side fuzz harness): a second cargo-fuzz target,
  `encode_roundtrip`, lands alongside the existing decode-only
  `decode_tga` target. The new target generates a width × height
  pixel buffer from arbitrary fuzz bytes (selector byte selecting one
  of the eight standalone writers + two `u16` dimensions + a tiled
  pixel-data tail), drives it through that writer
  (`encode_tga_uncompressed`, `encode_tga_uncompressed_rgb24`,
  `encode_tga_rle`, `encode_tga_rle_rgb24`, `encode_tga_grayscale`,
  `encode_tga_grayscale_rle`, `encode_tga_palette`,
  `encode_tga_palette_rle`), decodes the freshly-encoded file back
  through `parse_tga`, and asserts the round-tripped frame's `width`
  / `height` / `pixel_format` / payload length match the requested
  dimensions. Pixel-content equality isn't asserted — the RGBA→24/32
  depth auto-selection drops alpha when every input pixel is opaque,
  palette encoders re-index and so re-order colours, and RGB24-input
  writers always emit 24 bpp regardless of alpha — so frame-shape
  integrity is the strongest assertion that's valid across every
  writer. The harness shares the decoder target's 16-MiB raster cap;
  the tail is *tiled* (rather than zero-padded) so the RLE writers
  exercise their raw-packet path instead of collapsing every row to
  one giant run. `fuzz/Cargo.toml` gains the new `[[bin]]` entry and
  the daily fuzz workflow (`.github/workflows/fuzz.yml`) splits its
  30-minute budget evenly across both targets (the reusable workflow
  auto-discovers `fuzz/fuzz_targets/*.rs`). No source-of-truth crate
  changes; the new target consumes only the already-public encoder /
  decoder API surface.

- Round 8 part 2 (Image Identification Field — spec §3.3 / §C.3): the
  free-form, up-to-255-byte identification block written immediately
  after the 18-byte header is now a first-class round-trippable
  surface. Decoder side: `parse_tga_image_id(input) -> Option<&[u8]>`
  borrows the slice straight out of the input buffer (empty slice for
  the common `id_length == 0` case, `None` for inputs truncated before
  the declared length). No NUL trimming, no UTF-8 decode — the spec
  describes the field as "free-form identification" and declines to
  constrain the content, so the bytes are surfaced verbatim and the
  caller picks the convention. Encoder side:
  `splice_image_id(&mut Vec<u8>, &[u8])` rewrites a freshly-encoded
  base TGA to carry an ID — sets byte 0 to the new length, inserts the
  bytes at offset 18, and shifts the colour-map + pixel + any trailing
  payload by the same number of bytes (`Vec::splice` over an empty
  range, one shift, in place). The new cap constant `TGA_IMAGE_ID_MAX`
  matches the spec maximum of 255 bytes (the on-disk length field is a
  single byte). The splice composes cleanly with
  `encode_tga_with_extension`: call the splice *before* extension
  wrapping and the footer / extension area / postage-stamp / CCT /
  scan-line / developer-area offsets that the extension writer back-
  patches all land on post-splice byte positions. The new `tests/round8.rs`
  suite pins the round-trip across every base writer (true-colour 24 /
  32 / RLE, RGB24 uncompressed + RLE, palette uncompressed + RLE,
  grayscale uncompressed + RLE), the splice + extension area
  composition, and the edge cases (empty ID is a no-op, 255-byte ID is
  accepted, > 255 returns `InvalidData`, double-splice rejected, splice
  into a sub-header buffer rejected, decoder is panic-free on
  truncated / arbitrary input). Suite 136 → 153 tests, green standalone
  and with the `registry` feature on. The fuzz harness now drives
  `parse_tga_image_id` alongside every other public parser so the
  panic-free contract extends to the new surface.

- Round 8 (right-to-left columns): the decoder now honours
  image-descriptor **bit 4** (column ordering) instead of rejecting it
  as `Unsupported`. Per the TGA 2.0 FFS image-descriptor field
  (page 9, "Bits 5 & 4 … Bit 4 is for left-to-right ordering and bit 5
  is for top-to-bottom ordering", Table 2 - Image Origin), bit 4 set
  means the on-disk columns are right-to-left, so each decoded row is
  mirrored horizontally to reach the crate's normalised top-down,
  left-to-right output. The existing bit-5 row flip and the new bit-4
  column mirror compose: a file with bit 4 set and bit 5 clear
  (bottom-up + right-to-left) decodes as a 180° rotation. The single
  normalisation pass in `parse_tga` now applies both transforms in one
  copy. `TgaHeader::is_right_to_left` is retained as the descriptor
  accessor; its doc and the module-level descriptor table are corrected
  (an earlier comment had described bit 4 as "reserved / never set" —
  the authoritative TGA 2.0 FFS defines it as the horizontal-ordering
  bit). Two new tests in `tests/roundtrip.rs`
  (`decodes_right_to_left_columns_by_mirroring`,
  `right_to_left_plus_bottom_up_normalises_both_axes`) assert the
  mirrored and 180°-rotated outputs against hand-built expected buffers.
  Replaces the former `rejects_right_to_left_columns` test, whose
  premise (rejection) no longer holds.

### Fixed

- Round 7 (panic-free hardening): `parse_tga_postage_stamp` previously
  drove `decode_raw_pixels` against the parent header's pixel depth
  without first running `validate_depth`. A footer + extension area
  authored with `postage_stamp_offset` set, against a header whose
  `image_type` (true-colour / grayscale) was inconsistent with the
  `pixel_depth` field (e.g. type 2 + depth 4, type 3 + depth 0),
  reached `emit_pixel`'s `unreachable!()` arm and panicked the process —
  a contract violation, since every public parser is documented as
  "returns `Result`, never panics" regardless of how malformed the
  input is. `parse_tga` already validated depth pre-decode; the
  postage-stamp helper now mirrors that. Both `unreachable!()` arms in
  `emit_pixel` (unsupported depth on true-colour, and `ImageType::None`)
  are also rewritten as returned `Err`s so a future caller that forgets
  the validation still fails closed rather than aborting. Caught by
  the round-6 cargo-fuzz harness as `crash-aa6c89f0c369…`; the
  reproducer is now committed to the seed corpus as
  `r175-postage-stamp-invalid-depth.bin` and pinned in
  `tests/round7.rs` (9 new tests: corpus replay across every public
  parser + `parse_tga_postage_stamp` (type=2 depth=4, type=2 depth=0,
  type=3 depth=4, type=10 depth=4) returning `Unsupported` instead of
  panicking, plus the success-path regression tests at depth 24 / 8).
  Suite 127 → 136 tests, green standalone and with the `registry`
  feature on. 2 000-iter local fuzz run on the patched build reaches
  cov 561 / ft 751 / corp 79 with zero crashes.

### Added

- Round 7: cargo-fuzz `decode_tga` harness + daily 30-minute CI run
  (`.github/workflows/fuzz.yml`, cron 05:31 UTC, reusable
  `OxideAV/.github/.github/workflows/crate-fuzz.yml@master`). One
  decode-only panic-free target driving the full TGA decode chain
  (`parse_header` → optional Image-ID → optional colour-map →
  raw-or-§C.5-RLE body → optional 26-byte TGA 2.0 footer →
  495-byte extension area → postage stamp / §C.6.8 colour-correction
  table / §C.6.9 scan-line table / §C.7 developer-area tag directory)
  plus every public standalone parser (`parse_tga_footer`,
  `parse_tga_extension_area`, `parse_tga_postage_stamp`,
  `parse_tga_colour_correction_table`, `parse_tga_scan_line_table`,
  `parse_tga_developer_area`, `parse_tga_attributes_type`).
  A 16-MiB declared-raster cap mirrors what a real demuxer's
  sanity limits would enforce (`width × height × 4 ≤ 16 MiB`); the
  library itself keeps no policy cap. Seed corpus committed as
  10 small fixtures spanning types 1 / 2 / 3 / 9 / 10 / 11, RGBA /
  RGB24 / Gray8, plus two TGA 2.0 footer / extension / developer-area
  variants. 2 000-iter local smoke run reaches cov ≈ 566 ft ≈ 773
  with zero crashes. No external library oracle was consulted; the
  contract is purely panic-free under arbitrary bytes.

- Round 6: attributes-type *interpretation* (spec §C.6.13). Round 4
  carried the typed `AttributesType` view + `has_meaningful_alpha` /
  `is_premultiplied` predicates; this round makes the byte actionable.
  `AttributesType::normalize_rgba8` maps a single decoded `[R,G,B,A]`
  pixel to straight (non-premultiplied) alpha: `NoAlpha` (0) /
  `UndefinedIgnore` (1) force the pixel opaque, `UndefinedRetain` (2) /
  `UsefulAlpha` (3) / `Reserved` pass through, and `PremultipliedAlpha`
  (4) un-premultiplies each colour channel
  (`straight = round(stored × 255 / A)`, clamped; `A == 0` → transparent
  black) per the spec's Porter-Duff example. `apply_to_image` rewrites a
  decoded `TgaImage` in place (RGBA only; Rgb24 / Gray8 carry no alpha
  and are left untouched). The new `parse_tga_attributes_type` decoder
  helper surfaces the typed attribute straight from a file's footer +
  extension area, returning `None` for files with no extension area.
- Round 6: 21 new tests in `tests/round6.rs` (suite 106 → 127) covering
  per-variant `normalize_rgba8` semantics (opaque-forcing, pass-through,
  spec-example un-premultiply, round-to-nearest, overshoot clamp,
  transparent-black), `apply_to_image` across RGBA / Rgb24 / Gray8,
  `parse_tga_attributes_type` recovery (premultiplied / reserved /
  absent), and two end-to-end encode→decode→read-attribute→normalise
  round-trips (premultiplied + no-alpha).

- Round 5: colour-correction table *application* (spec §C.6.8). The
  table is now usable as a LUT, not just carried as metadata:
  `TgaColourCorrectionTable::correct_rgba16` maps an 8-bit `[R,G,B,A]`
  pixel to a full-precision 16-bit corrected pixel through its
  per-channel curves; `correct_rgba8` narrows that to 8 bits (high
  byte) so the default (identity) table is a bit-exact no-op;
  `correct_gray8` runs a luma sample through the green curve; and
  `apply_to_image(&mut TgaImage)` rewrites a decoded image in place for
  Rgba / Rgb24 / Gray8 pixel formats.
- Round 5: 14 new tests in `tests/round5.rs` (suite 92 → 106) covering
  identity no-op, per-channel curve addressing, 8-bit high-byte
  narrowing, inversion/brighten/darken curves, grayscale luma path,
  Rgb24 alpha-skip, and two end-to-end encode→decode→recover→apply
  round-trips (true-colour + grayscale).

### Fixed

- Round 5: colour-correction table on-disk layout. Spec §C.6.8 stores
  the 2048-byte block *interleaved* ("each set of four contiguous
  values are the desired A:R:G:B correction for that entry"), but the
  round-4 `parse`/`to_bytes` wrote it planar (four contiguous 256-entry
  curves). Both sides now interleave/de-interleave per spec; the
  in-memory struct stays planar for convenience. Round-trips were
  already symmetric so no released file was affected (round 4 is
  unreleased), but files written for/by spec-conformant third parties
  now interoperate correctly. New `tests/round5.rs` locks the byte
  layout down.

- Round 4: TGA 2.0 developer area (spec §C.7) parse + write. New
  `parse_tga_developer_area` returns a `TgaDeveloperArea` holding the
  tag directory (`Vec<TgaDeveloperTag>`), with a `payload(input, tag)`
  borrow accessor for each tag's application-defined bytes. New
  `DeveloperTagInput` lets callers attach an arbitrary list of
  application tags to `ExtensionAreaInput::developer_tags`; the writer
  lays the payloads down before the extension area, builds the
  directory, and back-patches the footer's
  `developer_directory_offset`. Marker tags (empty payload) round-trip
  with on-disk offset/size of zero.
- Round 4: colour-correction table (spec §C.6.8) parse + write.
  `TgaColourCorrectionTable` carries the four 256-entry u16 curves in
  ARGB order; `parse_tga_colour_correction_table` walks the
  `colour_correction_offset` from the extension area, and
  `ExtensionAreaInput::colour_correction_table` opts the writer into
  emitting the 2048-byte table and back-patching the offset.
  `TgaColourCorrectionTable::default()` is an identity curve (out = in
  with the input byte replicated in both bytes of the u16).
- Round 4: scan-line table (spec §C.6.9) parse + write.
  `TgaScanLineTable` holds the per-row u32 byte offsets (one entry per
  scanline); `parse_tga_scan_line_table` peeks the header to size the
  table from `height`. `ExtensionAreaInput::scan_line_table` writes
  the table and back-patches the offset.
- Round 4: typed `AttributesType` enum over the §C.6.13 byte
  (`NoAlpha`, `UndefinedIgnore`, `UndefinedRetain`, `UsefulAlpha`,
  `PremultipliedAlpha`, plus a `Reserved(u8)` catch-all so non-
  standard files round-trip bit-exactly). Reachable via
  `TgaExtensionArea::attributes()` while the raw `attributes_type` u8
  stays on the struct. New `has_meaningful_alpha()` /
  `is_premultiplied()` predicates surface the common-case decisions.
- Round 4: 24 new tests in `tests/round4.rs` covering typed-
  attributes roundtrip + reserved-byte preservation, CCT identity
  curve + tweaked-entry roundtrip + truncation rejection, SCT
  on-disk u32-LE layout + height-sized roundtrip + zero/zero-height/
  truncated rejection, developer area three-tag roundtrip + marker
  tags + payload-overrun rejection + truncated directory rejection,
  and an all-four-sections combined roundtrip. Brings the suite from
  68 → 92 tests, green standalone and with the `registry` feature on.

- Round 3: standalone RGB24-input entry points
  `encode_tga_uncompressed_rgb24` (image type 2, 24 bpp) and
  `encode_tga_rle_rgb24` (image type 10, 24 bpp). Symmetric to the
  RGBA writers but skip the alpha-detection scan — output is always
  24 bpp BGR on the wire, so callers that already know the input is
  fully opaque avoid the per-pixel alpha-byte walk.
- Round 3: 23 hardening tests in `tests/round3.rs`:
  RGB24-input roundtrip on both uncompressed + RLE writers,
  extension-area roundtrip across every writer surface (types
  1/2/3/9/10/11), extension-area encode→parse→re-encode→parse
  fixed-point convergence, postage-stamp coverage at 24-bit/32-bit/
  Gray8/palette parent depths (incl. the §C.6.10 "indices into
  parent palette" path via a Gray8-shaped index buffer), RLE
  pathological inputs (exact-128 runs, 129-pixel split, 300-pixel
  multi-packet rows, all-raw-packet alternation), 1×1 every-writer
  smoke. Brings the suite from 45 → 68 tests, all green standalone
  and with `--no-default-features`.

- Round 2: write paths for image types 1 (uncompressed colour-mapped),
  3 (uncompressed grayscale), 9 (RLE colour-mapped), and 11 (RLE
  grayscale) — `encode_tga_palette`, `encode_tga_grayscale`,
  `encode_tga_palette_rle`, `encode_tga_grayscale_rle`. Palette
  writers cap at 256 unique RGBA colours and emit a 32-bit BGRA
  colour-map.
- Round 2: TGA 2.0 extension-area body parse (spec §C.6) —
  `parse_tga_extension_area` returns a `TgaExtensionArea` with
  author, four 80-char comment lines, save timestamp, job name +
  elapsed time, software ID + version, key colour, pixel-aspect
  ratio, gamma, colour-correction / postage-stamp / scan-line table
  offsets, and attributes-type. `TgaTimestamp` encodes the §C.6.4
  date/time stamp; `TGA_EXTENSION_AREA_SIZE` const exposes the
  canonical 495-byte size.
- Round 2: TGA 2.0 extension-area write — `encode_tga_with_extension`
  appends a 495-byte extension area body + 26-byte footer to any TGA
  byte stream produced by the writers above. `ExtensionAreaInput`
  carries the user-supplied fields.
- Round 2: postage-stamp (thumbnail) extraction +
  emission. `parse_tga_postage_stamp` decodes the §C.6.10 embedded
  preview into a `TgaImage`; the postage-stamp is always uncompressed
  on disk (regardless of the parent's compression) and shares the
  parent's pixel-format and palette. The encoder serialises the
  thumbnail in whatever depth the parent header reports (24-bit BGR /
  32-bit BGRA / 8 bpp Gray).

## [0.0.1](https://github.com/OxideAV/oxideav-tga/releases/tag/v0.0.1) - 2026-05-05

### Added

- Round 1: clean-room Truevision TGA reader/writer per the public
  Truevision TGA File Format Specification v2.0 (1989).
- Read coverage for image types 1 (uncompressed colour-mapped),
  2 (uncompressed true-colour), 3 (uncompressed grayscale), 9 (RLE
  colour-mapped), 10 (RLE true-colour), and 11 (RLE grayscale).
- Pixel depth coverage 8 / 15 / 16 / 24 / 32 bpp; 16-bit decoded as
  A1R5G5B5 (top bit alpha), 24-bit as BGR, 32-bit as BGRA.
- Image descriptor byte handled for both bottom-left and top-left
  origins; output is always normalised to top-left.
- TGA 2.0 footer (`TRUEVISION-XFILE.\0`) recognised + extension
  area / developer area pointers parsed.
- Write paths for image type 2 (uncompressed true-colour, 24/32-bit)
  and image type 10 (RLE true-colour, 24/32-bit). RLE encoder picks
  runs of ≥ 2 consecutive identical pixels (max 128 per run-packet)
  vs raw runs (max 128 per raw-packet) per spec §C.5.
- Default-on `registry` feature gating the `oxideav-core` `Decoder` /
  `Encoder` / container glue; standalone (no-`registry`) build
  exposes only the framework-free API surface.