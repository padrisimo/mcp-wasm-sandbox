//! Newline-delimited JSON-RPC 2.0 over async byte streams.

use std::future::Future;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::error::McpWireError;
use crate::{
    INVALID_REQUEST, JSONRPC_VERSION, JsonRpcError, JsonRpcId, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse, PARSE_ERROR,
};

/// Handles one JSON-RPC request and returns the response to write.
pub trait RequestHandler {
    /// Handles a request. The returned response must echo the request id.
    fn handle(&mut self, request: JsonRpcRequest) -> impl Future<Output = JsonRpcResponse>;
}

/// Serves JSON-RPC over `reader` and `writer` until EOF.
pub async fn serve<R, W, H>(reader: R, mut writer: W, mut handler: H) -> Result<(), McpWireError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    H: RequestHandler,
{
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                write_response(&mut writer, error_response(PARSE_ERROR, "invalid JSON")).await?;
                continue;
            }
        };

        if !value.is_object() {
            write_response(
                &mut writer,
                error_response(INVALID_REQUEST, "invalid request"),
            )
            .await?;
            continue;
        }

        if value.get("id").is_some() {
            let request: JsonRpcRequest = match serde_json::from_value(value) {
                Ok(request) => request,
                Err(_) => {
                    write_response(
                        &mut writer,
                        error_response(INVALID_REQUEST, "invalid request"),
                    )
                    .await?;
                    continue;
                }
            };
            let response = handler.handle(request).await;
            write_response(&mut writer, response).await?;
        } else {
            // Notifications are never answered; invalid ones are dropped silently.
            let _ = serde_json::from_value::<JsonRpcNotification>(value);
        }
    }

    Ok(())
}

/// Serves JSON-RPC over process stdin and stdout until EOF.
pub async fn serve_stdio<H: RequestHandler>(handler: H) -> Result<(), McpWireError> {
    serve(tokio::io::stdin(), tokio::io::stdout(), handler).await
}

fn error_response(code: i64, message: &str) -> JsonRpcResponse {
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

async fn write_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    response: JsonRpcResponse,
) -> Result<(), McpWireError> {
    let mut line = serde_json::to_vec(&response)?;
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    struct CountingHandler {
        calls: Arc<AtomicUsize>,
    }

    impl RequestHandler for CountingHandler {
        async fn handle(&mut self, request: JsonRpcRequest) -> JsonRpcResponse {
            self.calls.fetch_add(1, Ordering::SeqCst);
            JsonRpcResponse::Success {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id: request.id,
                result: json!({"ok": true}),
            }
        }
    }

    fn counting_handler() -> (CountingHandler, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            CountingHandler {
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }

    fn response_values(
        output: &[u8],
    ) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
        let text = std::str::from_utf8(output)?;
        let mut values = Vec::new();
        for line in text.lines() {
            values.push(serde_json::from_str(line)?);
        }
        Ok(values)
    }

    #[tokio::test]
    async fn valid_request_writes_one_response_line() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input = concat!(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#, "\n").as_bytes();
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 1);
        assert_eq!(output.last(), Some(&b'\n'));

        let values = response_values(&output)?;
        assert_eq!(
            values,
            vec![json!({"jsonrpc": "2.0", "id": 7, "result": {"ok": true}})]
        );
        Ok(())
    }

    #[tokio::test]
    async fn notification_writes_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input = concat!(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n"
        )
        .as_bytes();
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(output.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn invalid_json_gets_parse_error() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let mut output = Vec::new();

        serve("{oops}\n".as_bytes(), &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let values = response_values(&output)?;
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["jsonrpc"], json!("2.0"));
        assert_eq!(values[0]["id"], json!(null));
        assert_eq!(values[0]["error"]["code"], json!(PARSE_ERROR));
        assert_eq!(values[0]["error"]["message"], json!("invalid JSON"));
        assert!(values[0]["error"].get("data").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn invalid_request_gets_invalid_request() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input = concat!("42\n", r#"{"jsonrpc":"2.0","id":1}"#, "\n").as_bytes();
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let values = response_values(&output)?;
        assert_eq!(values.len(), 2);
        for value in &values {
            assert_eq!(value["jsonrpc"], json!("2.0"));
            assert_eq!(value["id"], json!(null));
            assert_eq!(value["error"]["code"], json!(INVALID_REQUEST));
            assert_eq!(value["error"]["message"], json!("invalid request"));
            assert!(value["error"].get("data").is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn blank_lines_ignored() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let mut output = Vec::new();

        serve("\n\n".as_bytes(), &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(output.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn clean_eof_is_ok() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let input: &[u8] = b"";
        let mut output = Vec::new();

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(output.is_empty());
        Ok(())
    }
}
