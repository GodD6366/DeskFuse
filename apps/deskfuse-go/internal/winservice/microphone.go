package winservice

import (
	"context"
	"fmt"
)

// Windows microphone support is intentionally behind a small host contract.
// The Go/WebView layer must not pretend that a UI action installed an audio
// driver: capture, QUIC session ownership and the virtual input device belong
// to the native Windows backend.
const windowsMicrophoneUnavailable = "Windows 音频采集、跨端音频会话和虚拟麦克风驱动尚未接入当前宿主"

func (s *Service) microphoneStatus(_ context.Context, result any) error {
	return assign(result, map[string]any{
		"supported":        false,
		"driver_installed": false,
		"driver_ready":     false,
		"reason":           windowsMicrophoneUnavailable,
	})
}

func (s *Service) microphoneManageDriver(_ context.Context, args map[string]any, _ any) error {
	action, _ := args["action"].(string)
	if action == "" {
		return invalidParams("缺少麦克风驱动操作")
	}
	return unsupported(fmt.Sprintf("Windows 麦克风驱动操作 %q 不可用：%s", action, windowsMicrophoneUnavailable))
}
