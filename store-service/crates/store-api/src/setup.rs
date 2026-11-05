use axum::{
    extract::Path,
    http::{header, StatusCode},
    middleware,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use std::sync::Arc;

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";

pub(super) fn routes() -> Router<Arc<super::ApiState>> {
    let setup_api = Router::new()
        .route(
            "/connector-setup/api/connectors",
            get(super::list_connectors).post(super::create_connector),
        )
        .route(
            "/connector-setup/api/connectors/{id}/installer/{target}",
            get(super::installer),
        )
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
