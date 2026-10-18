fn main() {
    #[cfg(target_os = "macos")]
    if let Err(error) = deskfuse_update::installer::run_helper() {
        eprintln!("DeskFuse update: {error}");
        std::process::exit(1);
    }
}
