fn main() {
    #[cfg(any(target_os = "macos", windows))]
    if let Err(error) = deskfuse_update_platform::run_helper() {
        eprintln!("DeskFuse update: {error}");
        std::process::exit(1);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    eprintln!("DeskFuse update helper is unavailable on this platform");
}
