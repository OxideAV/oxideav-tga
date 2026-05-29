# oxideav-tga

Pure-Rust Truevision TGA (TARGA) reader/writer for the
[`oxideav`](https://github.com/OxideAV/oxideav) framework.

Clean-room implementation of the public **Truevision TGA File Format
Specification v2.0** (1989). No `image` crate's TGA submodule,
FreeImage, DevIL, GIMP TGA plugin, or NetPBM `targatoppm` source
consulted.

## Decode

| Type | Compression  | Pixel                        | Output  |
| ---- | ------------ | ---------------------------- | ------- |
| 1    | uncompressed | 8 bpp + palette (15/16/24/32) | `Rgba`  |
| 2    | uncompressed | 15 / 16 / 24 / 32 bpp        | `Rgba`  |
| 3    | uncompressed | 8 bpp grayscale              | `Gray8` |
| 9    | RLE          | 8 bpp + palette              | `Rgba`  |
| 10   | RLE          | 15 / 16 / 24 / 32 bpp        | `Rgba`  |
| 11   | RLE          | 8 bpp grayscale              | `Gray8` |

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
* The optional 26-byte TGA 2.0 footer is recognised. Use
  `parse_tga_footer` for the extension/developer-area offsets,
  `parse_tga_extension_area` for the 495-byte extension-area body
  (author / comments / timestamp / job / software ID + version /
  pixel-aspect / gamma / colour-correction + postage-stamp +
  scan-line offsets / attributes-type), and `parse_tga_postage_stamp`
  to pull out the embedded thumbnail.
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
  parsed by `parse_tga_scan_line_table`.
* The §C.7 developer-area tag directory is exposed as
  `TgaDeveloperArea` (a `Vec<TgaDeveloperTag>` with `tag_id` / `offset`
  / `size`), parsed by `parse_tga_developer_area`; each tag's
  application-defined payload bytes are borrowable via
  `dev.payload(input, tag)`.

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

`encode_tga_with_extension(base, &ExtensionAreaInput { … })` wraps
any of the writers above and appends a TGA 2.0 footer + extension
area body to the output. Optional companions on `ExtensionAreaInput`:

* `postage_stamp: Option<TgaImage>` — embedded thumbnail in the
  parent's pixel format (§C.6.10).
* `colour_correction_table: Option<TgaColourCorrectionTable>` —
  4 × 256 × u16 ARGB curves (§C.6.8), 2048 bytes on disk.
* `scan_line_table: Option<TgaScanLineTable>` — `height × u32` per-row
  byte offsets for partial-image readers (§C.6.9).
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

A cargo-fuzz harness under `fuzz/` drives the public decoder surface
(`parse_tga` + every `parse_tga_*` helper for footer / extension /
postage stamp / colour-correction / scan-line / developer area) with
arbitrary bytes; the contract is panic-free regardless of how hostile
the input is. A 16-MiB declared-raster cap inside the harness mirrors
what a real demuxer's sanity limits would enforce
(`width × height × 4 ≤ 16 MiB`); the library itself keeps no policy
cap. `.github/workflows/fuzz.yml` runs the harness daily for a
30-minute budget; eleven encoder-produced seeds live in
`fuzz/corpus/decode_tga/` (the eleventh is the reproducer for the
round-7 postage-stamp `validate_depth` regression below, kept as a
sentinel against future re-introduction).

A round-7 catch from this harness: `parse_tga_postage_stamp` was
driving `decode_raw_pixels` against the parent header's pixel depth
without first running `validate_depth`. Files crafted with a
`postage_stamp_offset` set against an `image_type` / `pixel_depth`
combination the spec rejects (e.g. type 2 + depth 4, type 3 + depth 0)
reached `emit_pixel`'s `unreachable!()` arm and panicked. The main
decoder (`parse_tga`) already validated depth pre-decode; the
postage-stamp path now mirrors that, and both `unreachable!()` arms in
`emit_pixel` have been replaced with returned `Err`s so future callers
fail closed.

```sh
cargo +nightly fuzz run decode_tga -- -runs=10000
```

## Lacks

* Image type 32 / 33 (compressed colour-mapped, Huffman + delta /
  4-pass) — Truevision never shipped them; no fixtures exist in the
  wild and the spec describes the format only at a high level.
* Application-level semantics for developer-area tag IDs — the spec
  declines to enumerate well-known IDs, so the parser surfaces tags
  + payload byte ranges and leaves interpretation to the caller.
