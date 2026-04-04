from __future__ import annotations

import argparse
import json
import shutil
import sys
from dataclasses import replace
from datetime import datetime
from pathlib import Path

from . import __version__
from .runner import CodexCommandConfig, FileChange, RunError, append_transcript_line, run_turn
from .runs import RunArtifacts, prepare_run_artifacts, write_last_session_id, write_summary
from .sessions import (
    SessionRecord,
    find_session_matches,
    list_recent_sessions,
    resolve_session,
)
from .verifier import verify_run


FOLLOWUP_STRATEGIES = {
    "last-message": """대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

직전 답변:
{last_message}

방금 결과를 기준으로 다음 작업 1개만 이어서 진행하세요.""",
    "plan-queue": """대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 결과를 바탕으로 개선점과 다음 작업 후보를 `plan_priority_queue.md` 에 정리하고,
가장 우선순위가 높은 항목 1개를 바로 진행하세요.

직전 답변:
{last_message}""",
    "bugfix": """대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 변경분 기준으로 남아 있는 버그나 리스크 1개만 골라 수정하세요.
수정이 끝나면 무엇을 고쳤는지 짧게 요약하세요.

직전 답변:
{last_message}""",
    "docs": """대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 작업을 기준으로 사용자 문서나 README 에 반영할 내용 1개만 정리하고 적용하세요.

직전 답변:
{last_message}""",
    "next-task": """대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 결과를 기준으로 다음 작업 1개만 이어서 진행하세요.

직전 답변:
{last_message}""",
}


class SafeFormatDict(dict):
    def __missing__(self, key: str) -> str:
        return "{" + key + "}"


def emit(message: str) -> None:
    print(message, flush=True)


def parse_auto_turns(value: str) -> int | None:
    normalized = value.strip().lower()
    if normalized in {"inf", "infinite", "unlimited", "-1"}:
        return None
    try:
        parsed = int(normalized)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(
            "--max-auto-turns 는 0 이상의 정수 또는 inf/infinite/unlimited/-1 이어야 합니다."
        ) from exc
    if parsed < 0:
        raise argparse.ArgumentTypeError("--max-auto-turns 는 0 이상의 정수 또는 inf/infinite/unlimited/-1 이어야 합니다.")
    return parsed


def format_auto_turn_limit(value: int | None) -> str:
    return "infinite" if value is None else str(value)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    raw_argv = list(argv if argv is not None else sys.argv[1:])
    if raw_argv and raw_argv[0] not in {"run", "sessions", "verify", "-h", "--help", "-V", "--version"}:
        raw_argv.insert(0, "run")
    if not raw_argv:
        raw_argv = ["run"]

    parser = argparse.ArgumentParser(
        prog="codex-exec-loop",
        description="Run Codex exec/resume loops with session selection, stop rules, and verification.",
    )
    parser.add_argument("-V", "--version", action="version", version=f"%(prog)s {__version__}")
    subparsers = parser.add_subparsers(dest="command", required=True)

    run_parser = subparsers.add_parser("run", help="start a new Codex exec run or resume a prior session")
    run_parser.add_argument("prompt", nargs="?", help="initial prompt to send")
    run_parser.add_argument("--mode", choices=("new", "existing"), help="session mode")
    run_parser.add_argument("--session-id", help="existing Codex session id to resume")
    run_parser.add_argument("--recent-limit", type=int, default=10, help="recent session count to display")
    run_parser.add_argument("--prompt-file", type=Path, help="read the initial prompt from a file")
    run_parser.add_argument("--followup", help="follow-up template text")
    run_parser.add_argument("--followup-file", type=Path, help="read the follow-up template from a file")
    run_parser.add_argument(
        "--followup-strategy",
        choices=sorted(FOLLOWUP_STRATEGIES),
        default="last-message",
        help="built-in follow-up template strategy",
    )
    run_parser.add_argument(
        "--max-auto-turns",
        type=parse_auto_turns,
        default=1,
        help="number of automatic resume turns after the first prompt, or inf/infinite/unlimited/-1",
    )
    run_parser.add_argument("--cwd", type=Path, help="working directory for new sessions")
    run_parser.add_argument("--codex-bin", default="codex", help="Codex executable to run")
    run_parser.add_argument("--model", help="Codex model override")
    run_parser.add_argument("--output-schema", type=Path, help="pass --output-schema to Codex exec")
    run_parser.add_argument("-c", "--config", action="append", default=[], help="extra Codex config overrides")
    run_parser.add_argument("--full-auto", action="store_true", help="pass --full-auto to Codex")
    run_parser.add_argument(
        "-y",
        "--yes",
        "--non-interactive",
        dest="non_interactive",
        action="store_true",
        help="disable interactive prompts and require explicit inputs",
    )
    run_parser.add_argument(
        "--skip-git-repo-check",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="pass --skip-git-repo-check to Codex",
    )
    run_parser.add_argument("--transcript", type=Path, help="write raw JSONL output and wrapper logs here")
    run_parser.add_argument("--output-dir", type=Path, help="directory for structured run output")
    run_parser.add_argument(
        "--stop-on-keyword",
        action="append",
        default=[],
        help="stop before the next auto-follow-up when the last message contains this keyword",
    )
    run_parser.add_argument(
        "--stop-when-no-files-changed",
        action="store_true",
        help="stop before the next auto-follow-up when the turn reports no file_change items",
    )
    run_parser.add_argument(
        "--fallback-new-on-missing-session",
        action="store_true",
        help="if an existing session cannot be resumed, fall back to a new session with the same prompt",
    )

    sessions_parser = subparsers.add_parser("sessions", help="list recent local Codex sessions")
    sessions_parser.add_argument("--limit", type=int, default=10, help="number of sessions to print")
    sessions_parser.add_argument("--query", help="filter by session id, cwd, or preview text")
    sessions_parser.add_argument("--json", action="store_true", help="print session list as JSON")
    sessions_parser.add_argument("--show-paths", action="store_true", help="print rollout file paths")

    verify_parser = subparsers.add_parser("verify", help="verify work products and structured run summaries")
    verify_parser.add_argument("--summary", type=Path, help="summary.json produced by --output-dir")
    verify_parser.add_argument("--must-exist", action="append", type=Path, default=[], help="file that must exist")
    verify_parser.add_argument(
        "--must-contain",
        action="append",
        default=[],
        help="content assertion in the form PATH::TEXT",
    )
    verify_parser.add_argument(
        "--expect-changed",
        action="append",
        type=Path,
        default=[],
        help="path that must appear in summary file_changes",
    )
    verify_parser.add_argument("--show-file", action="append", type=Path, default=[], help="print a file after verify")
    return parser.parse_args(raw_argv)


def read_text_argument(
    text: str | None,
    path: Path | None,
    prompt_label: str,
    interactive_allowed: bool = True,
) -> str:
    if path is not None:
        return path.read_text(encoding="utf-8").strip()
    if text is not None:
        return text.strip()
    if not sys.stdin.isatty():
        return sys.stdin.read().strip()
    if not interactive_allowed:
        return ""
    return input(prompt_label).strip()


def read_optional_text(text: str | None, path: Path | None) -> str:
    if path is not None:
        return path.read_text(encoding="utf-8").strip()
    if text is not None:
        return text.strip()
    return ""


def choose_mode() -> str:
    while True:
        emit("세션 모드를 선택하세요.")
        emit("  1. new")
        emit("  2. existing")
        choice = input("> ").strip().lower()
        if choice in {"1", "new", "n"}:
            return "new"
        if choice in {"2", "existing", "e"}:
            return "existing"
        emit("`new` 또는 `existing` 을 다시 입력하세요.")


def print_session_records(records: list[SessionRecord], show_paths: bool = False) -> None:
    for index, record in enumerate(records, start=1):
        emit(f"{index:>2}. {record.session_id}  {record.last_seen}  [{record.source}]")
        if record.cwd:
            emit(f"    cwd: {record.cwd}")
        if record.preview:
            emit(f"    last: {record.preview}")
        if show_paths and record.rollout_path:
            emit(f"    rollout: {record.rollout_path}")


def resolve_session_token(token: str, limit: int) -> SessionRecord | None:
    exact = resolve_session(token)
    if exact is not None:
        return exact
    matches = find_session_matches(token, limit=max(limit, 20))
    if len(matches) == 1:
        return matches[0]
    return None


def choose_existing_session(limit: int) -> SessionRecord:
    records = list_recent_sessions(limit=limit)
    if not records:
        raise SystemExit("로컬 Codex 세션을 찾지 못했습니다.")

    current_records = records
    while True:
        emit("최근 세션 목록:")
        print_session_records(current_records)
        choice = input("번호, session_id, 또는 /검색어 를 입력하세요: ").strip()
        if not choice:
            emit("빈 값은 사용할 수 없습니다.")
            continue
        if choice.startswith("/"):
            query = choice[1:].strip()
            current_records = list_recent_sessions(limit=limit, query=query) if query else records
            if not current_records:
                emit("검색 결과가 없습니다.")
                current_records = records
            continue
        if choice.isdigit():
            number = int(choice)
            if 1 <= number <= len(current_records):
                return current_records[number - 1]
            emit("목록 범위를 벗어났습니다.")
            continue
        resolved = resolve_session_token(choice, limit=limit)
        if resolved is not None:
            return resolved
        matches = find_session_matches(choice, limit=max(limit, 20))
        if not matches:
            emit("일치하는 세션을 찾지 못했습니다.")
            continue
        emit("일치 후보가 여러 개입니다. 더 긴 session_id 를 입력하거나 번호를 고르세요.")
        print_session_records(matches[:limit])
        current_records = matches[:limit]


def validate_session_or_exit(session_token: str, limit: int) -> SessionRecord:
    record = resolve_session_token(session_token, limit=limit)
    if record is None:
        raise SystemExit(f"session_id 를 찾지 못했습니다: {session_token}")
    return record


def render_followup(
    template: str,
    auto_turn: int,
    max_auto_turns: int | None,
    session_id: str,
    last_message: str,
) -> str:
    values = SafeFormatDict(
        auto_turn=auto_turn,
        max_auto_turns=format_auto_turn_limit(max_auto_turns),
        session_id=session_id,
        last_message=last_message,
    )
    return template.format_map(values).strip()


def ensure_codex_executable(codex_bin: str) -> None:
    if "/" in codex_bin:
        path = Path(codex_bin).expanduser()
        if not path.exists():
            raise SystemExit(f"Codex 실행 파일을 찾지 못했습니다: {codex_bin}")
        return
    if shutil.which(codex_bin) is None:
        raise SystemExit(f"PATH 에 Codex 실행 파일이 없습니다: {codex_bin}")


def choose_run_mode(args: argparse.Namespace) -> str:
    if args.mode:
        return args.mode
    if args.session_id:
        return "existing"
    if args.non_interactive:
        return "new"
    if args.prompt or args.prompt_file or not sys.stdin.isatty():
        return "new"
    return choose_mode()


def build_followup_template(args: argparse.Namespace) -> str:
    direct_template = read_optional_text(args.followup, args.followup_file)
    if direct_template:
        return direct_template
    return FOLLOWUP_STRATEGIES[args.followup_strategy]


def reset_run_artifacts(artifacts: RunArtifacts) -> None:
    paths = [artifacts.transcript_path, artifacts.summary_path, artifacts.session_id_path]
    for path in paths:
        if path is not None and path.exists():
            path.unlink()
    if artifacts.turns_dir is not None:
        for child in artifacts.turns_dir.glob("turn-*-last-message.txt"):
            child.unlink()


def serialize_file_changes(file_changes: list[FileChange], root_dir: Path | None) -> list[dict[str, str]]:
    serialized: list[dict[str, str]] = []
    normalized_root = root_dir.resolve() if root_dir is not None else None
    for change in file_changes:
        raw_path = Path(change.path).expanduser()
        if raw_path.is_absolute():
            normalized_path = raw_path
        elif normalized_root is not None:
            normalized_path = (normalized_root / raw_path).resolve()
        else:
            normalized_path = raw_path.resolve()
        item = {"path": str(normalized_path), "kind": change.kind}
        if normalized_root is not None:
            try:
                item["relative_path"] = str(normalized_path.relative_to(normalized_root))
            except ValueError:
                pass
        serialized.append(item)
    return serialized


def detect_stop_reason(args: argparse.Namespace, file_changes: list[FileChange], last_message: str) -> str | None:
    text = (last_message or "").lower()
    for keyword in args.stop_on_keyword:
        if keyword and keyword.lower() in text:
            return f"keyword:{keyword}"
    if args.stop_when_no_files_changed and not file_changes:
        return "no-files-changed"
    return None


def is_missing_session_error(message: str) -> bool:
    return "no rollout found for thread id" in message.lower()


def run_sessions_command(args: argparse.Namespace) -> int:
    records = list_recent_sessions(limit=args.limit, query=args.query)
    if args.json:
        print(json.dumps([record.to_dict() for record in records], ensure_ascii=False, indent=2))
        return 0
    if not records:
        emit("표시할 로컬 Codex 세션이 없습니다.")
        return 0
    print_session_records(records, show_paths=args.show_paths)
    return 0


def run_verify_command(args: argparse.Namespace) -> int:
    return verify_run(
        summary_path=args.summary.expanduser() if args.summary else None,
        must_exist=list(args.must_exist),
        must_contain_specs=list(args.must_contain),
        expect_changed=list(args.expect_changed),
        show_files=list(args.show_file),
    )


def run_loop(args: argparse.Namespace) -> int:
    ensure_codex_executable(args.codex_bin)

    mode = choose_run_mode(args)
    if mode == "new" and args.session_id:
        raise SystemExit("--mode new 에서는 --session-id 를 같이 쓸 수 없습니다.")

    existing_record: SessionRecord | None = None
    session_id = ""
    if mode == "existing":
        if args.session_id:
            existing_record = validate_session_or_exit(args.session_id, limit=args.recent_limit)
        elif args.non_interactive:
            raise SystemExit("--mode existing 에서는 --session-id 가 필요합니다.")
        else:
            existing_record = choose_existing_session(args.recent_limit)
        session_id = existing_record.session_id
        emit(f"[INFO] existing session: {existing_record.session_id} ({existing_record.last_seen})")
        if existing_record.cwd:
            emit(f"[INFO] cwd: {existing_record.cwd}")
        if existing_record.preview:
            emit(f"[INFO] last prompt: {existing_record.preview}")
        if args.cwd:
            emit("[WARN] --cwd 는 existing session 에서는 resume 자체에는 쓰이지 않습니다.")

    initial_prompt = read_text_argument(
        args.prompt,
        args.prompt_file,
        "보낼 프롬프트: ",
        interactive_allowed=not args.non_interactive,
    )
    if not initial_prompt:
        raise SystemExit("첫 프롬프트가 비어 있습니다. --prompt-file 또는 PROMPT 를 지정하세요.")

    followup_template = ""
    if args.max_auto_turns != 0:
        followup_template = build_followup_template(args)
    if args.max_auto_turns is None and not args.stop_on_keyword and not args.stop_when_no_files_changed:
        emit("[WARN] infinite auto-turns 에 stop rule 이 없습니다. 명시적으로 중단하기 전까지 계속 실행됩니다.")

    cwd_for_new_runs = args.cwd.expanduser().resolve() if args.cwd else None
    effective_root_dir = cwd_for_new_runs
    if effective_root_dir is None and existing_record and existing_record.cwd:
        effective_root_dir = Path(existing_record.cwd).expanduser().resolve()
    if effective_root_dir is None:
        effective_root_dir = Path.cwd().resolve()

    artifacts = prepare_run_artifacts(args.output_dir, args.transcript)
    if artifacts.output_dir is not None:
        reset_run_artifacts(artifacts)
    transcript_path = artifacts.transcript_path

    append_transcript_line(
        transcript_path,
        f"===== wrapper start mode={mode} auto_turns={format_auto_turn_limit(args.max_auto_turns)} started_at={datetime.now().astimezone().isoformat()} =====",
    )

    runner_config = CodexCommandConfig(
        codex_bin=args.codex_bin,
        cwd=cwd_for_new_runs,
        model=args.model,
        output_schema=args.output_schema.expanduser().resolve() if args.output_schema else None,
        config_overrides=list(args.config),
        skip_git_repo_check=args.skip_git_repo_check,
        full_auto=args.full_auto,
    )
    fallback_runner_config = runner_config
    if fallback_runner_config.cwd is None:
        fallback_runner_config = replace(fallback_runner_config, cwd=effective_root_dir)

    prompt = initial_prompt
    last_message = ""
    started_at = datetime.now().astimezone().isoformat()
    stop_reason = ""
    turn_reports: list[dict] = []

    try:
        zero_based_turn = 0
        while True:
            turn_number = zero_based_turn + 1
            total_turns_display = f"{args.max_auto_turns + 1}" if args.max_auto_turns is not None else "∞"
            resume_mode = mode == "existing" or zero_based_turn > 0
            label = "resume" if resume_mode else "new"
            output_last_message_path = artifacts.turn_last_message_path(turn_number)

            emit(f"[RUN] {label} turn {turn_number}/{total_turns_display}")
            emit("[USER]")
            emit(prompt)
            append_transcript_line(transcript_path, f"[USER TURN {turn_number}] {prompt}")

            command = (
                runner_config.build_resume_command(session_id, prompt, output_last_message_path=output_last_message_path)
                if resume_mode
                else runner_config.build_new_command(prompt, output_last_message_path=output_last_message_path)
            )

            try:
                result = run_turn(
                    command,
                    transcript_path=transcript_path,
                    output_last_message_path=output_last_message_path,
                )
            except RunError as exc:
                if (
                    resume_mode
                    and zero_based_turn == 0
                    and args.fallback_new_on_missing_session
                    and is_missing_session_error(str(exc))
                ):
                    fallback_message = "stored session could not be resumed; falling back to a new session"
                    emit(f"[WARN] {fallback_message}")
                    append_transcript_line(transcript_path, f"[WARN] {fallback_message}")
                    command = fallback_runner_config.build_new_command(
                        prompt,
                        output_last_message_path=output_last_message_path,
                    )
                    result = run_turn(
                        command,
                        transcript_path=transcript_path,
                        output_last_message_path=output_last_message_path,
                    )
                    mode = "new"
                else:
                    raise

            session_id = result.session_id
            last_message = result.last_message
            write_last_session_id(artifacts.session_id_path, session_id)

            serialized_changes = serialize_file_changes(result.file_changes, effective_root_dir)
            turn_reports.append(
                {
                    "index": turn_number,
                    "mode": label,
                    "prompt": prompt,
                    "session_id": session_id,
                    "last_message": last_message,
                    "usage": result.usage,
                    "file_changes": serialized_changes,
                    "changed_file_count": len(serialized_changes),
                    "command": command,
                    "output_last_message_path": str(output_last_message_path) if output_last_message_path else None,
                }
            )

            if args.max_auto_turns is not None and zero_based_turn >= args.max_auto_turns:
                stop_reason = "max-auto-turns"
                break

            stop_reason = detect_stop_reason(args, result.file_changes, last_message) or ""
            if stop_reason:
                emit(f"[STOP] {stop_reason}")
                append_transcript_line(transcript_path, f"[STOP] {stop_reason}")
                break

            prompt = render_followup(
                template=followup_template,
                auto_turn=turn_number,
                max_auto_turns=args.max_auto_turns,
                session_id=session_id,
                last_message=last_message,
            )
            emit(f"[AUTO] queued follow-up {turn_number}/{format_auto_turn_limit(args.max_auto_turns)}")
            zero_based_turn += 1

    except RunError as exc:
        stop_reason = "error"
        append_transcript_line(transcript_path, f"[ERROR] {exc}")
        emit(f"[FATAL] {exc}")
        return_code = 1
    else:
        emit(f"[DONE] session_id={session_id}")
        return_code = 0
    finally:
        ended_at = datetime.now().astimezone().isoformat()
        append_transcript_line(transcript_path, "===== wrapper end =====")
        summary_payload = {
            "version": 1,
            "started_at": started_at,
            "ended_at": ended_at,
            "mode": mode,
            "session_id": session_id,
            "working_dir": str(effective_root_dir),
            "output_dir": str(artifacts.output_dir) if artifacts.output_dir else None,
            "transcript_path": str(transcript_path) if transcript_path else None,
            "followup_strategy": args.followup_strategy,
            "max_auto_turns": args.max_auto_turns,
            "max_auto_turns_label": format_auto_turn_limit(args.max_auto_turns),
            "stop_reason": stop_reason,
            "turns": turn_reports,
        }
        write_summary(artifacts.summary_path, summary_payload)

    return return_code


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.command == "sessions":
        return run_sessions_command(args)
    if args.command == "verify":
        return run_verify_command(args)
    return run_loop(args)
