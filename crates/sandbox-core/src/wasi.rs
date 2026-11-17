//! Minimal WASI support for sandboxed modules.
//!
//! The guest gets the standard WASI calls, but no host access at all: no
//! preopened directories, no environment variables, no arguments and no
//! inherited streams. Everything written to stdout or stderr is captured and
//! returned to the caller.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use tokio::io::AsyncWrite;
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::p1::WasiP1Ctx;

/// Collects everything the guest writes to stdout or stderr.
#[derive(Clone, Default)]
pub struct CapturedOutput(Arc<Mutex<Vec<u8>>>);

impl CapturedOutput {
    /// The bytes written so far.
    pub fn contents(&self) -> Vec<u8> {
        self.0
            .lock()
            .map(|buffer| buffer.clone())
            .unwrap_or_default()
    }
}

impl IsTerminal for CapturedOutput {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for CapturedOutput {
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(CapturedStream(self.0.clone()))
    }
}

/// A stream that appends to the shared capture buffer.
struct CapturedStream(Arc<Mutex<Vec<u8>>>);

impl AsyncWrite for CapturedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.0.lock() {
            Ok(mut buffer) => {
                buffer.extend_from_slice(buf);
                Poll::Ready(Ok(buf.len()))
            }
            Err(_) => Poll::Ready(Err(std::io::Error::other("captured output is poisoned"))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// The WASI context of a sandboxed module.
pub struct WasiState {
    ctx: WasiP1Ctx,
    output: CapturedOutput,
}

impl WasiState {
    /// Builds a context without any host access.
    pub fn new() -> Self {
        let output = CapturedOutput::default();
        let mut builder = WasiCtxBuilder::new();
        builder.stdout(output.clone());
        builder.stderr(output.clone());

        Self {
            ctx: builder.build_p1(),
            output,
        }
    }

    /// Everything the guest wrote to stdout or stderr.
    pub fn output(&self) -> &CapturedOutput {
        &self.output
    }

    /// The preview1 context handed to the linker.
    pub fn ctx(&mut self) -> &mut WasiP1Ctx {
        &mut self.ctx
    }
}

impl Default for WasiState {
    fn default() -> Self {
        Self::new()
    }
}
