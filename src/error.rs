//! Crate-local error type used by `oxideav-tga`'s standalone (no
//! `oxideav-core`) public API.
//!
//! When the `registry` feature is enabled, [`TgaError`] gains a
//! `From<TgaError> for oxideav_core::Error` impl (defined in
//! [`crate::registry`]) so the trait-side surface (`Decoder` /
//! `Encoder`) can keep returning `oxideav_core::Result<T>` while the
//! underlying decode/encode functions stay framework-free.

use core::fmt;

/// `Result` alias scoped to `oxideav-tga`. Standalone (no
/// `oxideav-core`) callers see this; framework callers convert via the
/// gated `From<TgaError> for oxideav_core::Error` impl.
pub type Result<T> = core::result::Result<T, TgaError>;

/// Error variants returned by `oxideav-tga`'s standalone API.
///
/// The variants mirror the subset of `oxideav_core::Error` the codec
/// can hit. The crate intentionally avoids surfacing transport (`Io`)
/// or framework-specific (`FormatNotFound`, `CodecNotFound`) errors —
/// those originate in callers that are already linking `oxideav-core`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TgaError {
    /// The byte stream is malformed (truncated header, RLE packet runs
    /// past end of pixel data, palette entry size doesn't divide into
    /// the colour-map length, …).
    InvalidData(String),
    /// The byte stream uses a feature this codec doesn't implement
    /// (an image type outside the 1/2/3/9/10/11 set, an unsupported
    /// pixel depth, an alpha-channel bit count we don't handle, …).
    Unsupported(String),
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
}

impl fmt::Display for TgaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(s) => write!(f, "invalid data: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl std::error::Error for TgaError {}
