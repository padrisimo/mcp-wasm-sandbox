"""End-to-end tests for the HTTP MCP transport.

They start the server in HTTP mode and talk to it like a remote MCP client.
"""

import json
import socket
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
CANDIDATES = (ROOT / "target" / "release" / "mcp-server", ROOT / "target" / "debug" / "mcp-server")


def server_binary() -> Path:
    for candidate in CANDIDATES:
        if candidate.exists():
            return candidate
    pytest.skip("build the server first: cargo build --release -p mcp-server")


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_for(port: int, process: subprocess.Popen) -> None:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("the server exited")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return
        except OSError:
            time.sleep(0.1)
    raise RuntimeError("the server did not start")


def post(url: str, body: str):
    request = urllib.request.Request(
        url,
        data=body.encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            payload = response.read()
            return response.status, json.loads(payload) if payload else None
    except urllib.error.HTTPError as error:
        payload = error.read()
        return error.code, json.loads(payload) if payload else None


@pytest.fixture
def url():
    port = free_port()
    process = subprocess.Popen(
        [str(server_binary()), "--http", f"127.0.0.1:{port}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    endpoint = f"http://127.0.0.1:{port}/mcp"
    try:
        wait_for(port, process)
        # The lifecycle requires the handshake before any other request.
        status, _ = post(
            endpoint,
            '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05",'
            '"capabilities":{},"clientInfo":{"name":"pytest","version":"1.0"}}}',
        )
        assert status == 200
        yield endpoint
    finally:
        process.terminate()
        process.wait(timeout=10)


def test_tools_list_over_http(url):
    status, payload = post(url, '{"jsonrpc":"2.0","id":1,"method":"tools/list"}')

    assert status == 200
    assert payload["id"] == 1
    assert payload["result"]["tools"][0]["name"] == "run_wasm"


def test_a_notification_is_accepted_without_a_body(url):
    status, payload = post(url, '{"jsonrpc":"2.0","method":"notifications/initialized"}')

    assert status == 202
    assert payload is None


def test_invalid_json_is_rejected(url):
    status, payload = post(url, "{oops")

    assert status == 400
    assert payload["error"]["code"] == -32700
