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
//! * Bit 4: reserved (must be 0).
//! * Bit 5: pixel ordering — 0 = bottom-to-top, 1 = top-to-bottom. (The
//!   spec also describes bit 4 as "right-to-left" but no real-world
//!   encoder ever sets it; we honour bit 5 only.)
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
    /// pixel columns are in right-to-left order. Real-world TGA files
    /// never set this; the decoder rejects it as `Unsupported` rather
    /// than silently producing mirrored output.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TgaFooter {
    /// Byte offset of the extension area (0 if absent).
    pub extension_area_offset: u32,
    /// Byte offset of the developer directory (0 if absent).
    pub developer_directory_offset: u32,
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

/// Date/time stamp embedded in the extension area's "save date" field
/// (spec §C.6.4). All zeros means "not set".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TgaTimestamp {
    pub month: u16,
    pub day: u16,
    pub year: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

impl TgaTimestamp {
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

#[inline]
pub fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

#[inline]
pub fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}
