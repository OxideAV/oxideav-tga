//! Pure-Rust Truevision TGA (TARGA) reader/writer.
//!
//! Clean-room implementation of the public **Truevision TGA File
//! Format Specification v2.0** (1989, 39-page PDF). Spec PDF + the
//! Gamers.org plain-text mirror of the same document are the only
//! sources consulted.
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
//! Both row order (image-descriptor bit 5) and column order (bit 4) are
//! auto-detected; bottom-up rows are flipped and right-to-left columns
//! are mirrored so output is always normalised to a top-down,
//! left-to-right origin. The optional 26-byte TGA 2.0 footer is
//! recognised — use
//! [`parse_tga_footer`] for the offsets, [`parse_tga_extension_area`]
//! for the 495-byte extension-area body (author / comments / timestamp
//! / software ID + version / aspect-ratio / gamma / attributes-type /
//! pointers to colour-correction table + postage-stamp + scan-line
//! tables), [`parse_tga_postage_stamp`] for the embedded thumbnail,
//! [`parse_tga_colour_correction_table`] for the §C.6.8 256×4×u16
//! interleaved-ARGB curves (which can then be *applied* to a decoded
//! image via [`TgaColourCorrectionTable::apply_to_image`] /
//! `correct_rgba8` / `correct_rgba16` / `correct_gray8`),
//! [`parse_tga_scan_line_table`] for the §C.6.9 per-row
//! byte offsets, and [`parse_tga_developer_area`] for the §C.7
//! application-extensible tag directory.
//!
//! ## Write coverage
//!
//! | Type | Compression | Pixel input        | API                       |
//! | ---- | ----------- | ------------------ | ------------------------- |
//! | 1    | uncompressed | RGBA → 8-bit index | [`encode_tga_palette`]    |
//! | 2    | uncompressed | RGBA / RGB         | [`encode_tga_uncompressed`] |
//! | 3    | uncompressed | Gray8              | [`encode_tga_grayscale`]  |
//! | 9    | RLE          | RGBA → 8-bit index | [`encode_tga_palette_rle`] |
//! | 10   | RLE          | RGBA / RGB         | [`encode_tga_rle`]        |
//! | 11   | RLE          | Gray8              | [`encode_tga_grayscale_rle`] |
//!
//! Output depth for true-colour writers is auto-selected: 32 bpp if
//! any input alpha byte is `< 0xFF`, otherwise 24 bpp. Palette writers
//! cap the input at 256 unique RGBA colours and emit a 32-bit BGRA
//! colour-map. Top-down origin (descriptor bit 5 set) is used
//! unconditionally.
//!
//! [`encode_tga_with_extension`] wraps any of the encoders above and
//! appends a TGA 2.0 footer + 495-byte extension-area body authored
//! from an [`ExtensionAreaInput`] (optionally with a postage-stamp
//! thumbnail).
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

pub use decoder::{
    parse_tga, parse_tga_attributes_type, parse_tga_author_comments, parse_tga_author_name,
    parse_tga_colour_correction_table, parse_tga_developer_area, parse_tga_extension_area,
    parse_tga_footer, parse_tga_gamma, parse_tga_image_id, parse_tga_job_name, parse_tga_job_time,
    parse_tga_key_color, parse_tga_pixel_aspect_ratio, parse_tga_postage_stamp,
    parse_tga_scan_line_table, parse_tga_software_id, parse_tga_software_version,
    parse_tga_timestamp, resolve_alpha_with_targa32_fallback,
};
pub use encoder::{
    encode_tga_grayscale, encode_tga_grayscale_rle, encode_tga_palette, encode_tga_palette_rle,
    encode_tga_rle, encode_tga_rle_image, encode_tga_rle_rgb24, encode_tga_uncompressed,
    encode_tga_uncompressed_image, encode_tga_uncompressed_rgb24, encode_tga_with_extension,
    splice_image_id, DeveloperTagInput, ExtensionAreaInput, TGA_IMAGE_ID_MAX,
};
pub use error::{Result, TgaError};
pub use image::{TgaImage, TgaPixelFormat};
pub use types::{
    parse_extension_area, parse_footer, parse_header, AttributesType, GammaValue, ImageType,
    JobTime, KeyColor, PixelAspectRatio, SoftwareVersion, TgaAsciiField, TgaAuthorComments,
    TgaColourCorrectionTable, TgaDeveloperArea, TgaDeveloperTag, TgaExtensionArea, TgaFooter,
    TgaHeader, TgaScanLineTable, TgaTimestamp, TGA_ASCII_FIELD_MAX_CHARS, TGA_AUTHOR_COMMENT_LINES,
    TGA_AUTHOR_COMMENT_LINE_BYTES, TGA_AUTHOR_COMMENT_LINE_MAX_CHARS,
    TGA_COLOUR_CORRECTION_TABLE_ENTRIES, TGA_COLOUR_CORRECTION_TABLE_SIZE, TGA_EXTENSION_AREA_SIZE,
    TGA_FOOTER_MAGIC, TGA_FOOTER_SIZE, TGA_HEADER_SIZE,
};

#[cfg(feature = "registry")]
pub use registry::{register, register_codecs, register_containers};
