//! Main-display geometry boundary.

use crate::{PlatformError, PlatformResult};

pub use deskfuse_platform::MainDisplayGeometry;

// Must match DFRemoteVirtualDisplay.m. Excluding by native identity also works
// in the independent input-service process, without a racy UI-owned ID list.
fn is_remote_application_display(vendor: u32, model: u32) -> bool {
    vendor == 0x4453 && model == 0x5241
}

/// Read the current main-display geometry.
pub fn main_display_geometry() -> PlatformResult<MainDisplayGeometry> {
    let display = core_graphics::display::CGDisplay::main();
    let bounds = display.bounds();
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return Err(PlatformError::DisplayUnavailable);
    }
    Ok(MainDisplayGeometry {
        width: bounds.size.width.round() as u32,
        height: bounds.size.height.round() as u32,
        origin_x: bounds.origin.x,
        origin_y: bounds.origin.y,
    })
}

/// Enumerate the current native desktop. Quartz bounds are logical points even
/// on Retina and rotated displays; no second scale or rotation transform applies.
#[allow(unsafe_code)]
pub fn display_topology() -> PlatformResult<deskfuse_core::DisplayTopology> {
    use core_foundation::{base::TCFType, string::CFString, uuid::CFUUID};
    use core_foundation_sys::uuid::{CFUUIDCreateString, CFUUIDRef};
    use core_graphics::display::CGDisplay;
    use core_graphics::geometry::CGSize;
    use std::hash::{Hash, Hasher};
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> CFUUIDRef;
        fn CGDisplayVendorNumber(display: u32) -> u32;
        fn CGDisplayModelNumber(display: u32) -> u32;
        fn CGDisplayScreenSize(display: u32) -> CGSize;
    }
    let mut displays = Vec::new();
    for id in CGDisplay::active_displays().map_err(|_| PlatformError::DisplayUnavailable)? {
        let display = CGDisplay::new(id);
        // SAFETY: these read-only CoreGraphics queries accept active display IDs.
        let reserved = unsafe {
            is_remote_application_display(CGDisplayVendorNumber(id), CGDisplayModelNumber(id))
        };
        // Remote-application screens must never become mouse-shuttle targets.
        // Mirrored followers do not provide a distinct traversable desktop.
        if reserved || display.mirrors_display() != 0 {
            continue;
        }
        let bounds = display.bounds();
        // SAFETY: CoreGraphics accepts an active display ID and returns an owned
        // nullable CFUUID. Both Create-rule objects are released by their wrappers.
        let uuid = unsafe { CGDisplayCreateUUIDFromDisplayID(id) };
        if uuid.is_null() {
            return Err(PlatformError::DisplayUnavailable);
        }
        // SAFETY: non-null Create-rule UUID checked above.
        let uuid = unsafe { CFUUID::wrap_under_create_rule(uuid) };
        // SAFETY: uuid is retained and null selects the default CF allocator.
        let name = unsafe { CFUUIDCreateString(std::ptr::null(), uuid.as_concrete_TypeRef()) };
        if name.is_null() {
            return Err(PlatformError::DisplayUnavailable);
        }
        // SAFETY: CFUUIDCreateString returned a non-null owned CFString.
        let name = unsafe { CFString::wrap_under_create_rule(name) }.to_string();
        let scale = display
            .display_mode()
            .map(|mode| mode.pixel_width() as f64 / mode.width().max(1) as f64)
            .unwrap_or(1.0);
        // CoreGraphics reports the panel's EDID size in millimetres. Some
        // virtual displays return zero; those deliberately fall back to the
        // existing logical-point model.
        let physical = unsafe { CGDisplayScreenSize(id) };
        let physical_width_mm = (physical.width > 1.0).then_some(physical.width);
        let physical_height_mm = (physical.height > 1.0).then_some(physical.height);
        displays.push(deskfuse_core::DisplayDescriptor {
            id: name.into(),
            frame_x: bounds.origin.x,
            frame_y: bounds.origin.y,
            width: bounds.size.width.round() as u32,
            height: bounds.size.height.round() as u32,
            scale_factor: scale,
            rotation_degrees: (display.rotation().round() as u16) % 360,
            is_primary: id == CGDisplay::main().id,
            physical_width_mm,
            physical_height_mm,
        });
    }
    displays.sort_by(|a, b| a.id.cmp(&b.id));
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for d in &displays {
        d.id.hash(&mut hash);
        d.frame_x.to_bits().hash(&mut hash);
        d.frame_y.to_bits().hash(&mut hash);
        d.width.hash(&mut hash);
        d.height.hash(&mut hash);
        d.scale_factor.to_bits().hash(&mut hash);
        d.rotation_degrees.hash(&mut hash);
        d.is_primary.hash(&mut hash);
        d.physical_width_mm.map(f64::to_bits).hash(&mut hash);
        d.physical_height_mm.map(f64::to_bits).hash(&mut hash);
    }
    let topology = deskfuse_core::DisplayTopology {
        generation: hash.finish(),
        displays,
    };
    topology
        .validate()
        .map_err(|_| PlatformError::DisplayUnavailable)?;
    Ok(topology)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_deskfuse_application_displays_are_reserved() {
        assert!(is_remote_application_display(0x4453, 0x5241));
        assert!(!is_remote_application_display(0x4453, 1));
        assert!(!is_remote_application_display(0x0610, 0x5241));
        assert!(!is_remote_application_display(0, 0));
    }
}
