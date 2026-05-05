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

/// Pointers parsed out of the optional 26-byte TGA 2.0 footer. Round 1
/// only surfaces the offsets; the extension area body parse (gamma,
/// software ID + version, postage-stamp thumbnail, colour-correction
/// table, …) is deferred to round 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TgaFooter {
    /// Byte offset of the extension area (0 if absent).
    pub extension_area_offset: u32,
    /// Byte offset of the developer directory (0 if absent).
    pub developer_directory_offset: u32,
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
