package main

import (
	"io"
	"log"
	"os"
	"runtime"
	"time"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/bridge"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/clipboard"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/webview"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/winservice"
	"github.com/egoist/mygo"
)

// quitting 由退出流程置位。关闭窗口平时只收起界面：引擎、剪贴板共享和
// 托盘图标都在原生宿手里继续运行，点击 Dock 图标再打开窗口；只有真正退出
// 才允许窗口关闭。
var quitting bool

func main() {
	log.SetOutput(os.Stderr)
	log.SetPrefix("deskfuse: ")
	var caller webview.Caller
	var host *bridge.Client
	// Binding generation must never consume the inherited native protocol pipes.
	if os.Getenv("MYGO_GENERATE") == "" && os.Getenv("DESKFUSE_MYGO_BRIDGE") == "stdio-v1" {
		host = bridge.New(os.Stdin, os.Stdout)
		caller = host
	} else if runtime.GOOS == "windows" {
		// The Windows service owns identity, trust, network and clipboard policy;
		// this process is only the desktop UI on the other end of its pipe.
		caller = winservice.NewService(winservice.Dial(os.Getenv("DESKFUSE_WINDOWS_PIPE")))
	}
	var backend *webview.Backend
	if os.Getenv("DESKFUSE_UI_DEBUG") != "" {
		backend = webview.NewWithDebug(caller, func(op string, params any, err error) {
			log.Printf("ipc: op=%s params=%.160v err=%v", op, params, err)
		})
	} else {
		backend = webview.New(caller)
	}
	// Preview 是面板行的只读预览浮窗：页面给条目与行位置，窗口归 Go 进程。
	preview := &Preview{}
	mygo.Bind(backend, preview)
	if err := mygo.Protocol.HandleFunc("deskfuse", webview.BlobHandler(backend)); err != nil {
		log.Fatal(err)
	}
	mygo.SetFrontend(frontendAssets())
	mygo.App.OnBeforeQuit(func(*mygo.QuitEvent) { quitting = true })
	mygo.App.WhenReady(func() {
		log.Printf("ui: ready")
		url := "/"
		if dev := os.Getenv("DESKFUSE_UI_DEV_URL"); dev != "" {
			url = dev
		}
		window := mygo.NewWindow(mygo.WindowOptions{
			Title: "DeskFuse", URL: url, Width: 1120, Height: 720, MinWidth: 960, MinHeight: 620,
			TitleBarStyle: mygo.TitleBarHidden, TrafficLightPosition: &mygo.Point{X: 18, Y: 18},
			BackgroundColor: "light-dark(#f4f4f6, #1a1a1e)",
		})
		window.OnClose(func(e *mygo.CloseEvent) {
			if quitting {
				return
			}
			e.PreventDefault()
			window.Hide()
		})
		mygo.App.OnActivate(func(hasVisibleWindows bool) {
			// 点击 Dock 图标（或从 Finder 重新打开）时恢复被收起的窗口。
			if !quitting && !hasVisibleWindows {
				window.Show()
				window.Focus()
			}
		})
		panel := newPanelWindow()
		panel.preview = preview
		preview.attach(panel.window)
		preview.watchBlur(func() { panel.guardBlur(1500 * time.Millisecond) })
		aux := newAuxiliaryWindows()
		// 传输提示的关闭由进度窗口页面发起，窗口归属留在 Go 进程。
		backend.OnDismissTransfer = aux.dismiss
		if host != nil {
			go presentWindowEvents(window, panel, aux, host.Events())
		}
		window.OnClosed(func() { aux.stop(); preview.Stop(); log.Printf("ui: main window closed") })
		installSmoke(window, panel,
			func() (any, error) { return backend.Snapshot() },
			func(action string, params map[string]any) (any, error) { return backend.Invoke(action, params) },
			func(params map[string]any) (any, error) { return backend.QueryLibrary(params) },
		)
	})
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
	log.Printf("ui: app run returned")
}

// Retained for the existing transport activation tests during WebView cutover.
func newClipboardSource(stdin io.Reader, stdout io.Writer) clipboard.Source {
	if os.Getenv("DESKFUSE_MYGO_BRIDGE") == "stdio-v1" {
		return clipboard.NewBridgeSource(bridge.New(stdin, stdout))
	}
	return nil
}
