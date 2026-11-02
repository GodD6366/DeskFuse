package winservice

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

// StagedFile is one file the transfers page holds before sending.
type StagedFile struct {
	Path        string `json:"path"`
	Name        string `json:"name"`
	Size        int64  `json:"size"`
	IsDirectory bool   `json:"isDirectory"`
}

func inspectPaths(paths []string) []StagedFile {
	out := make([]StagedFile, 0, len(paths))
	for _, path := range paths {
		info, err := os.Stat(path)
		if err != nil {
			// Keep the reference so the page can report the bad path.
			out = append(out, StagedFile{Path: path, Name: filepath.Base(path)})
			continue
		}
		out = append(out, StagedFile{
			Path:        path,
			Name:        filepath.Base(path),
			Size:        info.Size(),
			IsDirectory: info.IsDir(),
		})
	}
	return out
}

// transfersChooseFiles opens the native picker and stages the selection.
func (s *Service) transfersChooseFiles(result any) error {
	if s.native == nil {
		return unsupported("此窗口没有接入系统文件选择。")
	}
	paths, picked, err := s.native.PickFiles("选择要发送的文件")
	if err != nil {
		return nativeError(err)
	}
	if !picked {
		return assign(result, map[string]any{"outcome": "applied", "value": []any{}})
	}
	return assign(result, map[string]any{"outcome": "applied", "value": inspectPaths(paths)})
}

// transfersInspectPaths turns drag-and-drop paths into staged metadata.
func (s *Service) transfersInspectPaths(args map[string]any, result any) error {
	raw, ok := args["paths"].([]any)
	if !ok || len(raw) == 0 {
		return invalidParams("需要 paths")
	}
	paths := make([]string, 0, len(raw))
	for _, value := range raw {
		path, ok := value.(string)
		if !ok || path == "" {
			return invalidParams("文件路径无效")
		}
		paths = append(paths, path)
	}
	return assign(result, map[string]any{"outcome": "applied", "value": inspectPaths(paths)})
}

// transfersSend offers explicit paths to one peer through the file clipboard.
func (s *Service) transfersSend(ctx context.Context, args map[string]any, result any) error {
	peer, err := requiredString(args, []string{"peer_id"}, "需要目标设备")
	if err != nil {
		return err
	}
	raw, ok := args["paths"].([]any)
	if !ok || len(raw) == 0 {
		return invalidParams("需要 paths")
	}
	paths := make([]string, 0, len(raw))
	for _, value := range raw {
		path, ok := value.(string)
		if !ok || path == "" {
			return invalidParams("文件路径无效")
		}
		paths = append(paths, path)
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
	return assign(result, map[string]any{"outcome": "queued", "action": "transfers.send"})
}

// setFiles persists the full file-transfer settings through the service.
func (s *Service) setFiles(ctx context.Context, args map[string]any, result any) error {
	raw, ok := args["settings"].(map[string]any)
	if !ok {
		return invalidParams("需要 settings")
	}
	settings := map[string]any{
		"enabled":                 raw["enabled"],
		"small_file_prefetch_enabled": raw["prefetch"],
		"small_file_threshold_bytes":  raw["threshold"],
		"cache_max_bytes":             raw["maximum"],
		"download_directory":          raw["download_directory"],
	}
	if dir, _ := raw["download_directory"].(string); dir == "" {
		settings["download_directory"] = ""
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callQueued(ctx, tagged("SetFileSettings", map[string]any{"settings": settings}), "set_files", result)
}

// clearCache removes completed transfers' cached bytes.
func (s *Service) clearCache(ctx context.Context, result any) error {
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	raw, err := s.callVariant(ctx, "ClearFileCache", "CacheCleared")
	if err != nil {
		return err
	}
	var reply struct {
		RemovedBytes uint64 `json:"removed_bytes"`
	}
	if err := json.Unmarshal(raw, &reply); err != nil {
		return fmt.Errorf("缓存清理结果无效: %w", err)
	}
	return assign(result, map[string]any{
		"outcome": "applied",
		"action":  "clear_cache",
		"value":   map[string]any{"removed_bytes": reply.RemovedBytes},
	})
}

// transfersCancel asks the service to stop one transfer.
func (s *Service) transfersCancel(ctx context.Context, args map[string]any, result any) error {
	id, err := requiredString(args, []string{"id", "transfer_id"}, "需要传输 ID")
	if err != nil {
		return err
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callQueued(ctx, tagged("CancelTransfer", map[string]any{"transfer_id": id}), "transfers.cancel", result)
}

// transfersRetry re-announces one failed outgoing transfer.
func (s *Service) transfersRetry(ctx context.Context, args map[string]any, result any) error {
	id, err := requiredString(args, []string{"id", "transfer_id"}, "需要传输 ID")
	if err != nil {
		return err
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callQueued(ctx, tagged("RetryTransfer", map[string]any{"transfer_id": id}), "transfers.retry", result)
}

// workflowSystemShortcuts returns the platform shortcut catalog. Windows has
// no App-Intents-style source, so the service reports an empty list; the step
// editor renders "no shortcuts" instead of an error.
func (s *Service) workflowSystemShortcuts(ctx context.Context, result any) error {
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	raw, err := s.callVariant(ctx, "GetSystemShortcuts", "SystemShortcuts")
	if err != nil {
		return err
	}
	var reply struct {
		Shortcuts []struct {
			ID   string `json:"id"`
			Name string `json:"name"`
		} `json:"shortcuts"`
	}
	if err := json.Unmarshal(raw, &reply); err != nil {
		return fmt.Errorf("快捷指令目录无效: %w", err)
	}
	shortcuts := make([]map[string]any, 0, len(reply.Shortcuts))
	for _, shortcut := range reply.Shortcuts {
		shortcuts = append(shortcuts, map[string]any{"id": shortcut.ID, "name": shortcut.Name})
	}
	return assign(result, map[string]any{"outcome": "applied", "value": shortcuts})
}
