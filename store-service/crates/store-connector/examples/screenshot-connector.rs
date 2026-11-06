#[cfg(unix)]
#[path = "screenshot/connector.rs"]
mod connector;

fn main() -> std::process::ExitCode {
    #[cfg(unix)]
    let result = connector::run();
    #[cfg(not(unix))]
    let result = Err::<(), _>("Screenshot Connector requires macOS");

    if result.is_err() {
        eprintln!("Screenshot Connector could not start or complete the requested operation");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
