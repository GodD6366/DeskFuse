//! Bounded, pure conversion of documented Windows clipboard formats.
#![cfg_attr(not(windows), allow(dead_code))]

use deskfuse_platform::{PlatformError, PlatformResult};

pub(crate) const MAX_FORMAT_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_FILES: usize = 4096;
pub(crate) const MAX_PATH_UNITS: usize = 32767;

pub(crate) fn encode_utf16(text: &str) -> PlatformResult<Vec<u8>> {
    if text.contains('\0') || text.len() > MAX_FORMAT_BYTES / 2 - 1 {
        return Err(PlatformError::PasteboardWriteFailed);
    }
    Ok(text
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect())
}

pub(crate) fn decode_utf16(bytes: &[u8]) -> PlatformResult<String> {
    if bytes.len() > MAX_FORMAT_BYTES || !bytes.len().is_multiple_of(2) {
        return Err(PlatformError::PasteboardReadFailed);
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .ok_or(PlatformError::PasteboardReadFailed)?;
    String::from_utf16(&units[..end]).map_err(|_| PlatformError::PasteboardReadFailed)
}

pub(crate) fn validate_png(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_FORMAT_BYTES && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

/// Encode a UTF-8 HTML fragment, with byte (not character) offsets.
pub(crate) fn encode_html(fragment: &[u8]) -> PlatformResult<Vec<u8>> {
    const PREFIX: &str = "<html><body><!--StartFragment-->";
    const SUFFIX: &str = "<!--EndFragment--></body></html>";
    if fragment.len() > MAX_FORMAT_BYTES - 512
        || fragment.contains(&0)
        || std::str::from_utf8(fragment).is_err()
    {
        return Err(PlatformError::PasteboardWriteFailed);
    }
    let header = |start: usize, end: usize, fs: usize, fe: usize| {
        format!(
            "Version:1.0\r\nStartHTML:{start:010}\r\nEndHTML:{end:010}\r\nStartFragment:{fs:010}\r\nEndFragment:{fe:010}\r\n"
        )
    };
    let start = header(0, 0, 0, 0).len();
    let fs = start + PREFIX.len();
    let fe = fs + fragment.len();
    let end = fe + SUFFIX.len();
    let mut result = header(start, end, fs, fe).into_bytes();
    result.extend_from_slice(PREFIX.as_bytes());
    result.extend_from_slice(fragment);
    result.extend_from_slice(SUFFIX.as_bytes());
    result.push(0);
    Ok(result)
}

/// Read the bounded ASCII header without scanning HTML fragment lines that
/// happen to resemble offset fields. Context and fragment offsets delimit it.
fn html_header(bytes: &[u8]) -> PlatformResult<([i64; 4], usize)> {
    let invalid = || PlatformError::PasteboardReadFailed;
    let mut offsets = [None; 4];
    let mut cursor = 0;
    let maximum = bytes.len().min(4096);
    while cursor < maximum {
        let boundary = [offsets[0], offsets[2]]
            .into_iter()
            .flatten()
            .filter_map(|offset| usize::try_from(offset).ok())
            .min()
            .unwrap_or(maximum)
            .min(maximum);
        if cursor >= boundary {
            break;
        }
        let Some(length) = bytes[cursor..boundary]
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
        else {
            break;
        };
        let line = &bytes[cursor..cursor + length];
        let mut next = cursor + length + 1;
        if bytes[cursor + length] == b'\r' && next < boundary && bytes[next] == b'\n' {
            next += 1;
        }
        let names: [&[u8]; 4] = [
            b"StartHTML:",
            b"EndHTML:",
            b"StartFragment:",
            b"EndFragment:",
        ];
        if let Some((index, tail)) = names
            .iter()
            .enumerate()
            .find_map(|(index, name)| line.strip_prefix(*name).map(|tail| (index, tail)))
        {
            if offsets[index].is_some() {
                return Err(invalid());
            }
            let value = std::str::from_utf8(tail)
                .map_err(|_| invalid())?
                .trim_matches([' ', '\t']);
            if value != "-1"
                && (value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()))
            {
                return Err(invalid());
            }
            let value = value.parse::<i64>().map_err(|_| invalid())?;
            if value < 0 && index >= 2 {
                return Err(invalid());
            }
            offsets[index] = Some(value);
        } else if !line.is_empty()
            && ![
                b"Version:".as_slice(),
                b"SourceURL:",
                b"StartSelection:",
                b"EndSelection:",
            ]
            .iter()
            .any(|name| line.starts_with(name))
        {
            break;
        }
        cursor = next;
    }
    let [html_start, html_end, start, end] = offsets;
    Ok((
        [
            html_start.ok_or_else(invalid)?,
            html_end.ok_or_else(invalid)?,
            start.ok_or_else(invalid)?,
            end.ok_or_else(invalid)?,
        ],
        cursor,
    ))
}

pub(crate) fn decode_html(bytes: &[u8]) -> PlatformResult<Vec<u8>> {
    let invalid = || PlatformError::PasteboardReadFailed;
    if bytes.len() > MAX_FORMAT_BYTES {
        return Err(invalid());
    }
    let ([html_start, html_end, start, end], header_end) = html_header(bytes)?;
    let start = usize::try_from(start).map_err(|_| invalid())?;
    let end = usize::try_from(end).map_err(|_| invalid())?;
    if start < header_end || start > end || end > bytes.len() {
        return Err(invalid());
    }
    // The documented -1/-1 pair means there is no surrounding HTML context;
    // absolute byte offsets of the fragment still remain mandatory and bounded.
    match (html_start, html_end) {
        (-1, -1) => {}
        (html_start, html_end) if html_start >= 0 && html_end >= 0 => {
            let html_start = usize::try_from(html_start).map_err(|_| invalid())?;
            let html_end = usize::try_from(html_end).map_err(|_| invalid())?;
            if html_start < header_end
                || html_start > start
                || end > html_end
                || html_end > bytes.len()
            {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    let fragment = bytes.get(start..end).ok_or_else(invalid)?;
    if start == 0 || fragment.contains(&0) || std::str::from_utf8(fragment).is_err() {
        return Err(invalid());
    }
    Ok(fragment.to_vec())
}

/// DROPFILES has a fixed 20-byte Win32 layout, followed by UTF-16 `MULTI_SZ`.
pub(crate) fn encode_hdrop(paths: &[String]) -> PlatformResult<Vec<u8>> {
    if paths.is_empty() || paths.len() > MAX_FILES {
        return Err(PlatformError::PasteboardWriteFailed);
    }
    let mut bytes = vec![0; 20];
    bytes[..4].copy_from_slice(&20_u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
    for path in paths {
        if !is_local_drive_path(path) || path.encode_utf16().count() > MAX_PATH_UNITS {
            return Err(PlatformError::PasteboardWriteFailed);
        }
        let encoded = encode_utf16(path)?;
        if bytes.len() + encoded.len() + 2 > MAX_FORMAT_BYTES {
            return Err(PlatformError::PasteboardWriteFailed);
        }
        bytes.extend_from_slice(&encoded);
    }
    bytes.extend_from_slice(&[0, 0]);
    Ok(bytes)
}

pub(crate) fn is_local_drive_path(path: &str) -> bool {
    let path = path.strip_prefix("\\\\?\\").unwrap_or(path);
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
        && !bytes.contains(&0)
}

pub(crate) fn decode_hdrop(bytes: &[u8]) -> PlatformResult<Vec<String>> {
    let fail = || PlatformError::PasteboardFileUnavailable;
    if bytes.len() < 24 || bytes.len() > MAX_FORMAT_BYTES {
        return Err(fail());
    }
    let offset = u32::from_le_bytes(bytes[..4].try_into().map_err(|_| fail())?) as usize;
    let wide = u32::from_le_bytes(bytes[16..20].try_into().map_err(|_| fail())?);
    if offset < 20 || !offset.is_multiple_of(2) || wide == 0 {
        return Err(fail());
    }
    let data = bytes.get(offset..).ok_or_else(fail)?;
    if !data.len().is_multiple_of(2) {
        return Err(fail());
    }
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    let mut paths = Vec::new();
    let mut start = 0;
    while start < units.len() {
        let length = units[start..]
            .iter()
            .position(|unit| *unit == 0)
            .ok_or_else(fail)?;
        if length == 0 {
            return if paths.is_empty() {
                Err(fail())
            } else {
                Ok(paths)
            };
        }
        if length > MAX_PATH_UNITS || paths.len() >= MAX_FILES {
            return Err(fail());
        }
        let path = String::from_utf16(&units[start..start + length]).map_err(|_| fail())?;
        if !is_local_drive_path(&path) {
            return Err(fail());
        }
        paths.push(path);
        start += length + 1;
    }
    Err(fail())
}

/// Convert a complete static PNG into a top-down, sRGB DIBV5. APNG is rejected
/// rather than silently choosing a frame. ICC/text metadata is ignored, never
/// opened or interpreted as an external profile. The original PNG is retained
/// separately by the caller; PNG plus DIB must fit the shared write budget.
pub(crate) fn png_to_dibv5(bytes: &[u8]) -> PlatformResult<Vec<u8>> {
    let invalid = || PlatformError::PasteboardWriteFailed;
    if !validate_png(bytes) {
        return Err(invalid());
    }
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(bytes),
        png::Limits {
            bytes: MAX_FORMAT_BYTES,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    let mut reader = decoder.read_info().map_err(|_| invalid())?;
    if reader.info().animation_control.is_some() {
        return Err(PlatformError::Unsupported {
            operation: "Windows clipboard animated PNG",
        });
    }
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0 || height == 0 || width > 32768 || height > 32768 {
        return Err(invalid());
    }
    let rgba_len = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(invalid)?;
    let dib_len = rgba_len.checked_add(124).ok_or_else(invalid)?;
    if dib_len > MAX_FORMAT_BYTES || bytes.len() > MAX_FORMAT_BYTES - dib_len {
        return Err(invalid());
    }
    let output_len = reader.output_buffer_size().ok_or_else(invalid)?;
    if output_len > rgba_len {
        return Err(invalid());
    }
    let mut pixel_buffer = vec![0; output_len];
    let info = reader
        .next_frame(&mut pixel_buffer)
        .map_err(|_| invalid())?;
    if info.width != width
        || info.height != height
        || info.bit_depth != png::BitDepth::Eight
        || info.buffer_size() != output_len
    {
        return Err(invalid());
    }
    reader.finish().map_err(|_| invalid())?;
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(invalid()),
    };
    if pixel_buffer.len() / channels != rgba_len / 4 || !pixel_buffer.len().is_multiple_of(channels)
    {
        return Err(invalid());
    }
    let mut dib = vec![0; dib_len];
    let negative_height = -i32::try_from(height).map_err(|_| invalid())?;
    dib[..4].copy_from_slice(&124_u32.to_le_bytes());
    dib[4..8].copy_from_slice(&width.to_le_bytes());
    dib[8..12].copy_from_slice(&negative_height.to_le_bytes());
    dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
    dib[14..16].copy_from_slice(&32_u16.to_le_bytes());
    dib[16..20].copy_from_slice(&3_u32.to_le_bytes());
    dib[20..24].copy_from_slice(
        &u32::try_from(rgba_len)
            .map_err(|_| invalid())?
            .to_le_bytes(),
    );
    for (offset, value) in [
        (40, 0x00ff_0000_u32),
        (44, 0x0000_ff00),
        (48, 0x0000_00ff),
        (52, 0xff00_0000),
        (56, 0x7352_4742),
        (108, 4),
    ] {
        dib[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (pixel, target) in pixel_buffer
        .chunks_exact(channels)
        .zip(dib[124..].as_chunks_mut::<4>().0.iter_mut())
    {
        let bgra = match channels {
            1 => [pixel[0], pixel[0], pixel[0], 255],
            2 => [pixel[0], pixel[0], pixel[0], pixel[1]],
            3 => [pixel[2], pixel[1], pixel[0], 255],
            4 => [pixel[2], pixel[1], pixel[0], pixel[3]],
            _ => unreachable!("validated channel count"),
        };
        target.copy_from_slice(&bgra);
    }
    Ok(dib)
}

/// Validate all native entries before allocating or clearing the clipboard.
pub(crate) fn validate_native_entries(entries: &[(u32, Vec<u8>)]) -> PlatformResult<()> {
    if entries.is_empty() {
        return Err(PlatformError::PasteboardWriteFailed);
    }
    let mut remaining = MAX_FORMAT_BYTES;
    let mut formats = std::collections::BTreeSet::new();
    for (format, bytes) in entries {
        if bytes.is_empty() || !formats.insert(*format) {
            return Err(PlatformError::PasteboardWriteFailed);
        }
        remaining = remaining
            .checked_sub(bytes.len())
            .ok_or(PlatformError::PasteboardWriteFailed)?;
    }
    Ok(())
}

/// Decode packed Windows DIB pixels and encode a bounded RGBA PNG. No ICC
/// profile paths are opened and no operating-system image decoder is invoked.
pub(crate) fn dib_to_png(bytes: &[u8]) -> PlatformResult<Vec<u8>> {
    let image = decode_dib(bytes)?;
    let mut output = BoundedPng(Vec::new());
    {
        let mut encoder = png::Encoder::new(&mut output, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|_| PlatformError::PasteboardReadFailed)?;
        writer
            .write_image_data(&image.rgba)
            .map_err(|_| PlatformError::PasteboardReadFailed)?;
        writer
            .finish()
            .map_err(|_| PlatformError::PasteboardReadFailed)?;
    }
    Ok(output.0)
}

struct BoundedPng(Vec<u8>);
impl std::io::Write for BoundedPng {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_FORMAT_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("clipboard PNG exceeds size limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct DibImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn dib_u32(bytes: &[u8], offset: usize) -> PlatformResult<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(PlatformError::PasteboardReadFailed)?
            .try_into()
            .map_err(|_| PlatformError::PasteboardReadFailed)?,
    ))
}

fn validate_dib_color_space(bytes: &[u8], header: usize) -> PlatformResult<()> {
    if header >= 108 {
        let color_space = dib_u32(bytes, 56)?;
        if !(matches!(color_space, 0x7352_4742 | 0x5769_6e20)
            || (color_space == 0 && bytes[60..108].iter().all(|byte| *byte == 0)))
        {
            return Err(PlatformError::Unsupported {
                operation: "Windows DIB color profile",
            });
        }
    }
    if header == 124 && (dib_u32(bytes, 112)? != 0 || dib_u32(bytes, 116)? != 0) {
        return Err(PlatformError::Unsupported {
            operation: "Windows DIB color profile",
        });
    }
    Ok(())
}

/// Supports 24/32-bit `BI_RGB` and canonical 32-bit `BI_BITFIELDS`. All dimensions,
/// headers, palettes, masks, profiles, strides and pixel extents are validated
/// before allocation. `BI_RGB`'s unused high byte is never interpreted as alpha.
fn decode_dib(bytes: &[u8]) -> PlatformResult<DibImage> {
    const MAX_DIMENSION: usize = 32768;
    let invalid = || PlatformError::PasteboardReadFailed;
    let unsupported = || PlatformError::Unsupported {
        operation: "Windows DIB pixel layout or color profile",
    };
    if bytes.len() < 40 || bytes.len() > MAX_FORMAT_BYTES {
        return Err(invalid());
    }
    let header = usize::try_from(dib_u32(bytes, 0)?).map_err(|_| invalid())?;
    if !matches!(header, 40 | 108 | 124) {
        return Err(unsupported());
    }
    if bytes.len() < header {
        return Err(invalid());
    }
    let width = i32::from_le_bytes(bytes[4..8].try_into().map_err(|_| invalid())?);
    let signed_height = i32::from_le_bytes(bytes[8..12].try_into().map_err(|_| invalid())?);
    let height = signed_height.checked_abs().ok_or_else(invalid)?;
    let width = usize::try_from(width).map_err(|_| invalid())?;
    let height = usize::try_from(height).map_err(|_| invalid())?;
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(invalid());
    }
    if u16::from_le_bytes([bytes[12], bytes[13]]) != 1 {
        return Err(invalid());
    }
    let bits = u16::from_le_bytes([bytes[14], bytes[15]]);
    let compression = dib_u32(bytes, 16)?;
    if !matches!((bits, compression), (24 | 32, 0) | (32, 3)) {
        return Err(unsupported());
    }
    // These true-color layouts do not require a palette; reject optimization
    // palettes rather than guessing where their pixel data starts.
    if dib_u32(bytes, 32)? != 0 {
        return Err(unsupported());
    }
    validate_dib_color_space(bytes, header)?;
    let mut offset = header;
    let alpha = if compression == 3 {
        if dib_u32(bytes, 40)? != 0x00ff_0000
            || dib_u32(bytes, 44)? != 0x0000_ff00
            || dib_u32(bytes, 48)? != 0x0000_00ff
        {
            return Err(unsupported());
        }
        if header == 40 {
            offset = 52;
            false
        } else {
            match dib_u32(bytes, 52)? {
                0 => false,
                0xff00_0000 => true,
                _ => return Err(unsupported()),
            }
        }
    } else {
        false
    };
    let bytes_per_pixel = usize::from(bits / 8);
    let row_bytes = width.checked_mul(bytes_per_pixel).ok_or_else(invalid)?;
    let stride = row_bytes.checked_add(3).ok_or_else(invalid)? & !3;
    let image_bytes = stride.checked_mul(height).ok_or_else(invalid)?;
    let end = offset.checked_add(image_bytes).ok_or_else(invalid)?;
    let pixels = bytes.get(offset..end).ok_or_else(invalid)?;
    let declared_size = usize::try_from(dib_u32(bytes, 20)?).map_err(|_| invalid())?;
    if declared_size != 0 && (declared_size < image_bytes || declared_size > bytes.len() - offset) {
        return Err(invalid());
    }
    let rgba_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(invalid)?;
    if rgba_len > MAX_FORMAT_BYTES {
        return Err(invalid());
    }
    let mut rgba = vec![0; rgba_len];
    for y in 0..height {
        let source_y = if signed_height > 0 { height - 1 - y } else { y };
        let row = &pixels[source_y * stride..source_y * stride + row_bytes];
        for (source, destination) in row.chunks_exact(bytes_per_pixel).zip(
            rgba[y * width * 4..(y + 1) * width * 4]
                .as_chunks_mut::<4>()
                .0
                .iter_mut(),
        ) {
            destination.copy_from_slice(&[
                source[2],
                source[1],
                source[0],
                if alpha { source[3] } else { 255 },
            ]);
        }
    }
    Ok(DibImage {
        width: u32::try_from(width).map_err(|_| invalid())?,
        height: u32::try_from(height).map_err(|_| invalid())?,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_text_roundtrip_and_rejections() {
        let text = "复制 🦀\r\n";
        assert_eq!(decode_utf16(&encode_utf16(text).unwrap()).unwrap(), text);
        assert!(encode_utf16("a\0b").is_err());
        assert!(decode_utf16(&[1]).is_err());
        assert!(decode_utf16(&[1, 0]).is_err());
        assert!(decode_utf16(&[0, 0xd8, 0, 0]).is_err());
    }
    #[test]
    fn html_offsets_are_utf8_bytes() {
        let fragment = "<b>复制 🦀</b>".as_bytes();
        let data = encode_html(fragment).unwrap();
        assert_eq!(decode_html(&data).unwrap(), fragment);
        let header_like = b"<div>hello\nStartFragment:0\nworld</div>";
        assert_eq!(
            decode_html(&encode_html(header_like).unwrap()).unwrap(),
            header_like
        );
        assert!(decode_html(b"StartFragment:999999\r\nEndFragment:1000000\r\n").is_err());
        assert!(decode_html(b"StartFragment:5\r\nEndFragment:1\r\n").is_err());
    }
    #[test]
    fn hdrop_roundtrip_without_accessing_filesystem() {
        let paths = vec![
            "C:\\Users\\chang\\测试.txt".into(),
            "D:\\folder\\🦀.png".into(),
        ];
        let bytes = encode_hdrop(&paths).unwrap();
        assert_eq!(decode_hdrop(&bytes).unwrap(), paths);
        assert!(decode_hdrop(&bytes[..bytes.len() - 2]).is_err());
        let mut wrong = bytes.clone();
        wrong[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_hdrop(&wrong).is_err());
        wrong = bytes;
        wrong[16..20].fill(0);
        assert!(decode_hdrop(&wrong).is_err());
    }
    #[test]
    fn png_rejects_non_png_bytes() {
        assert!(!validate_png(b"not an image"));
        assert!(validate_png(b"\x89PNG\r\n\x1a\n"));
    }
    #[test]
    fn paths_reject_remote_relative_and_device_namespaces() {
        for path in [
            "relative.txt",
            "C:relative",
            "\\\\host\\share\\file",
            "\\\\.\\PhysicalDrive0",
            "\\\\?\\UNC\\host\\share",
            "C:\\nul\0.txt",
        ] {
            assert!(encode_hdrop(&[path.into()]).is_err(), "{path:?}");
        }
        assert!(encode_hdrop(&[]).is_err());
        assert!(is_local_drive_path("\\\\?\\C:\\long-path.txt"));
    }
    fn dib_fixture(width: i32, height: i32, bits: u16, v5: bool, pixels: &[u8]) -> Vec<u8> {
        let header: u32 = if v5 { 124 } else { 40 };
        let mut bytes = vec![0; header as usize];
        bytes[..4].copy_from_slice(&header.to_le_bytes());
        bytes[4..8].copy_from_slice(&width.to_le_bytes());
        bytes[8..12].copy_from_slice(&height.to_le_bytes());
        bytes[12..14].copy_from_slice(&1_u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&bits.to_le_bytes());
        if v5 {
            bytes[16..20].copy_from_slice(&3_u32.to_le_bytes());
            for (offset, mask) in [
                (40, 0x00ff_0000_u32),
                (44, 0x0000_ff00),
                (48, 0x0000_00ff),
                (52, 0xff00_0000),
                (56, 0x7352_4742),
            ] {
                bytes[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
            }
        }
        bytes.extend_from_slice(pixels);
        bytes
    }
    #[test]
    fn dib_24_bottom_up_and_top_down_honor_stride() {
        // One pixel row is three bytes plus one DWORD-alignment padding byte.
        let bottom_up = dib_fixture(1, 2, 24, false, &[255, 0, 0, 99, 0, 0, 255, 88]);
        let top_down = dib_fixture(1, -2, 24, false, &[0, 0, 255, 88, 255, 0, 0, 99]);
        let expected = [255, 0, 0, 255, 0, 0, 255, 255];
        assert_eq!(decode_dib(&bottom_up).unwrap().rgba, expected);
        assert_eq!(decode_dib(&top_down).unwrap().rgba, expected);
        let encoded = dib_to_png(&bottom_up).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(encoded))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!((info.width, info.height), (1, 2));
        assert_eq!(decoded, expected);
    }
    #[test]
    fn dib_32_rgb_ignores_unused_alpha_v5_preserves_declared_alpha() {
        let rgb = dib_fixture(1, -1, 32, false, &[30, 20, 10, 0]);
        assert_eq!(decode_dib(&rgb).unwrap().rgba, [10, 20, 30, 255]);
        let v5 = dib_fixture(1, -1, 32, true, &[30, 20, 10, 128]);
        assert_eq!(decode_dib(&v5).unwrap().rgba, [10, 20, 30, 128]);
    }
    #[test]
    fn dib_rejects_truncation_overflow_invalid_headers_masks_and_profiles() {
        let valid = dib_fixture(1, 1, 24, false, &[1, 2, 3, 0]);
        for end in 0..valid.len() {
            assert!(decode_dib(&valid[..end]).is_err());
        }
        for (width, height) in [
            (0, 1),
            (-1, 1),
            (1, 0),
            (1, i32::MIN),
            (i32::MAX, i32::MAX),
            (32768, 32768),
        ] {
            assert!(decode_dib(&dib_fixture(width, height, 32, false, &[0; 4])).is_err());
        }
        for (offset, value) in [(0, u32::MAX), (20, u32::MAX), (32, 1)] {
            let mut bad = valid.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode_dib(&bad).is_err());
        }
        for (offset, value) in [
            (40, 0xff00_u32),
            (52, 0x00ff_0000),
            (56, 0x4c49_4e4b),
            (112, 124),
            (116, u32::MAX),
        ] {
            let mut bad = dib_fixture(1, 1, 32, true, &[0; 4]);
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode_dib(&bad).is_err());
        }
    }
    fn png_fixture(
        color: png::ColorType,
        width: u32,
        height: u32,
        pixels: &[u8],
        animated: bool,
    ) -> Vec<u8> {
        let mut output = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut output, width, height);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            if animated {
                encoder.set_animated(1, 0).unwrap();
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixels).unwrap();
            writer.finish().unwrap();
        }
        output
    }
    #[test]
    fn png_to_dibv5_preserves_alpha_and_top_down_order() {
        let rgba = [255, 0, 0, 128, 0, 0, 255, 255];
        let png = png_fixture(png::ColorType::Rgba, 1, 2, &rgba, false);
        let dib = png_to_dibv5(&png).unwrap();
        assert_eq!(i32::from_le_bytes(dib[8..12].try_into().unwrap()), -2);
        assert_eq!(&dib[124..], &[0, 0, 255, 128, 255, 0, 0, 255]);
        assert_eq!(decode_dib(&dib).unwrap().rgba, rgba);
        assert!(validate_native_entries(&[(17, dib), (0xc000, png)]).is_ok());
    }
    #[test]
    fn png_to_dibv5_expands_gray_gray_alpha_and_rgb() {
        for (color, pixels, expected) in [
            (png::ColorType::Grayscale, vec![80], [80, 80, 80, 255]),
            (
                png::ColorType::GrayscaleAlpha,
                vec![80, 40],
                [80, 80, 80, 40],
            ),
            (png::ColorType::Rgb, vec![10, 20, 30], [10, 20, 30, 255]),
        ] {
            let png = png_fixture(color, 1, 1, &pixels, false);
            assert_eq!(
                decode_dib(&png_to_dibv5(&png).unwrap()).unwrap().rgba,
                expected
            );
        }
    }
    #[test]
    fn png_to_dibv5_rejects_animation_truncation_and_crc_corruption() {
        let png = png_fixture(png::ColorType::Rgba, 1, 1, &[10, 20, 30, 40], false);
        for length in 0..png.len() {
            assert!(png_to_dibv5(&png[..length]).is_err(), "length={length}");
        }
        let apng = png_fixture(png::ColorType::Rgba, 1, 1, &[10, 20, 30, 40], true);
        assert!(matches!(
            png_to_dibv5(&apng),
            Err(PlatformError::Unsupported { .. })
        ));
        let mut corrupt = png;
        corrupt[29] ^= 1;
        assert!(png_to_dibv5(&corrupt).is_err());
    }
    #[test]
    fn native_entries_reject_duplicates_empty_and_total_over_budget() {
        assert!(validate_native_entries(&[]).is_err());
        assert!(validate_native_entries(&[(1, vec![])]).is_err());
        assert!(validate_native_entries(&[(1, vec![1]), (1, vec![2])]).is_err());
        assert!(validate_native_entries(&[(1, vec![0; MAX_FORMAT_BYTES]), (2, vec![1])]).is_err());
    }
    fn html_without_context(fragment: &[u8]) -> Vec<u8> {
        let header = |start: usize, end: usize| {
            format!(
                "Version:1.0\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:{start:010}\r\nEndFragment:{end:010}\r\n"
            )
        };
        let start = header(0, 0).len();
        let mut bytes = header(start, start + fragment.len()).into_bytes();
        bytes.extend_from_slice(fragment);
        bytes
    }
    #[test]
    fn html_accepts_no_context_with_markup_or_plain_utf8_fragment() {
        for fragment in ["<b>复制 🦀</b>", "复制 🦀", "StartFragment:0\n正文"] {
            let bytes = html_without_context(fragment.as_bytes());
            assert_eq!(decode_html(&bytes).unwrap(), fragment.as_bytes());
        }
    }
    #[test]
    fn html_rejects_mixed_context_sentinels_bad_fragment_offsets_and_utf8() {
        let valid = String::from_utf8(html_without_context("<b>复制</b>".as_bytes())).unwrap();
        assert!(decode_html(valid.replace("StartHTML:-1", "StartHTML:00").as_bytes()).is_err());
        assert!(decode_html(valid.replace("EndHTML:-1", "EndHTML:00").as_bytes()).is_err());
        assert!(decode_html(valid.replace("EndHTML:-1", "EndHTML:-2").as_bytes()).is_err());
        let start_line = valid
            .lines()
            .find(|line| line.starts_with("StartFragment:"))
            .unwrap();
        let end_line = valid
            .lines()
            .find(|line| line.starts_with("EndFragment:"))
            .unwrap();
        for line in [
            "StartFragment:0000000000",
            "StartFragment:9999999999",
            "StartFragment:-1",
        ] {
            assert!(decode_html(valid.replace(start_line, line).as_bytes()).is_err());
        }
        for line in [
            "EndFragment:0000000000",
            "EndFragment:9999999999",
            "EndFragment:-1",
        ] {
            assert!(decode_html(valid.replace(end_line, line).as_bytes()).is_err());
        }
        assert!(decode_html(&html_without_context(&[0xff])).is_err());
        assert!(decode_html(&html_without_context(&[0xe4, 0xb8])).is_err());
    }
}
