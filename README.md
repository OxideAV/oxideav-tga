# oxideav-tga

Pure-Rust Truevision TGA (TARGA) reader/writer for the
[`oxideav`](https://github.com/OxideAV/oxideav) framework.

Clean-room implementation of the public **Truevision TGA File Format
Specification v2.0** (1989). The spec PDF and the Gamers.org plain-
text mirror of the same document are the only sources consulted.

## Decode

| Type | Compression  | Pixel                        | Output  |
| ---- | ------------ | ---------------------------- | ------- |
| 1    | uncompressed | 8 bpp + palette (15/16/24/32) | `Rgba`  |
| 2    | uncompressed | 15 / 16 / 24 / 32 bpp        | `Rgba`  |
| 3    | uncompressed | 8 bpp grayscale              | `Gray8` |
| 9    | RLE          | 8 bpp + palette              | `Rgba`  |
| 10   | RLE          | 15 / 16 / 24 / 32 bpp        | `Rgba`  |
| 11   | RLE          | 8 bpp grayscale              | `Gray8` |

* Colour-mapped types (1 / 9) honour the §C.2 Color Map Origin (the
  "index of first color map entry", header bytes 3-4): the on-disk
  palette stores `cmap_length` entries beginning at that origin, so an
  image index `idx` addresses on-disk entry `idx - cmap_first`. Indices
  below the origin or past the stored length are rejected as out of
  range. Origin-0 files (everything this crate's encoder writes) are
  unaffected.
* 16-bit pixels decoded as A1R5G5B5 (top bit alpha); 15-bit forces
  alpha to `0xFF`.
* 24-bit pixels are BGR on disk, 32-bit are BGRA — both swapped to
  RGBA at decode time.
* Both image-descriptor ordering bits are honoured per the TGA 2.0 FFS
  image-descriptor field (Table 2 - Image Origin): bit 5 selects
  bottom-up vs top-down rows and bit 4 selects right-to-left vs
  left-to-right columns. Output is always normalised to a top-down,
  left-to-right origin — rows are flipped when bit 5 is clear and
  columns are mirrored when bit 4 is set (both axes for a file that sets
  bit 4 with bit 5 clear, i.e. a 180° rotation).
* The image-descriptor **data-storage interleaving flag** (Field 5.6, bits
  7-6) is surfaced as a typed `Interleaving` view via
  `TgaHeader::interleaving()` / the one-call `parse_tga_interleaving` reader
  (`NonInterleaved` / `TwoWay` / `FourWay` / `Reserved`, plus `raw` /
  `to_descriptor_bits` / `is_non_interleaved` / `is_two_way` / `is_four_way`
  / `is_reserved` / `is_tga2_compliant`). The TGA 2.0 FFS requires these
  bits to be zero ("Must be zero to insure future compatibility" — TGA 2.0
  abandoned the earlier interleaving scheme), so a conformant 2.0 file
  reports `NonInterleaved`; the legacy values defined by the earlier
  Truevision layout (00 non-interleaved / 01 two-way even-odd / 10 four-way
  / 11 reserved) are surfaced verbatim so a caller can recognise — and
  choose to reject — a legacy interleaved file. The crate does **not**
  de-interleave: the 2.0 FFS removed interleaving and never documents the
  reorder algorithm, and the decoder treats the raster as non-interleaved
  regardless (matching the dominant zero case). `TGA_INTERLEAVING_MASK`
  (`0xC0`) exposes the field's bit mask. This mirrors the surface-but-don't-
  guess approach already taken for the §C.2 attribute-bit count
  (`AttributeBits`).
* The optional 26-byte TGA 2.0 footer is recognised. Use
  `parse_tga_footer` for the extension/developer-area offsets — the
  returned `TgaFooter` carries typed accessors (`has_extension_area` /
  `has_developer_area` / `is_marker_only` for the zero-means-absent
  sentinel; `as_tuple` / `from_tuple` round-trip; `offsets_within(len)`
  sanity-checks both offsets against a buffer length; `to_bytes()`
  emits the canonical 26-byte trailer — LE `u32` extension offset, LE
  `u32` developer offset, 18-byte `"TRUEVISION-XFILE.\0"` signature —
  and round-trips through `parse_tga_footer` bit-exactly) and
  `TgaFooter::UNSET` is the all-zero sentinel,
  `parse_tga_extension_area` for the 495-byte extension-area body
  (author / comments / timestamp / job / software ID + version /
  pixel-aspect / gamma / colour-correction + postage-stamp +
  scan-line offsets / attributes-type), and `parse_tga_postage_stamp`
  to pull out the embedded thumbnail.
* The §C.6.10 Postage Stamp Image (Field 26) dimension header — the two
  leading on-disk size bytes that prefix the thumbnail's pixels ("The
  first byte … specifies the X size of the stamp in pixels, the second
  byte … the Y size") — is surfaced as a typed `PostageStamp` view
  matching the rest of the `_typed` family: `UNSET` / `new` / `as_tuple`
  / `from_tuple` / `is_unset` (either axis zero) / `pixel_count` (`u32`,
  so the 255×255 worst case doesn't overflow) / `within_recommended_size`
  (both edges ≤ 64 per Truevision's "does not recommend stamps larger
  than 64 x 64 pixels" guidance) / `clipped_to_recommended` (clamps each
  axis to the recommended 64-pixel cap — the recommendation is advisory,
  not a hard limit). `parse_tga_postage_stamp_dimensions` reads just
  those two bytes straight from a file (returning `None` for a TGA 1.0
  file / a file with no stamp, `Err` for an out-of-range offset) so a
  caller can inspect the stamp geometry without decoding its pixels. The
  `TGA_POSTAGE_STAMP_RECOMMENDED_MAX` (64) and `TGA_POSTAGE_STAMP_MAX`
  (255, the single-byte axis ceiling) constants expose the spec's
  dimension bounds.
* The §C.6.13 attributes byte is exposed as both the raw
  `attributes_type: u8` and a typed `AttributesType` enum reachable
  via `TgaExtensionArea::attributes()` (`NoAlpha` / `UndefinedIgnore`
  / `UndefinedRetain` / `UsefulAlpha` / `PremultipliedAlpha` plus a
  `Reserved(u8)` catch-all so non-standard files round-trip
  bit-exactly). `parse_tga_attributes_type` reads the typed attribute
  straight from a file's footer/extension area.
* The attributes type is also *applied*, not just carried:
  `AttributesType::normalize_rgba8` maps a decoded `[R,G,B,A]` pixel to
  straight (non-premultiplied) alpha — `NoAlpha`/`UndefinedIgnore` force
  opaque, `UndefinedRetain`/`UsefulAlpha`/`Reserved` pass through, and
  `PremultipliedAlpha` un-premultiplies each colour channel
  (`straight = round(stored × 255 / A)`, clamped; fully-transparent
  pixels become transparent black) per the spec's Porter-Duff example.
  `apply_to_image(&mut TgaImage)` rewrites a decoded image in place
  (RGBA only; Rgb24/Gray8 have no alpha and are left untouched).
* The §C.6.8 colour-correction table is exposed as
  `TgaColourCorrectionTable` (four 256-entry u16 curves in ARGB order),
  parsed by `parse_tga_colour_correction_table`. The on-disk block is
  the spec's *interleaved* 256 × 4 `u16` layout — each entry is four
  contiguous `(A, R, G, B)` shorts — de-interleaved into the struct's
  planar curves on parse and re-interleaved on `to_bytes`.
* The table is also *applied*, not just carried: `correct_rgba16`
  maps an 8-bit `[R,G,B,A]` pixel to a full-precision 16-bit corrected
  pixel through its per-channel curves; `correct_rgba8` narrows that to
  8 bits (high byte), so the identity table is a bit-exact no-op;
  `correct_gray8` runs a single luma sample through the green curve; and
  `apply_to_image(&mut TgaImage)` rewrites a decoded image in place
  (Rgba / Rgb24 / Gray8). BLACK maps to 0, WHITE to 65535 per the spec.
* The §C.6.9 scan-line table is exposed as `TgaScanLineTable` (a Vec
  of per-row u32 byte offsets, sized from the parent header's height),
  parsed by `parse_tga_scan_line_table`. Typed accessors on the struct
  (matching the pattern of `TgaFooter` / `KeyColor` / `TgaTimestamp` /
  `TgaAsciiField`): `EMPTY` (empty-table sentinel) + `Default` (==
  `EMPTY`); construction via `new` / `with_capacity(height)` /
  `FromIterator<u32>`; geometry / sentinel via `len` / `is_empty` /
  `is_unset` / `byte_size`; bounds-checked row-offset accessor `get(y)`;
  buffer-bounds sanity check `is_well_formed_within(input_len)`;
  direction-of-save predicates `is_strictly_increasing` (top-down)
  and `is_strictly_decreasing` (bottom-up — the spec's §C.6.9 "in
  the order that the image was saved (i.e., top down or bottom up)"
  branches); and `row_range(y, terminal)` / `row_bytes(input, y,
  terminal)` for deriving a `[start, end)` byte range for row `y`
  using the next row's recorded offset (or a caller-supplied terminal
  for the last row) and borrowing the row's bytes straight out of the
  input. The `TGA_SCAN_LINE_OFFSET_BYTES` constant exposes the on-disk
  4-byte size of one entry.
* The scan-line table is also *computed* and *used*, not just carried:
  `compute_tga_scan_line_table` derives the §C.6.9 table from any
  supported file's own pixel data (offset arithmetic for uncompressed
  types, one §C.5 packet walk for RLE types; entries in saved order,
  from-file-start, one per row). An RLE file whose packets span a
  scan-line boundary is rejected with `Unsupported` — the spec's
  packet rule ("should never encode pixels from more than one scan
  line") is what makes the table well-defined, and `parse_tga` still
  decodes such files whole. `parse_tga_scan_line(input, &table,
  index)` then does the random access the spec built the table for:
  decode exactly one row (no preceding rows touched), returning a
  1-pixel-tall `TgaImage` in the same normalised format `parse_tga`
  produces (palette lookup, BGR→RGBA swap, A1R5G5B5 expansion,
  bit-4 column mirroring); `index` is in the table's saved order, so
  bottom-up files address display row `height − 1 − y` at entry `y`.
  A table computed against a freshly-encoded base file can be handed
  straight to `ExtensionAreaInput::scan_line_table` — the extension
  writer only appends after the pixel data, so the offsets stay valid
  in the extended file.
* The §C.7 developer-area tag directory is exposed as
  `TgaDeveloperArea` (a `Vec<TgaDeveloperTag>` with `tag_id` / `offset`
  / `size`), parsed by `parse_tga_developer_area`; each tag's
  application-defined payload bytes are borrowable via
  `dev.payload(input, tag)`. Typed accessors on both structs (matching
  the pattern of `TgaScanLineTable` / `TgaFooter` / `TgaAsciiField` /
  `TgaTimestamp`): `TgaDeveloperTag` carries `new` / `as_tuple` /
  `from_tuple` (on-disk TAG, OFFSET, FIELD SIZE order); the §C.7
  tag-id range classifiers `is_developer_use` (0..=32767, "available
  for developer use") and `is_truevision_reserved` (32768..=65535,
  "reserved for Truevision"); `is_marker` (offset-0 / no-payload
  record); `is_well_formed_within(input_len)` (payload range fits the
  buffer, mirroring `parse`'s per-record rejection rule); and
  `to_bytes()` emitting the 10-byte LE record. `TgaDeveloperArea`
  carries `EMPTY` (empty-directory sentinel) + `Default` (== `EMPTY`);
  construction via `new` / `FromIterator<TgaDeveloperTag>`; geometry /
  sentinel via `len` / `is_empty` / `is_unset` / `directory_byte_size`
  (the spec's `n × 10 + 2` formula); positional + by-id lookup via
  `get(i)` / `find(tag_id)` (first match in directory order — the spec
  allows unsorted directories) / `contains(tag_id)`; whole-directory
  `is_well_formed_within(input_len)`; and `to_bytes()` serialising the
  count-prefixed directory bit-exactly as `parse` reads it. The
  `TGA_DEVELOPER_TAG_BYTES` (10) and
  `TGA_DEVELOPER_DIRECTORY_HEADER_BYTES` (2) constants expose the
  on-disk dimensions.
* The §C.2 Color Map Specification (header bytes 3-7 — Color Map Origin,
  Color Map Length, Color Map Entry Size) is exposed as a typed
  `TgaColorMap` (`first_index` / `entry_size` / `entries`, plus `empty`
  sentinel / `len` / `is_empty` / origin-aware `get(idx)`), parsed by
  `parse_tga_color_map`. The helper reads only the header + color-map
  block, so it surfaces the palette of a colour-mapped file **and** of a
  **Data Type 0 (No Image Data)** palette-only file (a header + color map
  with no pixel array) that `parse_tga` rejects as having no image.
  Entries are de-interleaved into straight RGBA exactly as `parse_tga`
  expands the palette internally (15/16-bit A1R5G5B5, 24-bit BGR, 32-bit
  BGRA), and the Color Map Origin is recorded so a logical pixel index
  `idx` resolves to `entries[idx - first_index]`. Returns `None` when the
  Color Map Type byte is `0` (no map present).
* The §3.3 / §C.3 Image Identification Field (the free-form,
  up-to-255-byte block at offset 18) is exposed verbatim by
  `parse_tga_image_id`; the helper returns the borrowed byte slice, an
  empty slice for the common `id_length == 0` case, or `None` when the
  buffer is truncated. Content is left untouched (no NUL trimming, no
  UTF-8 decode) because the spec leaves the format unconstrained.
* The §C.6.4 / §C.6.5 / §C.6.6 / §C.6.7 numeric extension-area fields are
  surfaced as typed views in addition to the raw on-disk tuples:
  `KeyColor` (§C.6.4) wraps the `[A,R,G,B]` quadruple with `from_argb` /
  `to_argb` / `as_rgba8` / `is_unset` / `has_alpha`; `PixelAspectRatio`
  (§C.6.5) wraps `(numerator, denominator)` with `is_unset` / `is_square`
  / `as_f32` / `corrected_display_height(h)` / `corrected_display_width(w)`
  for square-display-pixel resampling; `GammaValue` (§C.6.6) wraps the
  same SHORT pair with `is_unset` / `is_identity` / `as_f32` plus
  `apply_to_channel8` / `apply_to_rgba8` / `apply_to_image` (RGBA / Rgb24
  / Gray8) that raises each channel to the gamma exponent
  (`y = (x/255)^gamma × 255`, rounded, clamped, alpha untouched) — unset
  / identity / malformed gammas are bit-exact no-ops; `SoftwareVersion`
  (§C.6.7) wraps `(SHORT, BYTE-as-char)` with `is_unset` / `as_f32`
  (`number_times_100 / 100.0`). The typed views are reachable via
  `TgaExtensionArea::{key_color_typed, pixel_aspect_ratio_typed,
  gamma_typed, software_version_typed}` and as one-call parser helpers
  `parse_tga_key_color` / `parse_tga_pixel_aspect_ratio` /
  `parse_tga_gamma` / `parse_tga_software_version` straight from a TGA
  file's bytes — each returns `None` for a TGA 1.0 file or any file
  without an extension area.

* The extension area's four ASCII fields — Author Name (Field 11),
  Author Comments (Field 12), Job Name/ID (Field 14), Software ID
  (Field 16) — carry typed views matching the same pattern.
  `TgaAsciiField` wraps the parsed payload for the three 41-byte
  fields (`new` / `from_borrowed` / `as_str` / `into_inner` /
  `char_len` / `is_unset` (empty / NUL-only / blanks-and-NULs per
  the spec's recommended "fill with nulls or a series of blanks
  terminated by a null" sentinel) / `is_valid_ascii` (every byte
  printable ASCII `0x20..=0x7E` per the spec recommendation) /
  `fits_capacity` (40-character on-disk slot) / `trimmed` (leading
  + trailing ASCII whitespace stripped)). `TgaAuthorComments` wraps
  the four 81-byte lines of Field 12 (`new` / `from_strs` / `empty`
  / `line(i)` / `is_unset` / `is_valid_ascii` / `fits_capacity`
  (80-character per-line cap) / `joined` for a newline-separated
  paragraph that drops trailing blank lines). Reachable via
  `TgaExtensionArea::{author_name_typed, author_comments_typed,
  job_name_typed, software_id_typed}`, and as one-call parser
  helpers `parse_tga_author_name` / `parse_tga_author_comments` /
  `parse_tga_job_name` / `parse_tga_software_id` straight from a
  TGA file's bytes — each returns `None` for a TGA 1.0 file or any
  file without an extension area. Per-field constants
  `TGA_ASCII_FIELD_MAX_CHARS` (40), `TGA_AUTHOR_COMMENT_LINES` (4),
  `TGA_AUTHOR_COMMENT_LINE_BYTES` (81),
  `TGA_AUTHOR_COMMENT_LINE_MAX_CHARS` (80) expose the spec's
  on-disk dimensions.

* The extension area's Field 13 (Date/Time Stamp) and Field 15 (Job
  Time) sub-fields carry typed views matching the same pattern. The
  existing `TgaTimestamp` struct gains `UNSET` (all-zero sentinel),
  `is_valid` (month ∈ 1..=12, day ∈ 1..=31, year ≥ 1, hour ∈ 0..=23,
  minute ∈ 0..=59, second ∈ 0..=59), `as_tuple` / `from_tuple` (six-SHORT
  round-trip in on-disk order), and `iso8601` returning a sortable
  `"YYYY-MM-DDTHH:MM:SS"` string on a set timestamp / `None` on the
  unset sentinel. `JobTime` is new: `UNSET` / `new` / `from_tuple` /
  `as_tuple` / `is_unset` / `is_valid` (minutes and seconds ∈ 0..=59;
  hours covers the full SHORT range per the spec) / `total_seconds`
  (`u32`) / `as_f64_hours` (total seconds divided by 3600) /
  `hms_string` (zero-padded `"HH:MM:SS"`, hours widens past two digits
  at the SHORT cap). Reachable via `TgaExtensionArea::timestamp_typed`
  / `job_time_typed`, and as one-call parser helpers
  `parse_tga_timestamp` / `parse_tga_job_time` straight from a TGA
  file's bytes — each returns `None` for a TGA 1.0 file or any file
  without an extension area.

* The §C.2 Image Descriptor **attribute-bit count** (Field 5.6, bits 3-0
  — "the number of attribute bits per pixel … designated as Alpha Channel
  bits") is exposed as a typed `AttributeBits` view via
  `TgaHeader::attribute_bits()` (`count` / `is_none` / `has_alpha` /
  `is_canonical_for_depth` — 8 for 32 bpp, 1 for 16 bpp, 0 for the
  alpha-less depths) and read straight from a file by
  `parse_tga_attribute_bits`. Unlike the extension-area `AttributesType`
  (Field 24, TGA 2.0 only), this declaration lives in the fixed 18-byte
  header, so it is present in **every** TGA file including TGA 1.0. The
  spec ties the two together — a Field 24 value of `0` ("no Alpha data
  included") states "bits 3-0 of field 5.6 should also be set to zero" —
  so a zero attribute-bit count is the header-local signal that a 32-bpp
  pixel's fourth byte (or a 16-bpp pixel's top bit) carries no meaningful
  alpha and the picture should display opaque.
  `resolve_alpha_from_descriptor(input, &mut image)` is the apply-side:
  for an RGBA image whose header declares zero attribute bits it forces
  every alpha byte to `0xFF` (`TgaImage::force_opaque`) and otherwise
  leaves the decoded alpha untouched, returning the `AttributeBits` it
  read. It is the header-only counterpart to
  `resolve_alpha_with_targa32_fallback` (which prefers the TGA 2.0
  extension area and falls back to the all-alpha-zero heuristic): the
  descriptor resolver consults the header bits alone, never reads the
  extension area, and never inspects pixel values — so it acts even on a
  non-zero-but-uninitialised alpha plane when the header says "no
  attribute bits". `TGA_ATTRIBUTE_BITS_MAX` (15) exposes the four-bit
  field's ceiling.
* The well-known **TARGA-32 vs ARGB-32 ambiguity** (32-bpp files written
  by legacy paint tools that left `attributes_type` unset and the alpha
  channel uninitialised) has an opt-in resolver:
  `resolve_alpha_with_targa32_fallback(input, &mut image)`. If the file
  declares an `AttributesType` (TGA 2.0 footer + extension area), the
  resolver applies it (`AttributesType::apply_to_image` semantics) and
  returns `Some(attrs)`. Otherwise it checks `image.all_alpha_zero()` and
  — when every alpha byte is `0` — calls `image.force_opaque()` so the
  decoded picture displays as opaque rather than fully-transparent
  black, returning `None`. The default decode path is unchanged; callers
  who want this convention call the helper explicitly. `TgaImage` also
  exposes `all_alpha_zero(&self) -> bool` and `force_opaque(&mut self)`
  as standalone primitives.

## Encode

| Type | Compression  | Pixel input  | API                          |
| ---- | ------------ | ------------ | ---------------------------- |
| 1    | uncompressed | RGBA → index | `encode_tga_palette`         |
| 2    | uncompressed | RGBA         | `encode_tga_uncompressed`    |
| 2    | uncompressed | RGB24        | `encode_tga_uncompressed_rgb24` |
| 3    | uncompressed | Gray8        | `encode_tga_grayscale`       |
| 9    | RLE          | RGBA → index | `encode_tga_palette_rle`     |
| 10   | RLE          | RGBA         | `encode_tga_rle`             |
| 10   | RLE          | RGB24        | `encode_tga_rle_rgb24`       |
| 11   | RLE          | Gray8        | `encode_tga_grayscale_rle`   |

* RLE packets selected per spec §C.5: a run of ≥ 2 consecutive
  identical pixels emits a run-length packet (max 128 pixels per
  packet); isolated pixels coalesce into raw packets (max 128 pixels
  per packet). RLE doesn't cross scanline boundaries.
* True-colour RGBA writers auto-select depth: 32 bpp if any input
  alpha byte is `< 0xFF`, else 24 bpp. The RGB24-input variants
  always emit 24 bpp BGR (no alpha-detection scan).
* Palette writers cap at 256 unique RGBA colours and emit a 32-bit
  BGRA colour map (with a clear `Unsupported` error past that).
* Top-down origin (descriptor bit 5 set) is used unconditionally.

`splice_image_id(&mut base, image_id_bytes)?` rewrites a freshly-encoded
base TGA to carry an Image Identification Field (spec §3.3 / §C.3): the
helper sets byte 0 to the supplied length and inserts the bytes at offset
18, shifting the colour-map + pixel + any trailing extension area / footer
by the same number of bytes. The cap is the spec maximum of 255 bytes
(`TGA_IMAGE_ID_MAX`); an empty input is a no-op; calling the helper twice
on the same file is rejected so an existing ID is never overwritten.

`encode_tga_with_extension(base, &ExtensionAreaInput { … })` wraps
any of the writers above and appends a TGA 2.0 footer + extension
area body to the output. Optional companions on `ExtensionAreaInput`:

* `postage_stamp: Option<TgaImage>` — embedded thumbnail in the
  parent's pixel format (§C.6.10).
* `colour_correction_table: Option<TgaColourCorrectionTable>` —
  4 × 256 × u16 ARGB curves (§C.6.8), 2048 bytes on disk.
* `scan_line_table: Option<TgaScanLineTable>` — `height × u32` per-row
  byte offsets for partial-image readers (§C.6.9); build one against
  the base file with `compute_tga_scan_line_table` (the writer only
  appends, so the offsets stay valid in the extended file).
* `developer_tags: Vec<DeveloperTagInput>` — application-defined
  tagged payloads (§C.7); the writer lays the payloads down, builds
  the directory, and back-patches the footer's
  `developer_directory_offset`. Empty payloads emit spec-legal marker
  tags (offset / size = 0).

```rust
use oxideav_tga::{encode_tga_rle, parse_tga};

let rgba: Vec<u8> = /* 4 × W × H bytes, top-down RGBA */ vec![/* … */];
let bytes = encode_tga_rle(width, height, &rgba)?;
let img = parse_tga(&bytes)?;
assert_eq!(img.width as usize * img.height as usize * 4, img.data.len());
# Ok::<(), oxideav_tga::TgaError>(())
```

## Standalone vs registry-integrated

The crate's default `registry` Cargo feature pulls in `oxideav-core`
and exposes the framework `Decoder` / `Encoder` trait surface plus a
`registry::register` entry point for the `oxideav` umbrella crate.
Disable the feature for an `oxideav-core`-free build:

```toml
oxideav-tga = { version = "0.0", default-features = false }
```

The standalone build still exposes `parse_tga`, `encode_tga_uncompressed`,
`encode_tga_rle`, plus crate-local `TgaImage` / `TgaPixelFormat` /
`TgaError` types.

## Registration

```rust
let mut codecs = oxideav_core::CodecRegistry::new();
let mut containers = oxideav_core::ContainerRegistry::new();
oxideav_tga::register(&mut codecs, &mut containers);
```

## Fuzzing

Three cargo-fuzz harnesses live under `fuzz/`. `.github/workflows/fuzz.yml`
runs them daily, sharing a 30-minute budget (the reusable workflow
auto-discovers `fuzz/fuzz_targets/*.rs` and splits the time evenly).

### `decode_tga`

Drives the public decoder surface (`parse_tga` + every `parse_tga_*`
helper for footer / extension area / postage stamp / colour-correction /
scan-line / developer area / Image ID / attributes type / attribute bits /
interleaving flag / color map) with arbitrary
bytes. The contract is panic-free regardless of how hostile the input
is. A 16-MiB declared-raster cap inside the harness mirrors what a real
demuxer's sanity limits would enforce (`width × height × 4 ≤ 16 MiB`);
the library itself keeps no policy cap. Encoder-produced seeds live in
`fuzz/corpus/decode_tga/`, including a sentinel reproducer for a
previously-fixed postage-stamp depth-validation crash.

Every decode path validates the `image_type` / `pixel_depth`
combination before expanding pixels (including the postage-stamp path),
and `emit_pixel` returns `Err` for any combination the spec rejects, so
malformed input fails closed rather than panicking.

### `encode_roundtrip`

Generates a pixel buffer from arbitrary fuzz bytes (selector byte +
two `u16` dimensions + a tiled payload tail), feeds it through one of
the eight standalone writers
(`encode_tga_uncompressed`, `encode_tga_uncompressed_rgb24`,
`encode_tga_rle`, `encode_tga_rle_rgb24`,
`encode_tga_grayscale`, `encode_tga_grayscale_rle`,
`encode_tga_palette`, `encode_tga_palette_rle`),
decodes the result through `parse_tga`, and asserts the round-tripped
frame's `width` / `height` / `pixel_format` / payload length match the
requested dimensions. Pixel-content equality is not asserted (the
RGBA→24/32 depth auto-selection drops alpha when every input pixel is
opaque, palette encoders re-index and so re-order colours, and the
RGB24-input writers always emit 24 bpp regardless of alpha — frame
shape is the strongest assertion that's valid across every writer).
The same 16 MiB raster cap applies; the harness tiles (rather than
zero-pads) the fuzz tail so the RLE writers exercise their raw-packet
code path instead of collapsing every row to one giant run.

### `extension_roundtrip`

Attacks the **extension-area writer** that neither sibling target
reaches: `encode_tga_with_extension`. From arbitrary fuzz bytes it
builds a small base TGA plus an `ExtensionAreaInput` carrying every
optional block — gamma (Field 20), key colour (Field 18), pixel aspect
ratio (Field 19), software version (Field 17), attributes type
(Field 24), a §C.6.8 colour-correction table (Field 27), a §C.6.9
scan-line table (Field 25), a §C.6.10 postage-stamp thumbnail
(Field 26), and §C.7 developer tags. It then re-parses the encoded file
and asserts a faithful round trip: the footer is recognised as TGA 2.0
with a present extension area, the colour-correction table and
scan-line offsets come back **byte- / value-exact**, and the verbatim
SHORT-pair fields recover their exact tuples. These are stronger
oracles than `encode_roundtrip`'s frame-shape check because the
extension-area blocks are stored without any lossy re-quantisation, so
the assertions directly pin the writer's offset back-patch arithmetic
(the Field 21 / 22 / 23 internal offsets plus the developer-directory
offset that get filled in after each variable-length block lands).
Base image and postage stamp are clamped to tiny side lengths — the
logic under test is the offset arithmetic, not the pixel loop the other
two targets already stress at size. Minimised seeds live in
`fuzz/corpus/extension_roundtrip/`.

```sh
cargo +nightly fuzz run decode_tga -- -runs=10000
cargo +nightly fuzz run encode_roundtrip -- -runs=10000
cargo +nightly fuzz run extension_roundtrip -- -runs=10000
```

## Benchmarks

A `criterion` harness in `benches/codec.rs` characterises the decode +
encode hot paths on procedurally-generated 256×256 frames. Ten
scenarios isolate the cost of one decode or one encode call across the
crate's standalone API surface:

| Group   | Scenario              | What it isolates                                     |
| ------- | --------------------- | ---------------------------------------------------- |
| decode  | `uncompressed_24bpp`  | Type 2 / 24 bpp, every-pixel-unique gradient         |
| decode  | `uncompressed_32bpp`  | Type 2 / 32 bpp, alternating-alpha gradient          |
| decode  | `rle_24bpp_runs`      | Type 10 / 24 bpp, banded input → §C.5 run packets    |
| decode  | `rle_24bpp_noise`     | Type 10 / 24 bpp, gradient → §C.5 raw packets        |
| decode  | `grayscale_8bpp`      | Type 3 / 8 bpp luma                                  |
| decode  | `palette_8bpp`        | Type 1 / 8 bpp + 256-colour palette                  |
| encode  | `uncompressed_rgba`   | `encode_tga_uncompressed` (auto-24/32 depth)         |
| encode  | `uncompressed_rgb24`  | `encode_tga_uncompressed_rgb24` (fixed 24 bpp)       |
| encode  | `rle_rgba_runs`       | `encode_tga_rle`, run-heavy input                    |
| encode  | `rle_rgba_noise`      | `encode_tga_rle`, high-entropy input                 |

The benchmarks generate every input procedurally — no fixtures are
committed, the harness is self-contained, and a default `cargo bench`
run completes in roughly a minute on a laptop. Numbers move with
host hardware; the value of the harness is the *relative* cost
across scenarios (RLE vs uncompressed, run vs raw, 24 bpp vs 32 bpp,
true-colour vs palette vs grayscale).

```sh
cargo bench
# or to target a single scenario:
cargo bench --bench codec -- decode/rle_24bpp_runs
```

Profiling the `decode/rle_24bpp_runs` scenario surfaced the §C.5
run-packet decode as the dominant non-allocation hot spot: a run packet
expands one on-disk pixel into `count` identical output pixels, and the
decoder expands a run packet's source pixel exactly once and replicates
the emitted output bytes for the rest of the run, rather than
re-dispatching the full `image_type` / pixel-depth match per output
pixel. The decoded bytes are identical across all six RLE
image-type/depth combinations (pinned by tests); on the run-heavy decode
benchmark this is roughly a 70 % reduction in wall-clock time
(host-dependent). Raw-packet (high-entropy) decode is untouched.

## Lacks

* Image type 32 / 33 (compressed colour-mapped, Huffman + delta /
  4-pass) — Truevision never shipped them; no fixtures exist in the
  wild and the spec describes the format only at a high level.
* Application-level semantics for developer-area tag IDs — the spec
  declines to enumerate well-known IDs, so the parser surfaces tags
  + payload byte ranges and leaves interpretation to the caller.
