// Package clipboard implements the read-only clipboard history page state and
// the bridge adapter for the native clipboard library.
//
// The engine remains the source of truth. This package never fabricates rows,
// capabilities or previews: it only decodes the query response documented by
// ClipboardLibraryStore.swift and keeps the search/pagination state needed by
// the MyGo view.
package clipboard

// PageSize matches ClipboardPaging.pageSize in the macOS client. The engine
// clamps limits to 1...100.
const PageSize = 20

// DefaultMaxLoadedItems matches the macOS client's bounded scrolling window.
const DefaultMaxLoadedItems = 100

// Cursor is the engine's keyset pagination cursor. It is preferable to offset
// paging because a new clipboard row can be inserted while the user scrolls.
type Cursor struct {
	SortAtMS uint64 `json:"sort_at_ms"`
	ID       uint64 `json:"id"`
}

// Item mirrors the fields returned by the engine's `items` query. Optional
// fields stay pointers where zero and absent have different meanings.
type Item struct {
	ID             uint64   `json:"id"`
	RelatedIDs     []uint64 `json:"related_ids,omitempty"`
	CopiedAtMS     *uint64  `json:"copied_at_ms,omitempty"`
	MIME           string   `json:"mime"`
	Length         uint64   `json:"length"`
	Preview        string   `json:"preview"`
	Source         string   `json:"source"`
	HasImage       bool     `json:"has_image"`
	FileCount      uint32   `json:"file_count"`
	Memberships    []string `json:"memberships,omitempty"`
	SourceDeviceID string   `json:"source_device_id,omitempty"`
	SourceAppID    string   `json:"source_app_id,omitempty"`
	SourceAppName  string   `json:"source_app_name,omitempty"`
	Title          string   `json:"title,omitempty"`
	OCRText        string   `json:"ocr_text,omitempty"`
	OCRVersion     uint64   `json:"ocr_version,omitempty"`
	Remote         bool     `json:"remote,omitempty"`
	Available      *bool    `json:"available,omitempty"`
	SortAtMS       uint64   `json:"sort_at_ms,omitempty"`
}

// Page mirrors ClipboardLibraryPage in ClipboardLibraryStore.swift and the
// engine's items response.
type Page struct {
	Items      []Item  `json:"items"`
	Total      int     `json:"total"`
	TotalExact *bool   `json:"total_exact,omitempty"`
	NextOffset *int    `json:"next_offset,omitempty"`
	NextCursor *Cursor `json:"next_cursor,omitempty"`
	Revision   uint64  `json:"revision,omitempty"`
}

// Query is the request object accepted by the engine's library query. The
// fields and null-vs-empty semantics follow ClipboardLibraryStore.swift's
// itemRequest.
type Query struct {
	Op               string         `json:"op"`
	ListID           *string        `json:"list_id"`
	Search           string         `json:"query"`
	Kind             string         `json:"kind"`
	Source           *string        `json:"source"`
	Limit            int            `json:"limit"`
	IncludeTotal     bool           `json:"include_total"`
	Offset           int            `json:"offset"`
	Cursor           *Cursor        `json:"cursor,omitempty"`
	DayStartMS       int64          `json:"day_start_ms"`
	YesterdayStartMS int64          `json:"yesterday_start_ms"`
	DayEndMS         int64          `json:"day_end_ms"`
	Timezone         string         `json:"timezone"`
	RuleWindows      map[string]any `json:"rule_windows"`
}
