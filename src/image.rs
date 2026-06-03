//! Standalone image container returned by `oxideav-tga`'s framework-free
//! decode API and accepted by the standalone encode API.
//!
//! Defined here (rather than reusing `oxideav_core::VideoFrame`) so the
//! crate can be built with the default `registry` feature off — i.e.
//! without depending on `oxideav-core` at all. When the `registry`
//! feature is on the [`crate::registry`] module provides the matching
//! [`TgaPixelFormat`] ↔ `oxideav_core::PixelFormat` mapping so the
//! trait-side `Decoder` / `Encoder` impls keep working unchanged.

/// Pixel layout used by [`TgaImage`].
///
/// The decoder always produces [`TgaPixelFormat::Rgba`] (palette
/// expansion, BGR→RGB swapping, and 5-5-5 expansion all happen at
/// decode time so consumers don't have to know the on-disk quirks).
/// The encoder accepts [`Rgba`](Self::Rgba) (passes alpha through) or
/// [`Rgb24`](Self::Rgb24) (alpha defaulted to `0xFF`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TgaPixelFormat {
    /// 8-bit packed RGBA, 4 bytes per pixel.
    Rgba,
    /// 8-bit packed RGB, 3 bytes per pixel (encode input only).
    Rgb24,
    /// 8-bit single-channel grayscale (decode output only — type 3 / 11
    /// from the spec). One byte per pixel, repeated luma value.
    Gray8,
}

/// One decoded TGA frame, framework-free shape.
///
/// `pts` is `None` for the standalone [`crate::parse_tga`] entry point.
/// The registry-backed `Decoder` impl still passes `pts` through from
/// the surrounding `Packet`.
#[derive(Debug, Clone)]
pub struct TgaImage {
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Pixel layout the `data` carries. Decode produces
    /// [`TgaPixelFormat::Rgba`] for type 1/2/9/10 and
    /// [`TgaPixelFormat::Gray8`] for type 3/11.
    pub pixel_format: TgaPixelFormat,
    /// Row-major pixel bytes, `width × bytes_per_pixel(pixel_format) ×
    /// height` long. Always normalised to top-left origin regardless of
    /// the on-disk image-descriptor bits.
    pub data: Vec<u8>,
    /// Optional presentation timestamp. Always `None` from the
    /// standalone decode path.
    pub pts: Option<i64>,
}

impl TgaImage {
    /// Bytes-per-pixel implied by `pixel_format`.
    pub fn bytes_per_pixel(&self) -> usize {
        match self.pixel_format {
            TgaPixelFormat::Rgba => 4,
            TgaPixelFormat::Rgb24 => 3,
            TgaPixelFormat::Gray8 => 1,
        }
    }

    /// Bytes per row.
    pub fn stride(&self) -> usize {
        self.width as usize * self.bytes_per_pixel()
    }

    /// `true` when the image is RGBA, non-empty, and every alpha byte is
    /// exactly `0`.
    ///
    /// This is the diagnostic for the well-known "TARGA-32 vs ARGB-32"
    /// real-world ambiguity documented in `docs/image/tga/README.md`:
    /// some legacy paint applications wrote 32-bpp TGA files leaving the
    /// alpha channel at all-zero without setting an extension-area
    /// `attributes_type`. A naïve decoder would render every pixel as
    /// fully transparent black, even though the original intent was
    /// "opaque, alpha ignored". The convention documented for these
    /// files is "if alpha values are all 0, treat the file as opaque,
    /// not as fully-transparent black".
    ///
    /// Returns `false` for [`TgaPixelFormat::Rgb24`] / [`TgaPixelFormat::Gray8`]
    /// (no alpha channel to inspect) and for zero-sized images.
    ///
    /// Pair with [`Self::force_opaque`] to apply the fallback.
    pub fn all_alpha_zero(&self) -> bool {
        if self.pixel_format != TgaPixelFormat::Rgba {
            return false;
        }
        if self.data.is_empty() {
            return false;
        }
        self.data.chunks_exact(4).all(|p| p[3] == 0)
    }

    /// Force every alpha byte of an RGBA image to `0xFF` (fully opaque).
    ///
    /// No-op for [`TgaPixelFormat::Rgb24`] / [`TgaPixelFormat::Gray8`]
    /// (those formats have no alpha channel). Colour channels are left
    /// untouched.
    ///
    /// This is the apply-side of the TARGA-32 vs ARGB-32 fallback noted
    /// on [`Self::all_alpha_zero`]: a caller that detects all-zero alpha
    /// on a TGA file lacking a meaningful `attributes_type` byte can
    /// recover the practical "opaque" interpretation by calling this
    /// helper.
    pub fn force_opaque(&mut self) {
        if self.pixel_format != TgaPixelFormat::Rgba {
            return;
        }
        for px in self.data.chunks_exact_mut(4) {
            px[3] = 0xFF;
        }
    }
}
