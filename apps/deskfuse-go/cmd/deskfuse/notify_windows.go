//go:build windows

package main

import (
	"fmt"
	"strings"
	"sync"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/native"
)

// snapshotNotifier watches decoded snapshots and surfaces the events the user
// should notice even when the DeskFuse window is hidden: pairing requests and
// finished file transfers.
type snapshotNotifier struct {
	native native.Actions

	mu           sync.Mutex
	pairingSeen  map[string]bool
	transferPhase map[string]string
}

func newSnapshotNotifier(actions native.Actions) *snapshotNotifier {
	return &snapshotNotifier{
		native:        actions,
		pairingSeen:   map[string]bool{},
		transferPhase: map[string]string{},
	}
}

func (n *snapshotNotifier) observe(snapshot map[string]any) {
	if n == nil || n.native == nil {
		return
	}
	n.mu.Lock()
	defer n.mu.Unlock()

	pairing, _ := snapshot["pairing"].([]any)
	seen := map[string]bool{}
	for _, raw := range pairing {
		entry, ok := raw.(map[string]any)
		if !ok {
			continue
		}
		id, _ := entry["id"].(string)
		if id == "" {
			continue
		}
		seen[id] = true
		if !n.pairingSeen[id] {
			name, _ := entry["name"].(string)
			if name == "" {
				name = "新设备"
			}
			n.notify("设备配对请求", fmt.Sprintf("%s 请求配对", name))
		}
	}
	n.pairingSeen = seen

	transfers, _ := snapshot["transfers"].([]any)
	phases := map[string]string{}
	for _, raw := range transfers {
		entry, ok := raw.(map[string]any)
		if !ok {
			continue
		}
		id, _ := entry["id"].(string)
		phase, _ := entry["phase"].(string)
		if id == "" {
			continue
		}
		phases[id] = phase
		previous, existed := n.transferPhase[id]
		if !existed || previous == phase {
			continue
		}
		name, _ := entry["name"].(string)
		if name == "" {
			name = "文件"
		}
		switch phase {
		case "completed":
			n.notify("传输完成", name+" 已传输完成")
		case "failed":
			detail, _ := entry["detail"].(string)
			body := name + " 传输失败"
			if detail != "" {
				body += "：" + detail
			}
			n.notify("传输失败", body)
		}
	}
	n.transferPhase = phases
}

func (n *snapshotNotifier) notify(title, body string) {
	body = strings.TrimSpace(body)
	go func() {
		// Notifications must not block or panic the snapshot path.
		_ = n.native.Notify(title, body)
	}()
}
