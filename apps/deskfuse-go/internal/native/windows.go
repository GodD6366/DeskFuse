//go:build windows

package native

import (
	"bytes"
	"encoding/base64"
	"errors"
	"fmt"
	"image"
	"image/jpeg"
	"image/png"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"
	"unsafe"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/transfer"
)

// New returns the Windows implementation of the host primitives.
func New() Actions {
	trackForeground()
	return windowsActions{}
}

type windowsActions struct{}

var (
	user32                       = syscall.NewLazyDLL("user32.dll")
	procGetForegroundWindow      = user32.NewProc("GetForegroundWindow")
	procSetForegroundWindow      = user32.NewProc("SetForegroundWindow")
	procGetWindowThreadProcessID = user32.NewProc("GetWindowThreadProcessId")
	procSendInput                = user32.NewProc("SendInput")
)

const (
	inputKeyboard  = 1
	keyEventKeyUp  = 0x0002
	vkControl      = 0x11
	vkCharacterV   = 0x56
	foregroundPoll = 200 * time.Millisecond
)

// keyboardInput mirrors Win32 KEYBDINPUT.
type keyboardInput struct {
	Vk        uint16
	Scan      uint16
	Flags     uint32
	Time      uint32
	ExtraInfo uintptr
}

// sendInputEvent mirrors the 64-bit Win32 INPUT for keyboard events: a DWORD
// type, four bytes of padding and the 24-byte KEYBDINPUT union member.
type sendInputEvent struct {
	Type uint32
	_    uint32
	Ki   keyboardInput
	_    [8]byte
}

var (
	targetMu sync.Mutex
	// lastTarget is the most recent foreground window that belongs to another
	// process, captured before the user interacts with DeskFuse.
	lastTarget uintptr
	startOnce  sync.Once
)

// trackForeground remembers the last other application's window. Pasting must
// not depend on the desktop manager restoring focus after our panel hides.
func trackForeground() {
	startOnce.Do(func() {
		go func() {
			self := uint32(os.Getpid())
			for {
				hwnd, _, _ := procGetForegroundWindow.Call()
				if hwnd != 0 {
					var pid uint32
					procGetWindowThreadProcessID.Call(hwnd, uintptr(unsafe.Pointer(&pid)))
					if pid != 0 && pid != self {
						targetMu.Lock()
						lastTarget = hwnd
						targetMu.Unlock()
					}
				}
				time.Sleep(foregroundPoll)
			}
		}()
	})
}

func (windowsActions) WriteClipboard(formats []Format, files []string) error {
	reps := make([]transfer.Representation, 0, len(formats)+1)
	seen := make(map[string]bool, len(formats))
	add := func(format transfer.Format, data []byte) {
		if seen[string(format)] {
			return
		}
		seen[string(format)] = true
		reps = append(reps, transfer.Bytes(format, data))
	}
	for _, format := range formats {
		switch format.MIME {
		case "text/plain":
			add(transfer.Text, format.Data)
		case "text/html":
			add(transfer.HTML, format.Data)
		case "text/rtf", "application/rtf":
			// Windows apps look for the registered "Rich Text Format" name.
			// MyGo maps it back to text/rtf when this process reads it.
			add(transfer.Format("Rich Text Format"), format.Data)
		case "image/png":
			add(transfer.PNG, format.Data)
		case "image/jpeg":
			converted, err := reencodePNG(format.Data)
			if err != nil {
				return fmt.Errorf("图片格式转换失败: %w", err)
			}
			add(transfer.PNG, converted)
		default:
			add(transfer.Format(format.MIME), format.Data)
		}
	}
	if len(files) > 0 {
		lines := make([]string, 0, len(files))
		for _, path := range files {
			absolute, err := filepath.Abs(path)
			if err != nil {
				return fmt.Errorf("文件路径无效: %w", err)
			}
			lines = append(lines, (&url.URL{Scheme: "file", Path: "/" + filepath.ToSlash(absolute)}).String())
		}
		reps = append(reps, transfer.Bytes(transfer.FileList, []byte(strings.Join(lines, "\r\n"))))
	}
	if len(reps) == 0 {
		return errors.New("条目没有可写入剪贴板的内容")
	}
	data := transfer.New(transfer.NewItem(reps...))
	if err := mygo.Clipboard.Write(data); err != nil {
		return fmt.Errorf("写入剪贴板失败: %w", err)
	}
	// Flush materializes the delayed representations so the content outlives
	// this process. A failure leaves the live OLE data usable.
	if err := mygo.Clipboard.Flush(); err != nil && !errors.Is(err, mygo.ErrClipboardChanged) {
		return fmt.Errorf("保存剪贴板内容失败: %w", err)
	}
	return nil
}

func (windowsActions) PasteClipboard() error {
	targetMu.Lock()
	target := lastTarget
	targetMu.Unlock()
	if target == 0 {
		return errors.New("已复制到剪贴板，但还没有可粘贴的目标窗口；请手动粘贴。")
	}
	procSetForegroundWindow.Call(target)
	// Give the target time to take focus before injecting the keystroke.
	deadline := time.Now().Add(600 * time.Millisecond)
	for time.Now().Before(deadline) {
		current, _, _ := procGetForegroundWindow.Call()
		if current == target {
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	current, _, _ := procGetForegroundWindow.Call()
	if current != target {
		return errors.New("已复制到剪贴板，但无法切换到上一个窗口；请手动粘贴。")
	}
	return sendControlV()
}

func sendControlV() error {
	events := [4]sendInputEvent{
		{Type: inputKeyboard, Ki: keyboardInput{Vk: vkControl}},
		{Type: inputKeyboard, Ki: keyboardInput{Vk: vkCharacterV}},
		{Type: inputKeyboard, Ki: keyboardInput{Vk: vkCharacterV, Flags: keyEventKeyUp}},
		{Type: inputKeyboard, Ki: keyboardInput{Vk: vkControl, Flags: keyEventKeyUp}},
	}
	sent, _, lastErr := procSendInput.Call(uintptr(len(events)), uintptr(unsafe.Pointer(&events[0])), unsafe.Sizeof(events[0]))
	if sent != uintptr(len(events)) {
		return fmt.Errorf("发送粘贴按键失败: %v", lastErr)
	}
	return nil
}

func (windowsActions) SaveFile(defaultName string, filters []Filter, data []byte) (string, bool, error) {
	path, err := mygo.Dialog.Save(mygo.SaveDialogOptions{
		Title:             "保存",
		DefaultPath:       defaultName,
		Filters:           dialogFilters(filters),
		CreateDirectories: true,
	})
	if err != nil {
		return "", false, fmt.Errorf("保存面板不可用: %w", err)
	}
	if path == "" {
		return "", false, nil
	}
	if err := os.WriteFile(path, data, 0o644); err != nil {
		return "", false, fmt.Errorf("写入文件失败: %w", err)
	}
	return path, true, nil
}

func (windowsActions) ChooseFolder(title string) (string, bool, error) {
	paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
		Title:             title,
		Directory:         true,
		CreateDirectories: true,
	})
	if err != nil {
		return "", false, fmt.Errorf("选择文件夹失败: %w", err)
	}
	if len(paths) == 0 {
		return "", false, nil
	}
	return paths[0], true, nil
}

func (windowsActions) Reveal(paths []string) error {
	for _, path := range paths {
		if _, err := os.Stat(path); err != nil {
			return fmt.Errorf("文件已不可用: %w", err)
		}
		mygo.Shell.ShowItemInFolder(path)
		return nil
	}
	return errors.New("没有可显示的文件")
}

func (windowsActions) OpenPath(path string) error {
	info, err := os.Stat(path)
	if err != nil {
		return fmt.Errorf("文件已不可用: %w", err)
	}
	if info.IsDir() {
		mygo.Shell.ShowItemInFolder(path)
		return nil
	}
	if err := mygo.Shell.OpenPath(path); err != nil {
		return fmt.Errorf("无法打开文件: %w", err)
	}
	return nil
}

func (windowsActions) OpenURL(raw string) error {
	parsed, err := url.Parse(raw)
	if err != nil || (parsed.Scheme != "https" && parsed.Scheme != "http") {
		return errors.New("只能打开 http 或 https 链接")
	}
	if err := mygo.Shell.OpenExternal(raw); err != nil {
		return fmt.Errorf("无法打开链接: %w", err)
	}
	return nil
}

// systemSettingsPages is the allowlist of Settings pages the UI may open. A
// page name never becomes part of an arbitrary shell target.
var systemSettingsPages = map[string]bool{
	"startupapps":   true,
	"privacy":       true,
	"about":         true,
	"notifications": true,
}

func (windowsActions) OpenSystemSettings(page string) error {
	if !systemSettingsPages[page] {
		return fmt.Errorf("不支持的设置页: %s", page)
	}
	if err := mygo.Shell.OpenExternal("ms-settings:" + page); err != nil {
		return fmt.Errorf("无法打开系统设置: %w", err)
	}
	return nil
}

func (windowsActions) CopyText(text string) error {
	return windowsActions{}.WriteClipboard([]Format{{MIME: "text/plain", Data: []byte(text)}}, nil)
}

func dialogFilters(filters []Filter) []mygo.FileFilter {
	out := make([]mygo.FileFilter, 0, len(filters))
	for _, filter := range filters {
		out = append(out, mygo.FileFilter{Name: filter.Name, Extensions: filter.Extensions})
	}
	return out
}

// reencodePNG converts a decodable bitmap (JPEG in practice) to PNG so the
// system clipboard and the service both receive one canonical image format.
func reencodePNG(data []byte) ([]byte, error) {
	decoded, _, err := image.Decode(bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	var encoded bytes.Buffer
	if err := png.Encode(&encoded, decoded); err != nil {
		return nil, err
	}
	return encoded.Bytes(), nil
}

// PickExe asks the user to choose an executable, for the excluded-apps list.
func (windowsActions) PickExe(title string) (string, bool, error) {
	paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
		Title:   title,
		Filters: []mygo.FileFilter{{Name: "应用程序", Extensions: []string{"exe"}}},
	})
	if err != nil {
		return "", false, fmt.Errorf("选择应用失败: %w", err)
	}
	if len(paths) == 0 {
		return "", false, nil
	}
	return paths[0], true, nil
}

// PickFiles asks the user to choose one or more files for an explicit send.
func (windowsActions) PickFiles(title string) ([]string, bool, error) {
	paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{Title: title, Multiple: true})
	if err != nil {
		return nil, false, fmt.Errorf("选择文件失败: %w", err)
	}
	if len(paths) == 0 {
		return nil, false, nil
	}
	return paths, true, nil
}

// AppIcon renders an executable's icon as a PNG data URL. Windows may not
// expose an icon for every file; an empty string is a real "no icon" result.
func (windowsActions) AppIcon(path string, size int) (string, error) {
	if size <= 0 || size > 256 {
		size = 32
	}
	png, err := mygo.Shell.FileIcon(path, size)
	if err != nil || len(png) == 0 {
		return "", nil
	}
	return "data:image/png;base64," + base64.StdEncoding.EncodeToString(png), nil
}

// LocalWallpaper returns the desktop wallpaper for a local display as a JPEG
// data URL. Windows keeps the single desktop wallpaper in the registry; a
// per-monitor source needs the IDesktopWallpaper COM interface, which this
// adapter does not speak yet, so the shared wallpaper is reported for any
// requested display and an empty string when none is set.
func (windowsActions) LocalWallpaper(displayID string) (string, error) {
	path, err := wallpaperPath()
	if err != nil || path == "" {
		return "", nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return "", nil
	}
	jpegBytes, err := imageToJPEG(data)
	if err != nil || len(jpegBytes) == 0 {
		return "", nil
	}
	return "data:image/jpeg;base64," + base64.StdEncoding.EncodeToString(jpegBytes), nil
}

// SaveFileBytes is the diagnostics-export save dialog with a .json hint.
func (a windowsActions) SaveFileBytes(defaultName string, data []byte) (string, bool, error) {
	return a.SaveFile(defaultName, []Filter{{Name: "JSON", Extensions: []string{"json"}}}, data)
}

// Notify posts a native notification; the body is plain text.
func (windowsActions) Notify(title, body string) error {
	if err := mygo.NewNotification(mygo.NotificationOptions{Title: title, Body: body}).Show(); err != nil {
		return fmt.Errorf("通知不可用: %w", err)
	}
	return nil
}

const hkeyCurrentUserNative = 0x80000001

// wallpaperPath reads HKCU\Control Panel\Desktop\WallPaper.
func wallpaperPath() (string, error) {
	var key syscall.Handle
	err := syscall.RegOpenKeyEx(hkeyCurrentUserNative, syscall.StringToUTF16Ptr(`Control Panel\Desktop`), 0, syscall.KEY_QUERY_VALUE, &key)
	if err != nil {
		return "", err
	}
	defer syscall.RegCloseKey(key)
	var valueType uint32
	var size uint32
	name := syscall.StringToUTF16Ptr("WallPaper")
	err = syscall.RegQueryValueEx(key, name, nil, &valueType, nil, &size)
	if err != nil || (valueType != syscall.REG_SZ && valueType != syscall.REG_EXPAND_SZ) {
		return "", err
	}
	buffer := make([]uint16, size/2+1)
	if err := syscall.RegQueryValueEx(key, name, nil, &valueType, (*byte)(unsafe.Pointer(&buffer[0])), &size); err != nil {
		return "", err
	}
	return syscall.UTF16ToString(buffer), nil
}

// imageToJPEG decodes any supported image and re-encodes it as JPEG.
func imageToJPEG(data []byte) ([]byte, error) {
	decoded, _, err := image.Decode(bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	var out bytes.Buffer
	if err := jpeg.Encode(&out, decoded, &jpeg.Options{Quality: 82}); err != nil {
		return nil, err
	}
	return out.Bytes(), nil
}
