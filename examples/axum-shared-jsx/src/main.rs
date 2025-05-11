use axum::{
    Router,
    body::Body,
    extract::Path,
    http::{HeaderValue, Response, StatusCode, Uri, header},
    response::IntoResponse,
    routing::get,
};
use tracing_subscriber::prelude::*;

use axum_macros::FromRef;
use serde::Serialize;
use serde_json::json;
use tokio::net::TcpListener;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("unexpected")]
    Unexpected(String),
}

#[derive(Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum ErrorResponse {
    Unexpected { message: String },
}

impl From<Error> for ErrorResponse {
    fn from(error: Error) -> Self {
        match error {
            Error::Unexpected(message) => Self::Unexpected { message },
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        let status = match self {
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let mut res = axum::Json(ErrorResponse::from(self)).into_response();
        *res.status_mut() = status;
        res
    }
}

static STATIC_DIR: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/static");

#[derive(FromRef, Clone)]
pub struct AppState {
    runtime: js::Runtime,
}

async fn index(runtime: js::Runtime) -> impl IntoResponse {
    runtime.render(None, "root").await.into_response()
}

async fn items(runtime: js::Runtime) -> impl IntoResponse {
    let items = json!({
        "items": [
            { "id": 1, "name": "Item A", "description": "This is the first item." },
            { "id": 2, "name": "Item B", "description": "Another useful item." },
            { "id": 3, "name": "Item C", "description": "Yet another item here." }
        ]
    });
    runtime.render(Some(items), "items").await.into_response()
}

async fn spa_index() -> impl IntoResponse {
    if let Some(index) = STATIC_DIR.get_file("app/index.html") {
        return Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_str("text/html").unwrap(),
            )
            .body(Body::from(index.contents().to_owned()))
            .unwrap();
    };

    StatusCode::NOT_FOUND.into_response()
}

async fn assets(Path(path): Path<String>, uri: Uri) -> impl IntoResponse {
    let path = if path.starts_with("assets") {
        uri.to_string().trim_start_matches('/').to_string()
    } else {
        path
    };

    if let Some(file) = STATIC_DIR.get_file(&path) {
        let mime_type = mime_guess::from_path(path).first_or_text_plain();

        return Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime_type.as_ref()).unwrap(),
            )
            .body(Body::from(file.contents().to_owned()))
            .unwrap();
    }

    StatusCode::NOT_FOUND.into_response()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "axum_shared_jsx=debug,js=trace,tower_http=debug,axum::rejection=trace".into()
            }),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_file(true)
                .with_line_number(true)
                .with_thread_ids(true)
                .with_target(false),
        )
        .try_init()
        .ok();

    let runtime = js::Runtime::new(js::RuntimeConfig {
        workers: 1,
        js_src_dir: Some(include_dir::include_dir!("$CARGO_MANIFEST_DIR/src-web")),
        pages_dir: "pages/server".into(),
        ..Default::default()
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/items", get(items))
        .route("/app", get(spa_index))
        .route("/app/{*path}", get(assets))
        .route("/assets/{*path}", get(assets))
        .with_state(AppState { runtime });

    let listener = TcpListener::bind(format!("127.0.0.1:4000")).await?;

    tracing::info!("listening on http://{}", listener.local_addr()?);

    axum::serve(listener, app).await?;

    Ok(())
}
