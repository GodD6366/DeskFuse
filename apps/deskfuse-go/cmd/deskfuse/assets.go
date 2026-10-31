package main

import (
	frontend "github.com/GodD6366/DeskFuse/apps/deskfuse-go/frontend"
	"io/fs"
)

func frontendAssets() fs.FS {
	assets, err := fs.Sub(frontend.Assets, "dist")
	if err != nil {
		panic(err)
	}
	return assets
}
