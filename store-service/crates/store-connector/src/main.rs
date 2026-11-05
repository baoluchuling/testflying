use std::{env, time::Duration};
use store_connector::{ConnectorConfig, Executor};
use tokio::sync::watch;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "connector.json".into());
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
