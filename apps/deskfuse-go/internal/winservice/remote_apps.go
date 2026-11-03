package winservice

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// remoteAppsHostList exposes the Windows host's registered RemoteApp catalog.
// This is deliberately separate from remoteApps.catalog, which is the
// controller-side cross-device catalog and still requires the native RDP
// viewer/tunnel on the controlling platform.
func (s *Service) remoteAppsHostList(ctx context.Context, result any) error {
	raw, err := s.callVariant(ctx, "ListRemoteApps", "RemoteApps")
	if err != nil {
		return err
	}
	var payload struct {
		Available bool `json:"available"`
		Reason    any  `json:"reason"`
		Apps      any  `json:"apps"`
		Host      any  `json:"host"`
	}
	if err := json.Unmarshal(raw, &payload); err != nil {
		return fmt.Errorf("RemoteApp 主机目录无效: %w", err)
	}
	return assign(result, map[string]any{
		"available":    payload.Available,
		"reason":       payload.Reason,
		"applications": payload.Apps,
		"host":         payload.Host,
	})
}

func (s *Service) remoteAppsHostInstalled(ctx context.Context, result any) error {
	raw, err := s.callVariant(ctx, "ListInstalledApps", "InstalledApps")
	if err != nil {
		return err
	}
	var payload struct {
		Apps any `json:"apps"`
	}
	if err := json.Unmarshal(raw, &payload); err != nil {
		return fmt.Errorf("已安装应用目录无效: %w", err)
	}
	return assign(result, map[string]any{"applications": payload.Apps})
}

func remoteAppStringArg(action string, args map[string]any, key string) (string, error) {
	value, _ := args[key].(string)
	if strings.TrimSpace(value) == "" {
		return "", invalidParams(fmt.Sprintf("%s 需要参数 %s", action, key))
	}
	return value, nil
}

func (s *Service) remoteAppsHostRegister(ctx context.Context, action string, args map[string]any, result any) error {
	name, err := remoteAppStringArg(action, args, "name")
	if err != nil {
		return err
	}
	path, err := remoteAppStringArg(action, args, "path")
	if err != nil {
		return err
	}
	displayName, _ := args["display_name"].(string)
	return s.callQueued(ctx, tagged("RegisterRemoteApp", map[string]any{
		"name": name, "path": path, "display_name": displayName,
	}), action, result)
}

func (s *Service) remoteAppsHostUnregister(ctx context.Context, action string, args map[string]any, result any) error {
	name, err := remoteAppStringArg(action, args, "name")
	if err != nil {
		return err
	}
	return s.callQueued(ctx, tagged("UnregisterRemoteApp", map[string]any{"name": name}), action, result)
}

func (s *Service) remoteAppsHostConnection(ctx context.Context, args map[string]any, result any) error {
	name, err := remoteAppStringArg("remoteApps.hostConnection", args, "name")
	if err != nil {
		return err
	}
	raw, err := s.callVariant(ctx, tagged("RemoteAppConnection", map[string]any{"name": name}), "RemoteAppConnection")
	if err != nil {
		return err
	}
	var payload struct {
		RDP string `json:"rdp"`
	}
	if err := json.Unmarshal(raw, &payload); err != nil {
		return fmt.Errorf("RemoteApp RDP 配置无效: %w", err)
	}
	if strings.TrimSpace(payload.RDP) == "" {
		return fmt.Errorf("服务返回空的 RemoteApp RDP 配置")
	}
	return assign(result, map[string]any{"rdp": payload.RDP})
}

// remoteAppsPeerCatalog/Prepare are the first Windows controller-side bridge
// over the Rust desktop service. They expose the same versioned messages used
// by the macOS RemoteRdpSession without making Go a second QUIC implementation.
func (s *Service) remoteAppsPeerCatalog(ctx context.Context, args map[string]any, result any) error {
	peer, err := remoteAppStringArg("remoteApps.peerCatalog", args, "peer_id")
	if err != nil {
		return err
	}
	raw, err := s.callVariant(ctx, tagged("RemoteApplicationCatalog", map[string]any{"peer_id": peer}), "RemoteApplicationCatalog")
	if err != nil {
		return err
	}
	var payload struct {
		JSON string `json:"json"`
	}
	if err := json.Unmarshal(raw, &payload); err != nil || strings.TrimSpace(payload.JSON) == "" {
		return fmt.Errorf("远程应用目录响应无效")
	}
	var message map[string]any
	if err := json.Unmarshal([]byte(payload.JSON), &message); err != nil {
		return fmt.Errorf("远程应用目录 JSON 无效: %w", err)
	}
	return assign(result, message)
}

func (s *Service) remoteAppsPeerPrepare(ctx context.Context, args map[string]any, result any) error {
	peer, err := remoteAppStringArg("remoteApps.peerPrepare", args, "peer_id")
	if err != nil {
		return err
	}
	name, err := remoteAppStringArg("remoteApps.peerPrepare", args, "name")
	if err != nil {
		return err
	}
	raw, err := s.callVariant(ctx, tagged("RemoteApplicationPrepare", map[string]any{"peer_id": peer, "name": name}), "RemoteApplicationReady")
	if err != nil {
		return err
	}
	var payload struct {
		JSON string `json:"json"`
	}
	if err := json.Unmarshal(raw, &payload); err != nil || strings.TrimSpace(payload.JSON) == "" {
		return fmt.Errorf("远程应用启动响应无效")
	}
	var message map[string]any
	if err := json.Unmarshal([]byte(payload.JSON), &message); err != nil {
		return fmt.Errorf("远程应用启动 JSON 无效: %w", err)
	}
	return assign(result, message)
}

func (s *Service) remoteAppsPeerTunnel(ctx context.Context, args map[string]any, result any) error {
	peer, err := remoteAppStringArg("remoteApps.peerTunnel", args, "peer_id")
	if err != nil {
		return err
	}
	ticketValue, ok := args["ticket"].([]any)
	if !ok || len(ticketValue) != 32 {
		return invalidParams("remoteApps.peerTunnel 需要 32 字节启动票据")
	}
	ticket := make([]byte, len(ticketValue))
	for index, value := range ticketValue {
		number, ok := value.(float64)
		if !ok || number < 0 || number > 255 || number != float64(byte(number)) {
			return invalidParams("remoteApps.peerTunnel 启动票据无效")
		}
		ticket[index] = byte(number)
	}
	raw, err := s.callVariant(ctx, tagged("RemoteApplicationTunnel", map[string]any{
		"peer_id": peer, "ticket": ticket,
	}), "RemoteApplicationTunnel")
	if err != nil {
		return err
	}
	var payload struct {
		Port uint16 `json:"port"`
	}
	if err := json.Unmarshal(raw, &payload); err != nil || payload.Port == 0 {
		return fmt.Errorf("本地 RDP 端口响应无效")
	}
	return assign(result, map[string]any{"port": payload.Port})
}

// remoteAppsPeerOpen completes the Windows controller path: prepare a
// single-use ticket, bind a loopback tunnel, rewrite the logical RDP address
// to that port, and hand the temporary document to the native shell. Windows
// Credential Manager / mstsc owns credential prompts; DeskFuse never stores
// or injects the password here.
func (s *Service) remoteAppsPeerOpen(ctx context.Context, args map[string]any, result any) error {
	peer, err := remoteAppStringArg("remoteApps.peerOpen", args, "peer_id")
	if err != nil {
		return err
	}
	name, err := remoteAppStringArg("remoteApps.peerOpen", args, "name")
	if err != nil {
		return err
	}
	prepared := map[string]any{}
	if err := s.remoteAppsPeerPrepare(ctx, map[string]any{"peer_id": peer, "name": name}, &prepared); err != nil {
		return err
	}
	rdp, _ := prepared["rdp"].(string)
	if strings.TrimSpace(rdp) == "" {
		return fmt.Errorf("远程应用没有返回 RDP 配置")
	}
	ticket, ok := prepared["ticket"].([]any)
	if !ok || len(ticket) != 32 {
		return fmt.Errorf("远程应用启动票据无效")
	}
	tunnel := map[string]any{}
	if err := s.remoteAppsPeerTunnel(ctx, map[string]any{"peer_id": peer, "ticket": ticket}, &tunnel); err != nil {
		return err
	}
	port, ok := tunnel["port"].(float64)
	if !ok || port < 1 || port > 65535 {
		return fmt.Errorf("本地 RDP 端口无效")
	}
	rdp = strings.Replace(rdp, "full address:s:deskfuse-rdp.invalid", fmt.Sprintf("full address:s:127.0.0.1:%d", int(port)), 1)
	if !strings.Contains(rdp, "full address:s:127.0.0.1:") {
		return fmt.Errorf("RDP 配置缺少受控的目标地址")
	}
	file, err := os.CreateTemp("", "deskfuse-remote-*.rdp")
	if err != nil {
		return fmt.Errorf("无法创建临时 RDP 配置: %w", err)
	}
	path := filepath.Clean(file.Name())
	if _, err := file.WriteString(rdp); err != nil {
		file.Close()
		_ = os.Remove(path)
		return fmt.Errorf("无法写入 RDP 配置: %w", err)
	}
	if err := file.Close(); err != nil {
		_ = os.Remove(path)
		return fmt.Errorf("无法关闭 RDP 配置: %w", err)
	}
	if err := s.requireNative(); err != nil {
		_ = os.Remove(path)
		return err
	}
	if err := s.native.OpenPath(path); err != nil {
		_ = os.Remove(path)
		return nativeError(err)
	}
	// mstsc reads the document before opening its window. Keep it around long
	// enough for slow machines, then remove the one-use file automatically.
	go func() {
		time.Sleep(10 * time.Minute)
		_ = os.Remove(path)
	}()
	return assign(result, map[string]any{
		"outcome": "dispatched", "action": "remoteApps.peerOpen",
		"opened": true, "application_id": name,
	})
}
