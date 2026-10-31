// Package desktop reports the host platform to the shared application.
//
// It currently reports only facts available from the Go runtime. Capabilities
// stay empty until the existing native engines can be probed through their
// bridge; this package must not guess support from the OS name.
package desktop

import (
	"runtime"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/platform/contract"
)

// Provider returns the host provider used by the MyGo application host.
func Provider() contract.Provider {
	return host{}
}

type host struct{}

func (host) Descriptor() contract.Descriptor {
	return contract.Descriptor{
		OS:   contract.OS(runtime.GOOS),
		Arch: runtime.GOARCH,
	}
}

func (host) Capabilities() []contract.CapabilityStatus {
	return nil
}
