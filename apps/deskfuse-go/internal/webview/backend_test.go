package webview

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
)

type fakeCaller struct {
	op      string
	ops     []string
	params  any
	body    string
	respond func(op string, params any) string
}

func (f *fakeCaller) Call(_ context.Context, op string, params any, result any) error {
	f.op = op
	f.ops = append(f.ops, op)
	f.params = params
	body := f.body
	if f.respond != nil {
		body = f.respond(op, params)
	}
	*(result.(*json.RawMessage)) = json.RawMessage(body)
	return nil
}
func TestIdentifiersRemainLossless(t *testing.T) {
	caller := &fakeCaller{body: `{"items":[{"id":18446744073709551615,"length":42,"related_ids":[18446744073709551614]}],"revision":9223372036854775809}`}
	result, err := New(caller).QueryLibrary(Object{"op": "items"})
	if err != nil {
		t.Fatal(err)
	}
	item := result["items"].([]any)[0].(map[string]any)
	if item["id"] != "18446744073709551615" || result["revision"] != "9223372036854775809" {
		t.Fatal(result)
	}
	if item["related_ids"].([]any)[0] != "18446744073709551614" {
		t.Fatal(item)
	}
}
func TestBlobRejectsArbitraryPaths(t *testing.T) {
	backend := New(nil)
	for _, path := range []string{"deskfuse://blob/image/etc/passwd", "deskfuse://blob/image/not-an-id", "deskfuse://other/image/1"} {
		response := httptest.NewRecorder()
		BlobHandler(backend)(response, httptest.NewRequest("GET", path, nil))
		if response.Code != 400 && response.Code != 404 {
			t.Fatalf("%s: %d", path, response.Code)
		}
	}
}
func TestBlobServesBytesOutsideJSONIPC(t *testing.T) {
	caller := &fakeCaller{body: `{"data_base64":"aW1hZ2U="}`}
	response := httptest.NewRecorder()
	BlobHandler(New(caller))(response, httptest.NewRequest("GET", "deskfuse://blob/thumbnail/12?size=240", nil))
	if response.Code != 200 || response.Body.String() != "image" || caller.op != "clipboard.thumbnail" {
		t.Fatal(response, caller)
	}
}
func TestUnavailableBackendDoesNotFabricateData(t *testing.T) {
	if _, err := New(nil).Snapshot(); err == nil || !strings.Contains(err.Error(), "未连接") {
		t.Fatal(err)
	}
}

func TestSnapshotComposesMouseView(t *testing.T) {
	caller := &fakeCaller{body: `{"mouse":{"devices":[{"id":"a","name":"G502 LIGHTSPEED","model":"g502Lightspeed"},{"id":"b","name":"Some Mouse"}],"battery":[],"wheel":[],"g502_locations":{}}}`}
	result, err := New(caller).Snapshot()
	if err != nil {
		t.Fatal(err)
	}
	mouse := result["mouse"].(map[string]any)
	if models := mouse["models"].([]map[string]any); len(models) != 6 {
		t.Fatalf("models = %d", len(models))
	}
	options := mouse["options"].([]map[string]any)
	if len(options) != 2 || options[0]["id"] != "model:g502Lightspeed" || options[1]["id"] != "generic" {
		t.Fatalf("options = %v", options)
	}
}

func TestMousePhotoBlobServesBundledPhoto(t *testing.T) {
	caller := &fakeCaller{body: `{"data_base64":"cGhvdG8="}`}
	response := httptest.NewRecorder()
	BlobHandler(New(caller))(response, httptest.NewRequest("GET", "deskfuse://blob/mouse/g502-lightspeed-top", nil))
	if response.Code != 200 || response.Body.String() != "photo" || caller.op != "app.invoke" {
		t.Fatal(response, caller.op)
	}
	payload, _ := caller.params.(map[string]any)
	if payload["action"] != "mouse.photo" {
		t.Fatalf("action = %v", payload["action"])
	}
}

func TestInvokeRoutesMouseMappingThroughMiddleLayer(t *testing.T) {
	caller := &fakeCaller{body: `{"outcome":"dispatched"}`}
	_, err := New(caller).Invoke("mouse.setMapping", Object{
		"button": 210, "modifiers": 0,
		"action": Object{"kind": "shortcut", "key_code": 8, "modifiers": 1048576},
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(caller.ops) == 0 || caller.ops[len(caller.ops)-1] != "app.invoke" {
		t.Fatalf("middle layer did not route through app.invoke: %v", caller.ops)
	}
	payload, _ := caller.params.(map[string]any)
	if payload["action"] != "mouse.applyBinding" {
		t.Fatalf("last action = %v", payload["action"])
	}
}

func TestSnapshotAttachesDeviceArtwork(t *testing.T) {
	snapshot := `{"local":{"name":"godd-Mac","model_identifier":"Mac14,2"},"devices":[{"id":"p","name":"DeskFuse Windows"}]}`
	caller := &fakeCaller{respond: func(op string, params any) string {
		if op != "app.invoke" {
			return snapshot
		}
		payload, _ := params.(map[string]any)
		if payload["action"] == "devices.icon" {
			return `{"outcome":"ok","value":{"available":true,"png_base64":"aWNvbg=="}}`
		}
		return `{"outcome":"ok"}`
	}}
	result, err := New(caller).Snapshot()
	if err != nil {
		t.Fatal(err)
	}
	iconURL := func(entry map[string]any) string {
		value, _ := entry["icon_url"].(string)
		if !strings.HasPrefix(value, "deskfuse://blob/device-icon/") {
			t.Fatalf("icon_url = %q", value)
		}
		return value
	}
	local := result["local"].(map[string]any)
	localURL := iconURL(local)
	device := result["devices"].([]any)[0].(map[string]any)
	iconURL(device)

	response := httptest.NewRecorder()
	BlobHandler(New(nil))(response, httptest.NewRequest("GET", "deskfuse://blob/device-icon/"+strings.TrimPrefix(localURL, "deskfuse://blob/device-icon/"), nil))
	if response.Code != 404 {
		t.Fatalf("a backend without the platform cache must not serve artwork: %d", response.Code)
	}
	_ = device
}

func TestDeviceIconBlobServesCachedArtwork(t *testing.T) {
	snapshot := `{"local":{"name":"godd-Mac","model_identifier":"Mac14,2"}}`
	iconCalls := 0
	caller := &fakeCaller{respond: func(op string, params any) string {
		if op != "app.invoke" {
			return snapshot
		}
		if payload, _ := params.(map[string]any); payload["action"] == "devices.icon" {
			iconCalls++
		}
		return `{"outcome":"ok","value":{"available":true,"png_base64":"aWNvbg=="}}`
	}}
	backend := New(caller)
	result, err := backend.Snapshot()
	if err != nil {
		t.Fatal(err)
	}
	local := result["local"].(map[string]any)
	url, _ := local["icon_url"].(string)
	response := httptest.NewRecorder()
	BlobHandler(backend)(response, httptest.NewRequest("GET", url, nil))
	if response.Code != 200 || response.Body.String() != "icon" {
		t.Fatalf("%d %q", response.Code, response.Body.String())
	}
	// The second snapshot reuses the cached blob instead of asking again.
	if _, err := backend.Snapshot(); err != nil {
		t.Fatal(err)
	}
	if iconCalls != 1 {
		t.Fatalf("devices.icon called %d times", iconCalls)
	}
}
