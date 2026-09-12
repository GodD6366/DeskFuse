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
        for display in &topology.displays {
            if ready_only && !ready {
                continue;
            }
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
        cursor_x = cursor_x.max(offset_x + max_x);
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
                        if moving.iter().any(|m| {
                            stationary.iter().any(|n| {
                                (m.x + x + m.width).min(n.x + n.width) - (m.x + x).max(n.x) > 0.01
                                    && (m.y + y + m.height).min(n.y + n.height) - (m.y + y).max(n.y)
                                        > 0.01
                            })
                        }) {
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
