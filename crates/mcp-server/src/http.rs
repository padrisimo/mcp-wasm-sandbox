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

use crate::error::McpWireError;
use crate::protocol::{Dispatched, dispatch, error_response};
use crate::{PARSE_ERROR, RequestHandler};

/// Builds a router that serves JSON-RPC over HTTP.
///
/// The transport carries no session, so `make` builds a fresh handler for every
/// request: one client can neither lock another one out of the handshake nor
/// block it for the duration of a call.
pub fn router<H, F>(make: F) -> Router
where
    H: RequestHandler + Send + 'static,
    F: Fn() -> H + Send + Sync + 'static,
{
    Router::new()
        .route("/mcp", post(messages::<H, F>))
        .route("/", post(messages::<H, F>))
        .with_state(Arc::new(make))
}

/// Serves JSON-RPC over HTTP on `address` until the process ends.
pub async fn serve_http<H, F>(address: SocketAddr, make: F) -> Result<(), McpWireError>
where
    H: RequestHandler + Send + 'static,
    F: Fn() -> H + Send + Sync + 'static,
{
    let listener = tokio::net::TcpListener::bind(address).await?;
    let address = listener.local_addr()?;
    tracing::info!(%address, "serving json-rpc over http");

    axum::serve(listener, router(make).into_make_service()).await?;

    Ok(())
}

async fn messages<H, F>(State(make): State<Arc<F>>, body: Bytes) -> Response
where
    H: RequestHandler + Send + 'static,
    F: Fn() -> H + Send + Sync + 'static,
{
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

    match dispatch(&mut make(), message).await {
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
    use tower::ServiceExt;

    use super::router;
    use crate::Dispatcher;

    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;

    fn app() -> Result<Router, Box<dyn std::error::Error>> {
        let sandbox = Sandbox::new(SandboxConfig::default())?;
        // The transport has no session: a fresh handler per request.
        Ok(router(move || Dispatcher::stateless(sandbox.clone())))
    }

    async fn post(
        app: &Router,
        body: &str,
    ) -> Result<(StatusCode, serde_json::Value), Box<dyn std::error::Error>> {
        let response = app
            .clone()
            .oneshot(
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
            post(&app()?, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).await?;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["id"], json!(1));
        assert_eq!(value["result"]["tools"][0]["name"], json!("run_wasm"));
        Ok(())
    }

    #[tokio::test]
    async fn every_client_initializes_on_its_own() -> Result<(), Box<dyn std::error::Error>> {
        let app = app()?;

        let (first, _) = post(&app, INITIALIZE).await?;
        let (second, value) = post(&app, INITIALIZE).await?;

        assert_eq!(first, StatusCode::OK);
        assert_eq!(second, StatusCode::OK);
        assert_eq!(value["error"], serde_json::Value::Null);
        Ok(())
    }

    #[tokio::test]
    async fn a_notification_is_accepted_without_a_body() -> Result<(), Box<dyn std::error::Error>> {
        let (status, value) = post(
            &app()?,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .await?;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(value, serde_json::Value::Null);
        Ok(())
    }

    #[tokio::test]
    async fn invalid_json_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let (status, value) = post(&app()?, "{oops").await?;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"]["code"], json!(-32700));
        Ok(())
    }

    #[tokio::test]
    async fn an_unknown_method_answers_with_a_json_rpc_error()
    -> Result<(), Box<dyn std::error::Error>> {
        let (status, value) = post(&app()?, r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#).await?;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["error"]["code"], json!(-32601));
        Ok(())
    }
}
