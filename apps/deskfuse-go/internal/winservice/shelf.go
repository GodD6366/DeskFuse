package winservice

import (
	"context"
	"fmt"
	"strconv"
	"strings"
	"sync"
)

// shelfFile is one staged file reference. Only paths are held, never contents.
type shelfFile struct {
	ID   string `json:"id"`
	Path string `json:"path"`
	Name string `json:"name"`
}

// shelfState holds the desktop client's staging area. The service owns file
// transfer; this is only the list of references the page renders and sends.
type shelfState struct {
	mu        sync.Mutex
	files     []shelfFile
	preparing bool
	nextID    int
}

func newShelfState() *shelfState { return &shelfState{nextID: 1} }

func (s *shelfState) add(paths []string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, path := range paths {
		s.files = append(s.files, shelfFile{
			ID:   strconv.Itoa(s.nextID),
			Path: path,
			Name: shelfBaseName(path),
		})
		s.nextID++
	}
}

// shelfBaseName accepts both Windows and POSIX separators. Tests and
// cross-platform callers may construct Windows paths while running on another
// host, where filepath.Base would treat the backslash as an ordinary byte.
func shelfBaseName(value string) string {
	value = strings.TrimRight(value, `/\\`)
	if index := strings.LastIndexAny(value, `/\\`); index >= 0 {
		return value[index+1:]
	}
	return value
}

func (s *shelfState) remove(id, path string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	out := s.files[:0]
	for _, file := range s.files {
		if id != "" && file.ID == id {
			continue
		}
		if id == "" && path != "" && file.Path == path {
			continue
		}
		out = append(out, file)
	}
	s.files = out
}

func (s *shelfState) clear() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.files = nil
	s.preparing = false
}

func (s *shelfState) paths() []string {
	s.mu.Lock()
	defer s.mu.Unlock()
	out := make([]string, 0, len(s.files))
	for _, file := range s.files {
		out = append(out, file.Path)
	}
	return out
}

// view renders the snapshot projection the shelf page consumes.
func (s *shelfState) view() map[string]any {
	s.mu.Lock()
	defer s.mu.Unlock()
	files := make([]map[string]any, 0, len(s.files))
	for _, file := range s.files {
		files = append(files, map[string]any{"id": file.ID, "path": file.Path, "name": file.Name})
	}
	return map[string]any{"preparing": s.preparing, "files": files}
}

func (s *shelfState) empty() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return len(s.files) == 0
}

// shelfAdd appends local paths to the staging area and shows its window.
func (s *Service) shelfAdd(args map[string]any, result any) error {
	raw, ok := args["paths"].([]any)
	if !ok || len(raw) == 0 {
		return invalidParams("请选择本地文件")
	}
	paths := make([]string, 0, len(raw))
	for _, value := range raw {
		path, ok := value.(string)
		if !ok || path == "" {
			return invalidParams("文件路径无效")
		}
		paths = append(paths, path)
	}
	s.shelf.add(paths)
	if s.host != nil {
		s.host.ShowShelf()
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "shelf.add"})
}

// shelfStage materializes one library entry's local files into the shelf.
func (s *Service) shelfStage(ctx context.Context, args map[string]any, result any) error {
	id, err := uintArg(args, "id", "内容 ID 无效")
	if err != nil {
		return err
	}
	entry, err := s.contentEntry(ctx, id)
	if err != nil {
		return err
	}
	if len(entry.files) == 0 {
		return nativeError(fmt.Errorf("该条目没有可暂存的本地文件"))
	}
	s.shelf.add(entry.files)
	if s.host != nil {
		s.host.ShowShelf()
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "clipboard.stage"})
}

func (s *Service) shelfRemove(args map[string]any, result any) error {
	id, _ := stringValue(args, "id")
	path, _ := stringValue(args, "path")
	if id == "" && path == "" {
		return invalidParams("需要 id 或 path")
	}
	s.shelf.remove(id, path)
	return assign(result, map[string]any{"outcome": "applied", "action": "shelf.remove"})
}

// shelfClear empties the staging area and closes its window, mirroring the
// macOS "清空并关闭" affordance.
func (s *Service) shelfClear(result any) error {
	s.shelf.clear()
	if s.host != nil {
		s.host.HideShelf()
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "shelf.clear"})
}

// shelfShow presents the staging-area window.
func (s *Service) shelfShow(result any) error {
	if s.host == nil {
		return unsupported("此窗口没有接入桌面宿主。")
	}
	s.host.ShowShelf()
	return assign(result, map[string]any{"outcome": "applied", "action": "shelf.show"})
}

// shelfSend offers every staged file to one peer through the service's file
// clipboard. The service owns the transfer once the offer is announced.
func (s *Service) shelfSend(ctx context.Context, args map[string]any, result any) error {
	peer, err := requiredString(args, []string{"peer_id"}, "需要目标设备")
	if err != nil {
		return err
	}
	paths := s.shelf.paths()
	if len(paths) == 0 {
		return invalidParams("暂存区是空的")
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	s.mu.Lock()
	sequence := s.publishSeq + 1
	s.publishSeq = sequence
	s.mu.Unlock()
	_, err = s.client.Call(ctx, tagged("PublishFileClipboard", map[string]any{
		"session_epoch":  s.sessionEpoch,
		"sequence":       sequence,
		"paths":          paths,
		"target_peer_id": peer,
	}))
	if err != nil {
		return err
	}
	return assign(result, map[string]any{"outcome": "queued", "action": "shelf.send"})
}
