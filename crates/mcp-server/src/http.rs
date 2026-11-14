//! HTTP transport for JSON-RPC 2.0.
//!
//! The endpoint follows the MCP streamable HTTP shape for the client to server
//! direction: a POST carrying one JSON-RPC message, answered with the JSON-RPC
//! response. Notifications are accepted without a body, and the server never
//! initiates messages, so no server to client stream is needed.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use tokio::sync::Mutex;

use crate::error::McpWireError;
use crate::protocol::{Dispatched, dispatch, error_response};
use crate::{PARSE_ERROR, RequestHandler};

/// The handler shared by every request.
type Shared<H> = Arc<Mutex<H>>;

/// Builds the router that serves JSON-RPC over HTTP.
pub fn router<H: RequestHandler + Send + 'static>(handler: H) -> Router {
    let state: Shared<H> = Arc::new(Mutex::new(handler));

    Router::new()
        .route("/mcp", post(messages::<H>))
        .route("/", post(messages::<H>))
        .with_state(state)
}

/// Serves JSON-RPC over HTTP on `address` until the process ends.
pub async fn serve_http<H: RequestHandler + Send + 'static>(
    address: SocketAddr,
    handler: H,
) -> Result<(), McpWireError> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    let address = listener.local_addr()?;
    tracing::info!(%address, "serving json-rpc over http");

    axum::serve(listener, router(handler).into_make_service()).await?;

    Ok(())
}

async fn messages<H: RequestHandler + Send + 'static>(
    State(handler): State<Shared<H>>,
    body: Bytes,
) -> Response {
    let message = match std::str::from_utf8(&body) {
        Ok(message) => message,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(error_response(PARSE_ERROR, "invalid JSON")),
            )
                .into_response();
        }
    };

    let mut handler = handler.lock().await;

    match dispatch(&mut *handler, message).await {
        Dispatched::Response(response) => (StatusCode::OK, axum::Json(response)).into_response(),
        Dispatched::Malformed(response) => {
            (StatusCode::BAD_REQUEST, axum::Json(response)).into_response()
        }
        Dispatched::Notification => StatusCode::ACCEPTED.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use sandbox_core::{Sandbox, SandboxConfig};
    use serde_json::json;
    use tower::Service;

    use super::router;
    use crate::Dispatcher;

    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;

    fn app() -> Result<Router, Box<dyn std::error::Error>> {
        let dispatcher = Dispatcher::new(Sandbox::new(SandboxConfig::default())?);
        Ok(router(dispatcher))
    }

    async fn post(
        app: Router,
        body: &str,
    ) -> Result<(StatusCode, serde_json::Value), Box<dyn std::error::Error>> {
        let mut app = app;

        // The lifecycle requires the handshake before any other request.
        app.call(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(INITIALIZE))?,
        )
        .await?;

        let response = app
            .call(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))?,
            )
            .await?;

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await?;
        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes)?
        };

        Ok((status, value))
    }

    #[tokio::test]
    async fn a_request_is_answered_over_http() -> Result<(), Box<dyn std::error::Error>> {
        let (status, value) =
            post(app()?, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).await?;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["id"], json!(1));
        assert_eq!(value["result"]["tools"][0]["name"], json!("run_wasm"));
        Ok(())
    }

    #[tokio::test]
    async fn a_notification_is_accepted_without_a_body() -> Result<(), Box<dyn std::error::Error>> {
        let (status, value) = post(
            app()?,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .await?;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(value, serde_json::Value::Null);
        Ok(())
    }

    #[tokio::test]
    async fn invalid_json_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let (status, value) = post(app()?, "{oops").await?;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"]["code"], json!(-32700));
        Ok(())
    }

    #[tokio::test]
    async fn an_unknown_method_answers_with_a_json_rpc_error()
    -> Result<(), Box<dyn std::error::Error>> {
        let (status, value) = post(app()?, r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#).await?;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["error"]["code"], json!(-32601));
        Ok(())
    }
}
