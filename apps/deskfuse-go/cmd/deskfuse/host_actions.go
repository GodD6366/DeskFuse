package main

import (
	"fmt"
	"log"
	"runtime"

	"github.com/egoist/mygo"
)

// hostActions implements winservice.HostActions for the windows this process
// owns. The service never shows or closes a window itself.
type hostActions struct {
	window *mygo.Window
	panel  *panelWindow
	shelf  *shelfWindow
	hotkey *panelHotkey
}

func (h hostActions) Quit() {
	quitting = true
	mygo.App.Quit()
}

func (h hostActions) ShowMain(page, category string) {
	if h.panel != nil {
		h.panel.hide()
	}
	if h.window == nil {
		return
	}
	h.window.Show()
	h.window.Focus()
	if page == "" {
		return
	}
	script := fmt.Sprintf(`window.__deskfuseNavigate && window.__deskfuseNavigate(%q, %q)`, page, category)
	if handle := h.window.Page(); handle != nil {
		if _, err := handle.Eval(script); err != nil {
			log.Printf("navigate to %q failed: %v", page, err)
		}
	}
}

func (h hostActions) HidePanel() {
	if h.panel != nil {
		h.panel.hide()
	}
}

// centerWindow places the main window in the middle of the primary display's
// work area. MyGo's automatic centering uses screen coordinates that do not
// account for the display scale on Windows, so the position is set explicitly.
func centerWindow(window *mygo.Window, width, height int) {
	display := mygo.Screen.PrimaryDisplay()
	scale := display.ScaleFactor
	if scale <= 0 || display.WorkArea.Width <= 0 || display.WorkArea.Height <= 0 {
		return
	}
	workX := int(float64(display.WorkArea.X) / scale)
	workY := int(float64(display.WorkArea.Y) / scale)
	workWidth := int(float64(display.WorkArea.Width) / scale)
	workHeight := int(float64(display.WorkArea.Height) / scale)
	window.SetPosition(workX+(workWidth-width)/2, workY+(workHeight-height)/2)
}

// fitWindow narrows a requested DIP window size to the primary display's work
// area. macOS 的 NSScreen.visibleFrame 本身就是逻辑点（和窗口选项同一单位），
// 只有 Windows 返回物理像素、需要先除以缩放比；两者用同一套除法会把 macOS
// 窗口缩成一半（1360×860 变成 1000×500）。
func fitWindow(width, height int) (int, int) {
	display := mygo.Screen.PrimaryDisplay()
	scale := 1.0
	if runtime.GOOS == "windows" {
		scale = display.ScaleFactor
		if scale <= 0 {
			scale = 1
		}
	}
	if display.WorkArea.Width <= 0 || display.WorkArea.Height <= 0 {
		return width, height
	}
	if maxWidth := int(float64(display.WorkArea.Width)/scale) - 24; maxWidth > 320 && width > maxWidth {
		width = maxWidth
	}
	if maxHeight := int(float64(display.WorkArea.Height)/scale) - 24; maxHeight > 240 && height > maxHeight {
		height = maxHeight
	}
	return width, height
}

func (h hostActions) ShowShelf() {
	if h.shelf != nil {
		h.shelf.show()
	}
}

func (h hostActions) HideShelf() {
	if h.shelf != nil {
		h.shelf.hide()
	}
}

func (h hostActions) SetPanelShortcut(accelerator string) error {
	if h.hotkey == nil {
		return fmt.Errorf("桌面宿主尚未就绪")
	}
	return h.hotkey.set(accelerator)
}

func (h hostActions) SuspendPanelShortcut(suspend bool) error {
	if h.hotkey == nil {
		return fmt.Errorf("桌面宿主尚未就绪")
	}
	return h.hotkey.suspend(suspend)
}
