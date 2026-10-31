package ui

import (
	"testing"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/app"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/platform/contract"
)

func TestPlatformLabel(t *testing.T) {
	cases := []struct {
		name       string
		descriptor contract.Descriptor
		want       string
	}{
		{"macOS", contract.Descriptor{OS: contract.Darwin, Arch: "arm64"}, "macOS · arm64"},
		{"Windows", contract.Descriptor{OS: contract.Windows, Arch: "amd64"}, "Windows · amd64"},
		{"unknown OS", contract.Descriptor{}, "未知平台"},
		{"no arch", contract.Descriptor{OS: contract.Linux}, "Linux"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := platformLabel(tc.descriptor); got != tc.want {
				t.Fatalf("platformLabel(%+v) = %q, want %q", tc.descriptor, got, tc.want)
			}
		})
	}
}

func TestCapabilitySummaryDoesNotClaimSupport(t *testing.T) {
	if got := capabilitySummary(nil); got != "平台能力尚未接入" {
		t.Fatalf("empty capabilities = %q, want an explicit not-wired message", got)
	}

	statuses := []contract.CapabilityStatus{
		{State: contract.CapabilityAvailable},
		{State: contract.CapabilityUnavailable},
		{State: contract.CapabilityUnknown},
		{State: ""},
	}
	if got, want := capabilitySummary(statuses), "1 项可用 · 1 项不可用 · 2 项未探测"; got != want {
		t.Fatalf("capabilitySummary() = %q, want %q", got, want)
	}
}

func TestNavigationCoversEverySectionOnce(t *testing.T) {
	seenSections := map[app.Section]bool{}
	seenCommands := map[app.Command]bool{}
	for _, item := range navigation {
		if seenSections[item.section] {
			t.Errorf("section %q appears twice", item.section)
		}
		if seenCommands[item.command] {
			t.Errorf("command %q appears twice", item.command)
		}
		seenSections[item.section] = true
		seenCommands[item.command] = true
	}

	for _, section := range []app.Section{
		app.SectionOverview,
		app.SectionDevices,
		app.SectionClipboard,
		app.SectionTransfers,
		app.SectionInput,
		app.SectionSettings,
	} {
		if !seenSections[section] {
			t.Errorf("section %q has no navigation item", section)
		}
	}
}
