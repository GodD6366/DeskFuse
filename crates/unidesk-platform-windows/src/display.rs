//! Native monitor identities and DPI-aware coordinate conversion.
//!
//! Win32's desktop is in physical pixels under `PMv2` awareness. Protocol positions
//! are display-local logical points. Always select the monitor in physical space
//! before converting; there is no single global scale on mixed-DPI desktops.
#![allow(unsafe_code)]

use crate::{PlatformError, PlatformResult};
use std::hash::{Hash, Hasher};
use unidesk_core::{DisplayDescriptor, DisplayTopology};
use windows_sys::Win32::{
    Devices::Display::{
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
        DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_ROTATION_IDENTITY,
        DISPLAYCONFIG_ROTATION_ROTATE90, DISPLAYCONFIG_ROTATION_ROTATE180,
        DISPLAYCONFIG_ROTATION_ROTATE270, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
        DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
        QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
    },
    Foundation::{ERROR_INSUFFICIENT_BUFFER, RECT},
    Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW},
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            SetThreadDpiAwarenessContext,
        },
        Shell::GetScaleFactorForMonitor,
        WindowsAndMessaging::MONITORINFOF_PRIMARY,
    },
};

use unidesk_platform::MainDisplayGeometry;

/// A native snapshot is required to invert protocol display-local coordinates.
#[derive(Clone, Debug)]
pub struct NativeDisplay {
    pub descriptor: DisplayDescriptor,
    pub physical_left: i32,
    pub physical_top: i32,
    pub physical_width: u32,
    pub physical_height: u32,
}

impl NativeDisplay {
    fn valid_scale(&self) -> bool {
        self.descriptor.scale_factor.is_finite()
            && (0.25..=8.0).contains(&self.descriptor.scale_factor)
            && self.physical_width > 0
            && self.physical_height > 0
    }

    #[must_use]
    pub fn physical_to_local(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let local_x = x - f64::from(self.physical_left);
        let local_y = y - f64::from(self.physical_top);
        if !self.valid_scale()
            || !local_x.is_finite()
            || !local_y.is_finite()
            || local_x < 0.0
            || local_y < 0.0
            || local_x >= f64::from(self.physical_width)
            || local_y >= f64::from(self.physical_height)
        {
            return None;
        }
        Some((
            local_x / self.descriptor.scale_factor,
            local_y / self.descriptor.scale_factor,
        ))
    }

    #[must_use]
    pub fn local_to_physical(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if !self.valid_scale() || !self.descriptor.contains(x, y) {
            return None;
        }
        let x = x * self.descriptor.scale_factor;
        let y = y * self.descriptor.scale_factor;
        // Rounded logical dimensions can include a fractional pixel past the edge.
        if x >= f64::from(self.physical_width) || y >= f64::from(self.physical_height) {
            return None;
        }
        Some((
            f64::from(self.physical_left) + x,
            f64::from(self.physical_top) + y,
        ))
    }
}

/// Restore caller awareness even when enumeration fails. This never changes the
/// process-wide GPUI DPI policy or another thread's awareness.
struct DpiContext(DPI_AWARENESS_CONTEXT);
impl Drop for DpiContext {
    fn drop(&mut self) {
        // SAFETY: this context was returned for this same thread by SetThreadDpiAwarenessContext.
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

struct Monitor {
    name: String,
    bounds: RECT,
    primary: bool,
    scale: f64,
}
struct Enumeration {
    monitors: Vec<Monitor>,
    failed: bool,
}
unsafe extern "system" fn enumerate_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: isize,
) -> i32 {
    // SAFETY: EnumDisplayMonitors invokes synchronously with our exclusively borrowed state.
    let state = unsafe { &mut *(data as *mut Enumeration) };
    let mut info = MONITORINFOEXW::default();
    let Ok(info_size) = u32::try_from(size_of::<MONITORINFOEXW>()) else {
        state.failed = true;
        return 0;
    };
    info.monitorInfo.cbSize = info_size;
    let mut scale = 0;
    // SAFETY: monitor is provided by enumeration; outputs are initialized and correctly sized.
    if unsafe { GetMonitorInfoW(monitor, &raw mut info.monitorInfo) } == 0
        || unsafe { GetScaleFactorForMonitor(monitor, &raw mut scale) } < 0
        || !(25..=800).contains(&scale)
    {
        state.failed = true;
        return 0;
    }
    state.monitors.push(Monitor {
        name: wide_string(&info.szDevice),
        bounds: info.monitorInfo.rcMonitor,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        scale: f64::from(scale) / 100.0,
    });
    1
}

fn active_paths() -> PlatformResult<Vec<DISPLAYCONFIG_PATH_INFO>> {
    // A hotplug can race the size query. Bound retries and allocation on every attempt.
    for _ in 0..3 {
        let (mut paths_len, mut modes_len) = (0, 0);
        // SAFETY: both count output pointers refer to live u32 values.
        if unsafe {
            GetDisplayConfigBufferSizes(
                QDC_ONLY_ACTIVE_PATHS,
                &raw mut paths_len,
                &raw mut modes_len,
            )
        } != 0
            || paths_len == 0
            || paths_len > 256
            || modes_len > 1024
        {
            return Err(PlatformError::DisplayUnavailable);
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); paths_len as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); modes_len as usize];
        // SAFETY: buffers contain at least the supplied capacities; null topology ID is required for ONLY_ACTIVE_PATHS.
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &raw mut paths_len,
                paths.as_mut_ptr(),
                &raw mut modes_len,
                modes.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status != 0 {
            return Err(PlatformError::DisplayUnavailable);
        }
        paths.truncate(paths_len as usize);
        return Ok(paths);
    }
    Err(PlatformError::DisplayUnavailable)
}

/// Enumerate only traversable GDI monitors; mirrored targets collapse to one
/// source. IDs derive from the persistent monitor device path, not HMONITOR or
/// volatile DISPLAY1 numbering. No synthetic identity is returned on query failure.
///
/// # Errors
/// Returns `DisplayUnavailable` if a native query fails, monitor identification
/// is unavailable, or the resulting geometry is invalid.
pub fn native_displays() -> PlatformResult<Vec<NativeDisplay>> {
    // SAFETY: PMv2 is supported on the minimum Windows 11 target; restoration is scoped to this thread.
    let old = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if old.is_null() {
        return Err(PlatformError::DisplayUnavailable);
    }
    let _dpi = DpiContext(old);
    let mut enumeration = Enumeration {
        monitors: Vec::new(),
        failed: false,
    };
    // SAFETY: callback lifetime and exclusive state are bounded by this synchronous invocation.
    let ok = unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(enumerate_monitor),
            (&raw mut enumeration) as isize,
        )
    };
    if ok == 0 || enumeration.failed {
        return Err(PlatformError::DisplayUnavailable);
    }
    let mut names = Vec::new();
    for path in active_paths()? {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source.header.size = u32::try_from(size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>())
            .map_err(|_| PlatformError::DisplayUnavailable)?;
        source.header.adapterId = path.sourceInfo.adapterId;
        source.header.id = path.sourceInfo.id;
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        target.header.size = u32::try_from(size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>())
            .map_err(|_| PlatformError::DisplayUnavailable)?;
        target.header.adapterId = path.targetInfo.adapterId;
        target.header.id = path.targetInfo.id;
        // SAFETY: repr(C) header starts each correctly sized device-info structure.
        if unsafe { DisplayConfigGetDeviceInfo(&raw mut source.header) } != 0
            || unsafe { DisplayConfigGetDeviceInfo(&raw mut target.header) } != 0
        {
            return Err(PlatformError::DisplayUnavailable);
        }
        let device_path = wide_string(&target.monitorDevicePath).to_lowercase();
        if device_path.is_empty() {
            return Err(PlatformError::DisplayUnavailable);
        }
        let rotation = match path.targetInfo.rotation {
            DISPLAYCONFIG_ROTATION_IDENTITY => 0,
            DISPLAYCONFIG_ROTATION_ROTATE90 => 90,
            DISPLAYCONFIG_ROTATION_ROTATE180 => 180,
            DISPLAYCONFIG_ROTATION_ROTATE270 => 270,
            _ => return Err(PlatformError::DisplayUnavailable),
        };
        names.push((
            wide_string(&source.viewGdiDeviceName),
            device_path,
            rotation,
        ));
    }
    // Deterministic representative when one source is mirrored to multiple targets.
    names.sort();
    let mut result = Vec::new();
    for monitor in enumeration.monitors {
        let (_, path, rotation) = names
            .iter()
            .find(|(name, _, _)| *name == monitor.name)
            .ok_or(PlatformError::DisplayUnavailable)?;
        let width = monitor
            .bounds
            .right
            .checked_sub(monitor.bounds.left)
            .filter(|v| *v > 0)
            .ok_or(PlatformError::DisplayUnavailable)?
            .cast_unsigned();
        let height = monitor
            .bounds
            .bottom
            .checked_sub(monitor.bounds.top)
            .filter(|v| *v > 0)
            .ok_or(PlatformError::DisplayUnavailable)?
            .cast_unsigned();
        result.push(NativeDisplay {
            descriptor: DisplayDescriptor {
                id: stable_monitor_id(path).into(),
                // Presentation coordinates only: physical monitor selection and
                // pointer conversion must use physical_to_local/local_to_physical.
                frame_x: f64::from(monitor.bounds.left) / monitor.scale,
                frame_y: f64::from(monitor.bounds.top) / monitor.scale,
                width: logical_extent(width, monitor.scale)?,
                height: logical_extent(height, monitor.scale)?,
                scale_factor: monitor.scale,
                rotation_degrees: *rotation,
                is_primary: monitor.primary,
            },
            physical_left: monitor.bounds.left,
            physical_top: monitor.bounds.top,
            physical_width: width,
            physical_height: height,
        });
    }
    result.sort_by(|a, b| a.descriptor.id.cmp(&b.descriptor.id));
    topology_from_native(&result)?;
    Ok(result)
}

// The float-to-integer conversion is bounded and explicitly rounded before the
// cast; only protocol-valid logical dimensions reach it.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn logical_extent(physical: u32, scale: f64) -> PlatformResult<u32> {
    let logical = (f64::from(physical) / scale).ceil();
    if !logical.is_finite() || !(1.0..=65_536.0).contains(&logical) {
        return Err(PlatformError::DisplayUnavailable);
    }
    Ok(logical as u32)
}

fn topology_from_native(displays: &[NativeDisplay]) -> PlatformResult<DisplayTopology> {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for native in displays {
        let d = &native.descriptor;
        d.id.hash(&mut hash);
        native.physical_left.hash(&mut hash);
        native.physical_top.hash(&mut hash);
        native.physical_width.hash(&mut hash);
        native.physical_height.hash(&mut hash);
        d.scale_factor.to_bits().hash(&mut hash);
        d.rotation_degrees.hash(&mut hash);
        d.is_primary.hash(&mut hash);
    }
    let topology = DisplayTopology {
        generation: hash.finish(),
        displays: displays.iter().map(|d| d.descriptor.clone()).collect(),
    };
    topology
        .validate()
        .map_err(|_| PlatformError::DisplayUnavailable)?;
    Ok(topology)
}

/// Read the current native display topology.
///
/// # Errors
/// Returns `DisplayUnavailable` if native enumeration or geometry validation fails.
pub fn display_topology() -> PlatformResult<DisplayTopology> {
    topology_from_native(&native_displays()?)
}
/// Read the primary monitor's logical geometry.
///
/// # Errors
/// Returns `DisplayUnavailable` if enumeration fails or no primary monitor exists.
pub fn main_display_geometry() -> PlatformResult<MainDisplayGeometry> {
    let topology = display_topology()?;
    let d = topology
        .displays
        .iter()
        .find(|d| d.is_primary)
        .ok_or(PlatformError::DisplayUnavailable)?;
    Ok(MainDisplayGeometry {
        width: d.width,
        height: d.height,
        origin_x: d.frame_x,
        origin_y: d.frame_y,
    })
}
fn wide_string(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|c| *c == 0).unwrap_or(value.len())])
}
fn stable_monitor_id(path: &str) -> String {
    // FNV-1a is a stable non-security identifier compressor (never a trust hash).
    let hash = path.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("win-monitor-{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor() -> NativeDisplay {
        NativeDisplay {
            descriptor: DisplayDescriptor {
                id: "test".to_owned().into(),
                frame_x: -1280.0,
                frame_y: -100.0,
                width: 1280,
                height: 720,
                scale_factor: 1.5,
                rotation_degrees: 90,
                is_primary: false,
            },
            physical_left: -1920,
            physical_top: -150,
            physical_width: 1920,
            physical_height: 1080,
        }
    }
    #[test]
    fn negative_origin_mixed_dpi_roundtrip() {
        let d = monitor();
        assert_eq!(d.physical_to_local(-960.0, 390.0), Some((640.0, 360.0)));
        assert_eq!(d.local_to_physical(640.0, 360.0), Some((-960.0, 390.0)));
    }
    #[test]
    fn rotated_bounds_are_not_rotated_twice() {
        let d = monitor();
        assert_eq!(d.physical_to_local(-1920.0, -150.0), Some((0.0, 0.0)));
        assert_eq!(d.physical_to_local(0.0, 0.0), None);
        assert_eq!(d.local_to_physical(f64::NAN, 0.0), None);
        assert_eq!(d.local_to_physical(1280.0, 0.0), None);
    }
    #[test]
    fn invalid_scale_never_produces_native_coordinates() {
        let mut d = monitor();
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, 8.1] {
            d.descriptor.scale_factor = scale;
            assert_eq!(d.physical_to_local(-960.0, 390.0), None);
            assert_eq!(d.local_to_physical(640.0, 360.0), None);
        }
    }
    #[test]
    fn rounded_logical_extent_does_not_escape_physical_monitor() {
        let mut d = monitor();
        d.physical_width = 1919;
        assert_eq!(d.local_to_physical(1279.5, 0.0), None);
        assert!(d.local_to_physical(1279.0, 0.0).is_some());
    }
    #[test]
    fn monitor_id_is_stable_and_bounded() {
        let path = "\\\\?\\display#abc#uid123";
        assert_eq!(stable_monitor_id(path), stable_monitor_id(path));
        assert_ne!(stable_monitor_id(path), stable_monitor_id("other"));
        assert!(stable_monitor_id(path).len() <= 128);
    }
}
