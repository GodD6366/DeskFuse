//! Exact image identity shared by history and clipboard echo suppression.
use deskfuse_platform::PasteboardSnapshot;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};

const MAX_ENCODED_BYTES: u64 = 64 * 1024 * 1024;

/// One image, whether captured as PNG/TIFF or materialized as a file by Handoff.
/// Multiple files/items retain their full clipboard identity and restore order.
pub(crate) fn snapshot_digest(snapshot: &PasteboardSnapshot) -> Option<String> {
    if !snapshot.file_urls.is_empty() {
        let [path] = snapshot.file_urls.as_slice() else {
            return None;
        };
        crate::clipboard_library::image_file_mime(path)?;
        let file = std::fs::File::open(path).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > MAX_ENCODED_BYTES {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(MAX_ENCODED_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        return bytes_digest(&bytes);
    }
    let content_items = snapshot
        .items
        .iter()
        .filter(|item| {
            item.representations.iter().any(|r| {
                !matches!(
                    r.type_name.as_str(),
                    crate::clipboard_bundle::SOURCE_METADATA_TYPE
                        | "com.deskfuse.clipboard-origin"
                        | "com.apple.is-remote-clipboard"
                )
            })
        })
        .count();
    if content_items > 1 {
        return None;
    }
    if let Some(bytes) = snapshot.png.as_deref()
        && let Some(digest) = bytes_digest(bytes)
    {
        return Some(digest);
    }
    for kind in ["public.png", "public.tiff", "public.jpeg", "public.heic"] {
        if let Some(digest) = snapshot
            .items
            .iter()
            .flat_map(|i| &i.representations)
            .filter(|r| r.type_name == kind)
            .find_map(|r| bytes_digest(&r.data))
        {
            return Some(digest);
        }
    }
    None
}

fn bytes_digest(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_ENCODED_BYTES {
        return None;
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(32_768);
    limits.max_image_height = Some(32_768);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    if u64::from(image.width()) * u64::from(image.height()) * 4 > 128 * 1024 * 1024 {
        return None;
    }
    let mut rgba = image.to_rgba8();
    // Invisible RGB channels must not distinguish two otherwise identical images.
    for pixel in rgba.pixels_mut() {
        if pixel[3] == 0 {
            pixel.0 = [0; 4];
        }
    }
    let mut digest = Sha256::new();
    digest.update(b"deskfuse-image-rgba-v1\0");
    digest.update(rgba.width().to_le_bytes());
    digest.update(rgba.height().to_le_bytes());
    digest.update(rgba.as_raw());
    Some(format!("image-v1:{}", hex::encode(digest.finalize())))
}
