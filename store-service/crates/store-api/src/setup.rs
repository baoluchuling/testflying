use axum::{
    extract::{Path, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;
use uuid::Uuid;

use super::{bearer, registry::secret_eq, ApiState, Result, StoreError};

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";
const SESSION_COOKIE: &str = "__Secure-tf-setup";
const SESSION_HEADER: &str = "x-testflying-setup";
const SESSION_LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);

#[derive(Default)]
pub(super) struct Sessions(Mutex<HashMap<Uuid, Instant>>);

fn session_id(headers: &HeaderMap) -> Option<Uuid> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|cookie| cookie.trim().split_once('='))
        .find_map(|(name, value)| {
            (name == SESSION_COOKIE)
                .then(|| Uuid::parse_str(value).ok())
                .flatten()
        })
}

fn session_response(value: &str, expires: &str) -> Response {
    // The browser's default cookie path includes any reverse proxy prefix.
    let cookie = format!("{SESSION_COOKIE}={value}; Secure; HttpOnly; SameSite=Strict{expires}");
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
}

async fn login(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> Result<Response> {
    let now = Instant::now();
    let mut sessions = state.setup_sessions.0.lock().unwrap();
    sessions.retain(|_, expires| *expires > now);
    if let Some(id) = session_id(&headers) {
        sessions.remove(&id);
    }
    if sessions.len() >= 128 {
        return Err(StoreError::new(
            503,
            "setup_sessions_full",
            "Administrator session limit reached",
        )
        .into());
    }
    let id = Uuid::new_v4();
    sessions.insert(id, now + SESSION_LIFETIME);
    Ok(session_response(&id.to_string(), ""))
}

async fn logout(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> Result<Response> {
    if !session_request(&headers) {
        return Err(StoreError::new(
            403,
            "invalid_setup_request",
            "Setup request header required",
        )
        .into());
    }
    if let Some(id) = session_id(&headers) {
        state.setup_sessions.0.lock().unwrap().remove(&id);
    }
    Ok(session_response("", "; Max-Age=0"))
}

fn session_request(headers: &HeaderMap) -> bool {
    headers
        .get(SESSION_HEADER)
        .is_some_and(|value| value == "1")
}

async fn authorize_setup(
    State(state): State<Arc<ApiState>>,
    request: Request,
    next: Next,
) -> Result<Response> {
    let headers = request.headers();
    let session_valid = session_request(headers)
        && session_id(headers).is_some_and(|id| {
            state
                .setup_sessions
                .0
                .lock()
                .unwrap()
                .get(&id)
                .is_some_and(|expires| *expires > Instant::now())
        });
    if !session_valid && !secret_eq(&state.admin_token, bearer(headers)) {
        return Err(StoreError::new(
            401,
            "unauthorized",
            "Administrator token or setup session required",
        )
        .into());
    }
    let mut response = next.run(request).await;
    response.extensions_mut().insert(super::logs::Actor {
        kind: "administrator-session",
        id: None,
    });
    Ok(response)
}

pub(super) fn routes(state: Arc<super::ApiState>) -> Router<Arc<super::ApiState>> {
    let setup_api = Router::new()
        .route("/connector-setup/api/logs", get(super::logs::read))
        .route(
            "/connector-setup/api/connectors",
            get(super::list_connectors).post(super::create_connector),
        )
        .route(
            "/connector-setup/api/connectors/{id}/installer/{target}",
            get(super::installer),
        )
        .route(
            "/connector-setup/api/connectors/{id}/screenshot-installer",
            get(super::screenshots::screenshot_installer),
        )
        .merge(super::integrations::management_routes(
            "/connector-setup/api/integrations",
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            authorize_setup,
        ))
        .route(
            "/connector-setup/api/session",
            post(login)
                .route_layer(middleware::from_fn_with_state(
                    state,
                    super::authorize_admin,
                ))
                .delete(logout),
        )
        .route_layer(middleware::from_fn(super::integrations::authorize_https))
        .route_layer(middleware::map_response(|response| async {
            secure(response)
        }));

    Router::new()
        .route(
            "/connector-setup",
            get(|| async { secure(Redirect::temporary("./connector-setup/").into_response()) }),
        )
        .route(
            "/connector-setup/",
            get(|| async {
                document(
                    "text/html; charset=utf-8",
                    include_bytes!("setup/index.html"),
                )
            }),
        )
        .route("/connector-setup/{asset}", get(asset))
        .merge(setup_api)
}

fn secure(mut response: Response) -> Response {
    for (key, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::CONTENT_SECURITY_POLICY, CSP),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::X_FRAME_OPTIONS, "DENY"),
    ] {
        response.headers_mut().insert(key, value.parse().unwrap());
    }
    response
}

fn document(content_type: &'static str, bytes: &'static [u8]) -> Response {
    secure(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
}

async fn asset(Path(name): Path<String>) -> Response {
    let (kind, bytes): (&str, &[u8]) = match name.as_str() {
        "app.mjs" => (
            "text/javascript; charset=utf-8",
            include_bytes!("setup/app.mjs"),
        ),
        "integrations.mjs" => (
            "text/javascript; charset=utf-8",
            include_bytes!("setup/integrations.mjs"),
        ),
        "logs.mjs" => (
            "text/javascript; charset=utf-8",
            include_bytes!("setup/logs.mjs"),
        ),
        "packager.mjs" => (
            "text/javascript; charset=utf-8",
            include_bytes!("setup/packager.mjs"),
        ),
        "style.css" => ("text/css; charset=utf-8", include_bytes!("setup/style.css")),
        "jszip.min.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("setup/vendor/jszip.min.js"),
        ),
        "license.txt" => (
            "text/plain; charset=utf-8",
            include_bytes!("setup/vendor/JSZIP-LICENSE.markdown"),
        ),
        "ca.crt" => (
            "application/x-pem-file",
            include_bytes!("../../../deploy/testflying-prod-ca.crt"),
        ),
        "start.cmd" => (
            "text/plain; charset=utf-8",
            include_bytes!("../../../packaging/connector/start.cmd"),
        ),
        "start.sh" => (
            "text/plain; charset=utf-8",
            include_bytes!("../../../packaging/connector/start.sh"),
        ),
        "install.cmd" => (
            "text/plain; charset=utf-8",
            include_bytes!("../../../packaging/connector/install.cmd"),
        ),
        "install.ps1" => (
            "text/plain; charset=utf-8",
            include_bytes!("../../../packaging/connector/install.ps1"),
        ),
        "install.sh" => (
            "text/plain; charset=utf-8",
            include_bytes!("../../../packaging/connector/install.sh"),
        ),
        _ => return secure(StatusCode::NOT_FOUND.into_response()),
    };
    document(kind, bytes)
}
