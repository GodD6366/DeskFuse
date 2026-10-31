package main

import (
	"encoding/json"
	"fmt"
	"log"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/bridge"
	"github.com/egoist/mygo"
)

// presentWindowEvents turns native window requests (menu items, status item,
// shortcuts) into the same focus and page switch a user click would produce.
func presentWindowEvents(window *mygo.Window, panel *panelWindow, aux *auxiliaryWindows, events <-chan bridge.Event) {
	for event := range events {
		switch event.Name {
		case "ui.present":
			var payload struct {
				Page             string `json:"page"`
				SettingsCategory string `json:"settings_category"`
			}
			if len(event.Params) > 0 {
				_ = json.Unmarshal(event.Params, &payload)
			}
			panel.hide()
			window.Show()
			window.Focus()
			if payload.Page == "" {
				continue
			}
			script := fmt.Sprintf(`window.__deskfuseNavigate && window.__deskfuseNavigate(%q, %q)`,
				payload.Page, payload.SettingsCategory)
			if _, err := window.Page().Eval(script); err != nil {
				log.Printf("navigate to %q failed: %v", payload.Page, err)
			}
		case "ui.close":
			window.Hide()
		case "ui.dock":
			// Dock 可见性由原生宿主计算（设置项 + 远程应用覆盖）：产品窗口在
			// 这个进程里，Dock 图标也就归它，宿主自己始终保持 accessory。
			var payload struct {
				Visible bool `json:"visible"`
			}
			if len(event.Params) > 0 {
				_ = json.Unmarshal(event.Params, &payload)
			}
			applyDockPolicy(payload.Visible)
		case "panel.show":
			panel.present(event.Params)
		case "panel.toggle":
			panel.toggle(event.Params)
		case "panel.hide":
			panel.hide()
		case "ui.aux":
			aux.reconcile(decodeAuxiliaryState(event.Params))
		}
	}
	// 原生宿主退出（包括安装流程直接结束进程）后管道关闭：界面没有引擎可连，
	// 必须自己退出，否则会留下一个没有后端的第二个 DeskFuse 进程和图标。
	log.Printf("native host disconnected; quitting")
	mygo.App.Quit()
}

// applyDockPolicy shows or hides this process's Dock icon. The native host runs
// as an accessory app while this client is alive, so the product window's own
// process is the one the Dock shows.
func applyDockPolicy(visible bool) {
	if visible {
		mygo.App.Dock.Show()
		return
	}
	mygo.App.Dock.Hide()
}
