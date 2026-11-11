//! JSON-RPC 2.0 envelope, MCP wire payloads, and newline-delimited stdio transport.
//!
//! The JSON-RPC constants and envelope types, the `initialize`, `tools/list`,
//! and `tools/call` payloads, and the bridge from [`sandbox_core::SandboxError`]
//! to [`JsonRpcError`] come from Phase 1. Phase 1b adds [`McpWireError`] and
//! the newline-delimited transport ([`serve`] and [`serve_stdio`]); request
//! dispatch and MCP method routing arrive later.

mod error;
mod transport;

pub use error::McpWireError;
pub use transport::{RequestHandler, serve, serve_stdio};

use sandbox_core::SandboxError;
use serde::{Deserialize, Serialize};

/// JSON-RPC protocol version carried by every message.
pub const JSONRPC_VERSION: &str = "2.0";

/// Error code for a frame that is not valid JSON.
pub const PARSE_ERROR: i64 = -32700;

/// Error code for a JSON value that is not a valid request object.
pub const INVALID_REQUEST: i64 = -32600;

/// Error code for an unknown method.
pub const METHOD_NOT_FOUND: i64 = -32601;

/// Error code for params that do not match the method.
pub const INVALID_PARAMS: i64 = -32602;

/// Error code for an internal server fault.
pub const INTERNAL_ERROR: i64 = -32603;

/// Identifier of a JSON-RPC request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    /// Integer identifier.
    Number(i64),
    /// String identifier.
    String(String),
    /// Explicit `null` identifier.
    Null,
}

/// JSON-RPC 2.0 request: a call that expects a response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// Protocol version, normally [`JSONRPC_VERSION`].
    pub jsonrpc: String,
    /// Request identifier echoed in the response.
    pub id: JsonRpcId,
    /// Method name.
    pub method: String,
    /// Method parameters, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 notification: a call that expects no response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    /// Protocol version, normally [`JSONRPC_VERSION`].
    pub jsonrpc: String,
    /// Method name.
    pub method: String,
    /// Method parameters, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code.
    pub code: i64,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured error data, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response: exactly one of a result or an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponse {
    /// Successful response.
    Success {
        /// Protocol version, normally [`JSONRPC_VERSION`].
        jsonrpc: String,
        /// Identifier of the request being answered.
        id: JsonRpcId,
        /// Method result.
        result: serde_json::Value,
    },
    /// Failed response.
    Error {
        /// Protocol version, normally [`JSONRPC_VERSION`].
        jsonrpc: String,
        /// Identifier of the request being answered.
        id: JsonRpcId,
        /// Error object.
        error: JsonRpcError,
    },
}

/// A client or server implementation name and version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Implementation {
    /// Implementation name.
    pub name: String,
    /// Implementation version.
    pub version: String,
}

/// Params of the MCP `initialize` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    /// Protocol version requested by the client.
    pub protocol_version: String,
    /// Client capabilities, opaque in Phase 1.
    pub capabilities: serde_json::Value,
    /// Client name and version.
    pub client_info: Implementation,
}

/// Result of the MCP `initialize` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// Protocol version chosen by the server.
    pub protocol_version: String,
    /// Server capabilities, opaque in Phase 1.
    pub capabilities: serde_json::Value,
    /// Server name and version.
    pub server_info: Implementation,
}

/// Description of a tool exposed by the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    /// Tool name.
    pub name: String,
    /// Optional human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema of the tool arguments, opaque in Phase 1.
    pub input_schema: serde_json::Value,
}

/// Result of the MCP `tools/list` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsListResult {
    /// Available tools.
    pub tools: Vec<Tool>,
}

/// Params of the MCP `tools/call` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallParams {
    /// Name of the tool to call.
    pub name: String,
    /// Tool arguments, opaque in Phase 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
}

/// A content block returned by a tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
    /// Plain text content.
    Text {
        /// Text payload.
        text: String,
    },
}

/// Result of the MCP `tools/call` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallResult {
    /// Content blocks produced by the tool.
    pub content: Vec<Content>,
    /// Whether the tool reported a failure, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl From<SandboxError> for JsonRpcError {
    fn from(err: SandboxError) -> Self {
        Self {
            code: INTERNAL_ERROR,
            message: err.to_string(),
            data: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_core::SandboxError;
    use serde_json::json;

    #[test]
    fn constants() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(JSONRPC_VERSION, "2.0");
        assert_eq!(PARSE_ERROR, -32700);
        assert_eq!(INVALID_REQUEST, -32600);
        assert_eq!(METHOD_NOT_FOUND, -32601);
        assert_eq!(INVALID_PARAMS, -32602);
        assert_eq!(INTERNAL_ERROR, -32603);
        Ok(())
    }

    #[test]
    fn request_round_trip_number_id() -> Result<(), Box<dyn std::error::Error>> {
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Number(1),
            method: "tools/list".to_string(),
            params: None,
        };
        let value = serde_json::to_value(&request)?;
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
        );
        let decoded: JsonRpcRequest = serde_json::from_value(value)?;
        assert_eq!(decoded.method, "tools/list");
        assert_eq!(decoded.id, JsonRpcId::Number(1));
        assert_eq!(decoded, request);
        Ok(())
    }

    #[test]
    fn request_round_trip_string_id() -> Result<(), Box<dyn std::error::Error>> {
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::String("req-1".to_string()),
            method: "tools/call".to_string(),
            params: Some(json!({"name": "run_wasm", "arguments": {"module": "a.wasm"}})),
        };
        let value = serde_json::to_value(&request)?;
        assert_eq!(
            value,
            json!({
                "jsonrpc": "2.0",
                "id": "req-1",
                "method": "tools/call",
                "params": {"name": "run_wasm", "arguments": {"module": "a.wasm"}}
            })
        );
        let decoded: JsonRpcRequest = serde_json::from_value(value)?;
        assert_eq!(decoded.id, JsonRpcId::String("req-1".to_string()));
        assert_eq!(decoded, request);
        Ok(())
    }

    #[test]
    fn request_round_trip_null_id() -> Result<(), Box<dyn std::error::Error>> {
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Null,
            method: "ping".to_string(),
            params: None,
        };
        let value = serde_json::to_value(&request)?;
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "id": null, "method": "ping"})
        );
        let decoded: JsonRpcRequest = serde_json::from_value(value)?;
        assert_eq!(decoded.id, JsonRpcId::Null);
        Ok(())
    }

    #[test]
    fn request_params_omitted_when_none() -> Result<(), Box<dyn std::error::Error>> {
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Number(1),
            method: "tools/list".to_string(),
            params: None,
        };
        let value = serde_json::to_value(&request)?;
        assert!(value.get("params").is_none());

        let decoded: JsonRpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": null
        }))?;
        assert_eq!(decoded.params, None);
        let value = serde_json::to_value(&decoded)?;
        assert!(value.get("params").is_none());
        Ok(())
    }

    #[test]
    fn notification_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let notification = JsonRpcNotification {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: "notifications/initialized".to_string(),
            params: None,
        };
        let value = serde_json::to_value(&notification)?;
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        );
        assert!(value.get("id").is_none());
        let decoded: JsonRpcNotification = serde_json::from_value(value)?;
        assert_eq!(decoded.method, "notifications/initialized");
        assert_eq!(decoded, notification);

        let with_params = JsonRpcNotification {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: "notifications/progress".to_string(),
            params: Some(json!({"progress": 1})),
        };
        let value = serde_json::to_value(&with_params)?;
        assert_eq!(value["params"], json!({"progress": 1}));
        let decoded: JsonRpcNotification = serde_json::from_value(value)?;
        assert_eq!(decoded, with_params);
        Ok(())
    }

    #[test]
    fn response_success_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let response = JsonRpcResponse::Success {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Number(1),
            result: json!({"tools": []}),
        };
        let value = serde_json::to_value(&response)?;
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": []}})
        );
        let decoded: JsonRpcResponse = serde_json::from_value(value)?;
        assert_eq!(decoded, response);
        assert!(matches!(&decoded, JsonRpcResponse::Success { .. }));
        Ok(())
    }

    #[test]
    fn response_error_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let response = JsonRpcResponse::Error {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Number(1),
            error: JsonRpcError {
                code: INTERNAL_ERROR,
                message: "boom".to_string(),
                data: None,
            },
        };
        let value = serde_json::to_value(&response)?;
        assert_eq!(
            value,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {"code": -32603, "message": "boom"}
            })
        );
        let decoded: JsonRpcResponse = serde_json::from_value(value)?;
        assert_eq!(decoded, response);
        assert!(matches!(&decoded, JsonRpcResponse::Error { .. }));
        Ok(())
    }

    #[test]
    fn response_with_data() -> Result<(), Box<dyn std::error::Error>> {
        let response = JsonRpcResponse::Error {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::String("req-1".to_string()),
            error: JsonRpcError {
                code: INVALID_PARAMS,
                message: "bad params".to_string(),
                data: Some(json!({"detail": 1})),
            },
        };
        let value = serde_json::to_value(&response)?;
        assert_eq!(value["error"]["data"], json!({"detail": 1}));
        let decoded: JsonRpcResponse = serde_json::from_value(value)?;
        assert_eq!(decoded, response);
        Ok(())
    }

    #[test]
    fn fractional_id_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let result: Result<JsonRpcRequest, serde_json::Error> = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1.5,
            "method": "x"
        }));
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn initialize_params_camel_case() -> Result<(), Box<dyn std::error::Error>> {
        let params = InitializeParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: json!({}),
            client_info: Implementation {
                name: "test-client".to_string(),
                version: "1.0.0".to_string(),
            },
        };
        let value = serde_json::to_value(&params)?;
        assert_eq!(
            value,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "1.0.0"}
            })
        );
        assert!(value.get("protocolVersion").is_some());
        assert!(value.get("clientInfo").is_some());
        assert!(value.get("protocol_version").is_none());
        assert!(value.get("client_info").is_none());
        let decoded: InitializeParams = serde_json::from_value(value)?;
        assert_eq!(decoded, params);
        Ok(())
    }

    #[test]
    fn initialize_result_camel_case() -> Result<(), Box<dyn std::error::Error>> {
        let result = InitializeResult {
            protocol_version: "2024-11-05".to_string(),
            capabilities: json!({}),
            server_info: Implementation {
                name: "mcp-wasm-sandbox".to_string(),
                version: "0.1.0".to_string(),
            },
        };
        let value = serde_json::to_value(&result)?;
        assert_eq!(
            value,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "mcp-wasm-sandbox", "version": "0.1.0"}
            })
        );
        assert!(value.get("serverInfo").is_some());
        assert!(value.get("server_info").is_none());
        let decoded: InitializeResult = serde_json::from_value(value)?;
        assert_eq!(decoded, result);
        Ok(())
    }

    #[test]
    fn tool_camel_case_and_optional_description() -> Result<(), Box<dyn std::error::Error>> {
        let tool = Tool {
            name: "run_wasm".to_string(),
            description: None,
            input_schema: json!({}),
        };
        let value = serde_json::to_value(&tool)?;
        assert_eq!(value, json!({"name": "run_wasm", "inputSchema": {}}));
        assert!(value.get("inputSchema").is_some());
        assert!(value.get("input_schema").is_none());
        assert!(value.get("description").is_none());
        let decoded: Tool = serde_json::from_value(value)?;
        assert_eq!(decoded, tool);

        let tool = Tool {
            name: "run_wasm".to_string(),
            description: Some("Runs wasm".to_string()),
            input_schema: json!({"type": "object"}),
        };
        let value = serde_json::to_value(&tool)?;
        assert_eq!(
            value,
            json!({
                "name": "run_wasm",
                "description": "Runs wasm",
                "inputSchema": {"type": "object"}
            })
        );
        let decoded: Tool = serde_json::from_value(value)?;
        assert_eq!(decoded, tool);
        Ok(())
    }

    #[test]
    fn tools_list_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let list = ToolsListResult {
            tools: vec![Tool {
                name: "run_wasm".to_string(),
                description: Some("Runs wasm".to_string()),
                input_schema: json!({"type": "object"}),
            }],
        };
        let value = serde_json::to_value(&list)?;
        assert_eq!(
            value,
            json!({
                "tools": [{
                    "name": "run_wasm",
                    "description": "Runs wasm",
                    "inputSchema": {"type": "object"}
                }]
            })
        );
        let decoded: ToolsListResult = serde_json::from_value(value)?;
        assert_eq!(decoded, list);
        Ok(())
    }

    #[test]
    fn tools_call_params_optional_arguments() -> Result<(), Box<dyn std::error::Error>> {
        let params = ToolsCallParams {
            name: "run_wasm".to_string(),
            arguments: None,
        };
        let value = serde_json::to_value(&params)?;
        assert_eq!(value, json!({"name": "run_wasm"}));
        assert!(value.get("arguments").is_none());
        let decoded: ToolsCallParams = serde_json::from_value(value)?;
        assert_eq!(decoded, params);

        let params = ToolsCallParams {
            name: "run_wasm".to_string(),
            arguments: Some(json!({"module": "a.wasm"})),
        };
        let value = serde_json::to_value(&params)?;
        assert_eq!(
            value,
            json!({"name": "run_wasm", "arguments": {"module": "a.wasm"}})
        );
        let decoded: ToolsCallParams = serde_json::from_value(value)?;
        assert_eq!(decoded, params);
        Ok(())
    }

    #[test]
    fn content_text_shape() -> Result<(), Box<dyn std::error::Error>> {
        let content = Content::Text {
            text: "42".to_string(),
        };
        let value = serde_json::to_value(&content)?;
        assert_eq!(value, json!({"type": "text", "text": "42"}));
        let decoded: Content = serde_json::from_value(value)?;
        assert_eq!(decoded, content);
        Ok(())
    }

    #[test]
    fn tools_call_result_camel_case() -> Result<(), Box<dyn std::error::Error>> {
        let result = ToolsCallResult {
            content: vec![Content::Text {
                text: "42".to_string(),
            }],
            is_error: Some(false),
        };
        let value = serde_json::to_value(&result)?;
        assert_eq!(
            value,
            json!({
                "content": [{"type": "text", "text": "42"}],
                "isError": false
            })
        );
        assert!(value.get("isError").is_some());
        assert!(value.get("is_error").is_none());
        let decoded: ToolsCallResult = serde_json::from_value(value)?;
        assert_eq!(decoded, result);

        let result = ToolsCallResult {
            content: vec![Content::Text {
                text: "boom".to_string(),
            }],
            is_error: Some(true),
        };
        let value = serde_json::to_value(&result)?;
        assert_eq!(value["isError"], json!(true));
        let decoded: ToolsCallResult = serde_json::from_value(value)?;
        assert_eq!(decoded, result);

        let result = ToolsCallResult {
            content: vec![Content::Text {
                text: "42".to_string(),
            }],
            is_error: None,
        };
        let value = serde_json::to_value(&result)?;
        assert!(value.get("isError").is_none());
        let decoded: ToolsCallResult = serde_json::from_value(value)?;
        assert_eq!(decoded, result);
        Ok(())
    }

    #[test]
    fn sandbox_error_maps_to_internal_error() -> Result<(), Box<dyn std::error::Error>> {
        let error = JsonRpcError::from(SandboxError::FuelExhausted);
        assert_eq!(error.code, INTERNAL_ERROR);
        assert!(error.data.is_none());
        assert_eq!(error.message, "wasm execution exhausted its fuel budget");
        Ok(())
    }

    #[test]
    fn sandbox_error_message_preserved() -> Result<(), Box<dyn std::error::Error>> {
        let error = JsonRpcError::from(SandboxError::Execution("trap".to_string()));
        assert_eq!(error.code, INTERNAL_ERROR);
        assert!(error.data.is_none());
        assert_eq!(error.message, "wasm execution failed: trap");
        Ok(())
    }
}
