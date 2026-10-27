//! Shared layout resolution and device-group magnetic placement.
use crate::sharing::SharingDisplay;
use deskfuse_core::{
    DeviceId, DisplayKey, DisplayLayoutAttachment, DisplayLayoutSide, DisplayPlacementSetting,
    InputSharingSnapshot,
};
mod profiles;
pub use profiles::remember_layout;

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
        } else if profiles::profile_for(snapshot, *device, topology).is_some()
            || topology.displays.iter().any(|display| {
                snapshot
                    .settings
                    .placements
                    .iter()
                    .any(|p| p.display.device_id == *device && p.display.display_id == display.id)
            })
        {
            1
        } else {
            2
        }
    });
    let mut cursor_x = 0.0_f64;
    let mut result: Vec<SharingDisplay> = Vec::new();
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
        let profile = profiles::profile_for(snapshot, device_id, topology)
            .or_else(|| profiles::fallback_profile(snapshot, device_id));
        let saved = profile
            .map(|p| p.placements.as_slice())
            .unwrap_or(&snapshot.settings.placements);
        let reference_x = if profile.is_some() && device_id != local {
            result
                .iter()
                .filter(|d| d.key.device_id == local)
                .map(|d| d.x)
                .reduce(f64::min)
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let reference_y = if profile.is_some() && device_id != local {
            result
                .iter()
                .filter(|d| d.key.device_id == local)
                .map(|d| d.y)
                .reduce(f64::min)
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let configured_offset = topology.displays.iter().find_map(|display| {
            saved
                .iter()
                .find(|p| p.display.device_id == device_id && p.display.display_id == display.id)
                .map(|p| {
                    (
                        p.x + reference_x - display.frame_x,
                        p.y + reference_y - display.frame_y,
                    )
                })
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
        // Repair overlap without changing a device's native internal arrangement.
        // Gap repair waits until ALL groups exist: a saved desktop may connect
        // through another device that is later in the discovery order.
        let rects: Vec<_> = result.iter().map(LayoutRect::from).collect();
        let overlaps = rects[group_start..].iter().any(|a| {
            rects[..group_start]
                .iter()
                .any(|b| rectangles_overlap(a, b, 0.0, 0.0))
        });
        let attachment = profile
            .and_then(|p| p.attachment.clone())
            .or_else(|| profiles::attachment_for(&rects, device_id));
        let (dx, dy) = if overlaps {
            snap_device_move_with_attachment(&rects, device_id, 0.0, 0.0, None, attachment.as_ref())
                .or_else(|_| snap_device_move(&rects, device_id, 0.0, 0.0, None))
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
    repair_layout_gaps(&mut result, snapshot, local);
    // Unready desktops still reserve space, so UI and input routing resolve
    // identical coordinates and readiness changes cannot move other screens.
    if ready_only {
        result.retain(|display| display.ready);
    }
    result
}

/// Connect orphaned desktops only after resolving the full graph. Otherwise a
/// valid A-B-C layout could be reordered merely because C was discovered before B.
fn repair_layout_gaps(
    displays: &mut [SharingDisplay],
    snapshot: &InputSharingSnapshot,
    local: DeviceId,
) {
    use std::collections::BTreeSet;
    let devices: BTreeSet<_> = displays.iter().map(|d| d.key.device_id).collect();
    if !devices.contains(&local) {
        return;
    }
    for _ in 0..devices.len() {
        let rects: Vec<_> = displays.iter().map(LayoutRect::from).collect();
        let mut connected = BTreeSet::from([local]);
        loop {
            let count = connected.len();
            for a in &rects {
                if rects
                    .iter()
                    .any(|b| connected.contains(&b.key.device_id) && rectangles_touch(a, b))
                {
                    connected.insert(a.key.device_id);
                }
            }
            if connected.len() == count {
                break;
            }
        }
        if connected.len() == devices.len() {
            return;
        }
        let mut candidates = Vec::new();
        for device in devices.difference(&connected).copied() {
            let profile = snapshot
                .remote_topologies
                .get(&device)
                .and_then(|t| profiles::profile_for(snapshot, device, t))
                .or_else(|| profiles::fallback_profile(snapshot, device));
            let facing: Vec<_> = rects
                .iter()
                .filter(|r| r.key.device_id == device || connected.contains(&r.key.device_id))
                .cloned()
                .collect();
            let preferred = profile
                .and_then(|p| p.attachment.clone())
                .or_else(|| profiles::attachment_for(&facing, device));
            for neighbor in &connected {
                for side in [
                    DisplayLayoutSide::Left,
                    DisplayLayoutSide::Right,
                    DisplayLayoutSide::Above,
                    DisplayLayoutSide::Below,
                ] {
                    let attachment = DisplayLayoutAttachment {
                        neighbor: *neighbor,
                        side,
                    };
                    // Candidate seams belong to the connected component; collision
                    // checks still include every device, including other orphans.
                    if let Ok((dx, dy)) = snap_device_move_with_attachment(
                        &rects,
                        device,
                        0.0,
                        0.0,
                        None,
                        Some(&attachment),
                    ) {
                        let rank = preferred.as_ref().map_or(0, |p| {
                            u8::from(p.side != side) * 2 + u8::from(p.neighbor != *neighbor)
                        });
                        candidates.push((rank, dx.hypot(dy), device, dx, dy));
                    }
                }
            }
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        let Some((_, _, device, dx, dy)) = candidates.first() else {
            return;
        };
        for display in displays.iter_mut().filter(|d| d.key.device_id == *device) {
            display.x += dx;
            display.y += dy;
        }
    }
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
fn rectangles_touch(a: &LayoutRect, b: &LayoutRect) -> bool {
    let vertical = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    let horizontal = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    (vertical > 0.01
        && ((a.x + a.width - b.x).abs() <= 0.001 || (b.x + b.width - a.x).abs() <= 0.001))
        || (horizontal > 0.01
            && ((a.y + a.height - b.y).abs() <= 0.001 || (b.y + b.height - a.y).abs() <= 0.001))
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
    snap_device_move_with_attachment(rects, device, dx, dy, distance, None)
}
fn snap_device_move_with_attachment(
    rects: &[LayoutRect],
    device: DeviceId,
    dx: f64,
    dy: f64,
    distance: Option<f64>,
    attachment: Option<&DisplayLayoutAttachment>,
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
    let preferred_horizontal = moving
        .iter()
        .flat_map(|a| stationary.iter().map(move |b| {
            let overlap_x = (a.x + dx + a.width).min(b.x + b.width) - (a.x + dx).max(b.x);
            let overlap_y = (a.y + dy + a.height).min(b.y + b.height) - (a.y + dy).max(b.y);
            (overlap_x, overlap_y, overlap_x * overlap_y)
        }))
        .filter(|(_, _, area)| *area > 0.0)
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(overlap_x, overlap_y, _)| overlap_x <= overlap_y)
        .unwrap_or_else(|| dx.abs() >= dy.abs());
    let mut candidates = Vec::new();
    for a in &moving {
        for b in &stationary {
            if attachment.is_some_and(|a| {
                a.neighbor != b.key.device_id
                    && stationary.iter().any(|r| r.key.device_id == a.neighbor)
            }) {
                continue;
            }
            // Preserve the axis of the smaller overlap. A nearly aligned
            // vertical edge must win over a closer top/bottom seam.
            for horizontal in [preferred_horizontal] {
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
                    for (index, seam) in seams.into_iter().enumerate() {
                        let side = match (horizontal, index) {
                            (true, 0) => DisplayLayoutSide::Left,
                            (true, _) => DisplayLayoutSide::Right,
                            (false, 0) => DisplayLayoutSide::Above,
                            (false, _) => DisplayLayoutSide::Below,
                        };
                        if attachment.is_some_and(|a| a.side != side) {
                            continue;
                        }
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
    use deskfuse_core::{DisplayDescriptor, DisplayTopology, InputPeerStatus, InputSessionId};

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
            physical_width_mm: None,
            physical_height_mm: None,
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

    fn remember(snapshot: &mut InputSharingSnapshot, local: DeviceId, replace: bool) {
        let displays = layout_displays(snapshot, local, false);
        let mut settings = snapshot.settings.clone();
        remember_layout(&mut settings, snapshot, local, &displays, replace);
        snapshot.settings = settings;
        snapshot.settings.validate().unwrap();
    }

    fn laptop_and_monitor() -> InputSharingSnapshot {
        let local = device(1);
        let peer = device(2);
        let mut snapshot = InputSharingSnapshot {
            local_topology: Some(topology(vec![screen("local", 0.0, 0.0, 200, 100)])),
            remote_topologies: std::collections::BTreeMap::from([(
                peer,
                topology(vec![
                    screen("monitor", -100.0, 0.0, 100, 100),
                    screen("laptop", 0.0, 0.0, 100, 100),
                ]),
            )]),
            ..Default::default()
        };
        snapshot.settings.participants = vec![peer];
        snapshot.settings.placements = vec![
            DisplayPlacementSetting {
                display: DisplayKey {
                    device_id: local,
                    display_id: "local".into(),
                },
                x: 0.0,
                y: 0.0,
            },
            DisplayPlacementSetting {
                display: DisplayKey {
                    device_id: peer,
                    display_id: "monitor".into(),
                },
                x: -200.0,
                y: 0.0,
            },
            DisplayPlacementSetting {
                display: DisplayKey {
                    device_id: peer,
                    display_id: "laptop".into(),
                },
                x: -100.0,
                y: 0.0,
            },
        ];
        snapshot
    }

    #[test]
    fn clamshell_repairs_gap_and_reopening_restores_original_pair() {
        let local = device(1);
        let peer = device(2);
        let mut snapshot = laptop_and_monitor();
        let original = coordinates(&layout_displays(&snapshot, local, false));
        let open_topology = snapshot.remote_topologies[&peer].clone();
        remember(&mut snapshot, local, false);
        snapshot
            .remote_topologies
            .insert(peer, topology(vec![screen("monitor", 0.0, 0.0, 100, 100)]));
        let closed = layout_displays(&snapshot, local, false);
        let monitor = closed.iter().find(|d| d.key.device_id == peer).unwrap();
        assert_eq!((monitor.x, monitor.y), (-100.0, 0.0));
        assert!(rectangles_touch(
            &LayoutRect::from(monitor),
            &LayoutRect::from(&closed[0])
        ));
        no_device_overlap(&closed);
        remember(&mut snapshot, local, true);
        snapshot.remote_topologies.insert(peer, open_topology);
        assert_eq!(
            coordinates(&layout_displays(&snapshot, local, false)),
            original
        );
    }

    #[test]
    fn different_local_screen_combination_does_not_overwrite_office_layout() {
        let local = device(1);
        let mut snapshot = laptop_and_monitor();
        let office = snapshot.local_topology.clone();
        let original = coordinates(&layout_displays(&snapshot, local, false));
        remember(&mut snapshot, local, false);
        snapshot.local_topology = Some(topology(vec![screen("home", 0.0, 0.0, 150, 100)]));
        let mut home = layout_displays(&snapshot, local, false);
        for display in home.iter_mut().filter(|d| d.key.device_id != local) {
            display.x += 350.0;
        }
        let mut settings = snapshot.settings.clone();
        remember_layout(&mut settings, &snapshot, local, &home, true);
        snapshot.settings = settings;
        snapshot.local_topology = office;
        assert_eq!(
            coordinates(&layout_displays(&snapshot, local, false)),
            original
        );
        // Neither native primary-origin rebasing nor generation changes identify a new setup.
        let count = snapshot.settings.layout_profiles.len();
        let topology = snapshot.local_topology.as_mut().unwrap();
        topology.generation = 999;
        topology.displays[0].frame_x = 300.0;
        topology.displays[0].frame_y = -20.0;
        assert_eq!(
            coordinates(&layout_displays(&snapshot, local, false)),
            original
        );
        remember(&mut snapshot, local, false);
        assert_eq!(snapshot.settings.layout_profiles.len(), count);
    }

    #[test]
    fn offline_office_peer_keeps_relative_position_when_local_desktop_moves_at_home() {
        let local = device(1);
        let peer = device(2);
        let mut snapshot = laptop_and_monitor();
        remember(&mut snapshot, local, false);
        let remote = snapshot.remote_topologies.remove(&peer).unwrap();
        let mut displays = layout_displays(&snapshot, local, false);
        for d in &mut displays {
            d.x += 500.0;
            d.y += 200.0;
        }
        let mut settings = snapshot.settings.clone();
        remember_layout(&mut settings, &snapshot, local, &displays, true);
        snapshot.settings = settings;
        snapshot.remote_topologies.insert(peer, remote);
        let restored = layout_displays(&snapshot, local, false);
        let local_screen = restored.iter().find(|d| d.key.device_id == local).unwrap();
        let remote_screen = restored
            .iter()
            .find(|d| d.key.display_id == deskfuse_core::DisplayId::from("laptop"))
            .unwrap();
        assert_eq!(
            (
                remote_screen.x - local_screen.x,
                remote_screen.y - local_screen.y
            ),
            (-100.0, 0.0)
        );
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
    fn saved_chain_remains_connected_when_far_device_is_enumerated_first() {
        let local = device(1);
        let far = device(2);
        let near = device(3);
        let mut snapshot = InputSharingSnapshot {
            local_topology: Some(topology(vec![screen("local", 0.0, 0.0, 100, 100)])),
            ..Default::default()
        };
        snapshot.settings.participants = vec![far, near];
        for (peer, x) in [(far, 200.0), (near, 100.0)] {
            snapshot
                .remote_topologies
                .insert(peer, topology(vec![screen("remote", 0.0, 0.0, 100, 100)]));
            snapshot.settings.placements.push(DisplayPlacementSetting {
                display: DisplayKey {
                    device_id: peer,
                    display_id: "remote".into(),
                },
                x,
                y: 0.0,
            });
        }
        let resolved = layout_displays(&snapshot, local, false);
        assert_eq!(
            resolved.iter().find(|d| d.key.device_id == far).unwrap().x,
            200.0
        );
        assert_eq!(
            resolved.iter().find(|d| d.key.device_id == near).unwrap().x,
            100.0
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
