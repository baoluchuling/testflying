use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{env, path::Path, process::Stdio, time::Duration};
use store_protocol::crypto;
use tokio::{
    process::Command,
    sync::watch,
    time::{interval, sleep, timeout, Instant},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{client::IntoClientRequest, protocol::WebSocketConfig, Message},
};

type Failure = Box<dyn std::error::Error + Send + Sync>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    kind: String,
    connector_id: String,
    token: String,
    server_url: String,
    message_key: String,
}

async fn connection(config: &Config, jobs: &watch::Sender<()>) -> Result<(), Failure> {
    let mut url = reqwest::Url::parse(&config.server_url)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("HTTPS center required".into());
    }
    url.set_scheme("wss").map_err(|_| "Invalid center URL")?;
    let path = format!(
        "{}/v1/connectors/{}/ws",
        url.path().trim_end_matches('/'),
        config.connector_id
    );
    url.set_path(&path);
    let mut request = url.as_str().into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {}", config.token).parse()?);
    request
        .headers_mut()
        .insert("sec-websocket-protocol", crypto::SUBPROTOCOL.parse()?);
    let options = WebSocketConfig::default()
        .max_message_size(Some(crypto::MAX_RECORD_BYTES))
        .max_frame_size(Some(crypto::MAX_RECORD_BYTES));
    let (mut socket, _) = timeout(
        Duration::from_secs(10),
        connect_async_with_config(request, Some(options), false),
    )
    .await??;
    let mut channel = timeout(Duration::from_secs(10), async {
        let mut handshake = crypto::handshake(&config.message_key, &config.connector_id, true)?;
        let registration =
            serde_json::to_vec(&serde_json::json!({"connectorId":config.connector_id}))?;
        let mut buffer = vec![0; crypto::HANDSHAKE_BYTES];
        let size = handshake
            .write_message(&registration, &mut buffer)
            .map_err(|_| crypto::error())?;
        socket
            .send(Message::Binary(buffer[..size].to_vec().into()))
            .await?;
        let Some(Ok(Message::Binary(reply))) = socket.next().await else {
            return Err::<_, Failure>(crypto::error().into());
        };
        handshake
            .read_message(&reply, &mut buffer)
            .map_err(|_| crypto::error())?;
        let mut channel = crypto::Channel::new(handshake)?;
        for record in channel.encrypt(b"confirm")? {
            socket.send(Message::Binary(record.into())).await?;
        }
        let Some(Ok(Message::Binary(reply))) = socket.next().await else {
            return Err(crypto::error().into());
        };
        if channel.decrypt(&reply)?.as_deref() != Some(b"ready") {
            return Err(crypto::error().into());
        }
        Ok(channel)
    })
    .await??;
    let (mut write, mut read) = socket.split();
    let mut heartbeat = interval(Duration::from_secs(10));
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > Duration::from_secs(35) { return Ok(()); }
                timeout(Duration::from_secs(5), write.send(Message::Ping(Vec::new().into()))).await??;
            }
            message = read.next() => {
                let Some(message) = message else { return Ok(()); };
                last_seen = Instant::now();
                match message? {
                    Message::Binary(bytes) => match channel.decrypt(&bytes)?.as_deref() {
                        Some(b"screenshot_job_available") => { jobs.send_replace(()); }
                        Some(_) => return Err(crypto::error().into()),
                        None => {}
                    },
                    Message::Ping(bytes) => timeout(Duration::from_secs(5), write.send(Message::Pong(bytes))).await??,
                    Message::Close(_) => return Ok(()),
                    Message::Text(_) => return Err(crypto::error().into()),
                    _ => {}
                }
            }
        }
    }
}

async fn reconnect(config: &Config, jobs: watch::Sender<()>) {
    let mut delay = 1;
    loop {
        let started = Instant::now();
        let _ = connection(config, &jobs).await;
        if started.elapsed() >= Duration::from_secs(10) {
            delay = 1;
        }
        sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(15);
    }
}

async fn drain(
    config: &Path,
    mut jobs: watch::Receiver<()>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Failure> {
    let python = env::var_os("SCREENSHOT_CONNECTOR_PYTHON").ok_or("Python path is missing")?;
    let worker = config
        .parent()
        .ok_or("Invalid config path")?
        .join("scripts/screenshot_connector.py");
    let mut retry = false;
    loop {
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            result = jobs.changed() => { if result.is_err() { return Ok(()); } }
            _ = sleep(Duration::from_secs(5)), if retry => {}
        }
        if *stop.borrow() {
            return Ok(());
        }
        let mut child = Command::new(&python)
            .args(["-B"])
            .arg(&worker)
            .arg("--drain")
            .arg(config)
            .stdin(Stdio::piped())
            .spawn()?;
        let control = child.stdin.take().ok_or("Worker control pipe is missing")?;
        tokio::select! {
            _ = stop.changed() => {
                drop(control);
                let _ = timeout(Duration::from_secs(50), child.wait()).await;
                return Ok(());
            }
            result = child.wait() => { retry = !result?.success(); }
        }
    }
}

#[cfg(unix)]
async fn shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut terminate = signal(SignalKind::terminate()).expect("register SIGTERM");
    tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
}

#[cfg(not(unix))]
async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let path = env::args_os().nth(1).ok_or("Expected connector.json")?;
    let path = Path::new(&path);
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    if config.kind != "screenshot"
        || config.connector_id.len() != 36
        || !config
            .connector_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-')
    {
        return Err("Invalid screenshot configuration".into());
    }
    let (jobs, changes) = watch::channel(());
    let (stop, stopped) = watch::channel(false);
    let worker = drain(path, changes, stopped);
    tokio::pin!(worker);
    tokio::select! {
        _ = shutdown() => {
            stop.send_replace(true);
            worker.await
        }
        result = &mut worker => result,
        _ = reconnect(&config, jobs) => Ok(()),
    }
}
