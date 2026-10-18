//! Pure-Rust image inspection and transformation used by workflow workers.
//! No pasteboard, filesystem, or host-framework operation belongs here.
use std::io::Cursor;

use deskfuse_core::workflow::ImageFormat;
use image::{DynamicImage, GenericImageView, ImageReader, imageops::FilterType};

// HEIC is a system codec on macOS rather than a codec shipped by the common
// `image` crate. Keep the common path portable, but let the macOS adapter
// provide the missing decode/encode operations so HEIC still enters the same
// workflow pipeline as PNG/JPEG/WebP.
#[cfg(target_os = "macos")]
use deskfuse_platform_macos::image_processor as native_image_processor;

pub const MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;

#[derive(Clone, Debug)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub format: Option<ImageFormat>,
}
#[derive(Clone, Debug)]
pub struct ProcessedImage {
    pub data: Vec<u8>,
    pub tiff: Vec<u8>,
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
    pub format: ImageFormat,
}

fn reader(bytes: &[u8]) -> Result<ImageReader<Cursor<&[u8]>>, String> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err("图片为空或超过 128 MiB 限制".into());
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "无法识别图片格式")?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_IMAGE_BYTES as u64);
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    reader.limits(limits);
    Ok(reader)
}
fn decode(bytes: &[u8]) -> Result<(DynamicImage, Option<ImageFormat>), String> {
    let input = reader(bytes)?;
    let format = input.format().and_then(|format| match format {
        image::ImageFormat::Png => Some(ImageFormat::Png),
        image::ImageFormat::Jpeg => Some(ImageFormat::Jpeg),
        image::ImageFormat::WebP => Some(ImageFormat::Webp),
        image::ImageFormat::Tiff => None,
        _ => None,
    });
    let image = match input.decode() {
        Ok(image) => image,
        Err(_) => {
            #[cfg(target_os = "macos")]
            {
                let native = native_image_processor::decode_image(bytes)?;
                let detected = native_image_processor::inspect_image(bytes)?.format;
                let rgba = image::RgbaImage::from_raw(native.width, native.height, native.rgba)
                    .ok_or("图片像素数据长度不正确")?;
                return Ok((DynamicImage::ImageRgba8(rgba), detected.or(format)));
            }
            #[cfg(not(target_os = "macos"))]
            return Err("无法解码该图片".into());
        }
    };
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("图片尺寸超出限制（单边最多 16384，解码像素最多 32 Mi）".into());
    }
    Ok((image, format))
}

pub fn inspect_image(bytes: &[u8]) -> Result<ImageInfo, String> {
    let (image, format) = decode(bytes)?;
    let has_alpha = image.to_rgba8().pixels().any(|pixel| pixel[3] != 255);
    Ok(ImageInfo {
        width: image.width(),
        height: image.height(),
        has_alpha,
        format,
    })
}

fn encode(image: &DynamicImage, format: ImageFormat, quality: u8) -> Result<Vec<u8>, String> {
    if !(1..=100).contains(&quality) {
        return Err("编码质量必须为 1–100".into());
    }
    let mut output = Cursor::new(Vec::new());
    match format {
        ImageFormat::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality)
                .encode_image(image)
                .map_err(|_| "图片编码失败")?
        }
        ImageFormat::Png => image
            .write_to(&mut output, image::ImageFormat::Png)
            .map_err(|_| "图片编码失败")?,
        ImageFormat::Webp => image
            .write_to(&mut output, image::ImageFormat::WebP)
            .map_err(|_| "图片编码失败")?,
        ImageFormat::Heic => {
            #[cfg(target_os = "macos")]
            {
                let rgba = image.to_rgba8();
                let native = native_image_processor::ImagePixels {
                    width: rgba.width(),
                    height: rgba.height(),
                    rgba: rgba.into_raw(),
                };
                return native_image_processor::encode_pixels(&native, format, quality);
            }
            #[cfg(not(target_os = "macos"))]
            return Err("当前平台不支持 HEIC 编码".into());
        }
    }
    let bytes = output.into_inner();
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err("编码结果为空或超过 128 MiB 限制".into());
    }
    Ok(bytes)
}

pub(crate) fn bounded_thumbnail(image: DynamicImage, max_edge: u32) -> DynamicImage {
    if image.width() <= max_edge && image.height() <= max_edge {
        image
    } else {
        image.thumbnail(max_edge, max_edge)
    }
}

pub fn encode_image(
    bytes: &[u8],
    format: ImageFormat,
    quality: u8,
    max_edge: Option<u32>,
) -> Result<ProcessedImage, String> {
    let (mut image, _) = decode(bytes)?;
    if let Some(edge) = max_edge {
        if edge == 0 || edge > MAX_DIMENSION {
            return Err("最长边必须为 1–16384".into());
        }
        image = bounded_thumbnail(image, edge);
    }
    let data = encode(&image, format, quality)?;
    processed_representation(data, format)
}

pub fn processed_representation(
    data: Vec<u8>,
    format: ImageFormat,
) -> Result<ProcessedImage, String> {
    let (image, _) = decode(&data)?;
    let tiff = encode_tiff(&image)?;
    Ok(ProcessedImage {
        data,
        tiff,
        width: image.width(),
        height: image.height(),
        format,
    })
}

pub fn preview_png(bytes: &[u8], max_edge: u32) -> Result<Vec<u8>, String> {
    if max_edge == 0 || max_edge > MAX_DIMENSION {
        return Err("预览最长边必须为 1–16384".into());
    }
    let (image, _) = decode(bytes)?;
    encode(&bounded_thumbnail(image, max_edge), ImageFormat::Png, 100)
}

fn encode_tiff(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Tiff)
        .map_err(|_| "图片编码失败")?;
    Ok(output.into_inner())
}

#[allow(dead_code)]
pub fn tiff_representation(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let (image, _) = decode(bytes)?;
    encode_tiff(&image)
}

#[allow(dead_code)]
pub fn resize_image(
    bytes: &[u8],
    max_edge: Option<u32>,
    scale_percent: Option<u32>,
) -> Result<Vec<u8>, String> {
    if max_edge.is_some() == scale_percent.is_some() {
        return Err("请选择最长边或缩放比例".into());
    }
    let (image, _) = decode(bytes)?;
    let edge = max_edge.unwrap_or_else(|| {
        ((u64::from(image.width().max(image.height())) * u64::from(scale_percent.unwrap_or(100)))
            / 100) as u32
    });
    if edge == 0 || edge > MAX_DIMENSION {
        return Err("最长边必须为 1–16384".into());
    }
    encode(
        &image.resize(edge, edge, FilterType::Lanczos3),
        ImageFormat::Png,
        100,
    )
}

#[cfg(target_os = "macos")]
#[test]
fn macos_heic_round_trip_uses_workflow_codec_boundary() {
    let source = native_image_processor::ImagePixels {
        width: 2,
        height: 2,
        rgba: vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    };
    let heic = native_image_processor::encode_pixels(&source, ImageFormat::Heic, 80)
        .expect("macOS ImageIO should encode HEIC");
    let info = inspect_image(&heic).expect("workflow inspection should accept HEIC");
    assert_eq!(info.format, Some(ImageFormat::Heic));

    let jpeg = encode_image(&heic, ImageFormat::Jpeg, 80, None)
        .expect("workflow conversion should decode HEIC")
        .data;
    assert!(!jpeg.is_empty());
}
