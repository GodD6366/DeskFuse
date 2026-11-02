//go:build windows

package winservice

import "syscall"

const (
	esContinuous      = 0x80000000
	esSystemRequired  = 0x00000001
	esDisplayRequired = 0x00000002
)

var procSetThreadExecutionState = syscall.NewLazyDLL("kernel32.dll").NewProc("SetThreadExecutionState")

// setKeepAwake prevents the system and display from sleeping for as long as
// this process supplies a request. Clearing the request restores the system
// defaults; the process also releases them when it exits.
func setKeepAwake(enabled bool) error {
	flags := uintptr(esContinuous)
	if enabled {
		flags |= esSystemRequired | esDisplayRequired
	}
	if result, _, err := procSetThreadExecutionState.Call(flags); result == 0 {
		return err
	}
	return nil
}
