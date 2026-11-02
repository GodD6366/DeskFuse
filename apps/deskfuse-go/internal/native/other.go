//go:build !windows

package native

import "errors"

// New returns a stub on hosts where the native Windows adapter does not
// apply; those builds use their own bridge and never call these primitives.
func New() Actions { return unavailable{} }

type unavailable struct{}

var errUnavailable = errors.New("此平台没有本地剪贴板适配层")

func (unavailable) WriteClipboard([]Format, []string) error { return errUnavailable }
func (unavailable) PasteClipboard() error                   { return errUnavailable }
func (unavailable) SaveFile(string, []Filter, []byte) (string, bool, error) {
	return "", false, errUnavailable
}
func (unavailable) ChooseFolder(string) (string, bool, error) { return "", false, errUnavailable }
func (unavailable) Reveal([]string) error                     { return errUnavailable }
func (unavailable) OpenPath(string) error                     { return errUnavailable }
func (unavailable) OpenURL(string) error                      { return errUnavailable }
func (unavailable) OpenSystemSettings(string) error           { return errUnavailable }
func (unavailable) CopyText(string) error                     { return errUnavailable }
func (unavailable) PickExe(string) (string, bool, error)      { return "", false, errUnavailable }
func (unavailable) PickFiles(string) ([]string, bool, error)   { return nil, false, errUnavailable }
func (unavailable) AppIcon(string, int) (string, error)       { return "", errUnavailable }
func (unavailable) LocalWallpaper(string) (string, error)     { return "", errUnavailable }
func (unavailable) SaveFileBytes(string, []byte) (string, bool, error) {
	return "", false, errUnavailable
}
func (unavailable) Notify(string, string) error { return errUnavailable }
