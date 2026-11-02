package winservice

import (
	"context"
	"encoding/json"
	"fmt"
)

// inputChooseController makes this device the controller for the given peer.
// The Windows service expresses "take over as controller" as
// EnableInputControl{peer_id}, which switches the role and target atomically.
func (s *Service) inputChooseController(ctx context.Context, args map[string]any, result any) error {
	peer, err := requiredString(args, []string{"id", "peer_id"}, "需要设备")
	if err != nil {
		return err
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callQueued(ctx, tagged("EnableInputControl", map[string]any{"peer_id": peer}), "input.chooseController", result)
}

// inputWallpaper returns the local desktop wallpaper for the layout canvas.
// The display identifier is accepted for API parity; Windows currently
// reports the shared desktop wallpaper for any display.
func (s *Service) inputWallpaper(args map[string]any, result any) error {
	if s.native == nil {
		return unsupported("此窗口没有接入桌面壁纸读取。")
	}
	displayID, _ := stringValue(args, "display_id", "displayId", "id")
	image, err := s.native.LocalWallpaper(displayID)
	if err != nil {
		return nativeError(err)
	}
	value := map[string]any{"image": nil}
	if image != "" {
		value["image"] = image
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "input.wallpaper", "value": value})
}

// inputPreviewDisplayMove computes the snapped drag offset for one device
// move without committing it, mirroring the engine's preview_display_move.
func (s *Service) inputPreviewDisplayMove(ctx context.Context, args map[string]any, result any) error {
	rawDisplays, ok := args["displays"].([]any)
	if !ok {
		return invalidParams("需要 displays")
	}
	displays := make([]map[string]any, 0, len(rawDisplays))
	for _, raw := range rawDisplays {
		display, ok := raw.(map[string]any)
		if !ok {
			return invalidParams("displays 格式无效")
		}
		displays = append(displays, map[string]any{
			"id":        display["id"],
			"device_id": display["device_id"],
			"x":         display["x"],
			"y":         display["y"],
			"width":     display["width"],
			"height":    display["height"],
		})
	}
	deviceID, err := requiredString(args, []string{"device_id", "deviceId"}, "需要 device_id")
	if err != nil {
		return err
	}
	deltaX, okX := floatValue(args, "delta_x", "deltaX")
	deltaY, okY := floatValue(args, "delta_y", "deltaY")
	if !okX || !okY {
		return invalidParams("需要 delta_x/delta_y")
	}
	snapDistance, _ := floatValue(args, "snap_distance", "snapDistance")
	attach, _ := boolValue(args, "attach")
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	raw, err := s.callVariant(ctx, tagged("PreviewControlDeviceMove", map[string]any{
		"displays":      displays,
		"device_id":     deviceID,
		"delta_x":       deltaX,
		"delta_y":       deltaY,
		"snap_distance": snapDistance,
		"attach":        attach,
	}), "LayoutOffset")
	if err != nil {
		return err
	}
	var reply struct {
		X float64 `json:"x"`
		Y float64 `json:"y"`
	}
	if err := json.Unmarshal(raw, &reply); err != nil {
		return fmt.Errorf("吸附结果无效: %w", err)
	}
	return assign(result, map[string]any{
		"outcome": "applied",
		"action":  "input.previewDisplayMove",
		"value":   map[string]any{"x": reply.X, "y": reply.Y},
	})
}
