#[cfg(unix)]
#[path = "screenshot/connector.rs"]
mod connector;

fn main() -> std::process::ExitCode {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--version")) {
        println!("screenshot-connector {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
    }
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
