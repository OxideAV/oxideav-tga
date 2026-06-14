//! TGA structural types.
//!
//! TGA on-disk layout (TGA 2.0 spec, 1989, §C):
//! ```text
//!   [Header (18 B, fixed)]
//!     id_length     (u8)        — bytes of optional Image ID at offset 18
//!     cmap_type     (u8)        — 0 = no colour map, 1 = colour map present
//!     image_type    (u8)        — 0 / 1 / 2 / 3 / 9 / 10 / 11 (see below)
//!     cmap_first    (u16 LE)
//!     cmap_length   (u16 LE)
//!     cmap_entry_sz (u8)        — bits per colour-map entry
//!     x_origin      (u16 LE)
//!     y_origin      (u16 LE)
//!     width         (u16 LE)
//!     height        (u16 LE)
//!     depth         (u8)        — bits per pixel (8 / 15 / 16 / 24 / 32)
//!     descriptor    (u8)        — bottom 4 bits = α-channel bits,
//!                                 bits 4-5 = pixel ordering (see below).
//!   [Image ID, `id_length` bytes — usually a printable string, ignored here.]
//!   [Colour map, `cmap_length × ceil(cmap_entry_sz / 8)` bytes — present
//!    only when `cmap_type == 1`.]
//!   [Pixel data — uncompressed bytes for types 1/2/3, RLE-packetised for
//!    types 9/10/11.]
//!   [Optional TGA 2.0 footer (26 B) — present iff the last 26 bytes
//!    end with the magic `"TRUEVISION-XFILE.\0"`.]
//! ```
//!
//! Image-descriptor byte:
//!
//! * Bits 0-3: alpha-channel bit count (0 or 8 in practice; 1 is legal
//!   for the 16-bit A1R5G5B5 format but is implicit in the on-disk
//!   layout, not the descriptor).
//! * Bit 4: column ordering — 0 = left-to-right, 1 = right-to-left
//!   (TGA 2.0 FFS, Table 2 - Image Origin). The decoder mirrors columns
//!   so output is always left-to-right.
//! * Bit 5: row ordering — 0 = bottom-to-top, 1 = top-to-bottom.
//! * Bits 6-7: reserved (must be 0 in TGA 2.0).

/// Header size in bytes (always 18).
pub const TGA_HEADER_SIZE: usize = 18;

/// TGA 2.0 footer size in bytes.
pub const TGA_FOOTER_SIZE: usize = 26;

/// TGA 2.0 footer magic string. Note the trailing `.\0`.
pub const TGA_FOOTER_MAGIC: &[u8; 18] = b"TRUEVISION-XFILE.\0";

/// Image type codes per spec §C.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ImageType {
    /// No image data included (header-only).
    None = 0,
    /// Uncompressed colour-mapped image.
    UncompressedColourMapped = 1,
    /// Uncompressed true-colour image.
    UncompressedTrueColour = 2,
    /// Uncompressed black-and-white (grayscale) image.
    UncompressedGrayscale = 3,
    /// Run-length encoded colour-mapped image.
    RleColourMapped = 9,
    /// Run-length encoded true-colour image.
    RleTrueColour = 10,
    /// Run-length encoded black-and-white (grayscale) image.
    RleGrayscale = 11,
}

impl ImageType {
    /// Decode a raw image-type byte. Returns `None` for unknown codes
    /// (e.g. 32 / 33 = "compressed colour-mapped, Huffman + delta /
    /// 4-pass", which Truevision never shipped).
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::None,
            1 => Self::UncompressedColourMapped,
            2 => Self::UncompressedTrueColour,
            3 => Self::UncompressedGrayscale,
            9 => Self::RleColourMapped,
            10 => Self::RleTrueColour,
            11 => Self::RleGrayscale,
            _ => return None,
        })
    }

    /// `true` for image types 9/10/11 (RLE-compressed pixel data).
    pub fn is_rle(self) -> bool {
        matches!(
            self,
            Self::RleColourMapped | Self::RleTrueColour | Self::RleGrayscale
        )
    }

    /// `true` for image types 1/9 (palette-indexed pixel data).
    pub fn is_colour_mapped(self) -> bool {
        matches!(self, Self::UncompressedColourMapped | Self::RleColourMapped)
    }

    /// `true` for image types 3/11 (single-channel grayscale).
    pub fn is_grayscale(self) -> bool {
        matches!(self, Self::UncompressedGrayscale | Self::RleGrayscale)
    }
}

/// Parsed TGA header fields. Sizes match the on-disk u8/u16 widths.
#[derive(Debug, Clone, Copy)]
pub struct TgaHeader {
    pub id_length: u8,
    pub cmap_type: u8,
    pub image_type_raw: u8,
    pub cmap_first: u16,
    pub cmap_length: u16,
    pub cmap_entry_size: u8,
    pub x_origin: u16,
    pub y_origin: u16,
    pub width: u16,
    pub height: u16,
    pub depth: u8,
    pub descriptor: u8,
}

impl TgaHeader {
    /// Alpha-channel bit count from the bottom 4 bits of the descriptor.
    pub fn alpha_bits(&self) -> u8 {
        self.descriptor & 0x0F
    }

    /// `true` when bit 5 of the descriptor is set, i.e. the on-disk
    /// pixel rows are in top-to-bottom order. `false` (the most common
    /// case) means bottom-to-top, and the decoder will flip rows so the
    /// returned [`crate::TgaImage`] is always top-down.
    pub fn is_top_down(&self) -> bool {
        (self.descriptor & 0x20) != 0
    }

    /// `true` when bit 4 of the descriptor is set, i.e. the on-disk
    /// pixel columns are in right-to-left order (per the TGA 2.0 FFS
    /// image-descriptor field, Table 2 - Image Origin). The decoder
    /// mirrors columns so the returned [`crate::TgaImage`] is always
    /// left-to-right.
    pub fn is_right_to_left(&self) -> bool {
        (self.descriptor & 0x10) != 0
    }
}

/// Parse the 18-byte TGA header. Returns `None` if the input is shorter
/// than 18 bytes — caller turns that into a useful error message.
pub fn parse_header(input: &[u8]) -> Option<TgaHeader> {
    if input.len() < TGA_HEADER_SIZE {
        return None;
    }
    Some(TgaHeader {
        id_length: input[0],
        cmap_type: input[1],
        image_type_raw: input[2],
        cmap_first: read_u16_le(input, 3),
        cmap_length: read_u16_le(input, 5),
        cmap_entry_size: input[7],
        x_origin: read_u16_le(input, 8),
        y_origin: read_u16_le(input, 10),
        width: read_u16_le(input, 12),
        height: read_u16_le(input, 14),
        depth: input[16],
        descriptor: input[17],
    })
}

/// Pointers parsed out of the optional 26-byte TGA 2.0 footer. Use
/// [`parse_extension_area`] to walk the extension-area body the
/// `extension_area_offset` field points at.
///
/// The footer is the TGA 2.0 marker: a fixed 26-byte trailer with two
/// `u32` byte-offsets followed by the 18-byte ASCII signature
/// `"TRUEVISION-XFILE.\0"`. A TGA 1.0 file has no footer at all;
/// [`parse_footer`] / [`crate::parse_tga_footer`] return `None` on such
/// a file. Either offset may be zero — the spec uses zero as the
/// sentinel meaning "this section is not present in the file" — and a
/// canonical TGA 2.0 file with no metadata still emits a footer (with
/// both offsets set to zero) so a reader can distinguish v2-no-metadata
/// from v1-no-footer.
///
/// Typed accessors live in the `impl` block below
/// ([`TgaFooter::has_extension_area`], [`TgaFooter::has_developer_area`],
/// [`TgaFooter::is_marker_only`], [`TgaFooter::offsets_within`],
/// [`TgaFooter::to_bytes`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TgaFooter {
    /// Byte offset of the extension area (0 if absent).
    pub extension_area_offset: u32,
    /// Byte offset of the developer directory (0 if absent).
    pub developer_directory_offset: u32,
}

impl TgaFooter {
    /// All-zero sentinel: a TGA 2.0 marker footer with neither an
    /// extension area nor a developer directory. Same on-disk shape as
    /// what [`Self::to_bytes`] emits for `TgaFooter::default()`.
    pub const UNSET: Self = Self {
        extension_area_offset: 0,
        developer_directory_offset: 0,
    };

    /// Build a footer from the two byte offsets. Either may be zero
    /// (the spec's "not present" sentinel).
    pub fn new(extension_area_offset: u32, developer_directory_offset: u32) -> Self {
        Self {
            extension_area_offset,
            developer_directory_offset,
        }
    }

    /// Returns the `(extension_area_offset, developer_directory_offset)`
    /// pair, useful for destructuring without reaching into the fields.
    pub fn as_tuple(self) -> (u32, u32) {
        (self.extension_area_offset, self.developer_directory_offset)
    }

    /// Rebuild a footer from the offset tuple in
    /// `(extension_area_offset, developer_directory_offset)` order — the
    /// inverse of [`Self::as_tuple`].
    pub fn from_tuple(t: (u32, u32)) -> Self {
        Self {
            extension_area_offset: t.0,
            developer_directory_offset: t.1,
        }
    }

    /// `true` when the footer advertises a non-zero
    /// `extension_area_offset`. Zero is the spec's "no extension area"
    /// sentinel: a TGA 2.0 file may legitimately omit the extension area
    /// even though it carries a footer.
    pub fn has_extension_area(self) -> bool {
        self.extension_area_offset != 0
    }

    /// `true` when the footer advertises a non-zero
    /// `developer_directory_offset`. Same convention as
    /// [`Self::has_extension_area`].
    pub fn has_developer_area(self) -> bool {
        self.developer_directory_offset != 0
    }

    /// `true` when both offsets are zero — a *marker-only* footer.
    /// Equivalent to `self == TgaFooter::UNSET`. Indicates the writer
    /// chose to flag the file as TGA 2.0 without attaching any
    /// metadata. The file still ends in the canonical 26-byte trailer
    /// and the magic still matches.
    pub fn is_marker_only(self) -> bool {
        !self.has_extension_area() && !self.has_developer_area()
    }

    /// `true` when both `extension_area_offset` (when set) and
    /// `developer_directory_offset` (when set) fit inside a file of the
    /// supplied length, leaving at least the trailing 26-byte footer
    /// itself unconsumed. A zero offset is treated as "section absent"
    /// and contributes no constraint.
    ///
    /// `input_len` is the full file length, including the footer. The
    /// check is a quick sanity gate before walking the sections — it
    /// does not verify that the section bodies *contents* are
    /// well-formed; [`parse_extension_area`] / [`TgaDeveloperArea::parse`]
    /// do that.
    pub fn offsets_within(self, input_len: usize) -> bool {
        let footer_start = match input_len.checked_sub(TGA_FOOTER_SIZE) {
            Some(v) => v,
            None => return false,
        };
        if self.has_extension_area() && (self.extension_area_offset as usize) > footer_start {
            return false;
        }
        if self.has_developer_area() && (self.developer_directory_offset as usize) > footer_start {
            return false;
        }
        true
    }

    /// Serialise the footer to its canonical 26-byte on-disk layout
    /// (spec §C.4): two little-endian `u32` offsets followed by the
    /// 18-byte ASCII signature `"TRUEVISION-XFILE.\0"`.
    ///
    /// Round-trips with [`parse_footer`]: feeding the returned 26-byte
    /// slice into `parse_footer` reproduces the input `TgaFooter`
    /// bit-exactly.
    pub fn to_bytes(self) -> [u8; TGA_FOOTER_SIZE] {
        let mut out = [0u8; TGA_FOOTER_SIZE];
        out[0..4].copy_from_slice(&self.extension_area_offset.to_le_bytes());
        out[4..8].copy_from_slice(&self.developer_directory_offset.to_le_bytes());
        out[8..8 + TGA_FOOTER_MAGIC.len()].copy_from_slice(TGA_FOOTER_MAGIC.as_slice());
        out
    }
}

/// Fixed size of the TGA 2.0 extension area (spec §C.6) when written
/// per the standard. Real-world files always use exactly this size; the
/// `extension_size` field at offset 0 is parsed defensively but the
/// other field offsets are taken to be the canonical layout.
pub const TGA_EXTENSION_AREA_SIZE: usize = 495;

/// Parsed TGA 2.0 extension area body (spec §C.6).
///
/// String fields (`author_name`, `software_id`, `job_name`, comment
/// lines) are stored with trailing NUL bytes stripped and re-encoded as
/// `String` (the spec requires ASCII, but we accept UTF-8 lossy on
/// read — non-ASCII bytes from broken encoders won't blow up).
///
/// Numeric ratio fields (`pixel_aspect_ratio`, `gamma`) are returned
/// as `(numerator, denominator)` tuples; both default to `(0, 0)`
/// meaning "field not set".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgaExtensionArea {
    /// Reported size of the extension area in bytes (always 495 for
    /// canonical TGA 2.0; a smaller value indicates a truncated
    /// non-conformant file, larger values are reserved for future spec
    /// versions).
    pub extension_size: u16,
    /// Author of the image, ≤ 40 ASCII characters per spec.
    pub author_name: String,
    /// Author comment lines (always exactly 4, each ≤ 80 ASCII chars).
    pub author_comment: [String; 4],
    /// Date/time stamp the image was saved: (month 1-12, day 1-31,
    /// year YYYY, hour 0-23, minute 0-59, second 0-59). All zeros if
    /// not set.
    pub timestamp: TgaTimestamp,
    /// Name/ID of the job under which the image was made.
    pub job_name: String,
    /// Job elapsed time at save: (hours, minutes, seconds).
    pub job_time: (u16, u16, u16),
    /// Software ID that wrote the image.
    pub software_id: String,
    /// Software version, encoded as `(version × 100, letter)` per spec
    /// §C.6.7. `version_letter` is `' '` (0x20) when no letter applies.
    pub software_version: (u16, char),
    /// Key colour (ARGB packed): the colour considered "transparent"
    /// when displayed. Stored as `[A, R, G, B]`.
    pub key_color: [u8; 4],
    /// Pixel aspect ratio as (width, height); a value of `(0, 0)`
    /// means "no specific ratio" (i.e. square pixels assumed).
    pub pixel_aspect_ratio: (u16, u16),
    /// Gamma correction factor as (numerator, denominator); `(0, 0)`
    /// means "not specified".
    pub gamma: (u16, u16),
    /// Byte offset of the colour-correction table (0 = absent).
    pub colour_correction_offset: u32,
    /// Byte offset of the postage-stamp (thumbnail) image (0 = absent).
    pub postage_stamp_offset: u32,
    /// Byte offset of the scan-line table (0 = absent).
    pub scan_line_offset: u32,
    /// Attributes type for the alpha channel (spec §C.6.13):
    /// 0 = no alpha data
    /// 1 = undefined data, ignore
    /// 2 = undefined data, retain
    /// 3 = useful alpha channel data
    /// 4 = pre-multiplied alpha
    pub attributes_type: u8,
}

/// Typed view of the extension-area `attributes_type` byte (spec §C.6.13).
///
/// The on-disk value is a single byte; this enum is the friendlier
/// surface that pattern-matches over its five defined values. Use
/// [`AttributesType::from_u8`] to decode and [`AttributesType::as_u8`]
/// to encode. Any value outside `0..=4` decodes as
/// [`AttributesType::Reserved`] carrying the raw byte so round-tripping
/// stays bit-exact even for non-standard files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttributesType {
    /// `0` — no alpha data included.
    #[default]
    NoAlpha,
    /// `1` — undefined alpha data, may be ignored.
    UndefinedIgnore,
    /// `2` — undefined alpha data, but should be retained.
    UndefinedRetain,
    /// `3` — useful alpha-channel data (straight, non-premultiplied).
    UsefulAlpha,
    /// `4` — pre-multiplied alpha.
    PremultipliedAlpha,
    /// Any value not assigned by the spec; preserved verbatim.
    Reserved(u8),
}

impl AttributesType {
    /// Decode the §C.6.13 byte. Values 0..=4 map to the named variants;
    /// anything else becomes [`Self::Reserved`] preserving the byte.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::NoAlpha,
            1 => Self::UndefinedIgnore,
            2 => Self::UndefinedRetain,
            3 => Self::UsefulAlpha,
            4 => Self::PremultipliedAlpha,
            other => Self::Reserved(other),
        }
    }

    /// Encode back to the §C.6.13 byte.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::NoAlpha => 0,
            Self::UndefinedIgnore => 1,
            Self::UndefinedRetain => 2,
            Self::UsefulAlpha => 3,
            Self::PremultipliedAlpha => 4,
            Self::Reserved(v) => v,
        }
    }

    /// `true` for the variants that mean "the alpha channel carries
    /// meaningful image data" (`UsefulAlpha` or `PremultipliedAlpha`).
    /// Consumers can use this to decide whether the alpha channel is
    /// safe to composite against.
    pub fn has_meaningful_alpha(self) -> bool {
        matches!(self, Self::UsefulAlpha | Self::PremultipliedAlpha)
    }

    /// `true` for [`Self::PremultipliedAlpha`].
    pub fn is_premultiplied(self) -> bool {
        matches!(self, Self::PremultipliedAlpha)
    }

    /// Normalise a single decoded 8-bit `[R, G, B, A]` pixel to
    /// **straight** (non-premultiplied) alpha according to this
    /// attributes type (spec §C.6.13).
    ///
    /// The decoder produces verbatim on-disk samples; the attributes
    /// byte tells a consumer how those samples should be interpreted.
    /// This method maps any defined attributes type onto the common
    /// "straight RGBA" convention so a single compositing path works for
    /// every file:
    ///
    /// * [`Self::NoAlpha`] / [`Self::UndefinedIgnore`] — the alpha bytes
    ///   carry no meaningful data (spec: value 0 means "bits 3-0 of the
    ///   image descriptor should also be zero"; value 1 means "undefined
    ///   data … can be ignored"). The pixel is forced opaque (`A = 255`),
    ///   leaving the colour channels untouched.
    /// * [`Self::UndefinedRetain`] — the alpha is undefined but should be
    ///   preserved (spec: "should be retained"); the pixel is returned
    ///   verbatim.
    /// * [`Self::UsefulAlpha`] — the alpha is already straight; the pixel
    ///   is returned verbatim.
    /// * [`Self::PremultipliedAlpha`] — the colour channels were scaled
    ///   by the alpha at save time (spec example: straight
    ///   `(a,r,g,b) = (0.5, 1, 0, 0)` is stored as `(0.5, 0.5, 0, 0)`).
    ///   Each colour channel is un-premultiplied by dividing by the
    ///   normalised alpha — `straight = round(stored × 255 / A)` —
    ///   clamped to `0..=255`. A fully transparent pixel (`A == 0`) has
    ///   no recoverable colour and is returned as transparent black.
    /// * [`Self::Reserved`] — an unknown attributes byte carries no
    ///   defined semantics; the pixel is returned verbatim so nothing is
    ///   silently corrupted.
    pub fn normalize_rgba8(self, rgba: [u8; 4]) -> [u8; 4] {
        match self {
            Self::NoAlpha | Self::UndefinedIgnore => [rgba[0], rgba[1], rgba[2], 0xFF],
            Self::UndefinedRetain | Self::UsefulAlpha | Self::Reserved(_) => rgba,
            Self::PremultipliedAlpha => {
                let a = rgba[3];
                if a == 0 {
                    return [0, 0, 0, 0];
                }
                let unmul = |c: u8| -> u8 {
                    // straight = stored × 255 / A, rounded to nearest,
                    // clamped to the 8-bit range (a colour channel may
                    // legitimately exceed its alpha in a malformed file).
                    let num = c as u32 * 255 + (a as u32) / 2;
                    (num / a as u32).min(255) as u8
                };
                [unmul(rgba[0]), unmul(rgba[1]), unmul(rgba[2]), a]
            }
        }
    }

    /// Apply [`Self::normalize_rgba8`] in place across a decoded
    /// [`crate::image::TgaImage`], normalising it to straight alpha per
    /// this attributes type (spec §C.6.13).
    ///
    /// Only [`crate::image::TgaPixelFormat::Rgba`] images carry an alpha
    /// channel, so that is the only format affected:
    ///
    /// * [`crate::image::TgaPixelFormat::Rgb24`] and
    ///   [`crate::image::TgaPixelFormat::Gray8`] have no alpha to
    ///   interpret and are left untouched.
    ///
    /// [`Self::UndefinedRetain`], [`Self::UsefulAlpha`], and
    /// [`Self::Reserved`] leave an RGBA image bit-for-bit unchanged;
    /// [`Self::NoAlpha`] / [`Self::UndefinedIgnore`] force every pixel
    /// opaque; [`Self::PremultipliedAlpha`] un-premultiplies every pixel.
    pub fn apply_to_image(self, image: &mut crate::image::TgaImage) {
        if image.pixel_format != crate::image::TgaPixelFormat::Rgba {
            return;
        }
        // Fast path: the no-op variants don't need to walk the buffer.
        if matches!(
            self,
            Self::UndefinedRetain | Self::UsefulAlpha | Self::Reserved(_)
        ) {
            return;
        }
        for px in image.data.chunks_exact_mut(4) {
            let out = self.normalize_rgba8([px[0], px[1], px[2], px[3]]);
            px.copy_from_slice(&out);
        }
    }
}

/// Date/time stamp embedded in the extension area's **Field 13
/// (Date/Time Stamp)** at bytes 367-378 (12 bytes / 6 SHORTs). All
/// zeros means "not set" per the spec convention "If the fields are
/// not used, you should fill them with binary zeros (0)".
///
/// Per the spec PDF the six SHORTs are:
///
/// * SHORT 0 — Month (`1..=12`)
/// * SHORT 1 — Day (`1..=31`)
/// * SHORT 2 — Year (4-digit, e.g. `1989`)
/// * SHORT 3 — Hour (`0..=23`)
/// * SHORT 4 — Minute (`0..=59`)
/// * SHORT 5 — Second (`0..=59`)
///
/// Typed accessors live in the `impl` block further below
/// ([`TgaTimestamp::is_unset`], [`TgaTimestamp::is_valid`],
/// [`TgaTimestamp::as_tuple`], [`TgaTimestamp::from_tuple`],
/// [`TgaTimestamp::iso8601`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TgaTimestamp {
    pub month: u16,
    pub day: u16,
    pub year: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

/// Typed view of the extension area's **Field 15 (Job Time)** at bytes
/// 420-425 (6 bytes / 3 SHORTs).
///
/// Per the spec PDF the three SHORTs are:
///
/// * SHORT 0 — Hours elapsed (`0..=65535`, full SHORT range)
/// * SHORT 1 — Minutes elapsed (`0..=59`)
/// * SHORT 2 — Seconds elapsed (`0..=59`)
///
/// "If the fields are not used, you should fill them with binary
/// zeros (0)", so `(0, 0, 0)` is the spec sentinel for "field not
/// being used".
///
/// The field's purpose per spec is to "keep a running total of the
/// amount of time invested in a particular image" for billing,
/// costing, and time estimating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JobTime {
    /// Elapsed hours (full SHORT range, `0..=65535`).
    pub hours: u16,
    /// Elapsed minutes (`0..=59`).
    pub minutes: u16,
    /// Elapsed seconds (`0..=59`).
    pub seconds: u16,
}

impl JobTime {
    /// Spec sentinel: `(0, 0, 0)` per the §C.6 "fill with binary
    /// zeros" convention for an unused field.
    pub const UNSET: Self = Self {
        hours: 0,
        minutes: 0,
        seconds: 0,
    };

    /// Wrap a `(hours, minutes, seconds)` SHORT triple.
    pub fn new(hours: u16, minutes: u16, seconds: u16) -> Self {
        Self {
            hours,
            minutes,
            seconds,
        }
    }

    /// Wrap a `(hours, minutes, seconds)` tuple — mirror of
    /// [`Self::as_tuple`] for round-trip ergonomics.
    pub fn from_tuple(triple: (u16, u16, u16)) -> Self {
        Self {
            hours: triple.0,
            minutes: triple.1,
            seconds: triple.2,
        }
    }

    /// Re-emit the on-disk `(hours, minutes, seconds)` SHORT triple
    /// (the encoder's `ExtensionAreaInput::job_time` shape).
    pub fn as_tuple(self) -> (u16, u16, u16) {
        (self.hours, self.minutes, self.seconds)
    }

    /// `true` when every field is zero (the spec sentinel for an
    /// unused job-time field).
    pub fn is_unset(self) -> bool {
        self.hours == 0 && self.minutes == 0 && self.seconds == 0
    }

    /// `true` when minutes and seconds land in their spec ranges
    /// (`0..=59`). Hours is the full SHORT range (`0..=65535`) per the
    /// spec, so it is always in range. An unset job-time is also
    /// considered valid (every zero is in range).
    pub fn is_valid(self) -> bool {
        self.minutes <= 59 && self.seconds <= 59
    }

    /// Total elapsed seconds (`hours × 3600 + minutes × 60 + seconds`)
    /// as `u32` — the SHORT-range hours cap at 65 535, so the maximum
    /// total is `65 535 × 3600 + 59 × 60 + 59 = 235 929 599`, well
    /// inside `u32`.
    pub fn total_seconds(self) -> u32 {
        self.hours as u32 * 3600 + self.minutes as u32 * 60 + self.seconds as u32
    }

    /// Total elapsed hours as `f64` — convenient for "X hours of work"
    /// reports without losing the minute/second resolution.
    pub fn as_f64_hours(self) -> f64 {
        self.total_seconds() as f64 / 3600.0
    }

    /// Format the elapsed time as `"H:MM:SS"` with `H` zero-padded to
    /// at least 2 digits when the hours value is below 100, and
    /// `MM` / `SS` always zero-padded to 2 digits.
    ///
    /// Examples:
    ///
    /// * `(0, 0, 0)` → `"00:00:00"`
    /// * `(1, 23, 45)` → `"01:23:45"`
    /// * `(100, 0, 0)` → `"100:00:00"` (hours can exceed the 2-digit pad)
    pub fn hms_string(self) -> String {
        format!("{:02}:{:02}:{:02}", self.hours, self.minutes, self.seconds)
    }
}

/// Maximum payload character count for a 41-byte ASCII extension-area
/// field (Author Name / Job Name / Software ID).
///
/// The 41st byte is reserved for the trailing NUL terminator per spec —
/// the writable payload is therefore at most 40 ASCII characters.
pub const TGA_ASCII_FIELD_MAX_CHARS: usize = 40;

/// Width in bytes (including the trailing NUL) of one Author Comment
/// line on disk.
pub const TGA_AUTHOR_COMMENT_LINE_BYTES: usize = 81;

/// Maximum payload character count for one Author Comment line.
pub const TGA_AUTHOR_COMMENT_LINE_MAX_CHARS: usize = 80;

/// Number of Author Comment lines stored in the extension area.
pub const TGA_AUTHOR_COMMENT_LINES: usize = 4;

/// Typed view of one of the extension area's 41-byte ASCII fields —
/// **Author Name** (Field 11), **Job Name/ID** (Field 14), or
/// **Software ID** (Field 16).
///
/// All three fields share the same on-disk shape per the TGA 2.0 spec:
/// a fixed 41-byte slot whose payload is at most
/// [`TGA_ASCII_FIELD_MAX_CHARS`] ASCII characters, with the 41st byte
/// reserved as a binary-zero (NUL) terminator. An unused field is
/// recommended to be filled with NULs (or NUL-terminated blanks).
///
/// This typed view wraps the parsed payload as a `String` — the parser
/// already strips trailing NULs and decodes UTF-8 lossily, so the same
/// `String` instance round-trips through the encoder bit-exactly when
/// re-emitted. Helper methods classify the field per the spec
/// conventions:
///
/// * [`Self::is_unset`] — the field is empty, or every byte is a NUL
///   or ASCII space (the spec's "blanks terminated by a null" idiom).
/// * [`Self::is_valid_ascii`] — every byte is a printable ASCII
///   character (`0x20..=0x7E`) per the spec's recommendation that
///   "ASCII fields contain only printable ASCII characters".
/// * [`Self::trimmed`] — the payload with leading + trailing ASCII
///   whitespace removed (a borrowed sub-slice).
/// * [`Self::char_len`] — the byte length, which is also the character
///   count for valid ASCII payloads.
/// * [`Self::fits_capacity`] — the payload fits in the 40-character
///   on-disk slot; otherwise the encoder will silently truncate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TgaAsciiField {
    /// Parsed UTF-8 string (NUL-stripped on read; encoder silently
    /// truncates to [`TGA_ASCII_FIELD_MAX_CHARS`] when re-emitting).
    pub value: String,
}

impl TgaAsciiField {
    /// Wrap a `String` payload. Pass [`String::new()`] for an unset
    /// field.
    pub fn new(value: String) -> Self {
        Self { value }
    }

    /// Wrap a borrowed `&str` payload — convenience for tests and
    /// callers that hold their own owned string. Named `from_borrowed`
    /// (rather than `from_str`) to avoid colliding with
    /// `std::str::FromStr::from_str`.
    pub fn from_borrowed(s: &str) -> Self {
        Self {
            value: s.to_string(),
        }
    }

    /// Unwrap the payload string.
    pub fn into_inner(self) -> String {
        self.value
    }

    /// Borrow the payload string.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// `true` when the field carries the spec's "not being used"
    /// signal — empty payload, or every byte a NUL or ASCII space
    /// (the spec says: "you may fill it with nulls or a series of
    /// blanks (spaces) terminated by a null"). A non-ASCII byte that
    /// happens to be neither NUL nor space (i.e. high-bit set) is
    /// *not* treated as a sentinel.
    pub fn is_unset(&self) -> bool {
        self.value.is_empty() || self.value.bytes().all(|b| b == 0 || b == b' ')
    }

    /// `true` when every byte is a printable ASCII character
    /// (`0x20..=0x7E`). The spec recommends the field carry only
    /// printable ASCII; this check surfaces deviations without forcing
    /// rejection at parse time. An empty string is trivially valid.
    pub fn is_valid_ascii(&self) -> bool {
        self.value.bytes().all(|b| (0x20..=0x7E).contains(&b))
    }

    /// Number of bytes in the payload — also the character count when
    /// [`Self::is_valid_ascii`] is true.
    pub fn char_len(&self) -> usize {
        self.value.len()
    }

    /// `true` when the payload fits in the on-disk 40-character slot.
    /// The encoder silently truncates oversized payloads at write time
    /// (it can never exceed the 41-byte fixed field), so a caller that
    /// wants exact round-trip should ensure this is true before
    /// encoding.
    pub fn fits_capacity(&self) -> bool {
        self.value.len() <= TGA_ASCII_FIELD_MAX_CHARS
    }

    /// Payload with leading + trailing ASCII whitespace stripped.
    /// Useful when a writer filled the slot with `"NAME        \0"`
    /// and the reader wants just `"NAME"`.
    pub fn trimmed(&self) -> &str {
        self.value.trim_matches(|c: char| c.is_ascii_whitespace())
    }
}

/// Typed view of the extension area's **Author Comments** (Field 12)
/// 324-byte block — four 81-byte lines, each 80 ASCII characters plus
/// a binary-zero (NUL) terminator.
///
/// The four lines on disk are independent: each one is NUL-terminated
/// inside its 81-byte slot. The parser strips trailing NULs per line
/// and the encoder re-pads them; an unused line is recommended to be
/// filled with NULs or NUL-terminated blanks.
///
/// Helper methods mirror [`TgaAsciiField`] for the per-line and
/// whole-block conveniences callers usually want.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TgaAuthorComments {
    /// The four comment lines.
    pub lines: [String; TGA_AUTHOR_COMMENT_LINES],
}

impl TgaAuthorComments {
    /// Spec sentinel: four empty lines (every byte in the 324-byte
    /// block a NUL) per the §C.6 "fill with binary zeros" convention.
    pub const fn empty() -> Self {
        Self {
            lines: [String::new(), String::new(), String::new(), String::new()],
        }
    }

    /// Wrap four owned `String`s.
    pub fn new(lines: [String; TGA_AUTHOR_COMMENT_LINES]) -> Self {
        Self { lines }
    }

    /// Wrap four borrowed `&str` payloads.
    pub fn from_strs(lines: [&str; TGA_AUTHOR_COMMENT_LINES]) -> Self {
        Self {
            lines: [
                lines[0].to_string(),
                lines[1].to_string(),
                lines[2].to_string(),
                lines[3].to_string(),
            ],
        }
    }

    /// Borrow line `i` as `&str`. `i` is bounds-checked.
    pub fn line(&self, i: usize) -> Option<&str> {
        self.lines.get(i).map(String::as_str)
    }

    /// `true` when every line returns `true` from
    /// [`TgaAsciiField::is_unset`] — empty / blanks / NULs throughout.
    pub fn is_unset(&self) -> bool {
        self.lines
            .iter()
            .all(|s| s.is_empty() || s.bytes().all(|b| b == 0 || b == b' '))
    }

    /// `true` when every line is printable ASCII (`0x20..=0x7E`).
    pub fn is_valid_ascii(&self) -> bool {
        self.lines
            .iter()
            .all(|s| s.bytes().all(|b| (0x20..=0x7E).contains(&b)))
    }

    /// `true` when every line fits the 80-character per-line cap. The
    /// encoder silently truncates oversized lines.
    pub fn fits_capacity(&self) -> bool {
        self.lines
            .iter()
            .all(|s| s.len() <= TGA_AUTHOR_COMMENT_LINE_MAX_CHARS)
    }

    /// Join all four lines with `"\n"` separators, dropping trailing
    /// blank lines. Convenience for display surfaces that want a
    /// single human-readable paragraph rather than four fixed slots.
    pub fn joined(&self) -> String {
        let mut end = self.lines.len();
        while end > 0
            && (self.lines[end - 1].is_empty()
                || self.lines[end - 1].bytes().all(|b| b == 0 || b == b' '))
        {
            end -= 1;
        }
        self.lines[..end].join("\n")
    }
}

/// Typed view of the extension area's §C.6.4 **Key Color** quadruple.
///
/// The on-disk layout is four bytes ordered `A : R : G : B` (most
/// significant byte first; alpha if present, otherwise `0`). The spec
/// describes this field as "the background or transparent colour … the
/// colour of the non-image area of the screen". When no key colour is
/// recorded the field is set to all zeros. Note: an all-zero quadruple
/// is explicitly documented to also mean "key colour = black", so a
/// reader cannot distinguish "unset" from "intentional black" from the
/// four bytes alone — callers that need that distinction must consult
/// the surrounding context (typically presence of a non-default
/// `attributes_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyColor {
    /// Alpha-channel byte of the key colour (most significant byte of
    /// the on-disk `A:R:G:B` quadruple).
    pub a: u8,
    /// Red byte.
    pub r: u8,
    /// Green byte.
    pub g: u8,
    /// Blue byte.
    pub b: u8,
}

impl KeyColor {
    /// Read the four-byte spec layout (`[A, R, G, B]`).
    pub fn from_argb(bytes: [u8; 4]) -> Self {
        Self {
            a: bytes[0],
            r: bytes[1],
            g: bytes[2],
            b: bytes[3],
        }
    }

    /// Re-emit the on-disk spec layout (`[A, R, G, B]`).
    pub fn to_argb(self) -> [u8; 4] {
        [self.a, self.r, self.g, self.b]
    }

    /// View the colour in the framework-canonical `[R, G, B, A]` order.
    /// This is the order returned by [`crate::image::TgaPixelFormat::Rgba`]
    /// pixels — useful for chroma-keying a decoded RGBA image.
    pub fn as_rgba8(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// `true` for the all-zero "field not set" sentinel. Per spec
    /// §C.6.4, this is the same byte pattern a deliberate key-colour
    /// of opaque black would use; callers wanting to distinguish the
    /// two must look elsewhere in the file.
    pub fn is_unset(self) -> bool {
        self.a == 0 && self.r == 0 && self.g == 0 && self.b == 0
    }

    /// `true` when the §C.6.4 alpha byte is non-zero. The spec
    /// recommends `a == 0` "if you don't have an alpha channel in
    /// your application".
    pub fn has_alpha(self) -> bool {
        self.a != 0
    }
}

/// Typed view of the extension area's §C.6.5 **Pixel Aspect Ratio**
/// (`numerator / denominator`, expressed as the on-disk
/// `pixel_aspect_ratio` SHORT pair).
///
/// Per spec §C.6.5:
/// * `(0, 0)` means "no pixel aspect ratio specified" (square pixels
///   assumed by the consumer).
/// * Equal non-zero values mean "square pixels".
/// * Otherwise the value is the displayed pixel's width divided by its
///   height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PixelAspectRatio {
    /// Pixel-width SHORT (numerator).
    pub numerator: u16,
    /// Pixel-height SHORT (denominator).
    pub denominator: u16,
}

impl PixelAspectRatio {
    /// Spec sentinel for "field not specified".
    pub const UNSET: Self = Self {
        numerator: 0,
        denominator: 0,
    };

    /// Wrap a `(numerator, denominator)` SHORT pair.
    pub fn new(numerator: u16, denominator: u16) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// Re-emit the on-disk `(numerator, denominator)` tuple.
    pub fn as_tuple(self) -> (u16, u16) {
        (self.numerator, self.denominator)
    }

    /// `true` when the field carries the §C.6.5 "no pixel aspect ratio
    /// specified" sentinel (`(0, 0)`).
    pub fn is_unset(self) -> bool {
        self.numerator == 0 && self.denominator == 0
    }

    /// `true` when the field encodes square pixels — either the spec's
    /// `(0, 0)` sentinel (square pixels assumed) or any equal non-zero
    /// pair.
    pub fn is_square(self) -> bool {
        self.numerator == self.denominator
    }

    /// Numerical aspect ratio (`numerator / denominator`) as `f32`.
    /// Returns `None` for the spec's `(0, 0)` sentinel and for any
    /// pair with a zero denominator (the spec calls a zero denominator
    /// out as "no pixel aspect ratio is specified" per §C.6.5).
    pub fn as_f32(self) -> Option<f32> {
        if self.denominator == 0 {
            return None;
        }
        Some(self.numerator as f32 / self.denominator as f32)
    }

    /// Compute the corrected display height that preserves the source
    /// width's pixel-aspect-corrected appearance.
    ///
    /// Given a stored frame of `width × height` pixels written with
    /// this aspect ratio, the resampled display dimensions
    /// `(width, height × denominator / numerator)` yield square
    /// display pixels.
    ///
    /// Returns `None` if [`Self::as_f32`] does (unset / square-only).
    /// Rounding is to-nearest, ties to even via `f32::round`. Output
    /// height is clamped to `>= 1` so a degenerate ratio doesn't
    /// produce a zero-row frame.
    pub fn corrected_display_height(self, height: u32) -> Option<u32> {
        let r = self.as_f32()?;
        if !r.is_finite() || r <= 0.0 {
            return None;
        }
        let h = (height as f32 / r).round();
        Some((h as u32).max(1))
    }

    /// Companion to [`Self::corrected_display_height`]: corrected
    /// display width that preserves square display pixels for a
    /// `width × height` stored frame.
    pub fn corrected_display_width(self, width: u32) -> Option<u32> {
        let r = self.as_f32()?;
        if !r.is_finite() || r <= 0.0 {
            return None;
        }
        let w = (width as f32 * r).round();
        Some((w as u32).max(1))
    }
}

/// Typed view of the extension area's §C.6.6 **Gamma Value**
/// (`numerator / denominator`, SHORT pair).
///
/// Per spec §C.6.6:
/// * `(0, 0)` and any pair with `denominator == 0` mean "field not
///   being used".
/// * The resulting value should land in `0.0..=10.0` with one decimal
///   place of useful precision; an uncorrected image carries `(1, 1)`
///   (gamma 1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GammaValue {
    /// Gamma SHORT numerator.
    pub numerator: u16,
    /// Gamma SHORT denominator.
    pub denominator: u16,
}

impl GammaValue {
    /// Spec sentinel for "gamma value field not being used".
    pub const UNSET: Self = Self {
        numerator: 0,
        denominator: 0,
    };
    /// Convenience constant for an uncorrected (`1.0`) image per spec
    /// §C.6.6.
    pub const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };

    /// Wrap a `(numerator, denominator)` SHORT pair.
    pub fn new(numerator: u16, denominator: u16) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// Re-emit the on-disk `(numerator, denominator)` tuple.
    pub fn as_tuple(self) -> (u16, u16) {
        (self.numerator, self.denominator)
    }

    /// `true` when the field carries the §C.6.6 "not being used"
    /// signal — either the all-zero pair or any pair with a zero
    /// denominator (the spec explicitly calls "set the denominator to
    /// zero" as the way to mark the field unused).
    pub fn is_unset(self) -> bool {
        self.denominator == 0
    }

    /// `true` when the field encodes an uncorrected (gamma == 1.0)
    /// image (i.e. `numerator == denominator` and the field is set).
    pub fn is_identity(self) -> bool {
        !self.is_unset() && self.numerator == self.denominator
    }

    /// Numerical gamma value (`numerator / denominator`) as `f32`.
    /// Returns `None` for the spec's "field not being used" signal.
    pub fn as_f32(self) -> Option<f32> {
        if self.is_unset() {
            return None;
        }
        Some(self.numerator as f32 / self.denominator as f32)
    }

    /// Apply the inverse-gamma transform `straight = stored^gamma` to a
    /// single 8-bit channel sample. The §C.6.6 gamma value is the
    /// power-curve exponent the stored samples were already encoded
    /// with; raising to that exponent recovers the linear-light value
    /// the consumer expects. Bounded to `0..=255` after `round` to
    /// nearest.
    ///
    /// Behaviour:
    /// * `is_unset()` → input is returned unchanged (no gamma → no-op).
    /// * `is_identity()` → input is returned unchanged (`x^1 == x`).
    /// * out-of-range gamma (≤ 0 or non-finite) → input is returned
    ///   unchanged so a malformed file never blows up downstream
    ///   compositing.
    pub fn apply_to_channel8(self, sample: u8) -> u8 {
        let g = match self.as_f32() {
            Some(g) if g.is_finite() && g > 0.0 => g,
            _ => return sample,
        };
        if (g - 1.0).abs() < f32::EPSILON {
            return sample;
        }
        let x = sample as f32 / 255.0;
        let y = x.powf(g) * 255.0;
        y.round().clamp(0.0, 255.0) as u8
    }

    /// Apply [`Self::apply_to_channel8`] to the R/G/B channels of an
    /// `[R, G, B, A]` pixel, leaving the alpha byte untouched.
    pub fn apply_to_rgba8(self, rgba: [u8; 4]) -> [u8; 4] {
        if self.is_unset() || self.is_identity() {
            return rgba;
        }
        [
            self.apply_to_channel8(rgba[0]),
            self.apply_to_channel8(rgba[1]),
            self.apply_to_channel8(rgba[2]),
            rgba[3],
        ]
    }

    /// Apply [`Self::apply_to_rgba8`] to every pixel of a decoded
    /// [`crate::image::TgaImage`] in place.
    ///
    /// * RGBA images: every pixel's R/G/B channels are re-mapped;
    ///   alpha is left alone.
    /// * Rgb24 images: every byte in row-major order is treated as a
    ///   colour channel and re-mapped.
    /// * Gray8 images: every byte (luma) is re-mapped.
    ///
    /// Fast-path: no-op when the field is unset / identity / malformed,
    /// matching [`AttributesType::apply_to_image`] semantics.
    pub fn apply_to_image(self, image: &mut crate::image::TgaImage) {
        if self.is_unset() || self.is_identity() {
            return;
        }
        // Validate finiteness once up front; an unrecoverable gamma is
        // a no-op so an arbitrary file never panics downstream.
        match self.as_f32() {
            Some(g) if g.is_finite() && g > 0.0 => {}
            _ => return,
        }
        match image.pixel_format {
            crate::image::TgaPixelFormat::Rgba => {
                for px in image.data.chunks_exact_mut(4) {
                    let out = self.apply_to_rgba8([px[0], px[1], px[2], px[3]]);
                    px.copy_from_slice(&out);
                }
            }
            crate::image::TgaPixelFormat::Rgb24 => {
                for c in image.data.iter_mut() {
                    *c = self.apply_to_channel8(*c);
                }
            }
            crate::image::TgaPixelFormat::Gray8 => {
                for c in image.data.iter_mut() {
                    *c = self.apply_to_channel8(*c);
                }
            }
        }
    }
}

/// Typed view of the extension area's §C.6.7 **Software Version**
/// triple. The on-disk layout is a SHORT (`version × 100`) followed by
/// a single ASCII BYTE (release-letter; space `' '` when unused).
///
/// Per spec §C.6.7 example: version `1.17b` is stored as
/// `(117, 'b')`. An unused field is `(0, ' ')`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftwareVersion {
    /// `version × 100` (e.g. `417` for version `4.17`).
    pub number_times_100: u16,
    /// ASCII release-letter suffix; `' '` (0x20) when no letter.
    pub letter: char,
}

impl Default for SoftwareVersion {
    fn default() -> Self {
        Self::UNSET
    }
}

impl SoftwareVersion {
    /// Spec sentinel: `(0, ' ')` per §C.6.7's "if you do not use this
    /// field".
    pub const UNSET: Self = Self {
        number_times_100: 0,
        letter: ' ',
    };

    /// Wrap a `(version × 100, release-letter)` pair.
    pub fn new(number_times_100: u16, letter: char) -> Self {
        Self {
            number_times_100,
            letter,
        }
    }

    /// Re-emit the on-disk `(SHORT, BYTE-as-char)` tuple.
    pub fn as_tuple(self) -> (u16, char) {
        (self.number_times_100, self.letter)
    }

    /// `true` when the field carries the §C.6.7 "not being used"
    /// signal (`(0, ' ')`).
    pub fn is_unset(self) -> bool {
        self.number_times_100 == 0 && self.letter == ' '
    }

    /// Numerical version (`number_times_100 / 100.0`).
    ///
    /// Returns `None` when [`Self::is_unset`] is `true`. The spec uses
    /// `× 100` to give two decimals of precision; an `8.42` version is
    /// stored as `842`.
    pub fn as_f32(self) -> Option<f32> {
        if self.is_unset() {
            return None;
        }
        Some(self.number_times_100 as f32 / 100.0)
    }
}

/// Truevision's recommended maximum postage-stamp edge length, per spec
/// **Field 26 (Postage Stamp Image)**: "Truevision does not recommend
/// stamps larger than 64 x 64 pixels, and suggests that any stamps stored
/// larger be clipped." The recommendation is advisory — the on-disk size
/// header is a single byte per axis, so the format permits up to
/// 255 × 255 — and this constant carries the recommended cap so callers
/// can size or clip a thumbnail before embedding it.
pub const TGA_POSTAGE_STAMP_RECOMMENDED_MAX: u8 = 64;

/// Hard on-disk limit for a postage-stamp edge length, per spec
/// **Field 26**: "The first byte of the postage stamp image specifies the
/// X size of the stamp in pixels, the second byte … the Y size". Both
/// dimensions are single unsigned bytes, so neither axis can exceed 255.
pub const TGA_POSTAGE_STAMP_MAX: u8 = 255;

/// Typed view of the extension area's **Field 26 (Postage Stamp Image)**
/// dimension header per spec §C.6.10.
///
/// The postage stamp is the embedded thumbnail; its pixel payload is
/// decoded by [`crate::parse_tga_postage_stamp`]. This struct is the
/// lightweight view of just the two on-disk size bytes that prefix that
/// payload — "The first byte of the postage stamp image specifies the X
/// size of the stamp in pixels, the second byte … the Y size" — so a
/// caller can inspect or validate the stamp geometry (and the spec's
/// 64 × 64 recommendation) without decoding the pixels.
///
/// Both axes are single bytes on disk (`0..=255`); `(0, 0)` is the
/// "no stamp" sentinel (the offset that points at this header is itself
/// zero when no stamp is stored, per Field 22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PostageStamp {
    /// Stamp width in pixels (the first on-disk size byte).
    pub width: u8,
    /// Stamp height in pixels (the second on-disk size byte).
    pub height: u8,
}

impl PostageStamp {
    /// Spec sentinel for "no postage stamp" (`(0, 0)`).
    pub const UNSET: Self = Self {
        width: 0,
        height: 0,
    };

    /// Wrap a `(width, height)` pixel pair.
    pub fn new(width: u8, height: u8) -> Self {
        Self { width, height }
    }

    /// Re-emit the on-disk `(width, height)` byte pair (Field 26's two
    /// leading size bytes).
    pub fn as_tuple(self) -> (u8, u8) {
        (self.width, self.height)
    }

    /// Build from the on-disk `(width, height)` byte pair.
    pub fn from_tuple((width, height): (u8, u8)) -> Self {
        Self { width, height }
    }

    /// `true` when either dimension is zero. The spec stores the stamp
    /// only when Field 22 (Postage Stamp Offset) is non-zero, and a
    /// well-formed stamp has positive width and height, so a zero on
    /// either axis is the "absent / degenerate" case.
    pub fn is_unset(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Total pixel count (`width × height`). Promoted to `u32` so the
    /// 255 × 255 worst case (65025) does not overflow.
    pub fn pixel_count(self) -> u32 {
        self.width as u32 * self.height as u32
    }

    /// `true` when both edges fall within Truevision's recommended
    /// 64 × 64 maximum ([`TGA_POSTAGE_STAMP_RECOMMENDED_MAX`]). A larger
    /// stamp is still spec-legal — the recommendation is advisory and
    /// the on-disk header permits up to 255 per axis — but the spec
    /// "suggests that any stamps stored larger be clipped".
    pub fn within_recommended_size(self) -> bool {
        self.width <= TGA_POSTAGE_STAMP_RECOMMENDED_MAX
            && self.height <= TGA_POSTAGE_STAMP_RECOMMENDED_MAX
    }

    /// Clip both edges to Truevision's recommended 64 × 64 maximum,
    /// returning the clipped geometry. Stamps already within the
    /// recommendation are returned unchanged; this implements the spec's
    /// "suggests that any stamps stored larger be clipped" guidance at
    /// the dimension level (it does not touch pixel data — the caller
    /// resamples or crops the thumbnail to match).
    pub fn clipped_to_recommended(self) -> Self {
        Self {
            width: self.width.min(TGA_POSTAGE_STAMP_RECOMMENDED_MAX),
            height: self.height.min(TGA_POSTAGE_STAMP_RECOMMENDED_MAX),
        }
    }
}

impl TgaExtensionArea {
    /// Typed view of [`Self::attributes_type`] per spec §C.6.13.
    /// Equivalent to `AttributesType::from_u8(self.attributes_type)`.
    pub fn attributes(&self) -> AttributesType {
        AttributesType::from_u8(self.attributes_type)
    }

    /// Typed view of [`Self::key_color`] per spec §C.6.4.
    pub fn key_color_typed(&self) -> KeyColor {
        KeyColor::from_argb(self.key_color)
    }

    /// Typed view of [`Self::pixel_aspect_ratio`] per spec §C.6.5.
    pub fn pixel_aspect_ratio_typed(&self) -> PixelAspectRatio {
        PixelAspectRatio::new(self.pixel_aspect_ratio.0, self.pixel_aspect_ratio.1)
    }

    /// Typed view of [`Self::gamma`] per spec §C.6.6.
    pub fn gamma_typed(&self) -> GammaValue {
        GammaValue::new(self.gamma.0, self.gamma.1)
    }

    /// Typed view of [`Self::software_version`] per spec §C.6.7.
    pub fn software_version_typed(&self) -> SoftwareVersion {
        SoftwareVersion::new(self.software_version.0, self.software_version.1)
    }

    /// Typed view of [`Self::timestamp`] (Field 13 / "Date/Time
    /// Stamp"). This is the same struct already at
    /// [`Self::timestamp`]; the helper exists for naming symmetry
    /// with [`Self::job_time_typed`] / [`Self::key_color_typed`] /
    /// the rest of the `_typed` family.
    pub fn timestamp_typed(&self) -> TgaTimestamp {
        self.timestamp
    }

    /// Typed view of [`Self::job_time`] (Field 15 / "Job Time"). The
    /// on-disk tuple is wrapped in a [`JobTime`] with [`JobTime::is_unset`]
    /// / [`JobTime::is_valid`] / [`JobTime::total_seconds`] /
    /// [`JobTime::as_f64_hours`] / [`JobTime::hms_string`].
    pub fn job_time_typed(&self) -> JobTime {
        JobTime::from_tuple(self.job_time)
    }

    /// Typed view of [`Self::author_name`] (Field 11 / "Author Name").
    /// Wraps the parsed string in a [`TgaAsciiField`] surfacing
    /// `is_unset` / `is_valid_ascii` / `fits_capacity` / `trimmed`
    /// / `char_len` helpers.
    pub fn author_name_typed(&self) -> TgaAsciiField {
        TgaAsciiField::new(self.author_name.clone())
    }

    /// Typed view of [`Self::author_comment`] (Field 12 / "Author
    /// Comments"). Wraps the four parsed comment lines in a
    /// [`TgaAuthorComments`] surfacing `is_unset` / `is_valid_ascii` /
    /// `fits_capacity` / per-line `line()` / `joined()` helpers.
    pub fn author_comments_typed(&self) -> TgaAuthorComments {
        TgaAuthorComments::new(self.author_comment.clone())
    }

    /// Typed view of [`Self::job_name`] (Field 14 / "Job Name/ID").
    /// Wraps the parsed string in a [`TgaAsciiField`] surfacing the
    /// same helpers as [`Self::author_name_typed`].
    pub fn job_name_typed(&self) -> TgaAsciiField {
        TgaAsciiField::new(self.job_name.clone())
    }

    /// Typed view of [`Self::software_id`] (Field 16 / "Software ID").
    /// Wraps the parsed string in a [`TgaAsciiField`] surfacing the
    /// same helpers as [`Self::author_name_typed`].
    pub fn software_id_typed(&self) -> TgaAsciiField {
        TgaAsciiField::new(self.software_id.clone())
    }
}

impl TgaTimestamp {
    /// Spec sentinel: every field zero, per the §C.6 Date/Time field
    /// convention "If the fields are not used, you should fill them
    /// with binary zeros (0)".
    pub const UNSET: Self = Self {
        month: 0,
        day: 0,
        year: 0,
        hour: 0,
        minute: 0,
        second: 0,
    };

    /// `true` when every field is zero (the spec convention for "no
    /// timestamp set").
    pub fn is_unset(&self) -> bool {
        self.month == 0
            && self.day == 0
            && self.year == 0
            && self.hour == 0
            && self.minute == 0
            && self.second == 0
    }

    /// `true` when every field lands in the spec range:
    ///
    /// * Month ∈ `1..=12`
    /// * Day ∈ `1..=31` (no per-month calendar check — the spec only
    ///   defines the field width, not a calendar validator)
    /// * Year ≥ `1`     (the spec writes "4 digit, ie. 1989" so any
    ///   non-zero year is in range)
    /// * Hour ∈ `0..=23`
    /// * Minute ∈ `0..=59`
    /// * Second ∈ `0..=59`
    ///
    /// An unset timestamp is **not** valid by this check (the all-zero
    /// month + day + year fall outside the named ranges). Callers that
    /// want "unset OR valid" should chain
    /// `t.is_unset() || t.is_valid()`.
    pub fn is_valid(&self) -> bool {
        (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.year >= 1
            && self.hour <= 23
            && self.minute <= 59
            && self.second <= 59
    }

    /// Re-emit the timestamp as a `(month, day, year, hour, minute,
    /// second)` SHORT sextuple matching the on-disk SHORT order.
    pub fn as_tuple(&self) -> (u16, u16, u16, u16, u16, u16) {
        (
            self.month,
            self.day,
            self.year,
            self.hour,
            self.minute,
            self.second,
        )
    }

    /// Wrap a `(month, day, year, hour, minute, second)` sextuple in
    /// on-disk SHORT order — the mirror of [`Self::as_tuple`].
    pub fn from_tuple(t: (u16, u16, u16, u16, u16, u16)) -> Self {
        Self {
            month: t.0,
            day: t.1,
            year: t.2,
            hour: t.3,
            minute: t.4,
            second: t.5,
        }
    }

    /// Format the timestamp as a sortable ISO-8601 calendar-and-time
    /// string `"YYYY-MM-DDTHH:MM:SS"`:
    ///
    /// * `year` is zero-padded to at least 4 digits.
    /// * Every other field is zero-padded to 2 digits.
    /// * No timezone suffix — the spec leaves the timestamp's zone
    ///   unspecified, so the output is a naive local datetime.
    ///
    /// Returns `None` when [`Self::is_unset`] is `true` so callers can
    /// treat the sentinel as "no timestamp" rather than rendering
    /// `"0000-00-00T00:00:00"`.
    pub fn iso8601(&self) -> Option<String> {
        if self.is_unset() {
            return None;
        }
        Some(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        ))
    }
}

impl Default for TgaExtensionArea {
    fn default() -> Self {
        Self {
            extension_size: TGA_EXTENSION_AREA_SIZE as u16,
            author_name: String::new(),
            author_comment: [String::new(), String::new(), String::new(), String::new()],
            timestamp: TgaTimestamp::default(),
            job_name: String::new(),
            job_time: (0, 0, 0),
            software_id: String::new(),
            software_version: (0, ' '),
            key_color: [0; 4],
            pixel_aspect_ratio: (0, 0),
            gamma: (0, 0),
            colour_correction_offset: 0,
            postage_stamp_offset: 0,
            scan_line_offset: 0,
            attributes_type: 0,
        }
    }
}

/// Parse a TGA 2.0 extension area body from `input` starting at byte
/// offset `offset`. Returns `None` if `offset` is `0` (the convention
/// for "no extension area"), if the buffer is truncated, or if the
/// reported `extension_size` is so small that the standard fixed-layout
/// fields wouldn't fit.
pub fn parse_extension_area(input: &[u8], offset: u32) -> Option<TgaExtensionArea> {
    if offset == 0 {
        return None;
    }
    let off = offset as usize;
    // Need at least 2 bytes to read extension_size.
    if input.len() < off + 2 {
        return None;
    }
    let ext_size = read_u16_le(input, off);
    // Defensive: reject if extension area wouldn't fit in the buffer.
    // The spec layout is 495 bytes; we accept smaller too if the file
    // claims a smaller extension_size, but the canonical-offset fields
    // we read still need to fit.
    if input.len() < off + TGA_EXTENSION_AREA_SIZE {
        return None;
    }
    let buf = &input[off..off + TGA_EXTENSION_AREA_SIZE];

    let read_string = |slice: &[u8]| -> String {
        let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        String::from_utf8_lossy(&slice[..end]).into_owned()
    };

    let author_name = read_string(&buf[2..43]);
    let author_comment = [
        read_string(&buf[43..124]),
        read_string(&buf[124..205]),
        read_string(&buf[205..286]),
        read_string(&buf[286..367]),
    ];
    let timestamp = TgaTimestamp {
        month: read_u16_le(buf, 367),
        day: read_u16_le(buf, 369),
        year: read_u16_le(buf, 371),
        hour: read_u16_le(buf, 373),
        minute: read_u16_le(buf, 375),
        second: read_u16_le(buf, 377),
    };
    let job_name = read_string(&buf[379..420]);
    let job_time = (
        read_u16_le(buf, 420),
        read_u16_le(buf, 422),
        read_u16_le(buf, 424),
    );
    let software_id = read_string(&buf[426..467]);
    let software_version = (read_u16_le(buf, 467), buf[469] as char);
    let key_color = [buf[470], buf[471], buf[472], buf[473]];
    let pixel_aspect_ratio = (read_u16_le(buf, 474), read_u16_le(buf, 476));
    let gamma = (read_u16_le(buf, 478), read_u16_le(buf, 480));
    let colour_correction_offset = read_u32_le(buf, 482);
    let postage_stamp_offset = read_u32_le(buf, 486);
    let scan_line_offset = read_u32_le(buf, 490);
    let attributes_type = buf[494];

    Some(TgaExtensionArea {
        extension_size: ext_size,
        author_name,
        author_comment,
        timestamp,
        job_name,
        job_time,
        software_id,
        software_version,
        key_color,
        pixel_aspect_ratio,
        gamma,
        colour_correction_offset,
        postage_stamp_offset,
        scan_line_offset,
        attributes_type,
    })
}

/// Detect + parse the 26-byte TGA 2.0 footer at the tail of `input`.
/// Returns `None` if the magic isn't present (TGA 1.0 file, or a
/// truncated TGA 2.0 file).
pub fn parse_footer(input: &[u8]) -> Option<TgaFooter> {
    if input.len() < TGA_FOOTER_SIZE {
        return None;
    }
    let off = input.len() - TGA_FOOTER_SIZE;
    let magic = &input[off + 8..off + 8 + TGA_FOOTER_MAGIC.len()];
    if magic != TGA_FOOTER_MAGIC.as_slice() {
        return None;
    }
    Some(TgaFooter {
        extension_area_offset: read_u32_le(input, off),
        developer_directory_offset: read_u32_le(input, off + 4),
    })
}

/// Number of entries per colour-correction table channel (spec §C.6.8).
///
/// The table holds 256 entries, one per possible 8-bit input value.
/// Each entry carries four `u16` correction values (one per channel) in
/// (A, R, G, B) order. Each value is in the 0..=65535 range,
/// representing a 16-bit corrected sample value for the 8-bit input
/// index — BLACK maps to 0, WHITE to 65535.
pub const TGA_COLOUR_CORRECTION_TABLE_ENTRIES: usize = 256;

/// Total size of the colour-correction table in bytes
/// (`256 × 4 × 2 = 2048`).
pub const TGA_COLOUR_CORRECTION_TABLE_SIZE: usize = 4 * TGA_COLOUR_CORRECTION_TABLE_ENTRIES * 2;

/// 256 × 4 × u16 colour-correction table (spec §C.6.8).
///
/// In memory the four channel curves are stored as separate planar
/// arrays (`alpha` / `red` / `green` / `blue`), each indexed by the
/// 8-bit input sample. On disk, however, spec §C.6.8 stores the table
/// **interleaved**: a flat 2048-byte block of `256 × 4` little-endian
/// `u16`s where each set of four contiguous values is the
/// `(A, R, G, B)` correction for one entry. [`Self::parse`] /
/// [`Self::to_bytes`] perform the interleave/de-interleave so the
/// planar in-memory shape stays convenient while the on-disk bytes
/// match the spec layout exactly. The table is referenced by
/// [`TgaExtensionArea::colour_correction_offset`]; when that field is
/// zero, no table is present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgaColourCorrectionTable {
    /// 256-entry curve for the alpha channel.
    pub alpha: [u16; TGA_COLOUR_CORRECTION_TABLE_ENTRIES],
    /// 256-entry curve for the red channel.
    pub red: [u16; TGA_COLOUR_CORRECTION_TABLE_ENTRIES],
    /// 256-entry curve for the green channel.
    pub green: [u16; TGA_COLOUR_CORRECTION_TABLE_ENTRIES],
    /// 256-entry curve for the blue channel.
    pub blue: [u16; TGA_COLOUR_CORRECTION_TABLE_ENTRIES],
}

impl Default for TgaColourCorrectionTable {
    fn default() -> Self {
        // Identity table: out[i] = i << 8 | i  (so 0x00 → 0x0000,
        // 0xFF → 0xFFFF). Callers that want a no-op CCT can use this.
        let mut chan = [0u16; TGA_COLOUR_CORRECTION_TABLE_ENTRIES];
        for (i, slot) in chan.iter_mut().enumerate() {
            *slot = ((i as u16) << 8) | (i as u16);
        }
        Self {
            alpha: chan,
            red: chan,
            green: chan,
            blue: chan,
        }
    }
}

impl TgaColourCorrectionTable {
    /// Parse a colour-correction table from `input` starting at byte
    /// offset `offset`. Returns `None` if `offset == 0` (the convention
    /// for "no table"), or if the input is truncated before the full
    /// `256 × 4 × 2 = 2048` bytes.
    ///
    /// The on-disk layout is interleaved per spec §C.6.8: each entry is
    /// four contiguous little-endian `u16`s in `(A, R, G, B)` order.
    pub fn parse(input: &[u8], offset: u32) -> Option<Self> {
        if offset == 0 {
            return None;
        }
        let off = offset as usize;
        if input.len() < off + TGA_COLOUR_CORRECTION_TABLE_SIZE {
            return None;
        }
        let mut alpha = [0u16; TGA_COLOUR_CORRECTION_TABLE_ENTRIES];
        let mut red = [0u16; TGA_COLOUR_CORRECTION_TABLE_ENTRIES];
        let mut green = [0u16; TGA_COLOUR_CORRECTION_TABLE_ENTRIES];
        let mut blue = [0u16; TGA_COLOUR_CORRECTION_TABLE_ENTRIES];
        for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
            let base = off + i * 8;
            alpha[i] = read_u16_le(input, base);
            red[i] = read_u16_le(input, base + 2);
            green[i] = read_u16_le(input, base + 4);
            blue[i] = read_u16_le(input, base + 6);
        }
        Some(Self {
            alpha,
            red,
            green,
            blue,
        })
    }

    /// Serialise the table into its on-disk 2048-byte representation:
    /// 256 entries of four interleaved little-endian `u16`s in
    /// `(A, R, G, B)` order, per spec §C.6.8.
    pub fn to_bytes(&self) -> [u8; TGA_COLOUR_CORRECTION_TABLE_SIZE] {
        let mut out = [0u8; TGA_COLOUR_CORRECTION_TABLE_SIZE];
        for i in 0..TGA_COLOUR_CORRECTION_TABLE_ENTRIES {
            let base = i * 8;
            out[base..base + 2].copy_from_slice(&self.alpha[i].to_le_bytes());
            out[base + 2..base + 4].copy_from_slice(&self.red[i].to_le_bytes());
            out[base + 4..base + 6].copy_from_slice(&self.green[i].to_le_bytes());
            out[base + 6..base + 8].copy_from_slice(&self.blue[i].to_le_bytes());
        }
        out
    }

    /// Apply the correction curves to a single 8-bit RGBA pixel,
    /// producing a full-precision 16-bit `[R, G, B, A]` corrected pixel
    /// (spec §C.6.8 — the table maps each 8-bit input sample to the
    /// stored 16-bit correction).
    ///
    /// The input is `[R, G, B, A]` (the in-memory order of
    /// [`crate::TgaImage`] data); the output keeps that channel order
    /// while each component is looked up through its own curve.
    pub fn correct_rgba16(&self, rgba: [u8; 4]) -> [u16; 4] {
        [
            self.red[rgba[0] as usize],
            self.green[rgba[1] as usize],
            self.blue[rgba[2] as usize],
            self.alpha[rgba[3] as usize],
        ]
    }

    /// Apply the correction curves to a single 8-bit RGBA pixel,
    /// returning an 8-bit `[R, G, B, A]` corrected pixel.
    ///
    /// Each 16-bit corrected value from [`Self::correct_rgba16`] is
    /// narrowed to 8 bits by taking the high byte (`>> 8`). This is the
    /// inverse of the identity curve's `out = (in << 8) | in` mapping:
    /// a default (identity) table is a perfect no-op at 8-bit precision.
    pub fn correct_rgba8(&self, rgba: [u8; 4]) -> [u8; 4] {
        let c = self.correct_rgba16(rgba);
        [
            (c[0] >> 8) as u8,
            (c[1] >> 8) as u8,
            (c[2] >> 8) as u8,
            (c[3] >> 8) as u8,
        ]
    }

    /// Apply the correction curves to a single 8-bit grayscale sample,
    /// returning the 8-bit corrected luma.
    ///
    /// Grayscale TGAs (image types 3 / 11) carry no per-channel data, so
    /// the green curve is used as the luminance curve. (The choice is
    /// arbitrary among the three colour curves for a neutral grey; green
    /// is conventional for luma weighting and matches what a writer that
    /// built a single neutral ramp would store across R/G/B.)
    pub fn correct_gray8(&self, luma: u8) -> u8 {
        (self.green[luma as usize] >> 8) as u8
    }

    /// Apply the correction table in place to a decoded
    /// [`crate::TgaImage`].
    ///
    /// * [`crate::TgaPixelFormat::Rgba`] images have every pixel mapped
    ///   through [`Self::correct_rgba8`].
    /// * [`crate::TgaPixelFormat::Gray8`] images have every sample mapped
    ///   through [`Self::correct_gray8`].
    /// * [`crate::TgaPixelFormat::Rgb24`] images map R/G/B through their
    ///   curves (no alpha channel present).
    ///
    /// The image's dimensions and pixel format are unchanged; only the
    /// sample values are rewritten. A default (identity) table leaves an
    /// 8-bit image bit-for-bit unchanged.
    pub fn apply_to_image(&self, image: &mut crate::image::TgaImage) {
        match image.pixel_format {
            crate::image::TgaPixelFormat::Rgba => {
                for px in image.data.chunks_exact_mut(4) {
                    let out = self.correct_rgba8([px[0], px[1], px[2], px[3]]);
                    px.copy_from_slice(&out);
                }
            }
            crate::image::TgaPixelFormat::Rgb24 => {
                for px in image.data.chunks_exact_mut(3) {
                    px[0] = (self.red[px[0] as usize] >> 8) as u8;
                    px[1] = (self.green[px[1] as usize] >> 8) as u8;
                    px[2] = (self.blue[px[2] as usize] >> 8) as u8;
                }
            }
            crate::image::TgaPixelFormat::Gray8 => {
                for b in image.data.iter_mut() {
                    *b = self.correct_gray8(*b);
                }
            }
        }
    }
}

/// On-disk size of one entry in the §C.6.9 scan-line table — a single
/// little-endian `u32` byte offset per pixel row.
pub const TGA_SCAN_LINE_OFFSET_BYTES: usize = 4;

/// Per-row byte offsets parsed out of the scan-line table (spec §C.6.9).
///
/// On disk the table is a flat array of `height` little-endian u32
/// values, each holding the byte offset (from the start of the file)
/// of the corresponding pixel row. The table is intended for partial-
/// image readers that want to jump to a single row without walking the
/// preceding RLE-packetised rows.
///
/// The number of entries equals the image height; the caller is
/// expected to pass the parent header's `height` so we can size the
/// returned Vec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgaScanLineTable {
    /// `height` entries. `offsets[y]` is the byte offset of row `y`
    /// from the start of the file.
    pub offsets: Vec<u32>,
}

impl Default for TgaScanLineTable {
    /// Empty-table default (zero scan lines, no offsets recorded).
    fn default() -> Self {
        Self::EMPTY
    }
}

impl TgaScanLineTable {
    /// Empty-table sentinel (`offsets.len() == 0`).
    ///
    /// Matches [`Self::is_unset`] and is the spec's "no scan-line table
    /// present" shape; the §C.6.9 scan-line *offset* in the extension
    /// area being zero is the analogous file-level signal, and the
    /// in-memory empty table is what one would build from such a file.
    pub const EMPTY: Self = Self {
        offsets: Vec::new(),
    };

    /// Construct a table from a pre-built offsets vector.
    pub fn new(offsets: Vec<u32>) -> Self {
        Self { offsets }
    }

    /// Allocate an empty table sized for a `height`-row image. The
    /// returned table has `len() == 0` and capacity `height as usize`;
    /// callers populate it via `offsets.push`.
    pub fn with_capacity(height: u16) -> Self {
        Self {
            offsets: Vec::with_capacity(height as usize),
        }
    }

    /// Number of offsets in the table (matches the parent header's
    /// `height` for any well-formed file).
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    /// `true` when the table holds no offsets.
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// `true` when the table is the empty/marker shape — same as
    /// [`Self::is_empty`], named to match the typed-accessor convention
    /// used by the other extension-area surfaces.
    pub fn is_unset(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Bounds-checked accessor: returns the byte offset of row `y`, or
    /// `None` when `y >= len()`.
    pub fn get(&self, y: usize) -> Option<u32> {
        self.offsets.get(y).copied()
    }

    /// Total serialised size in bytes (`len() × 4`).
    pub fn byte_size(&self) -> usize {
        self.offsets.len() * TGA_SCAN_LINE_OFFSET_BYTES
    }

    /// Sanity check: every recorded offset must fit inside a buffer of
    /// `input_len` bytes (strictly less than `input_len`, since the
    /// offset points *to* a byte). Returns `true` for the empty table.
    ///
    /// This is a structural check only — it does not verify that the
    /// offsets point to actual row boundaries inside the image-data
    /// section, only that they are addressable.
    pub fn is_well_formed_within(&self, input_len: usize) -> bool {
        self.offsets.iter().all(|&o| (o as usize) < input_len)
    }

    /// `true` when consecutive offsets are strictly increasing — the
    /// shape produced by a top-down save where row `y+1` always lies
    /// after row `y` in the file. The empty table and any single-entry
    /// table are trivially strictly increasing.
    pub fn is_strictly_increasing(&self) -> bool {
        self.offsets.windows(2).all(|w| w[0] < w[1])
    }

    /// `true` when consecutive offsets are strictly decreasing — the
    /// shape produced by a bottom-up save where row `y+1` lies *before*
    /// row `y` in the file. The empty table and any single-entry
    /// table are trivially strictly decreasing.
    pub fn is_strictly_decreasing(&self) -> bool {
        self.offsets.windows(2).all(|w| w[0] > w[1])
    }

    /// Derive the `[start, end)` byte range covered by row `y` inside
    /// the file, given the byte offset `terminal` at which the row
    /// data ends (typically the start of the next section — extension
    /// area, postage stamp, colour-correction table, scan-line table
    /// itself, or the file footer). The range's `end` is the next row's
    /// recorded offset when `y + 1 < len()`, otherwise `terminal`.
    ///
    /// Returns `None` when `y >= len()`, when the row's recorded offset
    /// is greater than `terminal`, or when the next row's offset would
    /// move backwards relative to the row's start (a row's end must lie
    /// at or after its start so the range is well-defined).
    pub fn row_range(&self, y: usize, terminal: u32) -> Option<(u32, u32)> {
        let start = self.offsets.get(y).copied()?;
        let end = if y + 1 < self.offsets.len() {
            self.offsets[y + 1]
        } else {
            terminal
        };
        if end < start {
            return None;
        }
        Some((start, end))
    }

    /// Borrow the bytes of row `y` from `input` using
    /// [`Self::row_range`] for the bounds. Returns `None` if the row
    /// range is undefined or if the resolved `[start, end)` is not
    /// fully inside `input`.
    pub fn row_bytes<'a>(&self, input: &'a [u8], y: usize, terminal: u32) -> Option<&'a [u8]> {
        let (start, end) = self.row_range(y, terminal)?;
        let s = start as usize;
        let e = end as usize;
        if e > input.len() {
            return None;
        }
        Some(&input[s..e])
    }

    /// Parse a scan-line table from `input` starting at byte offset
    /// `offset`. Returns `None` if `offset == 0`, if `height == 0`, or
    /// if the input is truncated before `height × 4` bytes.
    pub fn parse(input: &[u8], offset: u32, height: u16) -> Option<Self> {
        if offset == 0 || height == 0 {
            return None;
        }
        let off = offset as usize;
        let n = height as usize;
        if input.len() < off + n * TGA_SCAN_LINE_OFFSET_BYTES {
            return None;
        }
        let mut offsets = Vec::with_capacity(n);
        for i in 0..n {
            offsets.push(read_u32_le(input, off + i * TGA_SCAN_LINE_OFFSET_BYTES));
        }
        Some(Self { offsets })
    }

    /// Serialise the table into its on-disk `4 × len()`-byte
    /// representation (little-endian u32s, one per row).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.byte_size());
        for &o in &self.offsets {
            out.extend_from_slice(&o.to_le_bytes());
        }
        out
    }
}

impl FromIterator<u32> for TgaScanLineTable {
    fn from_iter<I: IntoIterator<Item = u32>>(iter: I) -> Self {
        Self {
            offsets: iter.into_iter().collect(),
        }
    }
}

/// On-disk size of one developer-directory record — spec §C.7
/// ("each set is 10 bytes in size (1 short, 2 longs)"): a little-endian
/// `u16` tag id, a little-endian `u32` payload offset, and a
/// little-endian `u32` payload size.
pub const TGA_DEVELOPER_TAG_BYTES: usize = 10;

/// On-disk size of the developer-directory header — the leading SHORT
/// holding the number of tags. Spec §C.7's directory-size formula
/// `(NUMBER_OF_TAGS_IN_THE_DIRECTORY * 10) + 2` notes "the '+ 2'
/// includes the 2 bytes for the SHORT value specifying the number of
/// tags in the directory".
pub const TGA_DEVELOPER_DIRECTORY_HEADER_BYTES: usize = 2;

/// One entry from the developer area's tag directory (spec §C.7).
///
/// The developer area is the writer-extensible scratch space TGA 2.0
/// provides for application-defined tagged data. The directory at
/// [`TgaFooter::developer_directory_offset`] is a sequence of
/// `(tag_id, offset, size)` records that point into application-defined
/// byte ranges elsewhere in the file.
///
/// Per spec §C.7, each TAG is a SHORT in the range 0 to 65535: values
/// `0..=32767` are available for developer use, while `32768..=65535`
/// are reserved for Truevision. The parser stores all tags regardless;
/// semantic interpretation is left to the caller (spec §C.7 declines
/// to enumerate well-known tag IDs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TgaDeveloperTag {
    /// Tag identifier (`0..=32767` developer use, `32768..=65535`
    /// Truevision-reserved per spec §C.7).
    pub tag_id: u16,
    /// Byte offset of the tag's payload from the start of the file.
    pub offset: u32,
    /// Length of the tag's payload in bytes.
    pub size: u32,
}

impl TgaDeveloperTag {
    /// Construct a directory record from its three on-disk fields.
    pub fn new(tag_id: u16, offset: u32, size: u32) -> Self {
        Self {
            tag_id,
            offset,
            size,
        }
    }

    /// The record as a `(tag_id, offset, size)` tuple in on-disk field
    /// order (spec §C.7: TAG, then OFFSET, then FIELD SIZE).
    pub fn as_tuple(&self) -> (u16, u32, u32) {
        (self.tag_id, self.offset, self.size)
    }

    /// Build a record from a `(tag_id, offset, size)` tuple in on-disk
    /// field order. Inverse of [`Self::as_tuple`].
    pub fn from_tuple(t: (u16, u32, u32)) -> Self {
        Self {
            tag_id: t.0,
            offset: t.1,
            size: t.2,
        }
    }

    /// `true` when the tag id falls in the developer-use range
    /// (`0..=32767` per spec §C.7: "Values from 0 - 32767 are
    /// available for developer use").
    pub fn is_developer_use(&self) -> bool {
        self.tag_id <= 32767
    }

    /// `true` when the tag id falls in the Truevision-reserved range
    /// (`32768..=65535` per spec §C.7: "values from 32768 - 65535 are
    /// reserved for Truevision").
    pub fn is_truevision_reserved(&self) -> bool {
        self.tag_id >= 32768
    }

    /// `true` for a marker record — a directory entry whose payload
    /// `offset` is 0, i.e. the tag is present in the directory but
    /// carries no payload bytes. This is the shape
    /// [`crate::DeveloperTagInput`] writes for an empty payload, and
    /// the shape for which [`TgaDeveloperArea::payload`] returns
    /// `None`.
    pub fn is_marker(&self) -> bool {
        self.offset == 0
    }

    /// Sanity check: a marker record is trivially well-formed; a
    /// non-marker record's `[offset, offset + size)` payload range
    /// must fit inside a buffer of `input_len` bytes. Mirrors the
    /// bound [`TgaDeveloperArea::parse`] enforces per record.
    pub fn is_well_formed_within(&self, input_len: usize) -> bool {
        if self.offset == 0 {
            return true;
        }
        (self.offset as usize).saturating_add(self.size as usize) <= input_len
    }

    /// Serialise the record into its on-disk 10-byte form (spec §C.7
    /// Figure 2 - Developer Directory): little-endian `u16` tag id,
    /// little-endian `u32` offset, little-endian `u32` size.
    pub fn to_bytes(&self) -> [u8; TGA_DEVELOPER_TAG_BYTES] {
        let mut out = [0u8; TGA_DEVELOPER_TAG_BYTES];
        out[0..2].copy_from_slice(&self.tag_id.to_le_bytes());
        out[2..6].copy_from_slice(&self.offset.to_le_bytes());
        out[6..10].copy_from_slice(&self.size.to_le_bytes());
        out
    }
}

/// Parsed developer area: the tag directory plus a borrowed view of
/// every tag's payload bytes.
///
/// The owned `tags` Vec holds the directory entries; the
/// [`TgaDeveloperArea::payload`] method returns the slice of the
/// original input the tag points into. This is by design: the payload
/// format is application-defined, so the parser doesn't materialise
/// any further structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgaDeveloperArea {
    /// Directory entries in on-disk order.
    pub tags: Vec<TgaDeveloperTag>,
}

impl Default for TgaDeveloperArea {
    /// Empty-directory default (zero tags recorded).
    fn default() -> Self {
        Self::EMPTY
    }
}

impl FromIterator<TgaDeveloperTag> for TgaDeveloperArea {
    fn from_iter<I: IntoIterator<Item = TgaDeveloperTag>>(iter: I) -> Self {
        Self {
            tags: iter.into_iter().collect(),
        }
    }
}

impl TgaDeveloperArea {
    /// Empty-directory sentinel (`tags.len() == 0`).
    ///
    /// Matches [`Self::is_unset`] and is the in-memory shape for a file
    /// with no developer area; the file-level signal is the footer's
    /// developer-directory offset being zero (spec §C.7: "If the Offset
    /// is ZERO (binary zero) no directory and no Developer Area fields
    /// exist").
    pub const EMPTY: Self = Self { tags: Vec::new() };

    /// Construct a directory from a pre-built tag vector.
    pub fn new(tags: Vec<TgaDeveloperTag>) -> Self {
        Self { tags }
    }

    /// Number of records in the tag directory (the leading SHORT of
    /// the on-disk directory).
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// `true` when the directory holds no records.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// `true` when the directory is the empty shape — same as
    /// [`Self::is_empty`], named to match the typed-accessor convention
    /// used by the other TGA 2.0 surfaces.
    pub fn is_unset(&self) -> bool {
        self.tags.is_empty()
    }

    /// Total serialised size of the tag directory in bytes — spec
    /// §C.7's `(NUMBER_OF_TAGS_IN_THE_DIRECTORY * 10) + 2` formula.
    /// Matches `to_bytes().len()`.
    pub fn directory_byte_size(&self) -> usize {
        TGA_DEVELOPER_DIRECTORY_HEADER_BYTES + self.tags.len() * TGA_DEVELOPER_TAG_BYTES
    }

    /// Bounds-checked accessor: returns the directory record at
    /// on-disk position `i`, or `None` when `i >= len()`.
    pub fn get(&self, i: usize) -> Option<&TgaDeveloperTag> {
        self.tags.get(i)
    }

    /// Find the first record carrying `tag_id`, in on-disk order.
    /// Spec §C.7 permits any directory ordering ("The TAGS may appear
    /// in any order in the directory (i.e., they do not need to be
    /// sorted)"), so a lookup by id rather than position is the
    /// caller-facing primitive.
    pub fn find(&self, tag_id: u16) -> Option<&TgaDeveloperTag> {
        self.tags.iter().find(|t| t.tag_id == tag_id)
    }

    /// `true` when at least one record carries `tag_id`.
    pub fn contains(&self, tag_id: u16) -> bool {
        self.tags.iter().any(|t| t.tag_id == tag_id)
    }

    /// Sanity check: every record satisfies
    /// [`TgaDeveloperTag::is_well_formed_within`] against a buffer of
    /// `input_len` bytes. Returns `true` for the empty directory.
    ///
    /// This is a structural check only — it does not interpret any
    /// payload, only that each non-marker payload range is addressable.
    pub fn is_well_formed_within(&self, input_len: usize) -> bool {
        self.tags.iter().all(|t| t.is_well_formed_within(input_len))
    }

    /// Serialise the tag directory into its on-disk form (spec §C.7
    /// Figure 2 - Developer Directory): a little-endian `u16` record
    /// count followed by one 10-byte [`TgaDeveloperTag::to_bytes`]
    /// record per tag, in directory order. Byte-for-byte the layout
    /// [`Self::parse`] reads (the payloads themselves live elsewhere
    /// in the file and are not part of the directory).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.directory_byte_size());
        out.extend_from_slice(&(self.tags.len() as u16).to_le_bytes());
        for tag in &self.tags {
            out.extend_from_slice(&tag.to_bytes());
        }
        out
    }

    /// Parse the developer-area tag directory referenced by
    /// [`TgaFooter::developer_directory_offset`].
    ///
    /// On disk the directory is `2 + n × 10` bytes: a u16 count
    /// followed by `n` × `(u16 tag, u32 offset, u32 size)` records.
    /// Returns `None` if `offset == 0`, the count word is truncated,
    /// the record array is truncated, or any record points outside
    /// the input buffer.
    pub fn parse(input: &[u8], offset: u32) -> Option<Self> {
        if offset == 0 {
            return None;
        }
        let off = offset as usize;
        if input.len() < off + 2 {
            return None;
        }
        let n = read_u16_le(input, off) as usize;
        let dir_bytes = 2 + n * 10;
        if input.len() < off + dir_bytes {
            return None;
        }
        let mut tags = Vec::with_capacity(n);
        for i in 0..n {
            let rec_off = off + 2 + i * 10;
            let tag_id = read_u16_le(input, rec_off);
            let tag_offset = read_u32_le(input, rec_off + 2);
            let tag_size = read_u32_le(input, rec_off + 6);
            // Reject tags whose payload doesn't fit in the file:
            // tag_offset can legitimately be 0 (caller's convention
            // for "tag-only marker"), but if non-zero it must point
            // inside `input` and the (offset + size) must fit too.
            if tag_offset != 0 {
                let to = tag_offset as usize;
                let ts = tag_size as usize;
                if to.saturating_add(ts) > input.len() {
                    return None;
                }
            }
            tags.push(TgaDeveloperTag {
                tag_id,
                offset: tag_offset,
                size: tag_size,
            });
        }
        Some(Self { tags })
    }

    /// Borrow the payload bytes for `tag` from the original `input`
    /// buffer. Returns `None` if the tag's offset is `0` (marker tag
    /// with no payload) or if `input` is shorter than the tag claims.
    pub fn payload<'a>(&self, input: &'a [u8], tag: &TgaDeveloperTag) -> Option<&'a [u8]> {
        if tag.offset == 0 {
            return None;
        }
        let off = tag.offset as usize;
        let sz = tag.size as usize;
        if off.saturating_add(sz) > input.len() {
            return None;
        }
        Some(&input[off..off + sz])
    }
}

/// Typed view of a TGA **Color Map** (the on-disk palette declared by the
/// header's Color Map Specification — bytes 3-7: Color Map Origin, Color
/// Map Length, Color Map Entry Size — per spec §C.2 / the Data Type 1
/// header layout).
///
/// A color map is present whenever the header's Color Map Type byte
/// (byte 1) is `1`, regardless of the Image Type Code. It is the palette
/// consumed when decoding colour-mapped image types (1 / 9), but the spec
/// also permits a **Data Type 0 (No Image Data)** file to carry only a
/// color map (and/or metadata) with no pixel array — a palette-only TGA.
/// [`crate::parse_tga`] returns the fully decoded raster for image types
/// 1-11; this lighter view exposes just the palette, so a caller can pull
/// the color map out of a type-0 file (which `parse_tga` rejects as having
/// no image) or inspect a colour-mapped file's palette directly.
///
/// Each entry is de-interleaved from its on-disk form (15/16-bit
/// A1R5G5B5, 24-bit BGR, or 32-bit BGRA) into normalised straight RGBA,
/// matching how [`crate::parse_tga`] expands the palette internally. The
/// [`Self::first_index`] field records the Color Map Origin: the on-disk
/// entry at slot `i` of [`Self::entries`] is logical palette index
/// `first_index + i`, so a pixel value `idx` addresses
/// `entries[idx - first_index]` (per spec §C.2; mirrors the decoder's
/// origin-aware lookup).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TgaColorMap {
    /// Color Map Origin (header bytes 3-4): the logical palette index of
    /// the first stored entry. Indices below this value are not stored.
    pub first_index: u16,
    /// Color Map Entry Size in bits (header byte 7): 15, 16, 24, or 32.
    pub entry_size: u8,
    /// The decoded palette entries in straight RGBA, in on-disk order
    /// (entry `i` is logical index `first_index + i`).
    pub entries: Vec<[u8; 4]>,
}

impl TgaColorMap {
    /// Sentinel for "no color map" (origin 0, entry size 0, no entries).
    pub const fn empty() -> Self {
        Self {
            first_index: 0,
            entry_size: 0,
            entries: Vec::new(),
        }
    }

    /// `true` when no entries are stored — the empty / absent map.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of stored palette entries (== the header's Color Map
    /// Length when the map is well-formed).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Look up a logical palette index, honouring the Color Map Origin
    /// (spec §C.2): index `idx` addresses stored entry
    /// `idx - first_index`. Returns `None` for an index below the origin
    /// or past the stored length, mirroring the decoder's bounds rule.
    pub fn get(&self, idx: usize) -> Option<[u8; 4]> {
        let local = idx.checked_sub(self.first_index as usize)?;
        self.entries.get(local).copied()
    }
}

#[inline]
pub fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

#[inline]
pub fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}
