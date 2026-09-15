fn main() {
    #[cfg(target_os = "macos")]
    if unidesk_engine::input_service::main().is_err() {
        std::process::exit(1);
    }
}
