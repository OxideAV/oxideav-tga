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
}
