/*
 * Copyright (c) Roc Streaming authors
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

#include "host_proxy.hpp"

#include <dispatch/dispatch.h>

#include <atomic>
#include <cassert>
#include <memory>
#include <vector>

namespace rocvad {

namespace {

// 驱动进程只有一个 Host；代理与真实 Host 均在进程生命周期内保持有效。
std::atomic<AudioServerPlugInHostRef> real_host{nullptr};
AudioServerPlugInHostInterface proxy_host{};

struct Notification {
    AudioServerPlugInHostRef host;
    AudioObjectID object_id;
    std::vector<AudioObjectPropertyAddress> addresses;
};

dispatch_queue_t notification_queue()
{
    static dispatch_queue_t queue =
        dispatch_queue_create("com.deskfuse.microphone.host-notifications",
            DISPATCH_QUEUE_SERIAL);
    return queue;
}

OSStatus properties_changed(AudioServerPlugInHostRef,
    AudioObjectID object_id,
    UInt32 count,
    const AudioObjectPropertyAddress addresses[])
{
    auto host = real_host.load();
    if (!host) {
        return kAudioHardwareNotRunningError;
    }
    auto* notification = new Notification{host, object_id,
        std::vector<AudioObjectPropertyAddress>(addresses, addresses + count)};
    dispatch_async_f(notification_queue(), notification, [](void* context) {
        std::unique_ptr<Notification> n(static_cast<Notification*>(context));
        n->host->PropertiesChanged(
            n->host, n->object_id, UInt32(n->addresses.size()), n->addresses.data());
    });
    return kAudioHardwareNoError;
}

OSStatus copy_from_storage(AudioServerPlugInHostRef,
    CFStringRef key,
    CFPropertyListRef* data)
{
    auto host = real_host.load();
    return host->CopyFromStorage(host, key, data);
}

OSStatus write_to_storage(AudioServerPlugInHostRef, CFStringRef key, CFPropertyListRef data)
{
    auto host = real_host.load();
    return host->WriteToStorage(host, key, data);
}

OSStatus delete_from_storage(AudioServerPlugInHostRef, CFStringRef key)
{
    auto host = real_host.load();
    return host->DeleteFromStorage(host, key);
}

OSStatus request_device_configuration_change(AudioServerPlugInHostRef,
    AudioObjectID device_id,
    UInt64 change_action,
    void* change_info)
{
    auto host = real_host.load();
    return host->RequestDeviceConfigurationChange(host, device_id, change_action, change_info);
}

} // namespace

AudioServerPlugInHostRef make_async_host_proxy(AudioServerPlugInHostRef host)
{
    assert(host);
    real_host = host;
    proxy_host.PropertiesChanged = properties_changed;
    proxy_host.CopyFromStorage = copy_from_storage;
    proxy_host.WriteToStorage = write_to_storage;
    proxy_host.DeleteFromStorage = delete_from_storage;
    proxy_host.RequestDeviceConfigurationChange = request_device_configuration_change;
    return &proxy_host;
}

} // namespace rocvad
