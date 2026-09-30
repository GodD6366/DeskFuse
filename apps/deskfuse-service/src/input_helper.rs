#[cfg(windows)]
fn main() {
    let mut args = std::env::args().skip(1);
    let parent = match (args.next().as_deref(), args.next(), args.next()) {
        (Some("--parent"), Some(value), None) => value.parse::<u32>().ok(),
        _ => None,
    };
    let Some(parent) = parent.filter(|value| *value != 0 && *value != std::process::id()) else {
        eprintln!("deskfuse-input-helper requires one valid --parent PID");
        std::process::exit(2);
    };
    if let Err(error) = deskfuse_service::native::run_input_helper(parent) {
        eprintln!("DeskFuse input helper stopped: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("deskfuse-input-helper requires Windows");
    std::process::exit(1);
}
