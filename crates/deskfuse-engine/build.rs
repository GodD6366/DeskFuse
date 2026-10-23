use std::{env, fs, path::PathBuf};

fn main() {
    let directory = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../config")
        .canonicalize()
        .expect("product configuration directory");
    let product_path = directory.join("deskfuse.json");
    println!("cargo:rerun-if-changed={}", product_path.display());
    let product: serde_json::Value =
        serde_json::from_slice(&fs::read(&product_path).unwrap()).unwrap();
    assert_eq!(product["schema_version"], 1);
    let active_path = directory
        .join(product["active_distribution"].as_str().unwrap())
        .canonicalize()
        .unwrap();
    assert!(active_path.starts_with(&directory));
    println!("cargo:rerun-if-changed={}", active_path.display());
    let active: serde_json::Value =
        serde_json::from_slice(&fs::read(&active_path).unwrap()).unwrap();
    assert_eq!(active["schema_version"], 1);
    let repository = active["release_repository"].as_str().unwrap();
    let parts: Vec<_> = repository.split('/').collect();
    assert!(
        parts.len() == 2
            && parts.iter().all(|part| !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte)))
    );
    // 站点在自定义域名下重发同形的发行列表，供 api.github.com 不可达或限流时使用
    let feed = active["release_feed_url"].as_str().unwrap();
    assert!(
        feed.starts_with("https://")
            && feed
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !b"\"\\".contains(&byte)),
        "release feed must be a plain https URL"
    );
    // 站点把每个版本的元数据镜像在发行列表旁边，路径与 GitHub 资产一致
    let feed_files = feed
        .strip_suffix("releases.json")
        .map(|base| format!("{base}releases"))
        .expect("release feed must end with releases.json");
    let constants = [
        ("FEED", feed.to_owned()),
        ("FEED_FILES", feed_files),
        (
            "API",
            format!("https://api.github.com/repos/{repository}/releases?per_page=100"),
        ),
        (
            "DOWNLOAD",
            format!("https://github.com/{repository}/releases/download"),
        ),
        (
            "RELEASES",
            format!("https://github.com/{repository}/releases/tag"),
        ),
    ];
    let generated: String = constants
        .into_iter()
        .map(|(name, value)| format!("const {name}: &str = {value:?};\n"))
        .collect();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(output.join("distribution.rs"), generated).unwrap();
}
