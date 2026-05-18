# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.2](https://github.com/OxideAV/oxideav-tga/compare/v0.0.1...v0.0.2) - 2026-05-18

### Other

- round 3: RGB24-input standalone entry points + hardening tests
- fix rustdoc broken intra-doc link to TgaError
- clean round-2 [Unreleased] above 0.0.1 release notes
- Round 2: type 1/3/9/11 writers + TGA 2.0 extension area + thumbnail
- release v0.0.1

### Added

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