#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLI="$ROOT/.venv/bin/codex-exec-loop"

if [[ ! -x "$CLI" ]]; then
  echo "codex-exec-loop executable not found: $CLI" >&2
  echo "Run from the repo after creating .venv and installing editable package." >&2
  exit 1
fi

mkdir -p "$ROOT/artifacts" "$ROOT/logs"

STAMP="$(date +%s)"
TARGET_REL="artifacts/SMOKE_WORK_PRODUCT_${STAMP}.md"
TARGET_ABS="$ROOT/$TARGET_REL"
RUN_DIR="$ROOT/logs/artifact-smoke-${STAMP}"
SUMMARY_PATH="$RUN_DIR/summary.json"
TRANSCRIPT_PATH="$RUN_DIR/transcript.log"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

INITIAL_PROMPT="$TMP_DIR/initial.txt"
FOLLOWUP_PROMPT="$TMP_DIR/followup.txt"

cat >"$INITIAL_PROMPT" <<EOF
작업 디렉터리는 현재 저장소 루트입니다.

$TARGET_REL 파일을 새로 만들고 아래 내용을 정확히 써주세요.

# Codex Exec Loop Smoke Test

- turn-1

그 다음 무엇을 만들었는지 한 문단으로 짧게 요약해주세요.
EOF

cat >"$FOLLOWUP_PROMPT" <<EOF
대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

기존 $TARGET_REL 파일 맨 아래에 아래 한 줄만 추가해주세요.

- followup-{auto_turn}

기존 내용은 유지하고, 그 다음 무엇을 추가했는지 한 문단으로 짧게 요약해주세요.
EOF

echo "[INFO] target file: $TARGET_ABS"
echo "[INFO] run dir:      $RUN_DIR"

"$CLI" \
  --yes \
  --mode new \
  --cwd "$ROOT" \
  --prompt-file "$INITIAL_PROMPT" \
  --followup-file "$FOLLOWUP_PROMPT" \
  --max-auto-turns 1 \
  --full-auto \
  --output-dir "$RUN_DIR"

"$CLI" verify \
  --summary "$SUMMARY_PATH" \
  --must-exist "$TARGET_ABS" \
  --must-contain "$TARGET_ABS::- turn-1" \
  --must-contain "$TARGET_ABS::- followup-1" \
  --expect-changed "$TARGET_ABS"

echo
echo "[RESULT] created file:"
echo "$TARGET_ABS"
echo
sed -n '1,120p' "$TARGET_ABS"
echo
echo "[RESULT] summary:"
echo "$SUMMARY_PATH"
echo
echo "[RESULT] transcript:"
echo "$TRANSCRIPT_PATH"
