//go:build !windows

package main

import "github.com/egoist/mygo"

// Non-Windows shells own their native shelf window. Keep the shared host
// action contract buildable without creating a second MyGo window.
type shelfWindow struct{ window *mygo.Window }

func newShelfWindow() *shelfWindow { return &shelfWindow{} }
func (*shelfWindow) show()          {}
func (*shelfWindow) hide()          {}
