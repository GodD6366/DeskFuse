package winservice

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"
)

// formatWire is the service's typed clipboard payload: a MIME identity and the
// bytes encoded as standard base64.
type formatWire struct {
	MIME       string `json:"mime"`
	DataBase64 string `json:"data_base64"`
}

type libraryContentWire struct {
	Formats []formatWire `json:"formats"`
	Files   []string     `json:"files"`
}

// library forwards the shared UI's engine request to the Windows service. The
// engine request is a JSON string on the wire; the Go backend passes an object.
func (s *Service) library(ctx context.Context, variant string, params any, result any) error {
	request, err := libraryRequest(params)
	if err != nil {
		return err
	}
	raw, err := s.libraryJSON(ctx, variant, request)
	if err != nil {
		return err
	}
	return assign(result, raw)
}

// libraryJSON ensures a desktop session, sends one LibraryQuery/LibraryMutate
// request and returns the engine's JSON result unchanged.
func (s *Service) libraryJSON(ctx context.Context, variant, request string) (json.RawMessage, error) {
	if err := s.ensureSession(ctx); err != nil {
		return nil, err
	}
	envelope, err := s.client.Call(ctx, tagged(variant, map[string]any{"request": request}))
	if err != nil {
		return nil, err
	}
	raw, ok := envelope["LibraryResult"]
	if !ok {
		return nil, errors.New("服务返回了非内容库结果")
	}
	var reply struct {
		JSON string `json:"json"`
	}
	if err := json.Unmarshal(raw, &reply); err != nil {
		return nil, fmt.Errorf("内容库结果无效: %w", err)
	}
	if strings.TrimSpace(reply.JSON) == "" {
		return nil, errors.New("内容库返回了空结果")
	}
	return json.RawMessage(reply.JSON), nil
}

func libraryRequest(params any) (string, error) {
	switch value := params.(type) {
	case nil:
		return "", invalidParams("内容库请求不能为空")
	case string:
		if strings.TrimSpace(value) == "" {
			return "", invalidParams("内容库请求不能为空")
		}
		return value, nil
	case json.RawMessage:
		if len(value) == 0 {
			return "", invalidParams("内容库请求不能为空")
		}
		return string(value), nil
	case map[string]any:
		data, err := json.Marshal(normalizeLibraryRequest(value))
		if err != nil {
			return "", fmt.Errorf("内容库请求无效: %w", err)
		}
		return string(data), nil
	default:
		return "", invalidParams("内容库请求格式无效")
	}
}

// normalizeLibraryRequest restores the numeric identity fields the MyGo binding
// stringified for JavaScript. The engine's query/mutate schema declares these
// as u64; sending "42" would be rejected as an invalid ID.
func normalizeLibraryRequest(request map[string]any) map[string]any {
	normalized := make(map[string]any, len(request))
	for key, value := range request {
		normalized[key] = value
	}
	for _, key := range []string{"id", "history_id"} {
		if text, ok := normalized[key].(string); ok {
			if parsed, err := strconv.ParseUint(text, 10, 64); err == nil {
				normalized[key] = parsed
			}
		}
	}
	if ids, ok := normalized["ids"].([]any); ok {
		normalized["ids"] = numericIDs(ids)
	}
	if ids, ok := normalized["ids"].([]string); ok {
		values := make([]any, len(ids))
		for index, id := range ids {
			values[index] = id
		}
		normalized["ids"] = numericIDs(values)
	}
	if cursor, ok := normalized["cursor"].(map[string]any); ok {
		copied := make(map[string]any, len(cursor))
		for key, value := range cursor {
			copied[key] = value
		}
		if text, ok := copied["id"].(string); ok {
			if parsed, err := strconv.ParseUint(text, 10, 64); err == nil {
				copied["id"] = parsed
			}
		}
		normalized["cursor"] = copied
	}
	if requests, ok := normalized["requests"].([]any); ok {
		rows := make([]any, len(requests))
		for index, row := range requests {
			if object, ok := row.(map[string]any); ok {
				rows[index] = normalizeLibraryRequest(object)
			} else {
				rows[index] = row
			}
		}
		normalized["requests"] = rows
	}
	return normalized
}

func numericIDs(ids []any) []any {
	values := make([]any, len(ids))
	for index, value := range ids {
		if text, ok := value.(string); ok {
			if parsed, err := strconv.ParseUint(text, 10, 64); err == nil {
				values[index] = parsed
				continue
			}
		}
		values[index] = value
	}
	return values
}

// content materializes one library row in the shape the shared clipboard page
// consumes. Image and RTF payloads are only included when requested, matching
// the macOS bridge.
func (s *Service) content(ctx context.Context, params any, result any) error {
	args, err := object(params)
	if err != nil {
		return err
	}
	id, err := uintArg(args, "id", "内容 ID 无效")
	if err != nil {
		return err
	}
	includeImage, _ := boolValue(args, "include_image", "includeImage")
	includeRTF, _ := boolValue(args, "include_rtf", "includeRtf")

	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	content, err := s.libraryContent(ctx, id)
	if err != nil {
		return err
	}

	text, err := decodeFormatText(content.Formats, "text/plain")
	if err != nil {
		return err
	}
	imageBase64, _ := formatData(content.Formats, "image/png")
	rtfBase64, _ := formatData(content.Formats, "text/rtf")

	value := map[string]any{
		"id":                strconv.FormatUint(id, 10),
		"text":              text,
		"truncated":         false,
		"file_paths":        nonNilStrings(content.Files),
		"rtf_available":     rtfBase64 != "",
		"has_image":         imageBase64 != "",
		"can_request_files": false,
	}
	if includeImage && imageBase64 != "" {
		value["image_png_base64"] = imageBase64
	}
	if includeRTF && rtfBase64 != "" {
		value["rtf_base64"] = rtfBase64
	}
	return assign(result, value)
}

// thumbnail returns the service's display-sized PNG preview. The service clamps
// the requested size to 64...480 and reports an empty payload when no preview
// exists.
func (s *Service) thumbnail(ctx context.Context, params any, result any) error {
	args, err := object(params)
	if err != nil {
		return err
	}
	id, err := uintArg(args, "id", "内容 ID 无效")
	if err != nil {
		return err
	}
	fields := map[string]any{"id": id}
	if size, ok := uintValue(args, "max_pixel", "max_pixel_size"); ok {
		fields["max_pixel_size"] = size
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	raw, err := s.callVariant(ctx, tagged("LibraryThumbnail", fields), "LibraryThumbnail")
	if err != nil {
		return err
	}
	var reply struct {
		MIME       string `json:"mime"`
		DataBase64 string `json:"data_base64"`
	}
	if err := json.Unmarshal(raw, &reply); err != nil {
		return fmt.Errorf("缩略图结果无效: %w", err)
	}
	return assign(result, map[string]any{"mime": reply.MIME, "data_base64": reply.DataBase64})
}

func (s *Service) libraryContent(ctx context.Context, id uint64) (libraryContentWire, error) {
	raw, err := s.callVariant(ctx, tagged("LibraryContent", map[string]any{"id": id}), "LibraryContent")
	if err != nil {
		return libraryContentWire{}, err
	}
	var content libraryContentWire
	if err := json.Unmarshal(raw, &content); err != nil {
		return libraryContentWire{}, fmt.Errorf("内容库条目无效: %w", err)
	}
	return content, nil
}

func formatData(formats []formatWire, mime string) (string, bool) {
	for _, format := range formats {
		if format.MIME == mime && format.DataBase64 != "" {
			return format.DataBase64, true
		}
	}
	return "", false
}

func decodeFormatText(formats []formatWire, mime string) (string, error) {
	encoded, ok := formatData(formats, mime)
	if !ok {
		return "", nil
	}
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil {
		return "", fmt.Errorf("服务返回的 %s 不是有效 base64: %w", mime, err)
	}
	return string(data), nil
}

// callVariant sends one command and returns the named reply variant.
func (s *Service) callVariant(ctx context.Context, command any, variant string) (json.RawMessage, error) {
	envelope, err := s.client.Call(ctx, command)
	if err != nil {
		return nil, err
	}
	raw, ok := envelope[variant]
	if !ok {
		return nil, fmt.Errorf("服务返回了非 %s 结果", variant)
	}
	return raw, nil
}

// clipboardPeers projects the trusted, unblocked peer list from the last
// snapshot. The service snapshot remains the authority; this is the same list
// the share menu already renders.
func (s *Service) clipboardPeers(ctx context.Context, result any) error {
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	latest, ok := s.latestSnapshot()
	if !ok {
		return errors.New("尚未取得 Windows 服务快照")
	}
	peers := make([]map[string]any, 0, len(latest.Peers))
	for _, peer := range latest.Peers {
		if peer.Trust != "trusted" {
			continue
		}
		peers = append(peers, map[string]any{
			"id":        peer.PeerID,
			"name":      displayName(peer.DisplayName, peer.PeerID),
			"connected": peer.Connection == "connected",
		})
	}
	return assign(result, map[string]any{"outcome": "applied", "value": peers})
}

// clipboardSources returns the canonical source-device list from the service
// library state, in the {id,name} shape the source filter expects.
func (s *Service) clipboardSources(ctx context.Context, result any) error {
	raw, err := s.libraryJSON(ctx, "LibraryQuery", `{"op":"state"}`)
	if err != nil {
		return err
	}
	var state struct {
		Devices []struct {
			ID   string `json:"id"`
			Name string `json:"name"`
		} `json:"devices"`
	}
	if err := json.Unmarshal(raw, &state); err != nil {
		return fmt.Errorf("内容库状态无效: %w", err)
	}
	sources := make([]map[string]any, 0, len(state.Devices))
	for _, device := range state.Devices {
		sources = append(sources, map[string]any{"id": device.ID, "name": device.Name})
	}
	return assign(result, map[string]any{"outcome": "applied", "value": sources})
}

// clipboardShare sends one library entry to a paired device through the
// service's acknowledged clipboard/file paths. A workflow variant composes
// RunWorkflow + ShareContent because the service executes both commands.
func (s *Service) clipboardShare(ctx context.Context, args map[string]any, result any) error {
	id, err := uintArg(args, "id", "内容 ID 无效")
	if err != nil {
		return err
	}
	peer, err := requiredString(args, []string{"peer_id"}, "需要目标设备")
	if err != nil {
		return err
	}
	workflowID, _ := stringValue(args, "workflow_id", "workflowId")
	if workflowID != "" {
		return s.shareWithWorkflow(ctx, id, peer, workflowID, result)
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callCommand(ctx, tagged("ShareLibraryEntry", map[string]any{"id": id, "peer_id": peer}), "clipboard.share", result)
}

func (s *Service) shareWithWorkflow(ctx context.Context, id uint64, peer, workflowID string, result any) error {
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	content, err := s.libraryContent(ctx, id)
	if err != nil {
		return err
	}
	// RunWorkflow accepts MIME payloads only; file references need the deferred
	// file channel, which the UI action does not request here.
	if len(content.Files) > 0 {
		return unsupported("工作流共享文件条目尚未接入 Windows 适配层；请直接发送该条目。")
	}
	if len(content.Formats) == 0 {
		return invalidParams("内容库条目没有可执行的内容")
	}
	input := make([]map[string]any, 0, len(content.Formats))
	for _, format := range content.Formats {
		input = append(input, map[string]any{"mime": format.MIME, "data_base64": format.DataBase64})
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
	if len(run.Formats) == 0 {
		return errors.New("工作流没有生成可发送的内容")
	}
	if _, err := s.client.Call(ctx, tagged("ShareContent", map[string]any{
		"peer_id": peer,
		"formats": run.Formats,
	})); err != nil {
		return err
	}
	return assign(result, map[string]any{
		"outcome": "queued",
		"action":  "clipboard.share",
		"value":   map[string]any{"run_id": run.RunID, "message": run.Message},
	})
}

// updateRichText applies a plain-text library edit through LibraryMutate. Styled
// runs require RTF generation, which is a native text-system capability this
// adapter does not have; rejecting them is better than silently editing text
// without its formatting.
func (s *Service) updateRichText(ctx context.Context, args map[string]any, result any) error {
	op, _ := stringValue(args, "op")
	if op != "set_item_text" {
		return unsupported(fmt.Sprintf("Windows 适配层只支持 set_item_text，收到 %s。", op))
	}
	if runs, ok := sliceValues(args["runs"]); ok && len(runs) > 0 {
		return unsupported("富文本样式编辑需要 RTF 转换，Windows 适配层尚未实现；纯文本编辑可用。")
	}
	id, err := uintArg(args, "id", "内容 ID 无效")
	if err != nil {
		return err
	}
	text, _ := args["text"].(string)
	if strings.TrimSpace(text) == "" {
		return invalidParams("文本内容不能为空")
	}
	if utf8.RuneCountInString(text) > 1_000_000 {
		return invalidParams("文本内容超过 1,000,000 字符上限")
	}
	request, err := json.Marshal(map[string]any{"op": "set_item_text", "id": id, "text": text})
	if err != nil {
		return err
	}
	if _, err := s.libraryJSON(ctx, "LibraryMutate", string(request)); err != nil {
		return err
	}
	return assign(result, map[string]any{"outcome": "applied", "action": "clipboard.updateRichText"})
}
