//go:build !windows

package main

import (
	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/winservice"
	"github.com/egoist/mygo"
)

// installWindowsIntegration is a no-op where the native host supplies the
// menu bar item, dock icon and shortcuts itself.
func installWindowsIntegration(*mygo.Window, *panelWindow, *shelfWindow, *winservice.Service) {}
