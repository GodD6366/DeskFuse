// Package mouse owns the middle-layer mouse domain: it folds the native
// platform primitives (devices, battery, SmartShift, G502 locations) into the
// view the WebView renders, enforces mapping policy, and builds the
// contribution Markdown. The Rust engine still persists settings; native hosts
// only expose primitives.
package mouse

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

type Object = map[string]any

// Caller is the native platform primitive surface. Backend injects a request
// function so round trips keep logging and deadlines.
type Caller func(op string, params Object) (Object, error)

type Service struct {
	call Caller

	mu     sync.Mutex
	photos map[string][]byte
	// locations caches the last G502 position→event map so policy decisions do
	// not block on a platform round trip.
	locations map[string]int
}

func New(call Caller) *Service {
	return &Service{call: call, photos: map[string][]byte{}, locations: map[string]int{}}
}

type model struct {
	ID      string
	Name    string
	Summary string
}

// Canonical order and copy mirror LogitechHIDProtocol.Model.
var models = []model{
	{"g502Lightspeed", "G502 LIGHTSPEED", "11 个实体键 · 保留原生功能"},
	{"mxMaster3", "MX Master 3 / 3S", "手势键、模式切换键与标准按键"},
	{"mxMaster4", "MX Master 4", "手势键、模式切换键与标准按键"},
	{"mxAnywhere3", "MX Anywhere 3 / 3S", "可编程扩展按键"},
	{"mxErgo", "MX Ergo / Ergo S", "可编程扩展按键"},
	{"mxVertical", "MX Vertical", "可编程扩展按键"},
}

// G502 positions. Physical locations are independent of the firmware's
// Button Spy order; the saved map only overrides after a re-identification.
var g502Positions = []string{
	"left", "right", "middle", "tiltLeft", "tiltRight",
	"dpiUp", "dpiDown", "top", "back", "forward", "thumb",
}

var g502Factory = map[string]int{
	"left": 208, "right": 209, "middle": 210, "back": 211, "forward": 212,
	"thumb": 213, "dpiDown": 214, "dpiUp": 215, "top": 216, "tiltLeft": 217, "tiltRight": 218,
}

var g502Generic = map[string]int{
	"left": 0, "right": 1, "middle": 2, "back": 3, "forward": 4,
}

func modelOf(name string) string {
	value := strings.ToLower(name)
	switch {
	case strings.Contains(value, "g502 lightspeed"):
		return "g502Lightspeed"
	case strings.Contains(value, "mx master 4"):
		return "mxMaster4"
	case strings.Contains(value, "mx master 3"):
		return "mxMaster3"
	case strings.Contains(value, "mx anywhere 3"), strings.Contains(value, "mx anywhere3"):
		return "mxAnywhere3"
	case strings.Contains(value, "mx ergo"):
		return "mxErgo"
	case strings.Contains(value, "mx vertical"):
		return "mxVertical"
	}
	return ""
}

// Compose turns the native mouse snapshot into the UI-facing view. It is
// idempotent: the native host keeps sending primitives, the Go middle layer
// adds grouping, capability joins and the G502 location cache.
func (s *Service) Compose(raw Object) Object {
	if raw == nil {
		return raw
	}
	battery := listOfObjects(raw["battery"])
	wheel := listOfObjects(raw["wheel"])
	devices := listOfObjects(raw["devices"])

	for _, device := range devices {
		name, _ := device["name"].(string)
		id := text(device["model"])
		if id == "" {
			id = modelOf(name)
		}
		device["model"] = nullableString(id)
		applyDeviceCapabilities(device, modelOf(name), battery, wheel)
	}

	options := make([]Object, 0, len(models)+1)
	for _, entry := range models {
		ids := make([]string, 0, 1)
		for _, device := range devices {
			id, _ := device["id"].(string)
			if text(device["model"]) == entry.ID {
				ids = append(ids, id)
			}
		}
		if len(ids) == 0 {
			continue
		}
		options = append(options, Object{
			"id": "model:" + entry.ID, "title": entry.Name, "model": entry.ID, "device_ids": ids,
		})
	}
	generic := make([]string, 0, 1)
	for _, device := range devices {
		if text(device["model"]) == "" {
			if id, ok := device["id"].(string); ok {
				generic = append(generic, id)
			}
		}
	}
	if len(generic) > 0 || len(options) == 0 {
		options = append(options, Object{
			"id": "generic", "title": "通用鼠标", "model": nil, "device_ids": generic,
		})
	}
	raw["options"] = options

	composed := make([]Object, 0, len(models))
	for _, entry := range models {
		connected := false
		for _, device := range devices {
			if text(device["model"]) == entry.ID {
				connected = true
				break
			}
		}
		info := Object{
			"id": entry.ID, "name": entry.Name, "summary": entry.Summary, "connected": connected,
		}
		if batteryInfo := firstCapability(battery, entry.ID, "percent"); batteryInfo != nil {
			info["battery_percent"] = batteryInfo["percent"]
			info["battery_charging"] = batteryInfo["charging"]
		} else {
			info["battery_percent"] = nil
			info["battery_charging"] = false
		}
		if wheelInfo := firstCapability(wheel, entry.ID, "freespin"); wheelInfo != nil {
			info["wheel_available"] = true
			info["wheel_freespin"] = wheelInfo["freespin"]
			info["wheel_name"] = wheelInfo["name"]
		} else {
			info["wheel_available"] = false
			info["wheel_freespin"] = nil
			info["wheel_name"] = nil
		}
		composed = append(composed, info)
	}
	raw["models"] = composed

	locations := normalizeLocations(raw["g502_locations"])
	s.mu.Lock()
	s.locations = locations
	s.mu.Unlock()
	raw["g502"] = Object{
		"locations":        locations,
		"detecting":        raw["detecting"],
		"capture_released": raw["capture_released"],
		"last_button":      raw["last_button"],
	}
	return raw
}

func applyDeviceCapabilities(device Object, modelID string, battery, wheel []Object) {
	for _, info := range battery {
		if text(info["model"]) == modelID || (modelID != "" && modelOf(text(info["name"])) == modelID) {
			device["battery_percent"] = info["percent"]
			device["battery_charging"] = info["charging"]
			break
		}
	}
	for _, info := range wheel {
		if text(info["model"]) == modelID || (modelID != "" && modelOf(text(info["name"])) == modelID) {
			device["wheel_freespin"] = info["freespin"]
			break
		}
	}
}

func firstCapability(entries []Object, modelID, _ string) Object {
	for _, entry := range entries {
		if text(entry["model"]) == modelID || modelOf(text(entry["name"])) == modelID {
			return entry
		}
	}
	return nil
}

// Invoke handles the actions whose policy belongs to the middle layer. It
// returns handled=false for actions that are pure platform primitives.
func (s *Service) Invoke(action string, params Object) (bool, Object, error) {
	switch action {
	case "mouse.setMapping":
		result, err := s.setMapping(params)
		return true, result, err
	case "mouse.locateG502":
		result, err := s.locateG502(params)
		return true, result, err
	case "mouse.reset":
		if params["confirm"] != true {
			return true, nil, errors.New("恢复默认需要确认")
		}
		result, err := s.call("mouse.reset", Object{"confirm": true})
		return true, result, err
	case "mouse.setWheelMode":
		name, _ := params["name"].(string)
		freespin, ok := params["freespin"].(bool)
		if name == "" || !ok {
			return true, nil, errors.New("需要设备名称和滚轮模式")
		}
		result, err := s.call("mouse.setWheelMode", Object{"name": name, "freespin": freespin})
		return true, result, err
	}
	return false, nil, nil
}

func (s *Service) setMapping(params Object) (Object, error) {
	button, ok := intValue(params["button"])
	if !ok || button < 0 || button > 255 {
		return nil, errors.New("按键或修饰键无效")
	}
	modifiers, ok := uintValue(params["modifiers"])
	if !ok {
		return nil, errors.New("按键或修饰键无效")
	}
	action, err := actionOf(params["action"])
	if err != nil {
		return nil, err
	}
	if action != nil {
		kind, _ := action["kind"].(string)
		if kind == "mouse" && (button == 195 || button == 196 || isG502(button)) {
			return nil, errors.New("实体键不能映射为鼠标键")
		}
		if kind == "disabled" && isG502(button) {
			return nil, errors.New("G502 实体键不能停用")
		}
	}
	removed := make([]int, 0, 1)
	if isG502(button) {
		position := s.positionFor(button)
		if generic, ok := g502Generic[position]; ok {
			removed = append(removed, generic)
		}
	}
	payload := Object{"button": button, "modifiers": modifiers, "action": action, "remove_buttons": removed}
	return s.call("mouse.applyBinding", payload)
}

func (s *Service) locateG502(params Object) (Object, error) {
	position, _ := params["position"].(string)
	if _, ok := g502Factory[position]; !ok {
		return nil, errors.New("未知的 G502 位置")
	}
	button, ok := intValue(params["button"])
	if !ok || button < 208 || button > 218 {
		return nil, errors.New("需要位置和实体键编号")
	}
	locations := s.currentLocations()
	displaced := locations[position]
	for other, current := range locations {
		if current == button {
			locations[other] = displaced
			break
		}
	}
	locations[position] = button
	result, err := s.call("mouse.saveLocations", Object{"g502": locations})
	if err != nil {
		return nil, err
	}
	s.mu.Lock()
	s.locations = cloneLocations(locations)
	s.mu.Unlock()
	return result, nil
}

func (s *Service) currentLocations() map[string]int {
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.locations) == len(g502Factory) {
		return cloneLocations(s.locations)
	}
	locations := cloneLocations(g502Factory)
	if result, err := s.call("mouse.locations", Object{}); err == nil {
		if raw, ok := result["g502"].(Object); ok {
			for position, value := range raw {
				if number, ok := intValue(value); ok {
					locations[position] = number
				}
			}
		}
	}
	s.locations = cloneLocations(locations)
	return locations
}

func (s *Service) positionFor(button int) string {
	for position, value := range s.currentLocations() {
		if value == button {
			return position
		}
	}
	return ""
}

// Photo returns a bundled mouse photo, cached after the first platform round
// trip. Only the four native assets are accepted.
func (s *Service) Photo(name string) ([]byte, error) {
	switch name {
	case "g502-lightspeed-top", "g502-lightspeed-side", "mx-master-3-top", "mx-master-3-side":
	default:
		return nil, errors.New("未知的鼠标图片")
	}
	s.mu.Lock()
	if data, ok := s.photos[name]; ok {
		s.mu.Unlock()
		return data, nil
	}
	s.mu.Unlock()
	result, err := s.call("mouse.photo", Object{"name": name})
	if err != nil {
		return nil, err
	}
	encoded, _ := result["data_base64"].(string)
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil || len(data) == 0 {
		return nil, errors.New("鼠标图片不可用")
	}
	s.mu.Lock()
	s.photos[name] = data
	s.mu.Unlock()
	return data, nil
}

// Contribution builds the adapter data Markdown and file name. This is pure
// middle-layer formatting; the platform probe only reports raw samples.
func (s *Service) Contribution(params Object) (Object, error) {
	modelName := strings.TrimSpace(stringValue(params["model_name"]))
	title := modelName
	if title == "" {
		title = "未命名鼠标"
	}
	connection := stringValue(params["connection"])
	connectionTitle := map[string]string{
		"wired": "有线", "receiver": "USB 接收器", "bluetooth": "蓝牙",
	}[connection]
	if connectionTitle == "" {
		connectionTitle = connection
	}
	version := stringValue(params["app_version"])
	build := stringValue(params["app_build"])
	versionText := version
	if build != "" {
		versionText = fmt.Sprintf("%s (%s)", version, build)
	}
	detected := strings.TrimSpace(stringValue(params["detected_name"]))
	if detected == "" {
		detected = "未提供"
	}
	adaptation := "未适配（通用鼠标）"
	if supported, _ := params["is_supported"].(bool); supported {
		adaptation = "已适配"
	}
	captured := stringValue(params["captured_at"])
	if captured == "" {
		captured = time.Now().UTC().Format(time.RFC3339)
	}

	lines := []string{
		"# 鼠标适配数据：" + title,
		"",
		"## 环境",
		"- DeskFuse：" + versionText,
		"- macOS：" + stringValue(params["system_version"]),
		"- 采集时间：" + captured,
		"- 连接方式：" + connectionTitle,
		"- Vendor ID：" + formattedID(stringValue(params["vendor_id"])),
		"- Product ID：" + formattedID(stringValue(params["product_id"])),
		"- 系统检测名称：" + detected,
		"- 当前适配状态：" + adaptation,
	}
	if notes := strings.TrimSpace(stringValue(params["notes"])); notes != "" {
		lines = append(lines, "", "## 补充说明", "", notes)
	}
	lines = append(lines, "", "## 按键识别", "", "编号为 macOS `mouseEventButtonNumber`，从 0 开始。", "")
	samples := samplesOf(params["samples"])
	if len(samples) == 0 {
		lines = append(lines, "未记录到按键。")
	} else {
		lines = append(lines, "| 编号 | 名称 | 按下次数 |", "| ---: | --- | ---: |")
		for _, sample := range samples {
			lines = append(lines, fmt.Sprintf("| %d | %s | %d |",
				sample.number, escapeCell(sample.name), sample.count))
		}
	}
	markdown := strings.Join(lines, "\n") + "\n"
	return Object{"markdown": markdown, "filename": contributionFilename(title)}, nil
}

type sample struct {
	number int
	name   string
	count  int
}

func samplesOf(raw any) []sample {
	entries, _ := raw.([]any)
	result := make([]sample, 0, len(entries))
	for _, entry := range entries {
		value, ok := entry.(Object)
		if !ok {
			continue
		}
		number, ok := intValue(value["number"])
		if !ok {
			continue
		}
		count, _ := intValue(value["count"])
		result = append(result, sample{number: number, name: stringValue(value["name"]), count: count})
	}
	sort.Slice(result, func(i, j int) bool { return result[i].number < result[j].number })
	return result
}

func contributionFilename(title string) string {
	invalid := "/:\\?%*|\"<>"
	safe := strings.Map(func(r rune) rune {
		if strings.ContainsRune(invalid, r) {
			return '-'
		}
		return r
	}, strings.TrimSpace(title))
	if safe == "" {
		safe = "未命名"
	}
	return "DeskFuse-鼠标数据-" + safe + ".md"
}

func escapeCell(value string) string {
	return strings.ReplaceAll(strings.ReplaceAll(value, "|", "\\|"), "\n", " ")
}

func formattedID(raw string) string {
	value := strings.TrimSpace(raw)
	if value == "" {
		return "未提供"
	}
	body := value
	if strings.HasPrefix(strings.ToLower(body), "0x") {
		body = body[2:]
	}
	if number, err := strconv.ParseUint(body, 16, 16); err == nil {
		return fmt.Sprintf("0x%04X", number)
	}
	return value
}

func actionOf(raw any) (Object, error) {
	if raw == nil {
		return nil, nil
	}
	value, ok := raw.(Object)
	if !ok {
		return nil, errors.New("按键动作无效")
	}
	kind, _ := value["kind"].(string)
	switch kind {
	case "disabled":
		return value, nil
	case "mouse":
		if _, ok := intValue(value["button"]); !ok {
			return nil, errors.New("缺少目标按键")
		}
	case "shortcut":
		if _, ok := intValue(value["key_code"]); !ok {
			return nil, errors.New("缺少快捷键")
		}
	default:
		return nil, errors.New("按键动作无效")
	}
	return value, nil
}

func normalizeLocations(raw any) map[string]int {
	locations := cloneLocations(g502Factory)
	if value, ok := raw.(Object); ok {
		for position, entry := range value {
			if _, known := g502Factory[position]; !known {
				continue
			}
			if number, ok := intValue(entry); ok && number >= 208 && number <= 218 {
				locations[position] = number
			}
		}
	}
	return locations
}

func cloneLocations(source map[string]int) map[string]int {
	target := make(map[string]int, len(source))
	for key, value := range source {
		target[key] = value
	}
	return target
}

func isG502(button int) bool { return button >= 208 && button <= 218 }

func listOfObjects(raw any) []Object {
	values, _ := raw.([]any)
	result := make([]Object, 0, len(values))
	for _, value := range values {
		if entry, ok := value.(Object); ok {
			result = append(result, entry)
		}
	}
	return result
}

func stringValue(raw any) string {
	value, _ := raw.(string)
	return value
}

func text(raw any) string {
	value, _ := raw.(string)
	return value
}

func nullableString(value string) any {
	if value == "" {
		return nil
	}
	return value
}

func intValue(raw any) (int, bool) {
	switch value := raw.(type) {
	case json.Number:
		number, err := value.Int64()
		return int(number), err == nil
	case float64:
		return int(value), true
	case float32:
		return int(value), true
	case int:
		return value, true
	case int64:
		return int(value), true
	case string:
		number, err := strconv.Atoi(value)
		return number, err == nil
	}
	return 0, false
}

func uintValue(raw any) (uint64, bool) {
	switch value := raw.(type) {
	case json.Number:
		number, err := value.Int64()
		if err != nil || number < 0 {
			return 0, false
		}
		return uint64(number), true
	case float64:
		if value < 0 {
			return 0, false
		}
		return uint64(value), true
	case int:
		if value < 0 {
			return 0, false
		}
		return uint64(value), true
	case int64:
		if value < 0 {
			return 0, false
		}
		return uint64(value), true
	case uint64:
		return value, true
	case string:
		number, err := strconv.ParseUint(value, 10, 64)
		return number, err == nil
	}
	return 0, false
}
