//! Pure-Rust Truevision TGA (TARGA) reader/writer.
//!
//! Clean-room implementation of the public **Truevision TGA File
//! Format Specification v2.0** (1989, 39-page PDF). No `image` crate
//! TGA submodule, FreeImage, DevIL, GIMP TGA plugin, or NetPBM
//! `targatoppm` source consulted.
//!
//! ## Read coverage
//!
//! | Type | Compression | Pixel | Output |
//! | ---- | ----------- | ----- | ------ |
//! | 1    | uncompressed | 8 bpp + palette (15/16/24/32-bit entries) | `Rgba` |
//! | 2    | uncompressed | 15 / 16 / 24 / 32 bpp                     | `Rgba` |
//! | 3    | uncompressed | 8 bpp grayscale                           | `Gray8` |
//! | 9    | RLE          | 8 bpp + palette                           | `Rgba` |
//! | 10   | RLE          | 15 / 16 / 24 / 32 bpp                     | `Rgba` |
//! | 11   | RLE          | 8 bpp grayscale                           | `Gray8` |
//!
//! Bottom-up and top-down row orders are auto-detected from
//! image-descriptor bit 5; output is always normalised to top-left
//! origin. The optional 26-byte TGA 2.0 footer is recognised and its
//! extension-area / developer-area pointers are surfaced via
//! [`parse_tga_footer`]; the extension-area body parse (gamma, software
//! ID + version, postage-stamp thumbnail, colour-correction table) is
//! deferred to round 2.
//!
//! ## Write coverage
//!
//! * [`encode_tga_uncompressed`] — image type 2, 24 / 32 bpp.
//! * [`encode_tga_rle`] — image type 10, 24 / 32 bpp.
//!
//! Output depth is auto-selected: 32 bpp if any input alpha byte is
//! `< 0xFF`, otherwise 24 bpp. Top-down origin (descriptor bit 5 set)
//! is used unconditionally.
//!
//! ## Standalone vs registry-integrated
//!
//! The crate's default `registry` Cargo feature pulls in `oxideav-core`
//! and exposes the framework `Decoder` / `Encoder` trait surface plus
//! a [`registry::register`] entry point. Disable the feature
//! (`default-features = false`) for an `oxideav-core`-free build that
//! still exposes the standalone [`parse_tga`] / [`encode_tga_uncompressed`]
//! / [`encode_tga_rle`] API plus crate-local [`TgaImage`] /
//! [`TgaPixelFormat`] / [`TgaError`] types.

#[cfg(feature = "registry")]
pub mod container;
pub mod decoder;
pub mod encoder;
pub mod error;
pub mod image;
#[cfg(feature = "registry")]
pub mod registry;
pub mod types;

/// Codec id for TGA image frames.
pub const CODEC_ID_STR: &str = "tga";

pub use decoder::{parse_tga, parse_tga_footer};
pub use encoder::{
    encode_tga_rle, encode_tga_rle_image, encode_tga_uncompressed, encode_tga_uncompressed_image,
};
pub use error::{Result, TgaError};
pub use image::{TgaImage, TgaPixelFormat};
pub use types::{
    parse_footer, parse_header, ImageType, TgaFooter, TgaHeader, TGA_FOOTER_MAGIC, TGA_FOOTER_SIZE,
    TGA_HEADER_SIZE,
};

#[cfg(feature = "registry")]
pub use registry::{register, register_codecs, register_containers};
