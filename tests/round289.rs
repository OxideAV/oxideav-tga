//! Round 289 (profile + bit-identical RLE run-packet optimization).
//!
//! `decode_rle_pixels` previously called `emit_pixel` once per output
//! pixel, including inside a §C.5 run-length packet where every pixel
//! is byte-identical. The run path now expands the source pixel once
//! and replicates the emitted output bytes for the rest of the run.
//! That is a pure performance change: the decoded bytes must be
//! identical to what the per-pixel loop produced.
//!
//! These tests pin that bit-identical contract across every RLE image
//! type and pixel depth the crate decodes, with runs of varied length
//! (including the 128-pixel maximum and 1-pixel minimum) interleaved
//! with raw packets. The oracle is the fully-decoded image compared
//! against a hand-rolled, deliberately naive scalar reference that
//! expands each run pixel-by-pixel — independent of the optimized
//! code path under test.

use oxideav_tga::parse_tga;

/// Build a minimal TGA header (18 bytes) for the given parameters.
/// The argument list mirrors the on-wire header fields, hence the count.
#[allow(clippy::too_many_arguments)]
fn header(
    id_len: u8,
    cmap_type: u8,
    image_type: u8,
    cmap_first: u16,
    cmap_len: u16,
    cmap_entry_size: u8,
    width: u16,
    height: u16,
    depth: u8,
    descriptor: u8,
) -> Vec<u8> {
    let mut h = Vec::with_capacity(18);
    h.push(id_len);
    h.push(cmap_type);
    h.push(image_type);
    h.extend_from_slice(&cmap_first.to_le_bytes());
    h.extend_from_slice(&cmap_len.to_le_bytes());
    h.push(cmap_entry_size);
    h.extend_from_slice(&0u16.to_le_bytes()); // x origin
    h.extend_from_slice(&0u16.to_le_bytes()); // y origin
    h.extend_from_slice(&width.to_le_bytes());
    h.extend_from_slice(&height.to_le_bytes());
    h.push(depth);
    h.push(descriptor);
    h
}

/// A scripted RLE body: a list of (is_run, count, pixel-bytes-per-step).
/// For a run packet, `pixels` holds exactly one pixel (`in_bpp` bytes).
/// For a raw packet, `pixels` holds `count` pixels back to back.
struct Packet {
    is_run: bool,
    count: usize,
    pixels: Vec<u8>,
}

fn build_rle_body(packets: &[Packet]) -> Vec<u8> {
    let mut body = Vec::new();
    for p in packets {
        assert!((1..=128).contains(&p.count));
        let mut ctrl = (p.count - 1) as u8;
        if p.is_run {
            ctrl |= 0x80;
        }
        body.push(ctrl);
        body.extend_from_slice(&p.pixels);
    }
    body
}

/// Naive reference: expand the same packet script into the normalised
/// output the decoder must produce, pixel-by-pixel, with no run
/// shortcut. `expand` turns one on-disk pixel into its output bytes.
fn naive_expand(packets: &[Packet], in_bpp: usize, expand: impl Fn(&[u8]) -> Vec<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    for p in packets {
        if p.is_run {
            let px = &p.pixels[..in_bpp];
            for _ in 0..p.count {
                out.extend_from_slice(&expand(px));
            }
        } else {
            for i in 0..p.count {
                let px = &p.pixels[i * in_bpp..i * in_bpp + in_bpp];
                out.extend_from_slice(&expand(px));
            }
        }
    }
    out
}

fn expand5(v: u8) -> u8 {
    (v << 3) | (v >> 2)
}

#[test]
fn rle_truecolour_24_bit_identical() {
    // 24 bpp BGR on disk → RGBA. Mix of runs (incl. 128-max and
    // 1-min) and raw packets so total pixels == width*height.
    let in_bpp = 3;
    let packets = vec![
        Packet {
            is_run: true,
            count: 128,
            pixels: vec![0x10, 0x20, 0x30],
        },
        Packet {
            is_run: false,
            count: 3,
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8, 9],
        },
        Packet {
            is_run: true,
            count: 1,
            pixels: vec![0xAA, 0xBB, 0xCC],
        },
        Packet {
            is_run: true,
            count: 124,
            pixels: vec![0x44, 0x55, 0x66],
        },
    ];
    let total: usize = packets.iter().map(|p| p.count).sum();
    assert_eq!(total, 256);
    let mut file = header(0, 0, 10, 0, 0, 0, 16, 16, 24, 0x20);
    file.extend_from_slice(&build_rle_body(&packets));

    let img = parse_tga(&file).unwrap();
    let expected = naive_expand(&packets, in_bpp, |px| vec![px[2], px[1], px[0], 0xFF]);
    assert_eq!(img.data, expected);
}

#[test]
fn rle_truecolour_32_bit_identical() {
    let in_bpp = 4;
    let packets = vec![
        Packet {
            is_run: true,
            count: 100,
            pixels: vec![0x11, 0x22, 0x33, 0x7F],
        },
        Packet {
            is_run: false,
            count: 2,
            pixels: vec![9, 8, 7, 6, 5, 4, 3, 2],
        },
        Packet {
            is_run: true,
            count: 26,
            pixels: vec![0xDE, 0xAD, 0xBE, 0xEF],
        },
    ];
    let total: usize = packets.iter().map(|p| p.count).sum();
    assert_eq!(total, 128);
    let mut file = header(0, 0, 10, 0, 0, 0, 16, 8, 32, 0x20);
    file.extend_from_slice(&build_rle_body(&packets));

    let img = parse_tga(&file).unwrap();
    let expected = naive_expand(&packets, in_bpp, |px| vec![px[2], px[1], px[0], px[3]]);
    assert_eq!(img.data, expected);
}

#[test]
fn rle_truecolour_16_bit_identical() {
    // 16 bpp A1R5G5B5 little-endian on disk → RGBA expansion.
    let in_bpp = 2;
    let packets = vec![
        Packet {
            is_run: true,
            count: 64,
            pixels: vec![0x1F, 0x7C], // some A1R5G5B5 value
        },
        Packet {
            is_run: false,
            count: 4,
            pixels: vec![0x00, 0x80, 0xFF, 0x7F, 0x21, 0x04, 0x55, 0xAA],
        },
        Packet {
            is_run: true,
            count: 60,
            pixels: vec![0xE0, 0x03],
        },
    ];
    let total: usize = packets.iter().map(|p| p.count).sum();
    assert_eq!(total, 128);
    let mut file = header(0, 0, 10, 0, 0, 0, 16, 8, 16, 0x20);
    file.extend_from_slice(&build_rle_body(&packets));

    let img = parse_tga(&file).unwrap();
    let expected = naive_expand(&packets, in_bpp, |px| {
        let v = u16::from_le_bytes([px[0], px[1]]);
        let r = expand5(((v >> 10) & 0x1F) as u8);
        let g = expand5(((v >> 5) & 0x1F) as u8);
        let b = expand5((v & 0x1F) as u8);
        // depth 16: attribute bit governs alpha.
        let a = if (v & 0x8000) != 0 { 0xFF } else { 0x00 };
        vec![r, g, b, a]
    });
    assert_eq!(img.data, expected);
}

#[test]
fn rle_grayscale_bit_identical() {
    let in_bpp = 1;
    let packets = vec![
        Packet {
            is_run: true,
            count: 128,
            pixels: vec![0x42],
        },
        Packet {
            is_run: false,
            count: 5,
            pixels: vec![1, 2, 3, 4, 5],
        },
        Packet {
            is_run: true,
            count: 123,
            pixels: vec![0xF0],
        },
    ];
    let total: usize = packets.iter().map(|p| p.count).sum();
    assert_eq!(total, 256);
    let mut file = header(0, 0, 11, 0, 0, 0, 16, 16, 8, 0x20);
    file.extend_from_slice(&build_rle_body(&packets));

    let img = parse_tga(&file).unwrap();
    let expected = naive_expand(&packets, in_bpp, |px| vec![px[0]]);
    assert_eq!(img.data, expected);
}

#[test]
fn rle_colour_mapped_bit_identical() {
    // Type 9: 8-bit palette indices, 24-bit BGR palette entries.
    let in_bpp = 1;
    let cmap_len = 4u16;
    // Palette entries on disk are BGR (depth 24).
    let palette_bgr: [[u8; 3]; 4] = [
        [0x01, 0x02, 0x03],
        [0x10, 0x20, 0x30],
        [0xAA, 0xBB, 0xCC],
        [0xFF, 0x00, 0x80],
    ];
    let packets = vec![
        Packet {
            is_run: true,
            count: 128,
            pixels: vec![2],
        },
        Packet {
            is_run: false,
            count: 4,
            pixels: vec![0, 1, 2, 3],
        },
        Packet {
            is_run: true,
            count: 124,
            pixels: vec![1],
        },
    ];
    let total: usize = packets.iter().map(|p| p.count).sum();
    assert_eq!(total, 256);

    let mut file = header(0, 1, 9, 0, cmap_len, 24, 16, 16, 8, 0x20);
    for e in &palette_bgr {
        file.extend_from_slice(e); // BGR on disk
    }
    file.extend_from_slice(&build_rle_body(&packets));

    let img = parse_tga(&file).unwrap();
    let expected = naive_expand(&packets, in_bpp, |px| {
        let bgr = palette_bgr[px[0] as usize];
        vec![bgr[2], bgr[1], bgr[0], 0xFF]
    });
    assert_eq!(img.data, expected);
}

#[test]
fn rle_run_then_raw_alternating_24() {
    // Stress the replication/cursor bookkeeping with many short
    // alternating packets, ensuring run replication never bleeds into
    // the following raw packet's bytes.
    let in_bpp = 3;
    let mut packets = Vec::new();
    let mut total = 0usize;
    let mut seed = 7u8;
    while total < 256 {
        let remaining = 256 - total;
        let run = packets.len() % 2 == 0;
        let count = if run {
            remaining.clamp(1, 5)
        } else {
            remaining.clamp(1, 3)
        };
        if run {
            packets.push(Packet {
                is_run: true,
                count,
                pixels: vec![seed, seed.wrapping_add(1), seed.wrapping_add(2)],
            });
        } else {
            let mut px = Vec::new();
            for i in 0..count {
                let b = seed.wrapping_add(i as u8 * 3);
                px.extend_from_slice(&[b, b.wrapping_add(1), b.wrapping_add(2)]);
            }
            packets.push(Packet {
                is_run: false,
                count,
                pixels: px,
            });
        }
        seed = seed.wrapping_add(13);
        total += count;
    }
    assert_eq!(total, 256);

    let mut file = header(0, 0, 10, 0, 0, 0, 16, 16, 24, 0x20);
    file.extend_from_slice(&build_rle_body(&packets));

    let img = parse_tga(&file).unwrap();
    let expected = naive_expand(&packets, in_bpp, |px| vec![px[2], px[1], px[0], 0xFF]);
    assert_eq!(img.data, expected);
}
