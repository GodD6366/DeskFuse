package winservice

import (
	"path/filepath"
	"strings"
)

// clipboardChooseApp opens a native executable picker for the excluded-apps
// setting. The executable path is the stable per-app identity on Windows, the
// same value the service's clipboard capture records for the source process.
func (s *Service) clipboardChooseApp(result any) error {
	if s.native == nil {
		return unsupported("此窗口没有接入系统文件选择。")
	}
	path, picked, err := s.native.PickExe("选择应用")
	if err != nil {
		return nativeError(err)
	}
	if !picked {
		return assign(result, map[string]any{"outcome": "applied", "value": map[string]any{"bundle_id": nil}})
	}
	name := strings.TrimSuffix(filepath.Base(path), filepath.Ext(path))
	return assign(result, map[string]any{"outcome": "applied", "value": map[string]any{
		"bundle_id": path,
		"name":      name,
	}})
}

// clipboardAppInfo returns the display name and icon for an executable the
// excluded-apps list stored. Unknown paths yield a null icon, never a faked
// one.
func (s *Service) clipboardAppInfo(args map[string]any, result any) error {
	if s.native == nil {
		return unsupported("此窗口没有接入系统图标服务。")
	}
	path, _ := stringValue(args, "bundle_id", "bundleId", "id")
	if path == "" {
		return invalidParams("需要 bundle_id")
	}
	icon, err := s.native.AppIcon(path, 32)
	if err != nil {
		return nativeError(err)
	}
	name := strings.TrimSuffix(filepath.Base(path), filepath.Ext(path))
	return assign(result, map[string]any{"outcome": "applied", "value": map[string]any{
		"icon": icon,
		"name": name,
	}})
}
