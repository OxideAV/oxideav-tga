# oxideav-tga

[![CI](https://github.com/OxideAV/oxideav-tga/actions/workflows/ci.yml/badge.svg)](https://github.com/OxideAV/oxideav-tga/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/oxideav-tga.svg)](https://crates.io/crates/oxideav-tga) [![docs.rs](https://docs.rs/oxideav-tga/badge.svg)](https://docs.rs/oxideav-tga) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Pure-Rust Truevision TGA (TARGA) reader/writer for the
[`oxideav`](https://github.com/OxideAV/oxideav) framework.

Clean-room implementation of the public **Truevision TGA File Format
Specification v2.0** (1989). The spec PDF and the plain-text
transcription of the same document are the only sources consulted.

The crate follows the OxideAV image-crate contract (`IMAGE_CRATE_API`
in the workspace root): the same small root vocabulary every
`oxideav-<format>` picture crate exposes, usable with
`default-features = false` and no `oxideav-core`.

## Standalone use

```toml
oxideav-tga = { version = "0.0", default-features = false }
```

```rust
let bytes = std::fs::read("in.tga")?;
if oxideav_tga::probe(&bytes) {
    let info = oxideav_tga::info(&bytes)?;      // header only: width, height, format, alpha, footer
    let img  = oxideav_tga::decode(&bytes)?;    // TgaImage, native layout (Pal8 / Rgb24 / Rgba / Gray8)
    let rgba: Vec<u8> = img.to_rgba8();         // tightly packed RGBA, 4 * width bytes per row
    let (w, h) = (img.width(), img.height());

    let opts = oxideav_tga::EncodeOptions::default().with_rle(false);
    let out: Vec<u8> = oxideav_tga::encode_rgba8(w, h, &rgba, &opts)?;
    std::fs::write("out.tga", out)?;
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Root items: `probe`, `info -> ImageInfo`, `header -> TgaHeader` (the
typed depth accessor behind `info`), `decode -> TgaImage`,
`decode_with(&DecodeOptions)`, `decode_rgb8 -> RgbImage`,
`decode_rgba8 -> RgbaImage`, `decode_from<R: Read>`,
`encode(&TgaImage, &EncodeOptions)`, `encode_rgb8`, `encode_rgba8`,
`encode_to<W: Write>`; types `TgaImage { width, height, format, planes,
color, metadata, palette, extension }`, `Plane`, `ColorInfo`,
`ColorRange`, `Metadata`, `Palette`, `ImageInfo`, `RgbImage`,
`RgbaImage`, `PixelFormat` (= `TgaPixelFormat`), `DecodeOptions`,
`EncodeOptions`, `RowOrder`, `Error` (= `TgaError`). TGA holds one
picture, so there is no `decode_all`.

`TgaImage` constructors: `new(w, h, format, planes) -> Result`
(validates the plane geometry), `new_indexed(w, h, indices, palette)
-> Result`, `from_rgb8` / `from_rgba8` / `from_gray8` / `packed` (all
`-> Result`, `InvalidData` on a buffer shorter than the geometry), and
`with_color` / `with_metadata` / `with_palette` /
`with_extension`. Accessors: `width()`, `height()`, `format()`,
`stride()`, `as_bytes() -> Option<&[u8]>`, `data()`, `data_mut()`,
`into_raw()`, `to_rgb8()`, `to_rgba8()` (infallible on anything the
decoder produced; `try_to_rgb8` / `try_to_rgba8` report a bad
caller-assembled geometry instead of padding), `to_indexed() ->
Result` (re-index to `Pal8`, `Unsupported` past 256 colours),
`into_legacy_layout()` (the pre-contract `Gray8`-or-`Rgba` shape),
`has_alpha()`, `all_alpha_zero()` / `force_opaque()`.

The pre-contract names (`parse_tga`, `parse_header`, the
`encode_tga_*` writers, the one-field `parse_tga_<field>` readers,
`register_runtime`) remain for one release as `#[deprecated]` wrappers
that produce byte-identical results; see the CHANGELOG for the map.

## Framework use

The default `registry` feature pulls in `oxideav-core` and adds the
thin adapters over the same functions:

```rust
let mut ctx = oxideav_core::RuntimeContext::new();
oxideav_tga::register(&mut ctx);               // codec "tga" + container "tga" (.tga/.targa/.tpic/.vda/.icb/.vst)
// or the sub-registries: register_codecs(&mut ctx.codecs), register_containers(&mut ctx.containers),
// register_registries(&mut codecs, &mut containers)
```

* `make_decoder(&CodecParameters)` / `make_encoder(&CodecParameters)` —
  the factories the registry installs. The decoder emits the native
  layout exactly as `decode` returns it — `Gray8` for the grayscale
  types, `Pal8` + the palette side-channel for the colour-mapped types,
  `Rgb24` for 24-bit true colour, `Rgba` for 32-bit (and for 15 / 16-bit
  pixels, which have no core layout) — which is what the demuxer's
  stream parameters declare; nothing is pre-converted to `Rgba`, and no
  colour signal is stamped (TGA defines no colour space; the sRGB-like
  default is a documented convention). Convert through `oxideav-pixfmt`,
  or use `decode_rgba8` standalone;
  `make_decoder_with_display_options(TgaDisplayOptions)` finalises each
  frame through the display pipeline below instead. The encoder accepts
  `Rgba` / `Rgb24` / `Gray8` / `Pal8` (+ palette side-channel) frames
  and writes them with `EncodeOptions::default()`.
* Frame bridge: `From<TgaImage> for VideoFrame` (palette → the frame's
  RGB palette side-channel, colour → the colour-signal side-channel
  when specified), `TgaImage::from_video_frame(&VideoFrame,
  &CodecParameters) -> Result` and `TryFrom<(&VideoFrame,
  &CodecParameters)>`; `TgaPixelFormat` ↔ `oxideav_core::PixelFormat`
  map 1:1 by name (`From` / `TryFrom`).
* `oxideav_meta::register_all` dispatches here through the
  `oxideav_core::register!` entry point.

## Supported layouts

Decode (`decode` returns the native layout; the on-disk BGR(A) byte
order is swizzled to RGB(A)):

| Type | Compression  | Pixel                          | Native layout          |
| ---- | ------------ | ------------------------------ | ---------------------- |
| 1    | uncompressed | 8 bpp + palette (15/16/24/32)  | `Pal8` + `palette`     |
| 2    | uncompressed | 24 bpp                         | `Rgb24`                |
| 2    | uncompressed | 32 bpp                         | `Rgba`                 |
| 2    | uncompressed | 15 / 16 bpp                    | `Rgba` (expanded, see below) |
| 3    | uncompressed | 8 bpp grayscale                | `Gray8`                |
| 9    | RLE          | 8 bpp + palette                | `Pal8` + `palette`     |
| 10   | RLE          | 15 / 16 / 24 / 32 bpp          | `Rgb24` / `Rgba`       |
| 11   | RLE          | 8 bpp grayscale                | `Gray8`                |

Encode (`encode` writes the layout as given — never a silent
conversion; `encode_rgb8` → `Rgb24`, `encode_rgba8` → `Rgba`):

| Layout            | Image type (`rle` true / false) | Depth on the wire                       |
| ----------------- | ------------------------------- | --------------------------------------- |
| `Pal8` + palette  | 9 / 1                           | 8-bit indices + 15/16/24/32-bit colour map |
| `Rgb24`           | 10 / 2                          | 24 (BGR), 0 attribute bits              |
| `Rgba`            | 10 / 2                          | 32 (BGRA), 8 attribute bits (or `alpha_bits`) |
| `Gray8`           | 11 / 3                          | 8                                       |

* 15 / 16-bit true-colour pixels (`ARRRRRGG GGGBBBBB`, little-endian)
  have no `oxideav_core::PixelFormat` counterpart, so they are expanded
  at decode time exactly as 15 / 16-bit colour-map entries are: each
  5-bit channel becomes 8 bits by repeating its top three bits
  (`v << 3 | v >> 2`, so `0b11111` → `0xFF`), the 16-bit attribute bit
  becomes alpha `0xFF` / `0x00`, 15-bit is always opaque. There is no
  15 / 16-bit true-colour *encode* path (a `Pal8` image may still be
  written with a 15 / 16-bit colour map via `palette_entry_size`;
  that quantises).
* `Pal8` indices are rebased at decode so `0` is the first stored
  colour-map entry (the header's Color Map Origin is folded in); an
  index below the origin or past the map is `InvalidData`, so
  `to_rgb8` on a decoded image is exact. The encoder writes origin `0`.
* An `Rgba` image whose alpha is uniformly `0xFF` is still written at
  32 bits so that `decode(encode(img)) == img`; set
  `EncodeOptions::drop_opaque_alpha` for the pre-contract auto-depth
  rule (24 bits when opaque — the deprecated writers do this).
* `decode(encode(img)) == img` (planes, palette, `metadata.gamma`,
  extension record) is pinned for every layout × `rle` × `row_order`
  (`tests/contract.rs`, `fuzz/fuzz_targets/contract_identity.rs`).
* Unrepresentable inputs are `Error::Unsupported`: more than 256
  colours into a colour map (`to_indexed`), dimensions over 65535. A
  `Pal8` image without a palette or with an index outside it, or a
  plane shorter than its geometry, is `Error::InvalidData`.

## Options

`DecodeOptions` (`Default`, `with_*`, `unlimited()`): `max_width` /
`max_height` / `max_pixels: Option<u64>` / `max_bytes: Option<u64>`
(`None` = unlimited; defaults: no dimension / pixel limit — TGA
geometry is `u16` — and 1 GiB of decoded plane) and `strict: bool`
(default `false`). Limits are checked against the header before the
plane is allocated (`Error::LimitExceeded`). `strict` turns the spec's
*should* rules into errors: an RLE packet spanning a scan-line boundary
(§C.5) and a non-zero §C.2 interleaving flag (descriptor bits 7-6).

`EncodeOptions` (`#[non_exhaustive]`, `Default`, `with_*`) — every
behaviour is a field:

| Field | Default | Meaning |
| --- | --- | --- |
| `rle: bool` | `true` | image types 9 / 10 / 11 (§C.5 run-length) vs 1 / 2 / 3 |
| `row_order: RowOrder` | `TopDown` | descriptor bit 5; `BottomUp` stores the last row first |
| `alpha_bits: Option<u8>` | `None` (= 8) | descriptor attribute-bit count for `Rgba`; `0` = "TARGA-32", fourth byte not alpha |
| `drop_opaque_alpha: bool` | `false` | write a uniformly opaque `Rgba` image at 24 bits |
| `palette_entry_size: Option<ColorMapEntrySize>` | `None` | colour-map entry width; `None` picks the narrowest lossless (24 opaque / 32 with alpha); 15 / 16 quantise |
| `image_id: Vec<u8>` | empty | §C.3 Image Identification Field, 0..=255 bytes |
| `screen_origin: ImageOrigin` | `(0, 0)` | §5.1 / §5.2 lower-left on-screen placement |
| `extension: Option<ExtensionAreaInput>` | `None` | explicit TGA 2.0 extension area (+ postage stamp / colour-correction table / scan-line table / developer tags) |
| `footer: bool` | `false` | force a marker-only TGA 2.0 footer even with nothing to point at |

The RLE packetiser emits a run packet for ≥ 2 consecutive identical
pixels (max 128) and raw packets otherwise (max 128), never crossing a
scan line, so a §C.6.9 scan-line table can always describe the output.

## Metadata and colour

* `Metadata { icc, exif, xmp, gamma }`: TGA has no ICC / Exif / XMP
  carrier (always `None` on decode, ignored on encode); `gamma` is the
  TGA 2.0 extension area's §C.6.6 Gamma Value (`numerator /
  denominator`, `None` when absent / unset / zero denominator). Decode
  never applies it — `decode_tga_for_display` does. On encode a
  `metadata.gamma` is written into the extension area (`(round(g ×
  100), 100)`): into `EncodeOptions::extension` when given, else into
  the image's own `extension` record, else into a minimal area of its
  own.
* `TgaImage::extension: Option<TgaExtensionArea>` is the typed §C.6
  record (author, comments, timestamp, job, software, key colour, pixel
  aspect, gamma, attributes type, table / stamp offsets). `encode`
  re-emits it when `EncodeOptions::extension` is `None`
  (`ExtensionAreaInput::from_area`; the three table / stamp pointers
  are not carried — supply them explicitly).
* `ColorInfo { range, primaries, transfer, matrix }`: TGA signals no
  colour space, so `decode` always fills `ColorInfo::tga_default()` —
  full range, identity matrix (`0`), unspecified primaries and transfer
  (`2`). That is the crate's documented convention, not a value read
  from the file.
* `ImageInfo` extras beyond the contract floor: `image_type`, `depth`,
  `attribute_bits`, `top_down`, `right_to_left`, `color_map_length`,
  `color_map_entry_size`, `image_id_length`, `has_footer`,
  `has_extension_area`, `has_developer_area`. `has_alpha` is true for
  32- and 16-bit true colour and for 16 / 32-bit colour maps (whether
  the alpha is *meaningful* is the display pipeline's question).

## Limits

* `probe` is total and allocation-free; TGA has no leading magic, so it
  is a plausibility test (known image type at an allowed depth with
  non-zero geometry), with the `TRUEVISION-XFILE.\0` footer as the
  definite signal.
* `info` / `header` read the 18-byte header (and the footer) only.
* Every function returns `Error`, never panics, on hostile input; the
  fuzz targets cover `probe` / `info` / `decode` / `decode_with` /
  `decode_rgb8` / `decode_rgba8` and the encode↔decode identity.
* `Error` (= `TgaError`): `InvalidData(String)`, `Unsupported(String)`,
  `LimitExceeded(String)`, `Io(std::io::Error)` (+ `From<std::io::Error>`);
  `std::error::Error` + `Display`, no `Clone` / `PartialEq`.

## TGA 2.0 structures and readers

The structural readers keep their names (they are depth, not the
contract floor): `parse_tga_footer`, `parse_tga_extension_area`,
`parse_tga_postage_stamp` / `parse_tga_postage_stamp_dimensions`,
`parse_tga_colour_correction_table`, `parse_tga_scan_line_table` /
`compute_tga_scan_line_table` / `parse_tga_scan_line`,
`parse_tga_developer_area`, `parse_tga_color_map` /
`parse_tga_border_color`, `parse_tga_image_id`,
`resolve_alpha_with_targa32_fallback` / `resolve_alpha_from_descriptor`.
The one-field conveniences (`parse_tga_gamma`, `parse_tga_key_color`,
…, `parse_tga_attribute_bits`, `parse_tga_image_origin`, …) are
deprecated in favour of the typed accessors on `TgaExtensionArea` /
`TgaHeader` (`decode(..).extension`, `header(..)`). The thumbnail and
single-row readers return the legacy expanded layout (`Gray8` or
`Rgba`).

* Colour-mapped types (1 / 9) honour the §C.2 Color Map Origin (the
  "index of first color map entry", header bytes 3-4): the on-disk
  palette stores `cmap_length` entries beginning at that origin, so an
  image index `idx` addresses on-disk entry `idx - cmap_first`. Indices
  below the origin or past the stored length are rejected as out of
  range. Origin-0 files (everything this crate's encoder writes) are
  unaffected.
* 16-bit pixels decoded as A1R5G5B5 (top bit alpha); 15-bit forces
  alpha to `0xFF`.
* 24-bit pixels are BGR on disk, 32-bit are BGRA — both swizzled to
  RGB / RGBA at decode time (`Rgb24` / `Rgba`).
* Both image-descriptor ordering bits are honoured per the TGA 2.0 FFS
  image-descriptor field (Table 2 - Image Origin): bit 5 selects
  bottom-up vs top-down rows and bit 4 selects right-to-left vs
  left-to-right columns. Output is always normalised to a top-down,
  left-to-right origin — rows are flipped when bit 5 is clear and
  columns are mirrored when bit 4 is set (both axes for a file that sets
  bit 4 with bit 5 clear, i.e. a 180° rotation).
* The image-descriptor **data-storage interleaving flag** (Field 5.6, bits
  7-6) is surfaced as a typed `Interleaving` view via
  `TgaHeader::interleaving()` (reach the header with `header(&bytes)`)
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
* The §5.1 / §5.2 **Image Origin** (fixed-header Fields 5.1 + 5.2, bytes
  8-11) — the on-screen X/Y placement of the image's lower-left corner in
  the TARGA framebuffer's lower-left-origin coordinate system — is surfaced
  as a typed `ImageOrigin` view via `TgaHeader::image_origin()` (`x` /
  `y`, plus `ORIGIN`
  `(0,0)` sentinel / `new` / `from_header` / `as_tuple` / `from_tuple` /
  `is_origin` / `is_offset` / `to_bytes`). This is the screen-placement
  coordinate, orthogonal to the image-descriptor storage-order bits
  (`is_top_down` / `is_right_to_left`): a file can be stored top-down yet
  declare a non-zero on-screen origin. The decoder does not relocate the
  raster (it always emits a single normalised top-down, left-to-right
  image); the view lets a caller doing its own on-screen compositing read
  and honour the placement. Like the §C.2 attribute-bit count and
  interleaving flag, the field lives in the fixed header so the view works
  on TGA 1.0 files too. Mirrors the surface-but-classify approach of
  `AttributeBits` / `Interleaving` / `ColorMapType`.
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
* The §C.6.10 stamp is also *generated*, not just carried:
  `PostageStamp::recommended_for(src_w, src_h)` derives the thumbnail
  geometry for a source frame — the longer edge scaled down to the
  recommended 64-pixel cap, the shorter edge by the same ratio
  (aspect-ratio preserving, downscale-only so a source already ≤ 64×64
  is its own stamp, each axis floored at 1, degenerate source →
  `UNSET`). `PostageStamp::subsample(&image)` then builds the thumbnail
  by **point sub-sampling** (nearest-neighbour) a decoded `TgaImage` at
  that size, returning a new image in the source's pixel format
  (any layout, palette carried along) feedable straight to
  `ExtensionAreaInput::postage_stamp`. Per Field 26's "create one using
  sub-sampling techniques" and "If the original image is color mapped,
  DO NOT average the postage stamp, as you will create new colors not in
  your map", point sub-sampling only ever copies whole source pixels and
  never averages two colours into a third — safe for colour-mapped
  parents. Returns `None` (caller omits the stamp) for an empty source or
  one already within the recommended box, matching the no-op convention
  of `PixelAspectRatio::resampled`.
* The §C.6.13 attributes byte is exposed as both the raw
  `attributes_type: u8` and a typed `AttributesType` enum reachable
  via `TgaExtensionArea::attributes()` (`NoAlpha` / `UndefinedIgnore`
  / `UndefinedRetain` / `UsefulAlpha` / `PremultipliedAlpha` plus a
  `Reserved(u8)` catch-all so non-standard files round-trip
  bit-exactly). `TgaExtensionArea::attributes` reads the typed attribute
  straight from a file's footer/extension area.
* The attributes type is also *applied*, not just carried:
  `AttributesType::normalize_rgba8` maps a decoded `[R,G,B,A]` pixel to
  straight (non-premultiplied) alpha — `NoAlpha`/`UndefinedIgnore` force
  opaque, `UndefinedRetain`/`UsefulAlpha`/`Reserved` pass through, and
  `PremultipliedAlpha` un-premultiplies each colour channel
  (`straight = round(stored × 255 / A)`, clamped; fully-transparent
  pixels become transparent black) per the spec's Porter-Duff example.
  `apply_to_image(&mut TgaImage)` rewrites a decoded image in place
  (`Rgba` pixels, or the palette entries of a `Pal8` image; `Rgb24` /
  `Gray8` have no alpha and are left untouched).
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
  line") is what makes the table well-defined, and `decode` still
  decodes such files whole. `parse_tga_scan_line(input, &table,
  index)` then does the random access the spec built the table for:
  decode exactly one row (no preceding rows touched), returning a
  1-pixel-tall `TgaImage` in the legacy expanded layout (`Gray8`, else
  `Rgba`: palette lookup, BGR→RGBA swap, A1R5G5B5 expansion, bit-4
  column mirroring); `index` is in the table's saved order, so
  bottom-up files address display row `height − 1 − y` at entry `y`.
  A table computed against a freshly-encoded base file can be handed
  straight to `ExtensionAreaInput::scan_line_table` — the extension
  writer only appends after the pixel data, so the offsets stay valid
  in the extended file.
* The §C.7 developer-area tag directory is exposed as
  `TgaDeveloperArea` (a `Vec<TgaDeveloperTag>` with `tag_id` / `offset`
  / `size`), parsed by `parse_tga_developer_area`; each tag's
  application-defined payload bytes are borrowable via
  `dev.payload(input, tag)`, or in one call straight from a tag id with
  `dev.payload_by_id(input, tag_id)` (the spec §C.7 "tags may appear in
  any order" lookup — `find` + `payload` composed, first match in
  directory order, `None` for a missing id or a payload-less marker).
  Typed accessors on both structs (matching
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
  with no pixel array) that `decode` rejects as having no image.
  Entries are de-interleaved into straight RGBA exactly as `decode`
  expands the palette internally (15/16-bit A1R5G5B5, 24-bit BGR, 32-bit
  BGRA), and the Color Map Origin is recorded so a logical pixel index
  `idx` resolves to `entries[idx - first_index]`. Returns `None` when the
  Color Map Type byte is `0` (no map present).
* The §C.2 **Color Map Type** byte (fixed-header byte 1 — "0 means no
  color map is included … 1 means a color map is included") is surfaced
  as a typed `ColorMapType` view (`Absent` / `Present` / `Reserved(u8)`
  for the non-conformant non-`0`/`1` values the spec never defines)
  reachable via `TgaHeader::color_map_type()`, mirroring the `Interleaving` /
  `AttributeBits` surface. Predicates: `is_absent` / `is_present` /
  `is_reserved` / `is_conformant` (one of the two spec-legal values) /
  `has_map_data` (the decoder's lenient "any non-zero byte means map
  data follows" reading) plus `to_u8` for a bit-exact round trip.
* The §C.2 **TIPS border / background colour** is *applied*, not just
  skipped: the spec notes that for an **un**mapped image type (2 / 3 /
  10 / 11) a colour map may still be present, and "TIPS (a Targa paint
  system) will set the border color [to] the first map color if it is
  present." The main decode path reads and skips that vestigial map;
  `parse_tga_border_color` surfaces its first entry as straight RGBA.
  Returns `None` for a colour-mapped image type (1 / 9 — there the map
  is the working palette, so the caller uses `parse_tga_color_map`) and
  for a map-absent file. De-interleaving matches `parse_tga_color_map`
  exactly (15/16-bit A1R5G5B5, 24-bit BGR, 32-bit BGRA).
* The §3.3 / §C.3 Image Identification Field (the free-form,
  up-to-255-byte block at offset 18) is exposed verbatim by
  `parse_tga_image_id`; the helper returns the borrowed byte slice, an
  empty slice for the common `id_length == 0` case, or `None` when the
  buffer is truncated. Content is left untouched (no NUL trimming, no
  UTF-8 decode) because the spec leaves the format unconstrained.
* The §C.6.4 / §C.6.5 / §C.6.6 / §C.6.7 numeric extension-area fields are
  surfaced as typed views in addition to the raw on-disk tuples:
  `KeyColor` (§C.6.4) wraps the `[A,R,G,B]` quadruple with `from_argb` /
  `to_argb` / `as_rgba8` / `is_unset` / `has_alpha`. It is also
  *applied*, not just carried: spec §C.6.4 calls Field 18 the image's
  "transparent colour" (the colour the screen is cleared to), so
  `key_out_image(&mut img)` chroma-keys a decoded RGBA image — every
  pixel whose R/G/B equals the key colour has its alpha set to `0`
  (colour bytes untouched), and the call returns the number of pixels
  keyed out. `matches_rgb` / `matches_rgba` are the underlying match
  predicates (RGB-only vs exact four-channel; the RGB-only form is the
  useful one because the decoder forces alpha to `0xFF` for no-alpha
  24-bpp source files). Keying is a no-op (returns `0`) for `Rgb24` /
  `Gray8` images, which carry no alpha channel. `PixelAspectRatio`
  (§C.6.5) wraps `(numerator, denominator)` with `is_unset` / `is_square`
  / `as_f32` / `corrected_display_height(h)` / `corrected_display_width(w)`
  for square-display-pixel resampling. The aspect ratio is also
  *applied*, not just carried: `corrected_display_dimensions(w, h)`
  picks the square-pixel target size by **upscaling the shorter pixel
  axis only** (wide pixels stretch width, tall pixels stretch height —
  so no source sample is ever dropped); `resampled(&img)` produces a new
  nearest-neighbour-resampled `TgaImage` at that size (every pixel
  format — RGBA / Rgb24 / Gray8); and `apply_to_image(&mut img)` is the
  in-place companion. Unset / square / empty inputs are no-ops (`None` /
  image left untouched), matching the `GammaValue` apply-path
  semantics. `GammaValue` (§C.6.6) wraps the
  same SHORT pair with `is_unset` / `is_identity` / `as_f32` plus
  `apply_to_channel8` / `apply_to_rgba8` / `apply_to_image` (RGBA / Rgb24
  / Gray8) that raises each channel to the gamma exponent
  (`y = (x/255)^gamma × 255`, rounded, clamped, alpha untouched) — unset
  / identity / malformed gammas are bit-exact no-ops; `SoftwareVersion`
  (§C.6.7) wraps `(SHORT, BYTE-as-char)` with `is_unset` / `as_f32`
  (`number_times_100 / 100.0`). The typed views are reachable via
  `TgaExtensionArea::{key_color_typed, pixel_aspect_ratio_typed,
  gamma_typed, software_version_typed}` on `decode(..).extension` /
  `parse_tga_extension_area(..)` (both `None` for a TGA 1.0 file or any
  file without an extension area).

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
  job_name_typed, software_id_typed}`. Per-field constants
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
  / `job_time_typed`.

* The §C.2 Image Descriptor **attribute-bit count** (Field 5.6, bits 3-0
  — "the number of attribute bits per pixel … designated as Alpha Channel
  bits") is exposed as a typed `AttributeBits` view via
  `TgaHeader::attribute_bits()` (`count` / `is_none` / `has_alpha` /
  `is_canonical_for_depth` — 8 for 32 bpp, 1 for 16 bpp, 0 for the
  alpha-less depths) and read straight from a file by
  `TgaHeader::attribute_bits`. Unlike the extension-area `AttributesType`
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


## Display pipeline (composed metadata application)

`decode` is deliberately a **raw** decoder: it unpacks the on-disk
pixel array (BGR→RGB swap, 5-5-5 expansion, RLE) and normalises the
storage order, but applies **none** of the file's own §C.6 metadata. Every apply-path above is an isolated opt-in helper. The
composed counterpart, `decode_tga_for_display(input, &TgaDisplayOptions)`,
decodes a TGA and then applies the file's own metadata in **spec-faithful
order** to produce a display-ready frame:

1. **Alpha resolution** — the §C.6.13 Attributes Type (Field 24) when a
   TGA 2.0 extension area is present, otherwise the §C.2 header
   attribute-bit count (Field 5.6, present in every file including TGA
   1.0). The spec ties the two together (a Field-24 value of `0` says
   "bits 3-0 of field 5.6 should also be set to zero"), so Field 24 is the
   authoritative source and the header bits are the 1.0-compatible
   fallback. Pre-multiplied alpha is un-multiplied here, recovering
   straight colour **before** the tone curves run (the un-multiply is the
   inverse of a linear alpha scale, so it belongs on the pre-curve
   samples).
2. **Tone reproduction** — the §C.6.8 colour-correction table **or** the
   §C.6.6 gamma value, **never both**. The spec makes them mutually
   exclusive: Field 21 (Color Correction Offset) says to zero the offset
   "if … the Gamma Value setting is sufficient". When a correction table
   is present the pipeline applies it and skips gamma; otherwise it applies
   gamma. (Applying both would double-correct.)
3. **Key colour** (§C.6.4, the "background / transparent colour the screen
   would be cleared to") — a compositing decision made against the
   *post-tone* colour, so it runs after the tone curves.
4. **Pixel aspect ratio** (§C.6.5) — a geometry change (resample to square
   display pixels). It runs **last** so the per-pixel colour passes above
   operate on the stored raster, not an already-stretched one.

Each pass is independently gated by the typed `TgaDisplayOptions` (`ALL` /
`NONE` constants, `with_resolve_alpha` / `with_apply_tone` /
`with_apply_key_color` / `with_apply_pixel_aspect` builders,
`is_passthrough`). `TgaDisplayOptions::default()` enables all four passes;
`TgaDisplayOptions::NONE` makes `decode_tga_for_display` byte-identical to
`decode(..).into_legacy_layout()` (the pipeline runs on the expanded
`Gray8`-or-`Rgba` shape). `decode_tga_for_display_reported` additionally returns a
`TgaDisplayReport` describing what each pass did (`AlphaResolution` /
`ToneApplied` / keyed-pixel count / resampled flag, plus `applied_tone` /
`applied_key_color` / `is_colour_geometry_noop` predicates) so a caller can
audit which rule fired. The spec does not state a single canonical
pipeline; the order above is derived from the per-field constraints and is
the crate's defensible reading — where the spec is silent on a pass
boundary (e.g. the exact placement of the pre-multiplied-alpha un-multiply
relative to gamma) the choice is documented here as the crate's decision.
`decode`'s raw contract is unchanged; the pipeline is strictly additive
and works in both the registry and standalone builds.

`decode_tga_frame(input, &options)` returns a `TgaDecodedFrame` bundling the
finalized pixels + `TgaDisplayReport` **with** the file metadata the
pipeline does not fold into the raster: the §5.1/§5.2 on-screen `ImageOrigin`
placement (`screen_origin` / `has_screen_offset`), the §C.6.10 postage-stamp
thumbnail decoded to the main image's format (`postage_stamp` /
`has_postage_stamp`), and the §C.7 developer-area directory
(`developer_area`). One decode call replaces a re-walk with the individual
`parse_tga_*` helpers for a caller doing its own on-screen compositing or
building a thumbnail strip. A malformed thumbnail in an otherwise-valid file
is non-fatal (`postage_stamp: None`).

For the framework `Decoder` trait, the default `make_decoder` factory stays
a raw `decode` passthrough; `make_decoder_with_display_options`
builds a decoder that finalizes each frame through this pipeline.

`splice_image_id(&mut base, image_id_bytes)?` rewrites a freshly-encoded
base TGA to carry an Image Identification Field (spec §3.3 / §C.3): the
helper sets byte 0 to the supplied length and inserts the bytes at offset
18, shifting the colour-map + pixel + any trailing extension area / footer
by the same number of bytes. The cap is the spec maximum of 255 bytes
(`TGA_IMAGE_ID_MAX`); an empty input is a no-op; calling the helper twice
on the same file is rejected so an existing ID is never overwritten.

`set_image_origin(&mut base, origin)?` overwrites the §5.1 / §5.2 Image
Origin (fixed-header X/Y coordinates at bytes 8-11) on a freshly-encoded
base TGA — the base writers always emit the screen-origin default
`(0, 0)`. Unlike `splice_image_id`, the write does **not** change the file
length (both coordinates live in the fixed 18-byte header), so it composes
freely with `splice_image_id` / `encode_tga_with_extension` in any order
and the write round-trips bit-exactly through `TgaHeader::image_origin`.

`encode_tga_with_extension(base, &ExtensionAreaInput { … })` wraps
any encoded TGA byte stream and appends a TGA 2.0 footer + extension
area body to the output — the byte-level form of
`EncodeOptions::extension`. Optional companions on `ExtensionAreaInput`:

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

## Fuzzing

Five cargo-fuzz harnesses live under `fuzz/`. `.github/workflows/fuzz.yml`
runs them daily, sharing a 30-minute budget (the reusable workflow
auto-discovers `fuzz/fuzz_targets/*.rs` and splits the time evenly). No
external oracle is reachable (the clean-room wall bars third-party TGA
implementations), so no target compares pixels against another decoder.

* `decode_tga` — attacker-supplied bytes through the contract API
  (`probe` / `info` / `header` / `decode` / `decode_with` under a 16 MiB
  byte limit and in strict mode / `decode_rgb8` / `decode_rgba8`), every
  structural `parse_tga_*` reader (footer / extension area / postage
  stamp / §C.6.8 colour-correction table / §C.6.9 scan-line table +
  `compute_tga_scan_line_table` + `parse_tga_scan_line` / §C.7 developer
  area / Image ID / colour map / border colour) and the composed display
  pipeline across its option toggles. Purely panic-free: every call
  returns `Ok` or `Err`. A 16 MiB declared-raster cap gates the
  *unlimited* `decode` (a `u16 × u16 × 4` header can declare 16 GiB);
  `decode_with` runs under the same cap on every input to prove the
  limit check fails closed before allocating. Encoder-produced seeds
  live in `fuzz/corpus/decode_tga/`.
* `contract_identity` — `decode(encode(img)) == img` (format, plane
  bytes, palette, `metadata.gamma`) for every native layout (`Rgba` /
  `Rgb24` / `Gray8` / `Pal8` with a 24- or 32-bit colour map) across the
  lossless `EncodeOptions` axes — `rle`, `row_order`, `alpha_bits`,
  `image_id`, `screen_origin`, `footer`, a gamma that must come back
  through the extension area — plus `probe` / `info` agreement on the
  encoder's own output.
* `roundtrip_identity` / `encode_roundtrip` — the pre-contract writers
  (deprecated wrappers) through the decoder: bit-exact identity for the
  lossless ones (RGBA / RGB24 / Gray8 / default palette / explicit
  `Bits24` / `Bits32` colour maps), frame shape for all twelve selectors
  including the quantising 15 / 16-bit colour-map sizes. These pin the
  wrappers' byte-identity with earlier releases.
* `extension_roundtrip` — `encode_tga_with_extension` with an
  `ExtensionAreaInput` built from fuzz bytes (gamma / key colour /
  aspect ratio / software version / attributes type / colour-correction
  table / scan-line table / postage stamp / developer tags), re-read
  through the extension-area readers: footer recognised, table bytes
  and SHORT pairs back exactly. Exercises the offset back-patch
  arithmetic nothing else reaches.

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
| encode  | `uncompressed_rgba`   | `encode_rgba8`, `rle = false`, `drop_opaque_alpha` (24/32) |
| encode  | `uncompressed_rgb24`  | `encode_rgb8`, `rle = false` (fixed 24 bpp)          |
| encode  | `rle_rgba_runs`       | `encode_rgba8`, RLE, run-heavy input                 |
| encode  | `rle_rgba_noise`      | `encode_rgba8`, RLE, high-entropy input              |

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
