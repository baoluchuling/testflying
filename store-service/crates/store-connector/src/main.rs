mod install;

use std::{env, time::Duration};
use store_connector::{ConnectorConfig, Executor};
use tokio::sync::watch;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args_os().skip(1);
    let first = arguments.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--prepare-install")) {
        let paths: Vec<_> = arguments.map(std::path::PathBuf::from).collect();
        if paths.len() != 3 {
            return Err("Installation preparation requires package, installed-data and prepared directories".into());
        }
        install::prepare(&paths[0], &paths[1], &paths[2])?;
        return Ok(());
    }
    let path = first.unwrap_or_else(|| "connector.json".into());
    let config: ConnectorConfig = serde_json::from_slice(&std::fs::read(path)?)?;
    let executor = Executor::new(config.clone())?;
    let (stop, stopped) = watch::channel(false);
    let shutdown = tokio::spawn(async move {
        shutdown_signal().await;
        let _ = stop.send(true);
    });
    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    println!(
        "Connector {} listening on {}",
        config.connector_id,
        listener.local_addr()?
    );
    let result = store_connector::run(executor.clone(), listener, stopped).await;
    let _ = tokio::time::timeout(Duration::from_secs(300), executor.drain()).await;
    shutdown.abort();
    let _ = shutdown.await;
    result?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate =
            signal(SignalKind::terminate()).expect("Cannot register SIGTERM handler");
        tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = terminate.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
