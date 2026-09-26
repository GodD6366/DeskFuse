fn main() {
    #[cfg(any(target_os = "macos", windows))]
    if let Err(error) = unidesk_update::installer::run_helper() {
        eprintln!("uniDesk update: {error}");
        std::process::exit(1);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    eprintln!("uniDesk update helper is unavailable on this platform");
}
