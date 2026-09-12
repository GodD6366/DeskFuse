//! Single-image processing boundary. All work is synchronous and must be called
//! on an engine worker, never on the native UI or input-capture thread.
//!
//! Pixels are straight-alpha RGBA in sRGB, with orientation already applied.
#![allow(unsafe_code)] // ImageIO/CoreGraphics FFI is confined to this adapter.

use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    data::CFData,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_foundation_sys::{
    base::CFRelease,
    data::{CFDataCreateMutable, CFDataRef, CFMutableDataRef},
    dictionary::CFDictionaryRef,
    string::CFStringRef,
};
use core_graphics::{
    color_space::{CGColorSpace, kCGColorSpaceSRGB},
    context::{CGContext, CGInterpolationQuality},
    data_provider::CGDataProvider,
    geometry::{CGPoint, CGRect, CGSize},
    image::CGImage,
};
use foreign_types::ForeignType;
use std::{ffi::c_void, ptr, sync::Arc};
use unidesk_core::image_workflow::ImageFormat;

pub const MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;
// At most 128 MiB per decoded RGBA buffer.
const MAX_PIXELS: usize = 32 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;

#[derive(Clone, Debug)]
pub struct ImagePixels {
    pub width: u32,
    pub height: u32,
    /// Straight alpha, row-major RGBA, sRGB.
    pub rgba: Vec<u8>,
}
impl ImagePixels {
    pub fn has_transparency(&self) -> bool {
        self.rgba.chunks_exact(4).any(|p| p[3] != 255)
    }
    pub fn smart_format(&self) -> ImageFormat {
        if self.has_transparency() {
            ImageFormat::Png
        } else {
            ImageFormat::Jpeg
        }
    }
    fn validate(&self) -> Result<(), String> {
        if byte_count(self.width, self.height)? != self.rgba.len() {
            return Err("图片像素数据长度不正确".into());
        }
        Ok(())
    }
}

/// An owned opaque Core Foundation reference from a Create/Copy function.
struct OwnedCf(CFTypeRef);
impl OwnedCf {
    fn new(value: CFTypeRef, error: &str) -> Result<Self, String> {
        if value.is_null() {
            Err(error.into())
        } else {
            Ok(Self(value))
        }
    }
}
impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: constructed only from non-null owned CF Create/Copy references.
        unsafe { CFRelease(self.0) }
    }
}
fn byte_count(w: u32, h: u32) -> Result<usize, String> {
    let pixels = (w as usize).checked_mul(h as usize).ok_or("图片尺寸过大")?;
    if w == 0 || h == 0 || w > MAX_DIMENSION || h > MAX_DIMENSION || pixels > MAX_PIXELS {
        return Err("图片尺寸超出限制（单边最多 16384，解码像素最多 32 Mi）".into());
    }
    pixels.checked_mul(4).ok_or_else(|| "图片尺寸过大".into())
}
fn key(value: CFStringRef) -> CFString {
    // SAFETY: all callers pass immortal ImageIO CFString constants.
    unsafe { CFString::wrap_under_get_rule(value) }
}
fn s_rgb() -> Result<CGColorSpace, String> {
    // SAFETY: immutable framework constant.
    unsafe { CGColorSpace::create_with_name(kCGColorSpaceSRGB) }
        .ok_or_else(|| "无法创建 sRGB 色彩空间".into())
}

/// Rejects multi-frame data before decoding. ImageIO applies EXIF orientation
/// while decoding at full resolution; no embedded thumbnail is used.
pub fn decode_image(bytes: &[u8]) -> Result<ImagePixels, String> {
    decode_bounded(bytes, None)
}

/// ImageIO receives the thumbnail bound before it materializes any decoded
/// bitmap. The source is never decoded to full-resolution RGBA for previews.
pub fn preview_png(bytes: &[u8], max_edge: u32) -> Result<Vec<u8>, String> {
    if max_edge == 0 || max_edge > MAX_DIMENSION {
        return Err("预览最长边必须为 1–16384".into());
    }
    encode_pixels(
        &decode_bounded(bytes, Some(max_edge))?,
        ImageFormat::Png,
        100,
    )
}

fn decode_bounded(bytes: &[u8], max_edge: Option<u32>) -> Result<ImagePixels, String> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err("图片为空或超过 128 MiB 限制".into());
    }
    let data = CFData::from_buffer(bytes);
    // SAFETY: immutable framework constant. Disable source bitmap caching so
    // metadata inspection cannot retain a full-size decode before thumbnailing.
    let source_options = unsafe {
        CFDictionary::from_CFType_pairs(&[(
            key(kCGImageSourceShouldCache),
            CFBoolean::false_value().as_CFType(),
        )])
    };
    // SAFETY: live CFData and options; returned source has Create ownership.
    let source = OwnedCf::new(
        unsafe {
            CGImageSourceCreateWithData(
                data.as_concrete_TypeRef(),
                source_options.as_concrete_TypeRef(),
            )
        },
        "无法解码该图片",
    )?;
    // SAFETY: source is a live CGImageSource owned above.
    if unsafe { CGImageSourceGetCount(source.0) } != 1 {
        return Err("暂不支持动图或多帧图片".into());
    }
    // SAFETY: valid source and sole image index; Copy ownership transferred below.
    let props = unsafe { CGImageSourceCopyPropertiesAtIndex(source.0, 0, ptr::null()) };
    if props.is_null() {
        return Err("无法读取图片尺寸".into());
    }
    let props = unsafe { CFDictionary::<CFString, CFType>::wrap_under_create_rule(props) };
    let dimension = |k: CFStringRef| -> Result<u32, String> {
        props
            .find(key(k))
            .and_then(|n| n.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| "图片尺寸无效".into())
    };
    // SAFETY: framework constants.
    let (width, height) = unsafe {
        (
            dimension(kCGImagePropertyPixelWidth)?,
            dimension(kCGImagePropertyPixelHeight)?,
        )
    };
    byte_count(width, height)?;
    // SAFETY: framework constants are immutable and valid CFStrings.
    let opts = unsafe {
        CFDictionary::from_CFType_pairs(&[
            (
                key(kCGImageSourceCreateThumbnailFromImageAlways),
                CFBoolean::true_value().as_CFType(),
            ),
            (
                key(kCGImageSourceCreateThumbnailWithTransform),
                CFBoolean::true_value().as_CFType(),
            ),
            (
                key(kCGImageSourceThumbnailMaxPixelSize),
                CFNumber::from(i64::from(
                    max_edge.unwrap_or(width.max(height)).min(width.max(height)),
                ))
                .as_CFType(),
            ),
        ])
    };
    // SAFETY: live source and dictionary; image has Create ownership.
    let image =
        unsafe { CGImageSourceCreateThumbnailAtIndex(source.0, 0, opts.as_concrete_TypeRef()) };
    if image.is_null() {
        return Err("图片解码失败".into());
    }
    let image = unsafe { CGImage::from_ptr(image) };
    rasterize(&image, image.width() as u32, image.height() as u32)
}

fn bitmap_context(width: u32, height: u32) -> Result<CGContext, String> {
    byte_count(width, height)?;
    let color = s_rgb()?;
    // Premultiplied-last, 32-bit big endian => byte order R,G,B,A.
    // SAFETY: dimensions checked above, null requests framework-owned storage;
    // color space outlives the call and is retained by the returned context.
    let context = unsafe {
        CGBitmapContextCreate(
            ptr::null_mut(),
            width as usize,
            height as usize,
            8,
            width as usize * 4,
            color.as_ptr(),
            0x4001,
        )
    };
    if context.is_null() {
        return Err("无法分配图片处理内存".into());
    }
    Ok(unsafe { CGContext::from_ptr(context) })
}
fn rasterize(image: &CGImage, width: u32, height: u32) -> Result<ImagePixels, String> {
    let mut context = bitmap_context(width, height)?;
    context.set_interpolation_quality(CGInterpolationQuality::CGInterpolationQualityHigh);
    context.draw_image(
        CGRect::new(
            &CGPoint::new(0., 0.),
            &CGSize::new(width as f64, height as f64),
        ),
        image,
    );
    let mut rgba = context.data().to_vec();
    for p in rgba.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        if a == 0 {
            p[..3].fill(0);
        } else if a < 255 {
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Ok(ImagePixels {
        width,
        height,
        rgba,
    })
}
fn cg_image(image: &ImagePixels, white_background: bool) -> Result<CGImage, String> {
    image.validate()?;
    let mut bytes = image.rgba.clone();
    for p in bytes.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        for c in &mut p[..3] {
            *c = ((u32::from(*c) * a + 127) / 255 + if white_background { 255 - a } else { 0 })
                as u8;
        }
        if white_background {
            p[3] = 255;
        }
    }
    let provider = CGDataProvider::from_buffer(Arc::new(bytes));
    let color = s_rgb()?;
    // SAFETY: checked image dimensions and buffer size. CGImage retains the
    // provider and color space; resulting owned reference is checked for null.
    let raw = unsafe {
        CGImageCreate(
            image.width as usize,
            image.height as usize,
            8,
            32,
            image.width as usize * 4,
            color.as_ptr(),
            0x4001,
            provider.as_ptr(),
            ptr::null(),
            true,
            0,
        )
    };
    if raw.is_null() {
        return Err("无法创建待编码图片".into());
    }
    Ok(unsafe { CGImage::from_ptr(raw) })
}

/// Resize proportionally to the supplied bounding box, never enlarge.
pub fn resize_image(
    image: &ImagePixels,
    max_edge: Option<u32>,
    scale_percent: Option<u32>,
) -> Result<ImagePixels, String> {
    image.validate()?;
    if max_edge.is_some() == scale_percent.is_some() {
        return Err("请选择最长边或缩放比例".into());
    }
    let ratio = if let Some(edge) = max_edge {
        if edge == 0 {
            return Err("最长边必须大于零".into());
        }
        (f64::from(edge) / f64::from(image.width.max(image.height))).min(1.)
    } else {
        let percent = scale_percent.unwrap_or(100);
        if percent == 0 || percent > 100 {
            return Err("缩放比例必须为 1–100%".into());
        }
        f64::from(percent) / 100.
    };
    if ratio >= 1. {
        return Ok(image.clone());
    }
    resize_exact(
        image,
        ((f64::from(image.width) * ratio).round() as u32).max(1),
        ((f64::from(image.height) * ratio).round() as u32).max(1),
    )
}
fn resize_exact(image: &ImagePixels, width: u32, height: u32) -> Result<ImagePixels, String> {
    rasterize(&cg_image(image, false)?, width, height)
}

pub fn encode_pixels(
    image: &ImagePixels,
    format: ImageFormat,
    quality: u8,
) -> Result<Vec<u8>, String> {
    image.validate()?;
    if !(1..=100).contains(&quality) {
        return Err("编码质量必须为 1–100".into());
    }
    if format == ImageFormat::Webp {
        let encoder = webp::Encoder::from_rgba(&image.rgba, image.width, image.height);
        let result = encoder
            .encode_simple(false, f32::from(quality))
            .map_err(|e| format!("WebP 编码失败：{e:?}"))?;
        if result.is_empty() {
            return Err("WebP 编码失败".into());
        }
        return Ok(result.to_vec());
    }
    encode_native(
        image,
        format.uti(),
        quality,
        matches!(format, ImageFormat::Jpeg | ImageFormat::Heic),
    )
}
/// Compatibility representation generated only from final processed pixels.
pub fn encode_tiff(image: &ImagePixels) -> Result<Vec<u8>, String> {
    encode_native(image, "public.tiff", 100, false)
}
fn encode_native(
    image: &ImagePixels,
    uti: &str,
    quality: u8,
    white: bool,
) -> Result<Vec<u8>, String> {
    let image = cg_image(image, white)?;
    // SAFETY: null allocator selects system allocator, zero permits growth.
    let data = OwnedCf::new(
        unsafe { CFDataCreateMutable(ptr::null(), 0) }.cast(),
        "无法创建编码缓冲区",
    )?;
    let kind = CFString::new(uti);
    // SAFETY: live mutable CFData and UTI string; destination has Create ownership.
    let dest = OwnedCf::new(
        unsafe {
            CGImageDestinationCreateWithData(
                data.0 as CFMutableDataRef,
                kind.as_concrete_TypeRef(),
                1,
                ptr::null(),
            )
        },
        "该系统不支持此图片格式编码",
    )?;
    let props = unsafe {
        CFDictionary::from_CFType_pairs(&[(
            key(kCGImageDestinationLossyCompressionQuality),
            CFNumber::from(f64::from(quality) / 100.).as_CFType(),
        )])
    };
    // SAFETY: all references live through finalization; destination holds one image.
    unsafe {
        CGImageDestinationAddImage(dest.0, image.as_ptr(), props.as_concrete_TypeRef());
    }
    if !unsafe { CGImageDestinationFinalize(dest.0) } {
        return Err("图片编码失败".into());
    }
    // SAFETY: mutable CFData is toll-free compatible with CFData; Get wrapper
    // retains an independent reference until bytes have been copied.
    let output = unsafe { CFData::wrap_under_get_rule(data.0 as CFDataRef) };
    if output.is_empty() || output.len() as usize > MAX_IMAGE_BYTES {
        return Err("编码结果为空或超过 128 MiB 限制".into());
    }
    Ok(output.bytes().to_vec())
}

/// Enforce the *final* byte cap after every transformation. Keeps the final
/// format and shrinks no further than min(original longest edge, 256).
/// Returns both final pixels (for compatibility pasteboard data) and encoding.
pub fn encode_to_target(
    image: &ImagePixels,
    format: ImageFormat,
    quality: u8,
    target_bytes: u64,
) -> Result<(ImagePixels, Vec<u8>), String> {
    if target_bytes == 0 {
        return Err("目标体积必须大于零".into());
    }
    let mut current = image.clone();
    let floor = image.width.max(image.height).min(256);
    for _ in 0..32 {
        let initial = encode_pixels(&current, format, quality)?;
        if initial.len() as u64 <= target_bytes {
            return Ok((current, initial));
        }
        if format != ImageFormat::Png && quality > 40 {
            let lowest = encode_pixels(&current, format, 40)?;
            if lowest.len() as u64 <= target_bytes {
                let mut best = lowest;
                let (mut lo, mut hi) = (40u8, quality);
                while lo + 1 < hi {
                    let mid = lo + (hi - lo) / 2;
                    let candidate = encode_pixels(&current, format, mid)?;
                    if candidate.len() as u64 <= target_bytes {
                        best = candidate;
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                return Ok((current, best));
            }
        }
        let edge = current.width.max(current.height);
        if edge <= floor {
            break;
        }
        let next = ((f64::from(edge) * 0.8).floor() as u32).max(floor);
        current = resize_image(image, Some(next), None)?;
    }
    Err("在最低质量及尺寸限制内无法达到目标体积，请增大上限".into())
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithData(data: CFDataRef, options: CFDictionaryRef) -> CFTypeRef;
    fn CGImageSourceGetCount(source: CFTypeRef) -> usize;
    fn CGImageSourceGetType(source: CFTypeRef) -> CFStringRef;
    fn CGImageSourceCopyPropertiesAtIndex(
        source: CFTypeRef,
        index: usize,
        options: CFDictionaryRef,
    ) -> CFDictionaryRef;
    fn CGImageSourceCreateThumbnailAtIndex(
        source: CFTypeRef,
        index: usize,
        options: CFDictionaryRef,
    ) -> *mut core_graphics::sys::CGImage;
    fn CGImageDestinationCreateWithData(
        data: CFMutableDataRef,
        kind: CFStringRef,
        count: usize,
        options: CFDictionaryRef,
    ) -> CFTypeRef;
    fn CGImageDestinationAddImage(
        dest: CFTypeRef,
        image: *mut core_graphics::sys::CGImage,
        props: CFDictionaryRef,
    );
    fn CGImageDestinationFinalize(dest: CFTypeRef) -> bool;
    static kCGImageSourceShouldCache: CFStringRef;
    static kCGImagePropertyPixelWidth: CFStringRef;
    static kCGImagePropertyPixelHeight: CFStringRef;
    static kCGImageSourceCreateThumbnailFromImageAlways: CFStringRef;
    static kCGImageSourceCreateThumbnailWithTransform: CFStringRef;
    static kCGImageSourceThumbnailMaxPixelSize: CFStringRef;
    static kCGImageDestinationLossyCompressionQuality: CFStringRef;
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits: usize,
        stride: usize,
        space: *mut core_graphics::sys::CGColorSpace,
        info: u32,
    ) -> *mut core_graphics::sys::CGContext;
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits: usize,
        pixel_bits: usize,
        stride: usize,
        space: *mut core_graphics::sys::CGColorSpace,
        info: u32,
        provider: *mut core_graphics::sys::CGDataProvider,
        decode: *const f64,
        interpolate: bool,
        intent: u32,
    ) -> *mut core_graphics::sys::CGImage;
}

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
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
}
/// Validates the complete source and detects real transparency, not merely the
/// presence of an alpha channel in an otherwise opaque bitmap.
pub fn inspect_image(bytes: &[u8]) -> Result<ImageInfo, String> {
    let pixels = decode_image(bytes)?;
    let data = CFData::from_buffer(bytes);
    // SAFETY: live CFData and default options; Create ownership below.
    let source = OwnedCf::new(
        unsafe { CGImageSourceCreateWithData(data.as_concrete_TypeRef(), ptr::null()) },
        "无法识别图片格式",
    )?;
    // SAFETY: live image source; GetType returns a borrowed UTI CFString.
    let raw = unsafe { CGImageSourceGetType(source.0) };
    let uti = if raw.is_null() {
        String::new()
    } else {
        key(raw).to_string()
    };
    let format = match uti.as_str() {
        "public.png" => Some(ImageFormat::Png),
        "public.jpeg" => Some(ImageFormat::Jpeg),
        "org.webmproject.webp" | "public.webp" => Some(ImageFormat::Webp),
        "public.heic" | "public.heif" => Some(ImageFormat::Heic),
        _ => None,
    };
    Ok(ImageInfo {
        width: pixels.width,
        height: pixels.height,
        has_alpha: pixels.has_transparency(),
        format,
    })
}
/// Encode a single workflow step; returned TIFF is decoded from the encoded
/// output, so consumers cannot bypass lossy processing via its compatibility UTI.
pub fn encode_image(
    bytes: &[u8],
    format: ImageFormat,
    quality: u8,
    max_edge: Option<u32>,
) -> Result<ProcessedImage, String> {
    let source = decode_image(bytes)?;
    let pixels = if let Some(edge) = max_edge {
        resize_image(&source, Some(edge), None)?
    } else {
        source
    };
    let data = encode_pixels(&pixels, format, quality)?;
    processed_representation(data, format)
}
/// Preserve original valid encoded bytes while generating compatibility data.
/// Used when a compression budget is already satisfied (no recompression).
pub fn processed_representation(
    data: Vec<u8>,
    format: ImageFormat,
) -> Result<ProcessedImage, String> {
    let output = decode_image(&data)?;
    let tiff = encode_tiff(&output)?;
    Ok(ProcessedImage {
        data,
        tiff,
        width: output.width,
        height: output.height,
        format,
    })
}
pub fn tiff_representation(bytes: &[u8]) -> Result<Vec<u8>, String> {
    encode_tiff(&decode_image(bytes)?)
}
