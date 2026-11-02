//go:build !windows

package winservice

import "errors"

func loginItemStatus() (string, error) { return "", errors.New("此平台没有登录项适配层") }
func setLoginItem(bool) error          { return errors.New("此平台没有登录项适配层") }
