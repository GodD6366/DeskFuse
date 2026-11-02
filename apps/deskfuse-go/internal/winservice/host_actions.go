package winservice

import (
	"fmt"
	"net/url"
	"strings"
)

// maxIssueURLLength keeps the prefilled GitHub URL below the length the site
// accepts; the error text is truncated rather than dropping the report.
const maxIssueURLLength = 7000

// hostAction runs window and application actions owned by the desktop process.
func (s *Service) hostAction(action string, args map[string]any, result any) error {
	if s.host == nil {
		return unsupported(fmt.Sprintf("此窗口没有接入 %s 对应的桌面宿主操作。", action))
	}
	switch action {
	case "app.quit":
		s.host.Quit()
	case "app.showMain":
		s.host.ShowMain("", "")
	case "native.openPage":
		page, _ := stringValue(args, "page")
		category, _ := stringValue(args, "category", "settings_category")
		s.host.ShowMain(page, category)
	case "panel.hide":
		s.host.HidePanel()
	case "clipboard.showInMain":
		s.host.ShowMain("clipboard", "")
	case "clipboard.showPanel", "clipboard.togglePanel":
		return unsupported("Windows 版还没有剪贴板悬浮面板的入口；请从主窗口使用剪贴板。")
	}
	return assign(result, map[string]any{"outcome": "applied", "action": action})
}

// openURL opens an http(s) link in the default browser. The link is validated
// by the native layer, not trusted from the page.
func (s *Service) openURL(args map[string]any, result any) error {
	if s.native == nil {
		return unsupported("此窗口没有接入系统浏览器。")
	}
	raw, _ := stringValue(args, "url")
	if raw == "" {
		return invalidParams("需要 url")
	}
	if err := s.native.OpenURL(raw); err != nil {
		return nativeError(err)
	}
	return assign(result, map[string]any{"outcome": "dispatched", "action": "system.open_url"})
}

// reportProblem opens a prefilled GitHub issue with the environment summary
// and the raw error text. It never claims the report was submitted.
func (s *Service) reportProblem(args map[string]any, result any) error {
	if s.native == nil {
		return unsupported("此窗口没有接入系统浏览器。")
	}
	message, _ := stringValue(args, "error", "message")
	environment := s.environmentLines()
	body := strings.Join(environment, "\n")
	if message != "" {
		body = "问题描述：\n" + message + "\n\n" + body
	}
	link := "https://github.com/GodD6366/DeskFuse/issues/new?" + url.Values{
		"title": {"[Windows] " + firstLine(message)},
		"body":  {truncate(body, maxIssueURLLength/3)},
	}.Encode()
	if len(link) > maxIssueURLLength {
		link = truncate(link, maxIssueURLLength)
	}
	if err := s.native.OpenURL(link); err != nil {
		return nativeError(err)
	}
	return assign(result, map[string]any{"outcome": "dispatched", "action": "support.report"})
}

// copyEnvironment copies the same summary the issue page receives.
func (s *Service) copyEnvironment(result any) error {
	if s.native == nil {
		return unsupported("此窗口没有接入系统剪贴板。")
	}
	if err := s.native.CopyText(strings.Join(s.environmentLines(), "\n")); err != nil {
		return nativeError(err)
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "support.copy_environment"})
}

// environmentLines describes the client, service and devices without
// clipboard content, names of undiscovered devices or trust material.
func (s *Service) environmentLines() []string {
	lines := []string{"DeskFuse（Windows）· MyGo UI"}
	latest, ok := s.latestSnapshot()
	if !ok {
		lines = append(lines, "服务：尚未取得快照")
		return lines
	}
	lines = append(lines, fmt.Sprintf("版本：%s", latest.PackageVersion))
	lines = append(lines, fmt.Sprintf("本机：%s", displayName(latest.LocalName, latest.LocalDeviceID)))
	if latest.NetworkRunning {
		lines = append(lines, "网络：运行中")
	} else {
		lines = append(lines, "网络：未运行")
	}
	if latest.LastError != nil && *latest.LastError != "" {
		lines = append(lines, "最近错误："+*latest.LastError)
	}
	devices := make([]string, 0, len(latest.Peers))
	for _, peer := range latest.Peers {
		if peer.Trust != "trusted" {
			continue
		}
		state := "离线"
		if peer.Connection == "connected" {
			state = "在线"
		}
		devices = append(devices, fmt.Sprintf("%s（%s）", displayName(peer.DisplayName, peer.PeerID), state))
	}
	if len(devices) > 0 {
		lines = append(lines, "设备："+strings.Join(devices, "、"))
	}
	return lines
}

// setPreference stores a desktop-client preference. Only choices the Windows
// host can honour are accepted; macOS-only settings return an honest error.
func (s *Service) setPreference(args map[string]any, result any) error {
	key, _ := stringValue(args, "key")
	enabled, _ := boolValue(args, "value")
	if key == "" {
		return invalidParams("需要 key")
	}
	switch key {
	case "keep_awake":
		if err := setKeepAwake(enabled); err != nil {
			return nativeError(err)
		}
		if err := s.prefs.Set(key, enabled); err != nil {
			return nativeError(err)
		}
	case "workflows_enabled":
		if err := s.prefs.Set(key, enabled); err != nil {
			return nativeError(err)
		}
	default:
		return unsupported(fmt.Sprintf("Windows 版没有设置项 %s。", key))
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "settings.set", "value": enabled})
}

func firstLine(text string) string {
	if text == "" {
		return "使用中遇到的问题"
	}
	line := strings.TrimSpace(strings.SplitN(text, "\n", 2)[0])
	if line == "" {
		return "使用中遇到的问题"
	}
	if len([]rune(line)) > 60 {
		line = string([]rune(line)[:60]) + "…"
	}
	return line
}

func truncate(text string, limit int) string {
	runes := []rune(text)
	if len(runes) <= limit {
		return text
	}
	return string(runes[:limit]) + "\n…（已截断）"
}
