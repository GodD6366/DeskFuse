// Package devices owns the UI-facing device list details that the platform
// layer cannot express in plain fields: the product artwork the native app
// showed next to every device.
//
// macOS rasterizes the exact CoreTypes image (or the native SF Symbol fallback)
// and the middle layer caches it; platforms without that artwork leave the entry
// untouched and the page keeps its drawn fallback. The snapshot never fails
// because an icon is unavailable.
package devices

import (
	"crypto/sha1"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"sync"
)

// Caller runs one native platform primitive through app.invoke.
type Caller func(op string, params map[string]any) (map[string]any, error)

// iconPixels is the raster size requested from the platform. The page renders
// at 42–58 pt, so 116 px covers Retina without bloating the blob cache.
const iconPixels = 116

type Service struct {
	call    Caller
	mu      sync.Mutex
	icons   map[string][]byte
	missing map[string]bool
}

func New(call Caller) *Service {
	return &Service{call: call, icons: map[string][]byte{}, missing: map[string]bool{}}
}

// Compose adds icon_url to the local entry and to every device the platform can
// render. Unavailable icons are remembered so each snapshot does not retry them
// until the device is renamed.
func (s *Service) Compose(snapshot map[string]any) {
	if local, ok := snapshot["local"].(map[string]any); ok {
		s.Attach(local, "local")
	}
	list, _ := snapshot["devices"].([]any)
	for _, raw := range list {
		if device, ok := raw.(map[string]any); ok {
			s.Attach(device, "device")
		}
	}
}

// Attach resolves one entry's artwork and records icon_url when it exists.
func (s *Service) Attach(entry map[string]any, kind string) {
	name, _ := entry["name"].(string)
	if name == "" {
		return
	}
	model, _ := entry["model_identifier"].(string)
	key := iconKey(kind, name, model)
	switch s.lookup(key) {
	case iconReady:
		entry["icon_url"] = "deskfuse://blob/device-icon/" + key
		return
	case iconMissing:
		return
	}
	if s.call == nil {
		return
	}
	value, err := s.call("devices.icon", map[string]any{
		"name": name, "model_identifier": model, "size": iconPixels,
	})
	if err != nil {
		return
	}
	if available, _ := value["available"].(bool); !available {
		s.remember(key, nil)
		return
	}
	encoded, _ := value["png_base64"].(string)
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil || len(data) == 0 {
		return
	}
	s.remember(key, data)
	entry["icon_url"] = "deskfuse://blob/device-icon/" + key
}

// Photo serves a cached icon to the blob route.
func (s *Service) Photo(key string) ([]byte, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	data, ok := s.icons[key]
	if !ok {
		return nil, errors.New("未知的设备图标")
	}
	return data, nil
}

type iconState int

const (
	iconUnknown iconState = iota
	iconReady
	iconMissing
)

func (s *Service) lookup(key string) iconState {
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, ok := s.icons[key]; ok {
		return iconReady
	}
	if s.missing[key] {
		return iconMissing
	}
	return iconUnknown
}

func (s *Service) remember(key string, data []byte) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if data == nil {
		s.missing[key] = true
		return
	}
	s.icons[key] = data
}

// iconKey identifies one rendered icon. A rename or a different model resolves
// to a new key, so stale artwork never follows a device.
func iconKey(kind, name, model string) string {
	sum := sha1.Sum([]byte(kind + "\x00" + name + "\x00" + model))
	return hex.EncodeToString(sum[:16])
}
