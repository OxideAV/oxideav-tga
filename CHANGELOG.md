# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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