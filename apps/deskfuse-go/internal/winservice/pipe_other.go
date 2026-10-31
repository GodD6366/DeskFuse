//go:build !windows

package winservice

import (
	"context"
	"errors"
	"io"
)

// DefaultPipeName documents the Windows pipe even on hosts that cannot open it.
const DefaultPipeName = `\\.\pipe\com.deskfuse.app.desktop.v2`

// Dial exists on every host so the package compiles for tests and for
// cross-compilation checks; only Windows can open the service pipe.
func Dial(string) Dialer {
	return func(context.Context) (io.ReadWriteCloser, error) {
		return nil, errors.New("Windows 服务管道只存在于 Windows")
	}
}

// WaitForService reports that the current host has no service pipe.
func WaitForService(context.Context, Dialer) error {
	return errors.New("Windows 服务管道只存在于 Windows")
}
