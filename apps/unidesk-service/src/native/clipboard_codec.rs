//! Bounded common-format subset of the existing macOS native pasteboard bundle.
//! Same bincode fixed-integer/zlib wire; no engine dependency, no local file URLs.
use crate::desktop_protocol::{Format, MAX_CONTENT};
use base64::{Engine, engine::general_purpose::STANDARD};
use bincode::Options;
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use image::{ImageEncoder, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use std::io::{self, Cursor, Read, Write};
pub const BUNDLE_MIME: &str = "application/x-unidesk-pasteboard-v1";
#[derive(Serialize, Deserialize)]
struct Representation {
    type_name: String,
    data: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
struct Item {
    representations: Vec<Representation>,
}
fn native(mime: &str) -> Option<&'static str> {
    match mime {
        "text/plain" => Some("public.utf8-plain-text"),
        "image/png" => Some("public.png"),
        "text/html" => Some("public.html"),
        "text/rtf" => Some("public.rtf"),
        _ => None,
    }
}
fn common(native: &str) -> Option<&'static str> {
    match native {
        "public.utf8-plain-text" => Some("text/plain"),
        "public.png" => Some("image/png"),
        "public.html" => Some("text/html"),
        "public.rtf" => Some("text/rtf"),
        _ => None,
    }
}
fn codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit((MAX_CONTENT + 65536) as u64)
        .reject_trailing_bytes()
}
pub fn encode(formats: &[Format]) -> Result<(String, Vec<u8>), ()> {
    if formats.is_empty() || formats.len() > 4 {
        return Err(());
    }
    let mut total = 0usize;
    let mut representations = Vec::new();
    for format in formats {
        let name = native(&format.mime).ok_or(())?;
        if representations
            .iter()
            .any(|r: &Representation| r.type_name == name)
        {
            return Err(());
        }
        if format.data_base64.len() > MAX_CONTENT * 4 / 3 + 4 {
            return Err(());
        }
        let data = STANDARD.decode(&format.data_base64).map_err(|_| ())?;
        total = total.saturating_add(data.len());
        if total > MAX_CONTENT {
            return Err(());
        }
        representations.push(Representation {
            type_name: name.into(),
            data,
        });
    }
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::fast());
    codec()
        .serialize_into(&mut compressor, &vec![Item { representations }])
        .map_err(|_| ())?;
    compressor.flush().map_err(|_| ())?;
    Ok((BUNDLE_MIME.into(), compressor.finish().map_err(|_| ())?))
}
pub fn decode(mime: &str, data: &[u8]) -> Result<Vec<Format>, ()> {
    if data.len() > MAX_CONTENT + 65536 {
        return Err(());
    }
    if mime != BUNDLE_MIME {
        native(mime).ok_or(())?;
        if data.len() > MAX_CONTENT {
            return Err(());
        }
        return Ok(vec![Format {
            mime: mime.into(),
            data_base64: STANDARD.encode(data),
        }]);
    }
    let mut bytes = Vec::new();
    ZlibDecoder::new(data)
        .take((MAX_CONTENT + 65537) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() > MAX_CONTENT + 65536 {
        return Err(());
    }
    let items: Vec<Item> = codec().deserialize(&bytes).map_err(|_| ())?;
    // Multiple native items cannot be applied atomically with the Windows bridge.
    if items.len() != 1 {
        return Err(());
    }
    let mut output = Vec::new();
    let mut size = 0;
    if items[0].representations.len() > 512 {
        return Err(());
    }
    for rep in &items[0].representations {
        // Do not silently turn a file clipboard into its path text fallback.
        if matches!(
            rep.type_name.as_str(),
            "public.file-url"
                | "NSFilenamesPboardType"
                | "com.apple.pasteboard.promised-file-url"
                | "com.apple.pasteboard.promised-file-content-type"
                | "NSFilesPromisePboardType"
        ) {
            return Err(());
        }
        if let Some(mime) = common(&rep.type_name) {
            if output.iter().any(|r: &Format| r.mime == mime) {
                return Err(());
            }
            size += rep.data.len();
            if size > MAX_CONTENT {
                return Err(());
            }
            output.push(Format {
                mime: mime.into(),
                data_base64: STANDARD.encode(&rep.data),
            });
        }
    }
    // macOS preserves native items and often exposes only public.tiff. The
    // snapshot's derived PNG is not appended to that native bundle. Keep an
    // existing PNG verbatim; otherwise produce one bounded Windows rendition.
    if !output.iter().any(|r| r.mime == "image/png") {
        if let Some(tiff) = items[0]
            .representations
            .iter()
            .find(|r| r.type_name == "public.tiff")
        {
            let png = tiff_to_png(&tiff.data, MAX_CONTENT.saturating_sub(size))?;
            output.push(Format {
                mime: "image/png".into(),
                data_base64: STANDARD.encode(png),
            });
        }
    }
    if output.is_empty() {
        Err(())
    } else {
        Ok(output)
    }
}

fn tiff_to_png(data: &[u8], max_output: usize) -> Result<Vec<u8>, ()> {
    if max_output == 0 {
        return Err(());
    }
    let (width, height) = ImageReader::with_format(Cursor::new(data), ImageFormat::Tiff)
        .into_dimensions()
        .map_err(|_| ())?;
    // Validate pixel count before allocating a decoded surface. Encoded TIFF
    // and zlib/bincode input have already passed the common 1 MiB input budget.
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 4_194_304 {
        return Err(());
    }
    let mut reader = ImageReader::with_format(Cursor::new(data), ImageFormat::Tiff);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| ())?.to_rgba8();
    let mut output = BoundedOutput {
        bytes: Vec::new(),
        max: max_output,
    };
    image::codecs::png::PngEncoder::new(&mut output)
        .write_image(
            decoded.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| ())?;
    Ok(output.bytes)
}
struct BoundedOutput {
    bytes: Vec<u8>,
    max: usize,
}
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("clipboard PNG exceeds limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
