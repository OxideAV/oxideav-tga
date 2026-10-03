//! Crate-local error type used by `oxideav-tga`'s standalone (no
//! `oxideav-core`) public API.
//!
//! When the `registry` feature is enabled, [`TgaError`] gains a
//! `From<TgaError> for oxideav_core::Error` impl (defined in
//! `crate::registry`) so the trait-side surface (`Decoder` /
//! `Encoder`) can keep returning `oxideav_core::Result<T>` while the
//! underlying decode/encode functions stay framework-free.

use core::fmt;

/// `Result` alias scoped to `oxideav-tga`. Standalone (no
/// `oxideav-core`) callers see this; framework callers convert via the
/// gated `From<TgaError> for oxideav_core::Error` impl.
pub type Result<T> = core::result::Result<T, TgaError>;

/// The contract name for [`TgaError`].
pub type Error = TgaError;

/// Error variants returned by `oxideav-tga`'s standalone API.
///
/// The variants mirror the subset of `oxideav_core::Error` the codec
/// can hit plus the two the image-crate contract requires
/// (`LimitExceeded`, `Io`). Framework-specific errors
/// (`FormatNotFound`, `CodecNotFound`) originate in callers that are
/// already linking `oxideav-core`.
#[derive(Debug)]
#[non_exhaustive]
pub enum TgaError {
    /// The byte stream is malformed (truncated header, RLE packet runs
    /// past end of pixel data, palette entry size doesn't divide into
    /// the colour-map length, …), or a caller-assembled image is
    /// inconsistent (plane too short, palette index out of range).
    InvalidData(String),
    /// The byte stream uses a feature this codec doesn't implement
    /// (an image type outside the 1/2/3/9/10/11 set, an unsupported
    /// pixel depth, …), or the encoder was asked for something the
    /// format cannot represent (more than 256 colours into a colour
    /// map, a 15 / 16-bit true-colour write).
    Unsupported(String),
    /// A [`crate::DecodeOptions`] limit (dimensions / pixels / bytes)
    /// would be exceeded; nothing was allocated.
    LimitExceeded(String),
    /// A read / write on a caller-supplied stream failed
    /// ([`crate::decode_from`] / [`crate::encode_to`]).
    Io(std::io::Error),
}

impl TgaError {
    /// Construct a [`TgaError::InvalidData`] from a stringy message.
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidData(msg.into())
    }

    /// Construct a [`TgaError::Unsupported`] from a stringy message.
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::Unsupported(msg.into())
    }

    /// Construct a [`TgaError::LimitExceeded`] from a stringy message.
    pub fn limit(msg: impl Into<String>) -> Self {
        Self::LimitExceeded(msg.into())
    }
}

impl From<std::io::Error> for TgaError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl fmt::Display for TgaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(s) => write!(f, "invalid data: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::LimitExceeded(s) => write!(f, "limit exceeded: {s}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for TgaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
