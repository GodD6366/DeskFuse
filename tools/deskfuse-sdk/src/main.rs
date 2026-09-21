//! Hash a built SDK directory and write `manifest.json`.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize)]
struct SdkManifest {
    version: String,
    core_hash: String,
    api_hash: String,
    helper_hash: Option<String>,
    uniffi_version: String,
    min_macos: String,
    built_at_unix: u64,
}

fn hash_file(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(hex(Sha256::digest(&bytes).as_slice()))
}

fn hash_api_surface(root: &Path) -> String {
    let files = [
        "crates/deskfuse-collaboration/src/api.rs",
        "crates/deskfuse-platform/src/ports/mod.rs",
        "crates/deskfuse-platform/src/ports/clipboard.rs",
        "crates/deskfuse-platform/src/ports/file.rs",
        "crates/deskfuse-platform/src/ports/input.rs",
    ];
    let mut hasher = Sha256::new();
    for relative in files {
        if let Ok(content) = fs::read_to_string(root.join(relative)) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("pub ") {
                    hasher.update(trimmed.as_bytes());
                    hasher.update(b"\n");
                }
            }
        }
    }
    hex(hasher.finalize().as_slice())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn arg(flag: &str) -> Option<String> {
    env::args().skip_while(|arg| arg != flag).nth(1)
}

fn main() {
    let out_dir =
        PathBuf::from(arg("--out-dir").unwrap_or_else(|| "apps/deskfuse-macos/SDK".into()));
    let version = arg("--version").unwrap_or_else(|| env!("CARGO_PKG_VERSION").into());
    let root = PathBuf::from(arg("--root").unwrap_or_else(|| ".".into()));
    fs::create_dir_all(&out_dir).expect("create SDK directory");
    let manifest = SdkManifest {
        version,
        core_hash: hash_file(&out_dir.join("libdeskfuse_collaboration.a"))
            .unwrap_or_else(|| "missing".into()),
        api_hash: hash_api_surface(&root),
        helper_hash: hash_file(&out_dir.join("deskfuse-input-macos")),
        uniffi_version: "0.32.1".into(),
        min_macos: "14.0".into(),
        built_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
    };
    let path = out_dir.join("manifest.json");
    fs::write(&path, serde_json::to_vec_pretty(&manifest).expect("json")).expect("write manifest");
    println!("SDK manifest: {}", path.display());
}
