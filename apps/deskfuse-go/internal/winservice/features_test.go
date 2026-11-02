package winservice

import (
	"context"
	"errors"
	"strings"
	"testing"
)

// newFeatureService builds a service over a fake pipe so feature actions can
// be asserted against real commands without a live service.
func newFeatureService(t *testing.T, reply func(command map[string]any) map[string]any) (*Service, *fakePipe) {
	t.Helper()
	pipe, dial := newDesktopPipe(t, sampleSnapshot, reply)
	service := NewService(dial)
	service.SetNative(&fakeNative{})
	service.SetHost(&fakeHost{})
	openSession(t, service, pipe)
	return service, pipe
}

func TestShelfAddRemoveClearProjectsToSnapshot(t *testing.T) {
	service, _ := newFeatureService(t, queuedReply)

	mustInvoke(t, service, "shelf.add", map[string]any{"paths": []any{`C:\tmp\a.txt`, `C:\tmp\b.txt`}})
	host := &fakeHost{}
	service.SetHost(host)

	snapshot := mustCall(t, service, "app.snapshot", nil)
	shelf := snapshot["shelf"].(map[string]any)
	files := shelf["files"].([]any)
	if len(files) != 2 {
		t.Fatalf("shelf files = %d, want 2", len(files))
	}
	first := files[0].(map[string]any)
	if first["name"] != "a.txt" || first["path"] != `C:\tmp\a.txt` || first["id"] == "" {
		t.Fatalf("shelf file projection wrong: %v", first)
	}

	mustInvoke(t, service, "shelf.remove", map[string]any{"id": first["id"]})
	snapshot = mustCall(t, service, "app.snapshot", nil)
	if got := len(snapshot["shelf"].(map[string]any)["files"].([]any)); got != 1 {
		t.Fatalf("after remove files = %d, want 1", got)
	}

	mustInvoke(t, service, "shelf.clear", nil)
	snapshot = mustCall(t, service, "app.snapshot", nil)
	if got := len(snapshot["shelf"].(map[string]any)["files"].([]any)); got != 0 {
		t.Fatalf("after clear files = %d, want 0", got)
	}
	if host.shelfHidden == 0 {
		t.Fatal("shelf.clear must hide the shelf window")
	}
}

func TestShelfSendPublishesFileClipboardToPeer(t *testing.T) {
	service, pipe := newFeatureService(t, func(command map[string]any) map[string]any {
		if _, ok := command["PublishFileClipboard"]; ok {
			return queuedReply(command)
		}
		return queuedReply(command)
	})
	mustInvoke(t, service, "shelf.add", map[string]any{"paths": []any{`C:\tmp\a.txt`}})

	mustInvoke(t, service, "shelf.send", map[string]any{"peer_id": "peer-1"})
	command := nextCommand(t, pipe)
	publish := command["PublishFileClipboard"].(map[string]any)
	if publish["target_peer_id"] != "peer-1" {
		t.Fatalf("target peer = %v", publish["target_peer_id"])
	}
	paths := publish["paths"].([]any)
	if len(paths) != 1 || paths[0] != `C:\tmp\a.txt` {
		t.Fatalf("paths = %v", paths)
	}
	if seq, _ := publish["sequence"].(float64); seq != 1 {
		t.Fatalf("first sequence = %v, want 1", seq)
	}
}

func TestCompareVersions(t *testing.T) {
	cases := []struct {
		a, b string
		want int
	}{
		{"0.7.3", "0.7.2", 1},
		{"0.7.2", "0.7.2", 0},
		{"0.7.10", "0.7.9", 1},
		{"0.8.0", "0.7.99", 1},
		{"1.0.0", "0.9.9", 1},
		{"0.6.1", "0.7.0", -1},
	}
	for _, test := range cases {
		got := compareVersions(test.a, test.b)
		if (got > 0) != (test.want > 0) || (got < 0) != (test.want < 0) {
			t.Errorf("compareVersions(%q,%q) = %d, want sign %d", test.a, test.b, got, test.want)
		}
	}
}

func TestUpdateCheckProjectsAvailability(t *testing.T) {
	service, _ := newFeatureService(t, queuedReply)
	service.fetchRelease = func(ctx context.Context) (string, string, error) {
		return "9.9.9", "https://example.test/release", nil
	}
	// 快照里的 package_version 是 0.7.2（sampleSnapshot）。
	mustInvoke(t, service, "updates.check", nil)
	service.runUpdateCheck(context.Background(), "0.7.2")

	snapshot := mustCall(t, service, "app.snapshot", nil)
	update := snapshot["update"].(map[string]any)
	if update["available"] != true || update["target_version"] != "9.9.9" {
		t.Fatalf("update projection wrong: %v", update)
	}
	if update["channel"] != "manual" {
		t.Fatalf("channel = %v, want manual", update["channel"])
	}

	service.fetchRelease = func(ctx context.Context) (string, string, error) {
		return "0.1.0", "", nil
	}
	service.runUpdateCheck(context.Background(), "0.7.2")
	snapshot = mustCall(t, service, "app.snapshot", nil)
	update = snapshot["update"].(map[string]any)
	if update["available"] != false {
		t.Fatalf("older release must not be offered: %v", update)
	}

	service.fetchRelease = func(ctx context.Context) (string, string, error) {
		return "", "", errors.New("offline")
	}
	service.runUpdateCheck(context.Background(), "0.7.2")
	snapshot = mustCall(t, service, "app.snapshot", nil)
	update = snapshot["update"].(map[string]any)
	if message, _ := update["message"].(string); !strings.Contains(message, "无法检查更新") {
		t.Fatalf("failure message = %v", update["message"])
	}
}

func TestMacShortcutToAccelerator(t *testing.T) {
	// macOS default ⌃⌘V becomes the Windows canonical Ctrl+Shift+V.
	got, err := macShortcutToAccelerator(9, modControl|modCommand)
	if err != nil || got != "CmdOrCtrl+Shift+V" {
		t.Fatalf("default = %q, %v", got, err)
	}
	// 字面映射：⌃⇧K → Ctrl+Shift+K。
	got, err = macShortcutToAccelerator(40, modControl|modShift)
	if err != nil || got != "Ctrl+Shift+K" {
		t.Fatalf("literal = %q, %v", got, err)
	}
	// 未知键码如实报错。
	if _, err := macShortcutToAccelerator(200, modControl); err == nil {
		t.Fatal("unknown key code must fail")
	}
	// 无修饰键拒绝。
	if _, err := macShortcutToAccelerator(9, 0); err == nil {
		t.Fatal("modifier-less shortcut must fail")
	}
}

func TestChooseControllerMapsToEnableInputControl(t *testing.T) {
	service, pipe := newFeatureService(t, queuedReply)
	mustInvoke(t, service, "input.chooseController", map[string]any{"id": "peer-9"})
	command := nextCommand(t, pipe)
	enable, ok := command["EnableInputControl"].(map[string]any)
	if !ok || enable["peer_id"] != "peer-9" {
		t.Fatalf("command = %v", command)
	}
}

func TestChooseAppCancelReturnsNullBundle(t *testing.T) {
	service, _ := newFeatureService(t, queuedReply)
	// fakeNative.PickExe 返回 picked=false。
	value := mustInvoke(t, service, "clipboard.choose_app", nil)
	bundle := value["value"].(map[string]any)
	if bundle["bundle_id"] != nil {
		t.Fatalf("cancel must return null bundle_id: %v", bundle)
	}
}

func TestConfigureShortcutRejectsUnknownKeyCode(t *testing.T) {
	service, _ := newFeatureService(t, queuedReply)
	err := service.Call(context.Background(), "app.invoke", map[string]any{
		"action": "clipboard.configure_shortcut",
		"params": map[string]any{"shortcut": map[string]any{"key_code": 200, "modifiers": modControl}},
	}, nil)
	var remote *RemoteError
	if !errors.As(err, &remote) || remote.Code != "invalid_params" {
		t.Fatalf("unknown key code must be invalid_params, got %v", err)
	}
}

func TestPreviewDisplayMoveReturnsSnappedOffset(t *testing.T) {
	service, pipe := newFeatureService(t, func(command map[string]any) map[string]any {
		if _, ok := command["PreviewControlDeviceMove"]; ok {
			return map[string]any{"LayoutOffset": map[string]any{"x": 12.5, "y": -3}}
		}
		return queuedReply(command)
	})
	value := mustInvoke(t, service, "input.previewDisplayMove", map[string]any{
		"displays": []any{map[string]any{
			"id": "dp1", "device_id": "dev-1", "name": "显示器",
			"x": 0.0, "y": 0.0, "width": 1920.0, "height": 1080.0, "local": true, "ready": true,
		}},
		"device_id":     "dev-1",
		"delta_x":       13.0,
		"delta_y":       40.0,
		"snap_distance": 14.0,
		"attach":        false,
	})
	offset := value["value"].(map[string]any)
	if offset["x"] != 12.5 || offset["y"] != -3.0 {
		t.Fatalf("offset = %v", offset)
	}
	command := nextCommand(t, pipe)
	preview := command["PreviewControlDeviceMove"].(map[string]any)
	if preview["attach"] != false || preview["snap_distance"] != 14.0 {
		t.Fatalf("preview args = %v", preview)
	}
	displays := preview["displays"].([]any)
	first := displays[0].(map[string]any)
	if first["device_id"] != "dev-1" || first["width"] != 1920.0 {
		t.Fatalf("display wire = %v", first)
	}
}

func TestSetFilesMapsToServiceSettings(t *testing.T) {
	service, pipe := newFeatureService(t, queuedReply)
	mustInvoke(t, service, "set_files", map[string]any{"settings": map[string]any{
		"enabled": true, "prefetch": false, "threshold": 1024.0,
		"maximum": 2048.0, "download_directory": "C:/Recv",
	}})
	command := nextCommand(t, pipe)
	set := command["SetFileSettings"].(map[string]any)
	settings := set["settings"].(map[string]any)
	if settings["enabled"] != true || settings["small_file_prefetch_enabled"] != false ||
		settings["small_file_threshold_bytes"] != 1024.0 || settings["cache_max_bytes"] != 2048.0 ||
		settings["download_directory"] != "C:/Recv" {
		t.Fatalf("settings wire = %v", settings)
	}
}

func TestClearCacheReturnsRemovedBytes(t *testing.T) {
	service, pipe := newFeatureService(t, func(command map[string]any) map[string]any {
		if _, ok := command["ClearFileCache"]; ok {
			return map[string]any{"CacheCleared": map[string]any{"removed_bytes": 4096}}
		}
		return queuedReply(command)
	})
	value := mustInvoke(t, service, "clear_cache", nil)
	removed := value["value"].(map[string]any)["removed_bytes"]
	if removed != float64(4096) {
		t.Fatalf("removed = %v", removed)
	}
	nextCommand(t, pipe) // ClearFileCache
}

func TestTransfersCancelAndRetryMapToCommands(t *testing.T) {
	service, pipe := newFeatureService(t, queuedReply)
	mustInvoke(t, service, "transfers.cancel", map[string]any{"id": "tx-1"})
	command := nextCommand(t, pipe)
	if cancel := command["CancelTransfer"].(map[string]any); cancel["transfer_id"] != "tx-1" {
		t.Fatalf("cancel = %v", command)
	}
	mustInvoke(t, service, "transfers.retry", map[string]any{"id": "tx-1"})
	command = nextCommand(t, pipe)
	if retry := command["RetryTransfer"].(map[string]any); retry["transfer_id"] != "tx-1" {
		t.Fatalf("retry = %v", command)
	}
}

func TestTransfersSendPublishesExplicitPaths(t *testing.T) {
	service, pipe := newFeatureService(t, queuedReply)
	mustInvoke(t, service, "transfers.send", map[string]any{
		"peer_id": "peer-2",
		"paths":   []any{`C:\tmp\x.txt`},
	})
	command := nextCommand(t, pipe)
	publish := command["PublishFileClipboard"].(map[string]any)
	if publish["target_peer_id"] != "peer-2" {
		t.Fatalf("publish = %v", publish)
	}
}

func TestSystemShortcutsReturnsList(t *testing.T) {
	service, pipe := newFeatureService(t, func(command map[string]any) map[string]any {
		if _, ok := command["GetSystemShortcuts"]; ok {
			return map[string]any{"SystemShortcuts": map[string]any{
				"shortcuts": []any{map[string]any{"id": "s1", "name": "示例"}}}}
		}
		return queuedReply(command)
	})
	value := mustInvoke(t, service, "workflow.system_shortcuts", nil)
	shortcuts := value["value"].([]any)
	if len(shortcuts) != 1 {
		t.Fatalf("shortcuts = %v", shortcuts)
	}
	nextCommand(t, pipe) // GetSystemShortcuts
}
