package winservice

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
)

// preferences persists the desktop client's own UI preferences. Service
// settings stay in the service; only host-side choices (keep awake, whether
// workflows are shown) live here so they survive a restart.
type preferences struct {
	mu     sync.Mutex
	path   string
	values map[string]any
}

func newPreferences() *preferences {
	base := os.Getenv("LOCALAPPDATA")
	if base == "" {
		base = os.TempDir()
	}
	p := &preferences{
		path:   filepath.Join(base, "DeskFuse", "mygo-ui-preferences.json"),
		values: map[string]any{},
	}
	data, err := os.ReadFile(p.path)
	if err == nil {
		_ = json.Unmarshal(data, &p.values)
	}
	return p
}

func (p *preferences) Bool(key string) bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	value, _ := p.values[key].(bool)
	return value
}

func (p *preferences) Get(key string) (any, bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	value, ok := p.values[key]
	return value, ok
}

func (p *preferences) Set(key string, value any) error {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.values[key] = value
	if err := os.MkdirAll(filepath.Dir(p.path), 0o755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(p.values, "", "  ")
	if err != nil {
		return err
	}
	// Write next to the target and rename so a crash cannot leave a partial
	// preferences file that the next start would fail to decode.
	temporary := p.path + ".tmp"
	if err := os.WriteFile(temporary, data, 0o644); err != nil {
		return err
	}
	return os.Rename(temporary, p.path)
}

// valuesCopy returns a copy of the stored preferences for diagnostics export.
func (p *preferences) valuesCopy() map[string]any {
	p.mu.Lock()
	defer p.mu.Unlock()
	out := make(map[string]any, len(p.values))
	for key, value := range p.values {
		out[key] = value
	}
	return out
}
