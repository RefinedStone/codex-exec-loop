from __future__ import annotations

import json
import shlex
import subprocess
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path


class RunError(RuntimeError):
    pass


def emit(message: str) -> None:
    print(message, flush=True)


@dataclass(frozen=True)
class FileChange:
    path: str
    kind: str


@dataclass
class CodexCommandConfig:
    codex_bin: str = "codex"
    cwd: Path | None = None
    model: str | None = None
    config_overrides: list[str] = field(default_factory=list)
    skip_git_repo_check: bool = True
    full_auto: bool = False
    output_schema: Path | None = None

    def build_new_command(self, prompt: str, output_last_message_path: Path | None = None) -> list[str]:
        command = [self.codex_bin, "exec", "--json"]
        if self.cwd is not None:
            command.extend(["-C", str(self.cwd)])
        if self.model:
            command.extend(["-m", self.model])
        if self.full_auto:
            command.append("--full-auto")
        if self.skip_git_repo_check:
            command.append("--skip-git-repo-check")
        if self.output_schema is not None:
            command.extend(["--output-schema", str(self.output_schema)])
        if output_last_message_path is not None:
            command.extend(["-o", str(output_last_message_path)])
        for override in self.config_overrides:
            command.extend(["-c", override])
        command.append(prompt)
        return command

    def build_resume_command(
        self,
        session_id: str,
        prompt: str,
        output_last_message_path: Path | None = None,
    ) -> list[str]:
        command = [self.codex_bin, "exec", "resume", "--json"]
        if self.model:
            command.extend(["-m", self.model])
        if self.full_auto:
            command.append("--full-auto")
        if self.skip_git_repo_check:
            command.append("--skip-git-repo-check")
        if self.output_schema is not None:
            command.extend(["--output-schema", str(self.output_schema)])
        if output_last_message_path is not None:
            command.extend(["-o", str(output_last_message_path)])
        for override in self.config_overrides:
            command.extend(["-c", override])
        command.extend([session_id, prompt])
        return command


@dataclass
class TurnResult:
    session_id: str
    last_message: str
    usage: dict[str, int]
    return_code: int
    file_changes: list[FileChange]
    output_last_message_path: Path | None = None


def append_transcript_line(path: Path | None, line: str) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(line + "\n")


def _safe_json(line: str) -> dict | None:
    try:
        data = json.loads(line)
    except json.JSONDecodeError:
        return None
    return data if isinstance(data, dict) else None


def _extract_agent_text(item: dict) -> str:
    text = item.get("text")
    if isinstance(text, str) and text.strip():
        return text.strip()
    content = item.get("content")
    if not isinstance(content, list):
        return ""
    parts: list[str] = []
    for part in content:
        if not isinstance(part, dict):
            continue
        candidate = part.get("text")
        if isinstance(candidate, str) and candidate.strip():
            parts.append(candidate.strip())
    return "\n".join(parts).strip()


def _load_output_last_message(path: Path | None) -> str:
    if path is None or not path.exists():
        return ""
    return path.read_text(encoding="utf-8").strip()


def run_turn(
    command: list[str],
    transcript_path: Path | None = None,
    output_last_message_path: Path | None = None,
) -> TurnResult:
    append_transcript_line(
        transcript_path,
        f"===== codex turn start {datetime.now().astimezone().isoformat()} =====",
    )
    append_transcript_line(transcript_path, f"$ {shlex.join(command)}")

    process = subprocess.Popen(
        command,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
    )

    session_id = ""
    last_message = ""
    usage: dict[str, int] = {}
    completed = False
    recent_error = ""
    recent_plain_error = ""
    file_changes: list[FileChange] = []
    seen_changes: set[tuple[str, str]] = set()

    assert process.stdout is not None
    for raw_line in process.stdout:
        line = raw_line.rstrip("\n")
        append_transcript_line(transcript_path, line)
        event = _safe_json(line)
        if event is None:
            if line:
                emit(line)
                if line.startswith("Error:") or line.startswith("fatal:"):
                    recent_plain_error = line
            continue

        event_type = event.get("type")
        if event_type == "thread.started":
            session_id = str(event.get("thread_id", ""))
            emit(f"[SESSION] {session_id}")
            continue
        if event_type == "turn.started":
            emit("[TURN] started")
            continue
        if event_type == "item.completed":
            item = event.get("item")
            if not isinstance(item, dict):
                continue
            item_type = item.get("type")
            if item_type == "agent_message":
                last_message = _extract_agent_text(item)
                if last_message:
                    emit("[AGENT]")
                    emit(last_message)
                continue
            if item_type == "file_change":
                raw_changes = item.get("changes")
                if not isinstance(raw_changes, list):
                    continue
                for change in raw_changes:
                    if not isinstance(change, dict):
                        continue
                    path = str(change.get("path", "")).strip()
                    kind = str(change.get("kind", "")).strip() or "update"
                    if not path:
                        continue
                    key = (path, kind)
                    if key in seen_changes:
                        continue
                    seen_changes.add(key)
                    file_changes.append(FileChange(path=path, kind=kind))
            continue
        if event_type == "error":
            recent_error = str(event.get("message", "")).strip()
            if recent_error:
                emit(f"[ERROR] {recent_error}")
            continue
        if event_type == "turn.completed":
            raw_usage = event.get("usage")
            usage = raw_usage if isinstance(raw_usage, dict) else {}
            completed = True
            input_tokens = usage.get("input_tokens", 0)
            output_tokens = usage.get("output_tokens", 0)
            emit(f"[TURN] completed input={input_tokens} output={output_tokens}")
            continue

    return_code = process.wait()
    append_transcript_line(
        transcript_path,
        f"===== codex turn end rc={return_code} completed={completed} =====",
    )

    file_last_message = _load_output_last_message(output_last_message_path)
    if file_last_message:
        last_message = file_last_message

    if return_code != 0:
        detail = recent_error or recent_plain_error or f"codex exited with status {return_code}"
        raise RunError(detail)
    if not completed:
        raise RunError("turn.completed event was not observed")
    if not session_id:
        raise RunError("thread.started event did not provide a session id")

    return TurnResult(
        session_id=session_id,
        last_message=last_message,
        usage=usage,
        return_code=return_code,
        file_changes=file_changes,
        output_last_message_path=output_last_message_path,
    )
