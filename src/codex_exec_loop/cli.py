from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

from . import __version__
from .runner import CodexCommandConfig, RunError, append_transcript_line, run_turn
from .sessions import SessionRecord, list_recent_sessions, resolve_session


DEFAULT_FOLLOWUP_TEMPLATE = """대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

직전 답변:
{last_message}

방금 결과를 기준으로 다음 작업 1개만 이어서 진행하세요."""


class SafeFormatDict(dict):
    def __missing__(self, key: str) -> str:
        return "{" + key + "}"


def emit(message: str) -> None:
    print(message, flush=True)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    raw_argv = list(argv if argv is not None else sys.argv[1:])
    if raw_argv and raw_argv[0] not in {"run", "sessions", "-h", "--help", "-V", "--version"}:
        raw_argv.insert(0, "run")
    if not raw_argv:
        raw_argv = ["run"]

    parser = argparse.ArgumentParser(
        prog="codex-exec-loop",
        description="Run Codex exec/resume loops with session selection and automatic follow-ups.",
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
        "--max-auto-turns",
        type=int,
        default=1,
        help="number of automatic resume turns after the first prompt",
    )
    run_parser.add_argument("--cwd", type=Path, help="working directory for new sessions")
    run_parser.add_argument("--codex-bin", default="codex", help="Codex executable to run")
    run_parser.add_argument("--model", help="Codex model override")
    run_parser.add_argument("-c", "--config", action="append", default=[], help="extra Codex config overrides")
    run_parser.add_argument("--full-auto", action="store_true", help="pass --full-auto to Codex")
    run_parser.add_argument(
        "--skip-git-repo-check",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="pass --skip-git-repo-check to Codex",
    )
    run_parser.add_argument("--transcript", type=Path, help="write raw JSONL output and wrapper logs here")

    sessions_parser = subparsers.add_parser("sessions", help="list recent local Codex sessions")
    sessions_parser.add_argument("--limit", type=int, default=10, help="number of sessions to print")
    return parser.parse_args(raw_argv)


def read_text_argument(text: str | None, path: Path | None, prompt_label: str) -> str:
    if path is not None:
        return path.read_text(encoding="utf-8").strip()
    if text is not None:
        return text.strip()
    if not sys.stdin.isatty():
        return sys.stdin.read().strip()
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


def print_session_records(records: list[SessionRecord]) -> None:
    for index, record in enumerate(records, start=1):
        emit(f"{index:>2}. {record.session_id}  {record.last_seen}")
        emit(f"    {record.preview}")


def choose_existing_session(limit: int) -> str:
    records = list_recent_sessions(limit=limit)
    if not records:
        raise SystemExit("로컬 Codex 세션을 찾지 못했습니다.")

    emit("최근 세션 목록:")
    print_session_records(records)

    while True:
        choice = input("번호 또는 session_id 를 입력하세요: ").strip()
        if not choice:
            emit("빈 값은 사용할 수 없습니다.")
            continue
        if choice.isdigit():
            number = int(choice)
            if 1 <= number <= len(records):
                return records[number - 1].session_id
            emit("목록 범위를 벗어났습니다.")
            continue
        return choice


def validate_session_or_exit(session_id: str) -> SessionRecord:
    record = resolve_session(session_id)
    if record is None:
        raise SystemExit(f"session_id 를 찾지 못했습니다: {session_id}")
    return record


def render_followup(template: str, auto_turn: int, max_auto_turns: int, session_id: str, last_message: str) -> str:
    values = SafeFormatDict(
        auto_turn=auto_turn,
        max_auto_turns=max_auto_turns,
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


def run_sessions_command(args: argparse.Namespace) -> int:
    records = list_recent_sessions(limit=args.limit)
    if not records:
        emit("표시할 로컬 Codex 세션이 없습니다.")
        return 0
    print_session_records(records)
    return 0


def run_loop(args: argparse.Namespace) -> int:
    if args.max_auto_turns < 0:
        raise SystemExit("--max-auto-turns 는 0 이상이어야 합니다.")

    ensure_codex_executable(args.codex_bin)

    mode = args.mode
    if args.session_id and mode is None:
        mode = "existing"
    if mode is None:
        mode = choose_mode() if sys.stdin.isatty() else "new"

    if mode == "new" and args.session_id:
        raise SystemExit("--mode new 에서는 --session-id 를 같이 쓸 수 없습니다.")

    session_id = ""
    if mode == "existing":
        session_id = args.session_id or choose_existing_session(args.recent_limit)
        record = validate_session_or_exit(session_id)
        emit(f"[INFO] existing session: {record.session_id} ({record.last_seen})")
        if record.preview:
            emit(f"[INFO] last prompt: {record.preview}")
        if args.cwd:
            emit("[WARN] --cwd 는 existing session 에서는 무시됩니다.")

    initial_prompt = read_text_argument(args.prompt, args.prompt_file, "보낼 프롬프트: ")
    if not initial_prompt:
        raise SystemExit("첫 프롬프트가 비어 있습니다.")

    followup_template = ""
    if args.max_auto_turns > 0:
        followup_template = read_optional_text(args.followup, args.followup_file) or DEFAULT_FOLLOWUP_TEMPLATE

    transcript_path = args.transcript.expanduser() if args.transcript else None
    append_transcript_line(
        transcript_path,
        f"===== wrapper start mode={mode} auto_turns={args.max_auto_turns} =====",
    )

    runner_config = CodexCommandConfig(
        codex_bin=args.codex_bin,
        cwd=args.cwd.expanduser().resolve() if args.cwd else None,
        model=args.model,
        config_overrides=list(args.config),
        skip_git_repo_check=args.skip_git_repo_check,
        full_auto=args.full_auto,
    )

    prompt = initial_prompt
    last_message = ""

    try:
        for turn_index in range(args.max_auto_turns + 1):
            resume_mode = mode == "existing" or turn_index > 0
            label = "resume" if resume_mode else "new"
            emit(f"[RUN] {label} turn {turn_index + 1}/{args.max_auto_turns + 1}")
            emit("[USER]")
            emit(prompt)
            append_transcript_line(transcript_path, f"[USER TURN {turn_index + 1}] {prompt}")

            command = (
                runner_config.build_resume_command(session_id, prompt)
                if resume_mode
                else runner_config.build_new_command(prompt)
            )
            result = run_turn(command, transcript_path=transcript_path)
            session_id = result.session_id
            last_message = result.last_message

            if turn_index >= args.max_auto_turns:
                break

            prompt = render_followup(
                template=followup_template or DEFAULT_FOLLOWUP_TEMPLATE,
                auto_turn=turn_index + 1,
                max_auto_turns=args.max_auto_turns,
                session_id=session_id,
                last_message=last_message,
            )
            emit(f"[AUTO] queued follow-up {turn_index + 1}/{args.max_auto_turns}")

    except RunError as exc:
        append_transcript_line(transcript_path, f"[ERROR] {exc}")
        emit(f"[FATAL] {exc}")
        return 1
    finally:
        append_transcript_line(transcript_path, "===== wrapper end =====")

    emit(f"[DONE] session_id={session_id}")
    return 0


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.command == "sessions":
        return run_sessions_command(args)
    return run_loop(args)
