//! Narrow LAN interfaces and ImageIO preview adapters. Call image functions on
//! a blocking worker; neither adapter accesses AppKit or requires the UI thread.
#![allow(unsafe_code)] // Darwin/ImageIO FFI remains private to this module.

use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    data::CFData,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
    url::CFURL,
};
use core_foundation_sys::{
    array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef},
    base::CFRelease,
    data::{CFDataCreateMutable, CFDataRef, CFMutableDataRef},
    dictionary::CFDictionaryRef,
    string::CFStringRef,
    url::CFURLRef,
};
use std::{
    collections::BTreeSet,
    ffi::{CStr, c_char, c_int, c_uint, c_void},
    net::Ipv4Addr,
    path::Path,
    ptr,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanInterface {
    pub name: String,
    pub address: String,
    pub label: String,
}

// Darwin ABI from SDK ifaddrs.h, sys/socket.h and netinet/in.h. Keeping these
// private avoids adding a dependency or exporting platform struct layouts.
#[repr(C)]
struct Sockaddr {
    len: u8,
    family: u8,
    data: [c_char; 14],
}
#[repr(C)]
struct SockaddrIn {
    len: u8,
    family: u8,
    port: u16,
    address: u32,
    zero: [u8; 8],
}
#[repr(C)]
struct Ifaddrs {
    next: *mut Ifaddrs,
    name: *mut c_char,
    flags: c_uint,
    address: *mut Sockaddr,
    netmask: *mut Sockaddr,
    destination: *mut Sockaddr,
    data: *mut c_void,
}
unsafe extern "C" {
    fn getifaddrs(result: *mut *mut Ifaddrs) -> c_int;
    fn freeifaddrs(value: *mut Ifaddrs);
}
struct InterfaceList(*mut Ifaddrs);
impl Drop for InterfaceList {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: one successful getifaddrs result, released exactly once.
            unsafe { freeifaddrs(self.0) };
        }
    }
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCNetworkInterfaceCopyAll() -> CFArrayRef;
    fn SCNetworkInterfaceGetBSDName(interface: CFTypeRef) -> CFStringRef;
    fn SCNetworkInterfaceGetLocalizedDisplayName(interface: CFTypeRef) -> CFStringRef;
}

fn thunderbolt_bridges() -> Result<BTreeSet<String>, String> {
    // SAFETY: CopyAll takes no arguments and returns a Create-owned CFArray
    // of SCNetworkInterface objects, kept alive throughout this traversal.
    let interfaces = OwnedCf::new(
        unsafe { SCNetworkInterfaceCopyAll() }.cast(),
        "无法读取系统网络接口配置",
    )?;
    let array = interfaces.0.cast();
    let mut bridges = BTreeSet::new();
    // SAFETY: array is the live CFArray returned by CopyAll.
    for index in 0..unsafe { CFArrayGetCount(array) } {
        // SAFETY: index is in bounds; the array retains each interface.
        let interface = unsafe { CFArrayGetValueAtIndex(array, index) };
        // SAFETY: both getters receive a live SCNetworkInterface. Nullable
        // borrowed strings are checked and retained before conversion.
        let (name, label) = unsafe {
            (
                SCNetworkInterfaceGetBSDName(interface),
                SCNetworkInterfaceGetLocalizedDisplayName(interface),
            )
        };
        if name.is_null() || label.is_null() {
            continue;
        }
        // SAFETY: non-null Get-owned CFStrings are alive with the array.
        let (name, label) = unsafe {
            (
                CFString::wrap_under_get_rule(name).to_string(),
                CFString::wrap_under_get_rule(label).to_string(),
            )
        };
        if name.starts_with("bridge")
            && ["Thunderbolt", "雷雳", "雷電", "雷电"]
                .iter()
                .any(|kind| label.contains(kind))
        {
            bridges.insert(name);
        }
    }
    Ok(bridges)
}

/// Up, non-loopback IPv4 interfaces: Darwin `en<digits>` and system-identified
/// Thunderbolt bridges. Other virtual bridges, AWDL and tunnels stay excluded.
pub fn interfaces() -> Result<Vec<LanInterface>, String> {
    collect_interfaces(false)
}

/// All assigned IPv4 addresses, including loopback and virtual interfaces,
/// for validating requests to a wildcard listener.
pub fn all_ipv4_interfaces() -> Result<Vec<LanInterface>, String> {
    collect_interfaces(true)
}

fn collect_interfaces(all: bool) -> Result<Vec<LanInterface>, String> {
    let thunderbolt_bridges = if all {
        BTreeSet::new()
    } else {
        thunderbolt_bridges()?
    };
    let mut raw = ptr::null_mut();
    // SAFETY: valid writable out pointer; Darwin allocates the linked list.
    if unsafe { getifaddrs(&mut raw) } != 0 {
        return Err(format!(
            "无法读取局域网接口：{}",
            std::io::Error::last_os_error()
        ));
    }
    let list = InterfaceList(raw);
    let mut current = list.0;
    let mut result = Vec::new();
    while !current.is_null() {
        // SAFETY: each node belongs to the live getifaddrs list, which is not
        // mutated and remains owned by `list` until traversal has finished.
        let item = unsafe { &*current };
        current = item.next;
        if item.flags & 0x1 == 0
            || (!all && item.flags & 0x8 != 0)
            || item.address.is_null()
            || item.name.is_null()
        {
            continue;
        }
        // SAFETY: getifaddrs names are NUL-terminated and alive with their node.
        let name = unsafe { CStr::from_ptr(item.name) }.to_string_lossy();
        let ethernet = name
            .strip_prefix("en")
            .is_some_and(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()));
        if !all && !ethernet && !thunderbolt_bridges.contains(name.as_ref()) {
            continue;
        }
        // SAFETY: non-null sockaddr belongs to the live node. The family and
        // Darwin length fields are checked before reading the IPv4 layout.
        let addr = unsafe { &*item.address };
        if addr.family != 2 || usize::from(addr.len) < std::mem::size_of::<SockaddrIn>() {
            continue;
        }
        // SAFETY: AF_INET and sin_len establish a full, aligned sockaddr_in.
        let ipv4 = unsafe { &*item.address.cast::<SockaddrIn>() };
        let address = Ipv4Addr::from(ipv4.address.to_ne_bytes());
        if address.is_unspecified()
            || (!all && address.is_loopback())
            || address.is_multicast()
            || address.is_broadcast()
        {
            continue;
        }
        let name = name.into_owned();
        let address = address.to_string();
        result.push(LanInterface {
            label: format!("{name} · {address}"),
            name,
            address,
        });
    }
    // Preserve the existing automatic selection order; newly supported bridges
    // remain explicitly selectable without displacing the default LAN address.
    result.sort_by(|a, b| {
        (thunderbolt_bridges.contains(&a.name), &a.name, &a.address).cmp(&(
            thunderbolt_bridges.contains(&b.name),
            &b.name,
            &b.address,
        ))
    });
    result.dedup_by(|a, b| a.name == b.name && a.address == b.address);
    Ok(result)
}

/// Decode an image file with ImageIO, preserving EXIF orientation in a bounded
/// PNG. The source remains on disk; its original bytes/name are not modified.
pub fn image_png(path: &Path, max_pixel: u32) -> Result<Vec<u8>, String> {
    let metadata = path.metadata().map_err(|e| format!("无法读取图片：{e}"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err("图片为空、不是普通文件或超过 128 MiB 限制".into());
    }
    let url = CFURL::from_path(path, false).ok_or("无法创建图片文件地址")?;
    let options = source_options();
    // SAFETY: live CFURL and options; non-null result transfers Create ownership.
    let source = OwnedCf::new(
        unsafe {
            CGImageSourceCreateWithURL(url.as_concrete_TypeRef(), options.as_concrete_TypeRef())
        },
        "无法识别该图片",
    )?;
    source_png(&source, max_pixel)
}

/// Like `image_png`, including TIFF clipboard representations. Multi-image
/// sources use frame zero, matching the static preview rather than animation.
pub fn data_image_png(bytes: &[u8], max_pixel: u32) -> Result<Vec<u8>, String> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err("图片为空或超过 128 MiB 限制".into());
    }
    let data = CFData::from_buffer(bytes);
    let options = source_options();
    // SAFETY: immutable CFData/options stay alive while the source is decoded.
    let source = OwnedCf::new(
        unsafe {
            CGImageSourceCreateWithData(data.as_concrete_TypeRef(), options.as_concrete_TypeRef())
        },
        "无法识别该图片",
    )?;
    source_png(&source, max_pixel)
}

const MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;

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
        // SAFETY: this non-null reference has exactly one Create-owned retain.
        unsafe { CFRelease(self.0) };
    }
}

fn key(value: CFStringRef) -> CFType {
    // SAFETY: callers supply immutable ImageIO CFString constants; wrapping
    // under Get ownership retains a reference for the dictionary constructor.
    unsafe { CFString::wrap_under_get_rule(value) }.as_CFType()
}
fn source_options() -> CFDictionary<CFType, CFType> {
    // SAFETY: immutable framework constant.
    unsafe {
        CFDictionary::from_CFType_pairs(&[(
            key(kCGImageSourceShouldCache),
            CFBoolean::false_value().as_CFType(),
        )])
    }
}
fn source_png(source: &OwnedCf, max_pixel: u32) -> Result<Vec<u8>, String> {
    // SAFETY: valid ImageIO source with owned lifetime across decoding.
    if unsafe { CGImageSourceGetCount(source.0) } == 0 {
        return Err("文件不包含可解码的图片".into());
    }
    // SAFETY: all keys are immutable ImageIO constants. Transform applies EXIF
    // orientation; bounding the decode avoids full-resolution bitmap creation.
    let options = unsafe {
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
                CFNumber::from(i64::from(max_pixel.clamp(160, 2048))).as_CFType(),
            ),
            (
                key(kCGImageSourceShouldCache),
                CFBoolean::false_value().as_CFType(),
            ),
        ])
    };
    // SAFETY: frame zero exists; return is an owned CGImage, CFRelease-compatible.
    let image = OwnedCf::new(
        unsafe { CGImageSourceCreateThumbnailAtIndex(source.0, 0, options.as_concrete_TypeRef()) }
            .cast(),
        "无法生成图片预览",
    )?;
    // SAFETY: default allocator, initial capacity zero permits growth.
    let output = OwnedCf::new(
        unsafe { CFDataCreateMutable(ptr::null(), 0) }.cast(),
        "无法创建预览缓冲区",
    )?;
    let png = CFString::new("public.png");
    // SAFETY: live mutable CFData/UTI; destination has independent Create ownership.
    let destination = OwnedCf::new(
        unsafe {
            CGImageDestinationCreateWithData(
                output.0 as CFMutableDataRef,
                png.as_concrete_TypeRef(),
                1,
                ptr::null(),
            )
        },
        "无法创建 PNG 编码器",
    )?;
    // SAFETY: source image, destination and output stay live until finalized.
    unsafe {
        CGImageDestinationAddImage(
            destination.0,
            image.0 as *mut core_graphics::sys::CGImage,
            ptr::null(),
        )
    };
    // SAFETY: destination contains exactly the declared single image.
    if !unsafe { CGImageDestinationFinalize(destination.0) } {
        return Err("PNG 预览编码失败".into());
    }
    // SAFETY: mutable CFData is compatible with immutable CFData. Get rule
    // retains it independently until the bytes have been copied into Rust.
    let data = unsafe { CFData::wrap_under_get_rule(output.0 as CFDataRef) };
    if data.is_empty() || data.len() as usize > MAX_IMAGE_BYTES {
        return Err("预览为空或超出容量限制".into());
    }
    Ok(data.bytes().to_vec())
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithURL(url: CFURLRef, options: CFDictionaryRef) -> CFTypeRef;
    fn CGImageSourceCreateWithData(data: CFDataRef, options: CFDictionaryRef) -> CFTypeRef;
    fn CGImageSourceGetCount(source: CFTypeRef) -> usize;
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
        destination: CFTypeRef,
        image: *mut core_graphics::sys::CGImage,
        properties: CFDictionaryRef,
    );
    fn CGImageDestinationFinalize(destination: CFTypeRef) -> bool;
    static kCGImageSourceShouldCache: CFStringRef;
    static kCGImageSourceCreateThumbnailFromImageAlways: CFStringRef;
    static kCGImageSourceCreateThumbnailWithTransform: CFStringRef;
    static kCGImageSourceThumbnailMaxPixelSize: CFStringRef;
}
