//! Minimal WASI support for sandboxed modules.
//!
//! The guest gets the standard WASI calls, but no host access at all: no
//! preopened directories, no environment variables, no arguments and no
//! inherited streams. Everything written to stdout or stderr is captured and
//! returned to the caller.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use wasmtime_wasi::async_trait;
use wasmtime_wasi::preview1::WasiP1Ctx;
use wasmtime_wasi::{HostOutputStream, StdoutStream, StreamResult, Subscribe, WasiCtxBuilder};

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

impl StdoutStream for CapturedOutput {
    fn stream(&self) -> Box<dyn HostOutputStream> {
        Box::new(CapturedStream(self.0.clone()))
    }

    fn isatty(&self) -> bool {
        false
    }
}

/// A stream that appends to the shared capture buffer.
struct CapturedStream(Arc<Mutex<Vec<u8>>>);

#[async_trait]
impl Subscribe for CapturedStream {
    async fn ready(&mut self) {}
}

impl HostOutputStream for CapturedStream {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        if let Ok(mut buffer) = self.0.lock() {
            buffer.extend_from_slice(bytes.as_ref());
        }
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(usize::MAX)
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
