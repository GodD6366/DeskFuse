fn main() {
    #[cfg(target_os = "macos")]
    if let Err(error) = deskfuse_ffi::run_input_service() {
        eprintln!("DeskFuse input service: {error}");
        std::process::exit(1);
    }
}
