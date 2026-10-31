package clipboard

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

type fakeSource struct {
	mu      sync.Mutex
	calls   []Query
	respond func(Query) (Page, error)
}

func (f *fakeSource) Query(_ context.Context, request Query) (Page, error) {
	f.mu.Lock()
	f.calls = append(f.calls, request)
	f.mu.Unlock()
	return f.respond(request)
}

func (f *fakeSource) recorded() []Query {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]Query(nil), f.calls...)
}

func item(id uint64) Item {
	return Item{ID: id, Preview: fmt.Sprintf("item %d", id)}
}

func waitFor(t *testing.T, what string, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if condition() {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", what)
}

func TestStoreWithoutSourceIsUnavailable(t *testing.T) {
	store := NewStore(nil)
	t.Cleanup(store.Close)

	store.EnsureLoaded()
	store.SetSearch("anything")

	snapshot := store.Snapshot()
	if snapshot.State != StateUnavailable {
		t.Fatalf("state = %q, want %q", snapshot.State, StateUnavailable)
	}
	if len(snapshot.Items) != 0 {
		t.Fatalf("unavailable store returned items: %+v", snapshot.Items)
	}
}

func TestStoreLoadsSearchPageAndSelectsFirst(t *testing.T) {
	source := &fakeSource{respond: func(Query) (Page, error) {
		return Page{Items: []Item{item(7), item(8)}}, nil
	}}
	store := NewStore(source)
	store.debounce = 0
	store.timeout = time.Second
	t.Cleanup(store.Close)

	store.SetSearch("hello")
	waitFor(t, "loaded search page", func() bool {
		snapshot := store.Snapshot()
		return snapshot.State == StateLoaded && len(snapshot.Items) == 2
	})

	snapshot := store.Snapshot()
	if snapshot.SelectedID != 7 {
		t.Fatalf("selected = %d, want first item 7", snapshot.SelectedID)
	}
	if snapshot.Search != "hello" {
		t.Fatalf("search = %q, want hello", snapshot.Search)
	}

	queries := source.recorded()
	last := queries[len(queries)-1]
	if last.Op != "items" || last.Search != "hello" || last.Limit != PageSize || last.IncludeTotal {
		t.Fatalf("unexpected query: %+v", last)
	}
	if last.ListID != nil || last.Source != nil {
		t.Fatalf("read-only all-history query must keep list_id/source null: %+v", last)
	}
}

func TestStoreEmptyAndErrorStates(t *testing.T) {
	empty := NewStore(&fakeSource{respond: func(Query) (Page, error) { return Page{}, nil }})
	empty.debounce = 0
	t.Cleanup(empty.Close)
	empty.EnsureLoaded()
	waitFor(t, "empty state", func() bool { return empty.Snapshot().State == StateEmpty })

	failing := NewStore(&fakeSource{respond: func(Query) (Page, error) {
		return Page{}, errors.New("engine unavailable")
	}})
	failing.debounce = 0
	t.Cleanup(failing.Close)
	failing.EnsureLoaded()
	waitFor(t, "error state", func() bool { return failing.Snapshot().State == StateError })
	if got := failing.Snapshot().Error; !strings.Contains(got, "engine unavailable") {
		t.Fatalf("error = %q, want the engine message", got)
	}
}

func TestStaleSearchResultIsDiscarded(t *testing.T) {
	slowGate := make(chan struct{})
	slowReturned := make(chan struct{})
	source := &fakeSource{}
	source.respond = func(request Query) (Page, error) {
		switch request.Search {
		case "slow":
			<-slowGate
			close(slowReturned)
			return Page{Items: []Item{item(1)}}, nil
		case "fast":
			return Page{Items: []Item{item(2)}}, nil
		default:
			return Page{}, nil
		}
	}
	store := NewStore(source)
	store.debounce = 0
	store.timeout = 2 * time.Second
	t.Cleanup(store.Close)

	store.SetSearch("slow")
	waitFor(t, "slow query to start", func() bool { return len(source.recorded()) == 1 })

	store.SetSearch("fast")
	waitFor(t, "fast result", func() bool {
		snapshot := store.Snapshot()
		return snapshot.State == StateLoaded && len(snapshot.Items) == 1 && snapshot.Items[0].ID == 2
	})

	close(slowGate)
	select {
	case <-slowReturned:
	case <-time.After(2 * time.Second):
		t.Fatal("slow query did not return")
	}
	// The slow source ignores cancellation; the generation guard must still
	// reject its late page.
	time.Sleep(20 * time.Millisecond)

	snapshot := store.Snapshot()
	if len(snapshot.Items) != 1 || snapshot.Items[0].ID != 2 {
		t.Fatalf("stale search replaced newer results: %+v", snapshot.Items)
	}
}

func TestCursorPaginationDeduplicatesAndIgnoresConcurrentLoad(t *testing.T) {
	appendGate := make(chan struct{})
	var appendCalls atomic.Int32
	source := &fakeSource{}
	source.respond = func(request Query) (Page, error) {
		if request.Cursor == nil {
			return Page{
				Items:      []Item{item(3), item(2)},
				NextOffset: intPointer(20),
				NextCursor: &Cursor{SortAtMS: 100, ID: 2},
			}, nil
		}
		appendCalls.Add(1)
		<-appendGate
		return Page{Items: []Item{item(2), item(1)}}, nil
	}
	store := NewStore(source)
	store.debounce = 0
	store.timeout = 2 * time.Second
	t.Cleanup(store.Close)

	store.EnsureLoaded()
	waitFor(t, "first page", func() bool {
		snapshot := store.Snapshot()
		return snapshot.State == StateLoaded && len(snapshot.Items) == 2 && snapshot.HasMore
	})

	store.LoadMore()
	store.LoadMore()
	waitFor(t, "one append query", func() bool { return appendCalls.Load() == 1 })
	time.Sleep(20 * time.Millisecond)
	if calls := appendCalls.Load(); calls != 1 {
		t.Fatalf("concurrent LoadMore issued %d append queries, want 1", calls)
	}

	close(appendGate)
	waitFor(t, "appended page", func() bool {
		snapshot := store.Snapshot()
		return len(snapshot.Items) == 3 && !snapshot.LoadingMore
	})

	snapshot := store.Snapshot()
	wantIDs := []uint64{3, 2, 1}
	for index, want := range wantIDs {
		if snapshot.Items[index].ID != want {
			t.Fatalf("item %d = %d, want %d", index, snapshot.Items[index].ID, want)
		}
	}
	if snapshot.HasMore {
		t.Fatal("HasMore = true after a page without a next cursor")
	}

	queries := source.recorded()
	var appendQuery *Query
	for index := range queries {
		if queries[index].Cursor != nil {
			appendQuery = &queries[index]
		}
	}
	if appendQuery == nil {
		t.Fatal("no cursor query recorded")
	}
	if appendQuery.Cursor.ID != 2 || appendQuery.Cursor.SortAtMS != 100 || appendQuery.Offset != 20 {
		t.Fatalf("append query used the wrong cursor: %+v", *appendQuery)
	}
}

func TestStalePaginationIsDiscardedAfterSearch(t *testing.T) {
	appendGate := make(chan struct{})
	appendReturned := make(chan struct{})
	source := &fakeSource{}
	source.respond = func(request Query) (Page, error) {
		switch {
		case request.Search == "new":
			return Page{Items: []Item{item(9)}}, nil
		case request.Cursor == nil:
			return Page{
				Items:      []Item{item(1), item(2)},
				NextOffset: intPointer(20),
				NextCursor: &Cursor{SortAtMS: 100, ID: 2},
			}, nil
		default:
			<-appendGate
			close(appendReturned)
			return Page{Items: []Item{item(99)}}, nil
		}
	}
	store := NewStore(source)
	store.debounce = 0
	store.timeout = 2 * time.Second
	t.Cleanup(store.Close)

	store.EnsureLoaded()
	waitFor(t, "first page", func() bool {
		return store.Snapshot().State == StateLoaded
	})
	store.LoadMore()
	waitFor(t, "stale append to start", func() bool {
		for _, query := range source.recorded() {
			if query.Cursor != nil {
				return true
			}
		}
		return false
	})

	store.SetSearch("new")
	waitFor(t, "new search result", func() bool {
		snapshot := store.Snapshot()
		return snapshot.State == StateLoaded && len(snapshot.Items) == 1 && snapshot.Items[0].ID == 9
	})

	close(appendGate)
	select {
	case <-appendReturned:
	case <-time.After(2 * time.Second):
		t.Fatal("stale append did not return")
	}
	time.Sleep(20 * time.Millisecond)

	snapshot := store.Snapshot()
	if len(snapshot.Items) != 1 || snapshot.Items[0].ID != 9 {
		t.Fatalf("stale pagination leaked into the new search: %+v", snapshot.Items)
	}
	if snapshot.HasMore {
		t.Fatal("stale pagination changed HasMore for the new search")
	}
}

func TestLoadMoreErrorKeepsPageAndRetrySucceeds(t *testing.T) {
	var appendAttempts atomic.Int32
	source := &fakeSource{}
	source.respond = func(request Query) (Page, error) {
		if request.Cursor == nil {
			return Page{
				Items:      []Item{item(3), item(2)},
				NextOffset: intPointer(20),
				NextCursor: &Cursor{SortAtMS: 100, ID: 2},
			}, nil
		}
		if appendAttempts.Add(1) == 1 {
			return Page{}, errors.New("temporary bridge failure")
		}
		return Page{Items: []Item{item(1)}}, nil
	}
	store := NewStore(source)
	store.debounce = 0
	store.timeout = time.Second
	t.Cleanup(store.Close)

	store.EnsureLoaded()
	waitFor(t, "first page", func() bool {
		return store.Snapshot().State == StateLoaded
	})

	store.LoadMore()
	waitFor(t, "load-more error", func() bool {
		snapshot := store.Snapshot()
		return !snapshot.LoadingMore && snapshot.LoadMoreError != ""
	})
	snapshot := store.Snapshot()
	if len(snapshot.Items) != 2 {
		t.Fatalf("load-more error dropped the loaded page: %+v", snapshot.Items)
	}
	if !snapshot.HasMore {
		t.Fatal("load-more error cleared HasMore; retry would be impossible")
	}

	store.LoadMore()
	waitFor(t, "retried page", func() bool {
		snapshot := store.Snapshot()
		return len(snapshot.Items) == 3 && snapshot.LoadMoreError == ""
	})
	if attempts := appendAttempts.Load(); attempts != 2 {
		t.Fatalf("append attempts = %d, want 2", attempts)
	}
}

func TestSnapshotCopiesOptionalPointers(t *testing.T) {
	available := false
	copiedAt := uint64(123)
	source := &fakeSource{respond: func(Query) (Page, error) {
		return Page{Items: []Item{{
			ID:         1,
			Preview:    "preview",
			Available:  &available,
			CopiedAtMS: &copiedAt,
		}}}, nil
	}}
	store := NewStore(source)
	store.debounce = 0
	store.timeout = time.Second
	t.Cleanup(store.Close)

	store.EnsureLoaded()
	waitFor(t, "loaded page", func() bool { return store.Snapshot().State == StateLoaded })

	first := store.Snapshot()
	*first.Items[0].Available = true
	*first.Items[0].CopiedAtMS = 456

	second := store.Snapshot()
	if *second.Items[0].Available {
		t.Fatal("snapshot mutation changed Available")
	}
	if *second.Items[0].CopiedAtMS != 123 {
		t.Fatalf("snapshot mutation changed CopiedAtMS: %d", *second.Items[0].CopiedAtMS)
	}
}

func intPointer(value int) *int { return &value }
