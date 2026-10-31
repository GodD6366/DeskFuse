package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"time"

	"github.com/egoist/mygo"
)

// 启动真实 WebView 后执行只读界面冒烟检查；不申请权限、不操作业务数据。
//
// 用法：DESKFUSE_UI_SMOKE_OUTPUT=<目录> 启动。每次运行等待桥接快照到达后再截图，
// 避免把“正在启动”的中间态当成页面结果。
func installSmoke(window *mygo.Window, panel *panelWindow, snapshot func() (any, error), invoke func(string, map[string]any) (any, error), queryLibrary func(map[string]any) (any, error)) {
	output := os.Getenv("DESKFUSE_UI_SMOKE_OUTPUT")
	if output == "" {
		return
	}
	window.Page().OnDidFinishLoad(func() {
		go func() {
			results := make(map[string]any)
			defer func() {
				if err := os.MkdirAll(output, 0700); err != nil {
					fmt.Fprintln(os.Stderr, err)
				}
				data, _ := json.MarshalIndent(results, "", "  ")
				_ = os.WriteFile(filepath.Join(output, "smoke.json"), data, 0600)
				mygo.App.Quit()
			}()
			page := window.Page()
			eval := func(script string) any {
				value, err := page.Eval(script)
				if err != nil {
					results["error"] = err.Error()
					return nil
				}
				return value
			}
			waitFor := func(script string, limit time.Duration) bool {
				deadline := time.Now().Add(limit)
				for time.Now().Before(deadline) {
					if value, err := page.Eval(script); err == nil && value == true {
						return true
					}
					time.Sleep(100 * time.Millisecond)
				}
				return false
			}
			capture := func(name string) {
				png, err := window.CapturePage()
				if err != nil {
					results[name+"_capture_error"] = err.Error()
					return
				}
				if err := os.MkdirAll(output, 0700); err != nil {
					results[name+"_capture_error"] = err.Error()
					return
				}
				if err := os.WriteFile(filepath.Join(output, name+".png"), png, 0600); err != nil {
					results[name+"_capture_error"] = err.Error()
				}
			}

			// 快照到达前侧栏显示“正在启动”；等到真实状态再截图。
			bridged := waitFor(`(() => { const s = document.querySelector('.health-button span'); return !!s && s.textContent !== '正在启动' })()`, 12*time.Second)
			results["bridged"] = bridged
			// 引擎先启动、后加入设备网络；网络未就绪时截图会把“未连接电脑”的中间态当成结果。
			results["network_ready"] = waitFor(`(() => {
				const s = window.__deskfuseSnapshot && window.__deskfuseSnapshot()
				return !!s && (!!s.local?.network_id || (s.devices || []).length > 0)
			})()`, 20*time.Second)
			waitFor(`!!document.querySelector('.clipboard-page')`, 5*time.Second)
			// 首屏列表要等真实分页返回；只等页面挂载会把“正在读取…”当成结果。
			started := time.Now()
			results["clipboard_rows"] = waitFor(`(() => {
				const rows = document.querySelectorAll('.results > div').length
				const notice = !!document.querySelector('.notice')
				return rows > 0 || notice
			})()`, 25*time.Second)
			results["clipboard_rows_ms"] = time.Since(started).Milliseconds()
			results["clipboard_dom"] = eval(`JSON.stringify({
				rows: document.querySelectorAll('.results > div').length,
				notice: [...document.querySelectorAll('.notice')].map(e => e.innerText.slice(0,160)),
				host: (document.querySelector('.history')?.innerText || '').slice(0, 240),
				debug: window.__deskfuseClip || null
			})`)
			if queryLibrary != nil {
				now := time.Now()
				request := map[string]any{
					"op": "items", "list_id": nil, "query": "", "kind": "all", "source": nil,
					"limit": 20, "include_total": true, "offset": 0,
					"day_start_ms":       now.UnixMilli() - 86_400_000,
					"yesterday_start_ms": now.UnixMilli() - 172_800_000,
					"day_end_ms":         now.UnixMilli() + 86_400_000,
					"timezone":           "Asia/Shanghai",
					"rule_windows":       map[string]any{},
				}
				if value, err := queryLibrary(request); err == nil {
					results["raw_page"] = truncateJSON(value, 600)
				} else {
					results["raw_page_error"] = err.Error()
				}
			}
			state := eval(`JSON.stringify({title:document.title,clipboard:!!document.querySelector('.clipboard-page'),navigation:[...document.querySelectorAll('.nav-row span:first-of-type')].map(e=>e.textContent),health:document.querySelector('.health-button span')?.textContent,notice:document.querySelector('.service-notice span')?.textContent||'',width:innerWidth,height:innerHeight,scrollWidth:document.documentElement.scrollWidth})`)
			results["clipboard"] = state
			if value, err := snapshot(); err == nil {
				if data, err := json.MarshalIndent(value, "", "  "); err == nil {
					_ = os.MkdirAll(output, 0700)
					_ = os.WriteFile(filepath.Join(output, "snapshot.json"), data, 0600)
					results["snapshot_bytes"] = len(data)
				}
			} else {
				results["snapshot_error"] = err.Error()
			}
			if value, err := invoke("login_item.status", map[string]any{}); err == nil {
				results["login_item_status"] = value
			} else {
				results["login_item_error"] = err.Error()
			}
			capture("clipboard")

			// 逐页切换，只为截图；不点击任何业务按钮。
			pages := []struct{ name, nav string }{
				{"devices", "设备"},
				{"input", "键鼠共享"},
				{"transfers", "局域网传输"},
				{"remote", "远程应用"},
				{"microphone", "麦克风共享"},
				{"mouse", "鼠标自定义"},
			}
			for _, pageInfo := range pages {
				// 顶栏标题随选中项同步变化，早于页面组件加载；以内容变化判定挂载完成。
				before, _ := page.Eval(`document.querySelector('.page-host')?.innerText || ''`)
				previous, _ := json.Marshal(fmt.Sprint(before))
				clicked := eval(fmt.Sprintf(`(() => { const row = [...document.querySelectorAll('.nav-row')].find(e => e.textContent.trim().startsWith(%q)); if (!row) return false; row.click(); return true })()`, pageInfo.nav))
				if clicked != true {
					results[pageInfo.name+"_clicked"] = clicked
					continue
				}
				changed := waitFor(fmt.Sprintf(`(() => (document.querySelector('.page-host')?.innerText || '') !== %s)()`, previous), 8*time.Second)
				if !changed {
					results[pageInfo.name+"_ready"] = false
				}
				time.Sleep(400 * time.Millisecond)
				// 键盘共享需要用户先启用主控；冒烟默认只读，显式要求时才点一次按钮。
				if pageInfo.name == "input" && os.Getenv("DESKFUSE_UI_SMOKE_INPUT_ON") != "" {
					eval(`(() => { const b = [...document.querySelectorAll('button')].find(e => e.textContent.trim() === '在此 Mac 启用主控'); if (b) { b.click(); return true } return false })()`)
					waitFor(`!!document.querySelector('.quality-chip')`, 10*time.Second)
					time.Sleep(1200 * time.Millisecond)
				}
				results[pageInfo.name] = eval(`JSON.stringify({text:(document.querySelector('.page-host')?.innerText||'').slice(0,400),notice:document.querySelector('.service-notice span')?.textContent||''})`)
				capture(pageInfo.name)
			}

			// 鼠标域：中间层组装的视图、G502 位置原语、内置鼠标照片与适配数据探针。
			results["mouse_photo_engine"] = eval(`JSON.stringify({photos:[...document.querySelectorAll('img.photo')].map(i=>({complete:i.complete,width:i.naturalWidth})),hotspots:document.querySelectorAll('.hotspot').length,models:document.querySelectorAll('.supported-row').length})`)
			// 内置鼠标照片走 deskfuse://blob/mouse/*，由中间层缓存；用图片解码验证真实字节。
			eval(`(() => { const img = new Image(); img.onload = () => { window.__mousePhoto = {width: img.naturalWidth, height: img.naturalHeight} }; img.onerror = () => { window.__mousePhoto = {error: true} }; img.src = 'deskfuse://blob/mouse/g502-lightspeed-top'; return true })()`)
			time.Sleep(900 * time.Millisecond)
			results["mouse_photo_blob"] = eval(`JSON.stringify(window.__mousePhoto || null)`)
			if value, err := invoke("mouse.locations", map[string]any{}); err == nil {
				results["mouse_locations"] = truncateJSON(value, 400)
			} else {
				results["mouse_locations_error"] = err.Error()
			}
			if value, err := invoke("mouse.contribute.state", map[string]any{}); err == nil {
				results["mouse_contribution"] = truncateJSON(value, 400)
			} else {
				results["mouse_contribution_error"] = err.Error()
			}
			// 语音测试：页面挂载即 prepare；冒烟不请求麦克风权限，只验证状态机可用。
			if value, err := invoke("microphone.speech.state", map[string]any{}); err == nil {
				results["speech_state"] = truncateJSON(value, 300)
			} else {
				results["speech_state_error"] = err.Error()
			}
			if value, err := invoke("transfers.inspectPaths", map[string]any{"paths": []string{"/tmp"}}); err == nil {
				results["transfer_inspect"] = truncateJSON(value, 300)
			} else {
				results["transfer_inspect_error"] = err.Error()
			}
			if value, err := invoke("uninstall.plan", map[string]any{}); err == nil {
				results["uninstall_plan"] = truncateJSON(value, 400)
			} else {
				results["uninstall_plan_error"] = err.Error()
			}
			results["mouse_snapshot"] = eval(`JSON.stringify((() => { const m = window.__deskfuseSnapshot?.()?.mouse || {}; return {models:(m.models||[]).length,options:(m.options||[]).length,battery:(m.battery||[]).length,wheel:(m.wheel||[]).length,has_g502:!!m.g502,g502_locations:Object.keys(m.g502?.locations||{}).length} })())`)

			// The menu bar panel is a second window; verify it loads the same
			// bundle and renders real rows before hiding it again.
			// 面板是非激活窗口，只有应用在前台时才可能成为 key window；
			// 冒烟从终端启动，先把自己切到前台，才能验证键盘归属。
			mygo.App.Focus()
			time.Sleep(400 * time.Millisecond)
			results["app_active"] = mygo.App.IsActive()
			if panel != nil && panel.window != nil {
				if _, err := invoke("clipboard.showPanel", map[string]any{}); err != nil {
					results["panel_error"] = err.Error()
				} else {
					panelLoaded := false
					deadline := time.Now().Add(8 * time.Second)
					for time.Now().Before(deadline) {
						if value, err := panel.window.Page().Eval(`!!document.querySelector('.panel')`); err == nil && value == true {
							panelLoaded = true
							break
						}
						time.Sleep(100 * time.Millisecond)
					}
					results["panel_loaded"] = panelLoaded
					// 计数按列表逐项查询，记录多时需要一会儿才到齐。
					time.Sleep(2500 * time.Millisecond)
					if png, err := panel.window.CapturePage(); err == nil {
						_ = os.MkdirAll(output, 0700)
						_ = os.WriteFile(filepath.Join(output, "panel.png"), png, 0600)
					} else {
						results["panel_capture_error"] = err.Error()
					}
					results["panel"] = evalPanel(panel)
					results["panel_focused_before_preview"] = panel.window.IsFocused()
					// 瀑布流卡片与列表共用同一批记录；截图确认面板卡片的排版。
					if _, err := panel.window.Page().Eval(`(() => { const b = document.querySelectorAll('.modes button')[1]; if (b) b.click(); return !!b })()`); err == nil {
						gridDeadline := time.Now().Add(4 * time.Second)
						for time.Now().Before(gridDeadline) {
							if value, err := panel.window.Page().Eval(`document.querySelectorAll('.panel-card').length > 0`); err == nil && value == true {
								break
							}
							time.Sleep(100 * time.Millisecond)
						}
						time.Sleep(500 * time.Millisecond)
						results["panel_grid"] = evalPanel(panel)
						if png, err := panel.window.CapturePage(); err == nil {
							_ = os.MkdirAll(output, 0700)
							_ = os.WriteFile(filepath.Join(output, "panel-grid.png"), png, 0600)
						}
						_, _ = panel.window.Page().Eval(`(() => { const b = document.querySelectorAll('.modes button')[0]; if (b) b.click(); return !!b })()`)
						time.Sleep(400 * time.Millisecond)
					}
					// 快速预览：向面板派发空格，选中可预览的行后浮窗应出现在面板左侧。
					previewWindow := openPreviewForSmoke(panel, results)
					if previewWindow != nil {
						time.Sleep(900 * time.Millisecond)
						width, height := previewWindow.Size()
						results["preview_size"] = fmt.Sprintf("%dx%d", width, height)
						results["preview_visible"] = previewWindow.IsVisible()
						results["panel_visible_after_preview"] = panel.window.IsVisible()
						results["panel_focused"] = panel.window.IsFocused()
						if value, err := panel.window.Page().Eval(`document.hasFocus()`); err == nil {
							results["panel_document_focus"] = value
						}
						// 预览打开时方向键仍应落在面板里，并让 popover 跟随。
						before, _ := panel.window.Page().Eval(`document.querySelector('.rows .panel-row[aria-selected=true]')?.dataset.id || ''`)
						_, _ = panel.window.Page().Eval(`window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }))`)
						time.Sleep(700 * time.Millisecond)
						after, _ := panel.window.Page().Eval(`document.querySelector('.rows .panel-row[aria-selected=true]')?.dataset.id || ''`)
						results["preview_follow"] = fmt.Sprintf("%v -> %v", before, after)
						results["preview_visible_after_arrow"] = previewWindow.IsVisible()
						results["preview"] = evalWindow(previewWindow, `JSON.stringify({text:(document.body.innerText||'').slice(0,160),width:innerWidth,height:innerHeight})`)
						if png, err := previewWindow.CapturePage(); err == nil {
							_ = os.MkdirAll(output, 0700)
							_ = os.WriteFile(filepath.Join(output, "preview.png"), png, 0600)
						} else {
							results["preview_capture_error"] = err.Error()
						}
						_, _ = panel.window.Page().Eval(`window.dispatchEvent(new KeyboardEvent('keydown', { key: ' ' }))`)
						closed := false
						for i := 0; i < 20; i++ {
							if !previewWindow.IsVisible() {
								closed = true
								break
							}
							time.Sleep(100 * time.Millisecond)
						}
						results["preview_closed"] = closed
						if value, err := panel.window.Page().Eval(`document.querySelector('.error')?.textContent || ''`); err == nil {
							results["panel_error_after_preview"] = value
						}
						if value, err := panel.window.Page().Eval(`document.querySelector('.panel') ? 'visible' : 'gone'`); err == nil {
							results["panel_dom_after_preview"] = value
						}
					}
					// 图片条目的预览：同样的浮窗，尺寸由图片比例决定。
					if imageWindow := openImagePreviewForSmoke(panel, results); imageWindow != nil {
						width, height := imageWindow.Size()
						results["preview_image_size"] = fmt.Sprintf("%dx%d", width, height)
						if png, err := imageWindow.CapturePage(); err == nil {
							_ = os.MkdirAll(output, 0700)
							_ = os.WriteFile(filepath.Join(output, "preview-image.png"), png, 0600)
						}
						_, _ = panel.window.Page().Eval(`window.dispatchEvent(new KeyboardEvent('keydown', { key: ' ' }))`)
					}
					panel.hide()
				}
			}

			eval(`document.querySelector('.sidebar-footer .nav-row').click(); true`)
			if waitFor(`!!document.querySelector('.settings-page')`, 8*time.Second) {
				time.Sleep(2500 * time.Millisecond)
				results["settings"] = eval(`JSON.stringify({switches:document.querySelectorAll('[role=switch]').length,offline:!!document.querySelector('.offline'),text:(document.querySelector('.settings-page')?.innerText||'').slice(0,300)})`)
				capture("settings")
			} else {
				results["settings_ready"] = false
			}

			// 保存面板完成结果：设置 DESKFUSE_UI_SMOKE_SAVE 后原生保存面板会自动确认，
			// 这里用一条真实文本记录走完整动作，并检查写入的文件。
			if os.Getenv("DESKFUSE_UI_SMOKE_SAVE") == "1" {
				if value, err := snapshot(); err != nil {
					results["save_smoke_error"] = err.Error()
				} else if object, ok := value.(map[string]any); ok {
					if clipboard, ok := object["clipboard"].(map[string]any); ok {
						if history, ok := clipboard["history"].([]any); ok {
							for _, raw := range history {
								item, ok := raw.(map[string]any)
								if !ok {
									continue
								}
								hasImage, _ := item["has_image"].(bool)
								fileCount := 0
								switch v := item["file_count"].(type) {
								case float64:
									fileCount = int(v)
								case int:
									fileCount = v
								}
								if hasImage || fileCount > 0 {
									continue
								}
								id := fmt.Sprint(item["id"])
								outcome, err := invoke("clipboard.save", map[string]any{"id": id, "has_image": false, "file_count": 0})
								if err != nil {
									results["save_smoke_error"] = err.Error()
									break
								}
								results["save_smoke"] = outcome
								if envelope, ok := outcome.(map[string]any); ok {
									if saved, ok := envelope["value"].(map[string]any); ok {
										results["save_smoke_cancelled"] = saved["saved"] == false
									}
								}
								break
							}
						}
					}
				}
			}

			// 轻量动作探针：确认新桥接动作存在并能返回原生错误，
			// 不触发保存面板、编辑窗口或任何破坏性操作。
			if os.Getenv("DESKFUSE_UI_SMOKE_ACTIONS") != "" {
				probe := func(name, action string, params map[string]any) any {
					if _, err := invoke(action, params); err != nil {
						return err.Error()
					}
					return "ok"
				}
				actions := map[string]any{
					"clipboard.sources":            probe("sources", "clipboard.sources", map[string]any{}),
					"clipboard.save_guard":         probe("save", "clipboard.save", map[string]any{}),
					"clipboard.edit_guard":         probe("edit", "clipboard.edit", map[string]any{}),
					"clipboard.share_guard":        probe("share", "clipboard.share", map[string]any{}),
					"workflow.run_guard":           probe("workflow", "workflow.run", map[string]any{}),
					"clipboard.indexOCR_guard":     probe("ocr", "clipboard.indexOCR", map[string]any{}),
					"clipboard.cancel_guard":       probe("cancel", "clipboard.cancelPreparation", map[string]any{}),
					"devices.leaveNetwork_guard":   probe("leave", "devices.leaveNetwork", map[string]any{}),
					"remoteApps.openPicker_guard":  probe("picker", "remoteApps.openPicker", map[string]any{}),
					"input.chooseController_guard": probe("choose", "input.chooseController", map[string]any{}),
					"input.setParticipant_guard":   probe("participant", "input.setParticipant", map[string]any{}),
					"panel.hide":                   probe("panel", "panel.hide", map[string]any{}),
					"uninstall.plan":               probe("uninstall", "uninstall.plan", map[string]any{}),
				}
				// 拖拽吸附由引擎计算：用真实显示的坐标走一次，返回值必须是两个有限数。
				if value, err := snapshot(); err == nil {
					if object, ok := value.(map[string]any); ok {
						if input, ok := object["input"].(map[string]any); ok {
							if displays, ok := input["displays"].([]any); ok && len(displays) > 0 {
								rows := make([]map[string]any, 0, len(displays))
								device := ""
								for _, raw := range displays {
									display, ok := raw.(map[string]any)
									if !ok {
										continue
									}
									if device == "" {
										device = fmt.Sprint(display["device_id"])
									}
									rows = append(rows, map[string]any{
										"id": display["id"], "device_id": display["device_id"], "name": display["name"],
										"x": display["x"], "y": display["y"], "width": display["width"], "height": display["height"],
										"local": display["local"], "ready": display["ready"],
									})
								}
								if device != "" && len(rows) > 0 {
									if outcome, err := invoke("input.previewDisplayMove", map[string]any{
										"displays": rows, "device_id": device, "delta_x": 10.0, "delta_y": 10.0,
										"snap_distance": 14.0, "attach": true,
									}); err != nil {
										actions["input.previewDisplayMove"] = err.Error()
									} else {
										actions["input.previewDisplayMove"] = outcome
									}
								}
							}
						}
					}
				}
				// 远程应用目录：对第一台在线的非 Windows 设备读取真实目录，
				// 记录图标、运行中与分类字段，然后关闭目录连接。
				if value, err := snapshot(); err == nil {
					if object, ok := value.(map[string]any); ok {
						if devices, ok := object["devices"].([]any); ok {
							for _, raw := range devices {
								device, ok := raw.(map[string]any)
								if !ok || device["connected"] != true || device["trusted"] != true || device["remote_app_rdp"] == true {
									continue
								}
								peer := fmt.Sprint(device["id"])
								catalog, err := invoke("remoteApps.catalog", map[string]any{"device_id": peer})
								if err != nil {
									actions["remoteApps.catalog"] = err.Error()
								} else if payload, ok := catalog.(map[string]any); ok {
									apps, _ := payload["applications"].([]any)
									withIcon, running, opened := 0, 0, 0
									for _, entry := range apps {
										app, ok := entry.(map[string]any)
										if !ok {
											continue
										}
										if icon, ok := app["icon"].(string); ok && icon != "" {
											withIcon++
										}
										if app["running"] == true {
											running++
										}
										if app["opened"] == true {
											opened++
										}
									}
									actions["remoteApps.catalog"] = fmt.Sprintf("apps=%d icons=%d running=%d opened=%d rdp=%v", len(apps), withIcon, running, opened, payload["rdp"])
								}
								if outcome, err := invoke("remoteApps.setVirtualDisplay", map[string]any{"enabled": true}); err != nil {
									actions["remoteApps.setVirtualDisplay"] = err.Error()
								} else {
									actions["remoteApps.setVirtualDisplay"] = outcome
								}
								_, _ = invoke("remoteApps.setVirtualDisplay", map[string]any{"enabled": false})
								_, _ = invoke("remoteApps.closeCatalog", map[string]any{"device_id": peer})
								break
							}
						}
					}
				}
				results["actions"] = actions
			}

			// 浮动窗口（配对、传输进度）各自加载独立页面。这里验证页面能渲染；
			// 窗口何时显示由原生事件驱动，另由 Go 测试覆盖。
			for _, extra := range []struct {
				page, selector string
				width, height  int
			}{
				{"pairing", ".pairing .titlebar h1", 480, 320},
				{"transfer-progress", ".stack", 316, 100},
			} {
				window := mygo.NewWindow(mygo.WindowOptions{
					Title: extra.page, URL: "/?page=" + extra.page,
					Width: extra.width, Height: extra.height, Frameless: true, Panel: true,
				})
				loaded := false
				deadline := time.Now().Add(8 * time.Second)
				for time.Now().Before(deadline) {
					if value, err := window.Page().Eval("!!document.querySelector(" + strconv.Quote(extra.selector) + ")"); err == nil && value == true {
						loaded = true
						break
					}
					time.Sleep(100 * time.Millisecond)
				}
				results[extra.page+"_loaded"] = loaded
				time.Sleep(400 * time.Millisecond)
				if png, err := window.CapturePage(); err == nil {
					_ = os.MkdirAll(output, 0700)
					_ = os.WriteFile(filepath.Join(output, extra.page+".png"), png, 0600)
				} else {
					results[extra.page+"_capture_error"] = err.Error()
				}
				results[extra.page] = evalWindow(window, `JSON.stringify({text:(document.body.innerText||'').slice(0,160),width:innerWidth,height:innerHeight})`)
				window.Close()
			}
		}()
	})
}

// truncateJSON keeps a diagnostic snapshot readable in smoke.json.
func truncateJSON(value any, limit int) string {
	data, err := json.Marshal(value)
	if err != nil {
		return "marshal error: " + err.Error()
	}
	text := string(data)
	if len(text) > limit {
		return text[:limit] + "…"
	}
	return text
}

// openPreviewForSmoke selects the first previewable record and presses space,
// then returns the preview flyout once it shows.
func openPreviewForSmoke(panel *panelWindow, results map[string]any) *mygo.Window {
	page := panel.window.Page()
	rows, err := page.Eval(`Array.from(document.querySelectorAll('[data-id]')).slice(0, 8).map(e => e.dataset.id)`)
	if err != nil {
		results["preview_rows_error"] = err.Error()
		return nil
	}
	ids, _ := rows.([]any)
	for _, raw := range append([]any{nil}, ids...) {
		if raw != nil {
			_, _ = page.Eval(fmt.Sprintf(`(() => { const el = document.querySelector('[data-id=%s]'); if (el) el.click(); return !!el })()`, strconv.Quote(fmt.Sprint(raw))))
		}
		_, _ = page.Eval(`window.dispatchEvent(new KeyboardEvent('keydown', { key: ' ' }))`)
		deadline := time.Now().Add(2500 * time.Millisecond)
		for time.Now().Before(deadline) {
			for _, window := range mygo.Windows() {
				if window == panel.window || !window.IsFlyout() || window.IsDestroyed() {
					continue
				}
				if window.IsVisible() {
					return window
				}
			}
			time.Sleep(100 * time.Millisecond)
		}
		// 空格可能刚把上一个浮窗关掉；确保关闭后再试下一行。
		_, _ = page.Eval(`window.dispatchEvent(new KeyboardEvent('keydown', { key: ' ' }))`)
		time.Sleep(150 * time.Millisecond)
	}
	results["preview_open"] = false
	return nil
}

// openImagePreviewForSmoke picks rows until the preview shows an image.
func openImagePreviewForSmoke(panel *panelWindow, results map[string]any) *mygo.Window {
	page := panel.window.Page()
	// 切到「图片」智能列表，确保前几行都是图片记录。
	_, _ = page.Eval(`(() => { const b = [...document.querySelectorAll('.tabs button')].find(e => e.textContent.trim().startsWith('图片')); if (b) b.click(); return !!b })()`)
	time.Sleep(1200 * time.Millisecond)
	defer func() {
		_, _ = page.Eval(`(() => { const b = [...document.querySelectorAll('.tabs button')].find(e => e.textContent.trim().startsWith('全部')); if (b) b.click(); return !!b })()`)
		time.Sleep(400 * time.Millisecond)
	}()
	rows, err := page.Eval(`Array.from(document.querySelectorAll('.rows [data-id]')).map(e => e.dataset.id).slice(0, 8)`)
	if err != nil {
		results["preview_image_error"] = err.Error()
		return nil
	}
	ids, _ := rows.([]any)
	for _, raw := range ids {
		id := fmt.Sprint(raw)
		_, _ = page.Eval(fmt.Sprintf(`(() => { const el = document.querySelector('[data-id=%s]'); if (el) el.click(); return !!el })()`, strconv.Quote(id)))
		_, _ = page.Eval(`window.dispatchEvent(new KeyboardEvent('keydown', { key: ' ' }))`)
		var preview *mygo.Window
		deadline := time.Now().Add(2 * time.Second)
		for time.Now().Before(deadline) {
			for _, window := range mygo.Windows() {
				if window != panel.window && window.IsFlyout() && !window.IsDestroyed() && window.IsVisible() {
					preview = window
					break
				}
			}
			if preview != nil {
				break
			}
			time.Sleep(100 * time.Millisecond)
		}
		if preview == nil {
			continue
		}
		loaded := false
		imageDeadline := time.Now().Add(6000 * time.Millisecond)
		for time.Now().Before(imageDeadline) {
			if value, err := preview.Page().Eval(`(() => { const img = document.querySelector('.image'); return !!img && img.complete && img.naturalWidth > 0 })()`); err == nil && value == true {
				loaded = true
				break
			}
			time.Sleep(100 * time.Millisecond)
		}
		if loaded {
			return preview
		}
		if _, seen := results["preview_image_debug"]; !seen {
			results["preview_image_debug"] = evalWindow(preview, `JSON.stringify({text:(document.body.innerText||'').slice(0,120),html:(document.querySelector('.quick-look')?.innerHTML||'').slice(0,240)})`)
		}
		_, _ = page.Eval(`window.dispatchEvent(new KeyboardEvent('keydown', { key: ' ' }))`)
		time.Sleep(200 * time.Millisecond)
	}
	results["preview_image"] = false
	return nil
}

// evalPanel reads the panel window's own document, not the main window's.
func evalPanel(panel *panelWindow) any {
	if panel == nil || panel.window == nil {
		return nil
	}
	value, err := panel.window.Page().Eval(`JSON.stringify({
		rows: document.querySelectorAll('.rows [data-id]').length,
		cards: document.querySelectorAll('.panel-card').length,
		state: document.querySelector('.state')?.textContent || '',
		notice: document.querySelector('.notice')?.textContent || '',
		tabs: [...document.querySelectorAll('.tabs button')].map(e => e.textContent.trim()),
		footer: (document.querySelector('.foot')?.innerText || '').slice(0, 160),
		source: document.querySelector('.source-name')?.textContent || ''
	})`)
	if err != nil {
		return err.Error()
	}
	return value
}

// evalWindow reads a secondary window's document, not the main window's.
func evalWindow(window *mygo.Window, script string) any {
	if window == nil || window.Page() == nil {
		return nil
	}
	value, err := window.Page().Eval(script)
	if err != nil {
		return err.Error()
	}
	return value
}
