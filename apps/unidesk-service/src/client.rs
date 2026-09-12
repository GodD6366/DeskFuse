fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    {
        if std::env::args()
            .collect::<Vec<_>>()
            .get(1..)
            .is_none_or(|args| args != ["status"])
        {
            eprintln!("usage: unidesk-service-client status");
            return std::process::ExitCode::FAILURE;
        }
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .and_then(|runtime| runtime.block_on(unidesk_service::native::query_status()));
        match result {
            Ok(status) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&status).expect("status is JSON serializable")
                );
                std::process::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("uniDesk service unavailable: {error}");
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("unidesk-service-client requires Windows; no service connection attempted");
        std::process::ExitCode::FAILURE
    }
}
