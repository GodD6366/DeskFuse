//! Exact image identity shared by history and clipboard echo suppression.
use deskfuse_platform::{PasteboardItem, PasteboardSnapshot};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Cursor, Read};

const MAX_ENCODED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DHASH_BYTES: usize = 8 * 1024 * 1024;
const IMAGE_KINDS: [&str; 4] = ["public.png", "public.tiff", "public.jpeg", "public.heic"];

/// One pass over an image-bearing clipboard snapshot, computed on a blocking
/// thread so the bridge event loop never decodes image bytes.
///
/// `primary` follows [`snapshot_digest`] precedence: the exact-pixel identity
/// used for history rows, source metadata and image-echo detection.
/// `equivalent_ids` adds the exact encoded-byte hash and perceptual dHash of
/// every distinct image representation, deduplicated by exact bytes so each
/// blob is decoded at most once per pass.
#[derive(Debug, Default, Clone)]
pub(crate) struct ClipboardImageFacts {
    pub(crate) primary: Option<String>,
    pub(crate) equivalent_ids: Vec<String>,
}

/// Analyze `snapshot` in a single decode pass. See [`ClipboardImageFacts`].
pub(crate) fn analyze_snapshot(snapshot: &PasteboardSnapshot) -> ClipboardImageFacts {
    let mut ids = Vec::new();
    let mut decoded: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();

    let primary = if !snapshot.file_urls.is_empty() {
        // A Handoff-materialised file is identified by its pixels; the path
        // itself never participates in identity.
        snapshot_digest(snapshot)
    } else if content_items(&snapshot.items) <= 1 {
        let mut primary = snapshot
            .png
            .as_deref()
            .and_then(|png| inspect(png, true, &mut ids, &mut decoded).0);
        if primary.is_none() {
            'candidates: for kind in IMAGE_KINDS {
                for rep in image_reps(snapshot, kind) {
                    if let Some(digest) = inspect(&rep.data, true, &mut ids, &mut decoded).0 {
                        primary = Some(digest);
                        break 'candidates;
                    }
                }
            }
        }
        primary
    } else {
        None
    };

    // Every distinct representation still contributes its exact/perceptual
    // aliases, even when it lost the primary precedence race.
    if let Some(png) = snapshot.png.as_deref() {
        inspect(png, false, &mut ids, &mut decoded);
    }
    for kind in IMAGE_KINDS {
        for rep in image_reps(snapshot, kind) {
            inspect(&rep.data, false, &mut ids, &mut decoded);
        }
    }

    ids.sort();
    ids.dedup();
    ClipboardImageFacts {
        primary,
        equivalent_ids: ids,
    }
}

/// Decode `bytes` at most once per pass. Returns the exact-pixel digest
/// (only when `want_primary` and the bounded decoder accepts the blob) and
/// the perceptual alias used for Handoff rewrites.
fn inspect(
    bytes: &[u8],
    want_primary: bool,
    ids: &mut Vec<String>,
    decoded: &mut HashMap<String, (Option<String>, Option<String>)>,
) -> (Option<String>, Option<String>) {
    let exact = crate::clipboard_echo::hashed("img", bytes);
    if !ids.contains(&exact) {
        ids.push(exact.clone());
    }
    if let Some(result) = decoded.get(&exact) {
        return result.clone();
    }
    let result = if want_primary {
        match strict_image(bytes) {
            Some(image) => (
                Some(rgba_digest(&image)),
                (bytes.len() <= MAX_DHASH_BYTES).then(|| dhash_of(&image)),
            ),
            // The bounded decoder may reject what the unbounded one accepts;
            // the perceptual alias still applies in that case.
            None => (None, perceptual_dhash(bytes)),
        }
    } else {
        (None, perceptual_dhash(bytes))
    };
    decoded.insert(exact, result.clone());
    result
}

/// One image, whether captured as PNG/TIFF or materialized as a file by Handoff.
/// Multiple files/items retain their full clipboard identity and restore order.
pub(crate) fn snapshot_digest(snapshot: &PasteboardSnapshot) -> Option<String> {
    digest_parts(
        &snapshot.file_urls,
        snapshot.png.as_deref(),
        &snapshot.items,
    )
}

/// Borrowed form of [`snapshot_digest`] for callers that already hold the
/// pieces, such as the clipboard-sync decode task.
pub(crate) fn digest_parts(
    file_urls: &[String],
    png: Option<&[u8]>,
    items: &[PasteboardItem],
) -> Option<String> {
    if !file_urls.is_empty() {
        let [path] = file_urls else {
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
    if content_items(items) > 1 {
        return None;
    }
    if let Some(bytes) = png
        && let Some(digest) = bytes_digest(bytes)
    {
        return Some(digest);
    }
    for kind in IMAGE_KINDS {
        if let Some(digest) = items
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

fn content_items(items: &[PasteboardItem]) -> usize {
    items
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
        .count()
}

fn image_reps<'a>(
    snapshot: &'a PasteboardSnapshot,
    kind: &'static str,
) -> impl Iterator<Item = &'a deskfuse_platform::PasteboardRepresentation> {
    snapshot
        .items
        .iter()
        .flat_map(|item| &item.representations)
        .filter(move |rep| rep.type_name == kind)
}

fn bytes_digest(bytes: &[u8]) -> Option<String> {
    Some(rgba_digest(&strict_image(bytes)?))
}

/// Decode under the shared allocation/dimension bounds; `None` when the blob
/// is empty, oversized, undecodable or would exceed the pixel budget.
fn strict_image(bytes: &[u8]) -> Option<image::DynamicImage> {
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
    Some(image)
}

fn rgba_digest(image: &image::DynamicImage) -> String {
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
    format!("image-v1:{}", hex::encode(digest.finalize()))
}

fn dhash_of(image: &image::DynamicImage) -> String {
    dhash_from_luma(&image.to_luma8())
}

/// 8×8 邻域 dHash。只作接力改写识别，不入库、不写日志。
pub(crate) fn perceptual_dhash(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_DHASH_BYTES {
        return None;
    }
    let image = image::load_from_memory(bytes).ok()?.to_luma8();
    Some(dhash_from_luma(&image))
}

fn dhash_from_luma(image: &image::GrayImage) -> String {
    let small = image::imageops::resize(image, 9, 8, image::imageops::FilterType::Triangle);
    let mut bits: u64 = 0;
    for y in 0..8 {
        for x in 0..8 {
            if small.get_pixel(x, y).0[0] > small.get_pixel(x + 1, y).0[0] {
                bits |= 1 << (y * 8 + x);
            }
        }
    }
    format!("dhash:{bits:016x}")
}
