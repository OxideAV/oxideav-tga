# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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