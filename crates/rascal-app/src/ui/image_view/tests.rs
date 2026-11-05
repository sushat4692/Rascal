use std::io::Cursor;

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

use super::{ImageDiffMode, compute_difference, is_image_path};

#[test]
fn detects_image_extensions_case_insensitively() {
    assert!(is_image_path("a.png"));
    assert!(is_image_path("dir/photo.JPG"));
    assert!(is_image_path("icon.svg"));
    assert!(!is_image_path("README.md"));
    assert!(!is_image_path("archive.png.zip"));
}

#[test]
fn image_diff_mode_keys_roundtrip() {
    for mode in [
        ImageDiffMode::SideBySide,
        ImageDiffMode::Slider,
        ImageDiffMode::Difference,
    ] {
        assert_eq!(ImageDiffMode::from_key(mode.key()), Some(mode));
    }
}

fn png(image: RgbaImage) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

#[test]
fn computes_changed_pixels() {
    let old = png(RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255])));
    let mut new_image = RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]));
    new_image.put_pixel(1, 1, Rgba([255, 255, 255, 255]));

    let result = compute_difference(&old, &png(new_image)).unwrap();
    assert_eq!(result.changed, 1);
    assert_eq!(result.total, 4);
    assert!(!result.mismatch);

    let diff = image::load_from_memory(&result.png).unwrap().to_rgba8();
    assert_eq!(diff.get_pixel(1, 1), &Rgba([255, 80, 112, 255]));
    assert_eq!(diff.get_pixel(0, 0), &Rgba([0, 0, 0, 0]));
}

#[test]
fn handles_size_mismatch() {
    let old = png(RgbaImage::from_pixel(2, 1, Rgba([0, 0, 0, 255])));
    let new = png(RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255])));
    let result = compute_difference(&old, &new).unwrap();
    assert!(result.mismatch);
    assert_eq!(result.total, 2);
    assert_eq!(result.changed, 1);
}
