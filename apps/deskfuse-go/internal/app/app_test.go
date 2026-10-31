package app

import (
	"strings"
	"testing"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/clipboard"
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/platform/contract"
)

type fakeProvider struct {
	descriptor   contract.Descriptor
	capabilities []contract.CapabilityStatus
}

func (f fakeProvider) Descriptor() contract.Descriptor { return f.descriptor }
func (f fakeProvider) Capabilities() []contract.CapabilityStatus {
	return f.capabilities
}

func TestNewCopiesProviderFacts(t *testing.T) {
	provider := fakeProvider{
		descriptor: contract.Descriptor{OS: contract.Darwin, Arch: "arm64"},
		capabilities: []contract.CapabilityStatus{{
			Capability: contract.CapabilityClipboardHistory,
			State:      contract.CapabilityUnknown,
		}},
	}

	application := New(provider, nil)
	t.Cleanup(application.Close)
	snapshot := application.Snapshot()

	if snapshot.Platform != provider.descriptor {
		t.Fatalf("platform = %+v, want %+v", snapshot.Platform, provider.descriptor)
	}
	if snapshot.Section != SectionOverview {
		t.Fatalf("section = %q, want %q", snapshot.Section, SectionOverview)
	}
	if len(snapshot.Capabilities) != 1 {
		t.Fatalf("capabilities = %+v, want one entry", snapshot.Capabilities)
	}
}

func TestNewWithoutClipboardSourceIsUnavailable(t *testing.T) {
	application := New(nil, nil)
	t.Cleanup(application.Close)

	if state := application.Clipboard().Snapshot().State; state != clipboard.StateUnavailable {
		t.Fatalf("clipboard state = %q, want %q", state, clipboard.StateUnavailable)
	}
}

func TestDispatchMovesToRequestedSection(t *testing.T) {
	application := New(nil, nil)
	t.Cleanup(application.Close)

	result := application.Dispatch(ShowClipboard)

	if result.Section != SectionClipboard {
		t.Fatalf("result section = %q, want %q", result.Section, SectionClipboard)
	}
	if result.Revision != 1 {
		t.Fatalf("revision = %d, want 1", result.Revision)
	}
	if result.Message != statusForSection[SectionClipboard] {
		t.Fatalf("message = %q, want %q", result.Message, statusForSection[SectionClipboard])
	}
	if got := application.Snapshot().Section; got != SectionClipboard {
		t.Fatalf("snapshot section = %q, want %q", got, SectionClipboard)
	}
}

func TestUnknownCommandKeepsState(t *testing.T) {
	application := New(nil, nil)
	t.Cleanup(application.Close)
	before := application.Snapshot()

	result := application.Dispatch(Command("show.unknown"))

	if !strings.Contains(result.Message, "未知命令") {
		t.Fatalf("message = %q, want an unknown-command explanation", result.Message)
	}
	after := application.Snapshot()
	if after.Section != before.Section || after.Revision != before.Revision {
		t.Fatalf("state changed for an unknown command: before=%+v after=%+v", before, after)
	}
}

func TestSnapshotDoesNotExposeCapabilitySlice(t *testing.T) {
	application := New(fakeProvider{
		descriptor: contract.Descriptor{OS: contract.Windows, Arch: "amd64"},
		capabilities: []contract.CapabilityStatus{{
			Capability: contract.CapabilityFileTransfer,
			State:      contract.CapabilityUnknown,
		}},
	}, nil)
	t.Cleanup(application.Close)

	first := application.Snapshot()
	first.Capabilities[0].State = contract.CapabilityAvailable

	second := application.Snapshot()
	if second.Capabilities[0].State != contract.CapabilityUnknown {
		t.Fatalf("snapshot mutation leaked into app state: %+v", second.Capabilities)
	}
}

func TestEveryNavigationCommandHasASection(t *testing.T) {
	commands := []Command{
		ShowOverview,
		ShowDevices,
		ShowClipboard,
		ShowTransfers,
		ShowInput,
		ShowSettings,
	}
	for _, command := range commands {
		if _, ok := sectionForCommand[command]; !ok {
			t.Errorf("command %q has no section", command)
		}
	}
}
