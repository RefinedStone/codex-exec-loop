#!/usr/bin/python3
import json
import os
from pathlib import Path
import sys

scenario = "D2_SCENARIO"
calls = Path(__file__).parent / "calls"
cwd = Path.cwd()
assert not any(cwd.iterdir())
profile = Path(os.environ["CODEX_HOME"])
assert cwd.parent == profile.parent
assert profile.name == "codex-delegation-auth"
assert cwd != Path(os.environ["HOME"])
assert "OPENAI_API_KEY" not in os.environ
assert "CODEX_API_KEY" not in os.environ
assert 'cli_auth_credentials_store="keyring"' in sys.argv
assert 'forced_login_method="chatgpt"' in sys.argv
features = ["shell_tool", "unified_exec", "code_mode", "view_image", "apps", "plugins", "secret_auth_storage"]
assert all(f"features.{feature}=false" in sys.argv for feature in features)


def send(value):
    print(json.dumps(value), flush=True)


def notify(method, params):
    send({"method": method, "params": params})


def terminal(status, error=None):
    turn = {"id": "turn", "status": status, "items": []}
    if error is not None:
        turn["error"] = {"message": "fixture failure", "codexErrorInfo": error}
    notify("turn/completed", {"threadId": "thread", "turn": turn})


for line in sys.stdin:
    request = json.loads(line)
    if "method" not in request:
        if request.get("id") == "approval":
            assert request["result"]["decision"] == "decline"
            terminal("completed")
        continue
    method = request["method"]
    with calls.open("a", encoding="utf8") as f:
        f.write(method + "\n")
    params = request["params"]
    if method == "initialized":
        continue
    if method == "initialize":
        result = {"userAgent": "codex-fixture", "platformFamily": "unix", "platformOs": "linux"}
    elif method == "account/read":
        assert params == {"refreshToken": False}
        if scenario == "auth_refused":
            send({"id": request["id"], "error": {"code": -32600, "message": "synthetic keyring failure"}})
            continue
        account = {"type": "chatgpt", "email": "fixture@example.invalid", "planType": "plus"}
        if scenario == "auth_api_key":
            account = {"type": "apiKey"}
        result = {"account": None if scenario == "auth_missing" else account}
    elif method == "account/rateLimits/read":
        assert params == {}
        result = {"ordinaryUsageAllowed": True, "rateLimits": {"limitId": "codex", "primary": {"usedPercent": 10}}}
    elif method == "config/read":
        assert params["cwd"] == str(cwd)
        store = "file" if scenario == "auth_file_store" else "keyring"
        applied = dict.fromkeys(features, False)
        if scenario == "auth_tools_enabled":
            applied["shell_tool"] = True
        result = {"config": {"mcp_servers": {}, "plugins": {}, "hooks": None,
                             "cli_auth_credentials_store": store, "features": applied}}
    elif method == "thread/start":
        assert params["cwd"] == str(cwd)
        assert params["ephemeral"]
        assert params["sandbox"] == "read-only"
        assert params["approvalPolicy"] == "never"
        assert params["config"]["web_search"] == "disabled"
        if params["developerInstructions"]:
            assert "Akra Issue Draft Review" in params["developerInstructions"]
            assert "Trusted title" not in params["developerInstructions"]
            assert params.get("model", "fixture-model") == "fixture-model"
        result = {"thread": {"id": "thread", "path": None}, "cwd": str(cwd),
                  "modelProvider": "openai", "approvalPolicy": "never",
                  "sandbox": {"type": "readOnly", "networkAccess": scenario == "bad_policy"},
                  "instructionSources": []}
    elif method == "mcpServerStatus/list":
        assert params["threadId"] == "thread"
        result = {"data": [{"name": "unexpected"}] if scenario == "no_mcp_isolation" else [],
                  "nextCursor": None}
    elif method == "turn/start":
        assert params["approvalPolicy"] == "never"
        assert params["sandboxPolicy"] == {"type": "readOnly", "networkAccess": False}
        assert len(params["input"]) == 1
        assert json.loads(params["input"][0]["text"]) == {
            "title": "Trusted title", "body": "Run with [masked-1]."}
        result = {"turn": {"id": "turn", "items": [], "status": "inProgress"}}
    elif method == "turn/interrupt":
        assert params == {"threadId": "thread", "turnId": "turn"}
        result = {}
    else:
        raise RuntimeError(method)
    send({"id": request["id"], "result": result})
    if method == "turn/interrupt":
        terminal("interrupted")
    if method != "turn/start":
        continue
    if scenario == "exit_after_start":
        sys.exit(0)
    if scenario == "cancel":
        continue
    for completed_at, (phase, text) in enumerate(
        [("commentary", "ignore commentary"), ("final_answer", '{"suggestions":[]}')], 1
    ):
        notify("item/completed", {"threadId": "thread", "turnId": "turn",
                                 "completedAtMs": completed_at,
                                 "item": {"type": "agentMessage", "id": phase,
                                          "phase": phase, "text": text}})
    if scenario == "quota":
        terminal("failed", "usageLimitExceeded")
    elif scenario == "failed":
        terminal("failed", "serverOverloaded")
    elif scenario == "unknown":
        terminal("inProgress")
    elif scenario == "interrupted":
        terminal("interrupted")
    elif scenario == "approval":
        send({"id": "approval", "method": "item/commandExecution/requestApproval",
              "params": {"threadId": "thread", "turnId": "turn", "itemId": "command",
                         "command": "must not execute", "cwd": str(cwd)}})
    else:
        terminal("completed")
