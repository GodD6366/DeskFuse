package ui

import (
	"strings"
	"testing"
	"time"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/clipboard"
)

func TestClipboardTitleFallbacks(t *testing.T) {
	if got := clipboardTitle(clipboard.Item{Title: "Named", Preview: "ignored"}); got != "Named" {
		t.Fatalf("title = %q, want Named", got)
	}
	if got := clipboardTitle(clipboard.Item{Preview: "first line\nsecond line"}); got != "first line\nsecond line" {
		t.Fatalf("preview title = %q, want the engine preview", got)
	}
	if got := clipboardTitle(clipboard.Item{FileCount: 3}); got != "3 个文件" {
		t.Fatalf("file title = %q, want 3 个文件", got)
	}
	if got := clipboardTitle(clipboard.Item{HasImage: true}); got != "剪贴板图片" {
		t.Fatalf("image title = %q, want 剪贴板图片", got)
	}
	if got := clipboardTitle(clipboard.Item{}); got != "剪贴板文字" {
		t.Fatalf("empty title = %q, want 剪贴板文字", got)
	}
}

func TestClipboardHeadingIgnoresPreviewBody(t *testing.T) {
	if got := clipboardHeading(clipboard.Item{Title: "Named", Preview: "body"}); got != "Named" {
		t.Fatalf("heading = %q, want Named", got)
	}
	if got := clipboardHeading(clipboard.Item{Preview: "body only"}); got != "剪贴板文字" {
		t.Fatalf("heading = %q, want 剪贴板文字", got)
	}
	if got := clipboardHeading(clipboard.Item{FileCount: 2}); got != "2 个文件" {
		t.Fatalf("heading = %q, want 2 个文件", got)
	}
}

func TestClipboardDay(t *testing.T) {
	if got := clipboardDay(clipboard.Item{}); got != "其他" {
		t.Fatalf("nil copied time = %q, want 其他", got)
	}
	old := uint64(time.Date(2020, 1, 2, 15, 0, 0, 0, time.Local).UnixMilli())
	if got := clipboardDay(clipboard.Item{CopiedAtMS: &old}); got != "1月2日" {
		t.Fatalf("old copied time = %q, want 1月2日", got)
	}
}

func TestClipboardPreviewDoesNotClaimFullPayload(t *testing.T) {
	truncated := clipboard.Item{Length: 500, Preview: "short"}
	if note := clipboardPreviewNote(truncated); !strings.Contains(note, "预览片段") {
		t.Fatalf("truncated note = %q, want a preview-only explanation", note)
	}
	complete := clipboard.Item{Length: 5, Preview: "short"}
	if note := clipboardPreviewNote(complete); note != "" {
		t.Fatalf("complete note = %q, want empty", note)
	}
	image := clipboard.Item{HasImage: true, Preview: "image"}
	if note := clipboardPreviewNote(image); !strings.Contains(note, "不显示图片") {
		t.Fatalf("image note = %q, want an explicit media limitation", note)
	}
}

func TestClipboardEmptyCopy(t *testing.T) {
	title, message := clipboardEmptyCopy("  needle  ")
	if title != "没有匹配的记录" || !strings.Contains(message, "needle") {
		t.Fatalf("search empty copy = %q / %q", title, message)
	}
	title, message = clipboardEmptyCopy("   ")
	if title != "剪贴板历史为空" || !strings.Contains(message, "真实历史记录") {
		t.Fatalf("history empty copy = %q / %q", title, message)
	}
}

func TestClipboardSize(t *testing.T) {
	cases := map[uint64]string{
		0:       "0 B",
		999:     "999 B",
		1024:    "1.0 KB",
		1536:    "1.5 KB",
		1 << 20: "1.0 MB",
	}
	for input, want := range cases {
		if got := clipboardSize(input); got != want {
			t.Errorf("clipboardSize(%d) = %q, want %q", input, got, want)
		}
	}
}
