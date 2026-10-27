//! Ordinary interactive-desktop input injection, not authorization or a service.
//!
//! The session controller must authorize the peer and release input before
//! relinquishing control. This adapter does not enter Winlogon/UAC desktops or
//! bypass UIPI. `SendInput` cannot identify UIPI as the specific failure cause.
//! There is no arbitrary command, legacy Quartz, or SAS pathway.
use std::fmt;

/// A loop-suppression marker, not proof of identity or authorization.
pub const INPUT_INJECTION_MARKER: usize = 0x5544_534b;
#[must_use]
pub fn is_own_injected_event(extra_info: usize) -> bool {
    extra_info == INPUT_INJECTION_MARKER
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectionOutcome {
    Injected,
    /// An idempotent release/press already matched our owned input state.
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputError {
    UnsupportedUsage(u16),
    ModifierMustUseIndependentState,
    SecureAttentionSequenceUnsupported,
    InvalidVirtualDesktop,
    PixelOutsideVirtualDesktop,
    DesktopUnavailable,
    NativeCall {
        operation: &'static str,
        code: u32,
    },
    /// May be blocked input, UIPI or a desktop race. Error code is not definitive.
    InjectionRejected {
        requested: u32,
        inserted: u32,
        code: u32,
    },
}
impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedUsage(_) => f.write_str("unsupported HID usage"),
            _ => write!(f, "{self:?}"),
        }
    }
}
impl std::error::Error for InputError {}

/// Physical virtual-desktop pixels, not GPUI logical points. Supply a fresh
/// per-monitor-DPI-aware topology including its negative origin if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualDesktopPixels {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}
impl VirtualDesktopPixels {
    /// Windows documents endpoints 0 and 65535 as the surface's first and last
    /// pixel. Reject stale/out-of-range coordinates rather than silently clamp.
    /// # Errors
    /// Returns invalid topology or an out-of-bounds physical coordinate.
    pub fn normalize(self, x: i32, y: i32) -> Result<(i32, i32), InputError> {
        if self.width == 0
            || self.height == 0
            || i64::from(self.left) + i64::from(self.width) - 1 > i64::from(i32::MAX)
            || i64::from(self.top) + i64::from(self.height) - 1 > i64::from(i32::MAX)
        {
            return Err(InputError::InvalidVirtualDesktop);
        }
        let dx = i64::from(x) - i64::from(self.left);
        let dy = i64::from(y) - i64::from(self.top);
        if dx < 0 || dy < 0 || dx >= i64::from(self.width) || dy >= i64::from(self.height) {
            return Err(InputError::PixelOutsideVirtualDesktop);
        }
        let scale = |value: i64, size: u32| -> Result<i32, InputError> {
            if size == 1 {
                Ok(0)
            } else {
                i32::try_from((value * 65_535) / i64::from(size - 1))
                    .map_err(|_| InputError::InvalidVirtualDesktop)
            }
        };
        Ok((scale(dx, self.width)?, scale(dy, self.height)?))
    }
}

#[cfg(any(target_os = "windows", test))]
fn is_secure_attention_chord(control: bool, alt: bool, delete: bool) -> bool {
    control && alt && delete
}

#[cfg(target_os = "windows")]
pub use native::OrdinaryDesktopInjector;

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod native {
    use super::{
        INPUT_INJECTION_MARKER, InjectionOutcome, InputError, MouseButton, VirtualDesktopPixels,
        is_secure_attention_chord,
    };
    use crate::hid::{HidModifiers, ScanCode, keyboard_usage_to_scan_code};
    use std::{collections::BTreeSet, marker::PhantomData, mem::size_of, rc::Rc};
    use windows_sys::Win32::{
        Foundation::{GetLastError, SetLastError},
        System::{
            StationsAndDesktops::{
                CloseDesktop, DESKTOP_READOBJECTS, GetThreadDesktop, GetUserObjectInformationW,
                HDESK, OpenInputDesktop, UOI_NAME,
            },
            Threading::GetCurrentThreadId,
        },
        UI::Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
            KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSE_EVENT_FLAGS,
            MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
            MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
            MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
            MOUSEEVENTF_XUP, MOUSEINPUT, SendInput, VK_CONTROL, VK_DECIMAL, VK_DELETE, VK_MENU,
        },
    };

    /// Owned, thread-bound state. No worker or desktop switching is hidden here.
    /// After a desktop transition Drop cannot guarantee releases reach the old
    /// desktop: release before switching and handle failures in the session
    /// controller. Other desktops require a separate service/input helper.
    pub struct OrdinaryDesktopInjector {
        keys: BTreeSet<ScanCode>,
        buttons: BTreeSet<MouseButton>,
        _thread_bound: PhantomData<Rc<()>>,
    }
    impl OrdinaryDesktopInjector {
        /// Requires thread and active input desktop to be Default. This does not
        /// establish permission to control higher-integrity applications.
        /// # Errors
        /// Returns unsupported input, unavailable desktop, or a native injection failure.
        pub fn new() -> Result<Self, InputError> {
            ensure_default_input_desktop()?;
            Ok(Self {
                keys: BTreeSet::new(),
                buttons: BTreeSet::new(),
                _thread_bound: PhantomData,
            })
        }
        #[must_use]
        pub fn held_input_count(&self) -> usize {
            self.keys.len() + self.buttons.len()
        }

        /// HID key press/release. Side-specific modifiers use `set_modifiers`.
        /// Repeated down events are injected, preserving keyboard repeat.
        /// # Errors
        /// Returns unsupported input, unavailable desktop, or a native injection failure.
        pub fn keyboard(&mut self, usage: u16, down: bool) -> Result<InjectionOutcome, InputError> {
            if (0xe0..=0xe7).contains(&usage) {
                return Err(InputError::ModifierMustUseIndependentState);
            }
            let scan =
                keyboard_usage_to_scan_code(usage).ok_or(InputError::UnsupportedUsage(usage))?;
            // Lock keys toggle on down, and must not toggle again when a Mac
            // reports another flagsChanged event during the same held press.
            if down && matches!(usage, 0x39 | 0x47 | 0x53) && self.keys.contains(&scan) {
                return Ok(InjectionOutcome::Unchanged);
            }
            self.key(scan, down)
        }

        /// Releases precede presses. State changes only after each successful
        /// `SendInput`; partial failures leave accurately tracked held state.
        /// # Errors
        /// Returns unsupported input, unavailable desktop, or a native injection failure.
        pub fn set_modifiers(&mut self, modifiers: HidModifiers) -> Result<(), InputError> {
            let all: BTreeSet<_> = (0xe0..=0xe7)
                .filter_map(keyboard_usage_to_scan_code)
                .collect();
            let desired: BTreeSet<_> = modifiers
                .usages()
                .filter_map(keyboard_usage_to_scan_code)
                .collect();
            let mut prospective = self.keys.clone();
            prospective.retain(|scan| !all.contains(scan));
            prospective.extend(&desired);
            guard_sas(&prospective)?;
            for scan in &all {
                if self.keys.contains(scan) && !desired.contains(scan) {
                    self.key(*scan, false)?;
                }
            }
            for scan in desired {
                if !self.keys.contains(&scan) {
                    self.key(scan, true)?;
                }
            }
            Ok(())
        }

        fn key(&mut self, scan: ScanCode, down: bool) -> Result<InjectionOutcome, InputError> {
            if !down && !self.keys.contains(&scan) {
                return Ok(InjectionOutcome::Unchanged);
            }
            if down {
                let mut prospective = self.keys.clone();
                prospective.insert(scan);
                guard_sas(&prospective)?;
            }
            let flags = KEYEVENTF_SCANCODE
                | if scan.extended {
                    KEYEVENTF_EXTENDEDKEY
                } else {
                    0
                }
                | if down { 0 } else { KEYEVENTF_KEYUP };
            send_one(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: 0,
                        wScan: scan.code,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: INPUT_INJECTION_MARKER,
                    },
                },
            })?;
            if down {
                self.keys.insert(scan);
            } else {
                self.keys.remove(&scan);
            }
            Ok(InjectionOutcome::Injected)
        }

        /// # Errors
        /// Returns unsupported input, unavailable desktop, or a native injection failure.
        pub fn move_absolute(
            &self,
            desktop: VirtualDesktopPixels,
            x: i32,
            y: i32,
        ) -> Result<(), InputError> {
            let (dx, dy) = desktop.normalize(x, y)?;
            send_mouse(
                dx,
                dy,
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            )
        }

        /// 设备计数相对移动。不带绝对坐标标志，也不改本机保存的绝对位置。
        /// # Errors
        /// Returns unsupported input, unavailable desktop, or a native injection failure.
        pub fn move_relative(&self, dx: i32, dy: i32) -> Result<(), InputError> {
            if dx == 0 && dy == 0 {
                return Ok(());
            }
            send_mouse(dx, dy, 0, MOUSEEVENTF_MOVE)
        }

        /// # Errors
        /// Returns unsupported input, unavailable desktop, or a native injection failure.
        pub fn mouse_button(
            &mut self,
            button: MouseButton,
            down: bool,
        ) -> Result<InjectionOutcome, InputError> {
            if self.buttons.contains(&button) == down {
                return Ok(InjectionOutcome::Unchanged);
            }
            let (flag, data) = match (button, down) {
                (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
                (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
                (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                (MouseButton::Back, true) => (MOUSEEVENTF_XDOWN, 1),
                (MouseButton::Back, false) => (MOUSEEVENTF_XUP, 1),
                (MouseButton::Forward, true) => (MOUSEEVENTF_XDOWN, 2),
                (MouseButton::Forward, false) => (MOUSEEVENTF_XUP, 2),
            };
            send_mouse(0, 0, data, flag)?;
            if down {
                self.buttons.insert(button);
            } else {
                self.buttons.remove(&button);
            }
            Ok(InjectionOutcome::Injected)
        }

        /// Native wheel units: 120 is one notch; positive is up/right. Fractional
        /// notches are supported, interpreted by the receiving application.
        /// # Errors
        /// Returns unsupported input, unavailable desktop, or a native injection failure.
        pub fn scroll(&self, vertical: i32, horizontal: i32) -> Result<(), InputError> {
            if vertical != 0 {
                send_mouse(
                    0,
                    0,
                    u32::from_ne_bytes(vertical.to_ne_bytes()),
                    MOUSEEVENTF_WHEEL,
                )?;
            }
            if horizontal != 0 {
                send_mouse(
                    0,
                    0,
                    u32::from_ne_bytes(horizontal.to_ne_bytes()),
                    MOUSEEVENTF_HWHEEL,
                )?;
            }
            Ok(())
        }
        /// Windows 应用把 Ctrl+滚轮当作缩放，精确式触控板的捏合也是这样上报的。
        /// 用户已按住 Ctrl 时不重复按下，也不替用户松开。
        /// # Errors
        /// Returns unsupported input, unavailable desktop, or a native injection failure.
        pub fn zoom(&mut self, wheel: i32) -> Result<(), InputError> {
            if wheel == 0 {
                return Ok(());
            }
            let held = [0xe0, 0xe4]
                .into_iter()
                .filter_map(keyboard_usage_to_scan_code)
                .any(|scan| self.keys.contains(&scan));
            if held {
                return self.scroll(wheel, 0);
            }
            let control =
                keyboard_usage_to_scan_code(0xe0).ok_or(InputError::UnsupportedUsage(0xe0))?;
            self.key(control, true)?;
            let result = self.scroll(wheel, 0);
            let release = self.key(control, false);
            result?;
            release.map(|_| ())
        }
        /// # Errors
        /// Returns unsupported input, unavailable desktop, or a native injection failure.
        pub fn secure_attention_sequence(&self) -> Result<(), InputError> {
            Err(InputError::SecureAttentionSequenceUnsupported)
        }

        /// Release physical input that local applications already saw, before an
        /// exclusive capture handover (mirrors macOS `release_local_keyboard`).
        /// Unlike `release_all`, the key/button ups are injected without touching
        /// the tracked injected state — the physical edges were never tracked
        /// here — and carry the process marker so the capture hook ignores them.
        /// Attempts every release even after failure.
        /// # Errors
        /// Returns the first native injection failure after attempting the rest.
        pub fn release_local_physical(
            &mut self,
            usages: &[u16],
            buttons: u8,
        ) -> Result<(), InputError> {
            let mut first_error = None;
            for &usage in usages {
                let Some(scan) = keyboard_usage_to_scan_code(usage) else {
                    continue;
                };
                let flags = KEYEVENTF_SCANCODE
                    | KEYEVENTF_KEYUP
                    | if scan.extended {
                        KEYEVENTF_EXTENDEDKEY
                    } else {
                        0
                    };
                if let Err(error) = send_one(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: 0,
                            wScan: scan.code,
                            dwFlags: flags,
                            time: 0,
                            dwExtraInfo: INPUT_INJECTION_MARKER,
                        },
                    },
                }) {
                    first_error.get_or_insert(error);
                }
            }
            for (button, bit) in [
                (MouseButton::Left, 1),
                (MouseButton::Right, 2),
                (MouseButton::Middle, 4),
                (MouseButton::Back, 8),
                (MouseButton::Forward, 16),
            ] {
                if buttons & bit == 0 {
                    continue;
                }
                let (flag, data) = match button {
                    MouseButton::Left => (MOUSEEVENTF_LEFTUP, 0),
                    MouseButton::Right => (MOUSEEVENTF_RIGHTUP, 0),
                    MouseButton::Middle => (MOUSEEVENTF_MIDDLEUP, 0),
                    MouseButton::Back => (MOUSEEVENTF_XUP, 1),
                    MouseButton::Forward => (MOUSEEVENTF_XUP, 2),
                };
                if let Err(error) = send_mouse(0, 0, data, flag) {
                    first_error.get_or_insert(error);
                }
            }
            if let Some(error) = first_error {
                return Err(error);
            }
            Ok(())
        }

        /// Attempt every owned release even after failure. Failed releases remain
        /// tracked for retry; the owner must inspect errors before dropping.
        /// # Errors
        /// Returns unsupported input, unavailable desktop, or a native injection failure.
        pub fn release_all(&mut self) -> Result<(), InputError> {
            let mut first_error = None;
            for scan in self.keys.clone() {
                if let Err(error) = self.key(scan, false) {
                    first_error.get_or_insert(error);
                }
            }
            for button in self.buttons.clone() {
                if let Err(error) = self.mouse_button(button, false) {
                    first_error.get_or_insert(error);
                }
            }
            first_error.map_or(Ok(()), Err)
        }
    }
    impl Drop for OrdinaryDesktopInjector {
        fn drop(&mut self) {
            let _ = self.release_all();
        }
    }

    fn send_mouse(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> Result<(), InputError> {
        send_one(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: INPUT_INJECTION_MARKER,
                },
            },
        })
    }
    // INPUT is a fixed small Win32 ABI structure, well within an i32 byte length.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    fn send_one(input: INPUT) -> Result<(), InputError> {
        ensure_default_input_desktop()?;
        // SAFETY: initialized union matches type; pointer remains valid during
        // this synchronous one-element call. Clear thread-local stale LastError.
        let (inserted, code) = unsafe {
            SetLastError(0);
            let inserted = SendInput(1, &raw const input, size_of::<INPUT>() as i32);
            (inserted, GetLastError())
        };
        if inserted != 1 {
            return Err(InputError::InjectionRejected {
                requested: 1,
                inserted,
                code,
            });
        }
        Ok(())
    }

    fn guard_sas(keys: &BTreeSet<ScanCode>) -> Result<(), InputError> {
        ensure_default_input_desktop()?;
        // SendInput does not reset physical local keys. This preflight is not a
        // security boundary: local input can race it. Windows itself refuses
        // SendInput as a SAS mechanism.
        let held = |vk: u16| {
            // SAFETY: scalar virtual-key identifier; no pointers.
            unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
        };
        let control = keys.iter().any(|scan| scan.code == 0x1d) || held(VK_CONTROL);
        let alt = keys.iter().any(|scan| scan.code == 0x38) || held(VK_MENU);
        let delete =
            keys.iter().any(|scan| scan.code == 0x53) || held(VK_DELETE) || held(VK_DECIMAL);
        if is_secure_attention_chord(control, alt, delete) {
            Err(InputError::SecureAttentionSequenceUnsupported)
        } else {
            Ok(())
        }
    }

    struct OwnedDesktop(HDESK);
    impl Drop for OwnedDesktop {
        fn drop(&mut self) {
            // SAFETY: unique owner of handle returned by OpenInputDesktop.
            unsafe {
                CloseDesktop(self.0);
            }
        }
    }
    fn ensure_default_input_desktop() -> Result<(), InputError> {
        // SAFETY: scalar read-only request, non-inheritable; no desktop switch.
        let active = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
        if active.is_null() {
            return Err(InputError::DesktopUnavailable);
        }
        let active = OwnedDesktop(active);
        // SAFETY: borrowed desktop for this live thread; never closed here.
        let current = unsafe { GetThreadDesktop(GetCurrentThreadId()) };
        if current.is_null() || !desktop_is_default(current)? || !desktop_is_default(active.0)? {
            return Err(InputError::DesktopUnavailable);
        }
        Ok(())
    }
    // The fixed 128-unit UTF-16 array has a 256-byte length.
    #[allow(clippy::cast_possible_truncation)]
    fn desktop_is_default(desktop: HDESK) -> Result<bool, InputError> {
        let mut name = [0u16; 128];
        let mut needed = 0;
        // SAFETY: live desktop handle, writable UTF-16 array with its byte length,
        // and valid length out-pointer; API does not retain these pointers.
        let success = unsafe {
            GetUserObjectInformationW(
                desktop,
                UOI_NAME,
                name.as_mut_ptr().cast(),
                size_of_val(&name) as u32,
                &raw mut needed,
            )
        };
        if success == 0 {
            // SAFETY: immediate thread-local error read after failed API call.
            return Err(InputError::NativeCall {
                operation: "GetUserObjectInformationW",
                code: unsafe { GetLastError() },
            });
        }
        let end = name
            .iter()
            .position(|value| *value == 0)
            .ok_or(InputError::DesktopUnavailable)?;
        Ok(String::from_utf16_lossy(&name[..end]).eq_ignore_ascii_case("Default"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absolute_coordinates_cover_negative_origin_and_endpoints() {
        let desktop = VirtualDesktopPixels {
            left: -1920,
            top: -200,
            width: 4480,
            height: 1640,
        };
        assert_eq!(desktop.normalize(-1920, -200), Ok((0, 0)));
        assert_eq!(desktop.normalize(2559, 1439), Ok((65535, 65535)));
        assert_eq!(
            desktop.normalize(-1921, 0),
            Err(InputError::PixelOutsideVirtualDesktop)
        );
        assert_eq!(
            desktop.normalize(2560, 0),
            Err(InputError::PixelOutsideVirtualDesktop)
        );
    }
    #[test]
    fn invalid_or_degenerate_topology_has_defined_behavior() {
        let desktop = VirtualDesktopPixels {
            left: 0,
            top: 0,
            width: 1,
            height: 1,
        };
        assert_eq!(desktop.normalize(0, 0), Ok((0, 0)));
        assert_eq!(
            VirtualDesktopPixels {
                width: 0,
                ..desktop
            }
            .normalize(0, 0),
            Err(InputError::InvalidVirtualDesktop)
        );
        assert_eq!(
            VirtualDesktopPixels {
                left: i32::MAX,
                width: 2,
                ..desktop
            }
            .normalize(0, 0),
            Err(InputError::InvalidVirtualDesktop)
        );
        let wide = VirtualDesktopPixels {
            left: i32::MIN,
            width: u32::MAX,
            ..desktop
        };
        assert_eq!(wide.normalize(i32::MAX - 1, 0), Ok((65535, 0)));
    }
    #[test]
    fn only_complete_secure_attention_chord_is_rejected() {
        for bits in 0..8 {
            assert_eq!(
                is_secure_attention_chord(bits & 1 != 0, bits & 2 != 0, bits & 4 != 0),
                bits == 7
            );
        }
    }
    #[test]
    fn error_display_does_not_reveal_key_usage() {
        assert_eq!(
            InputError::UnsupportedUsage(0x1234).to_string(),
            "unsupported HID usage"
        );
    }

    #[test]
    fn injection_marker_is_only_a_loop_filter() {
        assert!(is_own_injected_event(INPUT_INJECTION_MARKER));
        assert!(!is_own_injected_event(0));
        assert!(!is_own_injected_event(INPUT_INJECTION_MARKER + 1));
    }
}
