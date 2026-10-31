// Package app owns shared application state and commands.
//
// It must not import MyGo or any UI package: the MyGo layer renders Snapshots
// and dispatches Commands. This keeps the state machine testable with plain
// `go test` and leaves the existing Rust/Swift/C# engines authoritative for
// business behavior.
package app

import (
	"fmt"
	"sync"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/clipboard"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/platform/contract"
)

// Section identifies a top-level product area.
type Section string

const (
	SectionOverview  Section = "overview"
	SectionDevices   Section = "devices"
	SectionClipboard Section = "clipboard"
	SectionTransfers Section = "transfers"
	SectionInput     Section = "input"
	SectionSettings  Section = "settings"
)

// Command asks the application to change its visible state.
type Command string

const (
	ShowOverview  Command = "show.overview"
	ShowDevices   Command = "show.devices"
	ShowClipboard Command = "show.clipboard"
	ShowTransfers Command = "show.transfers"
	ShowInput     Command = "show.input"
	ShowSettings  Command = "show.settings"
)

const initialStatus = "Go UI 已启动；剪贴板页面按桥接配置决定是否可用，其余业务能力尚未接入。"

var sectionForCommand = map[Command]Section{
	ShowOverview:  SectionOverview,
	ShowDevices:   SectionDevices,
	ShowClipboard: SectionClipboard,
	ShowTransfers: SectionTransfers,
	ShowInput:     SectionInput,
	ShowSettings:  SectionSettings,
}

var statusForSection = map[Section]string{
	SectionOverview:  "概览骨架已运行；业务状态与平台能力尚未接入。",
	SectionDevices:   "设备页等待接入现有引擎；本步骤未实现配对、连接或路由。",
	SectionClipboard: "剪贴板历史只读页面；数据来自原生引擎桥，不写入系统剪贴板。",
	SectionTransfers: "文件传输页等待接入现有引擎；本步骤未实现拖放与暂存区。",
	SectionInput:     "键鼠共享页等待接入现有引擎；本步骤未实现显示器布局或紧急收回。",
	SectionSettings:  "设置页等待接入现有引擎；本步骤未实现权限、更新或登录项。",
}

// Snapshot is an immutable view of the application state for one frame.
type Snapshot struct {
	Platform     contract.Descriptor
	Capabilities []contract.CapabilityStatus
	Section      Section
	Status       string
	Revision     uint64
}

// Result describes the outcome of a dispatched command.
type Result struct {
	Command  Command
	Section  Section
	Revision uint64
	Message  string
}

// App is the shared state store. It is safe for concurrent use; MyGo view
// callbacks and future engine events may arrive on different goroutines.
type App struct {
	mu             sync.Mutex
	platform       contract.Descriptor
	capabilities   []contract.CapabilityStatus
	section        Section
	status         string
	revision       uint64
	clipboardStore *clipboard.Store
}

// New creates the application state from the host provider and the optional
// clipboard source. A nil source means the native bridge is not configured;
// the clipboard page then reports unavailable instead of showing sample data.
func New(provider contract.Provider, clipboardSource clipboard.Source) *App {
	application := &App{
		section:        SectionOverview,
		status:         initialStatus,
		clipboardStore: clipboard.NewStore(clipboardSource),
	}
	if provider != nil {
		application.platform = provider.Descriptor()
		application.capabilities = append([]contract.CapabilityStatus(nil), provider.Capabilities()...)
	}
	return application
}

// Clipboard exposes the read-only clipboard page state.
func (a *App) Clipboard() *clipboard.Store {
	return a.clipboardStore
}

// Close cancels in-flight clipboard queries.
func (a *App) Close() {
	a.clipboardStore.Close()
}

// Snapshot returns a copy that the UI can read without holding the lock.
func (a *App) Snapshot() Snapshot {
	a.mu.Lock()
	defer a.mu.Unlock()
	return Snapshot{
		Platform:     a.platform,
		Capabilities: append([]contract.CapabilityStatus(nil), a.capabilities...),
		Section:      a.section,
		Status:       a.status,
		Revision:     a.revision,
	}
}

// Dispatch applies a navigation command and returns its outcome. Unknown
// commands leave the state untouched and do not advance the revision.
func (a *App) Dispatch(command Command) Result {
	a.mu.Lock()
	defer a.mu.Unlock()

	section, ok := sectionForCommand[command]
	if !ok {
		return Result{
			Command:  command,
			Section:  a.section,
			Revision: a.revision,
			Message:  fmt.Sprintf("未知命令 %q；状态未改变。", command),
		}
	}

	a.section = section
	a.status = statusForSection[section]
	a.revision++

	return Result{
		Command:  command,
		Section:  a.section,
		Revision: a.revision,
		Message:  a.status,
	}
}
