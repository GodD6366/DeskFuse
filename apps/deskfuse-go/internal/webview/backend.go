// Package webview exposes the existing native host to MyGo's typed page IPC.
package webview

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/devices"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/mouse"
)

type Caller interface {
	Call(context.Context, string, any, any) error
}
type Object = map[string]any

// Backend contains no UI or database ownership. Native state remains authoritative.
type Backend struct {
	client Caller
	debug  func(op string, params any, err error)
	// mouse owns the mouse domain's middle layer: view composition and policy.
	mouse *mouse.Service
	// devices passes the platform's product artwork through as cached blobs.
	devices *devices.Service
	// DismissTransfer 由页面调用，隐藏一条已失败或已取消的传输提示。提示是否
	// 显示属于界面状态，因此不写回引擎；实现由浮动窗口管理器注入。
	OnDismissTransfer func(string)
}

func New(client Caller) *Backend {
	backend := &Backend{client: client}
	backend.mouse = mouse.New(backend.platformCall)
	backend.devices = devices.New(backend.platformCall)
	return backend
}

// NewWithDebug records every native round trip; used by the UI smoke harness.
// It is a constructor rather than a method because mygo.Bind exports every
// exported method as page IPC, and a func parameter cannot cross that boundary.
func NewWithDebug(client Caller, report func(op string, params any, err error)) *Backend {
	backend := &Backend{client: client, debug: report}
	backend.mouse = mouse.New(backend.platformCall)
	backend.devices = devices.New(backend.platformCall)
	return backend
}

// platformCall runs a native platform primitive through app.invoke; the bridge
// protocol only accepts a fixed op set, so primitives are actions, not ops.
// Native replies wrap action values in {"outcome":…,"value":…}; the middle
// layer sees the value directly.
func (b *Backend) platformCall(op string, params mouse.Object) (mouse.Object, error) {
	reply, err := b.requestWithin("app.invoke", Object{"action": op, "params": params}, 15*time.Second)
	if err != nil {
		return nil, err
	}
	if value, ok := reply["value"].(Object); ok {
		return value, nil
	}
	return reply, nil
}
func (b *Backend) request(op string, params any) (Object, error) {
	return b.requestWithin(op, params, 15*time.Second)
}

// requestWithin is request with an explicit deadline. 保存面板等原生窗口由用户
// 操作，可能远超普通查询的 15 秒；超时只结束等待，不会取消原生动作。
func (b *Backend) requestWithin(op string, params any, timeout time.Duration) (Object, error) {
	if b.client == nil {
		return nil, errors.New("原生服务未连接，请通过 DeskFuse 原生宿主启动此界面。")
	}
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	var raw json.RawMessage
	err := b.client.Call(ctx, op, params, &raw)
	if b.debug != nil {
		b.debug(op, params, err)
	}
	if err != nil {
		return nil, err
	}
	var value Object
	decoder := json.NewDecoder(strings.NewReader(string(raw)))
	decoder.UseNumber()
	if err := decoder.Decode(&value); err != nil {
		return nil, fmt.Errorf("原生服务返回无效 JSON: %w", err)
	}
	return normalize(value).(map[string]any), nil
}

// Keep engine uint64 identifiers lossless when MyGo serializes them for JavaScript.
func normalize(value any) any {
	switch v := value.(type) {
	case map[string]any:
		for key, child := range v {
			switch key {
			case "id", "revision", "counts_revision", "build":
				if n, ok := child.(json.Number); ok {
					v[key] = n.String()
					continue
				}
			case "ids", "related_ids":
				if values, ok := child.([]any); ok {
					for i, n := range values {
						if number, ok := n.(json.Number); ok {
							values[i] = number.String()
						}
					}
					v[key] = values
					continue
				}
			}
			v[key] = normalize(child)
		}
		return v
	case []any:
		for i, child := range v {
			v[i] = normalize(child)
		}
		return v
	default:
		return value
	}
}
func (b *Backend) Snapshot() (Object, error) {
	snapshot, err := b.request("app.snapshot", Object{"include_history": false})
	if err != nil {
		return nil, err
	}
	// The Go middle layer owns the UI-facing mouse view; the native host only
	// supplies devices, battery, SmartShift and G502 locations.
	if raw, ok := snapshot["mouse"].(Object); ok {
		snapshot["mouse"] = b.mouse.Compose(raw)
	}
	// Device artwork is a platform primitive; the page only renders icon_url.
	b.devices.Compose(snapshot)
	return snapshot, nil
}
func (b *Backend) Hello() (Object, error) {
	return b.request("hello", Object{"client": "mygo-webview", "requires": []string{"app.snapshot", "app.invoke"}})
}
func (b *Backend) QueryLibrary(params Object) (Object, error) {
	return b.request("library.query", params)
}
func (b *Backend) MutateLibrary(params Object) (Object, error) {
	return b.request("library.mutate", params)
}
func (b *Backend) Invoke(action string, params Object) (Object, error) {
	// Mouse mapping policy, G502 positions and reset confirmation belong to the
	// Go middle layer; pure platform primitives continue to the native host.
	if handled, result, err := b.mouse.Invoke(action, params); handled {
		if b.debug != nil {
			b.debug("middle:"+action, params, err)
		}
		return result, err
	}
	timeout := 15 * time.Second
	switch action {
	// 需要用户在选择面板或管理员授权里操作，结果只在用户响应后返回。
	case "clipboard.save", "mouse.contribute.save", "uninstall.perform", "download_and_install_update":
		timeout = 10 * time.Minute
	// 首次使用会弹出系统权限对话框。
	case "microphone.speech.toggle":
		timeout = 2 * time.Minute
	}
	return b.requestWithin("app.invoke", Object{"action": action, "params": params}, timeout)
}

// MouseContribution builds the adapter data Markdown in the middle layer; the
// native probe only reports raw button samples.
func (b *Backend) MouseContribution(params Object) (Object, error) {
	return b.mouse.Contribution(params)
}

// DismissTransfer hides a failed or cancelled transfer pill. A method rather
// than a page event because the page owns the click that dismisses it.
func (b *Backend) DismissTransfer(id string) error {
	if id != "" && b.OnDismissTransfer != nil {
		b.OnDismissTransfer(id)
	}
	return nil
}
func (b *Backend) Content(id string) (Object, error) {
	if _, err := strconv.ParseUint(id, 10, 64); err != nil {
		return nil, errors.New("无效内容 ID")
	}
	value, err := b.request("clipboard.content", Object{"id": id, "include_image": false, "include_rtf": false})
	if err != nil {
		return nil, err
	}
	delete(value, "image_png_base64")
	delete(value, "rtf_base64")
	value["image_url"] = ""
	if hasImage, _ := value["has_image"].(bool); hasImage {
		value["image_url"] = "deskfuse://blob/image/" + id
	}
	return value, nil
}

// Blob serves only native-owned content IDs; arbitrary filesystem paths are never accepted.
func BlobHandler(b *Backend) http.HandlerFunc { return b.blob }

func (b *Backend) blob(w http.ResponseWriter, r *http.Request) {
	if r.Method != "GET" && r.Method != "HEAD" {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if r.URL.Host != "blob" {
		http.NotFound(w, r)
		return
	}
	parts := strings.Split(strings.Trim(r.URL.Path, "/"), "/")
	if len(parts) != 2 {
		http.NotFound(w, r)
		return
	}
	// Device artwork is rasterized by the platform host once and cached here.
	if parts[0] == "device-icon" {
		data, err := b.devices.Photo(parts[1])
		if err != nil {
			http.NotFound(w, r)
			return
		}
		writeImage(w, r, data, "private, max-age=3600")
		return
	}
	// Bundled mouse photos are static UI assets; the middle layer caches them.
	if parts[0] == "mouse" {
		data, err := b.mouse.Photo(parts[1])
		if err != nil {
			http.NotFound(w, r)
			return
		}
		writeImage(w, r, data, "private, max-age=3600")
		return
	}
	if _, err := strconv.ParseUint(parts[1], 10, 64); err != nil {
		http.Error(w, "invalid id", http.StatusBadRequest)
		return
	}
	var result Object
	var err error
	var encoded string
	switch parts[0] {
	case "thumbnail":
		size := 240
		if s := r.URL.Query().Get("size"); s != "" {
			size, err = strconv.Atoi(s)
			if err != nil || size < 64 || size > 480 {
				http.Error(w, "invalid size", 400)
				return
			}
		}
		result, err = b.request("clipboard.thumbnail", Object{"id": parts[1], "max_pixel": size})
		if result != nil {
			encoded, _ = result["data_base64"].(string)
		}
	case "image":
		result, err = b.request("clipboard.content", Object{"id": parts[1], "include_image": true, "include_rtf": false})
		if result != nil {
			encoded, _ = result["image_png_base64"].(string)
		}
	default:
		http.NotFound(w, r)
		return
	}
	if err != nil {
		http.Error(w, "原生内容服务不可用", http.StatusServiceUnavailable)
		return
	}
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil || len(data) == 0 {
		http.NotFound(w, r)
		return
	}
	writeImage(w, r, data, "private, no-store")
}

func writeImage(w http.ResponseWriter, r *http.Request, data []byte, cache string) {
	w.Header().Set("Content-Type", "image/png")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Cache-Control", cache)
	w.Header().Set("Content-Length", strconv.Itoa(len(data)))
	if r.Method == "GET" {
		_, _ = w.Write(data)
	}
}
