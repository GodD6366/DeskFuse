//go:build !windows

package winservice

import "errors"

func setKeepAwake(bool) error { return errors.New("此平台没有保持唤醒适配层") }
