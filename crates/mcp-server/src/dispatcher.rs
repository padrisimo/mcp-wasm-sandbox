//! Minimal MCP method routing for the stdio server.

use base64::Engine;
use sandbox_core::Sandbox;
use serde::Deserialize;
use serde_json::json;

use crate::{
    Content, INTERNAL_ERROR, INVALID_PARAMS, Implementation, InitializeParams, InitializeResult,
    JSONRPC_VERSION, JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse, METHOD_NOT_FOUND,
    RequestHandler, Tool, ToolsCallParams, ToolsCallResult, ToolsListResult,
};

/// Name of the tool exposed by this server.
const RUN_WASM_TOOL: &str = "run_wasm";

/// Message returned when the `run_wasm` arguments do not match the schema.
const INVALID_RUN_WASM_ARGS: &str = "invalid run_wasm arguments";

/// Routes MCP lifecycle methods to their responses.
pub struct Dispatcher {
    sandbox: Sandbox,
}

impl Dispatcher {
    /// Builds a dispatcher that runs guest code inside `sandbox`.
    pub fn new(sandbox: Sandbox) -> Self {
        Self { sandbox }
    }
}

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
            "tools/list" => success(id, ToolsListResult { tools: tools() }),
            "tools/call" => match params.map(serde_json::from_value::<ToolsCallParams>) {
                Some(Ok(params)) => match self.run_tool(&params).await {
                    Ok(result) => success(id, result),
                    Err(message) => error(id, INVALID_PARAMS, message),
                },
                _ => error(id, INVALID_PARAMS, "invalid tools/call params"),
            },
            _ => error(id, METHOD_NOT_FOUND, format!("method not found: {method}")),
        }
    }
}

impl Dispatcher {
    /// Runs a tool and reports its outcome as an MCP tool result.
    async fn run_tool(&self, params: &ToolsCallParams) -> Result<ToolsCallResult, String> {
        if params.name != RUN_WASM_TOOL {
            return Err(format!("unknown tool: {}", params.name));
        }

        let arguments = params
            .arguments
            .as_ref()
            .ok_or_else(|| INVALID_RUN_WASM_ARGS.to_string())?;
        let parsed: RunWasmArgs = serde_json::from_value(arguments.clone())
            .map_err(|_| INVALID_RUN_WASM_ARGS.to_string())?;
        let module = parsed.module()?;

        match self
            .sandbox
            .call_i32(&module, &parsed.export, &parsed.args)
            .await
        {
            Ok(values) => Ok(ToolsCallResult {
                content: vec![Content::Text {
                    text: format_values(&values),
                }],
                is_error: None,
            }),
            Err(err) => Ok(ToolsCallResult {
                content: vec![Content::Text {
                    text: err.to_string(),
                }],
                is_error: Some(true),
            }),
        }
    }
}

/// Arguments accepted by the `run_wasm` tool.
#[derive(Debug, Deserialize)]
struct RunWasmArgs {
    /// Module source in the WebAssembly text format.
    source: Option<String>,
    /// Module binary, base64 encoded.
    module_base64: Option<String>,
    /// Name of the exported function to call.
    export: String,
    /// Integer arguments for the exported function.
    #[serde(default)]
    args: Vec<i32>,
}

impl RunWasmArgs {
    /// The module bytes, from the text source or from the base64 binary.
    fn module(&self) -> Result<Vec<u8>, String> {
        if let Some(encoded) = &self.module_base64 {
            return base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|err| format!("invalid base64 module: {err}"));
        }

        match &self.source {
            Some(source) => Ok(source.as_bytes().to_vec()),
            None => Err(INVALID_RUN_WASM_ARGS.to_string()),
        }
    }
}

/// Tools advertised by this server.
fn tools() -> Vec<Tool> {
    vec![Tool {
        name: RUN_WASM_TOOL.to_string(),
        description: Some(
            "Compiles and runs a WebAssembly module inside the sandbox and returns its output."
                .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "source": {
                    "type": "string",
                    "description": "Module source in the WebAssembly text format (WAT)."
                },
                "module_base64": {
                    "type": "string",
                    "description": "WebAssembly binary, base64 encoded."
                },
                "export": {
                    "type": "string",
                    "description": "Name of the exported function to call."
                },
                "args": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "description": "Integer arguments for the exported function."
                }
            },
            "required": ["export"],
            "anyOf": [{"required": ["source"]}, {"required": ["module_base64"]}]
        }),
    }]
}

fn format_values(values: &[i32]) -> String {
    values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn success<T: serde::Serialize>(id: JsonRpcId, result: T) -> JsonRpcResponse {
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
    use sandbox_core::{Sandbox, SandboxConfig};
    use serde_json::json;

    use super::Dispatcher;
    use crate::{
        INVALID_PARAMS, JSONRPC_VERSION, JsonRpcId, JsonRpcRequest, JsonRpcResponse,
        METHOD_NOT_FOUND, RequestHandler,
    };

    const ADD_MODULE: &str = r#"(module
        (func (export "add") (param i32 i32) (result i32)
            local.get 0
            local.get 1
            i32.add))"#;

    const SPIN_MODULE: &str = r#"(module (func (export "spin") (loop br 0)))"#;

    fn request(id: i64, method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: JsonRpcId::Number(id),
            method: method.to_string(),
            params,
        }
    }

    fn dispatcher() -> Result<Dispatcher, Box<dyn std::error::Error>> {
        Ok(Dispatcher::new(Sandbox::new(SandboxConfig::default())?))
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
        let mut dispatcher = dispatcher()?;
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
        let mut dispatcher = dispatcher()?;
        let response = dispatcher
            .handle(request(2, "initialize", Some(json!({"capabilities": {}}))))
            .await;

        assert_error(response, 2, INVALID_PARAMS, "invalid initialize params")
    }

    #[tokio::test]
    async fn tools_list_advertises_the_run_wasm_tool() -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = dispatcher()?;
        let response = dispatcher.handle(request(3, "tools/list", None)).await;

        let JsonRpcResponse::Success { id, result, .. } = response else {
            return Err("expected a success response".into());
        };
        assert_eq!(id, JsonRpcId::Number(3));
        assert_eq!(result["tools"][0]["name"], json!("run_wasm"));
        assert_eq!(result["tools"][0]["inputSchema"]["type"], json!("object"));
        Ok(())
    }

    #[tokio::test]
    async fn tools_call_with_unknown_tool_is_invalid_params()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = dispatcher()?;
        let response = dispatcher
            .handle(request(
                4,
                "tools/call",
                Some(json!({"name": "unknown_tool"})),
            ))
            .await;

        assert_error(response, 4, INVALID_PARAMS, "unknown tool: unknown_tool")
    }

    #[tokio::test]
    async fn run_wasm_executes_the_module() -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = dispatcher()?;
        let response = dispatcher
            .handle(request(
                6,
                "tools/call",
                Some(json!({
                    "name": "run_wasm",
                    "arguments": {"source": ADD_MODULE, "export": "add", "args": [2, 3]}
                })),
            ))
            .await;

        let JsonRpcResponse::Success { id, result, .. } = response else {
            return Err("expected a success response".into());
        };
        assert_eq!(id, JsonRpcId::Number(6));
        assert_eq!(result["content"][0]["type"], json!("text"));
        assert_eq!(result["content"][0]["text"], json!("5"));
        Ok(())
    }

    #[tokio::test]
    async fn run_wasm_reports_execution_failures() -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = dispatcher()?;
        let response = dispatcher
            .handle(request(
                7,
                "tools/call",
                Some(json!({
                    "name": "run_wasm",
                    "arguments": {"source": SPIN_MODULE, "export": "spin"}
                })),
            ))
            .await;

        let JsonRpcResponse::Success { result, .. } = response else {
            return Err("expected a success response".into());
        };
        assert_eq!(result["isError"], json!(true));
        assert!(
            result["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("fuel")),
            "unexpected content: {result:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_wasm_with_invalid_arguments_is_invalid_params()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = dispatcher()?;
        let response = dispatcher
            .handle(request(
                8,
                "tools/call",
                Some(json!({"name": "run_wasm", "arguments": {}})),
            ))
            .await;

        assert_error(response, 8, INVALID_PARAMS, "invalid run_wasm arguments")
    }

    #[tokio::test]
    async fn run_wasm_accepts_a_base64_binary_module() -> Result<(), Box<dyn std::error::Error>> {
        use base64::Engine;

        let wasm = wat::parse_str(ADD_MODULE).map_err(std::io::Error::other)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&wasm);

        let mut dispatcher = dispatcher()?;
        let response = dispatcher
            .handle(request(
                9,
                "tools/call",
                Some(json!({
                    "name": "run_wasm",
                    "arguments": {"module_base64": encoded, "export": "add", "args": [2, 3]}
                })),
            ))
            .await;

        let JsonRpcResponse::Success { result, .. } = response else {
            return Err("expected a success response".into());
        };
        assert_eq!(result["content"][0]["text"], json!("5"));
        Ok(())
    }

    #[tokio::test]
    async fn unknown_method_is_method_not_found() -> Result<(), Box<dyn std::error::Error>> {
        let mut dispatcher = dispatcher()?;
        let response = dispatcher.handle(request(5, "nope", None)).await;

        assert_error(response, 5, METHOD_NOT_FOUND, "method not found: nope")
    }
}
