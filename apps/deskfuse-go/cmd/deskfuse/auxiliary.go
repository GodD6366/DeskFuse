package main

import (
	"encoding/json"
	"sync"

	"github.com/egoist/mygo"
)

// auxiliaryWindows owns the floating windows that used to be native SwiftUI
// scenes: the device pairing sheet and the transfer progress stack. Both are
// panels, so they float above other apps on every Space and never take focus
// from the app the user is working in. The pages render from the same engine
// snapshot as the main window; this type only decides when they are on screen
// and where.
type auxiliaryWindows struct {
	mu        sync.Mutex
	pairing   *mygo.Window
	transfers *mygo.Window
	dismissed map[string]bool
	last      auxiliaryState
}

// auxiliaryState is the subset of the engine snapshot the host pushes on every
// change: which requests still wait for a decision and what transfers exist.
type auxiliaryState struct {
	Pairing   []string            `json:"pairing"`
	Transfers []auxiliaryTransfer `json:"transfers"`
}

type auxiliaryTransfer struct {
	ID    string `json:"id"`
	Phase string `json:"phase"`
}

const (
	transferPanelWidth  = 316
	transferPanelPill   = 84 // 76 pt pill + 8 pt stack spacing
	transferPanelInset  = 16
	transferPanelMargin = 16
)

func newAuxiliaryWindows() *auxiliaryWindows {
	return &auxiliaryWindows{dismissed: map[string]bool{}}
}

func (a *auxiliaryWindows) reconcile(state auxiliaryState) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.last = state
	a.reconcilePairing()
	a.reconcileTransfers()
}

func (a *auxiliaryWindows) reconcilePairing() {
	if len(a.last.Pairing) == 0 {
		if a.pairing != nil {
			a.pairing.Hide()
		}
		return
	}
	if a.pairing == nil {
		a.pairing = mygo.NewWindow(mygo.WindowOptions{
			Title: "设备配对", URL: "/?page=pairing", Width: 480, Height: 320,
			Frameless: true, Panel: true, AlwaysOnTop: true, DisableResize: true,
			CornerRadius: 16, BackgroundColor: "light-dark(#ffffff, #1a1a1e)",
		})
	}
	// 与原生面板一致：出现新请求时前置，但不激活应用、不抢键盘。
	a.pairing.ShowInactive()
}

func (a *auxiliaryWindows) reconcileTransfers() {
	active := a.activeTransfers()
	if len(active) == 0 {
		if a.transfers != nil {
			a.transfers.Hide()
		}
		return
	}
	if a.transfers == nil {
		a.transfers = mygo.NewWindow(mygo.WindowOptions{
			Title: "传输进度", URL: "/?page=transfer-progress", Width: transferPanelWidth, Height: transferPanelPill,
			Frameless: true, Panel: true, AlwaysOnTop: true, DisableResize: true,
			Transparent: true, Vibrancy: mygo.VibrancyPopover, DisableShadow: true,
		})
	}
	area := mygo.Screen.DisplayNearestPoint(mygo.Screen.CursorScreenPoint()).WorkArea
	if area.Width <= 0 {
		area = mygo.Screen.PrimaryDisplay().WorkArea
	}
	height := len(active)*transferPanelPill + transferPanelInset
	if maximum := area.Height - 2*transferPanelMargin; height > maximum {
		height = maximum
	}
	a.transfers.SetBounds(mygo.Rectangle{
		X:      area.X + area.Width - transferPanelWidth - transferPanelMargin,
		Y:      area.Y + transferPanelMargin,
		Width:  transferPanelWidth,
		Height: height,
	})
	a.transfers.ShowInactive()
}

// activeTransfers drops completed transfers and the ones the user closed. The
// native panel auto-dismissed cancelled transfers after a short delay; the page
// keeps that timer and calls DismissTransfer, so both paths land here.
func (a *auxiliaryWindows) activeTransfers() []auxiliaryTransfer {
	live := make(map[string]bool, len(a.last.Transfers))
	active := make([]auxiliaryTransfer, 0, len(a.last.Transfers))
	for _, task := range a.last.Transfers {
		live[task.ID] = true
		if task.Phase == "completed" || a.dismissed[task.ID] {
			continue
		}
		active = append(active, task)
	}
	for id := range a.dismissed {
		if !live[id] {
			delete(a.dismissed, id)
		}
	}
	return active
}

// dismiss closes one pill and re-evaluates visibility immediately, so closing
// the last failure does not wait for the next snapshot push.
func (a *auxiliaryWindows) dismiss(id string) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.dismissed[id] = true
	a.reconcileTransfers()
}

// stop hides both windows when the app quits; the process itself owns their
// lifetime from here.
func (a *auxiliaryWindows) stop() {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.pairing != nil {
		a.pairing.Hide()
	}
	if a.transfers != nil {
		a.transfers.Hide()
	}
}

func decodeAuxiliaryState(params json.RawMessage) auxiliaryState {
	var state auxiliaryState
	if len(params) > 0 {
		_ = json.Unmarshal(params, &state)
	}
	return state
}
