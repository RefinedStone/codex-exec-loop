#!/usr/bin/env python3
"""Check a Codex app-server handshake without starting a turn.

Usage:
    python3 -I appserver_handshake.py [CODEX_PATH]

The script starts `codex app-server` on stdio and sends initialize,
initialized, account/read, and thread/list. These requests do not call a
model. The output contains only response keys, the account type, and the
plan type. It never prints email addresses or conversation content.
"""

import json
import select
import subprocess
import sys
import time

TIMEOUT_SECONDS = 20


def read_response(process, request_id):
    deadline = time.time() + TIMEOUT_SECONDS
    while time.time() < deadline:
        ready, _, _ = select.select([process.stdout], [], [], 0.5)
        if not ready:
            continue
        line = process.stdout.readline()
        if not line:
            return None
        message = json.loads(line)
        if message.get("id") == request_id and "method" not in message:
            return message
    return None


def send(process, message):
    process.stdin.write(json.dumps(message) + "\n")
    process.stdin.flush()


def describe(response):
    if response is None:
        return "no response"
    if "error" in response:
        message = str(response["error"].get("message", ""))[:120]
        return f"error {response['error'].get('code')} {message}"
    return "ok"


def main():
    codex = sys.argv[1] if len(sys.argv) > 1 else "codex"
    version = subprocess.run([codex, "--version"], capture_output=True, text=True, check=False)
    print(f"codex: {version.stdout.strip() or version.stderr.strip()}")

    process = subprocess.Popen(
        [codex, "app-server"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        bufsize=1,
    )
    try:
        send(process, {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {"name": "akra-handshake-check", "version": "0"},
                "capabilities": {"experimentalApi": False},
            },
        })
        response = read_response(process, 1)
        result = (response or {}).get("result") or {}
        print(f"initialize: {describe(response)} | keys: {sorted(result.keys())}")

        send(process, {"jsonrpc": "2.0", "method": "initialized", "params": {}})

        send(process, {"jsonrpc": "2.0", "id": 2, "method": "account/read", "params": {}})
        response = read_response(process, 2)
        result = (response or {}).get("result") or {}
        account = result.get("account") or {}
        print(
            f"account/read: {describe(response)} | keys: {sorted(result.keys())}"
            f" | type: {account.get('type')} | plan: {account.get('planType')}"
        )

        send(process, {"jsonrpc": "2.0", "id": 3, "method": "thread/list", "params": {"limit": 3}})
        response = read_response(process, 3)
        result = (response or {}).get("result") or {}
        threads = result.get("data") or []
        thread_keys = sorted(threads[0].keys()) if threads else []
        print(
            f"thread/list: {describe(response)} | keys: {sorted(result.keys())}"
            f" | count: {len(threads)} | thread keys: {thread_keys}"
        )
        return 0 if response is not None and "result" in response else 1
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()


if __name__ == "__main__":
    sys.exit(main())
