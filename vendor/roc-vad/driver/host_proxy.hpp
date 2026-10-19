/*
 * Copyright (c) Roc Streaming authors
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

#pragma once

#include <CoreAudio/AudioServerPlugIn.h>

namespace rocvad {

// 包装 coreaudiod 提供的 Host 接口，把 PropertiesChanged 改为在专用串行队列上异步投递。
//
// 进程外驱动的每个 Host 调用都是发往 coreaudiod 的同步 XPC。coreaudiod 处理设备到达等事件时，
// 会在同一个 delegate 队列上同步调用驱动（如 add_device_client）；若此时驱动的 XPC 串行队列
// 正在处理 coreaudiod 的 set_property_data，而 libASPL 在该回调里同步发送 PropertiesChanged，
// 双方互相等待，coreaudiod 对所有进程停止响应。异步投递让 HAL 回调立即返回，打破该环。
// 通知保持原有顺序；其余 Host 方法原样同步转发。
AudioServerPlugInHostRef make_async_host_proxy(AudioServerPlugInHostRef host);

} // namespace rocvad
