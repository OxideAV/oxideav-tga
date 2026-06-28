//! Composed, spec-ordered TGA metadata-application pipeline.
//!
//! [`crate::parse_tga`] is deliberately a *raw* decoder: it expands the
//! on-disk pixel array (palette lookup, BGR→RGBA swap, 5-5-5 expansion,
//! RLE), normalises the storage order to top-left, and stops. It applies
//! **none** of the file's own metadata — gamma, colour correction, the
//! attributes-type alpha interpretation, the key colour, or the pixel
//! aspect ratio. That keeps the raw decode contract pure: a caller that
//! wants the unmodified samples (e.g. to re-encode, or to drive its own
//! colour pipeline) gets exactly the bytes that were stored.
//!
//! This module adds the *opposite* convenience: [`decode_tga_for_display`]
//! decodes a TGA and then applies the file's own metadata, in the order
//! the spec dictates, to produce a frame that is ready to put on screen.
//! Each individual apply pass already exists as a standalone helper
//! ([`crate::AttributesType::apply_to_image`],
//! [`crate::GammaValue::apply_to_image`],
//! [`crate::TgaColourCorrectionTable::apply_to_image`],
//! [`crate::KeyColor::key_out_image`],
//! [`crate::PixelAspectRatio::apply_to_image`],
//! [`crate::resolve_alpha_from_descriptor`]); this module is the
//! spec-ordered *composition* of them, plus the typed
//! [`TgaDisplayOptions`] that gates each pass.
//!
//! # The application order (spec §C.6)
//!
//! The Truevision v2 spec (`docs/image/tga/tgaffs.pdf`) does not state a
//! single canonical pipeline, but the field descriptions constrain the
//! order. The pipeline this module runs, and the spec text behind each
//! step, is:
//!
//! 1. **Alpha resolution** — settle what the alpha channel *means* before
//!    any colour maths reads it. The spec ties the header
//!    Image-Descriptor attribute-bit count (Field 5.6 bits 3-0) to the
//!    extension-area Attributes Type (Field 24): a Field-24 value of `0`
//!    "(bits 3-0 of field 5.6 should also be set to zero)". Field 24 is
//!    the richer, authoritative source ("specifies the type of Alpha
//!    channel data contained in the file"), so when a TGA 2.0 extension
//!    area is present its Attributes Type wins; otherwise we fall back to
//!    the header attribute-bit count, which is present in every file
//!    including TGA 1.0. This is the
//!    [`resolve_alpha_with_targa32_fallback`](crate::resolve_alpha_with_targa32_fallback)
//!    / [`resolve_alpha_from_descriptor`](crate::resolve_alpha_from_descriptor)
//!    split, composed. Pre-multiplied alpha (Field 24 = 4) is
//!    un-multiplied here, recovering straight colour, **before** the tone
//!    curves run — the un-multiply is the inverse of a *linear* scale by
//!    alpha, so it belongs on the stored (pre-curve) samples, matching the
//!    spec's Porter-Duff straight/pre-multiplied framing.
//!
//! 2. **Tone reproduction** — colour correction (Field 27) *or* gamma
//!    (Field 20), never both. The spec makes them mutually exclusive
//!    alternatives: Field 21 (Color Correction Offset) says to set the
//!    offset to zero "if the image has no Color Correction Table **or if
//!    the Gamma Value setting is sufficient**." A file therefore carries a
//!    correction table when a single gamma exponent is *insufficient* and
//!    gamma otherwise. When a colour-correction table is present this
//!    pipeline applies it and skips gamma; when only a gamma value is
//!    present it applies gamma. Applying both would double-correct.
//!
//! 3. **Key colour** (Field 18) — the "background / transparent colour …
//!    the same color that the screen would be cleared to if erased." It is
//!    a compositing decision made against the *final displayed* colour, so
//!    it runs after the tone curves (a pixel keyed by its corrected colour,
//!    not its raw stored colour).
//!
//! 4. **Pixel aspect ratio** (Field 19) — a *geometry* change (resample to
//!    square display pixels). It must run last: every per-pixel colour
//!    operation above addresses the stored raster, so resampling has to
//!    come after them or the colour passes would be operating on
//!    already-stretched data.
//!
//! Every pass is independently gated by [`TgaDisplayOptions`]; the default
//! options run all four in the order above, which is the spec-faithful
//! "display this file" behaviour.

use crate::decoder::{
    parse_tga, parse_tga_colour_correction_table, parse_tga_developer_area,
    parse_tga_extension_area, parse_tga_gamma, parse_tga_image_origin, parse_tga_key_color,
    parse_tga_pixel_aspect_ratio, parse_tga_postage_stamp, resolve_alpha_from_descriptor,
    resolve_alpha_with_targa32_fallback,
};
use crate::error::Result;
use crate::image::TgaImage;
use crate::types::{
    AttributeBits, AttributesType, GammaValue, ImageOrigin, PixelAspectRatio, TgaDeveloperArea,
};

/// Which spec §C.6 metadata passes [`decode_tga_for_display`] should run
/// after the raw decode, and how.
///
/// [`Default`] enables every pass, reproducing the spec-faithful "put this
/// file on screen" behaviour. Disable individual passes for finer control
/// — e.g. a renderer that does its own gamma management would clear
/// [`Self::apply_tone`] while keeping [`Self::resolve_alpha`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TgaDisplayOptions {
    /// Pass 1 — resolve the alpha channel's meaning (Field 24 attributes
    /// type, else Field 5.6 header attribute bits). Un-premultiplies
    /// pre-multiplied alpha and forces opaque files opaque. See
    /// [`AlphaResolution`] for the precedence rule.
    pub resolve_alpha: bool,
    /// Pass 2 — apply the tone-reproduction curve: the §C.6.8 colour
    /// correction table if present, otherwise the §C.6.6 gamma value.
    /// Never both (the spec treats them as alternatives).
    pub apply_tone: bool,
    /// Pass 3 — chroma-key the §C.6.4 key colour (set matching pixels'
    /// alpha to zero). No-op on alpha-less pixel formats.
    pub apply_key_color: bool,
    /// Pass 4 — resample to square display pixels per the §C.6.5 pixel
    /// aspect ratio. Changes the frame geometry. Runs last.
    pub apply_pixel_aspect: bool,
}

impl Default for TgaDisplayOptions {
    fn default() -> Self {
        Self::ALL
    }
}

impl TgaDisplayOptions {
    /// Every pass enabled — the spec-faithful display default.
    pub const ALL: Self = Self {
        resolve_alpha: true,
        apply_tone: true,
        apply_key_color: true,
        apply_pixel_aspect: true,
    };

    /// Every pass disabled. [`decode_tga_for_display`] with this is
    /// byte-identical to [`parse_tga`] — useful as a builder base.
    pub const NONE: Self = Self {
        resolve_alpha: false,
        apply_tone: false,
        apply_key_color: false,
        apply_pixel_aspect: false,
    };

    /// Builder: set [`Self::resolve_alpha`].
    pub fn with_resolve_alpha(mut self, on: bool) -> Self {
        self.resolve_alpha = on;
        self
    }

    /// Builder: set [`Self::apply_tone`].
    pub fn with_apply_tone(mut self, on: bool) -> Self {
        self.apply_tone = on;
        self
    }

    /// Builder: set [`Self::apply_key_color`].
    pub fn with_apply_key_color(mut self, on: bool) -> Self {
        self.apply_key_color = on;
        self
    }

    /// Builder: set [`Self::apply_pixel_aspect`].
    pub fn with_apply_pixel_aspect(mut self, on: bool) -> Self {
        self.apply_pixel_aspect = on;
        self
    }

    /// `true` when no pass is enabled (the [`parse_tga`]-equivalent
    /// configuration).
    pub fn is_passthrough(self) -> bool {
        self == Self::NONE
    }
}

/// Which tone-reproduction curve [`decode_tga_for_display`] applied, if
/// any. Recorded on [`TgaDisplayReport`] so a caller can tell whether the
/// frame went through the colour-correction table, the gamma exponent, or
/// neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneApplied {
    /// No tone pass ran — the file carried neither a usable colour
    /// correction table nor a usable gamma value (or the pass was gated
    /// off).
    None,
    /// The §C.6.8 colour-correction table was applied (and gamma, if any,
    /// was deliberately skipped — the two are spec alternatives).
    ColourCorrection,
    /// The §C.6.6 gamma value was applied (no colour-correction table was
    /// present).
    Gamma(GammaValue),
}

/// How the alpha channel's meaning was resolved in pass 1. Mirrors the two
/// branches of [`resolve_alpha_with_targa32_fallback`] plus the header-only
/// fallback used when there is no TGA 2.0 extension area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaResolution {
    /// The pass was gated off ([`TgaDisplayOptions::resolve_alpha`] clear).
    Skipped,
    /// A TGA 2.0 extension area was present; its §C.6.13 Attributes Type
    /// drove the resolution.
    AttributesType(AttributesType),
    /// No extension area; the header §C.2 Field-5.6 attribute-bit count
    /// drove the resolution (TGA-1.0-compatible path), possibly with the
    /// all-alpha-zero "treat as opaque" fallback.
    HeaderBits(AttributeBits),
}

/// What [`decode_tga_for_display_reported`] did to the decoded frame.
///
/// Lets a caller audit the pipeline: which alpha rule fired, which tone
/// curve (if any) ran, how many pixels the key colour removed, and whether
/// the frame was resampled for pixel aspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TgaDisplayReport {
    /// Outcome of pass 1.
    pub alpha: AlphaResolution,
    /// Outcome of pass 2.
    pub tone: ToneApplied,
    /// Number of pixels pass 3 keyed out (`0` if the pass was gated off,
    /// the file declared no key colour, or the format carries no alpha).
    pub keyed_pixels: u32,
    /// `true` when pass 4 resampled the frame to a new geometry.
    pub resampled: bool,
}

impl TgaDisplayReport {
    /// `true` when the tone-reproduction pass (pass 2) changed any sample.
    pub fn applied_tone(&self) -> bool {
        !matches!(self.tone, ToneApplied::None)
    }

    /// `true` when the key-colour pass (pass 3) removed at least one pixel.
    pub fn applied_key_color(&self) -> bool {
        self.keyed_pixels > 0
    }

    /// `true` when **no** pass beyond alpha resolution altered the frame —
    /// no tone curve ran, no pixel was keyed out, and the geometry was not
    /// resampled. Alpha resolution is excluded because it can silently
    /// force a file opaque even for an otherwise-untouched frame; query
    /// [`Self::alpha`] directly to learn what it did.
    pub fn is_colour_geometry_noop(&self) -> bool {
        !self.applied_tone() && !self.applied_key_color() && !self.resampled
    }
}

/// Decode a TGA and apply the file's own §C.6 metadata in spec order to
/// produce a display-ready frame.
///
/// This is the composed counterpart to [`parse_tga`]: `parse_tga` returns
/// the raw decoded samples, this returns the frame after the metadata
/// passes selected by `options` have run (see the module docs for the
/// order + spec rationale).
///
/// With [`TgaDisplayOptions::NONE`] the result is byte-identical to
/// [`parse_tga`]. With [`TgaDisplayOptions::default`] every pass runs, in
/// the spec-faithful order.
///
/// Returns the same errors as [`parse_tga`] (the raw decode is the only
/// fallible step; all metadata passes are infallible no-ops when the
/// relevant field is absent or malformed).
pub fn decode_tga_for_display(input: &[u8], options: &TgaDisplayOptions) -> Result<TgaImage> {
    decode_tga_for_display_reported(input, options).map(|(img, _)| img)
}

/// [`decode_tga_for_display`] plus a [`TgaDisplayReport`] describing what
/// each pass did.
pub fn decode_tga_for_display_reported(
    input: &[u8],
    options: &TgaDisplayOptions,
) -> Result<(TgaImage, TgaDisplayReport)> {
    let mut image = parse_tga(input)?;
    let mut report = TgaDisplayReport {
        alpha: AlphaResolution::Skipped,
        tone: ToneApplied::None,
        keyed_pixels: 0,
        resampled: false,
    };

    // -- Pass 1: alpha resolution -----------------------------------------
    if options.resolve_alpha {
        report.alpha = resolve_alpha(input, &mut image);
    }

    // -- Pass 2: tone reproduction (colour correction OR gamma) -----------
    if options.apply_tone {
        report.tone = apply_tone(input, &mut image);
    }

    // -- Pass 3: key colour ------------------------------------------------
    if options.apply_key_color {
        if let Some(key) = parse_tga_key_color(input) {
            if !key.is_unset() {
                report.keyed_pixels = key.key_out_image(&mut image);
            }
        }
    }

    // -- Pass 4: pixel aspect (geometry; last) ----------------------------
    if options.apply_pixel_aspect {
        let par = parse_tga_pixel_aspect_ratio(input).unwrap_or(PixelAspectRatio::UNSET);
        if let Some(resampled) = par.resampled(&image) {
            image = resampled;
            report.resampled = true;
        }
    }

    Ok((image, report))
}

/// Pass 1 helper: resolve the alpha channel's meaning, preferring the TGA
/// 2.0 extension-area Attributes Type (Field 24) and falling back to the
/// header Field-5.6 attribute-bit count.
fn resolve_alpha(input: &[u8], image: &mut TgaImage) -> AlphaResolution {
    // The extension-area-aware resolver applies Field 24 when present and
    // otherwise runs the all-alpha-zero "treat as opaque" fallback. We
    // additionally fall through to the header attribute-bit resolver when
    // there is no extension area at all, so a TGA 1.0 file whose header
    // declares zero attribute bits is still forced opaque.
    if parse_tga_extension_area(input).is_some() {
        match resolve_alpha_with_targa32_fallback(input, image) {
            Some(attrs) => AlphaResolution::AttributesType(attrs),
            // Extension area present but no Attributes Type field readable —
            // the fallback heuristic ran; report it via the header bits.
            None => match resolve_alpha_from_descriptor_readonly(input) {
                Some(bits) => AlphaResolution::HeaderBits(bits),
                None => AlphaResolution::Skipped,
            },
        }
    } else {
        match resolve_alpha_from_descriptor(input, image) {
            Some(bits) => AlphaResolution::HeaderBits(bits),
            None => AlphaResolution::Skipped,
        }
    }
}

/// Read the header attribute-bit count without mutating the image (used
/// only to *report* which bits were seen when the extension-area path
/// already settled the pixels).
fn resolve_alpha_from_descriptor_readonly(input: &[u8]) -> Option<AttributeBits> {
    crate::decoder::parse_tga_attribute_bits(input)
}

/// Pass 2 helper: apply the colour-correction table if present, else the
/// gamma value. Never both (spec alternatives).
fn apply_tone(input: &[u8], image: &mut TgaImage) -> ToneApplied {
    if let Some(table) = parse_tga_colour_correction_table(input) {
        table.apply_to_image(image);
        return ToneApplied::ColourCorrection;
    }
    if let Some(gamma) = parse_tga_gamma(input) {
        if !gamma.is_unset() && !gamma.is_identity() {
            gamma.apply_to_image(image);
            return ToneApplied::Gamma(gamma);
        }
    }
    ToneApplied::None
}

/// A decoded TGA frame bundled with the file metadata that the display
/// pipeline does **not** fold into the raster.
///
/// [`decode_tga_for_display`] applies the per-pixel + geometry metadata
/// (alpha / tone / key colour / pixel aspect) and discards the rest.
/// Some §5 / §C.6 fields, though, describe how a frame should be *placed*
/// or *previewed* rather than how its pixels look — a caller doing its own
/// on-screen compositing or building a browser thumbnail strip needs those
/// alongside the finalized pixels. [`decode_tga_frame`] returns them
/// together so the caller makes a single decode call:
///
/// * [`Self::image`] — the finalized, display-ready frame (exactly what
///   [`decode_tga_for_display`] produces for the same options).
/// * [`Self::report`] — what each pipeline pass did (see
///   [`TgaDisplayReport`]).
/// * [`Self::screen_origin`] — the §5.1 / §5.2 on-screen lower-left
///   placement coordinate (orthogonal to the storage-order normalisation
///   the decoder already did). `(0, 0)` for a file that declares no
///   placement.
/// * [`Self::postage_stamp`] — the §C.6.10 embedded thumbnail, decoded to
///   the same normalised format as the main image, if the file ships one.
/// * [`Self::developer_area`] — the §C.7 application-extensible tag
///   directory, if present, so a caller can pull vendor payloads without a
///   second file walk.
#[derive(Debug, Clone)]
pub struct TgaDecodedFrame {
    /// The finalized, display-ready frame.
    pub image: TgaImage,
    /// What each display pass did.
    pub report: TgaDisplayReport,
    /// §5.1 / §5.2 on-screen lower-left placement coordinate. `(0, 0)`
    /// (the [`ImageOrigin::ORIGIN`] sentinel) when the file declares none.
    pub screen_origin: ImageOrigin,
    /// §C.6.10 embedded thumbnail, decoded to the same normalised format
    /// as the main image; `None` when the file ships no postage stamp.
    pub postage_stamp: Option<TgaImage>,
    /// §C.7 developer-area tag directory; `None` when the file has none.
    pub developer_area: Option<TgaDeveloperArea>,
}

impl TgaDecodedFrame {
    /// `true` when the file declared a non-`(0, 0)` on-screen placement —
    /// a caller compositing the frame onto a larger surface should honour
    /// [`Self::screen_origin`].
    pub fn has_screen_offset(&self) -> bool {
        self.screen_origin.is_offset()
    }

    /// `true` when the file shipped an embedded postage-stamp thumbnail.
    pub fn has_postage_stamp(&self) -> bool {
        self.postage_stamp.is_some()
    }
}

/// Decode a TGA to a display-ready frame **and** bundle the placement /
/// preview / developer metadata the pipeline does not fold into the raster.
///
/// The pixels + [`TgaDisplayReport`] are exactly what
/// [`decode_tga_for_display_reported`] produces for `options`; the bundle
/// additionally carries the §5.1 / §5.2 screen-origin coordinate, the
/// §C.6.10 postage-stamp thumbnail, and the §C.7 developer-area directory
/// — read once, from the same input, so the caller makes a single decode
/// call instead of re-walking the file with the individual `parse_tga_*`
/// helpers.
///
/// The postage stamp is decoded with [`parse_tga_postage_stamp`]; a
/// malformed thumbnail in an otherwise-valid file is **not** fatal — it is
/// surfaced as `postage_stamp: None` rather than failing the whole decode
/// (the main image is what the caller asked for). All other errors are the
/// same as [`decode_tga_for_display`].
pub fn decode_tga_frame(input: &[u8], options: &TgaDisplayOptions) -> Result<TgaDecodedFrame> {
    let (image, report) = decode_tga_for_display_reported(input, options)?;
    let screen_origin = parse_tga_image_origin(input).unwrap_or(ImageOrigin::ORIGIN);
    // A broken thumbnail shouldn't sink the main decode the caller wanted.
    let postage_stamp = parse_tga_postage_stamp(input).ok().flatten();
    let developer_area = parse_tga_developer_area(input);
    Ok(TgaDecodedFrame {
        image,
        report,
        screen_origin,
        postage_stamp,
        developer_area,
    })
}
