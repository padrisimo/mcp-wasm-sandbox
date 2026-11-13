"""End-to-end tests for the stdio MCP server.

They drive the server exactly like an MCP client would: one JSON-RPC message
per line over stdin, one response line per request.
"""

import json
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
CANDIDATES = (ROOT / "target" / "release" / "mcp-server", ROOT / "target" / "debug" / "mcp-server")

ADD_MODULE = (
    '(module (func (export "add") (param i32 i32) (result i32) '
    "local.get 0 local.get 1 i32.add))"
)


def server_binary() -> Path:
    for candidate in CANDIDATES:
        if candidate.exists():
            return candidate
    pytest.skip("build the server first: cargo build --release -p mcp-server")


@pytest.fixture
def server():
    process = subprocess.Popen(
        [str(server_binary())],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        bufsize=1,
    )
    try:
        yield process
    finally:
        process.stdin.close()
        process.wait(timeout=10)


def call(server, request):
    server.stdin.write(json.dumps(request) + "\n")
    server.stdin.flush()
    line = server.stdout.readline()
    assert line, "the server closed the stream"
    return json.loads(line)


def test_initialize_reports_the_server_info(server):
    response = call(
        server,
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "pytest", "version": "1.0"},
            },
        },
    )

    assert response["id"] == 1
    assert response["result"]["protocolVersion"] == "2024-11-05"
    assert response["result"]["serverInfo"]["name"] == "mcp-server"


def test_tools_list_advertises_run_wasm(server):
    response = call(server, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})

    assert response["id"] == 2
    assert [tool["name"] for tool in response["result"]["tools"]] == ["run_wasm"]


def test_tools_call_runs_the_module(server):
    response = call(
        server,
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "run_wasm",
                "arguments": {"source": ADD_MODULE, "export": "add", "args": [2, 3]},
            },
        },
    )

    assert response["id"] == 3
    assert response["result"]["content"][0]["text"] == "5"


def test_unknown_method_is_method_not_found(server):
    response = call(server, {"jsonrpc": "2.0", "id": 4, "method": "nope"})

    assert response["id"] == 4
    assert response["error"]["code"] == -32601
