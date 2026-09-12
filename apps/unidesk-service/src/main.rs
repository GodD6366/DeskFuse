fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    {
        if std::env::args_os().len() != 1 {
            eprintln!(
                "unidesk-service is started by Windows SCM; use unidesk-service-client for status"
            );
            return std::process::ExitCode::FAILURE;
        }
        match unidesk_service::native::run_dispatcher() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("uniDesk service failed: {error}");
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("unidesk-service requires Windows SCM; no service started");
        std::process::ExitCode::FAILURE
    }
}
