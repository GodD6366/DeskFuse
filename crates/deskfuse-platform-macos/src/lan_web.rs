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
use deskfuse_core::{ConnectionContext, InterfaceKind, LinkScope, TransportSessionId};
use deskfuse_platform::ports::LanWebPort;
use std::{
    collections::BTreeSet,
    ffi::{CStr, c_char, c_int, c_uint, c_void},
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    path::Path,
    process::Command,
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

/// Route attestation kept in the macOS adapter. Interface names and private
/// address classes are never used as physical evidence: route, flags,
/// ifconfig address/prefix, and SystemConfiguration hardware records must all
/// agree before a LAN result is returned.
pub fn transport_context(
    session_id: TransportSessionId,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
) -> Result<ConnectionContext, String> {
    let mut context = ConnectionContext {
        session_id,
        local_addr,
        remote_addr,
        interface_name: None,
        interface_kind: InterfaceKind::Other,
        scope: LinkScope::Unknown,
    };
    if local_addr.ip().is_loopback() || remote_addr.ip().is_loopback() {
        context.interface_kind = InterfaceKind::Loopback;
        return Ok(context);
    }
    let route = Command::new("/sbin/route")
        .args(["-n", "get", &remote_addr.ip().to_string()])
        .output()
        .map_err(|error| format!("route: {error}"))?;
    let interfaces = Command::new("/sbin/ifconfig")
        .arg("-a")
        .output()
        .map_err(|error| format!("ifconfig: {error}"))?;
    let hardware = Command::new("/usr/sbin/networksetup")
        .arg("-listallhardwareports")
        .output()
        .map_err(|error| format!("networksetup: {error}"))?;
    for (name, result) in [
        ("route", &route),
        ("ifconfig", &interfaces),
        ("networksetup", &hardware),
    ] {
        if !result.status.success() {
            // Preserve why attestation failed. The transport still returns an
            // unknown, unusable context, but no longer loses the command error.
            return Err(format!(
                "{name}: exit={:?} stderr={}",
                result.status.code(),
                String::from_utf8_lossy(&result.stderr)
                    .chars()
                    .take(200)
                    .collect::<String>()
            ));
        }
    }
    let route = String::from_utf8_lossy(&route.stdout);
    let interfaces = String::from_utf8_lossy(&interfaces.stdout);
    let hardware = String::from_utf8_lossy(&hardware.stdout);
    let (name, kind, scope) = classify_route(
        &route,
        &interfaces,
        &hardware,
        local_addr.ip(),
        remote_addr.ip(),
    );
    context.interface_name = name;
    context.interface_kind = kind;
    context.scope = scope;
    Ok(context)
}

fn classify_route(
    route: &str,
    interfaces: &str,
    hardware: &str,
    local: IpAddr,
    remote: IpAddr,
) -> (Option<String>, InterfaceKind, LinkScope) {
    let value = |name: &str| {
        route
            .lines()
            .find_map(|line| line.trim().strip_prefix(name).map(str::trim))
    };
    let Some(name) = value("interface:") else {
        return (None, InterfaceKind::Other, LinkScope::Unknown);
    };
    let result = |kind, scope| (Some(name.to_owned()), kind, scope);
    if local.is_loopback() || remote.is_loopback() || name.starts_with("lo") {
        return result(InterfaceKind::Loopback, LinkScope::Unknown);
    }
    if [
        "utun",
        "tun",
        "tap",
        "ppp",
        "ipsec",
        "tailscale",
        "wg",
        "awdl",
        "llw",
        "vmnet",
        "vmenet",
        "veth",
        "gif",
        "stf",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
    {
        return result(InterfaceKind::Vpn, LinkScope::Overlay);
    }
    let hardware_port = hardware.split("Hardware Port: ").find_map(|block| {
        block
            .lines()
            .any(|line| line.trim() == format!("Device: {name}"))
            .then(|| block.lines().next())
            .flatten()
    });
    let kind = match hardware_port {
        Some(port) if port.contains("Wi-Fi") || port.contains("AirPort") => InterfaceKind::WiFi,
        Some(port)
            if port.contains("Thunderbolt") || port.contains("雷雳") || port.contains("雷電") =>
        {
            InterfaceKind::Thunderbolt
        }
        Some(port)
            if port.contains("Ethernet") || port.contains("LAN") || port.contains("以太网") =>
        {
            InterfaceKind::Ethernet
        }
        _ => InterfaceKind::Other,
    };
    if kind == InterfaceKind::Other
        || (name.starts_with("bridge") && kind != InterfaceKind::Thunderbolt)
    {
        return result(InterfaceKind::Other, LinkScope::Unknown);
    }
    let flags = value("flags:").unwrap_or("");
    if flags.contains("GATEWAY") || flags.contains("REJECT") || flags.contains("BLACKHOLE") {
        return result(kind, LinkScope::Public);
    }
    let section = interfaces
        .split('\n')
        .skip_while(|line| !line.starts_with(&format!("{name}:")))
        .take_while_with_header();
    if kind == InterfaceKind::Thunderbolt && name.starts_with("bridge") {
        let members = section
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("member:")
                    .and_then(|value| value.split_whitespace().next())
            })
            .collect::<Vec<_>>();
        if members.is_empty()
            || members.iter().any(|member| {
                !hardware.split("Hardware Port: ").any(|block| {
                    block.lines().next().is_some_and(|port| {
                        port.contains("Thunderbolt")
                            || port.contains("雷雳")
                            || port.contains("雷電")
                    }) && block
                        .lines()
                        .any(|line| line.trim() == format!("Device: {member}"))
                })
            })
        {
            return result(InterfaceKind::Other, LinkScope::Unknown);
        }
    }
    let same_subnet = match (local, remote) {
        (IpAddr::V4(local), IpAddr::V4(remote)) => section.lines().any(|line| {
            let words = line.split_whitespace().collect::<Vec<_>>();
            if words.first() != Some(&"inet")
                || words
                    .get(1)
                    .and_then(|value| value.parse::<Ipv4Addr>().ok())
                    != Some(local)
            {
                return false;
            }
            let Some(index) = words.iter().position(|value| *value == "netmask") else {
                return false;
            };
            let Some(mask) = words
                .get(index + 1)
                .and_then(|value| u32::from_str_radix(value.trim_start_matches("0x"), 16).ok())
            else {
                return false;
            };
            mask != 0 && (u32::from(local) & mask) == (u32::from(remote) & mask)
        }),
        (IpAddr::V6(local), IpAddr::V6(remote)) => section.lines().any(|line| {
            let words = line.split_whitespace().collect::<Vec<_>>();
            if words.first() != Some(&"inet6")
                || words
                    .get(1)
                    .and_then(|value| value.split('%').next())
                    .and_then(|value| value.parse::<std::net::Ipv6Addr>().ok())
                    != Some(local)
            {
                return false;
            }
            let Some(index) = words.iter().position(|value| *value == "prefixlen") else {
                return false;
            };
            let Some(prefix) = words
                .get(index + 1)
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|prefix| *prefix > 0 && *prefix <= 128)
            else {
                return false;
            };
            let mask = u128::MAX << (128 - prefix);
            u128::from(local) & mask == u128::from(remote) & mask
        }),
        _ => false,
    };
    if same_subnet && local != remote && flags.contains("UP") {
        result(kind, LinkScope::Lan)
    } else {
        result(kind, LinkScope::Unknown)
    }
}

trait Section<'a>: Iterator<Item = &'a str> + Sized {
    fn take_while_with_header(self) -> String {
        let mut result = String::new();
        for (index, line) in self.enumerate() {
            if index > 0 && !line.starts_with(char::is_whitespace) {
                break;
            }
            result.push_str(line);
            result.push('\n');
        }
        result
    }
}
impl<'a, T: Iterator<Item = &'a str>> Section<'a> for T {}

/// Bind only to interfaces whose SystemConfiguration hardware record and
/// operational address both identify a physical LAN port.
pub fn transport_listener_sockets(port: u16) -> Result<Vec<(String, UdpSocket)>, String> {
    let hardware = Command::new("/usr/sbin/networksetup")
        .arg("-listallhardwareports")
        .output()
        .map_err(|error| format!("networksetup: {error}"))?;
    if !hardware.status.success() {
        return Ok(Vec::new());
    }
    let hardware = String::from_utf8_lossy(&hardware.stdout);
    let mut sockets = Vec::new();
    for interface in if_addrs::get_if_addrs().map_err(|error| error.to_string())? {
        if interface.is_loopback() || interface.is_p2p || !interface.is_oper_up() {
            continue;
        }
        let Some(index) = interface.index.and_then(std::num::NonZeroU32::new) else {
            continue;
        };
        let physical = hardware.split("Hardware Port: ").any(|block| {
            block
                .lines()
                .any(|line| line.trim() == format!("Device: {}", interface.name))
                && block.lines().next().is_some_and(|port| {
                    port.contains("Wi-Fi")
                        || port.contains("AirPort")
                        || port.contains("Ethernet")
                        || port.contains("LAN")
                        || port.contains("以太网")
                        || port.contains("Thunderbolt")
                        || port.contains("雷雳")
                        || port.contains("雷電")
                })
        });
        if !physical || (!interface.name.starts_with("en") && !interface.name.starts_with("bridge"))
        {
            continue;
        }
        if matches!(interface.ip(), IpAddr::V6(ip) if ip.is_unicast_link_local()) {
            continue;
        }
        let domain = if interface.ip().is_ipv4() {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        };
        let Ok(socket) =
            socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
        else {
            continue;
        };
        let bound = if interface.ip().is_ipv4() {
            socket.bind_device_by_index_v4(Some(index))
        } else {
            socket.bind_device_by_index_v6(Some(index))
        };
        if bound.is_err()
            || socket
                .bind(&SocketAddr::new(interface.ip(), port).into())
                .is_err()
            || socket.set_nonblocking(true).is_err()
        {
            continue;
        }
        sockets.push((interface.name, socket.into()));
    }
    Ok(sockets)
}

/// Complete adapter for transport callers; web and transport APIs share the
/// same host-owned implementation.
pub struct MacOSLanWebPort;
impl LanWebPort for MacOSLanWebPort {
    fn interfaces(
        &self,
    ) -> deskfuse_platform::PlatformResult<Vec<deskfuse_platform::ports::LanInterface>> {
        interfaces()
            .map(|items| {
                items
                    .into_iter()
                    .map(|item| deskfuse_platform::ports::LanInterface {
                        name: item.name,
                        address: item.address,
                        label: item.label,
                    })
                    .collect()
            })
            .map_err(|message| deskfuse_platform::PlatformError::InvalidRequest { message })
    }
    fn ipv4_addresses(&self) -> deskfuse_platform::PlatformResult<Vec<String>> {
        all_ipv4_interfaces()
            .map(|items| items.into_iter().map(|item| item.address).collect())
            .map_err(|message| deskfuse_platform::PlatformError::InvalidRequest { message })
    }
    fn image_png(
        &self,
        path: &Path,
        max_pixel_size: u32,
    ) -> deskfuse_platform::PlatformResult<Vec<u8>> {
        image_png(path, max_pixel_size)
            .map_err(|message| deskfuse_platform::PlatformError::InvalidRequest { message })
    }
    fn data_image_png(
        &self,
        data: &[u8],
        max_pixel_size: u32,
    ) -> deskfuse_platform::PlatformResult<Vec<u8>> {
        data_image_png(data, max_pixel_size)
            .map_err(|message| deskfuse_platform::PlatformError::InvalidRequest { message })
    }
    fn transport_context(
        &self,
        session_id: TransportSessionId,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
    ) -> deskfuse_platform::PlatformResult<ConnectionContext> {
        transport_context(session_id, local_addr, remote_addr)
            .map_err(|message| deskfuse_platform::PlatformError::InvalidRequest { message })
    }
    fn transport_listener_sockets(
        &self,
        port: u16,
    ) -> deskfuse_platform::PlatformResult<Vec<(String, UdpSocket)>> {
        transport_listener_sockets(port)
            .map_err(|message| deskfuse_platform::PlatformError::InvalidRequest { message })
    }
}

fn collect_interfaces(all: bool) -> Result<Vec<LanInterface>, String> {
    let thunderbolt_bridges = if all {
        BTreeSet::new()
    } else {
        thunderbolt_bridges()?
    };
    let mut result = ipv4_entries(all, &thunderbolt_bridges)?
        .into_iter()
        .map(|entry| {
            let address = entry.address.to_string();
            LanInterface {
                label: format!("{} · {address}", entry.name),
                name: entry.name,
                address,
            }
        })
        .collect::<Vec<_>>();
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

/// 物理局域网接口（筛选规则同 [`interfaces`]）上的 IPv4 地址及前缀长度。
/// 运行时在 mDNS 组播被 AP 过滤时据此推导单播探测网段；每次调用都实时读取，
/// 切换 Wi-Fi 或插拔网线后无需重启。
pub fn lan_ipv4_networks() -> Result<Vec<(Ipv4Addr, u8)>, String> {
    let bridges = thunderbolt_bridges()?;
    Ok(ipv4_entries(false, &bridges)?
        .into_iter()
        .map(|entry| (entry.address, entry.prefix))
        .collect())
}

/// 已启用接口上的一个 IPv4 地址。
struct Ipv4Entry {
    name: String,
    address: Ipv4Addr,
    prefix: u8,
}

/// 掩码前缀长度。Darwin 的 netmask sockaddr 可能截掉末尾的零字节（`sa_len`
/// 小于 `sockaddr_in`），所以只读取 `sa_len` 范围内的字节，缺失部分按 0 计。
fn netmask_prefix(mask: *const Sockaddr) -> u8 {
    if mask.is_null() {
        return 0;
    }
    // SAFETY: 非空 netmask 属于存活的 getifaddrs 节点，`sa_len` 字节均有效。
    let len = usize::from(unsafe { (*mask).len }).min(std::mem::size_of::<SockaddrIn>());
    // SAFETY: 同上，只读取前 `len` 个字节。
    let bytes = unsafe { std::slice::from_raw_parts(mask.cast::<u8>(), len) };
    let mut octets = [0_u8; 4];
    for (index, octet) in octets.iter_mut().enumerate() {
        // sockaddr_in 布局：len、family、port(2)，地址从第 4 字节开始。
        if let Some(byte) = bytes.get(4 + index) {
            *octet = *byte;
        }
    }
    u8::try_from(u32::from_be_bytes(octets).leading_ones()).unwrap_or(0)
}

fn ipv4_entries(
    all: bool,
    thunderbolt_bridges: &BTreeSet<String>,
) -> Result<Vec<Ipv4Entry>, String> {
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
        result.push(Ipv4Entry {
            name: name.into_owned(),
            address,
            prefix: netmask_prefix(item.netmask),
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造与 Darwin sockaddr 布局一致的缓冲区；`len` 模拟被截断的掩码。
    fn mask(len: u8, octets: [u8; 4]) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[0] = len;
        bytes[1] = 2;
        bytes[4..8].copy_from_slice(&octets);
        bytes
    }

    #[test]
    fn netmask_prefix_reads_full_and_trimmed_masks() {
        let full = mask(16, [255, 255, 252, 0]);
        assert_eq!(netmask_prefix(full.as_ptr().cast()), 22);
        // 截断到 7 字节：末尾的 0 被省略，仍应得到 /22。
        let trimmed = mask(7, [255, 255, 252, 0]);
        assert_eq!(netmask_prefix(trimmed.as_ptr().cast()), 22);
        // 截断到 5 字节时第二段之后都按 0 处理。
        let short = mask(5, [255, 255, 255, 0]);
        assert_eq!(netmask_prefix(short.as_ptr().cast()), 8);
        assert_eq!(netmask_prefix(ptr::null()), 0);
    }
}
