//! Newline-delimited JSON-RPC 2.0 over async byte streams.

use std::future::Future;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::error::McpWireError;
use crate::{
    INVALID_REQUEST, JSONRPC_VERSION, JsonRpcError, JsonRpcId, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse, PARSE_ERROR,
};

/// Maximum accepted length of a single NDJSON line, in bytes.
const MAX_LINE_BYTES: usize = 1024 * 1024;

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
    let mut reader = BufReader::new(reader);

    loop {
        let line = match read_line(&mut reader).await? {
            IncomingLine::Eof => break,
            IncomingLine::TooLong => {
                write_response(
                    &mut writer,
                    error_response(INVALID_REQUEST, "request too large"),
                )
                .await?;
                continue;
            }
            IncomingLine::Line(bytes) => bytes,
        };

        let line = match std::str::from_utf8(&line) {
            Ok(text) => text.trim(),
            Err(_) => {
                write_response(&mut writer, error_response(PARSE_ERROR, "invalid JSON")).await?;
                continue;
            }
        };

        if line.is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(line) {
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
        } else if let Err(err) = serde_json::from_value::<JsonRpcNotification>(value) {
            // Notifications are never answered; malformed ones are dropped with a warning.
            tracing::warn!(%err, "dropping invalid notification");
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

/// One line read from the input stream.
enum IncomingLine {
    /// The stream ended with no pending data.
    Eof,
    /// A complete line, including its trailing newline.
    Line(Vec<u8>),
    /// A line above [`MAX_LINE_BYTES`], already discarded.
    TooLong,
}

/// Reads one newline-terminated line, discarding anything above the size cap.
async fn read_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<IncomingLine, std::io::Error> {
    let mut line: Vec<u8> = Vec::new();
    let mut too_long = false;

    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if too_long {
                IncomingLine::TooLong
            } else if line.is_empty() {
                IncomingLine::Eof
            } else {
                IncomingLine::Line(line)
            });
        }

        let newline = available.iter().position(|&byte| byte == b'\n');
        let take = newline.map_or(available.len(), |position| position + 1);

        if !too_long {
            if line.len() + take > MAX_LINE_BYTES {
                too_long = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..take]);
            }
        }

        let complete = newline.is_some();
        reader.consume(take);

        if complete {
            return Ok(if too_long {
                IncomingLine::TooLong
            } else {
                IncomingLine::Line(line)
            });
        }
    }
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

    #[tokio::test]
    async fn invalid_utf8_gets_parse_error() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let mut output = Vec::new();
        let input: &[u8] = b"\xff\n";

        serve(input, &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let values = response_values(&output)?;
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["jsonrpc"], json!("2.0"));
        assert_eq!(values[0]["id"], json!(null));
        assert_eq!(values[0]["error"]["code"], json!(PARSE_ERROR));
        assert_eq!(values[0]["error"]["message"], json!("invalid JSON"));
        Ok(())
    }

    #[tokio::test]
    async fn oversized_line_gets_invalid_request() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let mut output = Vec::new();
        let mut input = vec![b'a'; MAX_LINE_BYTES + 1];
        input.push(b'\n');
        input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}\n");

        serve(input.as_slice(), &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let values = response_values(&output)?;
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["id"], json!(null));
        assert_eq!(values[0]["error"]["code"], json!(INVALID_REQUEST));
        assert_eq!(values[0]["error"]["message"], json!("request too large"));
        assert_eq!(values[1]["id"], json!(9));
        Ok(())
    }

    #[tokio::test]
    async fn invalid_notification_is_dropped() -> Result<(), Box<dyn std::error::Error>> {
        let (handler, calls) = counting_handler();
        let mut output = Vec::new();
        let input = b"{\"jsonrpc\":\"2.0\",\"method\":42}\n";

        serve(input.as_slice(), &mut output, handler).await?;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(output.is_empty());
        Ok(())
    }
}
