//! macOS input subprocess host. The compatibility IPC entry point is assembled
//! by the application host; native adapters are injected through typed ports.
mod host;
#[cfg(target_os = "macos")]
fn run_legacy_input_service() -> Result<(), String> {
    unidesk_engine::input_service::main()
}

use host::InputHost;
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn run_app() -> Result<(), String> {
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

fn main() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if std::env::args().nth(1).as_deref() == Some("--input-service") {
        return run_legacy_input_service();
    }
    run_app()
}
