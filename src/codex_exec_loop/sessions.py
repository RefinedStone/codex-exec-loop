from __future__ import annotations

import json
import os
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path


def default_codex_home() -> Path:
    raw = os.environ.get("CODEX_HOME")
    return Path(raw).expanduser() if raw else Path.home() / ".codex"


@dataclass(frozen=True)
class SessionRecord:
    session_id: str
    last_ts: int
    preview: str
    source: str
    cwd: str = ""
    rollout_path: Path | None = None

    @property
    def last_seen(self) -> str:
        return datetime.fromtimestamp(self.last_ts).astimezone().strftime("%Y-%m-%d %H:%M:%S")

    def matches(self, token: str) -> bool:
        normalized = token.strip().lower()
        if not normalized:
            return True
        return any(
            normalized in candidate.lower()
            for candidate in (self.session_id, self.preview, self.cwd, self.source)
            if candidate
        )

    def to_dict(self) -> dict[str, str | int | None]:
        return {
            "session_id": self.session_id,
            "last_ts": self.last_ts,
            "last_seen": self.last_seen,
            "preview": self.preview,
            "source": self.source,
            "cwd": self.cwd,
            "rollout_path": str(self.rollout_path) if self.rollout_path else None,
        }


def _clean_preview(text: str, limit: int = 120) -> str:
    compact = " ".join((text or "").split())
    if len(compact) <= limit:
        return compact
    return compact[: limit - 3] + "..."


def _safe_json(line: str) -> dict | None:
    try:
        data = json.loads(line)
    except json.JSONDecodeError:
        return None
    return data if isinstance(data, dict) else None


def load_history_sessions(codex_home: Path | None = None) -> dict[str, SessionRecord]:
    home = codex_home or default_codex_home()
    history_path = home / "history.jsonl"
    records: dict[str, SessionRecord] = {}
    if not history_path.exists():
        return records

    with history_path.open(encoding="utf-8") as handle:
        for line in handle:
            data = _safe_json(line)
            if not data:
                continue
            session_id = data.get("session_id")
            ts = int(data.get("ts", 0) or 0)
            if not session_id:
                continue
            preview = _clean_preview(str(data.get("text", "")))
            current = records.get(session_id)
            if current is None or ts >= current.last_ts:
                records[session_id] = SessionRecord(
                    session_id=session_id,
                    last_ts=ts,
                    preview=preview,
                    source="history",
                )
    return records


def load_rollout_sessions(codex_home: Path | None = None) -> dict[str, SessionRecord]:
    home = codex_home or default_codex_home()
    sessions_root = home / "sessions"
    records: dict[str, SessionRecord] = {}
    if not sessions_root.exists():
        return records

    for path in sorted(sessions_root.rglob("rollout-*.jsonl")):
        try:
            with path.open(encoding="utf-8") as handle:
                first_line = handle.readline()
        except OSError:
            continue
        data = _safe_json(first_line)
        if not data:
            continue
        payload = data.get("payload")
        if not isinstance(payload, dict):
            continue
        session_id = payload.get("id")
        if not session_id:
            continue
        ts = int(path.stat().st_mtime)
        current = records.get(session_id)
        if current is None or ts >= current.last_ts:
            records[session_id] = SessionRecord(
                session_id=session_id,
                last_ts=ts,
                preview=_clean_preview(str(payload.get("cwd", ""))),
                source="rollout",
                cwd=str(payload.get("cwd", "")),
                rollout_path=path,
            )
    return records


def build_session_index(codex_home: Path | None = None) -> dict[str, SessionRecord]:
    history_records = load_history_sessions(codex_home)
    rollout_records = load_rollout_sessions(codex_home)
    merged = dict(rollout_records)
    for session_id, record in history_records.items():
        current = merged.get(session_id)
        if current is None:
            merged[session_id] = record
            continue
        merged[session_id] = SessionRecord(
            session_id=session_id,
            last_ts=max(record.last_ts, current.last_ts),
            preview=record.preview or current.preview,
            source="history+rollout",
            cwd=current.cwd or record.cwd,
            rollout_path=current.rollout_path,
        )
    return merged


def list_recent_sessions(limit: int = 10, query: str | None = None, codex_home: Path | None = None) -> list[SessionRecord]:
    index = build_session_index(codex_home)
    records = sorted(index.values(), key=lambda item: item.last_ts, reverse=True)
    if query:
        records = [record for record in records if record.matches(query)]
    return records[:limit]


def resolve_session(session_id: str, codex_home: Path | None = None) -> SessionRecord | None:
    return build_session_index(codex_home).get(session_id)


def find_session_matches(token: str, limit: int = 20, codex_home: Path | None = None) -> list[SessionRecord]:
    records = list_recent_sessions(limit=limit, query=token, codex_home=codex_home)
    exact = [record for record in records if record.session_id == token]
    if exact:
        return exact
    prefix = [record for record in records if record.session_id.startswith(token)]
    if prefix:
        return prefix
    return records
