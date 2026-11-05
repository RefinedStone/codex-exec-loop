#!/usr/bin/env python3
"""Check that one app-server endpoint serves several connections.

Usage:
    python3 -I multi_client_check.py --codex CODEX_PATH [--clients N] [--socket-dir DIR]
    python3 -I multi_client_check.py --socket SOCKET_PATH [--clients N]

With --codex, the script starts `codex app-server --listen unix://PATH` on a
temporary socket and stops it at the end. With --socket, the script connects
to an endpoint that is already running, such as the Akra Gateway.

Each connection performs its own initialize and sends thread/list with the
same request id. With --socket, the script also asks akra/gateway/info. These
requests do not call a model. The output contains no conversation content.
The script uses only the Python standard library.
"""

import argparse
import base64
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

MAX_UNIX_SOCKET_PATH_BYTES = 103
START_TIMEOUT_SECONDS = 15
MAX_ERROR_MESSAGE_CHARS = 120


class WebSocketClient:
    """Minimal WebSocket client over a unix socket (text frames only)."""

    def __init__(self, path, timeout=10):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(timeout)
        self.sock.connect(path)
        self.buffer = b""
        key = base64.b64encode(os.urandom(16)).decode()
        request = (
            "GET / HTTP/1.1\r\n"
            "Host: localhost\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(request.encode())
        while b"\r\n\r\n" not in self.buffer:
            self._fill()
        head, self.buffer = self.buffer.split(b"\r\n\r\n", 1)
        self.status = head.split(b"\r\n", 1)[0].decode()
        if " 101 " not in f"{self.status} ":
            raise RuntimeError(f"WebSocket handshake failed: {self.status}")

    def _fill(self):
        chunk = self.sock.recv(65536)
        if not chunk:
            raise ConnectionError("socket closed")
        self.buffer += chunk

    def _take(self, size):
        while len(self.buffer) < size:
            self._fill()
        data, self.buffer = self.buffer[:size], self.buffer[size:]
        return data

    def _send_frame(self, opcode, payload):
        header = bytearray([0x80 | opcode])
        size = len(payload)
        if size < 126:
            header.append(0x80 | size)
        elif size < 65536:
            header.append(0x80 | 126)
            header += size.to_bytes(2, "big")
        else:
            header.append(0x80 | 127)
            header += size.to_bytes(8, "big")
        mask = os.urandom(4)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self.sock.sendall(bytes(header) + mask + masked)

    def send_json(self, message):
        self._send_frame(0x1, json.dumps(message).encode())

    def receive_json(self):
        message = b""
        while True:
            first, second = self._take(2)
            final = first & 0x80
            opcode = first & 0x0F
            size = second & 0x7F
            if size == 126:
                size = int.from_bytes(self._take(2), "big")
            elif size == 127:
                size = int.from_bytes(self._take(8), "big")
            mask = self._take(4) if second & 0x80 else None
            payload = self._take(size)
            if mask:
                payload = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
            if opcode == 0x9:
                self._send_frame(0xA, payload)
                continue
            if opcode == 0xA:
                continue
            if opcode == 0x8:
                raise ConnectionError("server closed the connection")
            message += payload
            if final:
                return json.loads(message)

    def call(self, request_id, method, params=None):
        request = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            request["params"] = params
        self.send_json(request)
        while True:
            message = self.receive_json()
            if message.get("id") == request_id and "method" not in message:
                return message

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


def describe(response):
    if "error" in response:
        message = str(response["error"].get("message", ""))
        if len(message) > MAX_ERROR_MESSAGE_CHARS:
            message = message[:MAX_ERROR_MESSAGE_CHARS] + "..."
        return f"error {response['error'].get('code')} {message}"
    return "ok"


def start_app_server(codex, socket_dir):
    directory = tempfile.mkdtemp(prefix="aks", dir=socket_dir)
    path = os.path.join(directory, "s.sock")
    if len(path.encode()) > MAX_UNIX_SOCKET_PATH_BYTES:
        shutil.rmtree(directory, ignore_errors=True)
        raise RuntimeError(f"socket path is too long ({len(path.encode())} bytes): {path}")
    process = subprocess.Popen(
        [codex, "app-server", "--listen", f"unix://{path}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    deadline = time.time() + START_TIMEOUT_SECONDS
    while time.time() < deadline:
        if os.path.exists(path):
            return process, directory, path
        if process.poll() is not None:
            break
        time.sleep(0.2)
    process.terminate()
    shutil.rmtree(directory, ignore_errors=True)
    raise RuntimeError("app-server did not create the socket")


def run_checks(path, client_count, ask_gateway_info):
    clients = []
    success = True
    try:
        for index in range(client_count):
            client = WebSocketClient(path)
            clients.append(client)
            print(f"client{index} handshake: {client.status}")
            if ask_gateway_info:
                response = client.call(0, "akra/gateway/info")
                result = response.get("result") or {}
                print(f"client{index} akra/gateway/info: {describe(response)} | keys: {sorted(result.keys())}")
            response = client.call(1, "initialize", {
                "clientInfo": {"name": f"akra-multi-client-check-{index}", "version": "0"},
                "capabilities": {"experimentalApi": False},
            })
            print(f"client{index} initialize: {describe(response)}")
            success = success and "result" in response
            client.send_json({"jsonrpc": "2.0", "method": "initialized", "params": {}})
        for index, client in enumerate(clients):
            response = client.call(2, "thread/list", {"limit": 2})
            threads = (response.get("result") or {}).get("data") or []
            print(f"client{index} thread/list (id=2): {describe(response)} | count: {len(threads)}")
            success = success and "result" in response
    finally:
        for client in clients:
            client.close()
    return success


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument("--codex", help="codex executable; start a temporary app-server")
    target.add_argument("--socket", help="existing unix socket endpoint, such as the Akra Gateway")
    parser.add_argument("--clients", type=int, default=2, help="number of connections (default 2)")
    parser.add_argument("--socket-dir", default="/tmp", help="directory for the temporary socket (default /tmp)")
    arguments = parser.parse_args()

    if arguments.socket:
        success = run_checks(os.path.expanduser(arguments.socket), arguments.clients, ask_gateway_info=True)
        return 0 if success else 1

    process, directory, path = start_app_server(arguments.codex, arguments.socket_dir)
    try:
        success = run_checks(path, arguments.clients, ask_gateway_info=False)
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
        shutil.rmtree(directory, ignore_errors=True)
    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(main())
