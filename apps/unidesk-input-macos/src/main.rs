//! macOS input subprocess host. Production capture/injection still lives in
//! `unidesk-engine`'s input service until native Host Ports are wired here.
mod host;

use host::InputHost;
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("unidesk-input-macos {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let data_dir = std::env::var_os("UNIDESK_INPUT_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("unidesk-input-macos"));
    let host = InputHost::assemble(data_dir);
    host.start().await.map_err(|error| error.to_string())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = async {
            loop {
                tokio::time::sleep(InputHost::watchdog_interval()).await;
                if !host.core.input().has_active_lease().await {
                    continue;
                }
            }
        } => {}
    }
    host.stop().await.map_err(|error| error.to_string())?;
    Ok(())
}
