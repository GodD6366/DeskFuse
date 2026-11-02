//go:build windows

package main

import (
	"github.com/egoist/mygo"
)

// shelfWindow owns the floating staging-area panel. It hosts the same Vue
// bundle with page=shelf and mirrors the macOS shelf panel: it floats above
// other apps, keeps its size, and hides on blur once the user clicks away.
type shelfWindow struct {
	window *mygo.Window
}

func newShelfWindow() *shelfWindow {
	shelf := &shelfWindow{}
	shelf.window = mygo.NewWindow(mygo.WindowOptions{
		Title:           "DeskFuse 暂存区",
		URL:             "/?page=shelf",
		Width:           280,
		Height:          320,
		Hidden:          true,
		Frameless:       true,
		Panel:           true,
		AlwaysOnTop:     true,
		DisableResize:   true,
		CornerRadius:    16,
		BackgroundColor: "light-dark(#ffffff, #1a1a1e)",
	})
	shelf.window.OnBlur(func() { shelf.hide() })
	return shelf
}

func (s *shelfWindow) show() {
	if s == nil || s.window == nil {
		return
	}
	centerFloating(s.window, 280, 320)
	s.window.Show()
	s.window.Focus()
}

func (s *shelfWindow) hide() {
	if s == nil || s.window == nil {
		return
	}
	s.window.Hide()
}

// centerFloating places a small floating window near the screen center,
// converted to DIP coordinates like the panel placement.
func centerFloating(window *mygo.Window, width, height int) {
	display := mygo.Screen.PrimaryDisplay()
	scale := display.ScaleFactor
	if scale <= 0 {
		scale = 1
	}
	workX := int(float64(display.WorkArea.X) / scale)
	workY := int(float64(display.WorkArea.Y) / scale)
	workWidth := int(float64(display.WorkArea.Width) / scale)
	workHeight := int(float64(display.WorkArea.Height) / scale)
	x := workX + (workWidth-width)/2
	y := workY + (workHeight-height)/3
	window.SetPosition(x, y)
}
