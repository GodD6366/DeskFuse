// Package ui is the shared MyGo interface used by macOS and Windows.
//
// The view reads immutable app.Snapshots and dispatches app.Commands; it owns
// no business state and never queries platform APIs directly. MyGo is pinned
// in go.mod and stays confined to this package and cmd/deskfuse.
package ui

import (
	myui "github.com/egoist/mygo/ui"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/app"
)

const sidebarWidth float32 = 228

// View returns the root native UI content for a MyGo window.
func View(application *app.App) *myui.Content {
	state := &viewState{}
	return myui.View(func(c *myui.Context) {
		if application == nil {
			myui.Column(c).Fill().Center().Children(func() {
				myui.Text(c, "DeskFuse 应用状态未初始化").FontSize(16).Bold()
			})
			return
		}

		snapshot := application.Snapshot()
		theme := c.Theme()
		myui.Row(c).Fill().Children(func() {
			renderSidebar(c, application, snapshot, theme)
			renderContent(c, application, snapshot, theme, state)
		})
	})
}

func renderSidebar(c *myui.Context, application *app.App, snapshot app.Snapshot, theme *myui.Theme) {
	myui.Column(c).
		Width(sidebarWidth).
		FillHeight().
		Background(theme.Surface).
		Padding(theme.Space(4)).
		Gap(theme.Space(2)).
		Children(func() {
			myui.Text(c, "DeskFuse").FontSize(22).Bold()
			myui.Text(c, "Go UI 骨架").FontSize(12).TextColor(theme.TextMuted)

			for _, item := range navigation {
				if item.section == snapshot.Section {
					myui.PrimaryButton(c.Key("nav."+string(item.section)), item.label).FillWidth()
					continue
				}
				if myui.Button(c.Key("nav."+string(item.section)), item.label).FillWidth().Clicked() {
					dispatch(c, application, item.command)
				}
			}

			myui.Spacer(c)
			myui.Text(c, platformLabel(snapshot.Platform)).FontSize(12).TextColor(theme.TextMuted)
			myui.Text(c, "业务与平台层不依赖 MyGo").FontSize(11).TextColor(theme.TextMuted)
		})
}

func renderContent(c *myui.Context, application *app.App, snapshot app.Snapshot, theme *myui.Theme, state *viewState) {
	if snapshot.Section == app.SectionClipboard {
		renderClipboardPage(c, application.Clipboard(), theme, state)
		return
	}

	myui.Scroll(c).
		Fill().
		Padding(theme.Space(8)).
		Gap(theme.Space(3)).
		Children(func() {
			myui.Text(c, sectionTitle(snapshot.Section)).FontSize(28).Bold()
			myui.Text(c, "共享 MyGo 界面 · "+platformLabel(snapshot.Platform)).
				FontSize(13).
				TextColor(theme.TextMuted)
			myui.Divider(c)
			myui.Text(c, snapshot.Status).FontSize(15).MaxWidth(680)
			myui.Text(c, "当前阶段只接通 Go module、MyGo 应用入口、共享 UI 与剪贴板只读页；设备、传输、键鼠和设置能力仍由现有原生实现负责，尚未桥接。").
				FontSize(13).
				MaxWidth(680).
				TextColor(theme.TextMuted)
			myui.Spacer(c)
			myui.Textf(c, "界面修订 %d · %s", snapshot.Revision, capabilitySummary(snapshot.Capabilities)).
				FontSize(11).
				TextColor(theme.TextMuted)
		})
}

func dispatch(c *myui.Context, application *app.App, command app.Command) {
	application.Dispatch(command)
	c.Invalidate()
}
