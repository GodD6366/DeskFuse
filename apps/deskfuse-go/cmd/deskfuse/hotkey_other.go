//go:build !windows

package main

// Global shortcut ownership stays in the native shell on non-Windows.
type panelHotkey struct{}

func newPanelHotkey(func()) *panelHotkey { return &panelHotkey{} }
func (*panelHotkey) set(string) error    { return nil }
func (*panelHotkey) suspend(bool) error  { return nil }
