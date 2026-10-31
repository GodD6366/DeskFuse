package clipboard

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"
)

// State is the page state the shared UI renders. Unknown and empty are
// distinct: an unavailable bridge must never look like an empty history.
type State string

const (
	StateUnavailable State = "unavailable"
	StateIdle        State = "idle"
	StateLoading     State = "loading"
	StateLoaded      State = "loaded"
	StateEmpty       State = "empty"
	StateError       State = "error"
)

// Snapshot is an immutable copy of the page state for one frame.
type Snapshot struct {
	State         State
	Items         []Item
	SelectedID    uint64
	Search        string
	HasMore       bool
	LoadingMore   bool
	LoadMoreError string
	Error         string
	Revision      uint64
}

// SelectedItem returns the selected row, if it is still loaded.
func (s Snapshot) SelectedItem() (Item, bool) {
	for _, item := range s.Items {
		if item.ID == s.SelectedID {
			return item, true
		}
	}
	return Item{}, false
}

type reloadPlan struct {
	ctx        context.Context
	generation uint64
	search     string
	debounce   bool
}

// Store owns the read-only clipboard page state. All methods are safe for
// concurrent use: the MyGo view, bridge replies and future engine events may
// arrive on different goroutines.
type Store struct {
	mu         sync.Mutex
	source     Source
	baseCtx    context.Context
	cancelBase context.CancelFunc
	cancel     context.CancelFunc
	notify     func()

	debounce  time.Duration
	timeout   time.Duration
	maxLoaded int
	closed    bool

	state         State
	items         []Item
	selectedID    uint64
	search        string
	cursor        *Cursor
	nextOffset    *int
	hasMore       bool
	loadingMore   bool
	loadMoreError string
	err           string
	revision      uint64
	generation    uint64
}

// NewStore creates a clipboard page store. A nil source means the native
// bridge was not configured: the page reports unavailable and never invents
// data.
func NewStore(source Source) *Store {
	ctx, cancel := context.WithCancel(context.Background())
	store := &Store{
		source:     source,
		baseCtx:    ctx,
		cancelBase: cancel,
		debounce:   120 * time.Millisecond,
		timeout:    5 * time.Second,
		maxLoaded:  DefaultMaxLoadedItems,
		state:      StateIdle,
	}
	if source == nil {
		store.state = StateUnavailable
	}
	return store
}

// SetNotify installs the redraw callback. main wires this to Window.Invalidate
// once the MyGo window exists; it is safe to call from any goroutine.
func (s *Store) SetNotify(fn func()) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return
	}
	s.notify = fn
}

// Close cancels in-flight queries and drops later replies.
func (s *Store) Close() {
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return
	}
	s.closed = true
	if s.cancel != nil {
		s.cancel()
		s.cancel = nil
	}
	s.cancelBase()
	s.notify = nil
	s.mu.Unlock()
}

// EnsureLoaded starts the first page once. The view calls it whenever the
// clipboard section is rendered.
func (s *Store) EnsureLoaded() {
	s.mu.Lock()
	if s.closed || s.state != StateIdle {
		s.mu.Unlock()
		return
	}
	plan, ok := s.beginReloadLocked(false)
	s.mu.Unlock()
	if !ok {
		return
	}
	s.notifyChange()
	go s.runQuery(plan)
}

// SetSearch replaces the query and starts a debounced first page. The old
// request is canceled and its generation invalidated, so a late reply cannot
// replace the newer search.
func (s *Store) SetSearch(search string) {
	s.mu.Lock()
	if s.closed || search == s.search {
		s.mu.Unlock()
		return
	}
	s.search = search
	if s.source == nil {
		s.state = StateUnavailable
		s.revision++
		s.mu.Unlock()
		s.notifyChange()
		return
	}
	plan, ok := s.beginReloadLocked(true)
	s.mu.Unlock()
	if !ok {
		return
	}
	s.notifyChange()
	go s.runQuery(plan)
}

// Retry reloads the current search from the first page.
func (s *Store) Retry() {
	s.mu.Lock()
	if s.closed || s.source == nil {
		s.mu.Unlock()
		return
	}
	plan, ok := s.beginReloadLocked(false)
	s.mu.Unlock()
	if !ok {
		return
	}
	s.notifyChange()
	go s.runQuery(plan)
}

// LoadMore appends the next cursor page. Concurrent calls are ignored; a
// search started while it is in flight invalidates its reply.
func (s *Store) LoadMore() {
	s.mu.Lock()
	if s.closed || s.source == nil || s.state != StateLoaded || !s.hasMore || s.loadingMore {
		s.mu.Unlock()
		return
	}
	generation := s.generation
	search := s.search
	var cursor *Cursor
	if s.cursor != nil {
		value := *s.cursor
		cursor = &value
	}
	offset := 0
	if s.nextOffset != nil {
		offset = *s.nextOffset
	}
	ctx, cancel := context.WithCancel(s.baseCtx)
	s.cancel = cancel
	s.loadingMore = true
	s.loadMoreError = ""
	s.revision++
	s.mu.Unlock()
	s.notifyChange()
	go s.runAppend(ctx, generation, search, cursor, offset)
}

// Select changes the preview target. Rows not in the loaded page are ignored.
func (s *Store) Select(id uint64) {
	changed := false
	s.mu.Lock()
	if !s.closed && s.selectedID != id {
		for _, item := range s.items {
			if item.ID == id {
				s.selectedID = id
				s.revision++
				changed = true
				break
			}
		}
	}
	s.mu.Unlock()
	if changed {
		s.notifyChange()
	}
}

// Snapshot copies the page state for the UI. Nested slices are copied so a
// caller cannot mutate the store through the snapshot.
func (s *Store) Snapshot() Snapshot {
	s.mu.Lock()
	defer s.mu.Unlock()
	return Snapshot{
		State:         s.state,
		Items:         copyItems(s.items),
		SelectedID:    s.selectedID,
		Search:        s.search,
		HasMore:       s.hasMore,
		LoadingMore:   s.loadingMore,
		LoadMoreError: s.loadMoreError,
		Error:         s.err,
		Revision:      s.revision,
	}
}

func (s *Store) beginReloadLocked(debounce bool) (reloadPlan, bool) {
	if s.closed || s.source == nil {
		return reloadPlan{}, false
	}
	s.generation++
	if s.cancel != nil {
		s.cancel()
	}
	ctx, cancel := context.WithCancel(s.baseCtx)
	s.cancel = cancel
	s.state = StateLoading
	s.err = ""
	s.loadMoreError = ""
	s.loadingMore = false
	s.hasMore = false
	s.cursor = nil
	s.nextOffset = nil
	s.items = nil
	s.selectedID = 0
	s.revision++
	return reloadPlan{
		ctx:        ctx,
		generation: s.generation,
		search:     s.search,
		debounce:   debounce,
	}, true
}

func (s *Store) runQuery(plan reloadPlan) {
	s.mu.Lock()
	debounce, timeout := s.debounce, s.timeout
	s.mu.Unlock()

	if plan.debounce && debounce > 0 {
		timer := time.NewTimer(debounce)
		select {
		case <-plan.ctx.Done():
			timer.Stop()
			return
		case <-timer.C:
		}
	}

	ctx, cancel := context.WithTimeout(plan.ctx, timeout)
	defer cancel()
	page, err := s.source.Query(ctx, s.request(plan.search, nil, 0))
	s.finish(plan, page, err, false)
}

func (s *Store) runAppend(ctx context.Context, generation uint64, search string, cursor *Cursor, offset int) {
	s.mu.Lock()
	timeout := s.timeout
	s.mu.Unlock()

	callCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	page, err := s.source.Query(callCtx, s.request(search, cursor, offset))
	s.finish(reloadPlan{ctx: ctx, generation: generation, search: search}, page, err, true)
}

func (s *Store) finish(plan reloadPlan, page Page, err error, appendPage bool) {
	s.mu.Lock()
	if s.closed || plan.generation != s.generation {
		s.mu.Unlock()
		return
	}

	if err != nil {
		if errors.Is(err, context.Canceled) {
			s.mu.Unlock()
			return
		}
		if appendPage {
			s.loadingMore = false
			s.loadMoreError = friendlyError(err)
		} else {
			s.state = StateError
			s.err = friendlyError(err)
			s.items = nil
			s.selectedID = 0
			s.cursor = nil
			s.nextOffset = nil
			s.hasMore = false
		}
		s.revision++
		s.mu.Unlock()
		s.notifyChange()
		return
	}

	if appendPage {
		s.appendItemsLocked(page.Items)
		s.loadingMore = false
		s.loadMoreError = ""
	} else {
		s.items = dedupeItems(page.Items)
		s.err = ""
		if len(s.items) == 0 {
			s.state = StateEmpty
		} else {
			s.state = StateLoaded
		}
	}
	s.cursor = page.NextCursor
	s.nextOffset = page.NextOffset
	s.hasMore = page.NextCursor != nil || page.NextOffset != nil
	s.normalizeSelectionLocked()
	s.revision++
	s.mu.Unlock()
	s.notifyChange()
}

func (s *Store) request(search string, cursor *Cursor, offset int) Query {
	now := time.Now()
	start := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
	return Query{
		Op:               "items",
		ListID:           nil,
		Search:           search,
		Kind:             "all",
		Source:           nil,
		Limit:            PageSize,
		IncludeTotal:     false,
		Offset:           offset,
		Cursor:           cursor,
		DayStartMS:       start.UnixMilli(),
		YesterdayStartMS: start.AddDate(0, 0, -1).UnixMilli(),
		DayEndMS:         start.AddDate(0, 0, 1).UnixMilli(),
		Timezone:         now.Location().String(),
		RuleWindows:      map[string]any{},
	}
}

func (s *Store) appendItemsLocked(incoming []Item) {
	seen := make(map[uint64]struct{}, len(s.items)+len(incoming))
	for _, item := range s.items {
		seen[item.ID] = struct{}{}
	}
	for _, item := range incoming {
		if _, exists := seen[item.ID]; exists {
			continue
		}
		seen[item.ID] = struct{}{}
		s.items = append(s.items, copyItem(item))
	}
	if s.maxLoaded > 0 && len(s.items) > s.maxLoaded {
		s.items = append([]Item(nil), s.items[len(s.items)-s.maxLoaded:]...)
	}
}

func (s *Store) normalizeSelectionLocked() {
	if len(s.items) == 0 {
		s.selectedID = 0
		return
	}
	for _, item := range s.items {
		if item.ID == s.selectedID {
			return
		}
	}
	s.selectedID = s.items[0].ID
}

func (s *Store) notifyChange() {
	s.mu.Lock()
	fn := s.notify
	s.mu.Unlock()
	if fn != nil {
		fn()
	}
}

func copyItems(items []Item) []Item {
	if len(items) == 0 {
		return nil
	}
	copied := make([]Item, len(items))
	for index, item := range items {
		copied[index] = copyItem(item)
	}
	return copied
}

func copyItem(item Item) Item {
	item.RelatedIDs = append([]uint64(nil), item.RelatedIDs...)
	item.Memberships = append([]string(nil), item.Memberships...)
	if item.CopiedAtMS != nil {
		value := *item.CopiedAtMS
		item.CopiedAtMS = &value
	}
	if item.Available != nil {
		value := *item.Available
		item.Available = &value
	}
	return item
}

func dedupeItems(items []Item) []Item {
	seen := make(map[uint64]struct{}, len(items))
	result := make([]Item, 0, len(items))
	for _, item := range items {
		if _, exists := seen[item.ID]; exists {
			continue
		}
		seen[item.ID] = struct{}{}
		result = append(result, copyItem(item))
	}
	return result
}

func friendlyError(err error) string {
	if errors.Is(err, context.DeadlineExceeded) {
		return "读取剪贴板历史超时；原生宿主未响应。"
	}
	message := strings.TrimSpace(err.Error())
	if message == "" {
		message = "未知错误"
	}
	return fmt.Sprintf("读取剪贴板历史失败：%s", message)
}
