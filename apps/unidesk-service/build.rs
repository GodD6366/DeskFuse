fn main() {
    println!("cargo:rerun-if-changed=input-helper.manifest");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("input-helper.manifest");
        println!("cargo:rustc-link-arg-bin=unidesk-input-helper=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=unidesk-input-helper=/MANIFESTINPUT:{}", manifest.display());
    }
}
