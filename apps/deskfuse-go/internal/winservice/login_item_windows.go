//go:build windows

package winservice

import (
	"errors"
	"os"
	"strings"
	"syscall"
	"unsafe"
)

const (
	runKeyPath = `Software\Microsoft\Windows\CurrentVersion\Run`
	runValue   = "DeskFuse"

	hkeyCurrentUser = 0x80000001
	keySetValue     = 0x0002
	regSZ           = 1
)

var (
	advapi32           = syscall.NewLazyDLL("advapi32.dll")
	procRegCreateKeyEx = advapi32.NewProc("RegCreateKeyExW")
	procRegSetValueEx  = advapi32.NewProc("RegSetValueExW")
	procRegDeleteValue = advapi32.NewProc("RegDeleteValueW")
)

// loginItemStatus reports the per-user Run entry as the shared settings page
// expects: "enabled" or "not_registered". Windows has no approval prompt, so
// "requires_approval" never occurs.
func loginItemStatus() (string, error) {
	value, err := readRunValue()
	if err != nil {
		if errors.Is(err, syscall.ERROR_FILE_NOT_FOUND) {
			return "not_registered", nil
		}
		return "", err
	}
	executable, err := os.Executable()
	if err != nil {
		return "", err
	}
	registered := strings.Trim(value, `"`)
	if strings.EqualFold(registered, executable) {
		return "enabled", nil
	}
	// A stale entry that points elsewhere is still a registered start item;
	// enabling again rewrites it.
	if registered != "" {
		return "enabled", nil
	}
	return "not_registered", nil
}

// setLoginItem writes or removes the per-user Run entry for this executable.
func setLoginItem(enabled bool) error {
	executable, err := os.Executable()
	if err != nil {
		return err
	}
	if !enabled {
		return deleteRunValue()
	}
	return writeRunValue(`"` + executable + `"`)
}

func runKey() (syscall.Handle, error) {
	var key syscall.Handle
	err := syscall.RegOpenKeyEx(hkeyCurrentUser, syscall.StringToUTF16Ptr(runKeyPath), 0, syscall.KEY_QUERY_VALUE, &key)
	return key, err
}

func readRunValue() (string, error) {
	key, err := runKey()
	if err != nil {
		return "", err
	}
	defer syscall.RegCloseKey(key)
	var valueType uint32
	var size uint32
	name := syscall.StringToUTF16Ptr(runValue)
	err = syscall.RegQueryValueEx(key, name, nil, &valueType, nil, &size)
	if err != nil {
		return "", err
	}
	if valueType != syscall.REG_SZ && valueType != syscall.REG_EXPAND_SZ {
		return "", syscall.ERROR_FILE_NOT_FOUND
	}
	buffer := make([]uint16, size/2+1)
	if err := syscall.RegQueryValueEx(key, name, nil, &valueType, (*byte)(unsafe.Pointer(&buffer[0])), &size); err != nil {
		return "", err
	}
	return syscall.UTF16ToString(buffer), nil
}

func writeRunValue(value string) error {
	var key syscall.Handle
	var disposition uint32
	status, _, _ := procRegCreateKeyEx.Call(
		hkeyCurrentUser,
		uintptr(unsafe.Pointer(syscall.StringToUTF16Ptr(runKeyPath))),
		0, 0, 0,
		keySetValue,
		0,
		uintptr(unsafe.Pointer(&key)),
		uintptr(unsafe.Pointer(&disposition)),
	)
	if status != 0 {
		return syscall.Errno(status)
	}
	defer syscall.RegCloseKey(key)
	data := syscall.StringToUTF16(value)
	status, _, _ = procRegSetValueEx.Call(
		uintptr(key),
		uintptr(unsafe.Pointer(syscall.StringToUTF16Ptr(runValue))),
		0,
		regSZ,
		uintptr(unsafe.Pointer(&data[0])),
		uintptr(len(data)*2),
	)
	if status != 0 {
		return syscall.Errno(status)
	}
	return nil
}

func deleteRunValue() error {
	var key syscall.Handle
	err := syscall.RegOpenKeyEx(hkeyCurrentUser, syscall.StringToUTF16Ptr(runKeyPath), 0, keySetValue, &key)
	if err != nil {
		if errors.Is(err, syscall.ERROR_FILE_NOT_FOUND) {
			return nil
		}
		return err
	}
	defer syscall.RegCloseKey(key)
	status, _, _ := procRegDeleteValue.Call(
		uintptr(key),
		uintptr(unsafe.Pointer(syscall.StringToUTF16Ptr(runValue))),
	)
	if status != 0 {
		// A missing value already means "not registered".
		if syscall.Errno(status) == syscall.ERROR_FILE_NOT_FOUND {
			return nil
		}
		return syscall.Errno(status)
	}
	return nil
}
