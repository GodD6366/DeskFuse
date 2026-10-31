//go:build windows

package winservice

import (
	"context"
	"fmt"
	"io"
	"os"
	"syscall"
	"time"
)

// DefaultPipeName is the desktop IPC pipe declared by the Windows service.
const DefaultPipeName = `\\.\pipe\com.deskfuse.app.desktop.v2`

const (
	genericRead    = 0x80000000
	genericWrite   = 0x40000000
	shareRead      = 0x00000001
	shareWrite     = 0x00000002
	openExisting   = 3
	fileAttrNormal = 0x00000080
)

// Dial opens one connection to the service pipe. The service authenticates the
// caller's session, so the client passes no credentials of its own.
func Dial(pipeName string) Dialer {
	if pipeName == "" {
		pipeName = DefaultPipeName
	}
	return func(ctx context.Context) (io.ReadWriteCloser, error) {
		name, err := syscall.UTF16PtrFromString(pipeName)
		if err != nil {
			return nil, err
		}
		handle, err := syscall.CreateFile(name, genericRead|genericWrite,
			shareRead|shareWrite, nil, openExisting, fileAttrNormal, 0)
		if err != nil {
			return nil, fmt.Errorf("打开命名管道失败: %w", err)
		}
		return os.NewFile(uintptr(handle), pipeName), nil
	}
}

// WaitForService retries Dial until the pipe accepts a connection or the
// deadline passes. The GUI starts with Windows, so the service may still be
// starting when the first request arrives.
func WaitForService(ctx context.Context, dial Dialer) error {
	backoff := 50 * time.Millisecond
	for {
		conn, err := dial(ctx)
		if err == nil {
			return conn.Close()
		}
		if ctx.Err() != nil {
			return err
		}
		select {
		case <-ctx.Done():
			return err
		case <-time.After(backoff):
		}
		if backoff < 500*time.Millisecond {
			backoff *= 2
		}
	}
}
