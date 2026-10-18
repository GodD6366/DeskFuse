use std::{env, path::PathBuf};

fn main() {
    let plist = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("resources/input-service-info.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    // Native input APIs can register this child with LaunchServices. Give only
    // the helper its own background identity, before any native API is called.
    // The Swift application's Dock policy and FFI libraries remain independent.
    for argument in [
        "-Xlinker",
        "-sectcreate",
        "-Xlinker",
        "__TEXT",
        "-Xlinker",
        "__info_plist",
        "-Xlinker",
        plist.to_str().expect("input service plist path"),
    ] {
        println!("cargo:rustc-link-arg-bin=deskfuse-input-service={argument}");
    }
}
