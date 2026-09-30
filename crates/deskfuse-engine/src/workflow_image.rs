//! Pure-Rust image inspection and transformation used by workflow workers.
//! No pasteboard, filesystem, or host-framework operation belongs here.
use std::io::Cursor;

use deskfuse_core::workflow::ImageFormat;
use image::{DynamicImage, GenericImageView, ImageReader, imageops::FilterType};

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
    let image = input.decode().map_err(|_| "无法解码该图片")?;
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
        ImageFormat::Heic => return Err("当前公共图片编码器不支持 HEIC".into()),
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
