//! Bounded still-image preparation before signing: transmitted metadata describes output bytes.
use std::io::Cursor;

pub const IMAGE_LIMIT: usize = 10 * 1024 * 1024;
pub const SOURCE_LIMIT: usize = 50 * 1024 * 1024;

pub fn prepare(
    name: String,
    mime: String,
    bytes: Vec<u8>,
) -> Result<(String, String, Vec<u8>), String> {
    prepare_with_limit(name, mime, bytes, IMAGE_LIMIT)
}

fn prepare_with_limit(
    name: String,
    mime: String,
    bytes: Vec<u8>,
    limit: usize,
) -> Result<(String, String, Vec<u8>), String> {
    if matches!(mime.as_str(), "image/heic" | "image/heif") {
        return Err("HEIC decoding is unavailable in this native build; export the photo as JPEG or PNG and reattach".into());
    }
    if bytes.len() <= limit {
        return Ok((name, mime, bytes));
    }
    if bytes.len() > SOURCE_LIMIT {
        return Err("Image source exceeds 50 MiB".into());
    }
    let mut reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| "Could not read image")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| "Could not read image")?
        .into_dimensions()
        .map_err(|_| "Could not read image dimensions")?;
    if u64::from(width) * u64::from(height) > 64_000_000 {
        return Err("Image exceeds 64 million pixels".into());
    }
    let decoded = reader
        .decode()
        .map_err(|_| "Could not decode image for compression")?;
    let base = width.max(height).min(2048);
    for scale in [1.0, 0.75, 0.55] {
        let edge = ((base as f64 * scale).round() as u32).max(1);
        let rgba = decoded.thumbnail(edge, edge).to_rgba8();
        // JPEG fallback matches web's white matte rather than losing transparent pixels to black.
        let rgb = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
            let p = rgba.get_pixel(x, y).0;
            let alpha = u32::from(p[3]);
            image::Rgb(
                [0, 1, 2]
                    .map(|i| ((u32::from(p[i]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8),
            )
        });
        for quality in [92, 85, 78, 68] {
            let mut output = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality)
                .encode_image(&rgb)
                .map_err(|_| "Could not encode image")?;
            if output.len() <= limit {
                let stem = name
                    .rsplit_once('.')
                    .filter(|(stem, _)| !stem.is_empty())
                    .map_or(name.as_str(), |(stem, _)| stem);
                return Ok((format!("{stem}.jpg"), "image/jpeg".into(), output));
            }
        }
    }
    Err("Image still exceeds the upload limit after compression".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r14_c_image_passthrough_compression_metadata_and_heic_failure() {
        let rgba = image::RgbaImage::from_fn(300, 300, |x, y| {
            image::Rgba([(x % 255) as u8, (y % 255) as u8, 0, 0])
        });
        let mut source = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(rgba)
            .write_to(&mut source, image::ImageFormat::Png)
            .unwrap();
        let bytes = source.into_inner();
        assert_eq!(
            prepare("small.png".into(), "image/png".into(), bytes.clone())
                .unwrap()
                .2,
            bytes
        );
        let (name, mime, compressed) =
            prepare_with_limit("shot.png".into(), "image/png".into(), bytes, 2000).unwrap();
        assert_eq!((name.as_str(), mime.as_str()), ("shot.jpg", "image/jpeg"));
        assert!(compressed.len() <= 2000);
        assert_eq!(
            image::load_from_memory(&compressed)
                .unwrap()
                .to_rgb8()
                .get_pixel(0, 0)
                .0,
            [255, 255, 255]
        );
        assert!(
            prepare("photo.heic".into(), "image/heic".into(), vec![1])
                .unwrap_err()
                .contains("export")
        );
        assert!(prepare_with_limit("bad.png".into(), "image/png".into(), vec![0; 10], 1).is_err());
    }
}
