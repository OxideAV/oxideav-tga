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

impl TgaExtensionArea {
    /// Typed view of [`Self::attributes_type`] per spec §C.6.13.
    /// Equivalent to `AttributesType::from_u8(self.attributes_type)`.
    pub fn attributes(&self) -> AttributesType {
        AttributesType::from_u8(self.attributes_type)
    }
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

/// Number of entries per colour-correction table channel (spec §C.6.8).
///
/// The table is `4 × 256 × u16` little-endian — one 256-entry curve per
/// channel in (A, R, G, B) order. Each entry is in the 0..=65535 range,
/// representing a 16-bit corrected sample value for the 8-bit input
/// index.
pub const TGA_COLOUR_CORRECTION_TABLE_ENTRIES: usize = 256;

/// Total size of the colour-correction table in bytes
/// (`4 × 256 × 2 = 2048`).
pub const TGA_COLOUR_CORRECTION_TABLE_SIZE: usize = 4 * TGA_COLOUR_CORRECTION_TABLE_ENTRIES * 2;

/// 4 × 256 × u16 colour-correction table (spec §C.6.8).
///
/// On disk the table is a flat `2048-byte` block of little-endian
/// u16s laid out as 4 contiguous 256-entry curves in (A, R, G, B)
/// channel order. The table is referenced by
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
    /// `4 × 256 × 2 = 2048` bytes.
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
        let mut cursor = off;
        for slot in &mut alpha {
            *slot = read_u16_le(input, cursor);
            cursor += 2;
        }
        for slot in &mut red {
            *slot = read_u16_le(input, cursor);
            cursor += 2;
        }
        for slot in &mut green {
            *slot = read_u16_le(input, cursor);
            cursor += 2;
        }
        for slot in &mut blue {
            *slot = read_u16_le(input, cursor);
            cursor += 2;
        }
        Some(Self {
            alpha,
            red,
            green,
            blue,
        })
    }

    /// Serialise the table into its on-disk 2048-byte representation
    /// (4 contiguous 256-entry curves of little-endian u16s in ARGB
    /// channel order).
    pub fn to_bytes(&self) -> [u8; TGA_COLOUR_CORRECTION_TABLE_SIZE] {
        let mut out = [0u8; TGA_COLOUR_CORRECTION_TABLE_SIZE];
        let mut off = 0usize;
        for channel in [&self.alpha, &self.red, &self.green, &self.blue] {
            for &v in channel {
                out[off..off + 2].copy_from_slice(&v.to_le_bytes());
                off += 2;
            }
        }
        out
    }
}

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

impl TgaScanLineTable {
    /// Parse a scan-line table from `input` starting at byte offset
    /// `offset`. Returns `None` if `offset == 0`, if `height == 0`, or
    /// if the input is truncated before `height × 4` bytes.
    pub fn parse(input: &[u8], offset: u32, height: u16) -> Option<Self> {
        if offset == 0 || height == 0 {
            return None;
        }
        let off = offset as usize;
        let n = height as usize;
        if input.len() < off + n * 4 {
            return None;
        }
        let mut offsets = Vec::with_capacity(n);
        for i in 0..n {
            offsets.push(read_u32_le(input, off + i * 4));
        }
        Some(Self { offsets })
    }

    /// Serialise the table into its on-disk `4 × len()`-byte
    /// representation (little-endian u32s, one per row).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.offsets.len() * 4);
        for &o in &self.offsets {
            out.extend_from_slice(&o.to_le_bytes());
        }
        out
    }
}

/// One entry from the developer area's tag directory (spec §C.7).
///
/// The developer area is the writer-extensible scratch space TGA 2.0
/// provides for application-defined tagged data. The directory at
/// [`TgaFooter::developer_directory_offset`] is a sequence of
/// `(tag_id, offset, size)` records that point into application-defined
/// byte ranges elsewhere in the file.
///
/// Tag IDs `0..=32767` are reserved for Truevision; `32768..=65535` are
/// available for user-defined fields. The parser stores all tags
/// regardless; semantic interpretation is left to the caller (spec
/// §C.7 declines to enumerate well-known tag IDs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TgaDeveloperTag {
    /// Tag identifier (Truevision-reserved < 32768, user > 32767).
    pub tag_id: u16,
    /// Byte offset of the tag's payload from the start of the file.
    pub offset: u32,
    /// Length of the tag's payload in bytes.
    pub size: u32,
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

impl TgaDeveloperArea {
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

#[inline]
pub fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

#[inline]
pub fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}
