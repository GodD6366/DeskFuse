//! Windows input helper. Native injection is injected through Host Ports.
mod host;

use host::InputHost;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), String> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("unidesk-input-windows {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let data_dir = std::env::var_os("UNIDESK_INPUT_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("unidesk-input-windows"));
    let host = InputHost::assemble(data_dir);
    host.core.start().await.map_err(|error| error.to_string())?;
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| error.to_string())?;
    host.core.stop().await.map_err(|error| error.to_string())?;
    Ok(())
}
