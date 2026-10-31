package ui

import (
	"fmt"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/clipboard"
)

// clipboardTitle follows the macOS row title fallbacks without inventing a
// title when the engine has none.
func clipboardTitle(item clipboard.Item) string {
	if title := strings.TrimSpace(item.Title); title != "" {
		return truncateRunes(title, 160)
	}
	if preview := strings.TrimSpace(item.Preview); preview != "" {
		return truncateRunes(preview, 160)
	}
	if item.FileCount > 0 {
		return fmt.Sprintf("%d 个文件", item.FileCount)
	}
	if item.HasImage {
		return "剪贴板图片"
	}
	return "剪贴板文字"
}

// clipboardHeading is the detail pane heading. It avoids repeating the full
// preview that already appears in the preview body.
func clipboardHeading(item clipboard.Item) string {
	if title := strings.TrimSpace(item.Title); title != "" {
		return truncateRunes(title, 160)
	}
	if item.FileCount > 0 {
		return fmt.Sprintf("%d 个文件", item.FileCount)
	}
	if item.HasImage {
		return "剪贴板图片"
	}
	return "剪贴板文字"
}

func clipboardDay(item clipboard.Item) string {
	if item.CopiedAtMS == nil {
		return "其他"
	}
	value := time.UnixMilli(int64(*item.CopiedAtMS))
	now := time.Now()
	if sameDate(value, now) {
		return "今天"
	}
	if sameDate(value, now.AddDate(0, 0, -1)) {
		return "昨天"
	}
	return value.Format("1月2日")
}

func clipboardKind(item clipboard.Item) string {
	switch {
	case item.FileCount > 0:
		return fmt.Sprintf("%d 项 · %s", item.FileCount, clipboardSize(item.Length))
	case item.HasImage:
		return "图片 · " + clipboardSize(item.Length)
	default:
		return "文字 · " + clipboardSize(item.Length)
	}
}

func clipboardSource(item clipboard.Item) string {
	if source := strings.TrimSpace(item.Source); source != "" {
		return source
	}
	return "未知来源"
}

func clipboardTime(item clipboard.Item) string {
	if item.CopiedAtMS == nil {
		return "时间未知"
	}
	value := time.UnixMilli(int64(*item.CopiedAtMS))
	now := time.Now()
	if sameDate(value, now) {
		return value.Format("15:04")
	}
	if sameDate(value, now.AddDate(0, 0, -1)) {
		return "昨天 " + value.Format("15:04")
	}
	return value.Format("01-02 15:04")
}

// clipboardPreviewNote explains what the read-only page does and does not
// show. It never presents an index summary as the full payload.
func clipboardPreviewNote(item clipboard.Item) string {
	switch {
	case item.FileCount > 0:
		return "文件条目：本页只显示索引摘要，不读取或展示文件内容。"
	case item.HasImage:
		if strings.TrimSpace(item.OCRText) == "" {
			return "图片条目：本页只显示索引摘要，不显示图片内容。"
		}
		return "图片条目：本页只显示索引摘要和引擎返回的识别文字。"
	case item.Length > uint64(len(item.Preview)):
		return "以上为引擎提供的预览片段；完整文本读取未接入。"
	default:
		return ""
	}
}

func clipboardSize(length uint64) string {
	const unit = 1024
	if length < unit {
		return fmt.Sprintf("%d B", length)
	}
	divisor, exponent := uint64(unit), 0
	for length/divisor >= unit && exponent < 3 {
		divisor *= unit
		exponent++
	}
	return fmt.Sprintf("%.1f %cB", float64(length)/float64(divisor), "KMGT"[exponent])
}

func truncateRunes(value string, limit int) string {
	if limit <= 0 || utf8.RuneCountInString(value) <= limit {
		return value
	}
	runes := []rune(value)
	return string(runes[:limit]) + "…"
}

func sameDate(left, right time.Time) bool {
	leftYear, leftMonth, leftDay := left.Date()
	rightYear, rightMonth, rightDay := right.Date()
	return leftYear == rightYear && leftMonth == rightMonth && leftDay == rightDay
}
