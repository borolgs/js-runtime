use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};
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

async fn index(runtime: js::Runtime) -> impl IntoResponse {
    runtime.render(None, "root").await.into_response()
}

async fn function(runtime: js::Runtime) -> impl IntoResponse {
    runtime
        .execute_script(js::Script::Function {
            args: Some(json!({"a": 1, "b": 1})),
            code: "console.log('sum'); args.a + args.b".into(),
        })
        .await
        .map(Json)
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

#[derive(FromRef, Clone)]
pub struct AppState {
    runtime: js::Runtime,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "axum_simple_jsx=debug,js=trace,tower_http=debug,axum::rejection=trace".into()
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
        js_src_dir: Some(include_dir::include_dir!("$CARGO_MANIFEST_DIR/src-js")),
        ..Default::default()
    });
    let app = Router::new()
        .route("/", get(index))
        .route("/items", get(items))
        .route("/function", get(function))
        .with_state(AppState { runtime });

    let listener = TcpListener::bind(format!("127.0.0.1:4000")).await?;

    tracing::info!("listening on http://{}", listener.local_addr()?);

    axum::serve(listener, app).await?;

    Ok(())
}
