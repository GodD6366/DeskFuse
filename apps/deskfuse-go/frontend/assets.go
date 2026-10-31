// Package frontend embeds the Vite output in the desktop client.
package frontend

import "embed"

//go:embed all:dist
var Assets embed.FS
