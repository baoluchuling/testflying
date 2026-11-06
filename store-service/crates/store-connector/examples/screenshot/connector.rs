use futures_util::{SinkExt, StreamExt};
use std::{env, path::Path, time::Duration};
use store_protocol::crypto;
use tokio::{
    sync::watch,
    time::{interval, sleep, timeout, Instant},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{client::IntoClientRequest, protocol::WebSocketConfig, Message},
};

#[path = "diagnostic.rs"]
mod diagnostic;
#[path = "worker.rs"]
mod worker;
use diagnostic::Log;
use worker::{Config, Worker};

type Failure = Box<dyn std::error::Error + Send + Sync>;

async fn connection(config: &Config, jobs: &watch::Sender<()>, log: &Log) -> Result<(), Failure> {
    let mut url = config.server_url.clone();
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
    log.event(None, "center_connected");
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

async fn reconnect(config: &Config, jobs: watch::Sender<()>, log: &Log) {
    let mut delay = 1;
    loop {
        let started = Instant::now();
        match connection(config, &jobs, log).await {
            Ok(()) => log.event(None, "center_disconnected"),
            Err(_) => log.event(None, "center_connection_failed"),
        }
        if started.elapsed() >= Duration::from_secs(10) {
            delay = 1;
        }
        sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(15);
    }
}

async fn drain(
    worker: &Worker<'_>,
    mut jobs: watch::Receiver<()>,
    mut stop: watch::Receiver<bool>,
) {
    let mut retry = false;
    loop {
        tokio::select! {
            _ = stop.changed() => return,
            result = jobs.changed() => { if result.is_err() { return; } }
            _ = sleep(Duration::from_secs(5)), if retry => {}
        }
        if *stop.borrow() {
            return;
        }
        retry = loop {
            match worker.once(&mut stop).await {
                Ok(true) => {}
                Ok(false) => break false,
                Err(_) => break true,
            }
        };
    }
}

async fn shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut terminate = signal(SignalKind::terminate()).expect("register SIGTERM");
    tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
}

#[tokio::main]
pub async fn run() -> Result<(), Failure> {
    if !cfg!(target_os = "macos") {
        return Err("Screenshot Connector requires macOS".into());
    }
    let mut args = env::args_os().skip(1);
    let first = args.next().ok_or("Expected connector.json")?;
    let once = first == "--once";
    let path = if once {
        args.next().ok_or("Expected connector.json")?
    } else {
        first
    };
    if args.next().is_some() {
        return Err("Usage: screenshot-connector [--once] connector.json".into());
    }
    let path = Path::new(&path);
    let log = Log::for_config(path);
    log.expire_old_entries();
    log.event(
        None,
        if once {
            "connector_started mode=once"
        } else {
            "connector_started mode=service"
        },
    );
    let config = Config::load(path).inspect_err(|_| log.event(None, "config_invalid"))?;
    let worker = Worker::new(&config, path)
        .inspect_err(|_| log.event(None, "center_client_setup_failed"))?;
    let (jobs, changes) = watch::channel(());
    let (stop, mut stopped) = watch::channel(false);
    let work = async {
        if once {
            worker.once(&mut stopped).await?;
        } else {
            drain(&worker, changes, stopped).await;
        }
        Ok(())
    };
    tokio::pin!(work);
    let retention = async {
        loop {
            sleep(Duration::from_secs(60)).await;
            log.expire_old_entries();
        }
    };
    let result = tokio::select! {
        _ = shutdown() => {
            stop.send_replace(true);
            work.await
        }
        result = &mut work => result,
        _ = reconnect(&config, jobs, &log), if !once => Ok(()),
        _ = retention, if !once => Ok(()),
    };
    log.event(None, "connector_stopped");
    result
}
