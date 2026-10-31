package devices

import (
	"encoding/base64"
	"errors"
	"strings"
	"testing"
)

var errPlatform = errors.New("platform unavailable")

func TestAttachCachesArtworkByDeviceIdentity(t *testing.T) {
	calls := 0
	call := func(op string, params map[string]any) (map[string]any, error) {
		calls++
		if op != "devices.icon" || params["name"] != "godd-Mac" || params["model_identifier"] != "Mac14,2" {
			t.Fatalf("op = %s params = %v", op, params)
		}
		return map[string]any{"available": true, "png_base64": base64.StdEncoding.EncodeToString([]byte("art"))}, nil
	}
	service := New(call)
	entry := map[string]any{"name": "godd-Mac", "model_identifier": "Mac14,2"}
	service.Attach(entry, "local")
	url, _ := entry["icon_url"].(string)
	if !strings.HasPrefix(url, "deskfuse://blob/device-icon/") {
		t.Fatalf("icon_url = %q", url)
	}
	service.Attach(entry, "local")
	if calls != 1 {
		t.Fatalf("icon resolved %d times", calls)
	}
	data, err := service.Photo(strings.TrimPrefix(url, "deskfuse://blob/device-icon/"))
	if err != nil || string(data) != "art" {
		t.Fatalf("%v %q", err, data)
	}
}

func TestAttachRemembersMissingArtworkWithoutBlockingTheSnapshot(t *testing.T) {
	calls := 0
	service := New(func(string, map[string]any) (map[string]any, error) {
		calls++
		return map[string]any{"available": false}, nil
	})
	entry := map[string]any{"name": "DeskFuse Windows"}
	service.Attach(entry, "device")
	if _, ok := entry["icon_url"]; ok {
		t.Fatal("unavailable artwork produced an icon_url")
	}
	service.Attach(entry, "device")
	if calls != 1 {
		t.Fatalf("missing artwork was retried %d times", calls)
	}
	if _, err := service.Photo("whatever"); err == nil {
		t.Fatal("unknown key served a photo")
	}
}

func TestAttachSurvivesPlatformErrorsAndRenames(t *testing.T) {
	failing := true
	calls := 0
	service := New(func(_ string, params map[string]any) (map[string]any, error) {
		calls++
		if failing {
			return nil, errPlatform
		}
		if params["name"] != "renamed" {
			t.Fatalf("name = %v", params["name"])
		}
		return map[string]any{"available": true, "png_base64": "aWNvbg=="}, nil
	})
	entry := map[string]any{"name": "old"}
	service.Attach(entry, "device")
	if _, ok := entry["icon_url"]; ok {
		t.Fatal("a platform error produced an icon_url")
	}
	// A platform error is not cached: the renamed device asks again.
	failing = false
	entry["name"] = "renamed"
	service.Attach(entry, "device")
	if _, ok := entry["icon_url"]; !ok {
		t.Fatal("renamed device did not resolve artwork")
	}
	if calls != 2 {
		t.Fatalf("calls = %d", calls)
	}
}

func TestAttachWithoutNameOrCallerDoesNothing(t *testing.T) {
	service := New(nil)
	service.Attach(map[string]any{}, "device")
	service.Attach(map[string]any{"name": "godd-Mac"}, "local")
}
