// Package native bridges the desktop UI host to the platform primitives the
// shared clipboard pages need: the system clipboard, native file dialogs and
// the shell. It reports real outcomes only; a failed native action returns an
// error instead of a fabricated success state.
package native

// Format is one clipboard representation decoded from the service's content
// reply: a MIME identity and its bytes.
type Format struct {
	MIME string
	Data []byte
}

// Filter limits a native file dialog to the given extensions.
type Filter struct {
	Name       string
	Extensions []string
}

// Actions is the platform primitive set the Windows UI adapter calls. The
// Windows implementation uses the host's own toolkit (MyGo) so clipboard
// ownership, dialogs and focus handoff stay in the desktop process; the
// service remains the authority for what goes on the clipboard.
type Actions interface {
	// WriteClipboard replaces the system clipboard with the given formats and
	// optional local file references.
	WriteClipboard(formats []Format, files []string) error
	// PasteClipboard focuses the last non-DeskFuse window and sends Ctrl+V.
	// The caller writes the clipboard first.
	PasteClipboard() error
	// SaveFile shows a native save dialog and writes data to the chosen path.
	// It reports saved=false when the user cancels.
	SaveFile(defaultName string, filters []Filter, data []byte) (path string, saved bool, err error)
	// ChooseFolder shows a native folder picker and reports saved=false when
	// the user cancels.
	ChooseFolder(title string) (path string, saved bool, err error)
	// Reveal selects the paths in the file manager.
	Reveal(paths []string) error
	// OpenPath opens a file or directory with the default application.
	OpenPath(path string) error
	// OpenURL opens an http(s) URL with the default browser.
	OpenURL(url string) error
	// OpenSystemSettings opens a Windows Settings page by name, such as
	// "startupapps". The implementation only accepts known page names.
	OpenSystemSettings(page string) error
	// CopyText replaces the clipboard with plain text.
	CopyText(text string) error
	// PickExe shows a native file picker limited to executables and returns the
	// chosen path. It reports picked=false when the user cancels.
	PickExe(title string) (path string, picked bool, err error)
	// PickFiles shows a native multi-select file picker and returns the chosen
	// paths. It reports picked=false when the user cancels.
	PickFiles(title string) (paths []string, picked bool, err error)
	// AppIcon renders the icon for an executable, as a PNG data URL
	// ("data:image/png;base64,…"), best-effort. It returns an empty string when
	// no icon can be produced.
	AppIcon(path string, size int) (string, error)
	// LocalWallpaper returns the wallpaper of a local display as a JPEG data
	// URL, best-effort. The display identifier matches the display IDs the
	// input settings reference; an empty string means no wallpaper.
	LocalWallpaper(displayID string) (string, error)
	// SaveFileBytes is SaveFile without a forced extension choice dialog filter
	// beyond the supplied ones; it exists so diagnostics exports can suggest a
	// .json name while still letting the user pick any location.
	SaveFileBytes(defaultName string, data []byte) (path string, saved bool, err error)
	// Notify shows a native notification. Grouping/collapse is the caller's job.
	Notify(title, body string) error
}
