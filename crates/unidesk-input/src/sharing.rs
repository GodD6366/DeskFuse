//! Multi-device, multi-display sharing policy. All positions are logical points;
//! backing scale and physical rotation have already been applied by the OS.
#![allow(missing_docs)]
use crate::{Edge, Point, Rect, Vector2};
use std::collections::{BTreeMap, BTreeSet};
use unidesk_core::{DeviceId, DisplayDescriptor, DisplayKey, EdgeSwitchMode, SharedInputEvent};

pub const SHARING_MODIFIERS: u64 = (1 << 18) | (1 << 19) | (1 << 20);
const CHORD_MASK: u64 = SHARING_MODIFIERS | (1 << 17);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotkeyChord {
    pub code: u16,
    pub modifiers: u64,
}
impl HotkeyChord {
    pub fn matches(self, code: u16, modifiers: u64) -> bool {
        self.code == code && (self.modifiers & CHORD_MASK) == (modifiers & CHORD_MASK)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotkeyBindings {
    /// Left, right, up, down, in that order. Emergency reclaim is fixed.
    pub directional: [HotkeyChord; 4],
    pub lock: HotkeyChord,
}
impl Default for HotkeyBindings {
    fn default() -> Self {
        Self {
            directional: [123, 124, 126, 125].map(|code| HotkeyChord {
                code,
                modifiers: SHARING_MODIFIERS,
            }),
            lock: HotkeyChord {
                code: 37,
                modifiers: SHARING_MODIFIERS,
            },
        }
    }
}
impl HotkeyBindings {
    pub fn matches(&self, code: u16, modifiers: u64) -> bool {
        Self::is_emergency(code, modifiers)
            || self.lock.matches(code, modifiers)
            || self
                .directional
                .iter()
                .any(|key| key.matches(code, modifiers))
    }
    pub fn is_emergency(code: u16, modifiers: u64) -> bool {
        HotkeyChord {
            code: 53,
            modifiers: SHARING_MODIFIERS,
        }
        .matches(code, modifiers)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct SharingDisplay {
    pub key: DisplayKey,
    pub display: DisplayDescriptor,
    pub x: f64,
    pub y: f64,
    /// The application admits only devices with a current pre-granted lease.
    pub ready: bool,
}
impl SharingDisplay {
    fn rect(&self) -> Rect {
        Rect::new(
            self.x,
            self.y,
            f64::from(self.display.width),
            f64::from(self.display.height),
        )
    }
    fn clamp(&self, p: Point) -> Point {
        Point::new(
            p.x.clamp(0.0, f64::from(self.display.width.saturating_sub(1))),
            p.y.clamp(0.0, f64::from(self.display.height.saturating_sub(1))),
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum SharingAction {
    Crossed {
        from: DisplayKey,
        to: DisplayKey,
        position: Point,
    },
    Route {
        target: DisplayKey,
        event: SharedInputEvent,
    },
    Reclaim,
    LockChanged(bool),
}
#[derive(Clone, Debug)]
struct EdgeAttempt {
    source: DisplayKey,
    target: DisplayKey,
    edge: Edge,
    since: u64,
    left: bool,
}
#[derive(Clone, Debug)]
pub struct SharingController {
    local: DeviceId,
    displays: BTreeMap<DisplayKey, SharingDisplay>,
    active: Option<DisplayKey>,
    position: Point,
    mode: EdgeSwitchMode,
    locked: bool,
    buttons: BTreeSet<u8>,
    consumed_keys: BTreeSet<u16>,
    hotkeys: HotkeyBindings,
    attempt: Option<EdgeAttempt>,
    entry_edge: Option<Edge>,
    disabled_corners: u8,
    corner_size: f64,
    modifiers: u64,
    held_keys: BTreeSet<u16>,
    await_key_up: BTreeSet<u16>,
}
impl SharingController {
    pub fn new(local: DeviceId) -> Self {
        Self {
            local,
            displays: BTreeMap::new(),
            active: None,
            position: Point::default(),
            mode: EdgeSwitchMode::Instant,
            locked: false,
            buttons: BTreeSet::new(),
            consumed_keys: BTreeSet::new(),
            hotkeys: HotkeyBindings::default(),
            attempt: None,
            entry_edge: None,
            disabled_corners: 0,
            corner_size: 16.0,
            modifiers: 0,
            held_keys: BTreeSet::new(),
            await_key_up: BTreeSet::new(),
        }
    }
    /// Rebuilding a layout resets edge gestures. Per-device display placement must
    /// be a translation of its native desktop; rearranging another Mac's screens
    /// would make physical input and synthetic positions disagree.
    pub fn set_layout(&mut self, displays: Vec<SharingDisplay>) -> Result<(), String> {
        let mut layout = BTreeMap::new();
        let mut offsets = BTreeMap::<DeviceId, (f64, f64)>::new();
        for d in displays {
            if !d.x.is_finite()
                || !d.y.is_finite()
                || d.display.width == 0
                || d.display.height == 0
                || !d.display.frame_x.is_finite()
                || !d.display.frame_y.is_finite()
                || d.key.display_id != d.display.id
                || !d.display.scale_factor.is_finite()
                || d.display.scale_factor <= 0.0
                || ![0, 90, 180, 270].contains(&d.display.rotation_degrees)
            {
                return Err("invalid display placement".into());
            }
            let offset = (d.x - d.display.frame_x, d.y - d.display.frame_y);
            if let Some(existing) = offsets.insert(d.key.device_id, offset)
                && ((existing.0 - offset.0).abs() > 0.01 || (existing.1 - offset.1).abs() > 0.01)
            {
                return Err("screens of one device must preserve native desktop positions".into());
            }
            if layout.insert(d.key.clone(), d).is_some() {
                return Err("duplicate display key".into());
            }
        }
        if !layout.values().any(|d| d.key.device_id == self.local) {
            return Err("local display missing".into());
        }
        if layout == self.displays {
            return Ok(());
        }
        self.displays = layout;
        self.attempt = None;
        self.entry_edge = None;
        // The application must reclaim before applying topology or readiness changes.
        if self.active.as_ref().is_none_or(|key| {
            !self
                .displays
                .get(key)
                .is_some_and(|d| d.ready || key.device_id == self.local)
        }) {
            self.reclaim();
        }
        Ok(())
    }
    pub fn set_disabled_corners(&mut self, mask: u8, size: f64) {
        let size = if size.is_finite() {
            size.clamp(0.0, 256.0)
        } else {
            16.0
        };
        if self.disabled_corners != mask & 0x0f || self.corner_size != size {
            self.disabled_corners = mask & 0x0f;
            self.corner_size = size;
            self.attempt = None;
        }
    }
    pub fn set_mode(&mut self, mode: EdgeSwitchMode) {
        if mode != self.mode {
            self.mode = mode;
            self.attempt = None;
        }
    }
    pub fn set_hotkeys(&mut self, hotkeys: HotkeyBindings) {
        if self.hotkeys != hotkeys {
            self.hotkeys = hotkeys;
            self.consumed_keys.clear();
        }
    }
    pub fn active_display(&self) -> Option<&DisplayKey> {
        self.active.as_ref()
    }
    pub fn position(&self) -> Point {
        self.position
    }
    pub fn is_remote(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|d| d.device_id != self.local)
    }
    pub fn is_locked(&self) -> bool {
        self.locked
    }
    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        self.attempt = None;
    }
    pub fn buttons_pressed(&self) -> bool {
        !self.buttons.is_empty()
    }
    pub fn reclaim(&mut self) {
        self.active = None;
        self.attempt = None;
        self.entry_edge = None;
        self.buttons.clear();
        self.consumed_keys.clear();
        self.locked = false;
        self.held_keys.clear();
        self.await_key_up.clear();
    }
    pub fn handle_input(
        &mut self,
        event: SharedInputEvent,
        global: Point,
        delta: Vector2,
        now_ms: u64,
    ) -> Vec<SharingAction> {
        let mut actions = Vec::new();
        // Hotkeys and buttons can be the first sample after capture starts.
        // Establish the physical local display for every kind of input.
        if !self.is_remote()
            && let Some(d) = self.displays.values().find(|d| {
                d.key.device_id == self.local
                    && global.x >= d.display.frame_x
                    && global.y >= d.display.frame_y
                    && global.x < d.display.frame_x + f64::from(d.display.width)
                    && global.y < d.display.frame_y + f64::from(d.display.height)
            })
        {
            if self.active.as_ref() != Some(&d.key) {
                self.attempt = None;
                self.entry_edge = None;
            }
            self.active = Some(d.key.clone());
            self.position = Point::new(global.x - d.display.frame_x, global.y - d.display.frame_y);
        }
        if let SharedInputEvent::Modifiers { modifiers } = event {
            self.modifiers = modifiers;
        }
        if let SharedInputEvent::Key {
            code,
            pressed,
            modifiers,
            repeat,
        } = event
        {
            self.modifiers = modifiers;
            if self.await_key_up.contains(&code) {
                if !pressed {
                    self.await_key_up.remove(&code);
                    self.held_keys.remove(&code);
                    self.consumed_keys.remove(&code);
                }
                return actions;
            }
            if pressed {
                self.held_keys.insert(code);
            } else {
                self.held_keys.remove(&code);
            }
            if !pressed && self.consumed_keys.remove(&code) {
                return actions;
            }
            if pressed
                && (self.consumed_keys.contains(&code) || self.hotkeys.matches(code, modifiers))
            {
                self.consumed_keys.insert(code);
                if repeat {
                    return actions;
                }
                if HotkeyBindings::is_emergency(code, modifiers) {
                    self.reclaim();
                    self.consumed_keys.insert(code);
                    return vec![SharingAction::Reclaim];
                }
                if self.hotkeys.lock.matches(code, modifiers) {
                    self.set_locked(!self.locked);
                    return vec![SharingAction::LockChanged(self.locked)];
                }
                if let Some(index) = self
                    .hotkeys
                    .directional
                    .iter()
                    .position(|key| key.matches(code, modifiers))
                {
                    self.switch_edge(
                        [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom][index],
                        now_ms,
                        true,
                        &mut actions,
                    );
                }
                return actions;
            }
        }
        if let SharedInputEvent::Button { button, pressed } = event {
            if pressed {
                self.buttons.insert(button);
                self.attempt = None;
            } else {
                self.buttons.remove(&button);
            }
        }
        if matches!(event, SharedInputEvent::Motion { .. }) {
            if !global.x.is_finite()
                || !global.y.is_finite()
                || !delta.x.is_finite()
                || !delta.y.is_finite()
            {
                return actions;
            }
            if self.is_remote() {
                self.position.x += delta.x;
                self.position.y += delta.y;
            } else if let Some(d) = self.displays.values().find(|d| {
                d.key.device_id == self.local
                    && global.x >= d.display.frame_x
                    && global.y >= d.display.frame_y
                    && global.x < d.display.frame_x + f64::from(d.display.width)
                    && global.y < d.display.frame_y + f64::from(d.display.height)
            }) {
                if self.active.as_ref() != Some(&d.key) {
                    self.attempt = None;
                    self.entry_edge = None;
                }
                self.active = Some(d.key.clone());
                self.position =
                    Point::new(global.x - d.display.frame_x, global.y - d.display.frame_y);
            }
            self.check_edges(now_ms, &mut actions);
            if let Some(d) = self.active.as_ref().and_then(|key| self.displays.get(key)) {
                self.position = d.clamp(self.position);
            }
            if self.is_remote() {
                actions.push(SharingAction::Route {
                    target: self.active.clone().expect("remote has display"),
                    event: SharedInputEvent::Motion {
                        x: self.position.x,
                        y: self.position.y,
                    },
                });
            }
        } else if self.is_remote() {
            actions.push(SharingAction::Route {
                target: self.active.clone().expect("remote has display"),
                event,
            });
        }
        actions
    }
    /// Called even with no new mouse samples, so a stationary dwell still fires.
    pub fn tick(&mut self, now_ms: u64) -> Vec<SharingAction> {
        let mut actions = Vec::new();
        self.check_edges(now_ms, &mut actions);
        actions
    }
    fn check_edges(&mut self, now: u64, actions: &mut Vec<SharingAction>) {
        let Some(d) = self.active.as_ref().and_then(|key| self.displays.get(key)) else {
            return;
        };
        let bounds = Rect::new(
            0.0,
            0.0,
            f64::from(d.display.width.saturating_sub(1)),
            f64::from(d.display.height.saturating_sub(1)),
        );
        if let Some(entry) = self.entry_edge
            && bounds.signed_distance_to_edge(self.position, entry) > 8.0
        {
            self.entry_edge = None;
        }
        let left = self.position.x < self.corner_size;
        let right = self.position.x >= bounds.width - self.corner_size;
        let top = self.position.y < self.corner_size;
        let bottom = self.position.y >= bounds.height - self.corner_size;
        if [
            (left && top, 1),
            (right && top, 2),
            (left && bottom, 4),
            (right && bottom, 8),
        ]
        .into_iter()
        .any(|(inside, bit)| inside && self.disabled_corners & bit != 0)
        {
            self.attempt = None;
            return;
        }
        let edges = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];
        let edge = edges.into_iter().find(|edge| {
            bounds.signed_distance_to_edge(self.position, *edge) <= 0.0
                && self.neighbor(*edge, false).is_some_and(|(target, _)| {
                    // macOS/Windows already move the physical pointer between
                    // this host's native displays. Warping here interrupts that
                    // motion. Explicit directional hotkeys still use switch_edge.
                    d.key.device_id != self.local || target.device_id != self.local
                })
        });
        if let Some(edge) = edge {
            self.switch_edge(edge, now, false, actions);
        } else if let Some(attempt) = &mut self.attempt {
            let another_edge = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom]
                .into_iter()
                .any(|edge| {
                    edge != attempt.edge
                        && bounds.signed_distance_to_edge(self.position, edge) <= 0.0
                });
            if another_edge {
                self.attempt = None;
                return;
            }
            if self.mode == EdgeSwitchMode::DoubleTap500Ms
                && now.saturating_sub(attempt.since) <= 500
            {
                attempt.left = true;
            } else {
                self.attempt = None;
            }
        }
    }
    fn neighbor(&self, edge: Edge, directional: bool) -> Option<(DisplayKey, Point)> {
        let source = self.displays.get(self.active.as_ref()?)?;
        let a = source.rect();
        let along = if edge.is_horizontal() {
            a.x + self.position.x
        } else {
            a.y + self.position.y
        };
        let mut candidates = self
            .displays
            .values()
            .filter(|d| d.key != source.key && (d.ready || d.key.device_id == self.local))
            .filter_map(|target| {
                let b = target.rect();
                let touches = match edge {
                    Edge::Left => (a.x - b.right()).abs() < 0.01,
                    Edge::Right => (a.right() - b.x).abs() < 0.01,
                    Edge::Top => (a.y - b.bottom()).abs() < 0.01,
                    Edge::Bottom => (a.bottom() - b.y).abs() < 0.01,
                };
                let (sa, se) = a.edge_span(edge);
                let (ta, te) = b.edge_span(edge);
                let start = sa.max(ta);
                let end = se.min(te);
                if !touches
                    || end - start <= 0.01
                    || (!directional && !(start..end).contains(&along))
                {
                    return None;
                }
                let mapped = if directional {
                    along.clamp(start, end - 1.0_f64.min((end - start) / 2.0))
                } else {
                    along
                };
                let p = match edge {
                    Edge::Left => Point::new(b.width - 10.0, mapped - b.y),
                    Edge::Right => Point::new(10.0, mapped - b.y),
                    Edge::Top => Point::new(mapped - b.x, b.height - 10.0),
                    Edge::Bottom => Point::new(mapped - b.x, 10.0),
                };
                Some((target.key.clone(), target.clamp(p), (mapped - along).abs()))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
        let best = candidates.first()?;
        // Identical overlapping placements are ambiguous; fail closed.
        if candidates
            .get(1)
            .is_some_and(|next| (next.2 - best.2).abs() < 0.01)
        {
            return None;
        }
        Some((best.0.clone(), best.1))
    }
    fn switch_edge(
        &mut self,
        edge: Edge,
        now: u64,
        directional: bool,
        actions: &mut Vec<SharingAction>,
    ) {
        let Some((target, position)) = self.neighbor(edge, directional) else {
            self.attempt = None;
            return;
        };
        let source = self.active.clone().expect("neighbor has active display");
        let device_cross = source.device_id != target.device_id;
        if self.locked
            || (device_cross && !self.buttons.is_empty())
            || (!directional && self.entry_edge == Some(edge))
        {
            self.attempt = None;
            return;
        }
        let fire = if !device_cross || directional || self.mode == EdgeSwitchMode::Instant {
            true
        } else {
            if self.attempt.as_ref().is_none_or(|a| {
                a.source != source
                    || a.target != target
                    || a.edge != edge
                    || (self.mode == EdgeSwitchMode::DoubleTap500Ms
                        && now.saturating_sub(a.since) > 500)
            }) {
                self.attempt = Some(EdgeAttempt {
                    source: source.clone(),
                    target: target.clone(),
                    edge,
                    since: now,
                    left: false,
                });
            }
            let attempt = self.attempt.as_ref().expect("attempt initialized");
            match self.mode {
                EdgeSwitchMode::Dwell250Ms => now.saturating_sub(attempt.since) >= 250,
                EdgeSwitchMode::DoubleTap500Ms => {
                    attempt.left && now.saturating_sub(attempt.since) <= 500
                }
                EdgeSwitchMode::Instant => true,
            }
        };
        if fire {
            if device_cross {
                self.await_key_up.extend(self.held_keys.iter().copied());
            }
            self.active = Some(target.clone());
            self.position = position;
            self.attempt = None;
            self.entry_edge = Some(edge.opposite());
            actions.push(SharingAction::Crossed {
                from: source,
                to: target.clone(),
                position,
            });
            if target.device_id != self.local {
                actions.push(SharingAction::Route {
                    target,
                    event: SharedInputEvent::Modifiers {
                        modifiers: self.modifiers,
                    },
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn device(n: u8) -> DeviceId {
        DeviceId::from_bytes([n; 32])
    }
    fn display(
        dev: u8,
        id: &str,
        os: (f64, f64),
        placed: (f64, f64),
        size: (u32, u32),
    ) -> SharingDisplay {
        SharingDisplay {
            key: DisplayKey {
                device_id: device(dev),
                display_id: id.into(),
            },
            display: DisplayDescriptor {
                id: id.into(),
                frame_x: os.0,
                frame_y: os.1,
                width: size.0,
                height: size.1,
                scale_factor: 2.0,
                rotation_degrees: 90,
                is_primary: true,
            },
            x: placed.0,
            y: placed.1,
            ready: true,
        }
    }
    fn controller(mode: EdgeSwitchMode) -> SharingController {
        let mut c = SharingController::new(device(1));
        c.set_layout(vec![
            display(1, "a", (0.0, 0.0), (0.0, 0.0), (100, 100)),
            display(2, "b", (-100.0, 0.0), (100.0, 0.0), (100, 100)),
            display(2, "c", (0.0, 0.0), (200.0, 0.0), (100, 100)),
        ])
        .unwrap();
        c.set_mode(mode);
        c
    }
    fn motion(
        c: &mut SharingController,
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
        t: u64,
    ) -> Vec<SharingAction> {
        c.handle_input(
            SharedInputEvent::Motion { x, y },
            Point::new(x, y),
            Vector2::new(dx, dy),
            t,
        )
    }
    fn crossed(a: &[SharingAction]) -> bool {
        a.iter().any(|a| matches!(a, SharingAction::Crossed { .. }))
    }
    #[test]
    fn first_sample_instant_and_bounce_guard() {
        let mut c = controller(EdgeSwitchMode::Instant);
        assert!(crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 0)));
        assert!(!crossed(&motion(&mut c, 99.0, 50.0, -2.0, 0.0, 1)));
        assert!(c.is_remote());
        motion(&mut c, 99.0, 50.0, 12.0, 0.0, 2);
        assert!(crossed(&motion(&mut c, 99.0, 50.0, -20.0, 0.0, 3)));
        assert!(!c.is_remote());
    }
    #[test]
    fn stationary_dwell_fires_at_250() {
        let mut c = controller(EdgeSwitchMode::Dwell250Ms);
        assert!(!crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 0)));
        assert!(!crossed(&c.tick(249)));
        assert!(crossed(&c.tick(250)));
    }
    #[test]
    fn double_tap_needs_leave_same_edge_within_500() {
        let mut c = controller(EdgeSwitchMode::DoubleTap500Ms);
        motion(&mut c, 99.0, 50.0, 0.0, 0.0, 0);
        assert!(!crossed(&c.tick(250)));
        assert!(!crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 300)));
        motion(&mut c, 80.0, 50.0, 0.0, 0.0, 310);
        assert!(crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 400)));
    }
    #[test]
    fn double_tap_expiry_and_topology_reset() {
        let mut c = controller(EdgeSwitchMode::DoubleTap500Ms);
        motion(&mut c, 99.0, 50.0, 0.0, 0.0, 0);
        motion(&mut c, 80.0, 50.0, 0.0, 0.0, 100);
        assert!(!crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 501)));
        motion(&mut c, 80.0, 50.0, 0.0, 0.0, 550);
        let mut layout = c.displays.values().cloned().collect::<Vec<_>>();
        layout[0].display.scale_factor = 1.0;
        c.set_layout(layout).unwrap();
        assert!(!crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 600)));
    }
    #[test]
    fn held_button_blocks_device_but_not_remote_native_screens() {
        let mut c = controller(EdgeSwitchMode::Instant);
        c.handle_input(
            SharedInputEvent::Button {
                button: 4,
                pressed: true,
            },
            Point::default(),
            Vector2::default(),
            0,
        );
        assert!(!crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 1)));
        c.handle_input(
            SharedInputEvent::Button {
                button: 4,
                pressed: false,
            },
            Point::new(99.0, 50.0),
            Vector2::default(),
            2,
        );
        assert!(crossed(&c.tick(3)));
        c.handle_input(
            SharedInputEvent::Button {
                button: 0,
                pressed: true,
            },
            Point::default(),
            Vector2::default(),
            4,
        );
        assert!(crossed(&motion(&mut c, 99.0, 50.0, 100.0, 0.0, 5)));
        assert_eq!(c.active_display().unwrap().display_id.as_str(), "c");
    }
    #[test]
    fn only_chord_escape_reclaims_and_lock_blocks() {
        let mut c = controller(EdgeSwitchMode::Instant);
        motion(&mut c, 99.0, 50.0, 0.0, 0.0, 0);
        let key = |m| SharedInputEvent::Key {
            code: 53,
            pressed: true,
            modifiers: m,
            repeat: false,
        };
        assert!(matches!(
            c.handle_input(key(0), Point::default(), Vector2::default(), 1)[0],
            SharingAction::Route { .. }
        ));
        assert_eq!(
            c.handle_input(
                key(SHARING_MODIFIERS),
                Point::default(),
                Vector2::default(),
                2
            ),
            vec![SharingAction::Reclaim]
        );
        assert!(!c.is_remote());
        c.set_locked(true);
        assert!(!crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 3)));
    }
    #[test]
    fn negative_native_coordinates_scale_rotation_and_two_controller_screens() {
        let mut c = SharingController::new(device(1));
        let mut right = display(2, "b", (-500.0, -80.0), (100.0, -50.0), (80, 150));
        right.display.scale_factor = 1.0;
        c.set_layout(vec![
            display(1, "a", (0.0, 0.0), (0.0, 0.0), (100, 100)),
            display(1, "left", (-100.0, 0.0), (-100.0, 0.0), (100, 100)),
            right,
        ])
        .unwrap();
        motion(&mut c, -50.0, 50.0, 0.0, 0.0, 0);
        assert_eq!(c.active_display().unwrap().display_id.as_str(), "left");
        let a = motion(&mut c, 99.0, 40.0, 0.0, 0.0, 1);
        assert!(
            a.iter()
                .any(|a| matches!(a,SharingAction::Crossed{position,..} if position.y==90.0))
        );
    }
    #[test]
    fn native_local_seams_do_not_warp_but_hotkeys_still_switch() {
        for (second, edge, across, key) in [
            ((100.0, 0.0), (99.0, 50.0), (101.0, 50.0), 124),
            ((0.0, 100.0), (50.0, 99.0), (50.0, 101.0), 125),
            ((-100.0, 0.0), (0.0, 50.0), (-1.0, 50.0), 123),
            ((0.0, -100.0), (50.0, 0.0), (50.0, -1.0), 126),
        ] {
            let mut c = SharingController::new(device(1));
            c.set_layout(vec![
                display(1, "a", (0.0, 0.0), (0.0, 0.0), (100, 100)),
                display(1, "b", second, second, (100, 100)),
            ])
            .unwrap();
            c.set_mode(EdgeSwitchMode::Instant);
            assert!(motion(&mut c, edge.0, edge.1, 0.0, 0.0, 0).is_empty());
            assert!(c.tick(300).is_empty());
            assert!(motion(&mut c, across.0, across.1, 0.0, 0.0, 301).is_empty());
            assert_eq!(c.active_display().unwrap().display_id.as_str(), "b");
            motion(&mut c, 50.0, 50.0, 0.0, 0.0, 302);
            let actions = c.handle_input(
                SharedInputEvent::Key {
                    code: key,
                    pressed: true,
                    modifiers: SHARING_MODIFIERS,
                    repeat: false,
                },
                Point::new(50.0, 50.0),
                Vector2::default(),
                303,
            );
            assert!(crossed(&actions));
            assert_eq!(c.active_display().unwrap().display_id.as_str(), "b");
        }
    }

    #[test]
    fn offline_and_nonoverlapping_segments_never_cross() {
        let mut c = controller(EdgeSwitchMode::Instant);
        let mut layout = c.displays.values().cloned().collect::<Vec<_>>();
        for d in &mut layout {
            if d.key.device_id == device(2) {
                d.ready = false;
            }
        }
        c.set_layout(layout).unwrap();
        assert!(!crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 0)));
    }
    #[test]
    fn reject_per_device_screen_rearrangement() {
        let mut c = controller(EdgeSwitchMode::Instant);
        let mut layout = c.displays.values().cloned().collect::<Vec<_>>();
        layout[2].x += 1.0;
        assert!(c.set_layout(layout).is_err());
    }
    #[test]
    fn modifiers_cross_before_click_and_ordinary_held_key_does_not_repeat_on_new_peer() {
        let mut c = controller(EdgeSwitchMode::Instant);
        let flags = (1 << 17) | (1 << 20);
        c.handle_input(
            SharedInputEvent::Key {
                code: 0,
                pressed: true,
                modifiers: flags,
                repeat: false,
            },
            Point::new(50.0, 50.0),
            Vector2::default(),
            0,
        );
        let actions = motion(&mut c, 99.0, 50.0, 0.0, 0.0, 1);
        assert!(matches!(actions[0], SharingAction::Crossed { .. }));
        assert!(
            matches!(actions[1],SharingAction::Route{event:SharedInputEvent::Modifiers{modifiers},..} if modifiers==flags)
        );
        assert!(
            c.handle_input(
                SharedInputEvent::Key {
                    code: 0,
                    pressed: true,
                    modifiers: flags,
                    repeat: true
                },
                Point::new(99.0, 50.0),
                Vector2::default(),
                2
            )
            .is_empty()
        );
        assert!(
            c.handle_input(
                SharedInputEvent::Key {
                    code: 0,
                    pressed: false,
                    modifiers: flags,
                    repeat: false
                },
                Point::new(99.0, 50.0),
                Vector2::default(),
                3
            )
            .is_empty()
        );
        assert!(matches!(
            c.handle_input(
                SharedInputEvent::Button {
                    button: 0,
                    pressed: true
                },
                Point::new(99.0, 50.0),
                Vector2::default(),
                4
            )[0],
            SharingAction::Route {
                event: SharedInputEvent::Button { .. },
                ..
            }
        ));
    }
    #[test]
    fn hotkey_without_prior_motion_and_corner_disable() {
        let mut c = controller(EdgeSwitchMode::Dwell250Ms);
        let a = c.handle_input(
            SharedInputEvent::Key {
                code: 124,
                pressed: true,
                modifiers: SHARING_MODIFIERS,
                repeat: false,
            },
            Point::new(40.0, 40.0),
            Vector2::default(),
            0,
        );
        assert!(crossed(&a));
        assert!(
            c.handle_input(
                SharedInputEvent::Key {
                    code: 124,
                    pressed: false,
                    modifiers: 0,
                    repeat: false
                },
                Point::new(40.0, 40.0),
                Vector2::default(),
                1
            )
            .is_empty()
        );
        assert!(matches!(
            c.handle_input(
                SharedInputEvent::Key {
                    code: 124,
                    pressed: true,
                    modifiers: 0,
                    repeat: false
                },
                Point::new(40.0, 40.0),
                Vector2::default(),
                2
            )[0],
            SharingAction::Route {
                event: SharedInputEvent::Key { code: 124, .. },
                ..
            }
        ));
        c.reclaim();
        c.set_mode(EdgeSwitchMode::Instant);
        c.set_disabled_corners(2, 16.0);
        assert!(!crossed(&motion(&mut c, 99.0, 4.0, 0.0, 0.0, 1)));
        assert!(crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 2)));
    }
    #[test]
    fn touching_a_different_edge_resets_double_tap() {
        let mut c = controller(EdgeSwitchMode::DoubleTap500Ms);
        motion(&mut c, 99.0, 50.0, 0.0, 0.0, 0);
        motion(&mut c, 50.0, 0.0, 0.0, 0.0, 100);
        assert!(!crossed(&motion(&mut c, 99.0, 50.0, 0.0, 0.0, 200)));
    }
}
