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
    let notifications = tokio::spawn(store_api::run_screenshot_notifications(state.clone()));
    let store_jobs = tokio::spawn(store_api::run_store_image_jobs(state.clone()));
    state.registry.logs.record_service("service.started");
    let maintenance_logs = state.registry.logs.clone();
    let maintenance = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let logs = maintenance_logs.clone();
            let _ = tokio::task::spawn_blocking(move || logs.prune()).await;
        }
    });
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
        store_jobs.abort();
        let _ = store_jobs.await;
        notifications.abort();
        let _ = notifications.await;
        shutdown.registry.logs.record_service("service.stopping");
        shutdown.transport.close_all();
    })
    .await?;
    maintenance.abort();
    let _ = maintenance.await;
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
