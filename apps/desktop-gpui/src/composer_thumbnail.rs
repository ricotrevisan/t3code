//! Decode only a bounded first frame off the UI executor; never give GPUI animations.
use std::io::Cursor;

pub(crate) fn thumbnail(bytes: &[u8]) -> Option<std::sync::Arc<gpui_kit::Image>> {
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        crate::views::message_media::check_gif_frames(bytes).ok()?;
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    // ImageReader::decode uses the still-image decoder (first frame for GIF/WebP).
    let decoded = reader.decode().ok()?.thumbnail(256, 256);
    let mut encoded = Cursor::new(Vec::new());
    decoded
        .write_to(&mut encoded, image::ImageFormat::Png)
        .ok()?;
    Some(std::sync::Arc::new(gpui_kit::Image::from_bytes(
        gpui_kit::ImageFormat::Png,
        encoded.into_inner(),
    )))
}
