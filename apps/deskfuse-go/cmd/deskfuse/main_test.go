package main

import (
	"io"
	"strings"
	"testing"
)

func TestClipboardSourceRequiresExactStdioMode(t *testing.T) {
	empty := func() io.Reader { return strings.NewReader("") }

	t.Setenv("DESKFUSE_MYGO_BRIDGE", "")
	if source := newClipboardSource(empty(), io.Discard); source != nil {
		t.Fatal("unset DESKFUSE_MYGO_BRIDGE enabled the clipboard bridge")
	}

	t.Setenv("DESKFUSE_MYGO_BRIDGE", "stdio-v2")
	if source := newClipboardSource(empty(), io.Discard); source != nil {
		t.Fatal("unsupported mode enabled the clipboard bridge")
	}

	reader, writer := io.Pipe()
	defer reader.Close()
	defer writer.Close()
	t.Setenv("DESKFUSE_MYGO_BRIDGE", "stdio-v1")
	if source := newClipboardSource(reader, io.Discard); source == nil {
		t.Fatal("stdio-v1 did not create a clipboard source")
	}
}
