package main

import (
	"encoding/json"
	"sync/atomic"
	"time"

	"github.com/egoist/mygo"
)

// panelWindow owns the compact clipboard panel opened by the status item or
// the global shortcut. It floats above other apps as a panel, like the native
// popover did, and loads the same Vue bundle as the main window.
type panelWindow struct {
	window  *mygo.Window
	preview *Preview
	// 预览 popover 会短暂夺走键盘；这段时间里面板失焦不算“点击别处”。
	blurGuard atomic.Int64
}

func newPanelWindow() *panelWindow {
	panel := &panelWindow{}
	panel.window = mygo.NewWindow(mygo.WindowOptions{
		Title:           "DeskFuse 剪贴板",
		URL:             "/?page=clipboard-panel",
		Width:           440,
		Height:          720,
		Hidden:          true,
		Frameless:       true,
		Panel:           true,
		AlwaysOnTop:     true,
		DisableResize:   true,
		CornerRadius:    12,
		BackgroundColor: "light-dark(#ffffff, #1a1a1e)",
	})
	// 失焦就收起，和原生 popover 的“点击别处关闭”一致；粘贴流程会先隐藏再激活目标应用。
	panel.window.OnBlur(func() {
		if time.Now().UnixNano() < panel.blurGuard.Load() {
			return
		}
		panel.hide()
	})
	return panel
}

// present shows the panel at the top-left screen point the native host
// computed, so the popover keeps the native placement rules.
func (p *panelWindow) present(params json.RawMessage) {
	if p == nil || p.window == nil {
		return
	}
	var request struct {
		X      *int `json:"x"`
		Y      *int `json:"y"`
		Width  *int `json:"width"`
		Height *int `json:"height"`
	}
	if len(params) > 0 {
		_ = json.Unmarshal(params, &request)
	}
	width, height := 440, 720
	if request.Width != nil && *request.Width >= 320 {
		width = *request.Width
	}
	if request.Height != nil && *request.Height >= 240 {
		height = *request.Height
	}
	p.window.SetSize(width, height)
	if request.X != nil && request.Y != nil {
		p.window.SetPosition(*request.X, *request.Y)
	}
	if p.preview != nil {
		_ = p.preview.Hide()
	}
	p.window.Show()
	p.window.Focus()
	// 面板窗口只是隐藏而不是卸载；每次打开时通知页面重新查询。
	if page := p.window.Page(); page != nil {
		_, _ = page.Eval(`window.__deskfusePanelShown && window.__deskfusePanelShown()`)
	}
}

// toggle hides a visible panel and otherwise shows it at the given point. The
// Go side owns this state because losing focus hides the window directly.
func (p *panelWindow) toggle(params json.RawMessage) {
	if p == nil || p.window == nil {
		return
	}
	if p.window.IsVisible() {
		p.hide()
		return
	}
	p.present(params)
}

// guardBlur ignores the blur that showing the preview popover causes.
func (p *panelWindow) guardBlur(d time.Duration) {
	p.blurGuard.Store(time.Now().Add(d).UnixNano())
}

func (p *panelWindow) hide() {
	if p == nil || p.window == nil {
		return
	}
	if p.preview != nil {
		_ = p.preview.Hide()
	}
	p.window.Hide()
}
