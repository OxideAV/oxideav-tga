//! Pure-Rust Truevision TGA (TARGA) reader/writer.
//!
//! Clean-room implementation of the public **Truevision TGA File
//! Format Specification v2.0** (1989). Spec PDF + the plain-text
//! transcription of the same document are the only sources consulted.
//!
//! The crate follows the OxideAV image-crate contract
//! (`IMAGE_CRATE_API`): the same small root vocabulary every
//! `oxideav-<format>` picture crate exposes, usable with
//! `default-features = false` and no `oxideav-core`.
//!
//! ```no_run
//! let bytes = std::fs::read("in.tga")?;
//! if oxideav_tga::probe(&bytes) {
//!     let info = oxideav_tga::info(&bytes)?;        // header only
//!     let img = oxideav_tga::decode(&bytes)?;       // TgaImage, native layout
//!     let rgba: Vec<u8> = img.to_rgba8();           // packed RGBA, 4 * width bytes per row
//!     let (w, h) = (img.width(), img.height());
//!     let _ = info;
//!
//!     let opts = oxideav_tga::EncodeOptions::default().with_rle(false);
//!     let out = oxideav_tga::encode_rgba8(w, h, &rgba, &opts)?;
//!     std::fs::write("out.tga", out)?;
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Read coverage
//!
//! | Type | Compression | Pixel | Native layout |
//! | ---- | ----------- | ----- | ------------- |
//! | 1    | uncompressed | 8 bpp + palette (15/16/24/32-bit entries) | `Pal8` + [`Palette`] |
//! | 2    | uncompressed | 24 bpp                                    | `Rgb24` |
//! | 2    | uncompressed | 32 bpp, 15 / 16 bpp (expanded)            | `Rgba` |
//! | 3    | uncompressed | 8 bpp grayscale                           | `Gray8` |
//! | 9    | RLE          | 8 bpp + palette                           | `Pal8` + [`Palette`] |
//! | 10   | RLE          | 15 / 16 / 24 / 32 bpp                     | `Rgb24` / `Rgba` |
//! | 11   | RLE          | 8 bpp grayscale                           | `Gray8` |
//!
//! Both row order (image-descriptor bit 5) and column order (bit 4) are
//! auto-detected; bottom-up rows are flipped and right-to-left columns
//! are mirrored so output is always normalised to a top-down,
//! left-to-right origin. The optional 26-byte TGA 2.0 footer is
//! recognised: the extension area lands on [`TgaImage::extension`] (and
//! its gamma on `metadata.gamma`); [`parse_tga_footer`] /
//! [`parse_tga_extension_area`] / [`parse_tga_postage_stamp`] /
//! [`parse_tga_colour_correction_table`] / [`parse_tga_scan_line_table`]
//! / [`parse_tga_developer_area`] walk the individual TGA 2.0
//! structures, and [`decode_tga_for_display`] applies the file's own
//! metadata (alpha interpretation, tone curve, key colour, pixel
//! aspect) in spec order.
//!
//! ## Write coverage
//!
//! [`encode`] writes a [`TgaImage`] in its layout: `Pal8` → type 1 / 9,
//! `Rgb24` → type 2 / 10 at 24 bpp, `Rgba` → type 2 / 10 at 32 bpp,
//! `Gray8` → type 3 / 11; [`EncodeOptions`] selects RLE vs
//! uncompressed, row order, alpha bits, colour-map entry size, Image ID,
//! screen origin and the TGA 2.0 footer / extension area.
//!
//! ## Standalone vs registry-integrated
//!
//! The crate's default `registry` Cargo feature pulls in `oxideav-core`
//! and exposes the framework `Decoder` / `Encoder` trait surface plus
//! the `register` entry point and the `make_decoder` /
//! `make_encoder` factories. Disable the feature
//! (`default-features = false`) for an `oxideav-core`-free build that
//! still exposes the whole standalone API.

pub mod api;
#[cfg(feature = "registry")]
pub mod container;
pub mod decoder;
pub mod display;
pub mod encoder;
pub mod error;
pub mod image;
pub mod options;
#[cfg(feature = "registry")]
pub mod registry;
pub mod types;

/// Codec id for TGA image frames.
pub const CODEC_ID_STR: &str = "tga";

// ---- The image-crate contract (IMAGE_CRATE_API) ----
pub use api::{
    decode, decode_from, decode_rgb8, decode_rgba8, decode_with, encode, encode_rgb8, encode_rgba8,
    encode_to, header, info, probe,
};
pub use error::{Error, Result, TgaError};
pub use image::{
    ColorInfo, ColorRange, ImageInfo, Metadata, Palette, PixelFormat, Plane, RgbImage, RgbaImage,
    TgaImage, TgaPixelFormat,
};
pub use options::{DecodeOptions, EncodeOptions, RowOrder};

// ---- TGA depth: footer / extension area / tables / display pipeline ----
pub use decoder::{
    compute_tga_scan_line_table, parse_tga_border_color, parse_tga_color_map,
    parse_tga_colour_correction_table, parse_tga_developer_area, parse_tga_extension_area,
    parse_tga_footer, parse_tga_image_id, parse_tga_postage_stamp,
    parse_tga_postage_stamp_dimensions, parse_tga_scan_line, parse_tga_scan_line_table,
    resolve_alpha_from_descriptor, resolve_alpha_with_targa32_fallback,
};
pub use display::{
    decode_tga_for_display, decode_tga_for_display_reported, decode_tga_frame, AlphaResolution,
    TgaDecodedFrame, TgaDisplayOptions, TgaDisplayReport, ToneApplied,
};
pub use encoder::{
    encode_tga_with_extension, set_image_origin, splice_image_id, DeveloperTagInput,
    ExtensionAreaInput, TGA_IMAGE_ID_MAX,
};
pub use types::{
    parse_extension_area, parse_footer, AttributeBits, AttributesType, ColorMapEntrySize,
    ColorMapType, GammaValue, ImageOrigin, ImageType, Interleaving, JobTime, KeyColor,
    PixelAspectRatio, PostageStamp, SoftwareVersion, TgaAsciiField, TgaAuthorComments, TgaColorMap,
    TgaColourCorrectionTable, TgaDeveloperArea, TgaDeveloperTag, TgaExtensionArea, TgaFooter,
    TgaHeader, TgaScanLineTable, TgaTimestamp, TGA_ASCII_FIELD_MAX_CHARS, TGA_ATTRIBUTE_BITS_MAX,
    TGA_AUTHOR_COMMENT_LINES, TGA_AUTHOR_COMMENT_LINE_BYTES, TGA_AUTHOR_COMMENT_LINE_MAX_CHARS,
    TGA_COLOUR_CORRECTION_TABLE_ENTRIES, TGA_COLOUR_CORRECTION_TABLE_SIZE,
    TGA_DEVELOPER_DIRECTORY_HEADER_BYTES, TGA_DEVELOPER_TAG_BYTES, TGA_EXTENSION_AREA_SIZE,
    TGA_FOOTER_MAGIC, TGA_FOOTER_SIZE, TGA_HEADER_SIZE, TGA_INTERLEAVING_MASK,
    TGA_POSTAGE_STAMP_MAX, TGA_POSTAGE_STAMP_RECOMMENDED_MAX, TGA_SCAN_LINE_OFFSET_BYTES,
};

// ---- Pre-contract names, kept for one release ----
#[allow(deprecated)]
pub use decoder::{
    parse_tga, parse_tga_attribute_bits, parse_tga_attributes_type, parse_tga_author_comments,
    parse_tga_author_name, parse_tga_color_map_type, parse_tga_gamma, parse_tga_image_origin,
    parse_tga_interleaving, parse_tga_job_name, parse_tga_job_time, parse_tga_key_color,
    parse_tga_pixel_aspect_ratio, parse_tga_software_id, parse_tga_software_version,
    parse_tga_timestamp,
};
#[allow(deprecated)]
pub use encoder::{
    encode_tga_grayscale, encode_tga_grayscale_rle, encode_tga_palette, encode_tga_palette_rle,
    encode_tga_palette_with_entry_size, encode_tga_rle, encode_tga_rle_image, encode_tga_rle_rgb24,
    encode_tga_uncompressed, encode_tga_uncompressed_image, encode_tga_uncompressed_rgb24,
};
#[allow(deprecated)]
pub use types::parse_header;

#[cfg(feature = "registry")]
#[doc(hidden)]
pub use registry::__oxideav_entry;
#[cfg(feature = "registry")]
#[allow(deprecated)]
pub use registry::register_runtime;
#[cfg(feature = "registry")]
pub use registry::{
    make_decoder, make_decoder_with_display_options, make_encoder, register, register_codecs,
    register_containers, register_registries,
};
