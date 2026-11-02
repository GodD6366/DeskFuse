package winservice

import (
	"context"
	"fmt"
)

// Modifier bits shared with the frontend (frontend/src/pages/settings/util.ts).
const (
	modShift   = 1 << 17
	modControl = 1 << 18
	modOption  = 1 << 19
	modCommand = 1 << 20
)

// macVirtualKeys maps macOS virtual key codes (what the shared library stores)
// to MyGo accelerator key names, so a recorded shortcut can be registered as a
// Windows global hotkey.
var macVirtualKeys = map[int]string{
	0: "A", 1: "S", 2: "D", 3: "F", 4: "H", 5: "G", 6: "Z", 7: "X", 8: "C", 9: "V",
	11: "B", 12: "Q", 13: "W", 14: "E", 15: "R", 16: "Y", 17: "T",
	31: "O", 32: "U", 34: "I", 35: "P", 37: "L", 38: "J", 40: "K", 45: "N", 46: "M",
	18: "1", 19: "2", 20: "3", 21: "4", 23: "5", 22: "6", 26: "7", 28: "8", 25: "9", 29: "0",
	24: "=", 27: "-", 30: "]", 33: "[", 39: "'", 41: ";", 42: `\\`, 43: ",", 44: "/", 47: ".", 50: "`",
	123: "Left", 124: "Right", 125: "Down", 126: "Up",
	53: "Escape", 49: "Space", 36: "Enter", 48: "Tab", 51: "Backspace", 117: "Delete",
	122: "F1", 120: "F2", 99: "F3", 118: "F4", 96: "F5", 97: "F6", 98: "F7", 100: "F8",
	101: "F9", 109: "F10", 103: "F11", 111: "F12",
	115: "Home", 119: "End", 116: "PageUp", 121: "PageDown", 114: "Insert",
	82: "num0", 83: "num1", 84: "num2", 85: "num3", 86: "num4",
	87: "num5", 88: "num6", 89: "num7", 91: "num8", 92: "num9",
	65: "numdec", 67: "nummult", 69: "numadd", 75: "numdiv", 76: "numenter", 78: "numsub",
}

// clipboardConfigureShortcut registers the global hotkey that opens the
// clipboard panel. The library already persisted the shortcut; this applies it
// to the OS. A null shortcut disables the global hotkey.
func (s *Service) clipboardConfigureShortcut(args map[string]any, result any) error {
	if s.host == nil {
		return unsupported("此窗口没有接入桌面宿主。")
	}
	shortcut, present := args["shortcut"]
	if !present || shortcut == nil {
		if err := s.host.SetPanelShortcut(""); err != nil {
			return nativeError(err)
		}
		return assign(result, map[string]any{"outcome": "applied", "action": "clipboard.configure_shortcut"})
	}
	value, ok := shortcut.(map[string]any)
	if !ok {
		return invalidParams("shortcut 必须是对象")
	}
	keyCode, ok := intOf(value["key_code"])
	if !ok {
		return invalidParams("shortcut.key_code 无效")
	}
	modifiers, _ := intOf(value["modifiers"])
	accelerator, err := macShortcutToAccelerator(keyCode, modifiers)
	if err != nil {
		return invalidParams(err.Error())
	}
	if err := s.host.SetPanelShortcut(accelerator); err != nil {
		return nativeError(err)
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "clipboard.configure_shortcut"})
}

// macShortcutToAccelerator converts a mac key code + modifiers to a MyGo
// accelerator. The macOS default ⌃⌘V maps to the Windows canonical
// Ctrl+Shift+V; other combinations translate literally (⌘ becomes the Windows
// key). Unknown key codes are an honest error, not a guessed binding.
func macShortcutToAccelerator(keyCode, modifiers int) (string, error) {
	// macOS default ⌃⌘V (key V = 9, Ctrl|Command) is Ctrl+Shift+V on Windows.
	if keyCode == 9 && modifiers == modControl|modCommand {
		return "CmdOrCtrl+Shift+V", nil
	}
	key, ok := macVirtualKeys[keyCode]
	if !ok {
		return "", fmt.Errorf("暂不支持键码 %d 的全局快捷键", keyCode)
	}
	var parts []string
	if modifiers&modCommand != 0 {
		parts = append(parts, "Command")
	}
	if modifiers&modControl != 0 {
		parts = append(parts, "Ctrl")
	}
	if modifiers&modOption != 0 {
		parts = append(parts, "Alt")
	}
	if modifiers&modShift != 0 {
		parts = append(parts, "Shift")
	}
	if len(parts) == 0 {
		return "", fmt.Errorf("全局快捷键需要至少一个修饰键")
	}
	return joinPlus(append(parts, key)), nil
}

func joinPlus(parts []string) string {
	out := ""
	for i, part := range parts {
		if i > 0 {
			out += "+"
		}
		out += part
	}
	return out
}

func intOf(value any) (int, bool) {
	switch v := value.(type) {
	case float64:
		return int(v), true
	case int:
		return v, true
	case int64:
		return int(v), true
	}
	return 0, false
}

// inputSetRecording suspends the desktop's own global hotkey while the page
// records a shortcut, so the recorded combination is not swallowed. The input
// runtime's shortcuts belong to the service and are untouched here.
func (s *Service) inputSetRecording(ctx context.Context, args map[string]any, result any) error {
	if s.host == nil {
		return unsupported("此窗口没有接入桌面宿主。")
	}
	enabled, _ := boolValue(args, "enabled")
	if err := s.host.SuspendPanelShortcut(enabled); err != nil {
		return nativeError(err)
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "input.set_recording"})
}
