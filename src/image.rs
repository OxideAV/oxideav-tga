//! The standalone image types: the shapes every `oxideav-<format>`
//! image crate shares (`IMAGE_CRATE_API`), specialised for TGA.
//!
//! * [`TgaImage`] — the native-layout image [`crate::decode`] returns
//!   and [`crate::encode`] consumes: dimensions, a [`PixelFormat`] tag,
//!   one packed [`Plane`], [`ColorInfo`], [`Metadata`], an optional
//!   [`Palette`] (colour-mapped files) and the typed TGA 2.0
//!   [`TgaExtensionArea`] record when the file carries one.
//! * [`RgbImage`] / [`RgbaImage`] — the tightly packed 8-bit raw paths
//!   ([`crate::decode_rgb8`] / [`crate::decode_rgba8`],
//!   [`TgaImage::to_rgb8`] / [`TgaImage::to_rgba8`]).
//! * [`ImageInfo`] — what [`crate::info`] reads from the header (and
//!   the optional footer) without touching a pixel.
//!
//! Defined here (rather than reusing `oxideav_core::VideoFrame`) so the
//! crate builds with the default `registry` feature off — i.e. without
//! depending on `oxideav-core` at all. With `registry` on,
//! `crate::registry` adds the `From<TgaImage> for VideoFrame`
//! conversion and its inverse so the framework `Decoder` / `Encoder`
//! are thin adapters over the same functions.

use crate::error::{Result, TgaError};
use crate::types::{ImageType, TgaExtensionArea};

/// Pixel layouts the standalone `oxideav-tga` API can produce / consume.
///
/// Variant names mirror `oxideav_core::PixelFormat` exactly, so the
/// `crate::registry` conversion layer is a 1:1 match-and-rebuild
/// rather than a re-pack. Every TGA layout is packed (one plane), and
/// the on-disk BGR(A) byte order is swizzled to RGB(A) at decode time
/// (and back at encode time), so the in-memory layouts are the
/// framework's RGB-ordered ones.
///
/// What [`crate::decode`] produces per spec image type / depth:
///
/// | Image type | Depth | Native layout |
/// |---|---|---|
/// | 1 / 9 (colour-mapped) | 8 | [`Pal8`](Self::Pal8) + [`TgaImage::palette`] |
/// | 2 / 10 (true-colour) | 24 | [`Rgb24`](Self::Rgb24) |
/// | 2 / 10 (true-colour) | 32 | [`Rgba`](Self::Rgba) |
/// | 2 / 10 (true-colour) | 15 / 16 | [`Rgba`](Self::Rgba), see below |
/// | 3 / 11 (black and white) | 8 | [`Gray8`](Self::Gray8) |
///
/// 15 / 16-bit true-colour pixels (spec §C.2: `ARRRRRGG GGGBBBBB`,
/// little-endian) have no `oxideav_core::PixelFormat` counterpart, so
/// they are expanded at decode time exactly as the colour map's 15 /
/// 16-bit entries are: each 5-bit channel becomes 8 bits by repeating
/// its top three bits (`v << 3 | v >> 2`, so `0b11111` is `0xFF`), and
/// the 16-bit attribute bit becomes alpha `0xFF` / `0x00` (15-bit:
/// always opaque). The expansion is deterministic and documented, but
/// not a round trip: there is no 15 / 16-bit encode path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TgaPixelFormat {
    /// 8-bit RGBA, 4 bytes per pixel (32-bit files, and the expansion
    /// of 15 / 16-bit files).
    Rgba,
    /// 8-bit RGB, 3 bytes per pixel (24-bit files).
    Rgb24,
    /// 8-bit single-channel grayscale, 1 byte per pixel (image types
    /// 3 / 11).
    Gray8,
    /// 8-bit palette index, 1 byte per pixel (image types 1 / 9). The
    /// colour map lives on [`TgaImage::palette`]; indices are rebased so
    /// that `0` is the first stored entry (the header's Color Map Origin
    /// is folded in at decode time).
    Pal8,
}

/// The contract name for [`TgaPixelFormat`].
pub type PixelFormat = TgaPixelFormat;

impl TgaPixelFormat {
    /// Bytes per pixel for the layout.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba => 4,
            Self::Rgb24 => 3,
            Self::Gray8 | Self::Pal8 => 1,
        }
    }

    /// `true` when the layout carries an alpha channel of its own
    /// (`Rgba`). Palette alpha is not part of the layout; see
    /// [`TgaImage::has_alpha`].
    pub fn has_alpha(self) -> bool {
        matches!(self, Self::Rgba)
    }

    /// The spec image type a layout encodes to (`rle` selects the §C.5
    /// run-length variant).
    pub fn image_type(self, rle: bool) -> ImageType {
        match (self, rle) {
            (Self::Pal8, false) => ImageType::UncompressedColourMapped,
            (Self::Pal8, true) => ImageType::RleColourMapped,
            (Self::Gray8, false) => ImageType::UncompressedGrayscale,
            (Self::Gray8, true) => ImageType::RleGrayscale,
            (Self::Rgb24 | Self::Rgba, false) => ImageType::UncompressedTrueColour,
            (Self::Rgb24 | Self::Rgba, true) => ImageType::RleTrueColour,
        }
    }
}

/// One pixel plane: `stride` bytes per row, `data` holding at least
/// `stride × height` bytes (rows may carry padding past the visible
/// width). TGA layouts are packed, so a [`TgaImage`] has exactly one.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Plane {
    /// Bytes per row.
    pub stride: usize,
    /// Row-major bytes, `stride × height` long.
    pub data: Vec<u8>,
}

impl Plane {
    /// Wrap a plane buffer with its row stride.
    pub fn new(stride: usize, data: Vec<u8>) -> Self {
        Self { stride, data }
    }
}

/// Nominal sample range (H.273 `VideoFullRangeFlag`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ColorRange {
    /// No range was signalled.
    #[default]
    Unspecified,
    /// Limited (video / studio) range: `VideoFullRangeFlag == 0`.
    Limited,
    /// Full (PC) range: `VideoFullRangeFlag == 1`.
    Full,
}

/// Colour signalling of an image: the sample range plus the H.273
/// `ColourPrimaries` / `TransferCharacteristics` /
/// `MatrixCoefficients` code points (`2` = unspecified).
///
/// TGA has no colour-space signalling of its own (no ICC, no
/// primaries; the TGA 2.0 extension area carries only a display gamma,
/// surfaced as [`Metadata::gamma`]), so [`crate::decode`] always fills
/// [`ColorInfo::tga_default`]: full-range RGB (`matrix` 0) with
/// unspecified primaries and transfer. This is the crate's documented
/// convention, not a value read from the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ColorInfo {
    /// Sample range.
    pub range: ColorRange,
    /// H.273 `ColourPrimaries` code point (`1` = BT.709 / sRGB, `2` =
    /// unspecified).
    pub primaries: u8,
    /// H.273 `TransferCharacteristics` code point (`13` = sRGB, `2` =
    /// unspecified).
    pub transfer: u8,
    /// H.273 `MatrixCoefficients` code point (`0` = identity / RGB).
    pub matrix: u8,
}

impl ColorInfo {
    /// H.273 "unspecified" code point.
    pub const UNSPECIFIED: u8 = 2;
    /// H.273 `MatrixCoefficients` identity (RGB / GBR) code point.
    pub const MATRIX_IDENTITY: u8 = 0;
    /// H.273 `ColourPrimaries` BT.709 / sRGB code point.
    pub const PRIMARIES_BT709: u8 = 1;
    /// H.273 `TransferCharacteristics` IEC 61966-2-1 sRGB code point.
    pub const TRANSFER_SRGB: u8 = 13;

    /// Build a description from its four parts.
    pub const fn new(range: ColorRange, primaries: u8, transfer: u8, matrix: u8) -> Self {
        Self {
            range,
            primaries,
            transfer,
            matrix,
        }
    }

    /// Every field unspecified.
    pub const fn unspecified() -> Self {
        Self::new(
            ColorRange::Unspecified,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
        )
    }

    /// TGA's documented default (the format signals no colour space):
    /// full-range RGB (`matrix` 0) with unspecified primaries and
    /// transfer.
    pub const fn tga_default() -> Self {
        Self::new(
            ColorRange::Full,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
            Self::MATRIX_IDENTITY,
        )
    }

    /// sRGB (IEC 61966-2-1): BT.709 primaries, sRGB transfer, identity
    /// matrix, full range.
    pub const fn srgb() -> Self {
        Self::new(
            ColorRange::Full,
            Self::PRIMARIES_BT709,
            Self::TRANSFER_SRGB,
            Self::MATRIX_IDENTITY,
        )
    }

    /// Set the range.
    pub fn with_range(mut self, range: ColorRange) -> Self {
        self.range = range;
        self
    }

    /// Set the primaries code point.
    pub fn with_primaries(mut self, primaries: u8) -> Self {
        self.primaries = primaries;
        self
    }

    /// Set the transfer code point.
    pub fn with_transfer(mut self, transfer: u8) -> Self {
        self.transfer = transfer;
        self
    }

    /// Set the matrix code point.
    pub fn with_matrix(mut self, matrix: u8) -> Self {
        self.matrix = matrix;
        self
    }

    /// `true` when both primaries and transfer are specified (`!= 2`).
    pub fn is_specified(&self) -> bool {
        self.primaries != Self::UNSPECIFIED && self.transfer != Self::UNSPECIFIED
    }
}

impl Default for ColorInfo {
    /// [`ColorInfo::tga_default`].
    fn default() -> Self {
        Self::tga_default()
    }
}

/// The metadata blobs every image crate surfaces: an ICC profile, an
/// Exif payload, an XMP packet and a file gamma. TGA has no ICC / Exif
/// / XMP mechanism (those are always `None` from [`crate::decode`] and
/// ignored by [`crate::encode`]); `gamma` is the TGA 2.0 extension
/// area's §C.6.6 Gamma Value as a ratio (`None` when the file has no
/// extension area or the field is unset / has a zero denominator). The
/// rest of the extension area is on [`TgaImage::extension`].
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Metadata {
    /// ICC profile bytes. TGA has no carrier; always `None` on decode.
    pub icc: Option<Vec<u8>>,
    /// Exif payload. TGA has no carrier; always `None` on decode.
    pub exif: Option<Vec<u8>>,
    /// XMP packet. TGA has no carrier; always `None` on decode.
    pub xmp: Option<Vec<u8>>,
    /// Display gamma (§C.6.6 `numerator / denominator`, e.g. `2.2`).
    /// Decoding never applies it (see [`crate::decode_tga_for_display`]
    /// for the display pipeline that does); encoding writes it into a
    /// TGA 2.0 extension area.
    pub gamma: Option<f32>,
}

impl Metadata {
    /// Empty metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or clear) the ICC profile.
    pub fn with_icc(mut self, icc: impl Into<Option<Vec<u8>>>) -> Self {
        self.icc = icc.into();
        self
    }

    /// Set (or clear) the Exif payload.
    pub fn with_exif(mut self, exif: impl Into<Option<Vec<u8>>>) -> Self {
        self.exif = exif.into();
        self
    }

    /// Set (or clear) the XMP packet.
    pub fn with_xmp(mut self, xmp: impl Into<Option<Vec<u8>>>) -> Self {
        self.xmp = xmp.into();
        self
    }

    /// Set (or clear) the file gamma.
    pub fn with_gamma(mut self, gamma: impl Into<Option<f32>>) -> Self {
        self.gamma = gamma.into();
        self
    }

    /// `true` when no field is set.
    pub fn is_empty(&self) -> bool {
        self.icc.is_none() && self.exif.is_none() && self.xmp.is_none() && self.gamma.is_none()
    }
}

/// Colour table of an indexed ([`Pal8`](TgaPixelFormat::Pal8)) image:
/// RGBA entries, index `i` at `entries[i]`. TGA builds it from the
/// colour map (§C.2): 24-bit BGR entries become opaque RGBA, 32-bit
/// BGRA entries keep their attribute byte as alpha, 15 / 16-bit entries
/// are expanded as described on [`TgaPixelFormat`]. The encoder writes
/// the map back at the narrowest lossless entry size (24-bit when every
/// entry is opaque, else 32-bit) unless
/// [`EncodeOptions::palette_entry_size`](crate::EncodeOptions::palette_entry_size)
/// pins one.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Palette {
    /// `[r, g, b, a]` per entry, at most 256 entries for an 8-bit index.
    pub entries: Vec<[u8; 4]>,
}

impl Palette {
    /// Wrap a list of RGBA entries.
    pub fn new(entries: Vec<[u8; 4]>) -> Self {
        Self { entries }
    }

    /// Build from packed RGB triples and an optional alpha tail (entries
    /// past the tail are opaque). A trailing partial triple is dropped.
    pub fn from_rgb(rgb: &[u8], alpha: Option<&[u8]>) -> Self {
        let alpha = alpha.unwrap_or(&[]);
        let entries = rgb
            .chunks_exact(3)
            .enumerate()
            .map(|(i, e)| [e[0], e[1], e[2], alpha.get(i).copied().unwrap_or(255)])
            .collect();
        Self { entries }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when the palette has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entry `index`, if present.
    pub fn get(&self, index: u8) -> Option<[u8; 4]> {
        self.entries.get(usize::from(index)).copied()
    }

    /// Packed RGB triples (alpha dropped).
    pub fn to_rgb(&self) -> Vec<u8> {
        self.entries
            .iter()
            .flat_map(|e| [e[0], e[1], e[2]])
            .collect()
    }

    /// `true` when any entry is not fully opaque.
    pub fn has_alpha(&self) -> bool {
        self.entries.iter().any(|e| e[3] != 255)
    }
}

/// Decoded TGA image in its native layout, as returned by
/// [`crate::decode`] and consumed by [`crate::encode`].
///
/// `planes` holds exactly one packed plane (every TGA layout is
/// packed), always normalised to a top-left origin regardless of the
/// on-disk image-descriptor origin bits; `color` is
/// [`ColorInfo::tga_default`]; `metadata.gamma` and `extension` come
/// from the TGA 2.0 extension area when the file has one; `palette` is
/// `Some` for [`Pal8`](TgaPixelFormat::Pal8).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TgaImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Native pixel layout.
    pub format: PixelFormat,
    /// Pixel planes — exactly one for TGA.
    pub planes: Vec<Plane>,
    /// Colour signalling (range + H.273 code points).
    pub color: ColorInfo,
    /// ICC / Exif / XMP / gamma.
    pub metadata: Metadata,
    /// Colour table for `Pal8`.
    pub palette: Option<Palette>,
    /// The typed TGA 2.0 extension area (§C.6: author, comments,
    /// timestamp, job, software, key colour, pixel aspect, gamma,
    /// attributes type and the three table / stamp offsets) when the
    /// file carries one; `None` for a TGA 1.0 file or a 2.0 file with a
    /// marker-only footer. [`crate::encode`] re-emits it when
    /// [`EncodeOptions::extension`](crate::EncodeOptions::extension) is
    /// `None` (see there for the field precedence).
    pub extension: Option<TgaExtensionArea>,
}

impl TgaImage {
    /// Assemble an image from its geometry, layout and planes (one for
    /// TGA). Colour is [`ColorInfo::tga_default`], metadata empty, no
    /// palette, no extension area; the `with_*` builders fill those in.
    ///
    /// The plane geometry is validated ([`TgaImage::validate`]) so an
    /// invalid image cannot be built here: exactly one plane, a stride
    /// of at least `width × bytes_per_pixel`, and at least
    /// `stride × height` bytes of data (a `Pal8` image also needs a
    /// palette; add it with [`TgaImage::with_palette`] and re-validate,
    /// or build the image with [`TgaImage::new_indexed`]).
    pub fn new(width: u32, height: u32, format: PixelFormat, planes: Vec<Plane>) -> Result<Self> {
        let img = Self::unchecked(width, height, format, planes);
        img.validate_planes()?;
        Ok(img)
    }

    /// [`TgaImage::new`] for an indexed image: `Pal8` indices plus their
    /// palette, validated together (geometry, palette present, every
    /// index inside the palette).
    pub fn new_indexed(
        width: u32,
        height: u32,
        indices: Vec<u8>,
        palette: Palette,
    ) -> Result<Self> {
        let img = Self::unchecked(
            width,
            height,
            PixelFormat::Pal8,
            vec![Plane::new(width as usize, indices)],
        )
        .with_palette(palette);
        img.validate()?;
        Ok(img)
    }

    /// [`TgaImage::new`] without the geometry check (the colour and
    /// metadata defaults are the same).
    pub(crate) fn unchecked(
        width: u32,
        height: u32,
        format: PixelFormat,
        planes: Vec<Plane>,
    ) -> Self {
        Self {
            width,
            height,
            format,
            planes,
            color: ColorInfo::tga_default(),
            metadata: Metadata::default(),
            palette: None,
            extension: None,
        }
    }

    /// A packed image over `data` with the layout's tight stride
    /// (`width × bytes_per_pixel`), any layout. Not validated; see
    /// [`TgaImage::from_rgb8`]. A `Pal8` image built this way still
    /// needs [`TgaImage::with_palette`].
    pub fn packed(width: u32, height: u32, format: PixelFormat, data: Vec<u8>) -> Self {
        let stride = width as usize * format.bytes_per_pixel();
        Self::unchecked(width, height, format, vec![Plane::new(stride, data)])
    }

    /// A packed `Rgb24` image over `data` (`3 × width × height` bytes,
    /// row-major, stride `3 × width`). Not validated (the contract's
    /// infallible constructor): a short buffer fails at
    /// [`TgaImage::validate`] / [`crate::encode`], and the conversions
    /// pad it with black.
    pub fn from_rgb8(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self::packed(width, height, PixelFormat::Rgb24, data)
    }

    /// A packed `Rgba` image over `data` (`4 × width × height` bytes);
    /// see [`TgaImage::from_rgb8`] about validation.
    pub fn from_rgba8(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self::packed(width, height, PixelFormat::Rgba, data)
    }

    /// A packed `Gray8` image over `data` (`width × height` bytes); see
    /// [`TgaImage::from_rgb8`] about validation.
    pub fn from_gray8(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self::packed(width, height, PixelFormat::Gray8, data)
    }

    /// Set the colour description.
    pub fn with_color(mut self, color: ColorInfo) -> Self {
        self.color = color;
        self
    }

    /// Set the metadata.
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Set (or clear) the palette.
    pub fn with_palette(mut self, palette: impl Into<Option<Palette>>) -> Self {
        self.palette = palette.into();
        self
    }

    /// Set (or clear) the typed extension-area record.
    pub fn with_extension(mut self, extension: impl Into<Option<TgaExtensionArea>>) -> Self {
        self.extension = extension.into();
        self
    }

    /// Image width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Native pixel layout.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Bytes-per-pixel implied by `format`.
    pub fn bytes_per_pixel(&self) -> usize {
        self.format.bytes_per_pixel()
    }

    /// Bytes per row of the (single) plane; `0` when there is none.
    pub fn stride(&self) -> usize {
        self.planes.first().map(|p| p.stride).unwrap_or(0)
    }

    /// The single packed plane's bytes. Always `Some` for an image this
    /// crate decoded (every TGA layout is packed); `None` only for a
    /// caller-assembled image with no plane.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.planes.first().map(|p| p.data.as_slice())
    }

    /// The pixel bytes (the single plane), or an empty slice when there
    /// is no plane. Rows are `stride()` bytes apart.
    pub fn data(&self) -> &[u8] {
        self.as_bytes().unwrap_or(&[])
    }

    /// Mutable view of the pixel bytes (see [`TgaImage::data`]).
    pub fn data_mut(&mut self) -> &mut [u8] {
        match self.planes.first_mut() {
            Some(p) => p.data.as_mut_slice(),
            None => &mut [],
        }
    }

    /// Consume the image, returning its pixel bytes: the plane for the
    /// packed layouts (every TGA layout).
    pub fn into_raw(self) -> Vec<u8> {
        let mut planes = self.planes.into_iter();
        let mut out = planes.next().map(|p| p.data).unwrap_or_default();
        for p in planes {
            out.extend_from_slice(&p.data);
        }
        out
    }

    /// `true` when the image can carry transparency: an `Rgba` layout,
    /// or a `Pal8` palette with a non-opaque entry.
    pub fn has_alpha(&self) -> bool {
        self.format.has_alpha() || self.palette.as_ref().is_some_and(Palette::has_alpha)
    }

    /// Check the plane geometry: exactly one plane, stride at least
    /// `width × bytes_per_pixel`, data at least `stride × height` bytes.
    pub(crate) fn validate_planes(&self) -> Result<()> {
        if self.planes.len() != 1 {
            return Err(TgaError::invalid(format!(
                "TGA: expected exactly one packed plane, got {}",
                self.planes.len()
            )));
        }
        let plane = &self.planes[0];
        let row = (self.width as usize)
            .checked_mul(self.bytes_per_pixel())
            .ok_or_else(|| TgaError::invalid("TGA: row size overflows"))?;
        if plane.stride < row {
            return Err(TgaError::invalid(format!(
                "TGA: stride {} shorter than a {}-pixel row of {} bytes",
                plane.stride, self.width, row
            )));
        }
        let need = if self.height == 0 {
            0
        } else {
            plane
                .stride
                .checked_mul(self.height as usize - 1)
                .and_then(|n| n.checked_add(row))
                .ok_or_else(|| TgaError::invalid("TGA: plane size overflows"))?
        };
        if plane.data.len() < need {
            return Err(TgaError::invalid(format!(
                "TGA: plane holds {} bytes, {}x{} at stride {} needs {}",
                plane.data.len(),
                self.width,
                self.height,
                plane.stride,
                need
            )));
        }
        Ok(())
    }

    /// Check the image is self-consistent: the plane geometry
    /// ([`TgaImage::new`]'s rule), plus for `Pal8` that a palette is
    /// attached, has at most 256 entries and covers every index used.
    pub fn validate(&self) -> Result<()> {
        self.validate_planes()?;
        if self.format == PixelFormat::Pal8 {
            let pal = self
                .palette
                .as_ref()
                .ok_or_else(|| TgaError::invalid("TGA: Pal8 image without a palette"))?;
            if pal.len() > 256 {
                return Err(TgaError::invalid(format!(
                    "TGA: palette has {} entries; an 8-bit index addresses at most 256",
                    pal.len()
                )));
            }
            let n = pal.len();
            let w = self.width as usize;
            let stride = self.stride();
            let data = self.data();
            for y in 0..self.height as usize {
                let row = &data[y * stride..y * stride + w];
                if let Some(&bad) = row.iter().find(|&&i| usize::from(i) >= n) {
                    return Err(TgaError::invalid(format!(
                        "TGA: palette index {bad} out of range (palette has {n} entries)"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Tightly packed RGBA, 4 bytes per pixel, row-major: `Rgba` copied,
    /// `Rgb24` widened with alpha `255`, `Gray8` replicated to R = G = B
    /// with alpha `255`, `Pal8` expanded through the palette (an index
    /// the palette does not cover, or a missing palette, yields
    /// transparent black). Exact for every layout this crate decodes;
    /// a caller-assembled image with a short buffer is padded with
    /// black — see [`TgaImage::try_to_rgba8`] to detect that instead.
    pub fn to_rgba8(&self) -> Vec<u8> {
        self.convert(true)
    }

    /// Tightly packed RGB, 3 bytes per pixel, row-major: alpha dropped
    /// (`Rgba` / palette alpha), otherwise as [`TgaImage::to_rgba8`].
    pub fn to_rgb8(&self) -> Vec<u8> {
        self.convert(false)
    }

    /// [`TgaImage::to_rgba8`] reporting a bad plane geometry / missing
    /// palette / out-of-range index instead of substituting black.
    pub fn try_to_rgba8(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(self.convert(true))
    }

    /// [`TgaImage::to_rgb8`] reporting a bad plane geometry / missing
    /// palette / out-of-range index instead of substituting black.
    pub fn try_to_rgb8(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(self.convert(false))
    }

    fn convert(&self, alpha: bool) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        let out_bpp = if alpha { 4 } else { 3 };
        let mut out = vec![0u8; w * h * out_bpp];
        if alpha {
            out.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255);
        }
        if w == 0 || h == 0 {
            return out;
        }
        let bpp = self.bytes_per_pixel();
        let stride = self.stride();
        let src = self.data();
        let row_bytes = w * bpp;

        // Palette lookup table: 256 RGBA cells; entries the palette
        // does not cover are transparent black.
        let lut: [[u8; 4]; 256] = match (&self.palette, self.format) {
            (Some(p), PixelFormat::Pal8) => {
                let mut lut = [[0u8; 4]; 256];
                for (slot, e) in lut.iter_mut().zip(p.entries.iter()) {
                    *slot = *e;
                }
                lut
            }
            _ => [[0u8; 4]; 256],
        };

        for y in 0..h {
            let Some(row) = src.get(y * stride..y * stride + row_bytes) else {
                break;
            };
            let dst = &mut out[y * w * out_bpp..(y + 1) * w * out_bpp];
            match self.format {
                PixelFormat::Rgba => {
                    for (s, d) in row.chunks_exact(4).zip(dst.chunks_exact_mut(out_bpp)) {
                        d.copy_from_slice(&s[..out_bpp]);
                    }
                }
                PixelFormat::Rgb24 => {
                    for (s, d) in row.chunks_exact(3).zip(dst.chunks_exact_mut(out_bpp)) {
                        d[..3].copy_from_slice(s);
                    }
                }
                PixelFormat::Gray8 => {
                    for (&g, d) in row.iter().zip(dst.chunks_exact_mut(out_bpp)) {
                        d[0] = g;
                        d[1] = g;
                        d[2] = g;
                    }
                }
                PixelFormat::Pal8 => {
                    for (&i, d) in row.iter().zip(dst.chunks_exact_mut(out_bpp)) {
                        d.copy_from_slice(&lut[usize::from(i)][..out_bpp]);
                    }
                }
            }
        }
        out
    }

    /// The pre-contract decode layout: `Gray8` stays `Gray8`, every
    /// other layout (including `Pal8` and `Rgb24`) becomes packed
    /// `Rgba` via [`TgaImage::to_rgba8`]. This is what the deprecated
    /// [`crate::parse_tga`] and the display pipeline return, byte for
    /// byte what earlier releases produced. Colour, metadata and the
    /// extension record are carried over; the palette is dropped (it
    /// has been applied).
    pub fn into_legacy_layout(self) -> Self {
        if self.format == PixelFormat::Gray8 {
            return self;
        }
        let rgba = self.to_rgba8();
        let mut out = Self::packed(self.width, self.height, PixelFormat::Rgba, rgba);
        out.color = self.color;
        out.metadata = self.metadata;
        out.extension = self.extension;
        out
    }

    /// Re-index an image to [`Pal8`](TgaPixelFormat::Pal8): the unique
    /// RGBA colours become the palette in first-seen order (`Gray8` /
    /// `Rgb24` sources are widened with opaque alpha first; a `Pal8`
    /// source is returned unchanged). Returns
    /// [`TgaError::Unsupported`] when the image has more than 256
    /// distinct colours — the 8-bit index cannot represent it. This is
    /// the conversion the colour-mapped encode paths need; `encode`
    /// never performs it implicitly.
    pub fn to_indexed(&self) -> Result<Self> {
        if self.format == PixelFormat::Pal8 {
            return Ok(self.clone());
        }
        let rgba = self.to_rgba8();
        let mut palette: Vec<[u8; 4]> = Vec::new();
        let mut indices: Vec<u8> = Vec::with_capacity(rgba.len() / 4);
        // Linear scan is fine for the <= 256 unique-colour budget.
        for chunk in rgba.chunks_exact(4) {
            let p = [chunk[0], chunk[1], chunk[2], chunk[3]];
            let idx = match palette.iter().position(|q| *q == p) {
                Some(i) => i,
                None => {
                    if palette.len() == 256 {
                        return Err(TgaError::unsupported(
                            "TGA palette encoder: input has > 256 unique RGBA colours \
                             (encode it as true-colour instead)",
                        ));
                    }
                    palette.push(p);
                    palette.len() - 1
                }
            };
            indices.push(idx as u8);
        }
        let mut out = Self::packed(self.width, self.height, PixelFormat::Pal8, indices)
            .with_palette(Palette::new(palette));
        out.color = self.color;
        out.metadata = self.metadata.clone();
        out.extension = self.extension.clone();
        Ok(out)
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
    /// Returns `false` for the alpha-less layouts and for zero-sized
    /// images. Pair with [`Self::force_opaque`] to apply the fallback.
    pub fn all_alpha_zero(&self) -> bool {
        if self.format != PixelFormat::Rgba {
            return false;
        }
        let data = self.data();
        if data.is_empty() {
            return false;
        }
        data.chunks_exact(4).all(|p| p[3] == 0)
    }

    /// Force every alpha byte of an RGBA image to `0xFF` (fully opaque);
    /// for a `Pal8` image every palette entry is made opaque. No-op for
    /// `Rgb24` / `Gray8`. Colour channels are left untouched.
    ///
    /// This is the apply-side of the TARGA-32 vs ARGB-32 fallback noted
    /// on [`Self::all_alpha_zero`].
    pub fn force_opaque(&mut self) {
        match self.format {
            PixelFormat::Rgba => {
                for px in self.data_mut().chunks_exact_mut(4) {
                    px[3] = 0xFF;
                }
            }
            PixelFormat::Pal8 => {
                if let Some(p) = self.palette.as_mut() {
                    for e in &mut p.entries {
                        e[3] = 0xFF;
                    }
                }
            }
            PixelFormat::Rgb24 | PixelFormat::Gray8 => {}
        }
    }
}

/// Tightly packed 8-bit RGB (3 bytes per pixel, row-major), the
/// [`crate::decode_rgb8`] result. Same definition in every image
/// crate.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RgbImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `3 × width × height` bytes.
    pub data: Vec<u8>,
}

impl RgbImage {
    /// Wrap a packed RGB buffer.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    /// The pixel bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume into the pixel bytes.
    pub fn into_raw(self) -> Vec<u8> {
        self.data
    }

    /// Bytes per row (`3 × width`).
    pub fn stride(&self) -> usize {
        self.width as usize * 3
    }
}

/// Tightly packed 8-bit RGBA (4 bytes per pixel, row-major), the
/// [`crate::decode_rgba8`] result. Same definition in every image
/// crate.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `4 × width × height` bytes.
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// Wrap a packed RGBA buffer.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    /// The pixel bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume into the pixel bytes.
    pub fn into_raw(self) -> Vec<u8> {
        self.data
    }

    /// Bytes per row (`4 × width`).
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }
}

/// What [`crate::info`] reads from the 18-byte header and the optional
/// TGA 2.0 footer, without decoding a pixel.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ImageInfo {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The native layout [`crate::decode`] would return.
    pub format: PixelFormat,
    /// Number of images; always `1` (TGA holds one picture — the
    /// postage-stamp thumbnail is metadata, see
    /// [`crate::parse_tga_postage_stamp`]).
    pub frames: u32,
    /// `true` when the file declares alpha: a 32-bit true-colour image,
    /// a 16-bit one (attribute bit), or a colour map with 16 / 32-bit
    /// entries. Whether the alpha is *meaningful* is a display question
    /// (see [`crate::resolve_alpha_with_targa32_fallback`]).
    pub has_alpha: bool,
    /// Colour signalling; always [`ColorInfo::tga_default`].
    pub color: ColorInfo,
    /// TGA has no ICC carrier; always `false`.
    pub has_icc: bool,
    /// TGA has no Exif carrier; always `false`.
    pub has_exif: bool,
    /// TGA has no XMP carrier; always `false`.
    pub has_xmp: bool,
    /// Spec image type (1 / 2 / 3 / 9 / 10 / 11).
    pub image_type: ImageType,
    /// On-disk bits per pixel (8 / 15 / 16 / 24 / 32).
    pub depth: u8,
    /// §C.2 image-descriptor attribute (alpha) bit count, bits 3-0.
    pub attribute_bits: u8,
    /// `true` when rows are stored top-down (descriptor bit 5).
    pub top_down: bool,
    /// `true` when columns are stored right-to-left (descriptor bit 4).
    pub right_to_left: bool,
    /// Colour-map entry count declared by the header (`0` when none).
    pub color_map_length: u16,
    /// Colour-map entry size in bits (15 / 16 / 24 / 32; `0` when none).
    pub color_map_entry_size: u8,
    /// Length of the Image ID field (0..=255 bytes).
    pub image_id_length: u8,
    /// `true` when the file ends with a TGA 2.0 footer.
    pub has_footer: bool,
    /// `true` when the footer points at an extension area that parses.
    pub has_extension_area: bool,
    /// `true` when the footer points at a developer area.
    pub has_developer_area: bool,
}

impl ImageInfo {
    /// A still image with the given geometry; every other field at its
    /// "absent" value.
    pub fn new(width: u32, height: u32, format: PixelFormat) -> Self {
        Self {
            width,
            height,
            format,
            frames: 1,
            has_alpha: format.has_alpha(),
            color: ColorInfo::tga_default(),
            has_icc: false,
            has_exif: false,
            has_xmp: false,
            image_type: format.image_type(false),
            depth: (format.bytes_per_pixel() * 8) as u8,
            attribute_bits: if format.has_alpha() { 8 } else { 0 },
            top_down: true,
            right_to_left: false,
            color_map_length: 0,
            color_map_entry_size: 0,
            image_id_length: 0,
            has_footer: false,
            has_extension_area: false,
            has_developer_area: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_rgba8_pal8_expands_and_pads_out_of_range() {
        let img = TgaImage::packed(3, 1, PixelFormat::Pal8, vec![0, 1, 9])
            .with_palette(Palette::new(vec![[10, 20, 30, 128], [40, 50, 60, 255]]));
        assert_eq!(
            img.to_rgba8(),
            vec![10, 20, 30, 128, 40, 50, 60, 255, 0, 0, 0, 0]
        );
        assert_eq!(img.to_rgb8(), vec![10, 20, 30, 40, 50, 60, 0, 0, 0]);
        assert!(matches!(img.try_to_rgb8(), Err(TgaError::InvalidData(_))));
    }

    #[test]
    fn to_rgba8_honours_stride_padding() {
        let img = TgaImage::unchecked(
            1,
            2,
            PixelFormat::Rgb24,
            vec![Plane::new(4, vec![1, 2, 3, 99, 4, 5, 6, 99])],
        );
        assert_eq!(img.to_rgba8(), vec![1, 2, 3, 255, 4, 5, 6, 255]);
        assert_eq!(img.to_rgb8(), vec![1, 2, 3, 4, 5, 6]);
        assert!(img.validate().is_ok());
    }

    #[test]
    fn gray_and_rgba_kernels() {
        let g = TgaImage::from_gray8(2, 1, vec![7, 200]);
        assert_eq!(g.to_rgba8(), vec![7, 7, 7, 255, 200, 200, 200, 255]);
        assert_eq!(g.to_rgb8(), vec![7, 7, 7, 200, 200, 200]);
        let a = TgaImage::from_rgba8(1, 1, vec![1, 2, 3, 4]);
        assert_eq!(a.to_rgb8(), vec![1, 2, 3]);
        assert_eq!(a.as_bytes(), Some(&[1u8, 2, 3, 4][..]));
        assert_eq!(a.stride(), 4);
        assert_eq!(a.into_raw(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn new_validates_geometry() {
        assert!(TgaImage::new(2, 2, PixelFormat::Rgb24, vec![]).is_err());
        assert!(TgaImage::new(2, 2, PixelFormat::Rgb24, vec![Plane::new(6, vec![0; 11])]).is_err());
        assert!(TgaImage::new(2, 2, PixelFormat::Rgb24, vec![Plane::new(5, vec![0; 12])]).is_err());
        assert!(TgaImage::new(2, 2, PixelFormat::Rgb24, vec![Plane::new(6, vec![0; 12])]).is_ok());
        assert!(TgaImage::new(0, 0, PixelFormat::Gray8, vec![Plane::new(0, vec![])]).is_ok());
    }

    #[test]
    fn indexed_round_trip_and_overflow() {
        let mut rgba = Vec::new();
        for i in 0..300u32 {
            rgba.extend_from_slice(&[(i % 256) as u8, (i / 256) as u8, 0, 255]);
        }
        let img = TgaImage::from_rgba8(300, 1, rgba);
        assert!(matches!(img.to_indexed(), Err(TgaError::Unsupported(_))));
        let small = TgaImage::from_rgba8(3, 1, vec![1, 2, 3, 4, 1, 2, 3, 4, 9, 9, 9, 9]);
        let idx = small.to_indexed().unwrap();
        assert_eq!(idx.format, PixelFormat::Pal8);
        assert_eq!(idx.data(), &[0, 0, 1]);
        assert_eq!(
            idx.palette.as_ref().unwrap().entries,
            vec![[1, 2, 3, 4], [9, 9, 9, 9]]
        );
        assert_eq!(idx.to_rgba8(), small.to_rgba8());
        assert!(idx.validate().is_ok());
    }

    #[test]
    fn legacy_layout_matches_pre_contract_shape() {
        let g = TgaImage::from_gray8(1, 1, vec![5]).into_legacy_layout();
        assert_eq!(g.format, PixelFormat::Gray8);
        let r = TgaImage::from_rgb8(1, 1, vec![1, 2, 3]).into_legacy_layout();
        assert_eq!(r.format, PixelFormat::Rgba);
        assert_eq!(r.data(), &[1, 2, 3, 255]);
    }
}
