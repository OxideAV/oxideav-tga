//! Decode-side limits ([`DecodeOptions`]) and encode-side behaviour
//! ([`EncodeOptions`]) for the `IMAGE_CRATE_API` root functions.

use crate::encoder::ExtensionAreaInput;
use crate::error::{Result, TgaError};
use crate::types::{ColorMapEntrySize, ImageOrigin};

/// Limits and strictness for [`crate::decode_with`].
///
/// Every limit is checked against the 18-byte header **before** any
/// pixel buffer is allocated, so a hostile header fails with
/// [`TgaError::LimitExceeded`] instead of committing memory. The
/// defaults are: no dimension / pixel-count limit (TGA dimensions are
/// `u16`, so the geometry is bounded by the format itself), decoded
/// plane capped at [`DecodeOptions::DEFAULT_MAX_BYTES`] (1 GiB),
/// `strict = false`.
///
/// `strict` governs the spec's *should* rules that the lenient decoder
/// tolerates:
///
/// * always (both modes): header completeness, a known image type
///   (1 / 2 / 3 / 9 / 10 / 11) at a depth the type allows, colour-map
///   presence for colour-mapped types, pixel data / RLE packets inside
///   the buffer, palette indices inside the colour map;
/// * `strict = false` (default): an RLE packet that spans a scan-line
///   boundary is accepted (§C.5 says packets *should* not cross lines;
///   files in the wild do), and a non-zero §C.2 interleaving flag
///   (descriptor bits 7-6, required to be zero by TGA 2.0) is ignored;
/// * `strict = true`: both are [`TgaError::InvalidData`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeOptions {
    /// Reject images wider than this (pixels).
    pub max_width: Option<u32>,
    /// Reject images taller than this (pixels).
    pub max_height: Option<u32>,
    /// Reject images with more than this many pixels (`width ×
    /// height`).
    pub max_pixels: Option<u64>,
    /// Reject images whose decoded plane would exceed this many bytes
    /// (`stride × height` of the native layout).
    pub max_bytes: Option<u64>,
    /// Enforce the spec's *should* rules (see the type docs).
    pub strict: bool,
}

impl DecodeOptions {
    /// Default [`Self::max_bytes`]: 1 GiB of decoded plane.
    pub const DEFAULT_MAX_BYTES: u64 = 1 << 30;

    /// The defaults (see the type docs).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or lift with `None`) the width limit.
    pub fn with_max_width(mut self, max_width: impl Into<Option<u32>>) -> Self {
        self.max_width = max_width.into();
        self
    }

    /// Set (or lift with `None`) the height limit.
    pub fn with_max_height(mut self, max_height: impl Into<Option<u32>>) -> Self {
        self.max_height = max_height.into();
        self
    }

    /// Set (or lift with `None`) the pixel-count limit.
    pub fn with_max_pixels(mut self, max_pixels: impl Into<Option<u64>>) -> Self {
        self.max_pixels = max_pixels.into();
        self
    }

    /// Set (or lift with `None`) the decoded-bytes limit.
    pub fn with_max_bytes(mut self, max_bytes: impl Into<Option<u64>>) -> Self {
        self.max_bytes = max_bytes.into();
        self
    }

    /// Set strict mode.
    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Lift every limit (`max_*` all `None`).
    pub fn unlimited(mut self) -> Self {
        self.max_width = None;
        self.max_height = None;
        self.max_pixels = None;
        self.max_bytes = None;
        self
    }

    /// Check a header's geometry against the limits. `bytes` is the
    /// decoded plane size the native layout implies.
    pub(crate) fn check(&self, width: u32, height: u32, bytes: u64) -> Result<()> {
        if let Some(m) = self.max_width {
            if width > m {
                return Err(TgaError::limit(format!(
                    "TGA: width {width} exceeds max_width {m}"
                )));
            }
        }
        if let Some(m) = self.max_height {
            if height > m {
                return Err(TgaError::limit(format!(
                    "TGA: height {height} exceeds max_height {m}"
                )));
            }
        }
        let pixels = u64::from(width) * u64::from(height);
        if let Some(m) = self.max_pixels {
            if pixels > m {
                return Err(TgaError::limit(format!(
                    "TGA: {pixels} pixels exceed max_pixels {m}"
                )));
            }
        }
        if let Some(m) = self.max_bytes {
            if bytes > m {
                return Err(TgaError::limit(format!(
                    "TGA: decoded plane of {bytes} bytes exceeds max_bytes {m}"
                )));
            }
        }
        Ok(())
    }
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            max_width: None,
            max_height: None,
            max_pixels: None,
            max_bytes: Some(Self::DEFAULT_MAX_BYTES),
            strict: false,
        }
    }
}

/// Row storage order of an encoded file (§C.2 image descriptor bit 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RowOrder {
    /// Rows stored first-row-first; descriptor bit 5 set ("top left"
    /// origin). The default: viewers that ignore the bit still show the
    /// picture the right way up.
    #[default]
    TopDown,
    /// Rows stored last-row-first; descriptor bit 5 clear ("bottom
    /// left" origin), the TARGA hardware's native order and the most
    /// common layout in the wild.
    BottomUp,
}

/// Behaviour of [`crate::encode`] / [`crate::encode_rgb8`] /
/// [`crate::encode_rgba8`] / [`crate::encode_to`].
///
/// One struct, every variant a field: compression (`rle`), row order,
/// the alpha-bit declaration, the colour-map entry size, the Image ID,
/// the on-screen origin and the TGA 2.0 footer / extension area. The
/// defaults write exactly what the pre-contract `encode_tga_rle` family
/// wrote for the same pixels: RLE, top-down, no Image ID, origin
/// `(0, 0)`, no footer unless the image carries metadata to store.
///
/// How the image's layout maps to the wire (never a silent
/// conversion):
///
/// | Layout | Image type | Depth | Descriptor alpha bits |
/// |---|---|---|---|
/// | `Rgb24` | 2 / 10 | 24 | 0 |
/// | `Rgba` | 2 / 10 | 32 | 8 (or [`alpha_bits`](Self::alpha_bits)) |
/// | `Gray8` | 3 / 11 | 8 | 0 |
/// | `Pal8` | 1 / 9 | 8 | 0 (the colour map carries alpha) |
///
/// An `Rgba` image whose alpha is uniformly `0xFF` is still written at
/// 32 bits unless [`drop_opaque_alpha`](Self::drop_opaque_alpha) is
/// set, so that `decode(encode(img)) == img` holds for the layout too.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct EncodeOptions {
    /// `true` (default) writes the §C.5 run-length image types
    /// (9 / 10 / 11); `false` the uncompressed ones (1 / 2 / 3).
    pub rle: bool,
    /// Row storage order (descriptor bit 5). Default
    /// [`RowOrder::TopDown`].
    pub row_order: RowOrder,
    /// Override the §C.2 descriptor attribute-bit count (bits 3-0) for
    /// an `Rgba` image. `None` (default) declares `8`. Setting `0` keeps
    /// the 32-bit layout but tells readers the fourth byte is not alpha
    /// (the "TARGA-32" convention). Ignored for the other layouts.
    pub alpha_bits: Option<u8>,
    /// Write an `Rgba` image whose alpha is uniformly `0xFF` as a
    /// 24-bit file (image type 2 / 10, depth 24, alpha bits 0) — the
    /// pre-contract writers' auto-depth rule. Default `false`: the
    /// layout is written as given. A decode of such a file yields
    /// `Rgb24`, not `Rgba`.
    pub drop_opaque_alpha: bool,
    /// Colour-map entry size for a `Pal8` image. `None` (default) picks
    /// the narrowest lossless width: 24-bit BGR when every entry is
    /// opaque, 32-bit BGRA otherwise. 15 / 16-bit entries quantise each
    /// channel to 5 bits (lossy; documented on
    /// [`ColorMapEntrySize`]). Ignored for the other layouts.
    pub palette_entry_size: Option<ColorMapEntrySize>,
    /// Image Identification Field (§C.3), 0..=255 bytes written
    /// verbatim after the header. Longer input is
    /// [`TgaError::InvalidData`].
    pub image_id: Vec<u8>,
    /// §5.1 / §5.2 on-screen lower-left placement written into header
    /// bytes 8-11. Default `(0, 0)`.
    pub screen_origin: ImageOrigin,
    /// Explicit TGA 2.0 extension area (+ optional postage stamp,
    /// colour-correction table, scan-line table, developer tags). When
    /// `Some`, it is written as given except that an unset `gamma`
    /// `(0, 0)` is filled from the image's `metadata.gamma`. When
    /// `None`, the image's own [`TgaImage::extension`](crate::TgaImage::extension)
    /// record is re-emitted (its gamma likewise synchronised with
    /// `metadata.gamma`), else a minimal extension area is written when
    /// `metadata.gamma` is `Some`, else no footer at all (a TGA 1.0
    /// file) unless [`footer`](Self::footer) asks for one.
    pub extension: Option<ExtensionAreaInput>,
    /// Always append the 26-byte TGA 2.0 footer, even when there is no
    /// extension area to point at (a "marker-only" footer with both
    /// offsets zero). Default `false`.
    pub footer: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            rle: true,
            row_order: RowOrder::TopDown,
            alpha_bits: None,
            drop_opaque_alpha: false,
            palette_entry_size: None,
            image_id: Vec::new(),
            screen_origin: ImageOrigin::ORIGIN,
            extension: None,
            footer: false,
        }
    }
}

impl EncodeOptions {
    /// The defaults: RLE, top-down, alpha bits `8` for `Rgba`, lossless
    /// colour-map entry size, no Image ID, origin `(0, 0)`, footer only
    /// when there is metadata to carry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Select RLE (`true`) or uncompressed (`false`) image types.
    pub fn with_rle(mut self, rle: bool) -> Self {
        self.rle = rle;
        self
    }

    /// Set the row storage order.
    pub fn with_row_order(mut self, row_order: RowOrder) -> Self {
        self.row_order = row_order;
        self
    }

    /// Set (or clear) the descriptor alpha-bit override.
    pub fn with_alpha_bits(mut self, alpha_bits: impl Into<Option<u8>>) -> Self {
        self.alpha_bits = alpha_bits.into();
        self
    }

    /// Set the opaque-alpha auto-depth rule.
    pub fn with_drop_opaque_alpha(mut self, drop: bool) -> Self {
        self.drop_opaque_alpha = drop;
        self
    }

    /// Set (or clear) the colour-map entry size.
    pub fn with_palette_entry_size(
        mut self,
        entry_size: impl Into<Option<ColorMapEntrySize>>,
    ) -> Self {
        self.palette_entry_size = entry_size.into();
        self
    }

    /// Set the Image ID bytes.
    pub fn with_image_id(mut self, image_id: impl Into<Vec<u8>>) -> Self {
        self.image_id = image_id.into();
        self
    }

    /// Set the on-screen origin.
    pub fn with_screen_origin(mut self, origin: ImageOrigin) -> Self {
        self.screen_origin = origin;
        self
    }

    /// Set (or clear) the explicit extension area.
    pub fn with_extension(mut self, extension: impl Into<Option<ExtensionAreaInput>>) -> Self {
        self.extension = extension.into();
        self
    }

    /// Force (or not) a TGA 2.0 footer.
    pub fn with_footer(mut self, footer: bool) -> Self {
        self.footer = footer;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_fire_in_order() {
        let o = DecodeOptions::default()
            .with_max_width(10u32)
            .with_max_height(10u32)
            .with_max_pixels(50u64)
            .with_max_bytes(100u64);
        assert!(o.check(5, 5, 75).is_ok());
        assert!(matches!(o.check(11, 1, 1), Err(TgaError::LimitExceeded(_))));
        assert!(matches!(o.check(1, 11, 1), Err(TgaError::LimitExceeded(_))));
        assert!(matches!(o.check(8, 8, 1), Err(TgaError::LimitExceeded(_))));
        assert!(matches!(
            o.check(5, 5, 101),
            Err(TgaError::LimitExceeded(_))
        ));
        assert!(o.unlimited().check(u32::MAX, u32::MAX, u64::MAX).is_ok());
    }

    #[test]
    fn encode_defaults_are_rle_top_down() {
        let o = EncodeOptions::default();
        assert!(o.rle);
        assert_eq!(o.row_order, RowOrder::TopDown);
        assert_eq!(o.alpha_bits, None);
        assert!(!o.drop_opaque_alpha);
        assert!(o.extension.is_none());
        assert!(!o.footer);
    }
}
