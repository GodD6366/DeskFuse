package winservice

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

// uninstallComponents enumerates what an uninstall can remove on Windows. The
// mac-only entries (keychain, privacy, microphone driver, homebrew) have no
// Windows counterpart and are absent rather than faked.
type uninstallComponents struct {
	Application  bool
	LoginItem    bool
	UserData     bool
	Caches       bool
	Preferences  bool
}

func uninstallSurvey() uninstallComponents {
	survey := uninstallComponents{Application: true}
	base := localAppDataDir()
	if base != "" {
		if _, err := os.Stat(filepath.Join(base, "DeskFuse")); err == nil {
			survey.UserData = true
			survey.Caches = true
		}
		if _, err := os.Stat(filepath.Join(base, "DeskFuse", "mygo-ui-preferences.json")); err == nil {
			survey.Preferences = true
		}
	}
	if status, err := loginItemStatus(); err == nil && status == "enabled" {
		survey.LoginItem = true
	}
	return survey
}

func localAppDataDir() string {
	if value := os.Getenv("LOCALAPPDATA"); value != "" {
		return value
	}
	return ""
}

// uninstallPlan reports what can be removed and the safe default selection
// (everything except the clipboard history database).
func (s *Service) uninstallPlan(result any) error {
	survey := uninstallSurvey()
	components := make([]string, 0, 5)
	defaults := make([]string, 0, 5)
	add := func(present bool, id string, keepByDefault bool) {
		if !present {
			return
		}
		components = append(components, id)
		if !keepByDefault {
			defaults = append(defaults, id)
		}
	}
	add(survey.Application, "application", false)
	add(survey.LoginItem, "login_item", false)
	add(survey.UserData, "user_data", true)
	add(survey.Caches, "caches", false)
	add(survey.Preferences, "preferences", false)
	return assign(result, map[string]any{
		"outcome":           "applied",
		"value":             map[string]any{"components": components, "default_selection": defaults, "installed_by_homebrew": false},
	})
}

// uninstallPerform removes the selected data components; when the application
// itself is selected it launches the NSIS uninstaller silently and quits, so
// the running client releases its executable before the uninstaller deletes
// the install directory.
func (s *Service) uninstallPerform(args map[string]any, result any) error {
	selected := map[string]bool{}
	if raw, ok := args["components"].([]any); ok {
		for _, value := range raw {
			if id, ok := value.(string); ok {
				selected[id] = true
			}
		}
	}
	if raw, ok := args["components"].([]string); ok {
		for _, id := range raw {
			selected[id] = true
		}
	}
	if len(selected) == 0 {
		return invalidParams("没有选择要卸载的内容")
	}

	base := filepath.Join(localAppDataDir(), "DeskFuse")
	var removeErr error
	remove := func(path string) {
		if removeErr != nil {
			return
		}
		if err := os.RemoveAll(path); err != nil && !os.IsNotExist(err) {
			removeErr = err
		}
	}
	if selected["caches"] || selected["user_data"] {
		remove(filepath.Join(base, "Cache"))
		remove(filepath.Join(base, "logs"))
	}
	if selected["preferences"] {
		remove(filepath.Join(base, "mygo-ui-preferences.json"))
	}
	if selected["user_data"] {
		remove(filepath.Join(base, "state.sqlite3"))
		remove(filepath.Join(base, "state.sqlite3-wal"))
		remove(filepath.Join(base, "state.sqlite3-shm"))
	}
	if selected["login_item"] {
		if err := setLoginItem(false); err != nil && removeErr == nil {
			removeErr = err
		}
	}
	if removeErr != nil {
		return nativeError(fmt.Errorf("清理本机数据失败: %w", removeErr))
	}

	if selected["application"] {
		uninstaller := filepath.Join(`C:\Program Files`, "DeskFuse", "Uninstall.exe")
		if _, err := os.Stat(uninstaller); err != nil {
			// 没有安装器时只能删除自身之外的文件并如实报告。
			return nativeError(fmt.Errorf("找不到 %s，无法卸载应用本身；已移除所选数据", uninstaller))
		}
		cmd := exec.Command(uninstaller, "/S")
		if err := cmd.Start(); err != nil {
			return nativeError(fmt.Errorf("启动卸载程序失败: %w", err))
		}
		if s.host != nil {
			s.host.Quit()
		}
		return assign(result, map[string]any{"outcome": "dispatched", "action": "uninstall.perform"})
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "uninstall.perform"})
}
