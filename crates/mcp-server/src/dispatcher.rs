//! Minimal MCP method routing for the stdio server.

use serde::Serialize;
use serde_json::json;

use crate::{
    INTERNAL_ERROR, INVALID_PARAMS, Implementation, InitializeParams, InitializeResult,
    JSONRPC_VERSION, JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse, METHOD_NOT_FOUND,
    RequestHandler, ToolsCallParams, ToolsListResult,
};

/// Routes MCP lifecycle methods to their responses.
#[derive(Debug, Default)]
pub struct Dispatcher;

impl RequestHandler for Dispatcher {
    async fn handle(&mut self, request: JsonRpcRequest) -> JsonRpcResponse {
        let JsonRpcRequest {
            id, method, params, ..
        } = request;

        match method.as_str() {
            "initialize" => match params.map(serde_json::from_value::<InitializeParams>) {
                Some(Ok(params)) => success(
                    id,
                    InitializeResult {
                        protocol_version: params.protocol_version,
                        capabilities: json!({"tools": {}}),
                        server_info: Implementation {
                            name: env!("CARGO_PKG_NAME").to_string(),
                            version: env!("CARGO_PKG_VERSION").to_string(),
                        },
                    },
                ),
                _ => error(id, INVALID_PARAMS, "invalid initialize params"),
            },
            "tools/list" => success(id, ToolsListResult { tools: Vec::new() }),
            "tools/call" => match params.map(serde_json::from_value::<ToolsCallParams>) {
                Some(Ok(params)) => {
                    error(id, INVALID_PARAMS, format!("unknown tool: {}", params.name))
                }
                _ => error(id, INVALID_PARAMS, "invalid tools/call params"),
            },
            _ => error(id, METHOD_NOT_FOUND, format!("method not found: {method}")),
        }
    }
}

fn success<T: Serialize>(id: JsonRpcId, result: T) -> JsonRpcResponse {
    match serde_json::to_value(result) {
        Ok(result) => JsonRpcResponse::Success {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result,
        },
        Err(_) => error(id, INTERNAL_ERROR, "failed to serialize result"),
    }
}

fn error(id: JsonRpcId, code: i64, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse::Error {
        jsonrpc: JSONRPC_VERSION.to_string(),
        id,
        error: JsonRpcError {
            code,
            message: message.into(),
            data: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Dispatcher;
    use crate::{
        INVALID_PARAMS, JSONRPC_VERSION, JsonRpcId, JsonRpcRequest, JsonRpcResponse,
        METHOD_NOT_FOUND, RequestHandler,
    };

    fn request(id: i64, method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Number(id),
            method: method.to_string(),
            params,
        }
    }

    fn assert_error(
        response: JsonRpcResponse,
        expected_id: i64,
        expected_code: i64,
        expected_message: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let JsonRpcResponse::Error { id, error, .. } = response else {
            return Err("expected an error response".into());
        };
        assert_eq!(id, JsonRpcId::Number(expected_id));
        assert_eq!(error.code, expected_code);
        assert_eq!(error.message, expected_message);
        assert!(error.data.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn initialize_returns_server_info() -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = Dispatcher;
        let response = dispatcher
            .handle(request(
                1,
                "initialize",
                Some(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1.0"}
                })),
            ))
            .await;

        let JsonRpcResponse::Success { id, result, .. } = response else {
            return Err("expected a success response".into());
        };
        assert_eq!(id, JsonRpcId::Number(1));
        assert_eq!(result["protocolVersion"], json!("2024-11-05"));
        assert_eq!(result["capabilities"], json!({"tools": {}}));
        assert_eq!(result["serverInfo"]["name"], json!("mcp-server"));
        assert_eq!(result["serverInfo"]["version"], json!("0.1.0"));
        Ok(())
    }

    #[tokio::test]
    async fn initialize_with_invalid_params_is_invalid_params()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = Dispatcher;
        let response = dispatcher
            .handle(request(2, "initialize", Some(json!({"capabilities": {}}))))
            .await;

        assert_error(response, 2, INVALID_PARAMS, "invalid initialize params")
    }

    #[tokio::test]
    async fn tools_list_returns_empty_tool_list() -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = Dispatcher;
        let response = dispatcher.handle(request(3, "tools/list", None)).await;

        let JsonRpcResponse::Success { id, result, .. } = response else {
            return Err("expected a success response".into());
        };
        assert_eq!(id, JsonRpcId::Number(3));
        assert_eq!(result, json!({"tools": []}));
        Ok(())
    }

    #[tokio::test]
    async fn tools_call_with_unknown_tool_is_invalid_params()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = Dispatcher;
        let response = dispatcher
            .handle(request(4, "tools/call", Some(json!({"name": "run_wasm"}))))
            .await;

        assert_error(response, 4, INVALID_PARAMS, "unknown tool: run_wasm")
    }

    #[tokio::test]
    async fn unknown_method_is_method_not_found() -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = Dispatcher;
        let response = dispatcher.handle(request(5, "nope", None)).await;

        assert_error(response, 5, METHOD_NOT_FOUND, "method not found: nope")
    }
}
