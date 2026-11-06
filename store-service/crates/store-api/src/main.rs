use std::{env, path::PathBuf, time::Duration};
use store_api::{registry::Registry, ApiState, ScreenshotUploader};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database =
        env::var("STORE_API_DATABASE").unwrap_or_else(|_| "store-registry.sqlite3".into());
    let registry = Registry::open(database, &env::var("STORE_API_NODE_SIGNING_KEY")?)?;
    let timeout = env::var("STORE_API_REQUEST_TIMEOUT_SECONDS")
        .unwrap_or_else(|_| "60".into())
        .parse::<u64>()?;
    if !(1..=300).contains(&timeout) {
        return Err("Request timeout must be between 1 and 300 seconds".into());
    }
    let state = ApiState::new(
        registry,
        env::var("STORE_API_ADMIN_TOKEN")?,
        env::var("STORE_API_TOKEN")?,
        env::var("STORE_API_PUBLIC_URL")?,
        PathBuf::from(
            env::var("STORE_API_CONNECTOR_RELEASE_ROOT").unwrap_or_else(|_| "releases".into()),
        ),
        Duration::from_secs(timeout),
    )?;
    let listener = tokio::net::TcpListener::bind(
        env::var("STORE_API_LISTEN").unwrap_or_else(|_| "0.0.0.0:8200".into()),
    )
    .await?;
    println!("Store API listening on {}", listener.local_addr()?);
    let shutdown = state.clone();
    let uploader = ScreenshotUploader::from_environment().unwrap_or_else(|error| {
        eprintln!("Screenshot delivery unavailable ({})", error.code);
        None
    });
    axum::serve(
        listener,
        store_api::router_with_screenshots(state, uploader),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        shutdown.transport.close_all();
    })
    .await?;
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
