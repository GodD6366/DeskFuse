package winservice

import (
	"encoding/json"
	"time"
)

// supportExportDiagnostics writes a timestamped JSON snapshot of the client
// state — versions, network/devices, input status and UI preferences. It
// deliberately excludes clipboard content, history entries and trust material,
// matching the macOS export's redaction contract.
func (s *Service) supportExportDiagnostics(result any) error {
	if s.native == nil {
		return unsupported("此窗口没有接入系统保存对话框。")
	}
	payload := map[string]any{
		"exported_at":   time.Now().Format(time.RFC3339),
		"platform":      "windows",
		"client":        "mygo-webview",
		"environment":   s.environmentLines(),
		"preferences":   s.prefs.valuesCopy(),
		"keep_awake":    s.prefs.Bool("keep_awake"),
		"update":        s.updates.view(s.currentVersion()),
	}
	if latest, ok := s.latestSnapshot(); ok {
		payload["service"] = map[string]any{
			"package_version": latest.PackageVersion,
			"local_device_id": latest.LocalDeviceID,
			"local_name":      latest.LocalName,
			"network_running": latest.NetworkRunning,
			"capabilities":    latest.Capabilities,
			"session_epoch":   latest.SessionEpoch,
		}
	}
	data, err := json.MarshalIndent(payload, "", "  ")
	if err != nil {
		return nativeError(err)
	}
	name := "deskfuse-diagnostics-" + time.Now().Format("20060102-150405") + ".json"
	path, saved, err := s.native.SaveFileBytes(name, data)
	if err != nil {
		return nativeError(err)
	}
	value := map[string]any{"saved": saved}
	if path != "" {
		value["path"] = path
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "support.export_diagnostics", "value": value})
}
