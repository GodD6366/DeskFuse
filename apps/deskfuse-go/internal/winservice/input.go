package winservice

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
)

// The Windows service stores deskfuse_core::InputSharingSettings: named
// shortcut bindings and a role enum. The shared UI consumes the macOS bridge's
// projection: a six-element shortcut array and a controller boolean. These
// types carry the service shape so read-modify-write commands preserve fields
// the UI does not edit (layout profiles, input paths, authorized controller).

type inputShortcutWire struct {
	KeyCode   uint16 `json:"key_code"`
	Modifiers uint64 `json:"modifiers"`
}

type inputShortcutBindingsWire struct {
	Left      inputShortcutWire `json:"left"`
	Right     inputShortcutWire `json:"right"`
	Up        inputShortcutWire `json:"up"`
	Down      inputShortcutWire `json:"down"`
	Lock      inputShortcutWire `json:"lock"`
	Emergency inputShortcutWire `json:"emergency"`
}

type displayKeyWire struct {
	DeviceID  string `json:"device_id"`
	DisplayID string `json:"display_id"`
}

type inputPlacementWire struct {
	Display displayKeyWire `json:"display"`
	X       float64        `json:"x"`
	Y       float64        `json:"y"`
}

type inputSettingsWire struct {
	FeatureEnabled       bool                      `json:"feature_enabled"`
	Shortcuts            inputShortcutBindingsWire `json:"shortcuts"`
	DisabledCorners      uint8                     `json:"disabled_corners"`
	CornerSize           uint16                    `json:"corner_size"`
	Enabled              bool                      `json:"enabled"`
	Role                 string                    `json:"role"`
	AuthorizedController *string                   `json:"authorized_controller"`
	Participants         []string                  `json:"participants"`
	SwitchMode           string                    `json:"switch_mode"`
	Placements           []inputPlacementWire      `json:"placements"`
	LayoutProfiles       []json.RawMessage         `json:"layout_profiles"`
	InputPaths           []json.RawMessage         `json:"input_paths"`
}

func (s inputSettingsWire) clone() inputSettingsWire {
	cloned := s
	cloned.Participants = append([]string(nil), s.Participants...)
	cloned.Placements = append([]inputPlacementWire(nil), s.Placements...)
	cloned.LayoutProfiles = append([]json.RawMessage(nil), s.LayoutProfiles...)
	cloned.InputPaths = append([]json.RawMessage(nil), s.InputPaths...)
	if cloned.Participants == nil {
		cloned.Participants = []string{}
	}
	if cloned.Placements == nil {
		cloned.Placements = []inputPlacementWire{}
	}
	if cloned.LayoutProfiles == nil {
		cloned.LayoutProfiles = []json.RawMessage{}
	}
	if cloned.InputPaths == nil {
		cloned.InputPaths = []json.RawMessage{}
	}
	return cloned
}

// view is the macOS bridge's InputSettings shape.
func (s inputSettingsWire) view() map[string]any {
	return map[string]any{
		"feature_enabled":  s.FeatureEnabled,
		"enabled":          s.Enabled,
		"controller":       s.Role == "Controller",
		"participants":     nonNilStrings(s.Participants),
		"switch_mode":      switchModeView(s.SwitchMode),
		"disabled_corners": s.DisabledCorners,
		"corner_size":      s.CornerSize,
		"placements":       placementViews(s.Placements),
		"shortcuts": []map[string]any{
			shortcutView(s.Shortcuts.Left),
			shortcutView(s.Shortcuts.Right),
			shortcutView(s.Shortcuts.Up),
			shortcutView(s.Shortcuts.Down),
			shortcutView(s.Shortcuts.Lock),
			shortcutView(s.Shortcuts.Emergency),
		},
	}
}

func shortcutView(shortcut inputShortcutWire) map[string]any {
	return map[string]any{"key_code": shortcut.KeyCode, "modifiers": shortcut.Modifiers}
}

func placementViews(placements []inputPlacementWire) []map[string]any {
	rows := make([]map[string]any, 0, len(placements))
	for _, placement := range placements {
		rows = append(rows, map[string]any{
			"device_id":  placement.Display.DeviceID,
			"display_id": placement.Display.DisplayID,
			"x":          placement.X,
			"y":          placement.Y,
		})
	}
	return rows
}

func switchModeView(mode string) string {
	switch mode {
	case "Dwell250Ms":
		return "dwell"
	case "DoubleTap500Ms":
		return "double_tap"
	case "Instant":
		return "instant"
	default:
		return "instant"
	}
}

func switchModeWire(mode string) (string, bool) {
	switch mode {
	case "instant", "Instant":
		return "Instant", true
	case "dwell", "Dwell250Ms":
		return "Dwell250Ms", true
	case "double_tap", "DoubleTap500Ms":
		return "DoubleTap500Ms", true
	default:
		return "", false
	}
}

// applyUI merges the subset of InputSettings the page edits into the service
// model, mirroring deskfuse-ffi's InputSettings::into_core. The service still
// validates the result; this layer never repairs an invalid combination into a
// different state.
func (s *inputSettingsWire) applyUI(values map[string]any) error {
	if value, ok := boolValue(values, "feature_enabled", "featureEnabled"); ok {
		s.FeatureEnabled = value
	}
	if value, ok := boolValue(values, "enabled"); ok {
		s.Enabled = value
	}
	if value, ok := boolValue(values, "controller"); ok {
		if value {
			s.Role = "Controller"
		} else {
			s.Role = "Disabled"
		}
	}
	if raw, ok := values["participants"]; ok {
		participants, err := stringList(raw, "participants 必须是设备 ID 列表")
		if err != nil {
			return err
		}
		s.Participants = participants
	}
	if text, ok := stringValue(values, "switch_mode", "switchMode"); ok {
		mode, valid := switchModeWire(text)
		if !valid {
			return invalidParams(fmt.Sprintf("未知切换模式 %s", text))
		}
		s.SwitchMode = mode
	}
	if value, ok := uintValue(values, "disabled_corners", "disabledCorners"); ok {
		if value > math.MaxUint8 {
			return invalidParams("disabled_corners 超出范围")
		}
		s.DisabledCorners = uint8(value)
	}
	if value, ok := uintValue(values, "corner_size", "cornerSize"); ok {
		if value > math.MaxUint16 {
			return invalidParams("corner_size 超出范围")
		}
		s.CornerSize = uint16(value)
	}
	if raw, ok := values["placements"]; ok {
		placements, err := placementValues(raw)
		if err != nil {
			return err
		}
		s.Placements = placements
	}
	if raw, ok := values["shortcuts"]; ok {
		bindings, err := shortcutBindings(raw)
		if err != nil {
			return err
		}
		s.Shortcuts = bindings
	}
	return nil
}

func stringList(raw any, message string) ([]string, error) {
	values, ok := sliceValues(raw)
	if !ok {
		return nil, invalidParams(message)
	}
	result := make([]string, 0, len(values))
	for _, value := range values {
		text, ok := value.(string)
		if !ok || text == "" {
			return nil, invalidParams(message)
		}
		result = append(result, text)
	}
	return result, nil
}

func placementValues(raw any) ([]inputPlacementWire, error) {
	values, ok := sliceValues(raw)
	if !ok {
		return nil, invalidParams("placements 必须是屏幕位置列表")
	}
	placements := make([]inputPlacementWire, 0, len(values))
	for _, value := range values {
		row, ok := value.(map[string]any)
		if !ok {
			return nil, invalidParams("屏幕位置格式无效")
		}
		device, ok := stringValue(row, "device_id", "deviceId")
		if !ok {
			return nil, invalidParams("屏幕位置缺少 device_id")
		}
		display, ok := stringValue(row, "display_id", "displayId")
		if !ok {
			return nil, invalidParams("屏幕位置缺少 display_id")
		}
		x, err := finiteFloatArg(row, []string{"x"}, "屏幕位置缺少有效 x")
		if err != nil {
			return nil, err
		}
		y, err := finiteFloatArg(row, []string{"y"}, "屏幕位置缺少有效 y")
		if err != nil {
			return nil, err
		}
		placements = append(placements, inputPlacementWire{
			Display: displayKeyWire{DeviceID: device, DisplayID: display},
			X:       x,
			Y:       y,
		})
	}
	return placements, nil
}

func shortcutBindings(raw any) (inputShortcutBindingsWire, error) {
	values, ok := sliceValues(raw)
	if !ok || len(values) != 6 {
		return inputShortcutBindingsWire{}, invalidParams("快捷键需要方向、锁定和紧急收回共六组按键")
	}
	parsed := make([]inputShortcutWire, 0, len(values))
	for _, value := range values {
		row, ok := value.(map[string]any)
		if !ok {
			return inputShortcutBindingsWire{}, invalidParams("快捷键格式无效")
		}
		keyCode, ok := uintValue(row, "key_code", "keyCode")
		if !ok || keyCode > math.MaxUint16 {
			return inputShortcutBindingsWire{}, invalidParams("快捷键缺少有效 key_code")
		}
		modifiers, ok := uintValue(row, "modifiers")
		if !ok {
			return inputShortcutBindingsWire{}, invalidParams("快捷键缺少 modifiers")
		}
		parsed = append(parsed, inputShortcutWire{KeyCode: uint16(keyCode), Modifiers: modifiers})
	}
	return inputShortcutBindingsWire{
		Left:      parsed[0],
		Right:     parsed[1],
		Up:        parsed[2],
		Down:      parsed[3],
		Lock:      parsed[4],
		Emergency: parsed[5],
	}, nil
}

// cachedInputSettings returns a copy of the last snapshot's input settings.
// Commands merge into it so fields the page does not edit (layout profiles,
// input paths, authorized controller) survive the round trip.
func (s *Service) cachedInputSettings(ctx context.Context) (inputSettingsWire, error) {
	if err := s.ensureSession(ctx); err != nil {
		return inputSettingsWire{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.latest == nil || s.latest.Input.Settings == nil {
		return inputSettingsWire{}, &RemoteError{
			Code:    "input_unavailable",
			Message: "尚未读取 Windows 键鼠设置，请刷新后重试。",
		}
	}
	return s.latest.Input.Settings.clone(), nil
}

func (s *Service) latestSnapshot() (snapshotWire, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.latest == nil {
		return snapshotWire{}, false
	}
	return *s.latest, true
}

// invokeInput handles the input-domain actions. A false handled result means
// the action belongs to another domain.
func (s *Service) invokeInput(ctx context.Context, action string, args map[string]any, result any) (bool, error) {
	switch action {
	case "input.setSettings", "set_input":
		return true, s.setInputSettings(ctx, args, result)
	case "input.setParticipant":
		return true, s.setInputParticipant(ctx, args, result)
	case "input.reclaim":
		return true, s.callQueued(ctx, "ReclaimInput", action, result)
	case "input.moveDevice":
		return true, s.moveInputDevice(ctx, args, result)
	case "input.reconnect":
		return true, s.reconnectInput(ctx, args, result)
	case "input.chooseController":
		return true, unsupported("Windows 服务没有按设备选择主控的命令；主控方向由控制端的 EnableInputControl 建立。")
	case "input.previewDisplayMove":
		return true, unsupported("Windows 桌面 IPC 没有拖动预览或吸附计算命令；吸附由服务在 MoveControlDevice 时执行。")
	case "input.wallpaper":
		return true, unsupported("Windows 桌面 IPC 没有屏幕预览命令。")
	case "input.set_recording":
		return true, unsupported("Windows 桌面 IPC 没有鼠标按键录制命令。")
	case "feature.setEnabled":
		feature, _ := stringValue(args, "feature")
		if feature != "input" {
			return false, nil
		}
		return true, s.setInputFeature(ctx, args, result)
	default:
		return false, nil
	}
}

func (s *Service) setInputSettings(ctx context.Context, args map[string]any, result any) error {
	settings, err := s.cachedInputSettings(ctx)
	if err != nil {
		return err
	}
	values, err := object(args["settings"])
	if err != nil {
		return err
	}
	if len(values) == 0 {
		return invalidParams("缺少 settings")
	}
	if err := settings.applyUI(values); err != nil {
		return err
	}
	return s.callCommand(ctx, tagged("SetInputSharingSettings", map[string]any{"settings": settings}), "input.setSettings", result)
}

func (s *Service) setInputParticipant(ctx context.Context, args map[string]any, result any) error {
	device, err := requiredString(args, []string{"device_id", "deviceId"}, "需要 device_id")
	if err != nil {
		return err
	}
	included, ok := boolValue(args, "included")
	if !ok {
		return invalidParams("需要 included")
	}
	settings, err := s.cachedInputSettings(ctx)
	if err != nil {
		return err
	}
	participants := make([]string, 0, len(settings.Participants)+1)
	for _, participant := range settings.Participants {
		if participant != device {
			participants = append(participants, participant)
		}
	}
	if included {
		participants = append(participants, device)
	}
	settings.Participants = participants
	return s.callCommand(ctx, tagged("SetInputSharingSettings", map[string]any{"settings": settings}), "input.setParticipant", result)
}

func (s *Service) setInputFeature(ctx context.Context, args map[string]any, result any) error {
	enabled, ok := boolValue(args, "enabled")
	if !ok {
		return invalidParams("需要 enabled")
	}
	settings, err := s.cachedInputSettings(ctx)
	if err != nil {
		return err
	}
	settings.FeatureEnabled = enabled
	return s.callCommand(ctx, tagged("SetInputSharingSettings", map[string]any{"settings": settings}), "feature.setEnabled", result)
}

func (s *Service) moveInputDevice(ctx context.Context, args map[string]any, result any) error {
	device, err := requiredString(args, []string{"deviceId", "device_id"}, "需要 device_id")
	if err != nil {
		return err
	}
	display, err := requiredString(args, []string{"displayId", "display_id"}, "需要 display_id")
	if err != nil {
		return err
	}
	x, err := finiteFloatArg(args, []string{"x"}, "屏幕位置缺少有效 x")
	if err != nil {
		return err
	}
	y, err := finiteFloatArg(args, []string{"y"}, "屏幕位置缺少有效 y")
	if err != nil {
		return err
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	command := tagged("MoveControlDevice", map[string]any{
		"device_id": device, "display_id": display, "x": x, "y": y,
	})
	return s.callCommand(ctx, command, "input.moveDevice", result)
}

func (s *Service) reconnectInput(ctx context.Context, args map[string]any, result any) error {
	device, err := requiredString(args, []string{"device_id", "deviceId"}, "需要 device_id")
	if err != nil {
		return err
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callCommand(ctx, tagged("RestartInputControl", map[string]any{"peer_id": device}), "input.reconnect", result)
}
