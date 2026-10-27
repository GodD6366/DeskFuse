//! Read-only native diagnostics. Never opens clipboard content, injects input,
//! creates an identity, installs a service, or switches the active desktop.
#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use deskfuse_platform_windows::{
        lan_ipv4_networks, native_displays, session::session_capabilities,
    };
    use serde_json::json;
    let session = session_capabilities();
    let displays = native_displays();
    let networks = lan_ipv4_networks();
    let (network_data, network_error) = match networks {
        Ok(networks) => (
            networks
                .into_iter()
                .map(|(address, prefix)| json!({"address": address.to_string(), "prefix": prefix}))
                .collect::<Vec<_>>(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error)),
    };
    let (display_data, display_error) = match displays {
        Ok(displays) => (
            displays
                .into_iter()
                .map(|d| {
                    json!({
                        "id": d.descriptor.id,
                        "logical": {"x": d.descriptor.frame_x, "y": d.descriptor.frame_y,
                            "width": d.descriptor.width, "height": d.descriptor.height},
                        "physical": {"x": d.physical_left, "y": d.physical_top,
                            "width": d.physical_width, "height": d.physical_height},
                        "scale": d.descriptor.scale_factor,
                        "rotation_degrees": d.descriptor.rotation_degrees,
                        "primary": d.descriptor.is_primary
                    })
                })
                .collect::<Vec<_>>(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let failed =
        display_error.is_some() || session.native_error.is_some() || network_error.is_some();
    println!(
        "{}",
        json!({
            "schema_version": 1,
            "package_version": env!("CARGO_PKG_VERSION"),
            "target": "x86_64-pc-windows-msvc",
            "read_only": true,
            "session": session,
            "displays": display_data,
            "display_error": display_error,
            "lan_networks": network_data,
            "lan_error": network_error,
            "runtime_connected": false,
            "service_installed_by_probe": false
        })
    );
    if failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("deskfuse-native-probe requires Windows; no native operations executed");
    std::process::ExitCode::FAILURE
}
