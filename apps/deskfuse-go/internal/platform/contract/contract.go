// Package contract defines the boundary between the shared Go UI and the
// existing platform engines.
//
// Types in this package must not depend on MyGo or any other UI framework so
// that business state and platform ports remain testable without a UI.
package contract

// OS identifies the host operating system with the same spelling as
// runtime.GOOS.
type OS string

const (
	Darwin  OS = "darwin"
	Windows OS = "windows"
	Linux   OS = "linux"
)

// Descriptor identifies the host the shared UI is running on. It reports
// facts observed at startup, never inferred product capabilities.
type Descriptor struct {
	OS   OS
	Arch string
}

// Capability names a product function whose support must be probed on the
// host. The values are stable identifiers, not user-facing labels.
type Capability string

const (
	CapabilityClipboardHistory  Capability = "clipboard.history"
	CapabilityClipboardRichText Capability = "clipboard.rich_text"
	CapabilityFileTransfer      Capability = "file.transfer"
	CapabilityInputSharing      Capability = "input.sharing"
	CapabilityMicrophone        Capability = "microphone"
	CapabilityRemoteApplication Capability = "remote_application"
)

// CapabilityState is deliberately tri-state: Unknown means the host has not
// reported a probe result yet and must not be rendered as unsupported.
type CapabilityState string

const (
	CapabilityUnknown     CapabilityState = "unknown"
	CapabilityAvailable   CapabilityState = "available"
	CapabilityUnavailable CapabilityState = "unavailable"
)

// CapabilityStatus is one host capability and the engine's latest result.
type CapabilityStatus struct {
	Capability Capability
	State      CapabilityState
	// Detail carries the platform engine's own explanation. It is empty
	// while the capability has not been probed.
	Detail string
}

// Provider reports host facts to the shared application layer. Implementations
// must ask the platform engine for capability results; deriving support from
// the OS name is not allowed.
type Provider interface {
	Descriptor() Descriptor
	Capabilities() []CapabilityStatus
}
