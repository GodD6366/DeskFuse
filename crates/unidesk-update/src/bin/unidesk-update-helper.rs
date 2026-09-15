fn main() {
    #[cfg(target_os = "macos")]
    if let Err(error) = unidesk_update::installer::run_helper() {
        eprintln!("uniDesk update: {error}");
        std::process::exit(1);
    }
}
