package ui

import (
	"context"
	"testing"
	"time"

	myui "github.com/egoist/mygo/ui"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/clipboard"
)

type viewSource struct {
	page clipboard.Page
}

func (s viewSource) Query(context.Context, clipboard.Query) (clipboard.Page, error) {
	return s.page, nil
}

func loadedClipboardStore(t *testing.T) *clipboard.Store {
	t.Helper()
	store := clipboard.NewStore(viewSource{page: clipboard.Page{
		Items: []clipboard.Item{
			{ID: 1, Preview: "first preview", Source: "Safari"},
			{ID: 2, Preview: "second preview", Source: "Notes"},
		},
	}})
	t.Cleanup(store.Close)
	store.EnsureLoaded()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if store.Snapshot().State == clipboard.StateLoaded {
			return store
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatal("clipboard store did not load")
	return nil
}

func TestClipboardViewRendersLoadedPageAndSelectsRows(t *testing.T) {
	store := loadedClipboardStore(t)
	tt := myui.NewTester(func(c *myui.Context) {
		renderClipboardPage(c, store, c.Theme(), &viewState{})
	}, 1120, 720)

	for _, text := range []string{"历史记录", "搜索", "刷新", "其他", "first preview", "预览片段", "来自 Safari"} {
		if !tt.HasText(text) {
			t.Errorf("loaded page does not show %q; texts=%v", text, tt.Texts())
		}
	}

	if err := tt.Click("second preview"); err != nil {
		t.Fatalf("click second row: %v", err)
	}
	tt.Frame()
	if selected := store.Snapshot().SelectedID; selected != 2 {
		t.Fatalf("selected = %d, want 2", selected)
	}
}

func TestClipboardViewRendersUnavailableState(t *testing.T) {
	store := clipboard.NewStore(nil)
	t.Cleanup(store.Close)
	tt := myui.NewTester(func(c *myui.Context) {
		renderClipboardPage(c, store, c.Theme(), &viewState{})
	}, 1120, 720)

	for _, text := range []string{"剪贴板桥接未启用", "剪贴板页面不可用", "不会显示模拟数据"} {
		if !tt.HasText(text) {
			t.Errorf("unavailable page does not show %q; texts=%v", text, tt.Texts())
		}
	}
}

type searchViewSource struct{}

func (searchViewSource) Query(_ context.Context, request clipboard.Query) (clipboard.Page, error) {
	if request.Search == "needle" {
		return clipboard.Page{Items: []clipboard.Item{{ID: 2, Preview: "matched row"}}}, nil
	}
	return clipboard.Page{Items: []clipboard.Item{{ID: 1, Preview: "initial row"}}}, nil
}

func TestClipboardViewSearchWiresStore(t *testing.T) {
	store := clipboard.NewStore(searchViewSource{})
	t.Cleanup(store.Close)
	store.EnsureLoaded()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) && store.Snapshot().State != clipboard.StateLoaded {
		time.Sleep(5 * time.Millisecond)
	}
	if store.Snapshot().State != clipboard.StateLoaded {
		t.Fatal("clipboard store did not load")
	}

	tt := myui.NewTester(func(c *myui.Context) {
		renderClipboardPage(c, store, c.Theme(), &viewState{})
	}, 1120, 720)
	if err := tt.Click("搜索剪贴板"); err != nil {
		t.Fatalf("focus search field: %v", err)
	}
	tt.Type("needle")

	deadline = time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		snapshot := store.Snapshot()
		if snapshot.Search == "needle" && snapshot.State == clipboard.StateLoaded &&
			len(snapshot.Items) == 1 && snapshot.Items[0].ID == 2 {
			tt.Frame()
			if !tt.HasText("matched row") {
				t.Fatalf("search result row not rendered; texts=%v", tt.Texts())
			}
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("search did not reach the store; snapshot=%+v", store.Snapshot())
}
