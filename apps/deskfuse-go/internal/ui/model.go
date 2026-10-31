package ui

import (
	"fmt"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/app"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/platform/contract"
)

// navItem is one top-level entry in the shared sidebar.
type navItem struct {
	section app.Section
	label   string
	command app.Command
}

var navigation = []navItem{
	{section: app.SectionOverview, label: "概览", command: app.ShowOverview},
	{section: app.SectionDevices, label: "设备", command: app.ShowDevices},
	{section: app.SectionClipboard, label: "剪贴板", command: app.ShowClipboard},
	{section: app.SectionTransfers, label: "文件传输", command: app.ShowTransfers},
	{section: app.SectionInput, label: "键鼠共享", command: app.ShowInput},
	{section: app.SectionSettings, label: "设置", command: app.ShowSettings},
}

func sectionTitle(section app.Section) string {
	for _, item := range navigation {
		if item.section == section {
			return item.label
		}
	}
	return "未知页面"
}

func platformLabel(descriptor contract.Descriptor) string {
	name := string(descriptor.OS)
	switch descriptor.OS {
	case contract.Darwin:
		name = "macOS"
	case contract.Windows:
		name = "Windows"
	case contract.Linux:
		name = "Linux"
	case "":
		name = "未知平台"
	}
	if descriptor.Arch == "" {
		return name
	}
	return name + " · " + descriptor.Arch
}

// capabilitySummary reports only what the engines have actually returned.
// An empty list means no bridge is wired yet, not that the host lacks support.
func capabilitySummary(statuses []contract.CapabilityStatus) string {
	if len(statuses) == 0 {
		return "平台能力尚未接入"
	}

	var available, unavailable, unknown int
	for _, status := range statuses {
		switch status.State {
		case contract.CapabilityAvailable:
			available++
		case contract.CapabilityUnavailable:
			unavailable++
		default:
			unknown++
		}
	}
	return fmt.Sprintf("%d 项可用 · %d 项不可用 · %d 项未探测", available, unavailable, unknown)
}
