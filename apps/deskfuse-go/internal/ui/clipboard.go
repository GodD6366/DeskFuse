package ui

import (
	"fmt"
	"strings"

	myui "github.com/egoist/mygo/ui"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/clipboard"
)

const clipboardListWidth float32 = 320

// viewState is per-window UI state that is not business state. Search text is
// kept here so the MyGo search field can bind to a stable pointer across
// frames while the clipboard store owns the actual query.
type viewState struct {
	clipboardSearch string
}

func renderClipboardPage(c *myui.Context, store *clipboard.Store, theme *myui.Theme, state *viewState) {
	store.EnsureLoaded()
	snapshot := store.Snapshot()
	searchChanged := false

	myui.Row(c).Fill().Children(func() {
		renderClipboardHistory(c, store, snapshot, theme, state, &searchChanged)
		myui.Box(c).Width(1).FillHeight().Background(theme.Border)
		renderClipboardPreview(c, snapshot, theme)
	})

	if searchChanged {
		store.SetSearch(state.clipboardSearch)
		c.Invalidate()
	}
}

func renderClipboardHistory(c *myui.Context, store *clipboard.Store, snapshot clipboard.Snapshot, theme *myui.Theme, state *viewState, searchChanged *bool) {
	myui.Column(c).
		Width(clipboardListWidth).
		FillHeight().
		Background(theme.Surface).
		Padding(theme.Space(3)).
		Gap(theme.Space(2)).
		Children(func() {
			myui.Row(c).FillWidth().Children(func() {
				myui.Text(c, "历史记录").FontSize(16).Bold()
				myui.Spacer(c)
				if snapshot.State != clipboard.StateUnavailable {
					if myui.Button(c.Key("clipboard.refresh"), "刷新").
						Disabled(snapshot.State == clipboard.StateLoading).
						Clicked() {
						store.Retry()
						c.Invalidate()
					}
				}
				myui.Textf(c, "已加载 %d 条", len(snapshot.Items)).FontSize(11).TextColor(theme.TextMuted)
			})

			if snapshot.State != clipboard.StateUnavailable {
				if myui.SearchField(c, &state.clipboardSearch).Label("搜索剪贴板").FillWidth().Changed() {
					*searchChanged = true
				}
			}

			switch snapshot.State {
			case clipboard.StateUnavailable:
				renderClipboardEmptyState(c, theme,
					"剪贴板桥接未启用",
					"当前 Go UI 未连接原生剪贴板引擎，因此不会显示模拟数据。\n\n以 DESKFUSE_MYGO_BRIDGE=stdio-v1 启动并由原生宿主提供 stdio 桥后可用。",
					"", nil)
			case clipboard.StateLoading:
				renderClipboardLoading(c, theme, "正在读取剪贴板历史…")
			case clipboard.StateError:
				renderClipboardEmptyState(c, theme,
					"无法读取剪贴板历史", snapshot.Error,
					"重试", func() {
						store.Retry()
						c.Invalidate()
					})
			case clipboard.StateEmpty:
				title, message := clipboardEmptyCopy(snapshot.Search)
				actionTitle := "重新加载"
				action := func() {
					store.Retry()
					c.Invalidate()
				}
				if strings.TrimSpace(snapshot.Search) != "" {
					actionTitle = "清除搜索"
					action = func() {
						state.clipboardSearch = ""
						store.SetSearch("")
						c.Invalidate()
					}
				}
				renderClipboardEmptyState(c, theme, title, message, actionTitle, action)
			case clipboard.StateLoaded:
				renderClipboardList(c, store, snapshot, theme)
			default:
				renderClipboardLoading(c, theme, "正在准备剪贴板页面…")
			}
		})
}

func renderClipboardLoading(c *myui.Context, theme *myui.Theme, message string) {
	myui.Column(c).Fill().Center().Gap(theme.Space(2)).Children(func() {
		myui.Spinner(c)
		myui.Text(c, message).FontSize(12).TextColor(theme.TextMuted)
	})
}

func renderClipboardEmptyState(c *myui.Context, theme *myui.Theme, title, message, actionTitle string, action func()) {
	myui.Column(c).Fill().Center().Gap(theme.Space(2)).Children(func() {
		myui.Text(c, title).FontSize(15).Bold()
		if message != "" {
			myui.Text(c, message).FontSize(12).TextColor(theme.TextMuted).MaxWidth(280)
		}
		if actionTitle != "" && action != nil {
			if myui.PrimaryButton(c.Key("clipboard.action."+actionTitle), actionTitle).Clicked() {
				action()
			}
		}
	})
}

func renderClipboardList(c *myui.Context, store *clipboard.Store, snapshot clipboard.Snapshot, theme *myui.Theme) {
	myui.Scroll(c).Grow(1).FillWidth().Gap(theme.Space(1)).Children(func() {
		day := ""
		for _, item := range snapshot.Items {
			if nextDay := clipboardDay(item); nextDay != day {
				myui.Text(c, nextDay).FontSize(11).TextColor(theme.TextMuted).Padding(theme.Space(1))
				day = nextDay
			}
			renderClipboardRow(c, store, item, item.ID == snapshot.SelectedID, theme)
		}
		renderClipboardFooter(c, store, snapshot, theme)
	})
}

func renderClipboardRow(c *myui.Context, store *clipboard.Store, item clipboard.Item, selected bool, theme *myui.Theme) {
	background := myui.Transparent
	if selected {
		background = theme.Selection
	}
	row := myui.ButtonBase(c.Key(fmt.Sprintf("clipboard.item.%d", item.ID))).
		FillWidth().
		Padding(theme.Space(2)).
		Radius(theme.Radius).
		Background(background).
		Label(clipboardTitle(item))
	row.Children(func() {
		myui.Column(c).FillWidth().Gap(2).Children(func() {
			myui.Text(c, clipboardTitle(item)).FontSize(13).Bold().MaxLines(2).Ellipsis("…")
			myui.Row(c).FillWidth().Gap(theme.Space(1)).Children(func() {
				myui.Text(c, "来自 "+clipboardSource(item)).
					FontSize(11).
					TextColor(theme.TextMuted).
					SingleLine().
					Ellipsis("…")
				myui.Spacer(c)
				myui.Text(c, clipboardTime(item)).FontSize(11).TextColor(theme.TextMuted)
			})
		})
	})
	if row.Clicked() {
		store.Select(item.ID)
		c.Invalidate()
	}
}

func renderClipboardFooter(c *myui.Context, store *clipboard.Store, snapshot clipboard.Snapshot, theme *myui.Theme) {
	if snapshot.LoadingMore {
		myui.Row(c).FillWidth().Gap(theme.Space(1)).Padding(theme.Space(2)).Children(func() {
			myui.Spinner(c)
			myui.Text(c, "正在加载更多…").FontSize(11).TextColor(theme.TextMuted)
		})
		return
	}
	if snapshot.LoadMoreError != "" {
		myui.Column(c).FillWidth().Gap(theme.Space(1)).Padding(theme.Space(2)).Children(func() {
			myui.Text(c, snapshot.LoadMoreError).FontSize(11).TextColor(theme.Danger).MaxWidth(280)
			if myui.Button(c.Key("clipboard.loadmore.retry"), "重试加载更多").Clicked() {
				store.LoadMore()
				c.Invalidate()
			}
		})
		return
	}
	if snapshot.HasMore {
		if myui.Button(c.Key("clipboard.loadmore"), "加载更多").FillWidth().Clicked() {
			store.LoadMore()
			c.Invalidate()
		}
		return
	}
	myui.Text(c, "已到列表末尾").FontSize(11).TextColor(theme.TextMuted).Padding(theme.Space(2))
}

func renderClipboardPreview(c *myui.Context, snapshot clipboard.Snapshot, theme *myui.Theme) {
	myui.Scroll(c).
		Fill().
		Padding(theme.Space(6)).
		Gap(theme.Space(3)).
		Children(func() {
			if snapshot.State == clipboard.StateUnavailable {
				myui.Text(c, "剪贴板页面不可用").FontSize(18).Bold()
				myui.Text(c, "未配置 DESKFUSE_MYGO_BRIDGE=stdio-v1，或原生桥未启动。").
					FontSize(12).
					TextColor(theme.TextMuted).
					MaxWidth(520)
				return
			}

			item, ok := snapshot.SelectedItem()
			if !ok {
				switch snapshot.State {
				case clipboard.StateError:
					myui.Text(c, "无法读取剪贴板历史").FontSize(18).Bold()
					myui.Text(c, snapshot.Error).FontSize(12).TextColor(theme.Danger).MaxWidth(520)
				case clipboard.StateLoading:
					myui.Text(c, "正在读取剪贴板历史…").FontSize(15).Bold()
				default:
					myui.Text(c, "选择一条历史查看预览").FontSize(15).Bold()
					myui.Text(c, "本页为只读；不会写入系统剪贴板。").FontSize(12).TextColor(theme.TextMuted)
				}
				return
			}

			myui.Text(c, clipboardHeading(item)).FontSize(22).Bold().MaxLines(2).Ellipsis("…")
			myui.Textf(c, "来自 %s · %s · %s", clipboardSource(item), clipboardKind(item), clipboardTime(item)).
				FontSize(12).
				TextColor(theme.TextMuted)
			myui.Divider(c)
			myui.Text(c, "预览片段").FontSize(11).TextColor(theme.TextMuted)
			preview := item.Preview
			if strings.TrimSpace(preview) == "" {
				preview = "没有可显示的文本预览。"
			}
			myui.Text(c, preview).FontSize(15).Selectable().MaxWidth(680)

			if ocr := strings.TrimSpace(item.OCRText); ocr != "" {
				myui.Text(c, "识别文字").FontSize(11).TextColor(theme.TextMuted)
				myui.Text(c, ocr).FontSize(13).Selectable().MaxWidth(680)
			}
			if note := clipboardPreviewNote(item); note != "" {
				myui.Text(c, note).FontSize(11).TextColor(theme.TextMuted).MaxWidth(520)
			}
			if item.Remote && item.Available != nil && !*item.Available {
				myui.Text(c, "这是远端索引记录；本地内容不可用。").
					FontSize(11).
					TextColor(theme.Warning).
					MaxWidth(520)
			}
		})
}

func clipboardEmptyCopy(search string) (string, string) {
	trimmed := strings.TrimSpace(search)
	if trimmed != "" {
		return "没有匹配的记录", fmt.Sprintf("没有找到包含“%s”的剪贴板记录。", truncateRunes(trimmed, 40))
	}
	return "剪贴板历史为空", "在任意应用里复制内容后，这里会显示真实历史记录。"
}
