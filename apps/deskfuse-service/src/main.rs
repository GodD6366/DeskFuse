fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    {
        if std::env::args_os().len() != 1 {
            eprintln!(
                "deskfuse-service is started by Windows SCM; use deskfuse-service-client for status"
            );
            return std::process::ExitCode::FAILURE;
        }
        match deskfuse_service::native::run_dispatcher() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("DeskFuse service failed: {error}");
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("deskfuse-service requires Windows SCM; no service started");
        std::process::ExitCode::FAILURE
    }
}
