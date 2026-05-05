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
* Image-descriptor bit 5 honoured: bottom-up files are flipped to
  top-down on the output. Bit 4 (right-to-left columns) is rejected
  rather than producing silently-mirrored pixels — no real-world TGA
  encoder sets it.
* The optional 26-byte TGA 2.0 footer is recognised; its
  extension-area / developer-area pointers are surfaced via
  [`parse_tga_footer`]. Full extension-area body parse (gamma,
  software ID + version, postage-stamp thumbnail, colour-correction
  table) is on the round-2 roadmap.

## Encode

* `encode_tga_uncompressed(w, h, &rgba)` writes image type 2.
* `encode_tga_rle(w, h, &rgba)` writes image type 10. RLE packets are
  selected per spec §C.5: a run of ≥ 2 consecutive identical pixels
  is emitted as a run-length packet (max 128 pixels per packet);
  isolated pixels are coalesced into raw packets (max 128 pixels per
  packet). RLE doesn't cross scanline boundaries.

Output depth is auto-selected: 32 bpp if any input alpha byte is
`< 0xFF`, otherwise 24 bpp. Top-down origin (descriptor bit 5 set)
is used unconditionally.

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

## Lacks

Round 1 limits — followups for round 2:

* Full TGA 2.0 extension area body parse (gamma, software ID +
  version, postage-stamp thumbnail, colour-correction table,
  attributes type, time/date stamp, …).
* Postage-stamp / thumbnail extraction.
* Write paths for image types 1 / 3 / 9 / 11 (palette + grayscale).
