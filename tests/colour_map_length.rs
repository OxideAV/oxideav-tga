//! A colour map longer than an 8-bit index can address (the header's
//! length field is 16-bit) decodes to the 256 addressable entries, so
//! every decoded `Pal8` image is encodable again (fuzz finding,
//! round 477: a 277-entry map decoded but `encode` refused the palette).

use oxideav_tga::{
    EncodeOptions, TGA_HEADER_SIZE, TgaPixelFormat, decode, encode, parse_tga_color_map,
};

fn long_map_file(entries: u16, first: u16) -> Vec<u8> {
    let mut f = vec![0u8; TGA_HEADER_SIZE];
    f[1] = 1;
    f[2] = 1;
    f[3..5].copy_from_slice(&first.to_le_bytes());
    f[5..7].copy_from_slice(&entries.to_le_bytes());
    f[7] = 24;
    f[12] = 2;
    f[14] = 1;
    f[16] = 8;
    f[17] = 0x20;
    for i in 0..entries {
        f.extend_from_slice(&[i as u8, (i >> 8) as u8, 0]);
    }
    // Two indices: the first stored entry and the last addressable one.
    f.push(first as u8);
    f.push(255);
    f
}

#[test]
fn a_map_longer_than_256_entries_keeps_the_addressable_ones() {
    let file = long_map_file(300, 0);
    let img = decode(&file).unwrap();
    assert_eq!(img.format, TgaPixelFormat::Pal8);
    let pal = img.palette.as_ref().unwrap();
    assert_eq!(pal.len(), 256);
    assert_eq!(pal.entries[255], [0, 0, 255, 255]); // BGR on disk: entry 255 = (b=255, g=0, r=0)
    assert_eq!(img.data(), &[0, 255]);
    assert!(img.validate().is_ok());
    // ... and it encodes again (the identity the registry path relies on).
    let out = encode(&img, &EncodeOptions::default()).unwrap();
    assert_eq!(decode(&out).unwrap(), img);
    // The typed colour-map view still exposes the whole declared table.
    assert_eq!(
        parse_tga_color_map(&file).unwrap().unwrap().entries.len(),
        300
    );
}

#[test]
fn a_non_zero_origin_still_rebases_within_the_kept_entries() {
    // Origin 10: pixel index 10 is stored entry 0, pixel 255 is entry 245.
    let file = long_map_file(400, 10);
    let img = decode(&file).unwrap();
    assert_eq!(img.palette.as_ref().unwrap().len(), 256);
    assert_eq!(img.data(), &[0, 245]);
}
