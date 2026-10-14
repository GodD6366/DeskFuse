//! Wi-Fi environment observation for input layout binding.
//!
//! Mirrors the macOS engine's CoreWLAN polling: the service samples the
//! current WLAN SSID and forwards it to the input actor, which binds saved
//! layout scenes to the network and restores them when it returns. The SSID
//! is only a layout switch key, never an authorization input.
#![allow(unsafe_code)]

use std::time::Duration;

use deskfuse_runtime::{RuntimeCommand, RuntimeHandle};

/// Forward SSID changes to the input actor; stops when the runtime goes away.
pub(super) fn spawn_observer(runtime: RuntimeHandle) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            // WLAN APIs block on the auto-config service; keep them off the
            // async executor. A failed probe means "no stable environment",
            // which is exactly what the actor expects on wired links.
            let ssid = tokio::task::spawn_blocking(current_ssid)
                .await
                .unwrap_or_default();
            if runtime
                .send(RuntimeCommand::ObserveWifiEnvironment { ssid })
                .await
                .is_err()
            {
                break;
            }
        }
    });
}

fn current_ssid() -> Option<String> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::NetworkManagement::WiFi::{
        WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST, WLAN_INTF_OPCODE, WlanCloseHandle,
        WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle, WlanQueryInterface,
        wlan_interface_state_connected, wlan_intf_opcode_current_connection,
    };

    // `WLAN_INTERFACE_INFO_LIST::InterfaceInfo` is a flexible array; index it
    // manually instead of relying on the length-1 declaration.
    #[allow(clippy::cast_ptr_alignment)]
    unsafe {
        let mut client: HANDLE = std::ptr::null_mut();
        let mut negotiated = 0_u32;
        if WlanOpenHandle(2, std::ptr::null(), &mut negotiated, &mut client) != 0 {
            return None;
        }
        let ssid = (|| {
            let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
            if WlanEnumInterfaces(client, std::ptr::null(), &mut list) != 0 || list.is_null() {
                return None;
            }
            let ssid = (0..(*list).dwNumberOfItems as usize).find_map(|index| {
                let info = std::ptr::addr_of!((*list).InterfaceInfo) as *const WLAN_INTERFACE_INFO;
                let info = info.add(index);
                if (*info).isState != wlan_interface_state_connected {
                    return None;
                }
                let mut size = 0_u32;
                let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
                let opcode: WLAN_INTF_OPCODE = wlan_intf_opcode_current_connection;
                if WlanQueryInterface(
                    client,
                    &(*info).InterfaceGuid,
                    opcode,
                    std::ptr::null(),
                    &mut size,
                    &mut data,
                    std::ptr::null_mut(),
                ) != 0
                    || data.is_null()
                {
                    return None;
                }
                let attributes = data as *const windows_sys::Win32::NetworkManagement::WiFi::WLAN_CONNECTION_ATTRIBUTES;
                let length = (*attributes).wlanAssociationAttributes.dot11Ssid.uSSIDLength;
                let ssid_bytes: [u8; 32] = std::ptr::read(std::ptr::addr_of!(
                    (*attributes).wlanAssociationAttributes.dot11Ssid.ucSSID
                ));
                let ssid = if (1..=32).contains(&length) {
                    String::from_utf8_lossy(&ssid_bytes[..length as usize])
                        .trim()
                        .to_owned()
                } else {
                    String::new()
                };
                WlanFreeMemory(data);
                if ssid.is_empty() { None } else { Some(ssid) }
            });
            WlanFreeMemory(list.cast());
            ssid
        })();
        WlanCloseHandle(client, std::ptr::null());
        ssid
    }
}

#[cfg(test)]
mod tests {
    use super::current_ssid;

    #[test]
    fn ssid_probe_is_best_effort_and_never_panics() {
        // Machines without a WLAN adapter or the auto-config service simply
        // report no environment; binding then stays disabled, same as wired.
        let _ = current_ssid();
    }
}
