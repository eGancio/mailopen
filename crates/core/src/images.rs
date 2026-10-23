//! Decode images from mail (inline parts, attachments) and fetch remote ones when the user
//! allows it. Always called off the GUI thread.

use std::time::Duration;

/// Pixels, RGBA, unpremultiplied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Remote images larger than this are not downloaded.
const MAX_REMOTE: u64 = 8 * 1024 * 1024;
/// Decoding refuses images that would need more memory than this.
const MAX_ALLOC: u64 = 256 * 1024 * 1024;

/// Decode PNG, JPEG, GIF (first frame), WebP, BMP or ICO, scaled down so neither side exceeds
/// `max_side`.
pub fn decode(bytes: &[u8], max_side: u32) -> Option<Decoded> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    let max_side = max_side.max(16);
    let img = if w > max_side || h > max_side {
        img.resize(max_side, max_side, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(Decoded {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// GET an `http(s)` image: no cookies, short timeout, size cap.
pub fn fetch(url: &str) -> Option<Vec<u8>> {
    let lower = url.trim().to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return None;
    }
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(12)))
            .http_status_as_error(false)
            .max_redirects(5)
            .build(),
    );
    let mut resp = agent.get(url.trim()).call().ok()?;
    if resp.status() != 200 {
        return None;
    }
    resp.body_mut()
        .with_config()
        .limit(MAX_REMOTE)
        .read_to_vec()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_and_scales() {
        let img = image::RgbaImage::from_pixel(400, 100, image::Rgba([10, 20, 30, 255]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let d = decode(&png, 200).unwrap();
        assert_eq!((d.width, d.height), (200, 50));
        assert_eq!(d.rgba.len(), 200 * 50 * 4);
        assert!(decode(b"not an image", 100).is_none());
        assert!(decode(b"", 100).is_none());
        assert!(fetch("file:///etc/passwd").is_none());
    }
}
