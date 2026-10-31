package winservice

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
)

// Service adapts the Windows desktop service to the operations the shared UI
// calls. It keeps no business state: every page reads the service snapshot and
// every action is a command, so the service stays the single authority.
type Service struct {
	client *Client

	// A Snapshot opens a clipboard generation and discards buffered content, so
	// refreshes must reuse it through Poll. Serializing keeps one epoch and one
	// cursor authoritative for the whole window.
	mu           sync.Mutex
	sessionEpoch uint64
	sequence     uint64
	polling      bool
	// latest is the last decoded snapshot. Commands that need read-modify-write
	// settings or the trusted peer list work from it instead of inventing state.
	latest *snapshotWire
}

// NewService returns an adapter over the given pipe dialer.
func NewService(dial Dialer) *Service { return &Service{client: New(dial)} }

// Call implements the same contract as the macOS stdio bridge.
//
// Every op is either backed by a real service command or rejected with a typed
// unsupported_platform error. The adapter never reports success for a command
// the service does not implement.
func (s *Service) Call(ctx context.Context, op string, params any, result any) error {
	switch op {
	case "app.snapshot":
		return s.snapshot(ctx, result)
	case "hello":
		return s.hello(ctx, result)
	case "app.invoke":
		return s.invoke(ctx, params, result)
	case "library.query":
		return s.library(ctx, "LibraryQuery", params, result)
	case "library.mutate":
		return s.library(ctx, "LibraryMutate", params, result)
	case "clipboard.content":
		return s.content(ctx, params, result)
	case "clipboard.thumbnail":
		return s.thumbnail(ctx, params, result)
	default:
		if op == "cancel" {
			// The pipe is one request per connection; the only cancellation is
			// closing a stalled connection through the context deadline.
			return unsupported("Windows 命名管道没有独立的 cancel 操作；请求超时会关闭连接。")
		}
		return unsupported(fmt.Sprintf("未知操作 %s", op))
	}
}

func (s *Service) snapshot(ctx context.Context, result any) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	if !s.polling {
		return s.openSessionLocked(ctx, result)
	}
	envelope, err := s.client.Call(ctx, pollCommand{Epoch: s.sessionEpoch, AfterSequence: s.sequence})
	if err == nil {
		return s.applyPollLocked(envelope, result)
	}
	// A rejected Poll means this generation is no longer usable: the desktop was
	// locked, switched or expired. Reopen once and let a real failure surface.
	var remote *RemoteError
	if !errors.As(err, &remote) {
		return err
	}
	s.polling = false
	return s.openSessionLocked(ctx, result)
}

// ensureSession makes the service accept commands before the page has issued
// its first snapshot. Snapshot is the only command that registers the client
// session, so a command arriving first opens one explicitly.
func (s *Service) ensureSession(ctx context.Context) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.polling {
		return nil
	}
	return s.openSessionLocked(ctx, nil)
}

// openSession issues the Snapshot that registers this client session and opens
// a clipboard generation. The caller holds mu.
func (s *Service) openSessionLocked(ctx context.Context, result any) error {
	envelope, err := s.client.Call(ctx, "Snapshot")
	if err != nil {
		return err
	}
	raw, ok := envelope["Snapshot"]
	if !ok {
		return errors.New("服务返回了非快照结果")
	}
	value, err := decodeSnapshot(raw)
	if err != nil {
		return err
	}
	s.sessionEpoch = value.raw.SessionEpoch
	s.sequence = 0
	s.polling = true
	s.latest = &value.raw
	return assign(result, value.raw.Shared())
}

type pollCommand struct {
	Epoch         uint64
	AfterSequence uint64
}

func (p pollCommand) MarshalJSON() ([]byte, error) {
	return json.Marshal(map[string]any{"Poll": map[string]any{
		"session_epoch":  p.Epoch,
		"after_sequence": p.AfterSequence,
	}})
}

// applyPollLocked decodes one Poll reply. The caller holds mu.
func (s *Service) applyPollLocked(envelope map[string]json.RawMessage, result any) error {
	raw, ok := envelope["Poll"]
	if !ok {
		return errors.New("服务返回了非轮询结果")
	}
	var poll struct {
		Snapshot          json.RawMessage `json:"snapshot"`
		ClipboardSequence uint64          `json:"clipboard_sequence"`
	}
	if err := json.Unmarshal(raw, &poll); err != nil {
		return fmt.Errorf("服务轮询结果无效: %w", err)
	}
	value, err := decodeSnapshot(poll.Snapshot)
	if err != nil {
		return err
	}
	s.sessionEpoch = value.raw.SessionEpoch
	s.sequence = poll.ClipboardSequence
	s.latest = &value.raw
	return assign(result, value.raw.Shared())
}

func (s *Service) hello(ctx context.Context, result any) error {
	// Reuse the open desktop generation; a bare Snapshot would increment the
	// epoch and discard buffered clipboard content.
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	latest, ok := s.latestSnapshot()
	if !ok {
		return errors.New("尚未取得 Windows 服务快照")
	}
	return assign(result, map[string]any{
		"client":                "mygo-webview",
		"platform":              "windows",
		"protocol":              ProtocolVersion,
		"package_version":       latest.PackageVersion,
		"session_full_presence": latest.SessionFullPresence,
		"capabilities": map[string]any{
			"clipboard": latest.Capabilities.Clipboard,
			"files":     latest.Capabilities.Files,
			"input":     latest.Capabilities.Input,
			"workflows": latest.Capabilities.WorkflowExecution && latest.Capabilities.WorkflowSync,
			"updates":   latest.Capabilities.Updater,
		},
	})
}

func (s *Service) invoke(ctx context.Context, params any, result any) error {
	values, err := object(params)
	if err != nil {
		return err
	}
	action, _ := values["action"].(string)
	if action == "" {
		return invalidParams("缺少 action")
	}
	args, err := object(values["params"])
	if err != nil {
		return err
	}

	// Actions with a dedicated multi-command sequence or a cached snapshot
	// dependency. Everything here either succeeds through real service commands
	// or returns a typed error before opening a connection.
	switch action {
	case "clipboard.peers":
		return s.clipboardPeers(ctx, result)
	case "clipboard.sources":
		return s.clipboardSources(ctx, result)
	case "clipboard.share":
		return s.clipboardShare(ctx, args, result)
	case "clipboard.updateRichText":
		return s.updateRichText(ctx, args, result)
	case "workflow.list":
		return s.workflowList(ctx, result)
	case "workflow.save":
		return s.workflowSave(ctx, args, result)
	case "workflow.delete":
		return s.workflowDelete(ctx, args, result)
	case "workflow.restore_conflict":
		return s.workflowRestoreConflict(ctx, args, result)
	case "workflow.cancel":
		return s.workflowCancel(ctx, args, result)
	case "workflow.run":
		// RunWorkflow exists, but its contract requires the desktop to apply the
		// returned formats to the native clipboard and only then report success.
		// The Go UI has no Windows pasteboard adapter, so reporting success here
		// would be false. See docs/MYGO_WINDOWS_ADAPTER.md.
		return &RemoteError{
			Code:    "capability_missing",
			Message: "Windows 服务可执行工作流，但结果必须由原生桌面进程写入剪贴板；当前 Go UI 尚未接入 Windows 粘贴板写入，因此未执行工作流。",
		}
	case "workflow.system_shortcuts":
		return unsupported("Windows 桌面 IPC 没有系统快捷指令目录命令。")
	case "updates.check", "download_and_install_update":
		return unsupported("Windows 桌面 IPC 没有更新命令，服务能力中的 updater 始终为 false。")
	case "uninstall.plan", "uninstall.perform":
		return unsupported("Windows 桌面 IPC 没有卸载命令。")
	}

	if handled, err := s.invokeInput(ctx, action, args, result); handled {
		return err
	}

	command, err := commandFor(action, args)
	if err != nil {
		return err
	}
	return s.callQueued(ctx, command, action, result)
}

// callQueued sends one command that the service admits or rejects synchronously.
// The service's Queued reply means only that the command was accepted.
func (s *Service) callQueued(ctx context.Context, command any, action string, result any) error {
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callCommand(ctx, command, action, result)
}

// callCommand sends one command on the already-open session and maps the reply
// to the shared UI outcome envelope.
func (s *Service) callCommand(ctx context.Context, command any, action string, result any) error {
	envelope, err := s.client.Call(ctx, command)
	if err != nil {
		return err
	}
	outcome := "applied"
	if _, ok := envelope["Queued"]; ok {
		outcome = "queued"
	}
	return assign(result, map[string]any{"outcome": outcome, "action": action})
}

// commandFor maps one shared-UI action onto a Windows service command.
func commandFor(action string, args map[string]any) (any, error) {
	stringArg := func(name string) (string, error) {
		value, _ := args[name].(string)
		if value == "" {
			return "", invalidParams(fmt.Sprintf("%s 需要参数 %s", action, name))
		}
		return value, nil
	}
	switch action {
	case "devices.refresh", "refresh":
		return "RefreshPeers", nil
	case "devices.connect":
		peer, err := stringArg("peer_id")
		if err != nil {
			return nil, err
		}
		return tagged("Connect", map[string]any{"peer_id": peer}), nil
	case "devices.disconnect":
		peer, err := stringArg("peer_id")
		if err != nil {
			return nil, err
		}
		return tagged("Disconnect", map[string]any{"peer_id": peer}), nil
	case "devices.pair":
		if ip, _ := args["ip"].(string); ip != "" {
			return tagged("RequestPairingAt", map[string]any{"ipv4": ip}), nil
		}
		peer, err := stringArg("peer_id")
		if err != nil {
			return nil, err
		}
		return tagged("RequestPairing", map[string]any{"peer_id": peer}), nil
	case "devices.accept":
		id, err := stringArg("request_id")
		if err != nil {
			return nil, err
		}
		return tagged("AcceptPairing", map[string]any{"request_id": id}), nil
	case "devices.reject":
		id, err := stringArg("request_id")
		if err != nil {
			return nil, err
		}
		return tagged("RejectPairing", map[string]any{"request_id": id}), nil
	case "pairing.approve":
		id, err := stringArg("id")
		if err != nil {
			return nil, err
		}
		return tagged("AcceptPairing", map[string]any{"request_id": id}), nil
	case "pairing.reject":
		id, err := stringArg("id")
		if err != nil {
			return nil, err
		}
		return tagged("RejectPairing", map[string]any{"request_id": id}), nil
	case "devices.forget":
		peer, err := stringArg("peer_id")
		if err != nil {
			return nil, err
		}
		return tagged("Forget", map[string]any{"peer_id": peer}), nil
	case "devices.block":
		if confirmed, _ := args["confirm"].(bool); !confirmed {
			return nil, &RemoteError{Code: "confirmation_required", Message: "阻止设备需要 confirm:true"}
		}
		peer, err := stringArg("peer_id")
		if err != nil {
			return nil, err
		}
		return tagged("Revoke", map[string]any{"peer_id": peer}), nil
	case "devices.unblock":
		peer, err := stringArg("peer_id")
		if err != nil {
			return nil, err
		}
		return tagged("AllowPairing", map[string]any{"peer_id": peer}), nil
	case "feature.setEnabled":
		feature, _ := args["feature"].(string)
		enabled, _ := args["enabled"].(bool)
		switch feature {
		case "files":
			return tagged("SetFileTransferEnabled", map[string]any{"enabled": enabled}), nil
		case "clipboard":
			return nil, unsupported("Windows 桌面 IPC 没有剪贴板总开关命令；剪贴板同步可用性由服务能力决定。")
		default:
			return nil, unsupported(fmt.Sprintf("Windows 端尚未提供功能 %s", feature))
		}
	case "lanWeb.configure":
		enabled, _ := args["enabled"].(bool)
		return tagged("ConfigureLanWeb", map[string]any{"enabled": enabled}), nil
	case "set_files":
		return nil, unsupported("Windows 桌面 IPC 只提供文件传输总开关（feature.setEnabled files），没有阈值、缓存或下载目录设置命令。")
	case "clear_cache", "transfers.cancel", "transfers.retry", "transfers.send":
		return nil, unsupported(fmt.Sprintf("Windows 适配层尚未接入 %s 对应的文件传输命令。", action))
	case "devices.setInputPath":
		return nil, unsupported("Windows 桌面 IPC 没有设置设备输入线路的命令。")
	case "clipboard.copy", "clipboard.paste":
		return nil, unsupported("Windows 服务没有本地剪贴板写入或粘贴命令；原生粘贴板仍由 Windows 桌面客户端持有。")
	case "clipboard.save", "clipboard.edit", "clipboard.reveal", "clipboard.openInPreview", "clipboard.stage", "clipboard.cancelPreparation":
		return nil, unsupported(fmt.Sprintf("Windows 桌面 IPC 没有 %s 对应的原生文件或窗口命令。", action))
	case "clipboard.choose_app", "clipboard.chooseApp", "clipboard.indexOCR", "clipboard.configure_shortcut", "clipboard.appInfo", "clipboard.showInMain", "clipboard.showPanel", "clipboard.togglePanel":
		return nil, unsupported(fmt.Sprintf("Windows 桌面 IPC 没有 %s 对应的原生命令。", action))
	default:
		return nil, unsupported(fmt.Sprintf("Windows 端尚未实现操作 %s", action))
	}
}

// tagged builds serde's externally tagged representation:
// {"Variant": {fields}} for a struct variant.
func tagged(variant string, fields map[string]any) map[string]any {
	return map[string]any{variant: fields}
}

func unsupported(message string) error {
	return &RemoteError{Code: "unsupported_platform", Message: message}
}

func invalidParams(message string) error {
	return &RemoteError{Code: "invalid_params", Message: message}
}

// object accepts the map the MyGo binding decodes, or raw JSON from the bridge.
func object(value any) (map[string]any, error) {
	switch v := value.(type) {
	case nil:
		return map[string]any{}, nil
	case map[string]any:
		return v, nil
	case json.RawMessage:
		if len(v) == 0 {
			return map[string]any{}, nil
		}
		var out map[string]any
		if err := json.Unmarshal(v, &out); err != nil {
			return nil, fmt.Errorf("参数格式无效: %w", err)
		}
		return out, nil
	default:
		return nil, errors.New("参数格式无效")
	}
}

func assign(result any, value any) error {
	if result == nil {
		return nil
	}
	data, err := json.Marshal(value)
	if err != nil {
		return err
	}
	return json.Unmarshal(data, result)
}
