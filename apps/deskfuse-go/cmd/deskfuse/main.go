package main

import (
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/url"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/bridge"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/clipboard"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/native"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/webview"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/winservice"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/transfer"
)

// quitting 由退出流程置位。关闭窗口平时只收起界面：引擎、剪贴板共享和
// 托盘图标都在原生宿手里继续运行，点击 Dock 图标再打开窗口；只有真正退出
// 才允许窗口关闭。
var quitting bool

func main() {
	log.SetOutput(os.Stderr)
	log.SetPrefix("deskfuse: ")
	// 服务管道一次只接受一个桌面客户端；第二次启动把命令行交给已在运行的
	// 实例并退出，避免两个窗口争用同一管道。
	if !mygo.App.RequestSingleInstanceLock() {
		return
	}
	var caller webview.Caller
	var host *bridge.Client
	var service *winservice.Service
	// Binding generation must never consume the inherited native protocol pipes.
	if os.Getenv("MYGO_GENERATE") == "" && os.Getenv("DESKFUSE_MYGO_BRIDGE") == "stdio-v1" {
		host = bridge.New(os.Stdin, os.Stdout)
		caller = host
	} else if runtime.GOOS == "windows" {
		// The Windows service owns identity, trust, network and clipboard policy;
		// this process is only the desktop UI on the other end of its pipe. The
		// native actions own the system clipboard, dialogs and shell, and the
		// window callbacks are installed once the windows exist.
		service = winservice.NewService(winservice.Dial(os.Getenv("DESKFUSE_WINDOWS_PIPE")))
		service.SetNative(native.New())
		caller = service
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
	mygo.App.OnBeforeQuit(func(*mygo.QuitEvent) {
		quitting = true
		// 剪贴板以延迟呈现的形式归本进程所有；退出前落盘，内容才不会随
		// 进程一起消失。
		_ = mygo.Clipboard.Flush()
	})
	mygo.App.WhenReady(func() {
		log.Printf("ui: ready")
		startURL := "/"
		if dev := os.Getenv("DESKFUSE_UI_DEV_URL"); dev != "" {
			startURL = dev
		} else if route := os.Getenv("DESKFUSE_INITIAL_ROUTE"); route != "" {
			// 轻量模式下宿主回收了界面进程，重新拉起时直接把落点写进 URL：
			// `page` 或 `page:category`（设置分类）。
			page, category, _ := strings.Cut(route, ":")
			values := url.Values{}
			values.Set("page", page)
			if category != "" {
				values.Set("category", category)
			}
			startURL = "/?" + values.Encode()
		}
		// 轻量模式恢复：宿主回收了界面进程，重新拉起时直接落到请求的界面。
		// 目标是剪贴板面板时主窗口保持隐蔽，只弹面板。
		panelOnly := os.Getenv("DESKFUSE_PANEL_FRAME") != ""
		// Windows 高缩放比下 1120×720 DIP 可能大于整个桌面；按主屏工作区的
		// DIP 尺寸收敛默认窗口，Min 尺寸也不能超过屏幕。
		width, height := fitWindow(1120, 720)
		minWidth, minHeight := fitWindow(960, 620)
		window := mygo.NewWindow(mygo.WindowOptions{
			Title: "DeskFuse", URL: startURL, Width: width, Height: height,
			MinWidth: minWidth, MinHeight: minHeight,
			TitleBarStyle: mygo.TitleBarHidden, TrafficLightPosition: &mygo.Point{X: 18, Y: 18},
			BackgroundColor: "light-dark(#f4f4f6, #1a1a1e)", Hidden: panelOnly,
		})
		if service != nil {
			centerWindow(window, width, height)
		}
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
		// 面板位置由宿主按状态栏图标算出；随启动参数带过来，省掉一次事件往返。
		if frame := os.Getenv("DESKFUSE_PANEL_FRAME"); frame != "" {
			var x, y, width, height int
			if _, err := fmt.Sscanf(frame, "%d,%d,%d,%d", &x, &y, &width, &height); err == nil {
				payload, _ := json.Marshal(map[string]int{"x": x, "y": y, "width": width, "height": height})
				panel.present(payload)
			}
		}
		panel.preview = preview
		preview.attach(panel.window)
		preview.watchBlur(func() { panel.guardBlur(1500 * time.Millisecond) })
		// The Windows host owns window actions and the tray/shortcut entry
		// points; on macOS the native host supplies those itself.
		if service != nil {
			shelf := newShelfWindow()
			service.SetHost(hostActions{window: window, panel: panel, shelf: shelf})
			installWindowsIntegration(window, panel, shelf, service)
			// 暂存区文件拖出：把暂存路径作为 CF_HDROP 交给系统拖拽。
			backend.OnShelfDrag = func(paths []string) error {
				lines := make([]string, 0, len(paths))
				for _, path := range paths {
					absolute, err := filepath.Abs(path)
					if err != nil {
						return err
					}
					lines = append(lines, (&url.URL{Scheme: "file", Path: "/" + filepath.ToSlash(absolute)}).String())
				}
				data := transfer.New(transfer.NewItem(transfer.Bytes(transfer.FileList, []byte(strings.Join(lines, "\r\n")))))
				return shelf.window.StartDataDrag(data, mygo.DragOptions{Operations: transfer.Copy})
			}
		}
		aux := newAuxiliaryWindows()
		// 传输提示的关闭由进度窗口页面发起，窗口归属留在 Go 进程。
		backend.OnDismissTransfer = aux.dismiss
		if host != nil {
			go presentWindowEvents(window, panel, aux, host.Events())
		}
		mygo.App.OnSecondInstance(func([]string, string) {
			if !quitting {
				window.Show()
				window.Focus()
			}
		})
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
