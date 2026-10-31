package mouse

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"
)

type fakeCaller struct {
	calls   []string
	params  []Object
	results map[string]Object
}

func (f *fakeCaller) call(op string, params Object) (Object, error) {
	f.calls = append(f.calls, op)
	f.params = append(f.params, params)
	if result, ok := f.results[op]; ok {
		return result, nil
	}
	return Object{"outcome": "dispatched"}, nil
}

func seed(t *testing.T) (*Service, *fakeCaller) {
	t.Helper()
	fake := &fakeCaller{results: map[string]Object{}}
	service := New(fake.call)
	// Seed the location cache the way a real snapshot would.
	service.Compose(Object{
		"devices":        []any{},
		"g502_locations": Object{},
	})
	return service, fake
}

func TestComposeGroupsModelsAndJoinsCapabilities(t *testing.T) {
	service, _ := seed(t)
	view := service.Compose(Object{
		"devices": []any{
			Object{"id": "a", "name": "G502 LIGHTSPEED", "model": "g502Lightspeed"},
			Object{"id": "b", "name": "Some Mouse"},
		},
		"battery": []any{
			Object{"name": "G502 LIGHTSPEED", "model": "g502Lightspeed", "percent": json.Number("80"), "charging": false},
		},
		"wheel": []any{
			Object{"name": "G502 LIGHTSPEED", "model": "g502Lightspeed", "freespin": true},
		},
		"g502_locations": Object{"left": json.Number("218")},
	})

	options := view["options"].([]Object)
	if len(options) != 2 || options[0]["id"] != "model:g502Lightspeed" || options[1]["id"] != "generic" {
		t.Fatalf("unexpected options: %+v", options)
	}
	if ids := options[1]["device_ids"].([]string); !reflect.DeepEqual(ids, []string{"b"}) {
		t.Fatalf("generic devices = %v", ids)
	}

	models := view["models"].([]Object)
	g502 := models[0]
	if g502["connected"] != true || g502["battery_percent"] != json.Number("80") ||
		g502["battery_charging"] != false || g502["wheel_available"] != true || g502["wheel_freespin"] != true {
		t.Fatalf("unexpected model join: %+v", g502)
	}
	locations := view["g502"].(Object)["locations"].(map[string]int)
	if locations["left"] != 218 {
		t.Fatalf("location override not applied: %v", locations["left"])
	}
}

func TestSetMappingRejectsPhysicalButtonConflicts(t *testing.T) {
	service, fake := seed(t)
	for _, action := range []Object{
		{"kind": "mouse", "button": json.Number("0")},
		{"kind": "disabled"},
	} {
		handled, _, err := service.Invoke("mouse.setMapping", Object{
			"button": json.Number("210"), "modifiers": json.Number("0"), "action": action,
		})
		if !handled || err == nil {
			t.Fatalf("expected %v to be rejected, got handled=%v err=%v", action, handled, err)
		}
	}
	if len(fake.calls) != 0 {
		t.Fatalf("rejected mapping reached the platform: %v", fake.calls)
	}
}

func TestSetMappingRemovesG502GenericBinding(t *testing.T) {
	service, fake := seed(t)
	handled, _, err := service.Invoke("mouse.setMapping", Object{
		"button": json.Number("210"), "modifiers": json.Number("0"),
		"action": Object{"kind": "shortcut", "key_code": json.Number("8"), "modifiers": json.Number("1048576")},
	})
	if !handled || err != nil {
		t.Fatalf("setMapping failed: handled=%v err=%v", handled, err)
	}
	if len(fake.calls) != 1 || fake.calls[0] != "mouse.applyBinding" {
		t.Fatalf("unexpected platform calls: %v", fake.calls)
	}
	removed := fake.params[0]["remove_buttons"].([]int)
	if !reflect.DeepEqual(removed, []int{2}) {
		t.Fatalf("remove_buttons = %v", removed)
	}
}

func TestLocateG502SwapsDisplacedEvent(t *testing.T) {
	service, fake := seed(t)
	_, _, err := service.Invoke("mouse.locateG502", Object{
		"position": "left", "button": json.Number("218"),
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(fake.calls) != 1 || fake.calls[0] != "mouse.saveLocations" {
		t.Fatalf("unexpected platform calls: %v", fake.calls)
	}
	saved := fake.params[0]["g502"].(map[string]int)
	if saved["left"] != 218 || saved["tiltRight"] != 208 {
		t.Fatalf("locate did not swap displaced event: %v", saved)
	}
}

func TestContributionMarkdown(t *testing.T) {
	service, _ := seed(t)
	result, err := service.Contribution(Object{
		"model_name": "MX Master 4", "connection": "bluetooth",
		"vendor_id": "046d", "product_id": "c548", "detected_name": "MX Master 4",
		"is_supported": true, "notes": "侧键需长按",
		"app_version": "0.7.1", "app_build": "120",
		"system_version": "macOS 26.0", "captured_at": "2026-10-31T10:00:00Z",
		"samples": []any{
			Object{"number": json.Number("4"), "name": "前进|键", "count": json.Number("2")},
			Object{"number": json.Number("3"), "name": "后退", "count": json.Number("1")},
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	markdown := result["markdown"].(string)
	for _, want := range []string{
		"# 鼠标适配数据：MX Master 4",
		"- DeskFuse：0.7.1 (120)",
		"- 连接方式：蓝牙",
		"- Vendor ID：0x046D",
		"- Product ID：0xC548",
		"- 当前适配状态：已适配",
		"## 补充说明\n\n侧键需长按",
		"| 3 | 后退 | 1 |\n| 4 | 前进\\|键 | 2 |",
	} {
		if !strings.Contains(markdown, want) {
			t.Fatalf("markdown missing %q:\n%s", want, markdown)
		}
	}
	if result["filename"] != "DeskFuse-鼠标数据-MX Master 4.md" {
		t.Fatalf("filename = %v", result["filename"])
	}
}
