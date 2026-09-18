//! Bounded placement memory, keyed by physical display geometry rather than network presence.
use super::*;
use std::fmt::Write;
use unidesk_core::{
    DisplayLayoutAttachment, DisplayLayoutProfile, DisplayTopology, InputSharingSettings,
};

// Stable, length-prefixed identities. Normalizing the native origin means changing
// the primary display without changing the arrangement is not a new environment.
fn topology_key(topology: &DisplayTopology) -> String {
    let min_x = topology
        .displays
        .iter()
        .map(|d| d.frame_x)
        .reduce(f64::min)
        .unwrap_or(0.0);
    let min_y = topology
        .displays
        .iter()
        .map(|d| d.frame_y)
        .reduce(f64::min)
        .unwrap_or(0.0);
    let mut displays: Vec<_> = topology.displays.iter().collect();
    displays.sort_by(|a, b| a.id.cmp(&b.id));
    let mut key = String::new();
    for d in displays {
        let _ = write!(
            key,
            "{}:{}:{},{},{},{},{},{};",
            d.id.0.len(),
            d.id,
            d.frame_x - min_x,
            d.frame_y - min_y,
            d.width,
            d.height,
            d.scale_factor,
            d.rotation_degrees
        );
    }
    key
}

pub(super) fn profile_for<'a>(
    snapshot: &'a InputSharingSnapshot,
    device: DeviceId,
    topology: &DisplayTopology,
) -> Option<&'a DisplayLayoutProfile> {
    let local_key = topology_key(snapshot.local_topology.as_ref()?);
    let device_key = topology_key(topology);
    snapshot.settings.layout_profiles.iter().rev().find(|p| {
        p.device_id == device && p.local_topology == local_key && p.device_topology == device_key
    })
}

pub(super) fn fallback_profile(
    snapshot: &InputSharingSnapshot,
    device: DeviceId,
) -> Option<&DisplayLayoutProfile> {
    let key = snapshot.local_topology.as_ref().map(topology_key);
    snapshot
        .settings
        .layout_profiles
        .iter()
        .rev()
        .find(|p| p.device_id == device && Some(&p.local_topology) == key.as_ref())
        .or_else(|| {
            snapshot
                .settings
                .layout_profiles
                .iter()
                .rev()
                .find(|p| p.device_id == device)
        })
}

/// Record new combinations automatically; only a user drag may replace a known
/// combination. Temporary reconnect/collision repairs must not rewrite history.
pub fn remember_layout(
    settings: &mut InputSharingSettings,
    snapshot: &InputSharingSnapshot,
    local: DeviceId,
    displays: &[SharingDisplay],
    replace: bool,
) {
    let Some(local_topology) = snapshot.local_topology.as_ref() else {
        return;
    };
    let local_key = topology_key(local_topology);
    let local_x = displays
        .iter()
        .filter(|d| d.key.device_id == local)
        .map(|d| d.x)
        .reduce(f64::min)
        .unwrap_or(0.0);
    let local_y = displays
        .iter()
        .filter(|d| d.key.device_id == local)
        .map(|d| d.y)
        .reduce(f64::min)
        .unwrap_or(0.0);
    let all_positions = placements(displays);
    let rects: Vec<_> = displays.iter().map(LayoutRect::from).collect();
    for (device, topology) in std::iter::once((local, local_topology))
        .chain(snapshot.remote_topologies.iter().map(|(id, t)| (*id, t)))
    {
        let mut positions: Vec<_> = all_positions
            .iter()
            .filter(|p| p.display.device_id == device)
            .cloned()
            .collect();
        if positions.is_empty() {
            continue;
        }
        let device_key = topology_key(topology);
        let existing = settings.layout_profiles.iter().position(|p| {
            p.device_id == device
                && p.local_topology == local_key
                && p.device_topology == device_key
        });
        if let Some(index) = existing {
            let previous = settings.layout_profiles.remove(index);
            if !replace {
                // Visiting an old combination refreshes recency, never its positions.
                settings.layout_profiles.push(previous);
                continue;
            }
        }
        if device != local {
            // Shared local screens can move at home without shifting an offline
            // office peer: remote placements are relative to the local desktop.
            for p in &mut positions {
                p.x -= local_x;
                p.y -= local_y;
            }
        }
        let attachment = if device == local {
            None
        } else {
            attachment_for(&rects, device)
        };
        settings.layout_profiles.push(DisplayLayoutProfile {
            device_id: device,
            local_topology: local_key.clone(),
            device_topology: device_key,
            placements: positions,
            attachment,
        });
        if settings.layout_profiles.len() > 128 {
            settings.layout_profiles.remove(0);
        }
    }
}

/// Remember a real seam, or the nearest facing edge for an already broken layout.
/// Cross-axis overlap excludes misleading diagonal edges of L-shaped desktops.
pub(super) fn attachment_for(
    rects: &[LayoutRect],
    device: DeviceId,
) -> Option<DisplayLayoutAttachment> {
    let mut candidates = Vec::new();
    for a in rects.iter().filter(|r| r.key.device_id == device) {
        for b in rects.iter().filter(|r| r.key.device_id != device) {
            if (a.y + a.height).min(b.y + b.height) - a.y.max(b.y) > 0.01 {
                candidates.push((
                    (b.x - a.x - a.width).abs(),
                    b.key.device_id,
                    DisplayLayoutSide::Left,
                ));
                candidates.push((
                    (a.x - b.x - b.width).abs(),
                    b.key.device_id,
                    DisplayLayoutSide::Right,
                ));
            }
            if (a.x + a.width).min(b.x + b.width) - a.x.max(b.x) > 0.01 {
                candidates.push((
                    (b.y - a.y - a.height).abs(),
                    b.key.device_id,
                    DisplayLayoutSide::Above,
                ));
                candidates.push((
                    (a.y - b.y - b.height).abs(),
                    b.key.device_id,
                    DisplayLayoutSide::Below,
                ));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    candidates
        .first()
        .map(|(_, neighbor, side)| DisplayLayoutAttachment {
            neighbor: *neighbor,
            side: *side,
        })
}
