package main

import (
	"fmt"
	"log"
	"net/url"
	"sync"
	"time"

	"github.com/egoist/mygo"
)

// Preview owns the read-only quick look bubble of the menu bar clipboard
// panel. It is the system popover the native app showed, anchored to the
// selected row: AppKit draws its material, rounded corners, shadow and arrow.
// The panel takes the keyboard back right after showing it, so Space, arrows
// and Escape stay in the list.
type Preview struct {
	mu     sync.Mutex
	parent *mygo.Window
	// onShow lets the panel ignore the blur that showing a popover causes,
	// as the native panel re-made itself the key window right after showing.
	onShow      func()
	window      *mygo.Window
	id          string
	pending     string
	pendingWait bool
}

func (p *Preview) attach(parent *mygo.Window) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.parent = parent
}

func (p *Preview) watchBlur(fn func()) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.onShow = fn
}

// Show opens the bubble, or moves an open one to a new row and loads another
// record. Coordinates are the row's CSS pixels, which equal DIPs relative to
// the panel page's top-left corner.
func (p *Preview) Show(params map[string]any) error {
	id, _ := params["id"].(string)
	if id == "" {
		return fmt.Errorf("需要内容 ID")
	}
	// 远端索引条目还没有本地内容：页面在这种情况下向来源设备请求后再重试。
	wait, _ := params["wait"].(bool)
	anchor := previewAnchor(params)
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.parent == nil || p.parent.IsDestroyed() {
		return fmt.Errorf("面板尚未就绪")
	}
	if p.window == nil || p.window.IsDestroyed() {
		// 系统 popover：材质、圆角、阴影与指向行的箭头都由 AppKit 绘制。
		if p.onShow != nil {
			p.onShow()
		}
		window := mygo.NewFlyout(mygo.FlyoutOptions{
			Parent: p.parent, Anchor: anchor, Placement: mygo.PlacementLeft,
			Hidden: true, Popover: true,
			// 尺寸由页面内容决定；上下限与原生 preferredContentSize 相同。
			FitContent: true, MinWidth: 220, MaxWidth: 720, MinHeight: 104, MaxHeight: 560,
			URL: "/?page=quick-look&id=" + url.QueryEscape(id) + previewWaitQuery(wait),
		})
		if page := window.Page(); page != nil {
			page.OnDidFinishLoad(func() {
				p.mu.Lock()
				next, wait := p.pending, p.pendingWait
				p.pending, p.pendingWait = "", false
				p.mu.Unlock()
				if next != "" {
					p.eval(next, wait)
				}
			})
		}
		window.OnClosed(func() {
			p.mu.Lock()
			if p.window == window {
				p.window = nil
				p.id = ""
			}
			p.mu.Unlock()
		})
		p.window = window
		p.id = id
		parent := p.parent
		log.Printf("preview: show id=%s anchor=%+v", id, anchor)
		window.Show()
		// AppKit 会把键盘交给 popover；原生面板随即重新成为 key window，
		// 让空格、方向键与 Esc 继续留在面板里。
		p.focusParent(parent)
		return nil
	}
	log.Printf("preview: update id=%s anchor=%+v", id, anchor)
	p.window.SetAnchor(anchor)
	if p.id != id {
		p.id = id
		p.eval(id, wait)
	}
	if !p.window.IsVisible() {
		if p.onShow != nil {
			p.onShow()
		}
		p.window.Show()
		p.focusParent(p.parent)
	}
	return nil
}

// Key forwards a key pressed while the popover holds the keyboard to the
// panel list, as the native popover's key monitor did: Space and Escape
// close, arrows move, Return pastes.
func (p *Preview) Key(key string) error {
	p.mu.Lock()
	parent := p.parent
	p.mu.Unlock()
	if parent == nil || parent.IsDestroyed() {
		return nil
	}
	page := parent.Page()
	if page == nil {
		return nil
	}
	if _, err := page.Eval(fmt.Sprintf(`window.__deskfusePreviewKey && window.__deskfusePreviewKey(%q)`, key)); err != nil {
		return err
	}
	p.focusParent(parent)
	return nil
}

// Hide closes the bubble but keeps the window for the next preview.
func (p *Preview) Hide() error {
	p.mu.Lock()
	window := p.window
	id := p.id
	p.window = nil
	p.id = ""
	p.pending = ""
	p.pendingWait = false
	p.mu.Unlock()
	if window != nil && !window.IsDestroyed() {
		// 关闭 popover 用 Close：它把键盘还给面板并释放 popover；下一次
		// 预览重新创建。不要在持锁时关闭，OnClosed 会回来拿同一把锁。
		log.Printf("preview: hide id=%s", id)
		window.Close()
	}
	return nil
}

// focusParent takes the keyboard back from the popover. AppKit may re-key
// the popover once its opening animation ends, so this runs again shortly.
func (p *Preview) focusParent(parent *mygo.Window) {
	if parent == nil || parent.IsDestroyed() {
		return
	}
	parent.Focus()
	go func() {
		for _, delay := range []time.Duration{120 * time.Millisecond, 350 * time.Millisecond, 900 * time.Millisecond} {
			time.Sleep(delay)
			if parent.IsDestroyed() || !parent.IsVisible() {
				return
			}
			parent.Focus()
		}
	}()
}

// eval points the open page at another record; before the page has loaded, the
// id waits for OnDidFinishLoad.
func (p *Preview) eval(id string, wait bool) {
	page := p.window.Page()
	if page == nil {
		p.pending, p.pendingWait = id, wait
		return
	}
	script := fmt.Sprintf(`window.__deskfuseQuickLook && window.__deskfuseQuickLook(%q, %t)`, id, wait)
	if _, err := page.Eval(script); err != nil {
		p.pending, p.pendingWait = id, wait
		log.Printf("preview update deferred: %v", err)
	}
}

// previewWaitQuery tells the page whether to ask the source device for a
// payload that is not local yet.
func previewWaitQuery(wait bool) string {
	if wait {
		return "&wait=1"
	}
	return ""
}

// Stop hides the bubble when the app closes.
func (p *Preview) Stop() { _ = p.Hide() }

// previewAnchor reads a row rectangle and clamps it to a placeable size.
func previewAnchor(params map[string]any) mygo.Rectangle {
	value := func(key string) int {
		number, _ := params[key].(float64)
		return int(number)
	}
	anchor := mygo.Rectangle{X: value("x"), Y: value("y"), Width: value("width"), Height: value("height")}
	if anchor.Width < 1 {
		anchor.Width = 1
	}
	if anchor.Height < 1 {
		anchor.Height = 1
	}
	return anchor
}
