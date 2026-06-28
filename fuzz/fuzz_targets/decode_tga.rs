#![no_main]

//! Decode arbitrary fuzz-supplied bytes through the full TGA decode
//! chain: `parse_header` (the 18-byte fixed header — id_length,
//! cmap_type, image_type 1/2/3/9/10/11, cmap origin/length/entry_size,
//! width / height / pixel_depth / image_descriptor), the optional
//! Image-ID + colour-map blocks, the raw-or-§C.5-RLE body (15/16-bit
//! A1R5G5B5 unpack, 24-bit BGR→RGB swap, 32-bit BGRA→RGBA swap, the
//! bottom-up→top-down vertical flip, the §C.5 RLE run / raw packet
//! decoder), and the optional 26-byte TGA 2.0 footer at the file
//! tail. Then re-walk the file through the standalone helpers:
//! `parse_tga_footer`, `parse_tga_extension_area`,
//! `parse_tga_postage_stamp`, `parse_tga_colour_correction_table`,
//! `parse_tga_scan_line_table`, `parse_tga_developer_area`,
//! `parse_tga_color_map`, `parse_tga_color_map_type`,
//! `parse_tga_border_color`, `parse_tga_interleaving`, and
//! `parse_tga_image_origin`. Finally, drive the composed §C.6 display
//! pipeline (`decode_tga_for_display` / `_reported`) across the option
//! toggles — the colour-touching passes run unconditionally on the capped
//! raster; the geometry (pixel-aspect resample) pass only when the
//! file's declared aspect ratio keeps the corrected raster under the cap.
//!
//! The contract under test is purely that every call *returns*: a
//! malformed stream yields `Err(TgaError::…)`, a well-formed one
//! yields `Ok(_)`, and no path may panic, abort, integer-overflow
//! (in a debug / ASAN build), index out of bounds, or OOM —
//! regardless of how hostile the bytes are. The return values are
//! intentionally discarded; a round-trip oracle would need a trusted
//! decoder of the *same* arbitrary bytes, which doesn't exist in this
//! codebase (the clean-room wall bars any third-party decoder as a
//! cross-decode oracle).
//!
//! # Why the raster cap
//!
//! `parse_tga` allocates a `width * height * (1 or 4)` byte output
//! buffer up front; `width` and `height` are arbitrary `u16` fields
//! straight off the wire, so the worst case is 65535 × 65535 × 4 ≈
//! 16 GiB. Letting the allocator OOM on a legitimate (parseable but
//! oversized) header is a false-positive that masks the real logic
//! bugs this harness is built to find. We therefore reject declared
//! frames whose total raster exceeds a 16 MiB harness cap (mirroring
//! what a real demuxer's sanity limits would do) before driving the
//! pixel decoder. The library itself keeps no policy raster cap —
//! the cap lives in the fuzz target only. Every parse / table /
//! footer / extension path is still exercised at sizes up to the cap.

use libfuzzer_sys::fuzz_target;
use oxideav_tga::{
    compute_tga_scan_line_table, decode_tga_for_display, decode_tga_for_display_reported,
    parse_tga, parse_tga_attribute_bits, parse_tga_attributes_type, parse_tga_border_color,
    parse_tga_color_map, parse_tga_color_map_type, parse_tga_colour_correction_table,
    parse_tga_developer_area, parse_tga_extension_area, parse_tga_footer, parse_tga_image_id,
    parse_tga_image_origin, parse_tga_interleaving, parse_tga_pixel_aspect_ratio,
    parse_tga_postage_stamp, parse_tga_scan_line, parse_tga_scan_line_table,
    resolve_alpha_from_descriptor, TgaDisplayOptions,
};
use oxideav_tga::{parse_header, TGA_HEADER_SIZE};

/// Upper bound on the declared output raster (16 MiB). Anything
/// larger is a resource request, not a logic path, so the harness
/// skips the pixel-decode call but still exercises every
/// header / footer / extension parser on the same bytes.
const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    // The cheap helpers run on every input regardless of declared
    // raster: they each parse a small fixed-size region and exercise
    // a different decoder code path.
    let _ = parse_tga_footer(data);
    let _ = parse_tga_extension_area(data);
    let _ = parse_tga_postage_stamp(data);
    let _ = parse_tga_colour_correction_table(data);
    let _ = parse_tga_developer_area(data);
    let _ = parse_tga_attributes_type(data);
    let _ = parse_tga_image_id(data);
    let _ = parse_tga_color_map(data);
    let _ = parse_tga_color_map_type(data);
    let _ = parse_tga_border_color(data);
    let _ = parse_tga_attribute_bits(data);
    let _ = parse_tga_interleaving(data);
    let _ = parse_tga_image_origin(data);

    // §C.6.9 random access: derive a scan-line table from the bytes
    // (an O(input) walk; the table itself is at most height × 4 ≈
    // 256 KiB and a single decoded row at most width × 4 ≈ 256 KiB,
    // so no raster cap is needed) and fetch one row through it; also
    // drive the row fetch through a *file-supplied* (i.e. attacker-
    // controlled) table when the input ships one.
    if let Ok(table) = compute_tga_scan_line_table(data) {
        let _ = parse_tga_scan_line(data, &table, 0);
    }
    if let Some(table) = parse_tga_scan_line_table(data) {
        let _ = parse_tga_scan_line(data, &table, 0);
        let _ = parse_tga_scan_line(data, &table, table.len().saturating_sub(1));
    }

    // Pre-screen the header (no pixel allocation) so we can bound the
    // decoder's per-frame allocation before it runs. A header that
    // doesn't even parse is still a perfectly good exercise of the
    // parse-rejection paths, so fall through to `parse_tga` in that
    // case — it will return the same `Err` cheaply.
    if data.len() >= TGA_HEADER_SIZE {
        if let Some(hdr) = parse_header(&data[..TGA_HEADER_SIZE]) {
            // Worst-case container size per pixel is 4 bytes (RGBA);
            // a width=0 / height=0 file is rejected by the decoder
            // itself so we don't need to special-case zero here.
            let total = (hdr.width as u64)
                .checked_mul(hdr.height as u64)
                .and_then(|wh| wh.checked_mul(4));
            match total {
                Some(n) if n <= MAX_OUTPUT_BYTES => {}
                _ => return,
            }
        }
    }

    if let Ok(mut image) = parse_tga(data) {
        // §C.2 header-local alpha resolver: mutates a decoded RGBA frame
        // in place from the descriptor's attribute-bit count. Must stay
        // panic-free on any decodable input.
        let _ = resolve_alpha_from_descriptor(data, &mut image);

        // Composed §C.6 display pipeline. The colour-touching passes
        // (alpha resolution + tone curve + key colour) operate in place on
        // the already-capped raster, so they are run with every selectable
        // toggle. The geometry pass (pixel aspect) is gated off here: the
        // file-supplied aspect numerator/denominator are arbitrary u16s, so
        // the resampled width/height can be up to ~65535× the source axis —
        // a resource request, not a logic path, exactly like the raster cap
        // above. Driving it unconditionally would let the harness OOM on a
        // legitimate-but-huge declared aspect ratio.
        let no_geom = TgaDisplayOptions::ALL.with_apply_pixel_aspect(false);
        let _ = decode_tga_for_display(data, &no_geom);
        let _ = decode_tga_for_display_reported(data, &no_geom);
        let _ = decode_tga_for_display(data, &TgaDisplayOptions::NONE);

        // Only drive the pixel-aspect resample when the file's declared
        // aspect ratio keeps the corrected raster under the same 16 MiB cap.
        if let Some(par) = parse_tga_pixel_aspect_ratio(data) {
            if let Some((dw, dh)) = par.corrected_display_dimensions(image.width, image.height) {
                let bounded = (dw as u64)
                    .checked_mul(dh as u64)
                    .and_then(|wh| wh.checked_mul(4))
                    .map(|n| n <= MAX_OUTPUT_BYTES)
                    .unwrap_or(false);
                if bounded {
                    let _ = decode_tga_for_display(data, &TgaDisplayOptions::ALL);
                }
            }
        }
    }
});
