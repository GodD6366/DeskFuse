//! Native shortcut edges intercepted before ordinary cross-screen key routing.
//! Configuration and routing contain metadata only; audio never enters this path.
use super::SYNTHETIC_SHARING_EVENT_MARKER;
use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, EventField},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use deskfuse_core::{DeviceId, SharedInputEvent};
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

const FN: u64 = 1 << 23;
static LOCAL_MODIFIERS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INJECTED_MODIFIERS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Kept separate from ordinary input leases so crossing cannot release a hold.
pub fn protected_modifiers() -> u64 {
    INJECTED_MODIFIERS.load(std::sync::atomic::Ordering::Acquire)
        | LOCAL_MODIFIERS.load(std::sync::atomic::Ordering::Acquire)
}
const MASK: u64 = 0x1E0000;
// Aggregate flag plus side-specific NX_DEVICE* bit. Fn has no side-specific
// Quartz key code; a reported Fn/Globe event uses key code 63 on either side.
fn modifier_flags(code: u16) -> u64 {
    match code {
        54 => (1 << 20) | 0x10,
        55 => (1 << 20) | 0x8,
        56 => (1 << 17) | 0x2,
        60 => (1 << 17) | 0x4,
        58 => (1 << 19) | 0x20,
        61 => (1 << 19) | 0x40,
        59 => (1 << 18) | 0x1,
        62 => (1 << 18) | 0x2000,
        63 => FN,
        _ => 0,
    }
}
fn modifier_pressed(flags: u64, code: u16) -> bool {
    let owned = modifier_flags(code);
    let aggregate = owned & (MASK | FN);
    let side = owned & !(MASK | FN);
    let opposite = match code {
        54 => 0x8,
        55 => 0x10,
        56 => 0x4,
        60 => 0x2,
        58 => 0x40,
        61 => 0x20,
        59 => 0x2000,
        62 => 0x1,
        _ => 0,
    };
    // Some keyboards omit device-specific bits. The changed key code still
    // identifies the side, so use the aggregate flag in that case.
    flags & aggregate != 0 && (side == 0 || flags & side != 0 || flags & (side | opposite) == 0)
}
fn strip_modifier(flags: u64, code: u16) -> u64 {
    let owned = modifier_flags(code);
    if owned == 0 {
        return flags;
    }
    let opposite = match code {
        54 => 0x8,
        55 => 0x10,
        56 => 0x4,
        60 => 0x2,
        58 => 0x40,
        61 => 0x20,
        59 => 0x2000,
        62 => 0x1,
        _ => 0,
    };
    let aggregate = owned & (MASK | FN);
    let mut result = flags & !owned;
    if flags & opposite != 0 {
        result |= flags & aggregate;
    }
    result
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutConfig {
    pub enabled: bool,
    pub key_code: u16,
    pub modifiers: u64,
}
impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            key_code: 63,
            modifiers: 0,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ShortcutEdge {
    pub serial: u64,
    pub pressed: bool,
    pub generation: u64,
    pub target: DeviceId,
    pub routed: bool,
    pub config: ShortcutConfig,
    pub passthrough: bool,
}
#[derive(Default)]
struct State {
    config: ShortcutConfig,
    local: Option<DeviceId>,
    target: Option<DeviceId>,
    controller: Option<DeviceId>,
    receiving: bool,
    permitted: bool,
    generation: u64,
    serial: u64,
    held: Option<ShortcutEdge>,
    edges: VecDeque<ShortcutEdge>,
    modifier_down: bool,
}
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}
fn invalidate(s: &mut State) {
    s.generation = s.generation.wrapping_add(1);
    // A local physical down must retain its physical up, even if routing or
    // sharing changes while held. Only forwarded holds need a synthetic release.
    if s.held.is_some_and(|edge| !edge.passthrough)
        && let Some(mut edge) = s.held.take()
    {
        edge.pressed = false;
        s.edges.push_back(edge);
    }
}
pub fn configure(config: ShortcutConfig) {
    let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
    if s.config != config {
        invalidate(&mut s);
        s.config = config;
        if s.held.is_none() {
            s.modifier_down = false;
        }
        super::timing::record!(
            "microphone_shortcut_config",
            std::time::Duration::ZERO,
            format!(
                "enabled={} key_code={} modifiers={}",
                config.enabled, config.key_code, config.modifiers
            )
        );
    }
}
pub fn set_route(
    local: DeviceId,
    target: Option<DeviceId>,
    controller: Option<DeviceId>,
    receiving: bool,
    permitted: bool,
) {
    let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
    if s.permitted && !permitted {
        invalidate(&mut s);
    }
    s.local = Some(local);
    s.target = target;
    s.controller = controller;
    s.receiving = receiving;
    s.permitted = permitted;
}
pub fn cancel() {
    invalidate(&mut state().lock().unwrap_or_else(|e| e.into_inner()));
}
/// With the tap removed, physical releases go straight to macOS. Forget local
/// bookkeeping so re-enabling capture cannot inherit a hold whose up we missed.
pub fn capture_stopped() {
    let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
    invalidate(&mut s);
    s.held = None;
    s.modifier_down = false;
    LOCAL_MODIFIERS.store(0, std::sync::atomic::Ordering::Release);
}
pub fn generation() -> u64 {
    state().lock().unwrap_or_else(|e| e.into_inner()).generation
}
pub fn current_target() -> Option<DeviceId> {
    let s = state().lock().unwrap_or_else(|e| e.into_inner());
    (s.permitted && s.config.enabled)
        .then(|| s.target.or(s.local))
        .flatten()
}
pub fn filter_modifiers(modifiers: u64) -> u64 {
    let s = state().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(edge) = s.held {
        strip_modifier(modifiers, edge.config.key_code)
    } else if s.config.enabled && s.permitted {
        strip_modifier(modifiers, s.config.key_code)
    } else {
        modifiers
    }
}
pub fn config() -> ShortcutConfig {
    state().lock().unwrap_or_else(|e| e.into_inner()).config
}
pub fn take_edges() -> Vec<ShortcutEdge> {
    state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .edges
        .drain(..)
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureDisposition {
    Ordinary,
    Suppress,
    Passthrough,
}
/// Called after the normal tap's synthetic marker/PID exclusion. Passthrough
/// bypasses ordinary routing while delivering the original hardware event.
pub fn capture(event: &mut SharedInputEvent, changed_modifier: Option<u16>) -> CaptureDisposition {
    let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
    capture_with_state(&mut s, event, changed_modifier)
}

fn capture_with_state(
    s: &mut State,
    event: &mut SharedInputEvent,
    changed_modifier: Option<u16>,
) -> CaptureDisposition {
    use CaptureDisposition::*;
    let config = s.held.map_or(s.config, |e| e.config);
    let (pressed, matching, repeated) = match *event {
        SharedInputEvent::Modifiers { modifiers }
            if changed_modifier == Some(config.key_code)
                && modifier_flags(config.key_code) != 0 =>
        {
            let down = modifier_pressed(modifiers, config.key_code);
            let changed = down != s.modifier_down;
            s.modifier_down = down;
            (
                down,
                changed && (!down || strip_modifier(modifiers, config.key_code) & MASK == 0),
                false,
            )
        }
        SharedInputEvent::Key {
            code,
            pressed,
            modifiers,
            repeat,
        } => (
            pressed,
            code == config.key_code && (!pressed || (modifiers & MASK) == config.modifiers),
            repeat,
        ),
        _ => (false, false, false),
    };
    let enabled = s.config.enabled && s.permitted;
    if (enabled || s.held.is_some()) && modifier_flags(config.key_code) != 0 {
        match event {
            SharedInputEvent::Modifiers { modifiers } | SharedInputEvent::Key { modifiers, .. } => {
                *modifiers = strip_modifier(*modifiers, config.key_code)
            }
            _ => {}
        }
    }
    if !matching {
        return Ordinary;
    }
    if !pressed {
        if let Some(mut edge) = s.held.take() {
            super::timing::record!(
                "microphone_shortcut_release",
                std::time::Duration::ZERO,
                format!("target={} passthrough={}", edge.target, edge.passthrough)
            );
            if edge.passthrough {
                LOCAL_MODIFIERS.store(0, std::sync::atomic::Ordering::Release);
                return Passthrough;
            }
            edge.pressed = false;
            s.edges.push_back(edge);
            return Suppress;
        }
        return Ordinary;
    }
    if let Some(edge) = s.held {
        return if edge.passthrough {
            Passthrough
        } else {
            Suppress
        };
    }
    if !enabled || repeated {
        return Ordinary;
    }
    let Some(local) = s.local else {
        return Ordinary;
    };
    // A receiving lease means this controlled device is the current keyboard
    // target. Its physical Fn must not round-trip through the controller.
    let target = if s.receiving {
        local
    } else {
        s.controller.or(s.target).unwrap_or(local)
    };
    let passthrough = target == local;
    s.serial = s.serial.wrapping_add(1);
    let edge = ShortcutEdge {
        serial: s.serial,
        pressed: true,
        generation: s.generation,
        target,
        routed: s.controller.is_none(),
        config,
        passthrough,
    };
    if !passthrough && s.edges.len() >= 32 {
        s.edges.clear();
        invalidate(s);
        return Ordinary;
    }
    s.held = Some(edge);
    super::timing::record!(
        "microphone_shortcut_press",
        std::time::Duration::ZERO,
        format!(
            "target={target} passthrough={passthrough} routed={}",
            edge.routed
        )
    );
    if passthrough {
        LOCAL_MODIFIERS.store(
            config.modifiers | modifier_flags(config.key_code),
            std::sync::atomic::Ordering::Release,
        );
        Passthrough
    } else {
        s.edges.push_back(edge);
        Suppress
    }
}

/// A single injected hold; Drop releases it on timeout, disconnect and task cancel.
/// This injector owns only the configured shortcut and has no pointer effects.
pub struct Injection {
    config: ShortcutConfig,
}
impl Injection {
    pub fn press(config: ShortcutConfig) -> Result<Self, String> {
        if !super::can_post_input() {
            return Err("请授权辅助功能后使用麦克风快捷键".into());
        }
        let modifiers = config.modifiers | modifier_flags(config.key_code);
        INJECTED_MODIFIERS.store(modifiers, std::sync::atomic::Ordering::Release);
        if let Err(error) = post(config, true) {
            INJECTED_MODIFIERS.store(0, std::sync::atomic::Ordering::Release);
            let _ = post(config, false);
            return Err(error);
        }
        Ok(Self { config })
    }
}
impl Drop for Injection {
    fn drop(&mut self) {
        INJECTED_MODIFIERS.store(0, std::sync::atomic::Ordering::Release);
        let _ = post(self.config, false);
    }
}
fn post(config: ShortcutConfig, pressed: bool) -> Result<(), String> {
    // A standalone modifier uses the same system state for both edges
    // table, rather than two independent short-lived private state tables.
    let source_state = if modifier_flags(config.key_code) != 0 {
        CGEventSourceStateID::HIDSystemState
    } else {
        CGEventSourceStateID::Private
    };
    let source = CGEventSource::new(source_state).map_err(|_| "快捷键事件源不可用")?;
    let flags = if pressed {
        config.modifiers | modifier_flags(config.key_code)
    } else {
        0
    };
    let emit = |code: u16, modifier: bool| -> Result<(), String> {
        let event = CGEvent::new_keyboard_event(source.clone(), code, pressed)
            .map_err(|_| "快捷键事件不可用")?;
        if modifier {
            event.set_type(CGEventType::FlagsChanged);
        }
        event.set_flags(CGEventFlags::from_bits_retain(flags));
        event.set_integer_value_field(
            EventField::EVENT_SOURCE_USER_DATA,
            SYNTHETIC_SHARING_EVENT_MARKER,
        );
        event.post(CGEventTapLocation::HID);
        Ok(())
    };
    if pressed && modifier_flags(config.key_code) == 0 {
        for code in super::injector::modifier_keycodes(config.modifiers) {
            emit(code, true)?;
        }
    }
    emit(config.key_code, modifier_flags(config.key_code) != 0)?;
    if !pressed && modifier_flags(config.key_code) == 0 {
        for code in super::injector::modifier_keycodes(config.modifiers) {
            emit(code, true)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pressed: bool) -> SharedInputEvent {
        SharedInputEvent::Key {
            code: 49,
            pressed,
            modifiers: 0,
            repeat: false,
        }
    }

    #[test]
    fn shortcut_hold_keeps_original_target_across_crossings_and_releases_it() {
        let local = DeviceId::from_bytes([1; 32]);
        let first = DeviceId::from_bytes([2; 32]);
        let second = DeviceId::from_bytes([3; 32]);
        let mut state = State {
            config: ShortcutConfig {
                enabled: true,
                key_code: 49,
                modifiers: 0,
            },
            local: Some(local),
            target: Some(first),
            permitted: true,
            ..Default::default()
        };
        assert_eq!(
            capture_with_state(&mut state, &mut key(true), None),
            CaptureDisposition::Suppress
        );
        let press = state.edges.pop_front().unwrap();
        state.target = Some(second);
        // Repeated downs on a new screen cannot inject a second hold there.
        assert_eq!(
            capture_with_state(&mut state, &mut key(true), None),
            CaptureDisposition::Suppress
        );
        assert!(state.edges.is_empty());
        assert_eq!(
            capture_with_state(&mut state, &mut key(false), None),
            CaptureDisposition::Suppress
        );
        let release = state.edges.pop_front().unwrap();
        assert_eq!(release.target, first);
        assert_eq!(release.serial, press.serial);
        assert!(!release.pressed);
        assert!(state.held.is_none());
        capture_with_state(&mut state, &mut key(true), None);
        assert_eq!(state.edges.pop_front().unwrap().target, second);
    }

    #[test]
    fn shortcut_revocation_releases_forwarded_hold_once_and_invalidates_pending_press() {
        let mut state = State {
            config: ShortcutConfig {
                enabled: true,
                key_code: 49,
                modifiers: 0,
            },
            local: Some(DeviceId::from_bytes([1; 32])),
            target: Some(DeviceId::from_bytes([2; 32])),
            permitted: true,
            ..Default::default()
        };
        capture_with_state(&mut state, &mut key(true), None);
        let press = state.edges.pop_front().unwrap();
        invalidate(&mut state);
        state.permitted = false;
        let release = state.edges.pop_front().unwrap();
        assert_eq!(release.target, press.target);
        assert_eq!(release.serial, press.serial);
        assert!(!release.pressed);
        assert_ne!(press.generation, state.generation);
        invalidate(&mut state);
        capture_with_state(&mut state, &mut key(false), None);
        assert!(state.edges.is_empty());
    }
}
