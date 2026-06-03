//! Criterion benchmarks for the TGA decode + encode hot paths.
//!
//! Ten scenarios isolate the cost of a single decode or encode call
//! on procedurally-generated inputs. No fixtures are committed: each
//! benchmark builds its pixel buffer or its on-disk byte stream once
//! during the bench's `iter_with_setup`-equivalent block, then drives
//! the public API per iteration. Wall-clock numbers depend on the
//! host; the value of these benchmarks is the relative cost across
//! scenarios (uncompressed vs RLE, run-heavy vs noise, 24 bpp vs 32
//! bpp, true-colour vs palette vs grayscale).
//!
//! The benches use only the crate's published standalone API
//! (`parse_tga`, `encode_tga_uncompressed`, `encode_tga_rle`,
//! `encode_tga_uncompressed_rgb24`, `encode_tga_grayscale`,
//! `encode_tga_palette`). The TGA 2.0 spec (`docs/image/tga/tgaffs.pdf`)
//! defines what each path does on the wire; the benches characterise
//! how long each takes on synthetic frames sized for sub-second
//! runs at criterion's default sample count.
//!
//! Scenario sizes are deliberately small (256 × 256) so the whole
//! harness completes in well under a minute on a laptop. Tune via
//! `cargo bench -- --sample-size N` if needed.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use oxideav_tga::{
    encode_tga_grayscale, encode_tga_palette, encode_tga_rle, encode_tga_uncompressed,
    encode_tga_uncompressed_rgb24, parse_tga,
};

const W: u16 = 256;
const H: u16 = 256;

// -- input synthesis ---------------------------------------------------------

/// Smooth 24-bit gradient: a sweep over (R, G, B) by row/col. Every
/// pixel is unique → opaque RGBA encoders pick 24 bpp; RLE encoders
/// degenerate to raw packets (no 2-in-a-row runs).
fn gradient_rgba_noise(w: u16, h: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            out.push((x & 0xFF) as u8);
            out.push((y & 0xFF) as u8);
            out.push(((x ^ y) & 0xFF) as u8);
            out.push(0xFF);
        }
    }
    out
}

/// Smooth 32-bit gradient with non-trivial alpha so the auto-depth
/// selector emits 32 bpp instead of 24 bpp.
fn gradient_rgba_alpha(w: u16, h: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            out.push((x & 0xFF) as u8);
            out.push((y & 0xFF) as u8);
            out.push(((x.wrapping_add(y)) & 0xFF) as u8);
            // Alternating alpha so at least some bytes are < 0xFF.
            out.push(if (x ^ y) & 1 == 0 { 0x7F } else { 0xFF });
        }
    }
    out
}

/// 24-bit run-heavy input: rows of 32 identical pixels followed by a
/// step. Each scanline has exactly 8 distinct colour bands, so the
/// RLE encoder packs each row into ~8 run packets of 32 pixels each.
fn rgba_runs(w: u16, h: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let band = (x / 32) as u8;
            let r = band.wrapping_mul(31);
            let g = (y as u8).wrapping_mul(7);
            out.push(r);
            out.push(g);
            out.push(band);
            out.push(0xFF);
        }
    }
    out
}

/// 8-bit grayscale gradient.
fn gray8(w: u16, h: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for x in 0..w {
            out.push(((x ^ y) & 0xFF) as u8);
        }
    }
    out
}

/// 24-bit RGB (alpha-free) gradient for the rgb24-input encoders.
fn rgb24(w: u16, h: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(w as usize * h as usize * 3);
    for y in 0..h {
        for x in 0..w {
            out.push((x & 0xFF) as u8);
            out.push((y & 0xFF) as u8);
            out.push(((x ^ y) & 0xFF) as u8);
        }
    }
    out
}

/// Palette-sized input: 256 unique RGBA colours mapped by `(x ^ y) % 256`.
/// Keeps the encoder under its 256-colour cap.
fn palette_rgba(w: u16, h: u16) -> Vec<u8> {
    let mut palette = [[0u8; 4]; 256];
    for (i, slot) in palette.iter_mut().enumerate() {
        let i = i as u8;
        *slot = [i, i.wrapping_mul(3), i.wrapping_mul(7), 0xFF];
    }
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let idx = ((x ^ y) & 0xFF) as usize;
            out.extend_from_slice(&palette[idx]);
        }
    }
    out
}

// -- benches: decode ---------------------------------------------------------

fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode");

    // type 2 / 24 bpp — every-pixel-unique gradient encoded uncompressed.
    let uncompressed_24 = encode_tga_uncompressed(W, H, &gradient_rgba_noise(W, H)).unwrap();
    group.bench_function("uncompressed_24bpp", |b| {
        b.iter(|| {
            let img = parse_tga(black_box(&uncompressed_24)).unwrap();
            black_box(img);
        })
    });

    // type 2 / 32 bpp — same gradient with alternating alpha.
    let uncompressed_32 = encode_tga_uncompressed(W, H, &gradient_rgba_alpha(W, H)).unwrap();
    group.bench_function("uncompressed_32bpp", |b| {
        b.iter(|| {
            let img = parse_tga(black_box(&uncompressed_32)).unwrap();
            black_box(img);
        })
    });

    // type 10 / 24 bpp / run-heavy — 8 long run packets per row.
    let rle_runs = encode_tga_rle(W, H, &rgba_runs(W, H)).unwrap();
    group.bench_function("rle_24bpp_runs", |b| {
        b.iter(|| {
            let img = parse_tga(black_box(&rle_runs)).unwrap();
            black_box(img);
        })
    });

    // type 10 / 24 bpp / noise — RLE degenerates to raw packets.
    let rle_noise = encode_tga_rle(W, H, &gradient_rgba_noise(W, H)).unwrap();
    group.bench_function("rle_24bpp_noise", |b| {
        b.iter(|| {
            let img = parse_tga(black_box(&rle_noise)).unwrap();
            black_box(img);
        })
    });

    // type 3 / 8 bpp grayscale.
    let gs = encode_tga_grayscale(W, H, &gray8(W, H)).unwrap();
    group.bench_function("grayscale_8bpp", |b| {
        b.iter(|| {
            let img = parse_tga(black_box(&gs)).unwrap();
            black_box(img);
        })
    });

    // type 1 / 8 bpp + 256-colour palette.
    let pal = encode_tga_palette(W, H, &palette_rgba(W, H)).unwrap();
    group.bench_function("palette_8bpp", |b| {
        b.iter(|| {
            let img = parse_tga(black_box(&pal)).unwrap();
            black_box(img);
        })
    });

    group.finish();
}

// -- benches: encode ---------------------------------------------------------

fn bench_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode");

    // type 2 / RGBA-in / auto-24-or-32-bpp.
    let rgba = gradient_rgba_alpha(W, H);
    group.bench_function("uncompressed_rgba", |b| {
        b.iter(|| {
            let bytes = encode_tga_uncompressed(W, H, black_box(&rgba)).unwrap();
            black_box(bytes);
        })
    });

    // type 2 / RGB24-in / fixed 24 bpp (no alpha scan).
    let rgb = rgb24(W, H);
    group.bench_function("uncompressed_rgb24", |b| {
        b.iter(|| {
            let bytes = encode_tga_uncompressed_rgb24(W, H, black_box(&rgb)).unwrap();
            black_box(bytes);
        })
    });

    // type 10 / run-heavy — exercises the §C.5 run-packet path.
    let runs = rgba_runs(W, H);
    group.bench_function("rle_rgba_runs", |b| {
        b.iter(|| {
            let bytes = encode_tga_rle(W, H, black_box(&runs)).unwrap();
            black_box(bytes);
        })
    });

    // type 10 / noise — exercises the §C.5 raw-packet path.
    let noise = gradient_rgba_noise(W, H);
    group.bench_function("rle_rgba_noise", |b| {
        b.iter(|| {
            let bytes = encode_tga_rle(W, H, black_box(&noise)).unwrap();
            black_box(bytes);
        })
    });

    group.finish();
}

criterion_group!(benches, bench_decode, bench_encode);
criterion_main!(benches);
