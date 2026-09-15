//! Shared layout resolution and device-group magnetic placement.
use crate::sharing::SharingDisplay;
use unidesk_core::{DeviceId, DisplayKey, DisplayPlacementSetting, InputSharingSnapshot};

/// Translate complete native device desktops as groups. Never synthesize a
/// display when a peer has not supplied a topology.
pub fn layout_displays(
    snapshot: &InputSharingSnapshot,
    local: DeviceId,
    ready_only: bool,
) -> Vec<SharingDisplay> {
    let mut groups = Vec::new();
    if let Some(topology) = &snapshot.local_topology {
        groups.push((local, topology, true));
    }
    for (peer, topology) in &snapshot.remote_topologies {
        if !snapshot.settings.participants.contains(peer) {
            continue;
        }
        let ready = snapshot
            .links
            .iter()
            .any(|link| link.peer_id == *peer && link.ready && link.control_session.is_some());
        groups.push((*peer, topology, ready));
    }
    // Reserve saved desktops before placing newly seen devices. Otherwise a
    // default-position group can occupy the saved position of a returning peer.
    // The local desktop is always the fixed anchor; sort stably within each tier.
    groups.sort_by_key(|(device, topology, _)| {
        if *device == local {
            0
        } else if topology.displays.iter().any(|display| {
            snapshot
                .settings
                .placements
                .iter()
                .any(|p| p.display.device_id == *device && p.display.display_id == display.id)
        }) {
            1
        } else {
            2
        }
    });
    let mut cursor_x = 0.0_f64;
    let mut result = Vec::new();
    for (device_id, topology, ready) in groups {
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
        let max_x = topology
            .displays
            .iter()
            .map(|d| d.frame_x + f64::from(d.width))
            .reduce(f64::max)
            .unwrap_or(min_x);
        let configured_offset = topology.displays.iter().find_map(|display| {
            snapshot
                .settings
                .placements
                .iter()
                .find(|p| p.display.device_id == device_id && p.display.display_id == display.id)
                .map(|p| (p.x - display.frame_x, p.y - display.frame_y))
        });
        let (offset_x, offset_y) = configured_offset.unwrap_or((cursor_x - min_x, -min_y));
        let group_start = result.len();
        for display in &topology.displays {
            result.push(SharingDisplay {
                key: DisplayKey {
                    device_id,
                    display_id: display.id.clone(),
                },
                display: display.clone(),
                x: display.frame_x + offset_x,
                y: display.frame_y + offset_y,
                ready,
            });
        }
        // Saved origins refer to a previous native topology. Resolution/primary
        // display changes can make whole device desktops intersect after wake.
        // Repair only the incoming group, retaining its native screen offsets.
        let rects: Vec<_> = result.iter().map(LayoutRect::from).collect();
        let overlaps = rects[group_start..].iter().any(|a| {
            rects[..group_start]
                .iter()
                .any(|b| rectangles_overlap(a, b, 0.0, 0.0))
        });
        let (dx, dy) = if overlaps {
            snap_device_move(&rects, device_id, 0.0, 0.0, None)
                // Large layouts can exceed the drag-preview limit. Placing at
                // the occupied right edge still guarantees no device overlap.
                .unwrap_or((cursor_x - (offset_x + min_x), 0.0))
        } else {
            (0.0, 0.0)
        };
        for display in &mut result[group_start..] {
            display.x += dx;
            display.y += dy;
        }
        cursor_x = cursor_x.max(offset_x + max_x + dx);
    }
    // Unready desktops still reserve space, so UI and input routing resolve
    // identical coordinates and readiness changes cannot move other screens.
    if ready_only {
        result.retain(|display| display.ready);
    }
    result
}

/// Lightweight geometry also used by the native canvas for drag previews.
#[derive(Clone, Debug)]
pub struct LayoutRect {
    /// Persistent screen identity.
    pub key: DisplayKey,
    /// Shared desktop horizontal origin.
    pub x: f64,
    /// Shared desktop vertical origin.
    pub y: f64,
    /// Logical width.
    pub width: f64,
    /// Logical height.
    pub height: f64,
}
impl From<&SharingDisplay> for LayoutRect {
    fn from(d: &SharingDisplay) -> Self {
        Self {
            key: d.key.clone(),
            x: d.x,
            y: d.y,
            width: f64::from(d.display.width),
            height: f64::from(d.display.height),
        }
    }
}
fn rectangles_overlap(a: &LayoutRect, b: &LayoutRect, dx: f64, dy: f64) -> bool {
    (a.x + dx + a.width).min(b.x + b.width) - (a.x + dx).max(b.x) > 0.01
        && (a.y + dy + a.height).min(b.y + b.height) - (a.y + dy).max(b.y) > 0.01
}
/// All screens are persisted together, so moving one cannot shift an unsaved neighbor.
pub fn placements(displays: &[SharingDisplay]) -> Vec<DisplayPlacementSetting> {
    displays
        .iter()
        .map(|d| DisplayPlacementSetting {
            display: d.key.clone(),
            x: d.x,
            y: d.y,
        })
        .collect()
}
/// Snap a whole device to a non-overlapping shared edge. `None` attaches on drop;
/// a distance limits magnetic feedback while dragging. Native screen offsets stay intact.
pub fn snap_device_move(
    rects: &[LayoutRect],
    device: DeviceId,
    dx: f64,
    dy: f64,
    distance: Option<f64>,
) -> Result<(f64, f64), String> {
    if !dx.is_finite()
        || !dy.is_finite()
        || distance.is_some_and(|d| !d.is_finite() || d < 0.0)
        || rects.len() > 128
        || rects.iter().any(|r| {
            !r.x.is_finite()
                || !r.y.is_finite()
                || !r.width.is_finite()
                || !r.height.is_finite()
                || r.width <= 0.0
                || r.height <= 0.0
        })
    {
        return Err("无效的屏幕位置".into());
    }
    let moving: Vec<_> = rects.iter().filter(|r| r.key.device_id == device).collect();
    let stationary: Vec<_> = rects.iter().filter(|r| r.key.device_id != device).collect();
    if moving.is_empty() {
        return Err("设备显示器已断开，请重新调整布局".into());
    }
    if stationary.is_empty() {
        return Ok((dx, dy));
    }
    let alignment = distance.unwrap_or(48.0);
    let mut candidates = Vec::new();
    for a in &moving {
        for b in &stationary {
            // Try all four seams, including cross-axis endpoints for irregular multi-screen groups.
            for horizontal in [true, false] {
                let (a_start, a_size, b_start, b_size, desired) = if horizontal {
                    (a.y, a.height, b.y, b.height, dy)
                } else {
                    (a.x, a.width, b.x, b.width, dx)
                };
                let overlap = 32.0_f64.min(a_size / 2.0).min(b_size / 2.0);
                let low = b_start - a_start - a_size + overlap;
                let high = b_start + b_size - a_start - overlap;
                let aligned = [
                    b_start - a_start,
                    b_start + b_size - a_start - a_size,
                    b_start + b_size / 2.0 - a_start - a_size / 2.0,
                ];
                let mut along = desired.clamp(low, high);
                if let Some(best) = aligned
                    .iter()
                    .min_by(|a, b| (**a - desired).abs().total_cmp(&(**b - desired).abs()))
                    && (*best - desired).abs() <= alignment
                {
                    along = *best;
                }
                for cross in [along, low, high] {
                    let seams = if horizontal {
                        [b.x - a.x - a.width, b.x + b.width - a.x]
                    } else {
                        [b.y - a.y - a.height, b.y + b.height - a.y]
                    };
                    for seam in seams {
                        let (x, y) = if horizontal {
                            (seam, cross)
                        } else {
                            (cross, seam)
                        };
                        let correction = (x - dx).hypot(y - dy);
                        if distance.is_some_and(|d| correction > d) {
                            continue;
                        }
                        if moving
                            .iter()
                            .any(|m| stationary.iter().any(|n| rectangles_overlap(m, n, x, y)))
                        {
                            continue;
                        }
                        candidates.push((correction, x, y));
                    }
                }
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
    });
    match candidates.first() {
        Some((_, x, y)) => Ok((*x, *y)),
        None if distance.is_some() => Ok((dx, dy)),
        None => Err("找不到可相接且不重叠的位置，请调整相邻设备".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unidesk_core::{DisplayDescriptor, DisplayTopology, InputPeerStatus, InputSessionId};

    fn device(seed: u8) -> DeviceId {
        DeviceId::from_bytes([seed; 32])
    }
    fn screen(id: &str, x: f64, y: f64, width: u32, height: u32) -> DisplayDescriptor {
        DisplayDescriptor {
            id: id.into(),
            frame_x: x,
            frame_y: y,
            width,
            height,
            scale_factor: 2.0,
            rotation_degrees: 0,
            is_primary: x == 0.0 && y == 0.0,
        }
    }
    fn topology(displays: Vec<DisplayDescriptor>) -> DisplayTopology {
        DisplayTopology {
            generation: 1,
            displays,
        }
    }
    fn no_device_overlap(displays: &[SharingDisplay]) {
        for a in displays {
            for b in displays {
                if a.key.device_id != b.key.device_id {
                    assert!(!rectangles_overlap(&a.into(), &b.into(), 0.0, 0.0));
                }
            }
        }
    }
    fn coordinates(displays: &[SharingDisplay]) -> Vec<(DisplayKey, f64, f64)> {
        displays.iter().map(|d| (d.key.clone(), d.x, d.y)).collect()
    }

    #[test]
    fn reconnect_repairs_overlap_without_changing_native_multiscreen_offsets() {
        let local = device(1);
        let peer = device(2);
        let mut snapshot = InputSharingSnapshot {
            local_topology: Some(topology(vec![
                screen("internal", -1512.0, 1000.0, 1512, 982),
                screen("external", 0.0, 0.0, 1920, 1080),
            ])),
            ..Default::default()
        };
        snapshot.settings.participants.push(peer);
        snapshot
            .remote_topologies
            .insert(peer, topology(vec![screen("remote", 0.0, 0.0, 2048, 1152)]));
        snapshot.settings.placements = vec![
            DisplayPlacementSetting {
                display: DisplayKey {
                    device_id: local,
                    display_id: "internal".into(),
                },
                x: -1512.0,
                y: 1000.0,
            },
            DisplayPlacementSetting {
                display: DisplayKey {
                    device_id: peer,
                    display_id: "remote".into(),
                },
                x: -1928.0,
                y: -88.0,
            },
        ];
        let saved = snapshot.settings.clone();
        let resolved = layout_displays(&snapshot, local, false);
        no_device_overlap(&resolved);
        assert_eq!((resolved[0].x, resolved[0].y), (-1512.0, 1000.0));
        assert_eq!((resolved[1].x, resolved[1].y), (0.0, 0.0));
        assert_ne!((resolved[2].x, resolved[2].y), (-1928.0, -88.0));
        // The repair does not rewrite saved settings during a transient topology.
        assert_eq!(snapshot.settings, saved);
        let remote = snapshot.remote_topologies.remove(&peer).unwrap();
        let _ = layout_displays(&snapshot, local, false);
        snapshot.remote_topologies.insert(peer, remote);
        assert_eq!(
            coordinates(&resolved),
            coordinates(&layout_displays(&snapshot, local, false))
        );
    }

    #[test]
    fn new_display_identity_does_not_occupy_another_devices_saved_position() {
        let local = device(1);
        let fresh = device(2);
        let saved = device(3);
        let mut snapshot = InputSharingSnapshot {
            local_topology: Some(topology(vec![screen("local", 0.0, 0.0, 100, 100)])),
            ..Default::default()
        };
        snapshot.settings.participants = vec![fresh, saved];
        for peer in [fresh, saved] {
            snapshot
                .remote_topologies
                .insert(peer, topology(vec![screen("current", 0.0, 0.0, 100, 100)]));
            snapshot.settings.placements.push(DisplayPlacementSetting {
                display: DisplayKey {
                    device_id: peer,
                    display_id: if peer == fresh { "replaced" } else { "current" }.into(),
                },
                x: if peer == fresh { 200.0 } else { 100.0 },
                y: 0.0,
            });
        }
        let resolved = layout_displays(&snapshot, local, false);
        no_device_overlap(&resolved);
        assert_eq!(
            resolved
                .iter()
                .find(|d| d.key.device_id == saved)
                .unwrap()
                .x,
            100.0
        );
        assert_eq!(
            resolved
                .iter()
                .find(|d| d.key.device_id == fresh)
                .unwrap()
                .x,
            200.0
        );
    }

    #[test]
    fn unready_devices_reserve_the_same_space_in_canvas_and_input_routing() {
        let local = device(1);
        let mut snapshot = InputSharingSnapshot {
            local_topology: Some(topology(vec![screen("local", 0.0, 0.0, 100, 100)])),
            ..Default::default()
        };
        for peer in [device(2), device(3)] {
            snapshot.settings.participants.push(peer);
            snapshot
                .remote_topologies
                .insert(peer, topology(vec![screen("remote", 0.0, 0.0, 100, 100)]));
            snapshot.settings.placements.push(DisplayPlacementSetting {
                display: DisplayKey {
                    device_id: peer,
                    display_id: "remote".into(),
                },
                x: 100.0,
                y: 0.0,
            });
        }
        snapshot.links.push(InputPeerStatus {
            peer_id: device(3),
            context: None,
            ready: true,
            reason: None,
            control_session: Some(InputSessionId::new()),
            lease_revision: 1,
            motion_mode: None,
            motion_fallback_reason: None,
        });
        let canvas = layout_displays(&snapshot, local, false);
        no_device_overlap(&canvas);
        let expected = canvas.into_iter().filter(|d| d.ready).collect::<Vec<_>>();
        assert_eq!(
            coordinates(&layout_displays(&snapshot, local, true)),
            coordinates(&expected)
        );
    }
}
