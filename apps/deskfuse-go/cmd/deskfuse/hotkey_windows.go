//go:build windows

package main

import (
	"sync"

	"github.com/egoist/mygo"
)

// panelHotkey owns the global shortcut that toggles the clipboard panel. It
// supports swapping the accelerator (settings → 快捷键) and suspending it
// while the page records a different shortcut.
type panelHotkey struct {
	mu        sync.Mutex
	accel     string
	suspended bool
	press     func()
	registered bool
}

func newPanelHotkey(press func()) *panelHotkey {
	return &panelHotkey{press: press}
}

// set registers a new accelerator; an empty string disables the hotkey.
func (h *panelHotkey) set(accel string) error {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.registered {
		mygo.GlobalShortcut.Unregister(h.accel)
		h.registered = false
	}
	h.accel = accel
	if accel == "" || h.suspended {
		return nil
	}
	if err := mygo.GlobalShortcut.Register(accel, h.press); err != nil {
		return err
	}
	h.registered = true
	return nil
}

// suspend unregisters the hotkey without forgetting the accelerator, so the
// page can record keys we would otherwise swallow.
func (h *panelHotkey) suspend(suspended bool) error {
	h.mu.Lock()
	defer h.mu.Unlock()
	if suspended == h.suspended {
		return nil
	}
	h.suspended = suspended
	if suspended {
		if h.registered {
			mygo.GlobalShortcut.Unregister(h.accel)
			h.registered = false
		}
		return nil
	}
	if h.accel != "" && !h.registered {
		if err := mygo.GlobalShortcut.Register(h.accel, h.press); err != nil {
			return err
		}
		h.registered = true
	}
	return nil
}
