from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class RunArtifacts:
    output_dir: Path | None
    transcript_path: Path | None
    summary_path: Path | None
    session_id_path: Path | None
    turns_dir: Path | None

    def turn_last_message_path(self, turn_index: int) -> Path | None:
        if self.turns_dir is None:
            return None
        return self.turns_dir / f"turn-{turn_index:02d}-last-message.txt"


def prepare_run_artifacts(output_dir: Path | None, transcript_override: Path | None) -> RunArtifacts:
    normalized_output_dir = output_dir.expanduser().resolve() if output_dir else None
    if normalized_output_dir is not None:
        normalized_output_dir.mkdir(parents=True, exist_ok=True)
        turns_dir = normalized_output_dir / "turns"
        turns_dir.mkdir(parents=True, exist_ok=True)
        summary_path = normalized_output_dir / "summary.json"
        session_id_path = normalized_output_dir / "last-session-id.txt"
        transcript_path = transcript_override.expanduser().resolve() if transcript_override else normalized_output_dir / "transcript.log"
        return RunArtifacts(
            output_dir=normalized_output_dir,
            transcript_path=transcript_path,
            summary_path=summary_path,
            session_id_path=session_id_path,
            turns_dir=turns_dir,
        )

    transcript_path = transcript_override.expanduser().resolve() if transcript_override else None
    return RunArtifacts(
        output_dir=None,
        transcript_path=transcript_path,
        summary_path=None,
        session_id_path=None,
        turns_dir=None,
    )


def write_summary(path: Path | None, payload: dict) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def write_last_session_id(path: Path | None, session_id: str) -> None:
    if path is None or not session_id:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(session_id + "\n", encoding="utf-8")
