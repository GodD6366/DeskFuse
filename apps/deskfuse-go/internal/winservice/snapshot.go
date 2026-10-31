package winservice

import (
	"encoding/json"
	"fmt"
	"strings"
)

// snapshotWire mirrors the subset of the Windows service snapshot the shared UI
// reads. Fields the service adds later are ignored rather than failing the page.
type snapshotWire struct {
	PackageVersion string `json:"package_version"`
	SessionEpoch   uint64 `json:"session_epoch"`
	LocalDeviceID  string `json:"local_device_id"`
	LocalName      string `json:"local_name"`
	NetworkRunning bool   `json:"network_running"`
	ControlPort    uint16 `json:"control_port"`
	Capabilities   struct {
		Clipboard         bool `json:"clipboard"`
		Files             bool `json:"files"`
		Input             bool `json:"input"`
		ElevatedIn        bool `json:"elevated_input"`
		SecureInput       bool `json:"secure_desktop_input"`
		LoginInput        bool `json:"login_input"`
		Updater           bool `json:"updater"`
		WorkflowExecution bool `json:"workflow_execution"`
		WorkflowSync      bool `json:"workflow_sync"`
	} `json:"capabilities"`
	Peers         []peerWire     `json:"peers"`
	Pending       []pendingWire  `json:"pending"`
	FileTransfers []transferWire `json:"file_transfers"`
	FileCache     struct {
		MaxBytes  uint64 `json:"max_bytes"`
		UsedBytes uint64 `json:"used_bytes"`
	} `json:"file_cache"`
	FileSettings  fileSettingsWire `json:"file_settings"`
	PairingStatus *string          `json:"pairing_status"`
	LastError     *string          `json:"last_error"`
	// SessionFullPresence is false while this client session is relaxed
	// (locked console or remote protocol); such sessions cannot change policy.
	SessionFullPresence bool      `json:"session_full_presence"`
	Input               inputWire `json:"input"`
	LanWeb              struct {
		Enabled bool   `json:"enabled"`
		Running bool   `json:"running"`
		Port    uint16 `json:"port"`
		URL     string `json:"url"`
	} `json:"lan_web"`
}

type peerWire struct {
	PeerID         string            `json:"peer_id"`
	DisplayName    string            `json:"display_name"`
	Trust          string            `json:"trust"`
	Connection     string            `json:"connection"`
	DiscoveredOnly bool              `json:"discovered_only"`
	SelectedRoute  *string           `json:"selected_route"`
	Routes         []json.RawMessage `json:"routes"`
}

type pendingWire struct {
	RequestID   string   `json:"request_id"`
	PeerID      string   `json:"peer_id"`
	DisplayName string   `json:"display_name"`
	Incoming    bool     `json:"incoming"`
	ExpiresAt   uint64   `json:"expires_at_unix_ms"`
	OwnerName   string   `json:"owner_name"`
	MemberNames []string `json:"member_names"`
}

type transferWire struct {
	TransferID       string   `json:"transfer_id"`
	PeerID           string   `json:"peer_id"`
	Direction        string   `json:"direction"`
	Names            []string `json:"names"`
	TotalBytes       uint64   `json:"total_bytes"`
	TransferredBytes uint64   `json:"transferred_bytes"`
	State            string   `json:"state"`
	Detail           *string  `json:"detail"`
	StagedPaths      []string `json:"staged_paths"`
}

type fileSettingsWire struct {
	Enabled           bool   `json:"enabled"`
	Prefetch          bool   `json:"small_file_prefetch_enabled"`
	Threshold         uint64 `json:"small_file_threshold_bytes"`
	CacheMaxBytes     uint64 `json:"cache_max_bytes"`
	DownloadDirectory string `json:"download_directory"`
}

// inputWire is the service's InputState projection. The service owns the input
// runtime; this adapter only renames and derives page fields from reported
// values. Local display geometry is not part of the wire snapshot, so only
// remote topologies are listed; nothing is synthesized for the local desktop.
type inputWire struct {
	HelperConnected  bool                         `json:"helper_connected"`
	Managed          bool                         `json:"managed"`
	Status           string                       `json:"status"`
	Reason           *string                      `json:"reason"`
	ControllerID     *string                      `json:"controller_id"`
	DisplayCount     uint32                       `json:"display_count"`
	LocalDeviceID    string                       `json:"local_device_id"`
	Settings         *inputSettingsWire           `json:"settings"`
	RemoteTopologies []remoteTopologyWire         `json:"remote_topologies"`
	RemoteWallpapers map[string]map[string]string `json:"remote_wallpapers"`
	Links            []inputLinkWire              `json:"links"`
	Environment      inputEnvironmentWire         `json:"environment"`
	Quality          *inputQualityWire            `json:"quality"`
}

type remoteTopologyWire struct {
	PeerID   string `json:"peer_id"`
	Topology struct {
		Generation uint64                  `json:"generation"`
		Displays   []displayDescriptorWire `json:"displays"`
	} `json:"topology"`
}

type displayDescriptorWire struct {
	ID              string  `json:"id"`
	FrameX          float64 `json:"frame_x"`
	FrameY          float64 `json:"frame_y"`
	Width           uint32  `json:"width"`
	Height          uint32  `json:"height"`
	ScaleFactor     float64 `json:"scale_factor"`
	RotationDegrees uint16  `json:"rotation_degrees"`
	IsPrimary       bool    `json:"is_primary"`
}

type inputLinkWire struct {
	PeerID string `json:"peer_id"`
	Ready  bool   `json:"ready"`
}

type inputEnvironmentWire struct {
	SSID         *string           `json:"ssid"`
	Message      string            `json:"message"`
	Conflict     bool              `json:"conflict"`
	ObservedSSID *string           `json:"observed_ssid"`
	PeerSSIDs    []json.RawMessage `json:"peer_ssids"`
	Rules        []json.RawMessage `json:"rules"`
}

type inputQualityWire struct {
	Grade            string              `json:"grade"`
	Mode             string              `json:"mode"`
	Fallback         *string             `json:"fallback"`
	RTTP50Micros     uint64              `json:"rtt_p50_micros"`
	RTTP95Micros     uint64              `json:"rtt_p95_micros"`
	RTTP99Micros     uint64              `json:"rtt_p99_micros"`
	E2EP50Micros     uint64              `json:"e2e_p50_micros"`
	E2EP95Micros     uint64              `json:"e2e_p95_micros"`
	Hitches          uint32              `json:"hitches"`
	LossRate         float64             `json:"loss_rate"`
	ReplaceRate      float64             `json:"replace_rate"`
	QueueDepth       uint32              `json:"queue_depth"`
	InjectMicros     uint64              `json:"inject_micros"`
	PresentSupported bool                `json:"present_supported"`
	Samples          []qualitySampleWire `json:"samples"`
	Suggestions      []json.RawMessage   `json:"suggestions"`
	Empty            bool                `json:"empty"`
}

type qualitySampleWire struct {
	Second uint32  `json:"second"`
	RTTMS  float64 `json:"rtt_ms"`
	E2EMS  float64 `json:"e2e_ms"`
}

// snapshot is one decoded Windows service snapshot.
type snapshot struct {
	raw snapshotWire
}

func decodeSnapshot(payload json.RawMessage) (*snapshot, error) {
	var value snapshotWire
	if err := json.Unmarshal(payload, &value); err != nil {
		return nil, fmt.Errorf("服务快照不是有效 JSON: %w", err)
	}
	return &snapshot{raw: value}, nil
}

// Shared renders the snapshot in the shape the shared UI already consumes.
// Platform-specific capability flags come from the service and are never
// inferred from the host name.
func (s snapshotWire) Shared() map[string]any {
	caps := s.Capabilities
	devices := make([]map[string]any, 0, len(s.Peers))
	for _, peer := range s.Peers {
		devices = append(devices, peer.view())
	}
	pairing := make([]map[string]any, 0, len(s.Pending))
	for _, request := range s.Pending {
		pairing = append(pairing, map[string]any{
			"id":         request.RequestID,
			"request_id": request.RequestID,
			"peer_id":    request.PeerID,
			"name":       request.DisplayName,
			"incoming":   request.Incoming,
			"expires_at": request.ExpiresAt,
		})
	}
	transfers := make([]map[string]any, 0, len(s.FileTransfers))
	for _, transfer := range s.FileTransfers {
		transfers = append(transfers, transfer.view())
	}

	local := map[string]any{
		"id":              s.LocalDeviceID,
		"name":            s.LocalName,
		"runtime_status":  runningLabel(s.NetworkRunning),
		"network_running": s.NetworkRunning,
		"control_port":    s.ControlPort,
		"package_version": s.PackageVersion,
		"session_epoch":   s.SessionEpoch,
		"pairing_message": deref(s.PairingStatus),
	}
	if s.LastError != nil {
		local["error"] = *s.LastError
	}

	workflowsEnabled := caps.WorkflowExecution && caps.WorkflowSync
	return map[string]any{
		"platform":     "windows",
		"engine_ready": true,
		"revision":     s.SessionEpoch,
		"local":        local,
		"local_id":     s.LocalDeviceID,
		"local_name":   s.LocalName,
		"devices":      devices,
		"pairing":      pairing,
		"transfers":    transfers,
		"files": map[string]any{
			"enabled":            s.FileSettings.Enabled,
			"prefetch":           s.FileSettings.Prefetch,
			"threshold":          s.FileSettings.Threshold,
			"maximum":            s.FileSettings.CacheMaxBytes,
			"download_directory": s.FileSettings.DownloadDirectory,
		},
		"cache": map[string]any{
			"max_bytes":  s.FileCache.MaxBytes,
			"used_bytes": s.FileCache.UsedBytes,
		},
		"features": map[string]any{
			"clipboard":           caps.Clipboard,
			"files":               caps.Files,
			"input":               caps.Input,
			"updates":             caps.Updater,
			"mouse":               false,
			"microphone":          false,
			"remote_applications": false,
		},
		"capabilities": map[string]any{
			"clipboard":            caps.Clipboard,
			"files":                caps.Files,
			"input":                caps.Input,
			"elevated_input":       caps.ElevatedIn,
			"secure_desktop_input": caps.SecureInput,
			"login_input":          caps.LoginInput,
			"updater":              caps.Updater,
			"workflow_execution":   caps.WorkflowExecution,
			"workflow_sync":        caps.WorkflowSync,
		},
		"input":       s.Input.view(s.Peers),
		"lan_web":     map[string]any{"enabled": s.LanWeb.Enabled, "running": s.LanWeb.Running, "port": s.LanWeb.Port, "url": s.LanWeb.URL},
		"permissions": []any{},
		"app": map[string]any{
			"workflows_enabled": workflowsEnabled,
		},
	}
}

// view projects the service input state onto the fields the shared page reads.
// Values that the protocol does not carry (local display geometry, pointer
// lock, recording, remote-application input) are omitted rather than invented.
func (in inputWire) view(peers []peerWire) map[string]any {
	names := make(map[string]string, len(peers))
	for _, peer := range peers {
		names[peer.PeerID] = displayName(peer.DisplayName, peer.PeerID)
	}
	ready := make(map[string]bool, len(in.Links))
	for _, link := range in.Links {
		ready[link.PeerID] = link.Ready
	}

	displays := make([]map[string]any, 0)
	for _, topology := range in.RemoteTopologies {
		name := names[topology.PeerID]
		if name == "" {
			name = topology.PeerID
		}
		wallpapers := in.RemoteWallpapers[topology.PeerID]
		for _, display := range topology.Topology.Displays {
			wallpaper := wallpapers[display.ID]
			displays = append(displays, map[string]any{
				"id":             display.ID,
				"device_id":      topology.PeerID,
				"name":           name,
				"x":              display.FrameX,
				"y":              display.FrameY,
				"width":          display.Width,
				"height":         display.Height,
				"scale":          display.ScaleFactor,
				"rotation":       display.RotationDegrees,
				"local":          false,
				"ready":          ready[topology.PeerID],
				"has_wallpaper":  wallpaper != "",
				"wallpaper_jpeg": nullableString(wallpaper),
			})
		}
	}

	view := map[string]any{
		"summary":                         in.summary(),
		"managed":                         in.Managed,
		"managed_paused":                  in.Managed && in.Status == "PausedByUser",
		"can_configure":                   !in.Managed && in.Settings != nil && in.Settings.Role == "Controller",
		"can_operate_remote_applications": false,
		"displays":                        displays,
		"environment_message":             in.Environment.Message,
		"environment_conflict":            in.Environment.Conflict,
		"ssid":                            nullablePointer(in.Environment.SSID),
		"observed_ssid":                   nullablePointer(in.Environment.ObservedSSID),
		"peer_ssids":                      nonNilRaw(in.Environment.PeerSSIDs),
		"rules":                           nonNilRaw(in.Environment.Rules),
	}
	if in.Settings != nil {
		view["settings"] = in.Settings.view()
	}
	if in.Reason != nil && (!in.HelperConnected || in.Status == "Unavailable") {
		view["error"] = *in.Reason
	}
	if in.Quality != nil {
		view["quality"] = in.Quality.view()
	}
	return view
}

func (in inputWire) summary() string {
	switch {
	case !in.HelperConnected:
		return "Windows 输入辅助进程未连接，键鼠共享暂不可用。"
	case in.Managed && in.Status == "PausedByUser":
		return "远程控制已暂停。"
	case in.Managed:
		return "这台电脑正被远程电脑控制。"
	case in.Settings != nil && in.Settings.Role == "Controlled":
		return "此电脑受远程主控控制。"
	case in.Settings != nil && in.Settings.Role == "Controller":
		return "此电脑作为主控。"
	default:
		return "键鼠共享尚未启用。"
	}
}

// view exposes the quality metrics whose units and meaning match the macOS
// projection. Per-second hitch attribution and slow-injection counts are not
// carried by the service report and stay absent.
func (q inputQualityWire) view() map[string]any {
	samples := make([]map[string]any, 0, len(q.Samples))
	maxRTT := 0.0
	hasMax := false
	for _, sample := range q.Samples {
		samples = append(samples, map[string]any{
			"second": sample.Second,
			"rtt_ms": sample.RTTMS,
		})
		if sample.RTTMS > maxRTT {
			maxRTT = sample.RTTMS
			hasMax = true
		}
	}
	view := map[string]any{
		"grade":      q.Grade,
		"rtt_p50_ms": microsToMillis(q.RTTP50Micros),
		"rtt_p95_ms": microsToMillis(q.RTTP95Micros),
		// The service report covers a 60-second window, so its hitch count is
		// already a per-minute rate.
		"hitches_per_minute": float64(q.Hitches),
		"samples":            samples,
		"suggestions":        nonNilRaw(q.Suggestions),
	}
	if hasMax {
		view["rtt_max_ms"] = maxRTT
	}
	return view
}

func microsToMillis(micros uint64) float64 { return float64(micros) / 1000 }

func nullableString(value string) any {
	if value == "" {
		return nil
	}
	return value
}

func nullablePointer(value *string) any {
	if value == nil {
		return nil
	}
	return *value
}

func (p peerWire) view() map[string]any {
	trusted := p.Trust == "trusted"
	blocked := p.Trust == "blocked"
	connected := p.Connection == "connected"
	view := map[string]any{
		"id":                 p.PeerID,
		"peer_id":            p.PeerID,
		"name":               displayName(p.DisplayName, p.PeerID),
		"trusted":            trusted,
		"blocked":            blocked,
		"connected":          connected,
		"member":             !p.DiscoveredOnly,
		"discovered_only":    p.DiscoveredOnly,
		"can_trigger_update": false,
		"detail":             peerDetail(p.Trust, p.Connection),
		"link_status":        linkStatus(p.Connection),
		"routes":             p.Routes,
	}
	if p.SelectedRoute != nil {
		view["endpoint"] = *p.SelectedRoute
		view["selected_route"] = *p.SelectedRoute
	}
	return view
}

func (t transferWire) view() map[string]any {
	phase := transferPhase(t.State)
	view := map[string]any{
		"id":                t.TransferID,
		"transfer_id":       t.TransferID,
		"peer_id":           t.PeerID,
		"name":              transferName(t.Names),
		"names":             t.Names,
		"phase":             phase,
		"state":             t.State,
		"outgoing":          t.Direction == "send" || t.Direction == "sending" || t.Direction == "outgoing",
		"transferred":       t.TransferredBytes,
		"transferred_bytes": t.TransferredBytes,
		"total":             t.TotalBytes,
		"total_bytes":       t.TotalBytes,
		"can_retry":         phase == "failed" || phase == "cancelled",
		"staged_paths":      t.StagedPaths,
	}
	if t.Detail != nil {
		view["detail"] = *t.Detail
	} else {
		view["detail"] = ""
	}
	return view
}

// transferPhase maps the service's transfer states onto the phases the shared
// transfer view renders. Unknown states stay visible instead of showing as done.
func transferPhase(state string) string {
	switch strings.ToLower(state) {
	case "offered":
		return "queued"
	case "sending", "requested", "requesting":
		return "sending"
	case "receiving", "staging":
		return "receiving"
	case "verifying":
		return "verifying"
	case "staged", "delivering":
		return "delivering"
	case "completed", "sent", "applied":
		return "completed"
	case "cancelled", "canceled":
		return "cancelled"
	case "failed", "expired":
		return "failed"
	default:
		return strings.ToLower(state)
	}
}

func peerDetail(trust, connection string) string {
	switch {
	case trust == "blocked":
		return "已阻止"
	case trust == "pending", trust == "pairing":
		return "等待确认"
	case connection == "connected":
		return "已连接"
	case connection == "connecting":
		return "正在连接"
	case connection == "backoff":
		return "等待重连"
	case connection == "failed":
		return "连接失败"
	case trust == "trusted":
		return "未连接"
	default:
		return "未配对"
	}
}

func linkStatus(connection string) string {
	switch connection {
	case "connected":
		return ""
	case "connecting":
		return "正在连接"
	case "backoff":
		return "等待重连"
	case "failed":
		return "连接失败"
	default:
		return "未连接"
	}
}

func displayName(name, id string) string {
	if name != "" {
		return name
	}
	if len(id) > 8 {
		return id[:8]
	}
	return id
}

func transferName(names []string) string {
	switch len(names) {
	case 0:
		return "文件"
	case 1:
		return names[0]
	default:
		return fmt.Sprintf("%s 等 %d 个文件", names[0], len(names))
	}
}

func runningLabel(running bool) string {
	if running {
		return "running"
	}
	return "stopped"
}

func deref(value *string) string {
	if value == nil {
		return ""
	}
	return *value
}
