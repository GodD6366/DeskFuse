package winservice

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/native"
)

// contentEntry is one materialized library row: its wire formats decoded, the
// local files it references and its plain text.
type contentEntry struct {
	formats []native.Format
	files   []string
	text    string
}

func (s *Service) contentEntry(ctx context.Context, id uint64) (contentEntry, error) {
	if err := s.ensureSession(ctx); err != nil {
		return contentEntry{}, err
	}
	content, err := s.libraryContent(ctx, id)
	if err != nil {
		return contentEntry{}, err
	}
	entry := contentEntry{files: content.Files}
	for _, format := range content.Formats {
		if format.MIME == "text/plain" {
			entry.text, _ = decodeFormatText(content.Formats, "text/plain")
			entry.formats = append(entry.formats, native.Format{MIME: "text/plain", Data: []byte(entry.text)})
			continue
		}
		data, err := base64.StdEncoding.DecodeString(format.DataBase64)
		if err != nil {
			return contentEntry{}, invalidParams(fmt.Sprintf("条目 %s 的格式无效", format.MIME))
		}
		entry.formats = append(entry.formats, native.Format{MIME: format.MIME, Data: data})
	}
	return entry, nil
}

func (s *Service) requireNative() error {
	if s.native == nil {
		return unsupported("此窗口没有接入 Windows 本地剪贴板适配层。")
	}
	return nil
}

// clipboardCopy restores one entry onto the system clipboard. Paste repeats
// the write and then hands the keystroke to the previous application.
func (s *Service) clipboardCopy(ctx context.Context, args map[string]any, result any, paste bool) error {
	if err := s.requireNative(); err != nil {
		return err
	}
	id, err := uintArg(args, "id", "内容 ID 无效")
	if err != nil {
		return err
	}
	entry, err := s.contentEntry(ctx, id)
	if err != nil {
		return err
	}
	plain, _ := boolValue(args, "plain_text", "plainText")
	formats := entry.formats
	files := entry.files
	if plain {
		if entry.text == "" {
			return invalidParams("该条目没有可复制的纯文本")
		}
		formats = []native.Format{{MIME: "text/plain", Data: []byte(entry.text)}}
		files = nil
	}
	if err := s.native.WriteClipboard(formats, files); err != nil {
		return nativeError(err)
	}
	action := "clipboard.copy"
	if paste {
		action = "clipboard.paste"
		// The panel floats above other apps; hiding first lets the target
		// window take the foreground before the keystroke is injected.
		if s.host != nil {
			s.host.HidePanel()
		}
		if err := s.native.PasteClipboard(); err != nil {
			return nativeError(err)
		}
	}
	return assign(result, map[string]any{"outcome": "applied", "action": action})
}

// clipboardSave writes the entry through a native save dialog. Files are
// copied into a chosen folder; images and text use a save-file dialog.
func (s *Service) clipboardSave(ctx context.Context, args map[string]any, result any) error {
	if err := s.requireNative(); err != nil {
		return err
	}
	id, err := uintArg(args, "id", "内容 ID 无效")
	if err != nil {
		return err
	}
	entry, err := s.contentEntry(ctx, id)
	if err != nil {
		return err
	}
	hasImage, _ := boolValue(args, "has_image", "hasImage")
	if hasImage {
		if data, ok := formatBytes(entry.formats, "image/png"); ok {
			path, saved, err := s.native.SaveFile("剪贴板图片.png",
				[]native.Filter{{Name: "PNG 图片", Extensions: []string{"png"}}}, data)
			if err != nil {
				return nativeError(err)
			}
			return saveOutcome(result, saved, path)
		}
	}
	if len(entry.files) > 0 {
		for _, path := range entry.files {
			if _, err := os.Stat(path); err != nil {
				return nativeError(errors.New("文件已不可用，请重新获取。"))
			}
		}
		folder, saved, err := s.native.ChooseFolder("选择保存位置")
		if err != nil {
			return nativeError(err)
		}
		if !saved {
			return saveOutcome(result, false, "")
		}
		for _, source := range entry.files {
			destination := uniquePath(filepath.Join(folder, filepath.Base(source)))
			if err := copyFile(source, destination); err != nil {
				return nativeError(fmt.Errorf("复制文件失败: %w", err))
			}
		}
		return saveOutcome(result, true, folder)
	}
	if entry.text == "" {
		return invalidParams("该条目没有可保存的内容")
	}
	path, saved, err := s.native.SaveFile("剪贴板文本.txt",
		[]native.Filter{{Name: "文本文件", Extensions: []string{"txt"}}}, []byte(entry.text))
	if err != nil {
		return nativeError(err)
	}
	return saveOutcome(result, saved, path)
}

func saveOutcome(result any, saved bool, path string) error {
	value := map[string]any{"saved": saved}
	if path != "" {
		value["path"] = path
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "clipboard.save", "value": value})
}

// clipboardReveal selects a file entry in Explorer. Text and image entries
// have no local file to reveal, matching the macOS action.
func (s *Service) clipboardReveal(ctx context.Context, args map[string]any, result any) error {
	if err := s.requireNative(); err != nil {
		return err
	}
	id, err := uintArg(args, "id", "内容 ID 无效")
	if err != nil {
		return err
	}
	entry, err := s.contentEntry(ctx, id)
	if err != nil {
		return err
	}
	if len(entry.files) == 0 {
		return nativeError(errors.New("没有可显示的文件"))
	}
	if err := s.native.Reveal(entry.files); err != nil {
		return nativeError(err)
	}
	return assign(result, map[string]any{"outcome": "dispatched", "action": "clipboard.reveal"})
}

// clipboardOpen writes image or text entries to a temporary file and opens it
// with the default application; file entries open in place.
func (s *Service) clipboardOpen(ctx context.Context, args map[string]any, result any, action string) error {
	if err := s.requireNative(); err != nil {
		return err
	}
	id, err := uintArg(args, "id", "内容 ID 无效")
	if err != nil {
		return err
	}
	entry, err := s.contentEntry(ctx, id)
	if err != nil {
		return err
	}
	if len(entry.files) > 0 {
		for _, path := range entry.files {
			if err := s.native.OpenPath(path); err != nil {
				return nativeError(err)
			}
		}
		return assign(result, map[string]any{"outcome": "dispatched", "action": action})
	}
	name := fmt.Sprintf("deskfuse-%d", id)
	var data []byte
	switch {
	case formatHas(entry.formats, "image/png"):
		data, _ = formatBytes(entry.formats, "image/png")
		name += ".png"
	case entry.text != "":
		data = []byte(entry.text)
		name += ".txt"
	default:
		return nativeError(errors.New("该条目没有可打开的内容"))
	}
	temporary := filepath.Join(os.TempDir(), name)
	if err := os.WriteFile(temporary, data, 0o644); err != nil {
		return nativeError(fmt.Errorf("写入临时文件失败: %w", err))
	}
	if err := s.native.OpenPath(temporary); err != nil {
		return nativeError(err)
	}
	return assign(result, map[string]any{"outcome": "dispatched", "action": action})
}

// workflowRun executes the service's synchronous workflow and applies the
// returned formats to the clipboard, which the service's contract requires
// before the run may be reported as complete.
func (s *Service) workflowRun(ctx context.Context, args map[string]any, result any) error {
	if err := s.requireNative(); err != nil {
		return err
	}
	workflowID, err := requiredString(args, []string{"workflow_id", "workflowId"}, "需要工作流")
	if err != nil {
		return err
	}
	id, err := uintArg(args, "history_id", "内容 ID 无效")
	if err != nil {
		return err
	}
	entry, err := s.contentEntry(ctx, id)
	if err != nil {
		return err
	}
	if len(entry.files) > 0 {
		return unsupported("Windows 工作流只能处理 MIME 内容；文件条目请直接使用或共享。")
	}
	if len(entry.formats) == 0 {
		return invalidParams("内容库条目没有可执行的内容")
	}
	input := make([]map[string]any, 0, len(entry.formats))
	for _, format := range entry.formats {
		input = append(input, map[string]any{
			"mime":        format.MIME,
			"data_base64": base64.StdEncoding.EncodeToString(format.Data),
		})
	}
	raw, err := s.callVariant(ctx, tagged("RunWorkflow", map[string]any{
		"workflow_id": workflowID,
		"input":       input,
	}), "WorkflowResult")
	if err != nil {
		return err
	}
	var run struct {
		RunID   string       `json:"run_id"`
		Formats []formatWire `json:"formats"`
		Message string       `json:"message"`
	}
	if err := json.Unmarshal(raw, &run); err != nil {
		return fmt.Errorf("工作流结果无效: %w", err)
	}
	output := make([]native.Format, 0, len(run.Formats))
	for _, format := range run.Formats {
		data, err := base64.StdEncoding.DecodeString(format.DataBase64)
		if err != nil {
			return nativeError(fmt.Errorf("工作流输出的 %s 无效", format.MIME))
		}
		output = append(output, native.Format{MIME: format.MIME, Data: data})
	}
	if len(output) == 0 {
		return nativeError(errors.New("工作流没有生成可复制的内容"))
	}
	if err := s.native.WriteClipboard(output, nil); err != nil {
		return nativeError(err)
	}
	return assign(result, map[string]any{
		"outcome": "applied",
		"action":  "workflow.run",
		"value":   map[string]any{"run_id": run.RunID, "message": run.Message},
	})
}

func nativeError(err error) error {
	var remote *RemoteError
	if errors.As(err, &remote) {
		return err
	}
	return &RemoteError{Code: "native_error", Message: err.Error()}
}

func formatBytes(formats []native.Format, mime string) ([]byte, bool) {
	for _, format := range formats {
		if format.MIME == mime && len(format.Data) > 0 {
			return format.Data, true
		}
	}
	return nil, false
}

func formatHas(formats []native.Format, mime string) bool {
	_, ok := formatBytes(formats, mime)
	return ok
}

func uniquePath(path string) string {
	if _, err := os.Stat(path); errors.Is(err, os.ErrNotExist) {
		return path
	}
	dir := filepath.Dir(path)
	extension := filepath.Ext(path)
	stem := strings.TrimSuffix(filepath.Base(path), extension)
	for index := 2; ; index++ {
		candidate := filepath.Join(dir, fmt.Sprintf("%s %d%s", stem, index, extension))
		if _, err := os.Stat(candidate); errors.Is(err, os.ErrNotExist) {
			return candidate
		}
	}
}

func copyFile(source, destination string) error {
	data, err := os.ReadFile(source)
	if err != nil {
		return err
	}
	return os.WriteFile(destination, data, 0o644)
}
