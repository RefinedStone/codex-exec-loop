from __future__ import annotations

import json
from pathlib import Path


def emit(message: str) -> None:
    print(message, flush=True)


def parse_contains_spec(spec: str) -> tuple[Path, str]:
    path_text, separator, expected = spec.partition("::")
    if not separator:
        raise ValueError(f"--must-contain 형식은 PATH::TEXT 이어야 합니다: {spec}")
    path = Path(path_text).expanduser()
    return path, expected


def load_summary(path: Path | None) -> dict:
    if path is None:
        return {}
    return json.loads(path.read_text(encoding="utf-8"))


def collect_changed_paths(summary: dict) -> set[str]:
    collected: set[str] = set()
    for turn in summary.get("turns", []):
        if not isinstance(turn, dict):
            continue
        for change in turn.get("file_changes", []):
            if not isinstance(change, dict):
                continue
            absolute = change.get("path")
            relative = change.get("relative_path")
            if isinstance(absolute, str):
                collected.add(str(Path(absolute).expanduser().resolve()))
            if isinstance(relative, str):
                collected.add(relative)
    return collected


def verify_run(
    summary_path: Path | None,
    must_exist: list[Path],
    must_contain_specs: list[str],
    expect_changed: list[Path],
    show_files: list[Path],
) -> int:
    summary = load_summary(summary_path)
    changed_paths = collect_changed_paths(summary)
    failures: list[str] = []

    if summary_path is not None:
        emit(f"[SUMMARY] {summary_path}")
        session_id = summary.get("session_id") or "-"
        stop_reason = summary.get("stop_reason") or "-"
        emit(f"[SUMMARY] session_id={session_id} stop_reason={stop_reason}")

    for raw_path in must_exist:
        path = raw_path.expanduser()
        if path.exists():
            emit(f"[PASS] exists: {path}")
        else:
            failures.append(f"missing file: {path}")
            emit(f"[FAIL] missing file: {path}")

    for spec in must_contain_specs:
        path, expected = parse_contains_spec(spec)
        target = path.expanduser()
        if not target.exists():
            failures.append(f"missing file for contains check: {target}")
            emit(f"[FAIL] missing file for contains check: {target}")
            continue
        content = target.read_text(encoding="utf-8")
        if expected in content:
            emit(f"[PASS] contains: {target} :: {expected}")
        else:
            failures.append(f"text not found in {target}: {expected}")
            emit(f"[FAIL] text not found in {target}: {expected}")

    for raw_path in expect_changed:
        target = raw_path.expanduser()
        normalized = str(target.resolve())
        relative = raw_path.as_posix()
        if normalized in changed_paths or relative in changed_paths:
            emit(f"[PASS] changed: {raw_path}")
        else:
            failures.append(f"expected changed path not found in summary: {raw_path}")
            emit(f"[FAIL] expected changed path not found in summary: {raw_path}")

    for raw_path in show_files:
        target = raw_path.expanduser()
        emit(f"[FILE] {target}")
        if target.exists():
            emit(target.read_text(encoding="utf-8"))
        else:
            emit("(missing)")

    if failures:
        emit(f"[VERIFY] failed: {len(failures)} issue(s)")
        return 1

    emit("[VERIFY] success")
    return 0
