//go:build windows

package main

import (
	_ "embed"
	"encoding/json"
	"log"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/winservice"
	"github.com/egoist/mygo"
)

// trayIcon is the app icon shown in the notification area (256 px, scaled by
// the shell). It is the same artwork as the Windows desktop package.
//
//go:embed tray.png
var trayIcon []byte

// defaultPanelShortcut opens the clipboard panel anywhere, matching the
// Windows canonical default (the macOS ⌃⌘V maps to this in the adapter).
const defaultPanelShortcut = "CmdOrCtrl+Shift+V"

// installWindowsIntegration adds the notification-area icon, the global
// clipboard shortcut, the staging-area window wiring, snapshot-driven
// notifications and the startup update check. Together with the tray menu they
// replace the macOS menu bar item that the shared UI expects.
func installWindowsIntegration(window *mygo.Window, panel *panelWindow, shelf *shelfWindow, service *winservice.Service) {
	host := hostActions{window: window, panel: panel, shelf: shelf}
	hotkey := newPanelHotkey(func() {
		if panel.window.IsVisible() {
			panel.hide()
			return
		}
		showPanelAtCursor(panel)
	})
	host.hotkey = hotkey

	tray, err := mygo.NewTray(mygo.TrayOptions{
		ID:      "deskfuse-main",
		Icon:    trayIcon,
		ToolTip: "DeskFuse",
		Menu: mygo.NewMenu([]*mygo.MenuItem{
			{Label: "打开 DeskFuse", Click: func(*mygo.MenuItem, *mygo.Window) {
				host.ShowMain("", "")
			}},
			{Label: "剪贴板面板", Click: func(*mygo.MenuItem, *mygo.Window) {
				showPanelAtCursor(panel)
			}},
			{Label: "文件暂存区", Click: func(*mygo.MenuItem, *mygo.Window) {
				host.ShowShelf()
			}},
			mygo.Separator(),
			{Label: "退出 DeskFuse", Click: func(*mygo.MenuItem, *mygo.Window) {
				host.Quit()
			}},
		}),
	})
	if err != nil {
		log.Printf("tray icon unavailable: %v", err)
	} else {
		tray.OnClick(func() { host.ShowMain("", "") })
	}
	if err := hotkey.set(defaultPanelShortcut); err != nil {
		log.Printf("clipboard shortcut unavailable: %v", err)
	}

	if service != nil {
		notifier := newSnapshotNotifier(service.NativeActions())
		service.OnSnapshot = notifier.observe
		// Report update status at startup, like the macOS client's periodic check.
		go func() {
			_ = service.BackgroundInvoke("updates.check", map[string]any{})
		}()
	}
}

func (h hostActions) ShowPanel() { showPanelAtCursor(h.panel) }

// showPanelAtCursor opens the compact clipboard panel just below the pointer,
// clamped to the display work area so it never runs off-screen.
func showPanelAtCursor(panel *panelWindow) {
	if panel == nil || panel.window == nil {
		return
	}
	display := mygo.Screen.PrimaryDisplay()
	scale := display.ScaleFactor
	if scale <= 0 {
		scale = 1
	}
	width, height := 440, 720
	// Display bounds and the cursor are physical pixels; window sizes are DIPs.
	work := mygo.Rectangle{
		X:      int(float64(display.WorkArea.X) / scale),
		Y:      int(float64(display.WorkArea.Y) / scale),
		Width:  int(float64(display.WorkArea.Width) / scale),
		Height: int(float64(display.WorkArea.Height) / scale),
	}
	if work.Width <= 0 || work.Height <= 0 {
		return
	}
	if width > work.Width-16 {
		width = work.Width - 16
	}
	if height > work.Height-16 {
		height = work.Height - 16
	}
	cursor := mygo.Screen.CursorScreenPoint()
	x := int(float64(cursor.X)/scale) - width/2
	y := int(float64(cursor.Y)/scale) + 12
	if x < work.X+8 {
		x = work.X + 8
	}
	if limit := work.X + work.Width - width - 8; x > limit {
		x = limit
	}
	if y < work.Y+8 {
		y = work.Y + 8
	}
	if limit := work.Y + work.Height - height - 8; y > limit {
		y = limit
	}
	params, _ := json.Marshal(map[string]int{"x": x, "y": y, "width": width, "height": height})
	panel.present(params)
}
