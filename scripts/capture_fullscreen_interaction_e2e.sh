#!/usr/bin/env bash
set -euo pipefail

umask 077

usage() {
  cat <<'EOF'
Usage:
  bash scripts/capture_fullscreen_interaction_e2e.sh --output-dir <path>

Builds the clean candidate, runs the production fullscreen TUI against a
deterministic synthetic Codex app-server in tmux, injects a high-rate SGR mouse
gesture, and records sanitized selection/copy/input-latency evidence.
EOF
}

output_dir=""
while (($# > 0)); do
  case "$1" in
    --output-dir)
      [[ -n "${2-}" ]] || { printf 'missing value for --output-dir\n' >&2; exit 2; }
      output_dir="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

[[ -n "$output_dir" ]] || { printf '%s\n' '--output-dir is required' >&2; exit 2; }
[[ ! -e "$output_dir" && ! -L "$output_dir" ]] || {
  printf 'refusing to overwrite existing output: %s\n' "$output_dir" >&2
  exit 1
}

for command in base64 cargo git grep node sed sha256sum tmux wc; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "$command" >&2
    exit 1
  }
done

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
candidate_commit="$(git rev-parse HEAD)"
candidate_tree="$(git rev-parse HEAD^{tree})"

assert_clean_candidate() {
  local phase="$1"
  local current_commit
  local current_tree
  local status
  current_commit="$(git rev-parse HEAD)"
  current_tree="$(git rev-parse HEAD^{tree})"
  status="$(git status --porcelain=v1 --untracked-files=all)"
  if [[ "$current_commit" != "$candidate_commit" || "$current_tree" != "$candidate_tree" || -n "$status" ]]; then
    printf 'capture candidate changed during %s\n' "$phase" >&2
    exit 1
  fi
}

assert_clean_candidate before-build
source "$HOME/.cargo/env"
capture_target_dir="${AKRA_CAPTURE_TARGET_DIR:-$HOME/.cache/akra-fullscreen-interaction-target}"
CARGO_TARGET_DIR="$capture_target_dir" cargo build --locked --bin akra >/dev/null
binary="$capture_target_dir/debug/akra"
[[ -x "$binary" ]] || { printf 'built Akra binary is unavailable\n' >&2; exit 1; }
assert_clean_candidate after-build

raw_root="$(mktemp -d "$HOME/.akra-fullscreen-interaction.XXXXXX")"
fake_bin_dir="$raw_root/bin"
workspace="$raw_root/workspace"
isolated_home="$raw_root/home"
akra_home="$raw_root/akra-home"
codex_home="$raw_root/codex-home"
mkdir -m 700 "$fake_bin_dir" "$workspace" "$isolated_home" "$akra_home" "$codex_home"
fake_codex="$fake_bin_dir/codex"
launch_gate="$raw_root/launch"
raw_pty="$raw_root/pty.raw"
pipe_ready="$raw_root/pty.ready"
pipe_done="$raw_root/pty.done"
tmux_config="$raw_root/tmux.conf"
socket_name="akra-fullscreen-interaction-$$-$RANDOM"
session_name="interaction"
pane_target=""

cleanup() {
  tmux -L "$socket_name" kill-server >/dev/null 2>&1 || true
  if [[ "${AKRA_CAPTURE_KEEP_RAW:-0}" == 1 ]]; then
    printf 'retained raw interaction capture: %s\n' "$raw_root" >&2
    return
  fi
  for ((cleanup_attempt = 1; cleanup_attempt <= 10; cleanup_attempt += 1)); do
    rm -rf "$raw_root" 2>/dev/null || true
    [[ ! -e "$raw_root" ]] && break
    sleep 0.05
  done
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

cat >"$fake_codex" <<'NODE'
#!/usr/bin/node
'use strict';

const readline = require('readline');

if (process.argv.includes('--version')) {
  process.stdout.write('codex-cli 0.144.1-synthetic\n');
  process.exit(0);
}

let activeTurn = null;
let streamTimer = null;

function send(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

function threadRecord(id, cwd) {
  return {
    id,
    name: 'Fullscreen interaction capture',
    preview: '',
    cwd,
    source: 'vscode',
    modelProvider: 'openai',
    updatedAt: 1785790000,
    path: null,
    status: { type: 'idle' },
    gitInfo: null,
    turns: [],
  };
}

function finishTurn() {
  if (!activeTurn) return;
  const { threadId, turnId, itemId, text } = activeTurn;
  send({
    method: 'item/completed',
    params: {
      threadId,
      turnId,
      completedAtMs: 1785790004000,
      item: { id: itemId, type: 'agentMessage', phase: 'final_answer', text },
    },
  });
  send({
    method: 'turn/completed',
    params: {
      threadId,
      turn: {
        id: turnId,
        items: [],
        status: 'completed',
        error: null,
        itemsView: 'notLoaded',
        startedAt: 1785790000,
        completedAt: 1785790004,
        durationMs: 4000,
      },
    },
  });
  activeTurn = null;
}

function startStreaming(threadId, turnId) {
  const itemId = 'agent-interaction';
  const chunks = [
    'Response surface ready.\n',
    'The transcript owns stable rendered cells.\n',
    'Pointer reports are collected independently.\n',
    'Prompt cards remain application chrome.\n',
    'Assistant output remains selectable.\n',
    'Fast drag samples collapse before repaint.\n',
    'Wide glyph mapping remains stable.\n',
    'Resize receipts remain guarded.\n',
    'Clipboard delivery uses OSC 52.\n',
    'SELECTABLE_RESPONSE_CANARY\n',
    'The stream continues after the selection proof.\n',
    'Final frame delivery remains coherent.\n',
  ];
  activeTurn = { threadId, turnId, itemId, text: '' };
  send({
    method: 'item/started',
    params: {
      threadId,
      turnId,
      startedAtMs: 1785790001000,
      item: { id: itemId, type: 'agentMessage', phase: 'final_answer', text: '' },
    },
  });
  let index = 0;
  const emitNext = () => {
    if (!activeTurn) return;
    if (index >= chunks.length) {
      finishTurn();
      return;
    }
    const delta = chunks[index++];
    activeTurn.text += delta;
    send({
      method: 'item/agentMessage/delta',
      params: { threadId, turnId, itemId, phase: 'final_answer', delta },
    });
    const delay = delta.includes('SELECTABLE_RESPONSE_CANARY') ? 1800 : 35;
    streamTimer = setTimeout(emitNext, delay);
  };
  emitNext();
}

readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const request = JSON.parse(line);
  if (!Object.hasOwn(request, 'id')) return;
  const { id, method } = request;
  const params = request.params || {};

  if (method === 'initialize') {
    send({ id, result: { userAgent: 'codex-app-server/synthetic-interaction', platformFamily: 'unix', platformOs: 'linux-x64' } });
  } else if (method === 'account/read') {
    send({ id, result: { account: null, requiresOpenAIAuth: false } });
  } else if (method === 'thread/list') {
    send({ id, result: { data: [], nextCursor: null } });
  } else if (method === 'thread/read') {
    send({ id, result: { thread: threadRecord(params.threadId || 'interaction-thread', params.cwd || '/capture') } });
  } else if (method === 'thread/start') {
    const cwd = params.cwd || '/capture';
    send({
      id,
      result: {
        thread: threadRecord('interaction-thread', cwd),
        approvalPolicy: params.approvalPolicy || 'on-request',
        approvalsReviewer: params.approvalsReviewer || 'user',
        cwd,
        model: 'gpt-5.6-synthetic',
        modelProvider: 'openai',
        reasoningEffort: 'medium',
        sandbox: { type: 'workspaceWrite' },
        serviceTier: null,
      },
    });
  } else if (method === 'turn/start') {
    const threadId = params.threadId || 'interaction-thread';
    const turnId = `interaction-turn-${id}`;
    send({ id, result: { turn: { id: turnId } } });
    startStreaming(threadId, turnId);
  } else if (method === 'turn/interrupt') {
    if (streamTimer) clearTimeout(streamTimer);
    activeTurn = null;
    send({ id, result: {} });
  } else if (method === 'thread/archive') {
    send({ id, result: {} });
  } else {
    send({ id, error: { message: `unsupported synthetic method: ${method}` } });
  }
});
NODE
chmod 700 "$fake_codex"

git -C "$workspace" init -q
git -C "$workspace" config user.name 'Akra Capture'
git -C "$workspace" config user.email 'capture@example.invalid'

cat >"$tmux_config" <<'TMUX'
set-option -g status off
set-option -g default-terminal tmux-256color
set-option -g default-shell /bin/bash
set-option -g history-limit 2000
set-option -g remain-on-exit on
TMUX
chmod 600 "$tmux_config"

capture_path="$fake_bin_dir:/usr/bin:/bin"
launch_command="$(printf \
  'while test ! -e %q; do sleep 0.01; done; exec /usr/bin/env -i HOME=%q USERPROFILE=%q USER=akra-capture LOGNAME=akra-capture SHELL=/bin/bash AKRA_HOME=%q CODEX_HOME=%q PATH=%q LANG=C.UTF-8 LC_ALL=C.UTF-8 TERM=tmux-256color AKRA_APP_SERVER_PROMPT_LOG=0 CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART=0 AKRA_TUI_MOUSE_CAPTURE=on %q' \
  "$launch_gate" "$isolated_home" "$isolated_home" "$akra_home" "$codex_home" \
  "$capture_path" "$binary")"
pane_target="$(tmux -L "$socket_name" -f "$tmux_config" new-session -d -P -F '#{pane_id}' \
  -s "$session_name" -x 120 -y 30 -c "$workspace" "$launch_command")"
raw_pipe_command="$(printf ': > %q; cat > %q; : > %q' "$pipe_ready" "$raw_pty" "$pipe_done")"
: >"$raw_pty"
chmod 600 "$raw_pty"
tmux -L "$socket_name" pipe-pane -o -t "$pane_target" "$raw_pipe_command"

capture_plain() {
  tmux -L "$socket_name" capture-pane -p -N -S 0 -E - -t "$pane_target"
}

wait_for_text() {
  local needle="$1"
  local attempts="$2"
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    if capture_plain | grep -Fq -- "$needle"; then
      return 0
    fi
    sleep 0.05
  done
  printf 'timed out waiting for pane text: %s\n' "$needle" >&2
  capture_plain >&2 || true
  return 1
}

wait_for_file() {
  local path="$1"
  local attempts="$2"
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    [[ -e "$path" ]] && return 0
    sleep 0.05
  done
  printf 'timed out waiting for file: %s\n' "$path" >&2
  return 1
}

pane_dead() {
  tmux -L "$socket_name" display-message -p -t "$pane_target" '#{pane_dead}'
}

wait_for_file "$pipe_ready" 200
: >"$launch_gate"
wait_for_text 'Describe a task or type' 400
tmux -L "$socket_name" send-keys -t "$pane_target" -l 'PROMPT_CHROME_CANARY'
tmux -L "$socket_name" send-keys -t "$pane_target" Enter
wait_for_text 'SELECTABLE_RESPONSE_CANARY' 400

capture_plain >"$raw_root/before-selection.txt"
target_line="$(grep -n -m1 'SELECTABLE_RESPONSE_CANARY' "$raw_root/before-selection.txt")"
target_row_one_based="${target_line%%:*}"
target_text="${target_line#*:}"
target_prefix="${target_text%%SELECTABLE_RESPONSE_CANARY*}"
target_column_one_based="$(( ${#target_prefix} + 1 ))"
target_literal='SELECTABLE_RESPONSE_CANARY'
target_length="${#target_literal}"
target_end_column="$((target_column_one_based + target_length - 1))"

drag_payload="$(printf '\033[<0;%s;%sM' "$target_column_one_based" "$target_row_one_based")"
for ((sample = 0; sample < 1200; sample += 1)); do
  sample_column="$((target_column_one_based + (sample % target_length)))"
  drag_payload+="$(printf '\033[<32;%s;%sM' "$sample_column" "$target_row_one_based")"
done
drag_payload+="$(printf '\033[<32;%s;%sM' "$target_end_column" "$target_row_one_based")"
drag_payload+="$(printf '\033[<0;%s;%sm' "$target_end_column" "$target_row_one_based")"
drag_started_ms="$(date +%s%3N)"
tmux -L "$socket_name" send-keys -t "$pane_target" -l -- "$drag_payload"
wait_for_text 'copied selection to terminal clipboard' 100
drag_completed_ms="$(date +%s%3N)"
drag_latency_ms="$((drag_completed_ms - drag_started_ms))"
((drag_latency_ms < 1500)) || {
  printf 'fast drag settled too slowly: %sms\n' "$drag_latency_ms" >&2
  exit 1
}

tmux -L "$socket_name" capture-pane -p -e -N -S 0 -E - -t "$pane_target" \
  >"$raw_root/selection-frame.ansi"
capture_plain >"$raw_root/selection-frame.txt"
selection_sgr="$(printf '\033[48;2;42;72;112m')"
grep -aFq -- "$selection_sgr" "$raw_root/selection-frame.ansi" || {
  printf 'selection frame did not retain the expected RGB background\n' >&2
  exit 1
}

clipboard_payload="$(printf '%s' 'SELECTABLE_RESPONSE_CANARY' | base64 -w0)"
grep -aFq -- "]52;c;$clipboard_payload" "$raw_pty" || {
  printf 'raw PTY did not contain the expected OSC 52 selection payload\n' >&2
  exit 1
}

tmux -L "$socket_name" send-keys -t "$pane_target" C-c
sleep 0.1
[[ "$(pane_dead)" == 0 ]] || {
  printf 'Ctrl+C terminated the session while a transcript selection was active\n' >&2
  exit 1
}

wait_for_text 'Final frame delivery remains coherent.' 400
sleep 0.2
move_payload=""
for ((sample = 0; sample < 3000; sample += 1)); do
  move_payload+="$(printf '\033[<35;%s;%sM' "$((20 + sample % 40))" "$((5 + sample % 10))")"
done
move_started_ms="$(date +%s%3N)"
tmux -L "$socket_name" send-keys -t "$pane_target" -l -- "$move_payload"
tmux -L "$socket_name" send-keys -t "$pane_target" -l 'LAG_INPUT_CANARY'
wait_for_text 'LAG_INPUT_CANARY' 100
move_completed_ms="$(date +%s%3N)"
move_latency_ms="$((move_completed_ms - move_started_ms))"
((move_latency_ms < 1500)) || {
  printf 'real input remained behind unused mouse motion: %sms\n' "$move_latency_ms" >&2
  exit 1
}

tmux -L "$socket_name" pipe-pane -t "$pane_target"
wait_for_file "$pipe_done" 200
[[ -s "$raw_pty" ]] || { printf 'raw PTY capture is empty\n' >&2; exit 1; }
tmux -L "$socket_name" send-keys -t "$pane_target" C-q
for ((attempt = 1; attempt <= 200; attempt += 1)); do
  [[ "$(pane_dead)" == 1 ]] && break
  sleep 0.05
done
[[ "$(pane_dead)" == 1 ]] || { printf 'Akra did not exit after Ctrl+Q\n' >&2; exit 1; }
runtime_exit_status="$(tmux -L "$socket_name" display-message -p -t "$pane_target" '#{pane_dead_status}')"
[[ "$runtime_exit_status" == 0 ]] || {
  printf 'Akra exited with status %s\n' "$runtime_exit_status" >&2
  exit 1
}
assert_clean_candidate after-capture

mkdir -p "$output_dir/frames"
sed \
  -e "s|$raw_root|<isolated>|g" \
  -e "s|$repo_root|<repo>|g" \
  "$raw_root/selection-frame.txt" >"$output_dir/frames/selection-copy-120x30.txt"

cat >"$output_dir/environment-stamp.txt" <<EOF
candidate_commit: $candidate_commit
candidate_tree: $candidate_tree
binary_sha256: $(sha256sum "$binary" | cut -d' ' -f1)
synthetic_codex_sha256: $(sha256sum "$fake_codex" | cut -d' ' -f1)
captured_at_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)
environment: $(. /etc/os-release; printf '%s' "$PRETTY_NAME") / $(uname -srmo)
terminal: $(tmux -V) detached PTY
geometry: 120x30
frontend: fullscreen alternate-screen
candidate_clean_checks: before-build, after-build, after-capture
EOF

cat >"$output_dir/scenario-results.txt" <<EOF
result: pass
synthetic_app_server: true
source_build: true
rapid_drag_samples: 1200
rapid_drag_settle_ms: $drag_latency_ms
unused_motion_samples: 3000
motion_then_key_visible_ms: $move_latency_ms
selection_text: SELECTABLE_RESPONSE_CANARY
selection_background: RGB(42,72,112)
osc52_payload_verified: true
ctrl_c_preserved_session: true
clean_exit_status: $runtime_exit_status
raw_pty_retained: false
EOF

cat >"$output_dir/README.md" <<'EOF'
# Fullscreen interaction E2E evidence

This candidate-specific E3 capture uses the production `akra` binary in a real detached tmux PTY
with a deterministic owner-only synthetic Codex app-server. It proves that a 1,200-sample SGR drag
settles to the final pointer position, retains the semantic selection background, emits the exact
OSC 52 clipboard payload, and leaves Ctrl+C available as copy instead of session termination.

The same run injects 3,000 unused all-motion reports before ordinary keyboard input. The input is
visible within the recorded bound because composition drops hover motion that the TUI does not use.
The submitted prompt and composer remain application chrome; response text is the selectable
surface. Raw PTY bytes and isolated paths are intentionally not retained.
EOF

printf 'fullscreen interaction evidence written: %s\n' "$output_dir"
