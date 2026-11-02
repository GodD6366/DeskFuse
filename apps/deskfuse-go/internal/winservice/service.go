package winservice

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/native"
)

// HostActions are the UI-host operations the pages issue (quit, show the main
// window). They belong to the desktop process, not the service, so the window
// owner injects them.
type HostActions interface {
	Quit()
	// ShowMain shows and focuses the main window, optionally navigating to a
	// page and settings category.
	ShowMain(page, category string)
	HidePanel()
	ShowShelf()
	HideShelf()
	// SetPanelShortcut registers the global hotkey that opens the clipboard
	// panel; an empty accelerator disables it.
	SetPanelShortcut(accelerator string) error
	// SuspendPanelShortcut unregisters/reregisters the panel hotkey while the
	// page records a shortcut, so the recorded keys are not swallowed.
	SuspendPanelShortcut(suspend bool) error
}

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

	// native owns the host-side clipboard, dialogs and shell. A nil value keeps
	// those actions unavailable instead of simulating them.
	native native.Actions
	host   HostActions
	// prefs holds the desktop client's own preferences; the service remains
	// authoritative for everything it reports.
	prefs *preferences
	// shelf is the desktop staging area; the service owns the transfers.
	shelf *shelfState
	// updates is the desktop client's own update status (GitHub Releases).
	updates *updateState
	// publishSeq numbers clipboard/file publishes within this session; the
	// service rejects stale sequences.
	publishSeq uint64
	// OnSnapshot runs after each successful snapshot decode with the shared
	// projection. The host uses it for notifications; it must be non-blocking
	// and must not call back into the Service synchronously.
	OnSnapshot func(map[string]any)
	// fetchRelease is the GitHub Releases probe; tests stub it to stay offline.
	fetchRelease func(ctx context.Context) (version, url string, err error)
}

// NewService returns an adapter over the given pipe dialer.
func NewService(dial Dialer) *Service {
	return &Service{
		client:  New(dial),
		prefs:   newPreferences(),
		shelf:   newShelfState(),
		updates: newUpdateState(),
	}
}

// currentVersion returns the service package version from the latest snapshot.
func (s *Service) currentVersion() string {
	latest, ok := s.latestSnapshot()
	if !ok {
		return ""
	}
	return latest.PackageVersion
}

// SetNative installs the host-side platform primitives. The window owner calls
// it once before the UI starts loading pages.
func (s *Service) SetNative(actions native.Actions) { s.native = actions }

// NativeActions returns the installed host primitives for host-side watchers.
func (s *Service) NativeActions() native.Actions { return s.native }

// BackgroundInvoke runs one app action outside a page request, for host-driven
// tasks such as the startup update check. It uses a detached context.
func (s *Service) BackgroundInvoke(action string, params map[string]any) error {
	return s.invoke(context.Background(), map[string]any{"action": action, "params": params}, nil)
}

// SetHost installs the window callbacks used by app and panel actions.
func (s *Service) SetHost(host HostActions) { s.host = host }

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
	shared := s.sharedSnapshot(value.raw)
	if s.OnSnapshot != nil {
		s.OnSnapshot(shared)
	}
	return assign(result, shared)
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
	shared := s.sharedSnapshot(value.raw)
	if s.OnSnapshot != nil {
		s.OnSnapshot(shared)
	}
	return assign(result, shared)
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
		return s.workflowRun(ctx, args, result)
	case "workflow.system_shortcuts":
		return s.workflowSystemShortcuts(ctx, result)
	// Host-side clipboard actions: the service is the content authority, the
	// desktop process owns the system clipboard and its native UI.
	case "clipboard.copy":
		return s.clipboardCopy(ctx, args, result, false)
	case "clipboard.paste":
		return s.clipboardCopy(ctx, args, result, true)
	case "clipboard.save":
		return s.clipboardSave(ctx, args, result)
	case "clipboard.reveal":
		return s.clipboardReveal(ctx, args, result)
	case "clipboard.openInPreview":
		return s.clipboardOpen(ctx, args, result, action)
	case "clipboard.edit":
		return s.clipboardOpen(ctx, args, result, action)
	// Window and host actions belong to the process that owns the windows.
	case "app.quit", "app.showMain", "native.openPage", "panel.hide",
		"clipboard.showInMain", "clipboard.showPanel", "clipboard.togglePanel":
		return s.hostAction(action, args, result)
	case "system.open_url":
		return s.openURL(args, result)
	case "support.report", "support.reportProblem":
		return s.reportProblem(args, result)
	case "support.copy_environment":
		return s.copyEnvironment(result)
	case "login_item.status":
		status, err := loginItemStatus()
		if err != nil {
			return nativeError(err)
		}
		return assign(result, map[string]any{"outcome": "applied", "value": status})
	case "login_item.set":
		enabled, _ := boolValue(args, "enabled")
		if err := setLoginItem(enabled); err != nil {
			return nativeError(err)
		}
		return assign(result, map[string]any{"outcome": "applied", "value": enabled})
	case "login_item.open_settings":
		if s.native == nil {
			return unsupported("此窗口没有接入 Windows 系统设置。")
		}
		if err := s.native.OpenSystemSettings("startupapps"); err != nil {
			return nativeError(err)
		}
		return assign(result, map[string]any{"outcome": "dispatched", "action": action})
	case "settings.set":
		return s.setPreference(args, result)
	// Staging area (shelf): host-side references, service-side transfers.
	case "shelf.add":
		return s.shelfAdd(args, result)
	case "shelf.remove":
		return s.shelfRemove(args, result)
	case "shelf.clear":
		return s.shelfClear(result)
	case "shelf.send":
		return s.shelfSend(ctx, args, result)
	case "shelf.show":
		return s.shelfShow(result)
	case "clipboard.stage":
		return s.shelfStage(ctx, args, result)
	// Updates: host-side check against GitHub Releases (no auto-installer).
	case "updates.check":
		return s.updatesCheck(ctx, result)
	case "download_and_install_update":
		return unsupported("Windows 版尚未提供自动安装；请在发布页下载新版安装包。")
	// Uninstall and diagnostics.
	case "uninstall.plan":
		return s.uninstallPlan(result)
	case "uninstall.perform":
		return s.uninstallPerform(args, result)
	case "support.export_diagnostics":
		return s.supportExportDiagnostics(result)
	// App picking and icons for the excluded-apps setting.
	case "clipboard.choose_app", "clipboard.chooseApp":
		return s.clipboardChooseApp(result)
	case "clipboard.appInfo", "clipboard.appinfo":
		return s.clipboardAppInfo(args, result)
	// Global shortcut recording and input takeover.
	case "clipboard.configure_shortcut":
		return s.clipboardConfigureShortcut(args, result)
	case "input.set_recording":
		return s.inputSetRecording(ctx, args, result)
	case "input.chooseController":
		return s.inputChooseController(ctx, args, result)
	case "input.wallpaper":
		return s.inputWallpaper(args, result)
	case "input.previewDisplayMove":
		return s.inputPreviewDisplayMove(ctx, args, result)
	// File transfer settings and transfer control.
	case "set_files":
		return s.setFiles(ctx, args, result)
	case "clear_cache":
		return s.clearCache(ctx, result)
	case "transfers.cancel":
		return s.transfersCancel(ctx, args, result)
	case "transfers.retry":
		return s.transfersRetry(ctx, args, result)
	case "transfers.send":
		return s.transfersSend(ctx, args, result)
	case "transfers.chooseFiles":
		return s.transfersChooseFiles(result)
	case "transfers.inspectPaths":
		return s.transfersInspectPaths(args, result)
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
	case "set_files", "clear_cache", "transfers.cancel", "transfers.retry", "transfers.send",
		"transfers.chooseFiles", "transfers.inspectPaths":
		// 由 invoke 在宿主层或服务层处理，不会到达这里；防御性保留。
		return nil, unsupported(fmt.Sprintf("%s 不是服务命令。", action))
	case "devices.setInputPath":
		return nil, unsupported("Windows 桌面 IPC 没有设置设备输入线路的命令。")
	case "clipboard.indexOCR":
		return nil, unsupported("Windows 客户端没有 OCR 索引能力。")
	case "updates.check", "download_and_install_update", "uninstall.plan", "uninstall.perform",
		"support.export_diagnostics", "shelf.show", "shelf.add", "shelf.remove", "shelf.clear", "shelf.send",
		"clipboard.choose_app", "clipboard.chooseApp", "clipboard.appInfo", "clipboard.configure_shortcut",
		"input.set_recording", "input.chooseController", "input.wallpaper":
		// 由宿主侧在 invoke 中处理，不会到达这里；防御性保留。
		return nil, unsupported(fmt.Sprintf("%s 不是服务命令。", action))
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
