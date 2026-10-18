fn main() {
    println!("cargo:rerun-if-changed=input-helper.manifest");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("input-helper.manifest");
        // Suppress rustc/link.exe's default uiAccess=false fragment so the
        // helper's explicit uiAccess=true manifest is the sole UAC policy.
        println!("cargo:rustc-link-arg-bin=deskfuse-input-helper=/MANIFESTUAC:NO");
        println!("cargo:rustc-link-arg-bin=deskfuse-input-helper=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=deskfuse-input-helper=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
