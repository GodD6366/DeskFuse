package winservice

import (
	"context"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/native"
)

// fakePipe answers one request per connection, as the Windows service does,
// and reports the acknowledgement byte the client must send.
type fakePipe struct {
	commands chan map[string]any
	acks     chan byte
	reply    func(command map[string]any) map[string]any
}

func newFakePipe(t *testing.T, reply func(command map[string]any) map[string]any) (*fakePipe, Dialer) {
	t.Helper()
	pipe := &fakePipe{commands: make(chan map[string]any, 16), acks: make(chan byte, 16), reply: reply}
	dial := func(ctx context.Context) (io.ReadWriteCloser, error) {
		client, server := net.Pipe()
		go serveFake(server, pipe, t)
		return client, nil
	}
	return pipe, dial
}

// newDesktopPipe serves Snapshot/Poll from the provided snapshot and delegates
// every other command to reply.
func newDesktopPipe(t *testing.T, snapshot string, reply func(command map[string]any) map[string]any) (*fakePipe, Dialer) {
	t.Helper()
	return newFakePipe(t, func(command map[string]any) map[string]any {
		if _, ok := command["Snapshot"]; ok {
			return map[string]any{"Snapshot": json.RawMessage(snapshot)}
		}
		if _, ok := command["Poll"]; ok {
			return map[string]any{"Poll": map[string]any{
				"snapshot":           json.RawMessage(snapshot),
				"clipboard_sequence": 0,
			}}
		}
		return reply(command)
	})
}

func serveFake(server net.Conn, pipe *fakePipe, t *testing.T) {
	defer server.Close()
	_ = server.SetDeadline(time.Now().Add(5 * time.Second))
	var header [4]byte
	if _, err := io.ReadFull(server, header[:]); err != nil {
		return
	}
	payload := make([]byte, binary.LittleEndian.Uint32(header[:]))
	if _, err := io.ReadFull(server, payload); err != nil {
		t.Errorf("读取请求失败: %v", err)
		return
	}
	var raw map[string]any
	if err := json.Unmarshal(payload, &raw); err != nil {
		t.Errorf("请求不是有效 JSON: %v", err)
		return
	}
	command, err := normalizeCommand(raw)
	if err != nil {
		t.Errorf("%v", err)
		return
	}
	var requestID uint64
	if value, ok := raw["request_id"].(float64); ok {
		requestID = uint64(value)
	}
	pipe.commands <- command

	reply := map[string]any{"version": 2, "request_id": requestID, "result": pipe.reply(command)}
	data, err := json.Marshal(reply)
	if err != nil {
		t.Errorf("编码响应失败: %v", err)
		return
	}
	binary.LittleEndian.PutUint32(header[:], uint32(len(data)))
	if _, err := server.Write(header[:]); err != nil {
		t.Errorf("写入响应头失败: %v", err)
		return
	}
	if _, err := server.Write(data); err != nil {
		t.Errorf("写入响应失败: %v", err)
		return
	}
	var ack [1]byte
	if _, err := io.ReadFull(server, ack[:]); err == nil {
		pipe.acks <- ack[0]
	}
}

// normalizeCommand turns unit commands ("Snapshot") into a map so tests can
// assert on both forms uniformly.
func normalizeCommand(raw map[string]any) (map[string]any, error) {
	switch command := raw["command"].(type) {
	case map[string]any:
		return command, nil
	case string:
		return map[string]any{command: map[string]any{}}, nil
	default:
		return nil, errors.New("请求缺少 command")
	}
}

func mustCall(t *testing.T, service *Service, op string, params any) map[string]any {
	t.Helper()
	var result json.RawMessage
	if err := service.Call(context.Background(), op, params, &result); err != nil {
		t.Fatalf("%s 失败: %v", op, err)
	}
	var value map[string]any
	if err := json.Unmarshal(result, &value); err != nil {
		t.Fatalf("%s 返回无效 JSON: %v", op, err)
	}
	return value
}

func mustInvoke(t *testing.T, service *Service, action string, params map[string]any) map[string]any {
	t.Helper()
	return mustCall(t, service, "app.invoke", map[string]any{"action": action, "params": params})
}

func openSession(t *testing.T, service *Service, pipe *fakePipe) {
	t.Helper()
	mustCall(t, service, "app.snapshot", nil)
	select {
	case command := <-pipe.commands:
		if _, ok := command["Snapshot"]; !ok {
			t.Fatalf("期望 Snapshot 命令，实际 %v", command)
		}
	case <-time.After(time.Second):
		t.Fatal("服务没有收到 Snapshot")
	}
}

func nextCommand(t *testing.T, pipe *fakePipe) map[string]any {
	t.Helper()
	select {
	case command := <-pipe.commands:
		return command
	case <-time.After(time.Second):
		t.Fatal("服务没有收到命令")
		return nil
	}
}

func queuedReply(map[string]any) map[string]any {
	return map[string]any{"Queued": map[string]any{"message": "queued"}}
}

const sampleSnapshot = `{
  "package_version": "0.9.0",
  "session_epoch": 4,
  "client_session_id": 1,
  "sequence": 9,
  "local_device_id": "win-local",
  "local_name": "DESKTOP-AB12CD",
  "network_running": true,
  "control_port": 42424,
  "capabilities": {"clipboard": true, "files": true, "input": true, "elevated_input": true,
                   "secure_desktop_input": false, "login_input": false, "updater": false,
                   "workflow_execution": true, "workflow_sync": true},
  "peers": [
    {"peer_id": "mac-1", "display_name": "Studio", "trust": "trusted", "connection": "connected",
     "discovered_only": false, "selected_route": "lan", "routes": [{"endpoint": "10.0.0.5"}]},
    {"peer_id": "mac-2", "display_name": "Library", "trust": "blocked", "connection": "disconnected",
     "discovered_only": false, "selected_route": null, "routes": []}
  ],
  "pending": [{"request_id": "req-1", "peer_id": "mac-3", "display_name": "Laptop",
               "incoming": true, "expires_at_unix_ms": 1700000000000,
               "owner_name": "Laptop", "member_names": ["Laptop"]}],
  "file_transfers": [
    {"transfer_id": "t-1", "peer_id": "mac-1", "direction": "recv", "names": ["report.pdf"],
     "total_bytes": 2048, "transferred_bytes": 1024, "state": "staging", "detail": null, "staged_paths": []}
  ],
  "file_cache": {"max_bytes": 1073741824, "used_bytes": 4096},
  "file_settings": {"enabled": true, "small_file_prefetch_enabled": true, "small_file_threshold_bytes": 4096,
                    "cache_max_bytes": 1073741824, "download_directory": "C:\\Users\\me\\Downloads"},
  "pairing_status": null,
  "last_error": "会话已锁定",
  "session_full_presence": true,
  "input": {"helper_connected": true, "managed": false, "status": "standby", "display_count": 1},
  "lan_web": {"enabled": false, "running": false, "port": 0, "url": ""}
}`

const sampleInputSnapshot = `{
  "package_version": "0.9.0",
  "session_epoch": 7,
  "local_device_id": "win-local",
  "local_name": "DESKTOP-AB12CD",
  "network_running": true,
  "control_port": 42424,
  "capabilities": {"clipboard": true, "files": true, "input": true, "updater": false,
                   "workflow_execution": true, "workflow_sync": true},
  "peers": [
    {"peer_id": "mac-1", "display_name": "Studio", "trust": "trusted", "connection": "connected",
     "discovered_only": false, "selected_route": "lan", "routes": []}
  ],
  "pending": [],
  "file_transfers": [],
  "file_cache": {"max_bytes": 0, "used_bytes": 0},
  "file_settings": {"enabled": true, "small_file_prefetch_enabled": false,
                    "small_file_threshold_bytes": 0, "cache_max_bytes": 0, "download_directory": ""},
  "pairing_status": null,
  "last_error": null,
  "input": {
    "helper_connected": true,
    "managed": false,
    "status": "standby",
    "reason": null,
    "display_count": 1,
    "local_device_id": "win-local",
    "settings": {
      "feature_enabled": true,
      "shortcuts": {
        "left": {"key_code": 123, "modifiers": 1835008},
        "right": {"key_code": 124, "modifiers": 1835008},
        "up": {"key_code": 126, "modifiers": 1835008},
        "down": {"key_code": 125, "modifiers": 1835008},
        "lock": {"key_code": 37, "modifiers": 1835008},
        "emergency": {"key_code": 53, "modifiers": 1835008}
      },
      "disabled_corners": 0,
      "corner_size": 16,
      "enabled": true,
      "role": "Controller",
      "authorized_controller": null,
      "participants": ["mac-1"],
      "switch_mode": "Instant",
      "placements": [{"display": {"device_id": "mac-1", "display_id": "d1"}, "x": 1920.0, "y": 0.0}],
      "layout_profiles": [{"device_id": "mac-1", "local_topology": "l", "device_topology": "r",
                           "placements": [], "attachment": null}],
      "input_paths": []
    },
    "remote_topologies": [
      {"peer_id": "mac-1", "topology": {"generation": 1, "displays": [
        {"id": "d1", "frame_x": 0.0, "frame_y": 0.0, "width": 1920, "height": 1080,
         "scale_factor": 2.0, "rotation_degrees": 0, "is_primary": true}
      ]}}
    ],
    "remote_wallpapers": {"mac-1": {"d1": "aGk="}},
    "links": [{"peer_id": "mac-1", "ready": true}],
    "environment": {"ssid": "home", "message": "", "conflict": false,
                    "observed_ssid": "home", "peer_ssids": [], "rules": []},
    "quality": {"grade": "smooth", "mode": "datagram", "fallback": null,
                "rtt_p50_micros": 4000, "rtt_p95_micros": 9000, "rtt_p99_micros": 12000,
                "e2e_p50_micros": 5000, "e2e_p95_micros": 10000, "hitches": 2,
                "loss_rate": 0.0, "replace_rate": 0.0, "queue_depth": 1, "inject_micros": 500,
                "present_supported": true, "empty": false,
                "samples": [{"second": 1, "rtt_ms": 4.0, "e2e_ms": 5.0},
                            {"second": 2, "rtt_ms": 9.0, "e2e_ms": 10.0}],
                "suggestions": []}
  },
  "lan_web": {"enabled": false, "running": false, "port": 0, "url": ""}
}`

func TestHelloReusesOpenSession(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, queuedReply)
	service := NewService(dial)
	openSession(t, service, pipe)

	value := mustCall(t, service, "hello", nil)
	if value["package_version"] != "0.9.0" || value["session_full_presence"] != true {
		t.Fatalf("hello 结果错误: %v", value)
	}
	capabilities := value["capabilities"].(map[string]any)
	if capabilities["workflows"] != true || capabilities["updates"] != false {
		t.Fatalf("hello 能力错误: %v", capabilities)
	}
	select {
	case command := <-pipe.commands:
		t.Fatalf("hello 不应重新打开 Snapshot 代次: %v", command)
	case <-time.After(50 * time.Millisecond):
	}
}

func TestSnapshotTranslatesWindowsServiceState(t *testing.T) {
	pipe, dial := newFakePipe(t, func(map[string]any) map[string]any {
		return map[string]any{"Snapshot": json.RawMessage(sampleSnapshot)}
	})
	service := NewService(dial)

	value := mustCall(t, service, "app.snapshot", nil)

	select {
	case command := <-pipe.commands:
		if _, ok := command["Snapshot"]; !ok {
			t.Fatalf("期望 Snapshot 命令，实际 %v", command)
		}
	case <-time.After(time.Second):
		t.Fatal("服务没有收到命令")
	}

	if value["platform"] != "windows" || value["engine_ready"] != true {
		t.Fatalf("平台或就绪标记错误: %v", value["platform"])
	}
	local, _ := value["local"].(map[string]any)
	if local["id"] != "win-local" || local["name"] != "DESKTOP-AB12CD" || local["error"] != "会话已锁定" {
		t.Fatalf("本机信息映射错误: %v", local)
	}
	devices, _ := value["devices"].([]any)
	if len(devices) != 2 {
		t.Fatalf("设备数量错误: %d", len(devices))
	}
	studio := devices[0].(map[string]any)
	if studio["trusted"] != true || studio["connected"] != true || studio["blocked"] != false || studio["endpoint"] != "lan" {
		t.Fatalf("已连接设备映射错误: %v", studio)
	}
	blocked := devices[1].(map[string]any)
	if blocked["blocked"] != true || blocked["connected"] != false || blocked["link_status"] != "未连接" {
		t.Fatalf("已阻止设备映射错误: %v", blocked)
	}
	pairing := value["pairing"].([]any)
	if pairing[0].(map[string]any)["id"] != "req-1" {
		t.Fatalf("配对请求映射错误: %v", pairing)
	}
	transfer := value["transfers"].([]any)[0].(map[string]any)
	if transfer["phase"] != "receiving" || transfer["transferred"] != float64(1024) || transfer["can_retry"] != false {
		t.Fatalf("传输映射错误: %v", transfer)
	}
	features := value["features"].(map[string]any)
	if features["clipboard"] != true || features["updates"] != false || features["microphone"] != false {
		t.Fatalf("功能标记不能从主机推断: %v", features)
	}
	capabilities := value["capabilities"].(map[string]any)
	if capabilities["microphone"] != false || capabilities["microphone_reason"] == "" {
		t.Fatalf("Windows 麦克风能力必须显式标记不可用及原因: %v", capabilities)
	}
	if value["clipboard"].(map[string]any)["automatic_ocr"] != false {
		t.Fatalf("Windows Go bridge 未接入 OCR 时必须关闭自动 OCR: %v", value["clipboard"])
	}
	if value["files"].(map[string]any)["download_directory"] != `C:\Users\me\Downloads` {
		t.Fatalf("文件设置映射错误: %v", value["files"])
	}
	app := value["app"].(map[string]any)
	if app["workflows_enabled"] != true {
		t.Fatalf("工作流能力必须来自服务能力: %v", app)
	}
}

func TestSnapshotProjectsServiceInputState(t *testing.T) {
	_, dial := newDesktopPipe(t, sampleInputSnapshot, queuedReply)
	service := NewService(dial)

	value := mustCall(t, service, "app.snapshot", nil)
	input := value["input"].(map[string]any)
	if input["can_configure"] != true || input["managed"] != false || input["summary"] == "" {
		t.Fatalf("输入状态映射错误: %v", input)
	}
	settings := input["settings"].(map[string]any)
	if settings["controller"] != true || settings["enabled"] != true || settings["feature_enabled"] != true {
		t.Fatalf("输入设置映射错误: %v", settings)
	}
	shortcuts := settings["shortcuts"].([]any)
	if len(shortcuts) != 6 || shortcuts[5].(map[string]any)["key_code"] != float64(53) {
		t.Fatalf("快捷键顺序必须与 macOS 桥接一致: %v", shortcuts)
	}
	displays := input["displays"].([]any)
	if len(displays) != 1 {
		t.Fatalf("远端屏幕映射错误: %v", displays)
	}
	display := displays[0].(map[string]any)
	if display["device_id"] != "mac-1" || display["local"] != false || display["wallpaper_jpeg"] != "aGk=" {
		t.Fatalf("屏幕字段映射错误: %v", display)
	}
	quality := input["quality"].(map[string]any)
	if quality["rtt_p50_ms"] != float64(4) || quality["hitches_per_minute"] != float64(2) {
		t.Fatalf("质量指标换算错误: %v", quality)
	}
}

func TestInvokeTranslatesPairingActions(t *testing.T) {
	cases := []struct {
		action  string
		params  map[string]any
		variant string
		field   string
		value   any
	}{
		{"devices.refresh", nil, "RefreshPeers", "", nil},
		{"devices.pair", map[string]any{"ip": "192.168.1.30"}, "RequestPairingAt", "ipv4", "192.168.1.30"},
		{"devices.pair", map[string]any{"peer_id": "mac-1"}, "RequestPairing", "peer_id", "mac-1"},
		{"devices.accept", map[string]any{"request_id": "req-1"}, "AcceptPairing", "request_id", "req-1"},
		{"pairing.approve", map[string]any{"id": "req-2"}, "AcceptPairing", "request_id", "req-2"},
		{"pairing.reject", map[string]any{"id": "req-2"}, "RejectPairing", "request_id", "req-2"},
		{"devices.block", map[string]any{"peer_id": "mac-2", "confirm": true}, "Revoke", "peer_id", "mac-2"},
		{"devices.unblock", map[string]any{"peer_id": "mac-2"}, "AllowPairing", "peer_id", "mac-2"},
		{"lanWeb.configure", map[string]any{"enabled": true}, "ConfigureLanWeb", "enabled", true},
	}
	for _, test := range cases {
		t.Run(fmt.Sprint(test.action, test.value), func(t *testing.T) {
			pipe, dial := newDesktopPipe(t, sampleSnapshot, queuedReply)
			service := NewService(dial)
			openSession(t, service, pipe)
			outcome := mustInvoke(t, service, test.action, test.params)
			if outcome["outcome"] != "queued" {
				t.Fatalf("结果错误: %v", outcome)
			}
			command := nextCommand(t, pipe)
			value, ok := command[test.variant]
			if !ok {
				t.Fatalf("期望 %s 命令，实际 %v", test.variant, command)
			}
			if test.field != "" {
				fields, _ := value.(map[string]any)
				if fields[test.field] != test.value {
					t.Fatalf("%s 参数错误: %v", test.field, fields)
				}
			}
		})
	}
}

func TestWindowsRemoteAppHostActionsUseNativeServiceCommands(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		switch {
		case command["ListRemoteApps"] != nil:
			return map[string]any{"RemoteApps": map[string]any{
				"available": true,
				"reason":    nil,
				"apps":      []any{map[string]any{"name": "demo", "path": `C:\\demo.exe`}},
				"host":      map[string]any{"ready": true},
			}}
		case command["ListInstalledApps"] != nil:
			return map[string]any{"InstalledApps": map[string]any{
				"apps": []any{map[string]any{"name": "Notepad", "path": `C:\\Windows\\notepad.exe`}},
			}}
		case command["RemoteAppConnection"] != nil:
			return map[string]any{"RemoteAppConnection": map[string]any{"rdp": "full-rdp"}}
		default:
			return queuedReply(command)
		}
	})
	service := NewService(dial)
	openSession(t, service, pipe)

	catalog := mustInvoke(t, service, "remoteApps.hostCatalog", nil)
	if catalog["available"] != true || len(catalog["applications"].([]any)) != 1 {
		t.Fatalf("本机 RemoteApp 目录映射错误: %v", catalog)
	}
	if command := nextCommand(t, pipe); command["ListRemoteApps"] == nil {
		t.Fatalf("目录请求命令错误: %v", command)
	}

	installed := mustInvoke(t, service, "remoteApps.hostInstalled", nil)
	if len(installed["applications"].([]any)) != 1 {
		t.Fatalf("已安装应用映射错误: %v", installed)
	}
	if command := nextCommand(t, pipe); command["ListInstalledApps"] == nil {
		t.Fatalf("已安装应用请求命令错误: %v", command)
	}

	registered := mustInvoke(t, service, "remoteApps.hostRegister", map[string]any{
		"name": "demo", "path": `C:\\demo.exe`, "display_name": "Demo",
	})
	if registered["outcome"] != "queued" {
		t.Fatalf("注册结果错误: %v", registered)
	}
	registerCommand := nextCommand(t, pipe)
	if fields := registerCommand["RegisterRemoteApp"].(map[string]any); fields["name"] != "demo" || fields["display_name"] != "Demo" {
		t.Fatalf("注册参数错误: %v", registerCommand)
	}

	connection := mustInvoke(t, service, "remoteApps.hostConnection", map[string]any{"name": "demo"})
	if connection["rdp"] != "full-rdp" {
		t.Fatalf("RDP 配置映射错误: %v", connection)
	}
	if fields := nextCommand(t, pipe)["RemoteAppConnection"].(map[string]any); fields["name"] != "demo" {
		t.Fatalf("RDP 请求参数错误: %v", fields)
	}

	unregistered := mustInvoke(t, service, "remoteApps.hostUnregister", map[string]any{"name": "demo"})
	if unregistered["outcome"] != "queued" {
		t.Fatalf("移除结果错误: %v", unregistered)
	}
	if fields := nextCommand(t, pipe)["UnregisterRemoteApp"].(map[string]any); fields["name"] != "demo" {
		t.Fatalf("移除参数错误: %v", fields)
	}
}

func TestWindowsRemoteAppPeerBridgeUsesRustDesktopProtocol(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		switch {
		case command["RemoteApplicationCatalog"] != nil:
			return map[string]any{"RemoteApplicationCatalog": map[string]any{"json": `{"type":"catalog","version":2,"available":true,"apps":[{"name":"notepad","display_name":"记事本","path":"C:\\Windows\\notepad.exe","command_line":"","icon_path":"","icon_index":0}],"host":{"port":3389,"checks":[]}}`}}
		case command["RemoteApplicationPrepare"] != nil:
			return map[string]any{"RemoteApplicationReady": map[string]any{"json": `{"type":"ready","version":2,"name":"notepad","ticket":[1,2,3],"rdp":"full-rdp","host":{"port":3389,"checks":[]}}`}}
		case command["RemoteApplicationTunnel"] != nil:
			return map[string]any{"RemoteApplicationTunnel": map[string]any{"port": 43123}}
		default:
			return queuedReply(command)
		}
	})
	service := NewService(dial)
	openSession(t, service, pipe)

	catalog := mustInvoke(t, service, "remoteApps.peerCatalog", map[string]any{"peer_id": "win-peer"})
	if catalog["type"] != "catalog" || len(catalog["apps"].([]any)) != 1 {
		t.Fatalf("远端目录解包错误: %v", catalog)
	}
	if fields := nextCommand(t, pipe)["RemoteApplicationCatalog"].(map[string]any); fields["peer_id"] != "win-peer" {
		t.Fatalf("目录请求参数错误: %v", fields)
	}

	ready := mustInvoke(t, service, "remoteApps.peerPrepare", map[string]any{"peer_id": "win-peer", "name": "notepad"})
	if ready["type"] != "ready" || ready["rdp"] != "full-rdp" {
		t.Fatalf("远端启动响应解包错误: %v", ready)
	}
	if fields := nextCommand(t, pipe)["RemoteApplicationPrepare"].(map[string]any); fields["peer_id"] != "win-peer" || fields["name"] != "notepad" {
		t.Fatalf("启动请求参数错误: %v", fields)
	}

	tunnel := mustInvoke(t, service, "remoteApps.peerTunnel", map[string]any{
		"peer_id": "win-peer", "ticket": []any{float64(1), float64(2), float64(3), float64(4), float64(5), float64(6), float64(7), float64(8), float64(9), float64(10), float64(11), float64(12), float64(13), float64(14), float64(15), float64(16), float64(17), float64(18), float64(19), float64(20), float64(21), float64(22), float64(23), float64(24), float64(25), float64(26), float64(27), float64(28), float64(29), float64(30), float64(31), float64(32)},
	})
	if tunnel["port"] != float64(43123) {
		t.Fatalf("隧道端口映射错误: %v", tunnel)
	}
	fields := nextCommand(t, pipe)["RemoteApplicationTunnel"].(map[string]any)
	if fields["peer_id"] != "win-peer" || fields["ticket"] == "" {
		t.Fatalf("隧道请求参数错误: %v", fields)
	}
}

func TestWindowsRemoteAppPeerOpenDispatchesLoopbackRDPDocument(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		switch {
		case command["RemoteApplicationPrepare"] != nil:
			return map[string]any{"RemoteApplicationReady": map[string]any{"json": `{"type":"ready","version":2,"name":"notepad","ticket":[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32],"rdp":"full address:s:deskfuse-rdp.invalid\r\nremoteapplicationprogram:s:||notepad\r\n","host":{"port":3389,"checks":[]}}`}}
		case command["RemoteApplicationTunnel"] != nil:
			return map[string]any{"RemoteApplicationTunnel": map[string]any{"port": 43123}}
		default:
			return queuedReply(command)
		}
	})
	service := NewService(dial)
	nativeHost := &fakeNative{}
	service.SetNative(nativeHost)
	openSession(t, service, pipe)
	outcome := mustInvoke(t, service, "remoteApps.peerOpen", map[string]any{
		"peer_id": "win-peer", "name": "notepad",
	})
	if outcome["opened"] != true || len(nativeHost.opened) != 1 {
		t.Fatalf("RDP 文档没有交给 native shell: %v opened=%v", outcome, nativeHost.opened)
	}
	data, err := os.ReadFile(nativeHost.opened[0])
	if err != nil {
		t.Fatalf("临时 RDP 文档不可读: %v", err)
	}
	if !strings.Contains(string(data), "full address:s:127.0.0.1:43123") {
		t.Fatalf("RDP 地址没有替换为 loopback: %s", data)
	}
	_ = os.Remove(nativeHost.opened[0])
}

func TestBlockRequiresConfirmation(t *testing.T) {
	_, dial := newFakePipe(t, queuedReply)
	service := NewService(dial)
	err := service.Call(context.Background(), "app.invoke", map[string]any{
		"action": "devices.block", "params": map[string]any{"peer_id": "mac-2"},
	}, nil)
	var remote *RemoteError
	if !errors.As(err, &remote) || remote.Code != "confirmation_required" {
		t.Fatalf("期望确认错误，实际 %v", err)
	}
}

func TestRejectionUsesTypedBoundaryError(t *testing.T) {
	_, dial := newFakePipe(t, func(map[string]any) map[string]any {
		return map[string]any{"Rejected": map[string]any{
			"code": "session_unavailable", "message": "会话已锁定", "boundary_error": "session_unavailable",
		}}
	})
	service := NewService(dial)
	err := service.Call(context.Background(), "app.snapshot", nil, nil)
	var remote *RemoteError
	if !errors.As(err, &remote) {
		t.Fatalf("期望远程错误，实际 %v", err)
	}
	if remote.Code != "session_unavailable" || remote.Message != "会话已锁定" || !remote.Retryable {
		t.Fatalf("拒绝信息映射错误: %+v", remote)
	}
}

func TestUnsupportedOperationsFailBeforeDialing(t *testing.T) {
	dialed := false
	service := NewService(func(ctx context.Context) (io.ReadWriteCloser, error) {
		dialed = true
		return nil, errors.New("不应连接")
	})
	if err := service.Call(context.Background(), "cancel", map[string]any{}, nil); err == nil {
		t.Fatal("cancel 必须返回真实错误")
	}
	cases := []struct {
		action string
		code   string
	}{
		{"clipboard.paste", "unsupported_platform"},
		{"clipboard.save", "unsupported_platform"},
		{"download_and_install_update", "invalid_params"},
		{"remoteApps.catalog", "unsupported_platform"},
		{"workflow.run", "unsupported_platform"},
	}
	for _, test := range cases {
		err := service.Call(context.Background(), "app.invoke", map[string]any{
			"action": test.action, "params": map[string]any{},
		}, nil)
		var remote *RemoteError
		if !errors.As(err, &remote) || remote.Code != test.code {
			t.Fatalf("%s 期望 %s，实际 %v", test.action, test.code, err)
		}
	}
	if dialed {
		t.Fatal("不支持的操作不应打开服务连接")
	}
}

func TestWindowsMicrophoneContractReportsNativeBackendBoundary(t *testing.T) {
	service := NewService(func(context.Context) (io.ReadWriteCloser, error) {
		return nil, errors.New("microphone status must not open the desktop pipe")
	})
	status := map[string]any{}
	if err := service.microphoneStatus(context.Background(), &status); err != nil {
		t.Fatalf("麦克风状态查询失败: %v", err)
	}
	if status["supported"] != false || status["driver_ready"] != false || status["reason"] == "" {
		t.Fatalf("麦克风能力边界错误: %v", status)
	}
	err := service.microphoneManageDriver(context.Background(), map[string]any{"action": "install"}, nil)
	var remote *RemoteError
	if !errors.As(err, &remote) || remote.Code != "unsupported_platform" {
		t.Fatalf("未接入原生驱动时必须返回结构化 unsupported_platform: %v", err)
	}
}

func TestProtocolVersionMismatchIsRejected(t *testing.T) {
	dial := func(ctx context.Context) (io.ReadWriteCloser, error) {
		client, server := net.Pipe()
		go func() {
			defer server.Close()
			var header [4]byte
			if _, err := io.ReadFull(server, header[:]); err != nil {
				return
			}
			payload := make([]byte, binary.LittleEndian.Uint32(header[:]))
			_, _ = io.ReadFull(server, payload)
			data := []byte(`{"version":1,"request_id":1,"result":{"Snapshot":{}}}`)
			binary.LittleEndian.PutUint32(header[:], uint32(len(data)))
			_, _ = server.Write(header[:])
			_, _ = server.Write(data)
		}()
		return client, nil
	}
	err := NewService(dial).Call(context.Background(), "app.snapshot", nil, nil)
	if err == nil || !contains(err.Error(), "版本不兼容") {
		t.Fatalf("期望版本错误，实际 %v", err)
	}
}

func TestRefreshPollsTheOpenGeneration(t *testing.T) {
	pipe, dial := newFakePipe(t, func(command map[string]any) map[string]any {
		if _, ok := command["Poll"]; ok {
			return map[string]any{"Poll": map[string]any{
				"snapshot":           json.RawMessage(sampleSnapshot),
				"clipboard":          nil,
				"clipboard_sequence": 7,
			}}
		}
		return map[string]any{"Snapshot": json.RawMessage(sampleSnapshot)}
	})
	service := NewService(dial)

	mustCall(t, service, "app.snapshot", nil)
	mustCall(t, service, "app.snapshot", nil)
	if _, ok := (<-pipe.commands)["Snapshot"]; !ok {
		t.Fatal("首次读取必须是 Snapshot")
	}
	poll, ok := (<-pipe.commands)["Poll"].(map[string]any)
	if !ok {
		t.Fatal("后续读取必须复用 Poll")
	}
	if poll["session_epoch"] != float64(4) || poll["after_sequence"] != float64(0) {
		t.Fatalf("Poll 参数错误: %v", poll)
	}
	mustCall(t, service, "app.snapshot", nil)
	next := (<-pipe.commands)["Poll"].(map[string]any)
	if next["after_sequence"] != float64(7) {
		t.Fatalf("游标未推进: %v", next)
	}
}

func TestRejectedPollReopensTheSession(t *testing.T) {
	pipe, dial := newFakePipe(t, func(command map[string]any) map[string]any {
		if _, ok := command["Poll"]; ok {
			return map[string]any{"Rejected": map[string]any{"code": "session_unavailable", "message": "会话已切换"}}
		}
		return map[string]any{"Snapshot": json.RawMessage(sampleSnapshot)}
	})
	service := NewService(dial)

	mustCall(t, service, "app.snapshot", nil)
	value := mustCall(t, service, "app.snapshot", nil)
	if value["local"].(map[string]any)["id"] != "win-local" {
		t.Fatalf("重新打开会话后快照错误: %v", value)
	}
	<-pipe.commands
	if _, ok := (<-pipe.commands)["Poll"]; !ok {
		t.Fatal("期望一次 Poll")
	}
	if _, ok := (<-pipe.commands)["Snapshot"]; !ok {
		t.Fatal("被拒绝的 Poll 之后必须重新 Snapshot")
	}
}

func TestLibraryQueryOpensSessionAndReturnsEngineJSON(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["LibraryQuery"]; ok {
			return map[string]any{"LibraryResult": map[string]any{
				"json": `{"lists":[],"revision":3}`,
			}}
		}
		return queuedReply(command)
	})
	service := NewService(dial)

	var result json.RawMessage
	if err := service.Call(context.Background(), "library.query", map[string]any{"op": "state"}, &result); err != nil {
		t.Fatal(err)
	}
	if string(result) != `{"lists":[],"revision":3}` {
		t.Fatalf("内容库结果未原样返回: %s", result)
	}
	snapshotCommand := nextCommand(t, pipe)
	if _, ok := snapshotCommand["Snapshot"]; !ok {
		t.Fatalf("命令前的首次读取必须打开会话: %v", snapshotCommand)
	}
	command := nextCommand(t, pipe)
	query, ok := command["LibraryQuery"].(map[string]any)
	if !ok || query["request"] != `{"op":"state"}` {
		t.Fatalf("LibraryQuery 参数错误: %v", command)
	}
}

func TestLibraryMutateRestoresNumericIDs(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["LibraryMutate"]; ok {
			return map[string]any{"LibraryResult": map[string]any{"json": `{"revision":4}`}}
		}
		return queuedReply(command)
	})
	service := NewService(dial)
	openSession(t, service, pipe)

	params := map[string]any{"op": "set_item_title", "id": "42", "title": "标题"}
	if err := service.Call(context.Background(), "library.mutate", params, nil); err != nil {
		t.Fatal(err)
	}
	command := nextCommand(t, pipe)
	mutate := command["LibraryMutate"].(map[string]any)
	var request map[string]any
	if err := json.Unmarshal([]byte(mutate["request"].(string)), &request); err != nil {
		t.Fatal(err)
	}
	if request["id"] != float64(42) {
		t.Fatalf("数字 ID 必须恢复为 JSON number: %v", request)
	}
}

func TestClipboardContentProjectsServiceFormats(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["LibraryContent"]; ok {
			return map[string]any{"LibraryContent": map[string]any{
				"formats": []any{
					map[string]any{"mime": "text/plain", "data_base64": "aGVsbG8="},
					map[string]any{"mime": "image/png", "data_base64": "cG5n"},
					map[string]any{"mime": "text/rtf", "data_base64": "cnRm"},
				},
				"files": []any{"C:\\Users\\me\\a.txt"},
			}}
		}
		return queuedReply(command)
	})
	service := NewService(dial)
	openSession(t, service, pipe)

	value := mustCall(t, service, "clipboard.content", map[string]any{
		"id": "42", "include_image": false, "include_rtf": false,
	})
	if value["id"] != "42" || value["text"] != "hello" || value["has_image"] != true ||
		value["rtf_available"] != true || value["truncated"] != false || value["can_request_files"] != false {
		t.Fatalf("内容映射错误: %v", value)
	}
	files := value["file_paths"].([]any)
	if len(files) != 1 || files[0] != `C:\Users\me\a.txt` {
		t.Fatalf("文件路径映射错误: %v", value["file_paths"])
	}
	if _, ok := value["image_png_base64"]; ok {
		t.Fatalf("未请求图片时不得下发图片字节: %v", value)
	}
	if _, ok := value["rtf_base64"]; ok {
		t.Fatalf("未请求 RTF 时不得下发 RTF 字节: %v", value)
	}

	command := nextCommand(t, pipe)
	content := command["LibraryContent"].(map[string]any)
	if content["id"] != float64(42) {
		t.Fatalf("LibraryContent 参数错误: %v", command)
	}

	withImage := mustCall(t, service, "clipboard.content", map[string]any{
		"id": "42", "include_image": true, "include_rtf": true,
	})
	if withImage["image_png_base64"] != "cG5n" || withImage["rtf_base64"] != "cnRm" {
		t.Fatalf("请求的字节未下发: %v", withImage)
	}
}

func TestClipboardThumbnailProjectsServicePreview(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["LibraryThumbnail"]; ok {
			return map[string]any{"LibraryThumbnail": map[string]any{
				"mime": "image/png", "data_base64": "aGk=",
			}}
		}
		return queuedReply(command)
	})
	service := NewService(dial)
	openSession(t, service, pipe)

	value := mustCall(t, service, "clipboard.thumbnail", map[string]any{"id": "9", "max_pixel": 240})
	if value["mime"] != "image/png" || value["data_base64"] != "aGk=" {
		t.Fatalf("缩略图映射错误: %v", value)
	}
	command := nextCommand(t, pipe)
	thumbnail := command["LibraryThumbnail"].(map[string]any)
	if thumbnail["id"] != float64(9) || thumbnail["max_pixel_size"] != float64(240) {
		t.Fatalf("缩略图参数错误: %v", command)
	}
}

func TestClipboardPeersUsesSnapshotPeers(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, queuedReply)
	service := NewService(dial)
	openSession(t, service, pipe)

	value := mustInvoke(t, service, "clipboard.peers", nil)
	peers, ok := value["value"].([]any)
	if !ok || len(peers) != 1 {
		t.Fatalf("只应返回可信未阻止的设备: %v", value)
	}
	peer := peers[0].(map[string]any)
	if peer["id"] != "mac-1" || peer["name"] != "Studio" || peer["connected"] != true {
		t.Fatalf("设备映射错误: %v", peer)
	}
}

func TestClipboardSourcesUsesLibraryState(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["LibraryQuery"]; ok {
			return map[string]any{"LibraryResult": map[string]any{
				"json": `{"devices":[{"id":"mac-1","name":"Studio"}]}`,
			}}
		}
		return queuedReply(command)
	})
	service := NewService(dial)
	openSession(t, service, pipe)

	value := mustInvoke(t, service, "clipboard.sources", nil)
	sources := value["value"].([]any)
	if len(sources) != 1 || sources[0].(map[string]any)["id"] != "mac-1" {
		t.Fatalf("来源列表映射错误: %v", value)
	}
}

func TestClipboardShareUsesSupportedCommands(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["ShareLibraryEntry"]; ok {
			return queuedReply(command)
		}
		if _, ok := command["LibraryContent"]; ok {
			return map[string]any{"LibraryContent": map[string]any{
				"formats": []any{map[string]any{"mime": "text/plain", "data_base64": "aGVsbG8="}},
				"files":   []any{},
			}}
		}
		if _, ok := command["RunWorkflow"]; ok {
			return map[string]any{"WorkflowResult": map[string]any{
				"run_id": "run-1", "workflow_id": "wf-1",
				"formats": []any{map[string]any{"mime": "text/plain", "data_base64": "b3V0"}},
				"message": "done",
			}}
		}
		if _, ok := command["ShareContent"]; ok {
			return queuedReply(command)
		}
		return queuedReply(command)
	})
	service := NewService(dial)
	openSession(t, service, pipe)

	outcome := mustInvoke(t, service, "clipboard.share", map[string]any{"id": "5", "peer_id": "mac-1"})
	if outcome["outcome"] != "queued" {
		t.Fatalf("直接共享结果错误: %v", outcome)
	}
	direct := nextCommand(t, pipe)
	share, ok := direct["ShareLibraryEntry"].(map[string]any)
	if !ok || share["id"] != float64(5) || share["peer_id"] != "mac-1" {
		t.Fatalf("共享命令错误: %v", direct)
	}

	outcome = mustInvoke(t, service, "clipboard.share", map[string]any{
		"id": "5", "peer_id": "mac-1", "workflow_id": "wf-1",
	})
	if outcome["outcome"] != "queued" {
		t.Fatalf("工作流共享结果错误: %v", outcome)
	}
	if _, ok := nextCommand(t, pipe)["LibraryContent"]; !ok {
		t.Fatal("工作流共享必须先读取内容")
	}
	if _, ok := nextCommand(t, pipe)["RunWorkflow"]; !ok {
		t.Fatal("工作流共享必须执行工作流")
	}
	shared := nextCommand(t, pipe)
	if _, ok := shared["ShareContent"]; !ok {
		t.Fatalf("工作流输出必须交给 ShareContent: %v", shared)
	}
}

func TestWorkflowCommandsUseServiceCatalog(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["GetWorkflows"]; ok {
			return map[string]any{"Workflows": map[string]any{
				"workflows": []any{map[string]any{"id": "wf-1", "name": "示例", "version": 2}},
			}}
		}
		if _, ok := command["GetWorkflowCatalog"]; ok {
			return map[string]any{"WorkflowCatalog": map[string]any{
				"json": `{"sync_status":"idle","peers":[],"conflicts":[],"issues":[],"error":null}`,
			}}
		}
		return queuedReply(command)
	})
	service := NewService(dial)
	openSession(t, service, pipe)

	value := mustInvoke(t, service, "workflow.list", nil)
	payload := value["value"].(map[string]any)
	definitions := payload["definitions"].([]any)
	if len(definitions) != 1 || definitions[0].(map[string]any)["id"] != "wf-1" {
		t.Fatalf("工作流定义映射错误: %v", payload)
	}
	catalog := payload["catalog"].(map[string]any)
	if catalog["sync_status"] != "idle" {
		t.Fatalf("工作流同步状态映射错误: %v", catalog)
	}
	nextCommand(t, pipe) // GetWorkflows
	nextCommand(t, pipe) // GetWorkflowCatalog

	workflow := map[string]any{"version": 2, "id": "wf-2", "name": "新工作流", "enabled": true,
		"input_type": "Any", "steps": []any{}}
	if outcome := mustInvoke(t, service, "workflow.save", map[string]any{"workflow": workflow}); outcome["outcome"] != "queued" {
		t.Fatalf("保存结果错误: %v", outcome)
	}
	save := nextCommand(t, pipe)
	saved := save["SaveWorkflow"].(map[string]any)["workflow"].(map[string]any)
	if saved["id"] != "wf-2" {
		t.Fatalf("SaveWorkflow 参数错误: %v", save)
	}

	err := service.Call(context.Background(), "app.invoke", map[string]any{
		"action": "workflow.delete", "params": map[string]any{"id": "wf-2"},
	}, nil)
	var remote *RemoteError
	if !errors.As(err, &remote) || remote.Code != "confirmation_required" {
		t.Fatalf("删除工作流必须要求确认: %v", err)
	}
	if outcome := mustInvoke(t, service, "workflow.delete", map[string]any{"id": "wf-2", "confirm": true}); outcome["outcome"] != "queued" {
		t.Fatalf("删除结果错误: %v", outcome)
	}
	deleted := nextCommand(t, pipe)
	if deleted["DeleteWorkflow"].(map[string]any)["id"] != "wf-2" {
		t.Fatalf("DeleteWorkflow 参数错误: %v", deleted)
	}
}

func TestInputSetSettingsPreservesServiceFields(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleInputSnapshot, queuedReply)
	service := NewService(dial)
	openSession(t, service, pipe)

	settings := map[string]any{
		"feature_enabled":  true,
		"enabled":          true,
		"controller":       true,
		"participants":     []any{"mac-1"},
		"switch_mode":      "dwell",
		"disabled_corners": 1,
		"corner_size":      20,
		"placements": []any{map[string]any{
			"device_id": "mac-1", "display_id": "d1", "x": 10.0, "y": 20.0,
		}},
		"shortcuts": []any{
			map[string]any{"key_code": 10.0, "modifiers": 1835008.0},
			map[string]any{"key_code": 11.0, "modifiers": 1835008.0},
			map[string]any{"key_code": 12.0, "modifiers": 1835008.0},
			map[string]any{"key_code": 13.0, "modifiers": 1835008.0},
			map[string]any{"key_code": 14.0, "modifiers": 1835008.0},
			map[string]any{"key_code": 15.0, "modifiers": 1835008.0},
		},
	}
	if outcome := mustInvoke(t, service, "set_input", map[string]any{"settings": settings}); outcome["outcome"] != "queued" {
		t.Fatalf("设置结果错误: %v", outcome)
	}
	command := nextCommand(t, pipe)
	body := command["SetInputSharingSettings"].(map[string]any)
	sent := body["settings"].(map[string]any)
	if sent["role"] != "Controller" || sent["switch_mode"] != "Dwell250Ms" {
		t.Fatalf("设置映射错误: %v", sent)
	}
	shortcuts := sent["shortcuts"].(map[string]any)
	if shortcuts["left"].(map[string]any)["key_code"] != float64(10) {
		t.Fatalf("快捷键数组顺序映射错误: %v", shortcuts)
	}
	if profiles, ok := sent["layout_profiles"].([]any); !ok || len(profiles) != 1 {
		t.Fatalf("未编辑的 layout_profiles 必须保留: %v", sent)
	}
	if sent["authorized_controller"] != nil {
		t.Fatalf("authorized_controller 必须保持 null: %v", sent)
	}
}

func TestInputActionsMapToServiceCommands(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleInputSnapshot, queuedReply)
	service := NewService(dial)
	openSession(t, service, pipe)

	mustInvoke(t, service, "input.reclaim", nil)
	if _, ok := nextCommand(t, pipe)["ReclaimInput"]; !ok {
		t.Fatal("input.reclaim 必须映射到 ReclaimInput")
	}

	mustInvoke(t, service, "input.moveDevice", map[string]any{
		"deviceId": "mac-1", "displayId": "d1", "x": 1.5, "y": 2.5,
	})
	move := nextCommand(t, pipe)["MoveControlDevice"].(map[string]any)
	if move["device_id"] != "mac-1" || move["display_id"] != "d1" ||
		move["x"] != 1.5 || move["y"] != 2.5 {
		t.Fatalf("MoveControlDevice 参数错误: %v", move)
	}

	mustInvoke(t, service, "input.reconnect", map[string]any{"device_id": "mac-1"})
	if nextCommand(t, pipe)["RestartInputControl"].(map[string]any)["peer_id"] != "mac-1" {
		t.Fatal("input.reconnect 必须映射到 RestartInputControl")
	}

	mustInvoke(t, service, "input.setParticipant", map[string]any{"device_id": "mac-2", "included": true})
	body := nextCommand(t, pipe)["SetInputSharingSettings"].(map[string]any)["settings"].(map[string]any)
	participants := body["participants"].([]any)
	if len(participants) != 2 || participants[1] != "mac-2" {
		t.Fatalf("参与设备映射错误: %v", participants)
	}

	mustInvoke(t, service, "feature.setEnabled", map[string]any{"feature": "input", "enabled": false})
	body = nextCommand(t, pipe)["SetInputSharingSettings"].(map[string]any)["settings"].(map[string]any)
	if body["feature_enabled"] != false {
		t.Fatalf("功能开关映射错误: %v", body)
	}

	mustInvoke(t, service, "devices.setInputPath", map[string]any{"device_id": "mac-1", "path": "wifi"})
	pathCommand := nextCommand(t, pipe)["SetInputSharingSettings"].(map[string]any)
	settings := pathCommand["settings"].(map[string]any)
	paths := settings["input_paths"].([]any)
	if len(paths) != 1 {
		t.Fatalf("键鼠线路没有写入: %v", settings["input_paths"])
	}
	pathSetting := paths[0].(map[string]any)
	if pathSetting["device_id"] != "mac-1" {
		t.Fatalf("线路设备错误: %v", pathSetting)
	}
	policy := pathSetting["policy"].(map[string]any)
	if policy["Only"] != "WiFi" {
		t.Fatalf("线路策略错误: %v", policy)
	}

	mustInvoke(t, service, "feature.setEnabled", map[string]any{"feature": "clipboard", "enabled": false})
	clipboard := nextCommand(t, pipe)["SetClipboardSync"].(map[string]any)
	if clipboard["enabled"] != false {
		t.Fatalf("剪贴板开关映射错误: %v", clipboard)
	}
}

func TestInputPathProjectionAndAutoReset(t *testing.T) {
	settings := inputSettingsWire{
		InputPaths: []json.RawMessage{
			json.RawMessage(`{"device_id":"mac-1","policy":{"Only":"WiFi"}}`),
			json.RawMessage(`{"device_id":"mac-2","policy":{"Only":"Ethernet"}}`),
		},
	}
	current, seen := settings.inputPathFor("mac-1")
	if current != "wifi" || !containsString(seen, "ethernet") {
		t.Fatalf("线路投影错误: current=%q seen=%v", current, seen)
	}
	if current, _ := settings.inputPathFor("mac-3"); current != "auto" {
		t.Fatalf("未知设备应为 auto，实际 %q", current)
	}
}

func TestUpdateRichTextPlainTextUsesLibraryMutate(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["LibraryMutate"]; ok {
			return map[string]any{"LibraryResult": map[string]any{"json": `{"revision":5}`}}
		}
		return queuedReply(command)
	})
	service := NewService(dial)
	openSession(t, service, pipe)

	outcome := mustInvoke(t, service, "clipboard.updateRichText", map[string]any{
		"op": "set_item_text", "id": "42", "text": "新文本",
	})
	if outcome["outcome"] != "applied" {
		t.Fatalf("纯文本编辑结果错误: %v", outcome)
	}
	command := nextCommand(t, pipe)
	mutate := command["LibraryMutate"].(map[string]any)
	var request map[string]any
	if err := json.Unmarshal([]byte(mutate["request"].(string)), &request); err != nil {
		t.Fatal(err)
	}
	if request["op"] != "set_item_text" || request["id"] != float64(42) || request["text"] != "新文本" {
		t.Fatalf("LibraryMutate 请求错误: %v", request)
	}
}

func TestUpdateRichTextStyledRunsUsesPortableRTF(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["LibraryMutate"]; ok {
			return map[string]any{"LibraryResult": map[string]any{"json": `{"revision":6}`}}
		}
		return queuedReply(command)
	})
	service := NewService(dial)
	openSession(t, service, pipe)
	mustInvoke(t, service, "clipboard.updateRichText", map[string]any{
		"op": "set_item_text", "id": "42", "text": "粗体和普通",
		"runs": []any{
			map[string]any{"text": "粗体", "bold": true},
			map[string]any{"text": "和普通"},
		},
	})
	command := nextCommand(t, pipe)
	mutate := command["LibraryMutate"].(map[string]any)
	var request map[string]any
	if err := json.Unmarshal([]byte(mutate["request"].(string)), &request); err != nil {
		t.Fatal(err)
	}
	rtfText, ok := request["rtf"].(string)
	if !ok || rtfText == "" {
		t.Fatalf("缺少 RTF 字节: %v", request)
	}
	encoded, err := base64.StdEncoding.DecodeString(rtfText)
	if err != nil {
		t.Fatalf("RTF base64 无效: %v", err)
	}
	if got := string(encoded); !strings.Contains(got, `\rtf1`) || !strings.Contains(got, `\b `) || !strings.Contains(got, `\u`) {
		t.Fatalf("RTF 内容不完整: %q", got)
	}
}

func contains(value, part string) bool { return strings.Contains(value, part) }

// fakeNative records the host primitives so clipboard actions can be asserted
// without touching the real system clipboard.
type fakeNative struct {
	formats  []native.Format
	files    []string
	pastes   int
	opened   []string
	failWith error
}

func (f *fakeNative) WriteClipboard(formats []native.Format, files []string) error {
	if f.failWith != nil {
		return f.failWith
	}
	f.formats, f.files = formats, files
	return nil
}
func (f *fakeNative) PasteClipboard() error { f.pastes++; return f.failWith }
func (f *fakeNative) SaveFile(string, []native.Filter, []byte) (string, bool, error) {
	return "", false, nil
}
func (f *fakeNative) ChooseFolder(string) (string, bool, error) { return "", false, nil }
func (f *fakeNative) Reveal([]string) error                     { return nil }
func (f *fakeNative) OpenPath(path string) error                { f.opened = append(f.opened, path); return nil }
func (f *fakeNative) OpenURL(string) error                      { return nil }
func (f *fakeNative) OpenSystemSettings(string) error           { return nil }
func (f *fakeNative) CopyText(string) error                     { return nil }
func (f *fakeNative) PickExe(string) (string, bool, error)      { return "", false, nil }
func (f *fakeNative) PickFiles(string) ([]string, bool, error)  { return nil, false, nil }
func (f *fakeNative) AppIcon(string, int) (string, error)       { return "", nil }
func (f *fakeNative) LocalWallpaper(string) (string, error)     { return "", nil }
func (f *fakeNative) SaveFileBytes(string, []byte) (string, bool, error) {
	return "", false, nil
}
func (f *fakeNative) Notify(string, string) error { return nil }

type fakeHost struct{ quit, shown, panelHidden, shelfShown, shelfHidden int }

func (h *fakeHost) Quit()                           { h.quit++ }
func (h *fakeHost) ShowMain(string, string)         { h.shown++ }
func (h *fakeHost) HidePanel()                      { h.panelHidden++ }
func (h *fakeHost) ShowShelf()                      { h.shelfShown++ }
func (h *fakeHost) HideShelf()                      { h.shelfHidden++ }
func (h *fakeHost) SetPanelShortcut(string) error   { return nil }
func (h *fakeHost) SuspendPanelShortcut(bool) error { return nil }

func TestClipboardCopyAppliesServiceFormats(t *testing.T) {
	pipe, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["LibraryContent"]; ok {
			return map[string]any{"LibraryContent": map[string]any{
				"formats": []any{
					map[string]any{"mime": "text/plain", "data_base64": "aGVsbG8="},
					map[string]any{"mime": "image/png", "data_base64": "cG5n"},
				},
				"files": []any{},
			}}
		}
		return queuedReply(command)
	})
	service := NewService(dial)
	nativeActions := &fakeNative{}
	host := &fakeHost{}
	service.SetNative(nativeActions)
	service.SetHost(host)

	value := mustInvoke(t, service, "clipboard.copy", map[string]any{"id": "42"})
	if value["action"] != "clipboard.copy" {
		t.Fatalf("复制结果错误: %v", value)
	}
	if len(nativeActions.formats) != 2 || string(nativeActions.formats[0].Data) != "hello" ||
		nativeActions.formats[1].MIME != "image/png" {
		t.Fatalf("写入格式错误: %+v", nativeActions.formats)
	}
	if nativeActions.pastes != 0 || host.panelHidden != 0 {
		t.Fatal("复制不得触发粘贴或隐藏面板")
	}

	nextCommand(t, pipe) // LibraryContent for the first copy
	mustInvoke(t, service, "clipboard.paste", map[string]any{"id": "42"})
	if nativeActions.pastes != 1 || host.panelHidden != 1 {
		t.Fatalf("粘贴必须先隐藏面板再注入按键: paste=%d hidden=%d", nativeActions.pastes, host.panelHidden)
	}
}

func TestClipboardPlainTextCopyKeepsOnlyText(t *testing.T) {
	_, dial := newDesktopPipe(t, sampleSnapshot, func(command map[string]any) map[string]any {
		if _, ok := command["LibraryContent"]; ok {
			return map[string]any{"LibraryContent": map[string]any{
				"formats": []any{
					map[string]any{"mime": "text/plain", "data_base64": "aGVsbG8="},
					map[string]any{"mime": "text/rtf", "data_base64": "cnRm"},
				},
				"files": []any{},
			}}
		}
		return queuedReply(command)
	})
	service := NewService(dial)
	nativeActions := &fakeNative{}
	service.SetNative(nativeActions)

	mustInvoke(t, service, "clipboard.copy", map[string]any{"id": "42", "plain_text": true})
	if len(nativeActions.formats) != 1 || nativeActions.formats[0].MIME != "text/plain" {
		t.Fatalf("纯文本复制只应写入 text/plain: %+v", nativeActions.formats)
	}
}
