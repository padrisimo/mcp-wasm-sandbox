//! JSON-RPC message dispatch, shared by every transport.
//!
//! The transport owns the bytes; this module owns the protocol. A message
//! arrives, it is classified as a request or a notification, and requests are
//! handed to the [`RequestHandler`].

use std::future::Future;

use crate::{
    INVALID_REQUEST, JSONRPC_VERSION, JsonRpcError, JsonRpcId, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse, PARSE_ERROR,
};

/// Handles one JSON-RPC request and returns the response to write.
pub trait RequestHandler {
    /// Handles a request. The returned response must echo the request id.
    fn handle(&mut self, request: JsonRpcRequest) -> impl Future<Output = JsonRpcResponse> + Send;
}

/// The outcome of dispatching one JSON-RPC message.
#[derive(Debug, Clone, PartialEq)]
pub enum Dispatched {
    /// The message was a request: this is the response to send.
    Response(JsonRpcResponse),
    /// The message was a notification: there is nothing to answer.
    Notification,
    /// The message is not a valid JSON-RPC message: this is the error to report.
    Malformed(JsonRpcResponse),
}

/// Dispatches one JSON-RPC message and returns what to answer.
pub async fn dispatch<H: RequestHandler>(handler: &mut H, message: &str) -> Dispatched {
    let value: serde_json::Value = match serde_json::from_str(message) {
        Ok(value) => value,
        Err(_) => {
            return Dispatched::Malformed(error_response(PARSE_ERROR, "invalid JSON"));
        }
    };

    if !value.is_object() {
        return Dispatched::Malformed(error_response(INVALID_REQUEST, "invalid request"));
    }

    // The presence of an id tells requests and notifications apart.
    if value.get("id").is_some() {
        let request: JsonRpcRequest = match serde_json::from_value(value) {
            Ok(request) => request,
            Err(_) => {
                return Dispatched::Malformed(error_response(INVALID_REQUEST, "invalid request"));
            }
        };

        return Dispatched::Response(handler.handle(request).await);
    }

    // Notifications are never answered; malformed ones are dropped with a warning.
    if let Err(err) = serde_json::from_value::<JsonRpcNotification>(value) {
        tracing::warn!(%err, "dropping invalid notification");
    }

    Dispatched::Notification
}

/// Builds a JSON-RPC error frame.
pub fn error_response(code: i64, message: &str) -> JsonRpcResponse {
    JsonRpcResponse::Error {
        jsonrpc: JSONRPC_VERSION.to_string(),
        id: JsonRpcId::Null,
        error: JsonRpcError {
            code,
            message: message.to_string(),
            data: None,
        },
    }
}
