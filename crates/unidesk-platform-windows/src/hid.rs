//! USB HID keyboard-page (0x07) usages to Windows set-1 scan codes.
//!
//! These are physical keys, not Quartz key codes or Unicode characters. Layout and
//! input-method interpretation belongs to Windows. Pause's E1 sequence and keys
//! outside the explicit table are rejected rather than guessed with `MapVirtualKey`.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScanCode {
    pub code: u16,
    pub extended: bool,
}

impl ScanCode {
    const fn plain(code: u16) -> Self {
        Self {
            code,
            extended: false,
        }
    }

    const fn extended(code: u16) -> Self {
        Self {
            code,
            extended: true,
        }
    }
}

/// Resolve only keyboard-page usages. The caller must independently negotiate
/// the HID protocol; this function must never receive legacy Quartz key codes.
#[must_use]
pub fn keyboard_usage_to_scan_code(usage: u16) -> Option<ScanCode> {
    let plain = match usage {
        0x04 => 0x1e,
        0x05 => 0x30,
        0x06 => 0x2e,
        0x07 => 0x20,
        0x08 => 0x12,
        0x09 => 0x21,
        0x0a => 0x22,
        0x0b => 0x23,
        0x0c => 0x17,
        0x0d => 0x24,
        0x0e => 0x25,
        0x0f => 0x26,
        0x10 => 0x32,
        0x11 => 0x31,
        0x12 => 0x18,
        0x13 => 0x19,
        0x14 => 0x10,
        0x15 => 0x13,
        0x16 => 0x1f,
        0x17 => 0x14,
        0x18 => 0x16,
        0x19 => 0x2f,
        0x1a => 0x11,
        0x1b => 0x2d,
        0x1c => 0x15,
        0x1d => 0x2c,
        0x1e..=0x26 => usage - 0x1e + 0x02,
        0x27 => 0x0b,
        0x28 => 0x1c,
        0x29 => 0x01,
        0x2a => 0x0e,
        0x2b => 0x0f,
        0x2c => 0x39,
        0x2d => 0x0c,
        0x2e => 0x0d,
        0x2f => 0x1a,
        0x30 => 0x1b,
        0x31 | 0x32 => 0x2b,
        0x33 => 0x27,
        0x34 => 0x28,
        0x35 => 0x29,
        0x36 => 0x33,
        0x37 => 0x34,
        0x38 => 0x35,
        0x39 => 0x3a,
        0x3a..=0x43 => usage - 0x3a + 0x3b,
        0x44 => 0x57,
        0x45 => 0x58,
        0x47 => 0x46, // Scroll Lock; Pause (0x48) is deliberately unsupported.
        0x53 => 0x45, // Num Lock
        0x55 => 0x37,
        0x56 => 0x4a,
        0x57 => 0x4e,
        0x59 => 0x4f,
        0x5a => 0x50,
        0x5b => 0x51,
        0x5c => 0x4b,
        0x5d => 0x4c,
        0x5e => 0x4d,
        0x5f => 0x47,
        0x60 => 0x48,
        0x61 => 0x49,
        0x62 => 0x52,
        0x63 => 0x53,
        0x64 => 0x56,
        0x68..=0x72 => usage - 0x68 + 0x64, // F13..F23
        0x73 => 0x76, // F24 is not contiguous with F13..F23 in the Windows table.
        0xe0 => 0x1d,
        0xe1 => 0x2a,
        0xe2 => 0x38,
        0xe5 => 0x36,
        _ => {
            let extended = match usage {
                0x46 => 0x37, // Print Screen
                0x49 => 0x52,
                0x4a => 0x47,
                0x4b => 0x49,
                0x4c => 0x53,
                0x4d => 0x4f,
                0x4e => 0x51,
                0x4f => 0x4d,
                0x50 => 0x4b,
                0x51 => 0x50,
                0x52 => 0x48,
                0x54 => 0x35,
                0x58 => 0x1c,
                0x65 => 0x5d,
                0xe3 => 0x5b,
                0xe4 => 0x1d,
                0xe6 => 0x38,
                0xe7 => 0x5c,
                _ => return None,
            };
            return Some(ScanCode::extended(extended));
        }
    };
    Some(ScanCode::plain(plain))
}

/// Canonical reverse mapping for `WH_KEYBOARD_LL` scan codes. HID 0x31/0x32
/// share a Windows scan code and are represented as 0x31 on capture. Pause must
/// be rejected using its `VK_PAUSE` tag before calling this function (Windows can
/// report the same base scan code as Num Lock).
#[must_use]
pub fn scan_code_to_keyboard_usage(scan: ScanCode) -> Option<u16> {
    // Windows can label Num Lock as extended even though its set-1 make is 45.
    if scan.code == 0x45 {
        return Some(0x53);
    }
    (0x04..=0xe7).find(|usage| keyboard_usage_to_scan_code(*usage) == Some(scan))
}

/// Independent left/right modifiers, using HID's modifier byte order. GUI means
/// Windows on this adapter (Command on macOS); Alt means Option on macOS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HidModifiers(pub u8);

impl HidModifiers {
    pub const LEFT_CONTROL: u8 = 1 << 0;
    pub const LEFT_SHIFT: u8 = 1 << 1;
    pub const LEFT_ALT: u8 = 1 << 2;
    pub const LEFT_GUI: u8 = 1 << 3;
    pub const RIGHT_CONTROL: u8 = 1 << 4;
    pub const RIGHT_SHIFT: u8 = 1 << 5;
    pub const RIGHT_ALT: u8 = 1 << 6;
    pub const RIGHT_GUI: u8 = 1 << 7;

    pub fn usages(self) -> impl Iterator<Item = u16> {
        (0..8)
            .filter(move |bit| self.0 & (1 << bit) != 0)
            .map(|bit| 0xe0 + bit)
    }

    #[must_use]
    pub fn control(self) -> bool {
        self.0 & (Self::LEFT_CONTROL | Self::RIGHT_CONTROL) != 0
    }

    #[must_use]
    pub fn alt(self) -> bool {
        self.0 & (Self::LEFT_ALT | Self::RIGHT_ALT) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_letters_and_navigation_are_not_quartz_codes() {
        assert_eq!(keyboard_usage_to_scan_code(0), None);
        assert_eq!(keyboard_usage_to_scan_code(4), Some(ScanCode::plain(0x1e)));
        assert_eq!(
            keyboard_usage_to_scan_code(0x1d),
            Some(ScanCode::plain(0x2c))
        );
        assert_eq!(
            keyboard_usage_to_scan_code(0x4c),
            Some(ScanCode::extended(0x53))
        );
        assert_eq!(
            keyboard_usage_to_scan_code(0x63),
            Some(ScanCode::plain(0x53))
        );
        assert_eq!(
            keyboard_usage_to_scan_code(0x58),
            Some(ScanCode::extended(0x1c))
        );
        assert_eq!(
            keyboard_usage_to_scan_code(0x28),
            Some(ScanCode::plain(0x1c))
        );
    }

    #[test]
    fn modifier_sides_and_command_alt_control_semantics_are_preserved() {
        assert_eq!(
            keyboard_usage_to_scan_code(0xe0),
            Some(ScanCode::plain(0x1d))
        );
        assert_eq!(
            keyboard_usage_to_scan_code(0xe4),
            Some(ScanCode::extended(0x1d))
        );
        assert_eq!(
            keyboard_usage_to_scan_code(0xe3),
            Some(ScanCode::extended(0x5b))
        );
        assert_eq!(
            keyboard_usage_to_scan_code(0xe7),
            Some(ScanCode::extended(0x5c))
        );
        let modifiers = HidModifiers(HidModifiers::LEFT_GUI | HidModifiers::RIGHT_ALT);
        assert_eq!(modifiers.usages().collect::<Vec<_>>(), [0xe3, 0xe6]);
        assert!(!modifiers.control());
        assert!(modifiers.alt());
    }

    #[test]
    fn f24_uses_the_windows_scan_code_not_a_contiguous_guess() {
        assert_eq!(
            keyboard_usage_to_scan_code(0x72),
            Some(ScanCode::plain(0x6e))
        );
        assert_eq!(
            keyboard_usage_to_scan_code(0x73),
            Some(ScanCode::plain(0x76))
        );
        assert_eq!(
            scan_code_to_keyboard_usage(ScanCode::plain(0x76)),
            Some(0x73)
        );
        assert_eq!(scan_code_to_keyboard_usage(ScanCode::plain(0x6f)), None);
    }

    #[test]
    fn reverse_mapping_preserves_physical_keys_and_canonicalizes_only_iso_alias() {
        for usage in 0x04..=0xe7 {
            if let Some(scan) = keyboard_usage_to_scan_code(usage) {
                assert_eq!(
                    scan_code_to_keyboard_usage(scan),
                    Some(if usage == 0x32 { 0x31 } else { usage })
                );
            }
        }
        assert_eq!(
            scan_code_to_keyboard_usage(ScanCode::extended(0x45)),
            Some(0x53)
        );
        assert_eq!(scan_code_to_keyboard_usage(ScanCode::extended(0xff)), None);
    }

    #[test]
    fn unsupported_and_complex_keys_are_rejected() {
        for usage in [1, 2, 3, 0x48, 0x66, 0x67, 0x87, 0xffff] {
            assert_eq!(keyboard_usage_to_scan_code(usage), None);
        }
    }
}
