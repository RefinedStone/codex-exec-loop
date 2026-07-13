#!/usr/bin/env bash
set -euo pipefail

umask 077

usage() {
  cat <<'EOF'
Usage:
  bash scripts/capture_typed_activity_rail_e2e.sh --output <path>

Builds the current clean source tree, drives Akra through a deterministic synthetic
Codex app-server in a detached tmux PTY, and writes a sanitized JSON evidence record.
EOF
}

output_path=""
while (($# > 0)); do
  case "$1" in
    --output)
      [[ -n "${2-}" ]] || { printf 'missing value for --output\n' >&2; exit 2; }
      output_path="$2"
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

[[ -n "$output_path" ]] || { printf '--output is required\n' >&2; exit 2; }

for command in cargo git grep ln node sha256sum tmux; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "$command" >&2
    exit 1
  }
done

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
if [[ -e "$output_path" || -L "$output_path" ]]; then
  printf 'refusing to overwrite existing output: %s\n' "$output_path" >&2
  exit 1
fi

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
cargo build --locked --bin akra >/dev/null
binary="$repo_root/target/debug/akra"
[[ -x "$binary" ]] || { printf 'built Akra binary is unavailable\n' >&2; exit 1; }
assert_clean_candidate after-build

raw_root="$(mktemp -d "$HOME/.akra-typed-rail-e2e.XXXXXX")"
chmod 700 "$raw_root"
fake_bin_dir="$raw_root/bin"
workspace="$raw_root/workspace"
isolated_home="$raw_root/home"
akra_home="$raw_root/akra-home"
codex_home="$raw_root/codex-home"
mkdir -m 700 "$fake_bin_dir" "$workspace" "$isolated_home" "$akra_home" "$codex_home"
fake_codex="$fake_bin_dir/codex"
release_file="$fake_bin_dir/release"
launch_gate="$raw_root/launch"
raw_pty="$raw_root/pty.raw"
raw_pipe_ready="$raw_root/pty.ready"
raw_pipe_done="$raw_root/pty.done"
tmux_config="$raw_root/tmux.conf"
socket_name="akra-typed-rail-$$-$RANDOM"
session_name="rail"
staging_path=""

cleanup() {
  tmux -L "$socket_name" kill-server >/dev/null 2>&1 || true
  [[ -z "$staging_path" ]] || rm -f "$staging_path"
  rm -rf "$raw_root"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

cat >"$fake_codex" <<'NODE'
#!/usr/bin/env node
'use strict';

const fs = require('fs');
const path = require('path');
const readline = require('readline');

if (process.argv.includes('--version')) {
  process.stdout.write('codex-cli 0.144.1-synthetic\n');
  process.exit(0);
}

const releaseFile = path.join(path.dirname(process.argv[1]), 'release');
let completionSent = false;
let activeTurn = null;

function send(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

function threadRecord(id, cwd) {
  return {
    id,
    name: 'Typed rail capture',
    preview: '',
    cwd,
    source: 'vscode',
    modelProvider: 'openai',
    updatedAt: 1783960000,
    path: null,
    status: { type: 'idle' },
    gitInfo: null,
    turns: [],
  };
}

function completeTurn() {
  if (completionSent || !activeTurn || !fs.existsSync(releaseFile)) return;
  completionSent = true;
  const { threadId, turnId } = activeTurn;
  send({
    method: 'item/completed',
    params: {
      threadId,
      turnId,
      completedAtMs: 1783960002000,
      item: {
        id: 'command-capture',
        type: 'commandExecution',
        command: 'synthetic typed rail capture',
        commandActions: [],
        cwd: '/capture',
        status: 'completed',
      },
    },
  });
  send({
    method: 'item/started',
    params: {
      threadId,
      turnId,
      startedAtMs: 1783960002050,
      item: {
        id: 'agent-capture',
        type: 'agentMessage',
        phase: 'final_answer',
        text: '',
      },
    },
  });
  send({
    method: 'item/completed',
    params: {
      threadId,
      turnId,
      completedAtMs: 1783960002100,
      item: {
        id: 'agent-capture',
        type: 'agentMessage',
        phase: 'final_answer',
        text: 'D4_COMMITTED_OUTPUT_CANARY',
      },
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
        startedAt: 1783960000,
        completedAt: 1783960002,
        durationMs: 2000,
      },
    },
  });
}

setInterval(completeTurn, 25).unref();

readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const request = JSON.parse(line);
  if (!Object.hasOwn(request, 'id')) return;
  const id = request.id;
  const method = request.method;
  const params = request.params || {};

  if (method === 'initialize') {
    send({ id, result: { userAgent: 'codex-app-server/synthetic-capture', platformFamily: 'unix', platformOs: 'linux-x64' } });
  } else if (method === 'account/read') {
    send({ id, result: { account: null, requiresOpenAIAuth: false } });
  } else if (method === 'thread/list') {
    send({ id, result: { data: [], nextCursor: null } });
  } else if (method === 'thread/read') {
    send({ id, result: { thread: threadRecord(params.threadId || 'capture-thread', params.cwd || '/capture') } });
  } else if (method === 'thread/start') {
    const cwd = params.cwd || '/capture';
    send({
      id,
      result: {
        thread: threadRecord('capture-thread', cwd),
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
    const threadId = params.threadId || 'capture-thread';
    const turnId = `capture-turn-${id}`;
    activeTurn = { threadId, turnId };
    send({ id, result: { turn: { id: turnId } } });
    send({
      method: 'item/started',
      params: {
        threadId,
        turnId,
        startedAtMs: 1783960001000,
        item: {
          id: 'command-capture',
          type: 'commandExecution',
          command: 'synthetic typed rail capture',
          commandActions: [],
          cwd: '/capture',
          status: 'inProgress',
        },
      },
    });
    send({
      method: 'item/commandExecution/outputDelta',
      params: {
        threadId,
        turnId,
        itemId: 'command-capture',
        delta: '\u001b[31mD4_RAW_ACTIVITY_SECRET\u001b[0m\n',
      },
    });
  } else if (method === 'turn/interrupt' || method === 'thread/archive') {
    send({ id, result: {} });
  } else {
    send({ id, error: { message: `unsupported synthetic capture method: ${method}` } });
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

launch_command="$(printf \
  'while test ! -e %q; do sleep 0.01; done; exec env -u WT_SESSION -u AKRA_TRACE -u RUST_LOG -u AKRA_TRACE_SPANS -u AKRA_TRACE_FILE -u AKRA_TOKIO_CONSOLE HOME=%q USERPROFILE=%q AKRA_HOME=%q CODEX_HOME=%q PATH=%q TERM=tmux-256color AKRA_APP_SERVER_PROMPT_LOG=0 AKRA_CAPTURE_SYNTHETIC=1 CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART=0 CODEX_EXEC_LOOP_INLINE_HISTORY_MODE=scrollback CODEX_EXEC_LOOP_HISTORY_INSERT_MODE=standard %q' \
  "$launch_gate" "$isolated_home" "$isolated_home" "$akra_home" "$codex_home" \
  "$fake_bin_dir:$PATH" "$binary")"
pane_target="$(tmux -L "$socket_name" -f "$tmux_config" new-session -d -P -F '#{pane_id}' \
  -s "$session_name" -x 160 -y 24 -c "$workspace" "$launch_command")"
raw_pipe_command="$(printf ': > %q; cat > %q; : > %q' \
  "$raw_pipe_ready" "$raw_pty" "$raw_pipe_done")"
: >"$raw_pty"
chmod 600 "$raw_pty"
tmux -L "$socket_name" pipe-pane -o -t "$pane_target" "$raw_pipe_command"

capture_current() {
  tmux -L "$socket_name" capture-pane -p -N -S 0 -E - -t "$pane_target"
}

wait_for_current() {
  local needle="$1"
  local attempts="$2"
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    local frame
    frame="$(capture_current)"
    if [[ "$frame" == *"$needle"* ]]; then
      return 0
    fi
    sleep 0.1
  done
  printf 'timed out waiting for current pane predicate: %s\n' "$needle" >&2
  return 1
}

wait_for_full() {
  local needle="$1"
  local attempts="$2"
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    local frame
    frame="$(tmux -L "$socket_name" capture-pane -p -N -S - -E - -t "$pane_target")"
    if [[ "$frame" == *"$needle"* ]]; then
      return 0
    fi
    sleep 0.1
  done
  printf 'timed out waiting for full pane predicate: %s\n' "$needle" >&2
  return 1
}

wait_for_geometry() {
  local expected="$1"
  local attempts="$2"
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    local actual
    actual="$(tmux -L "$socket_name" display-message -p -t "$pane_target" '#{pane_width}x#{pane_height}')"
    [[ "$actual" == "$expected" ]] && return 0
    sleep 0.05
  done
  printf 'timed out waiting for pane geometry: %s\n' "$expected" >&2
  return 1
}

wait_for_file() {
  local path="$1"
  local attempts="$2"
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    [[ -e "$path" ]] && return 0
    sleep 0.05
  done
  printf 'timed out waiting for capture flush\n' >&2
  return 1
}

capture_checkpoint() {
  local name="$1"
  for ((checkpoint_attempt = 1; checkpoint_attempt <= 10; checkpoint_attempt += 1)); do
    local before
    local after
    local history_size
    before="$(tmux -L "$socket_name" display-message -p -t "$pane_target" \
      'geometry=#{pane_width}x#{pane_height} cursor=#{cursor_x},#{cursor_y} history=#{history_size} dead=#{pane_dead}')"
    [[ "$before" == *' dead=0' ]] || {
      printf 'pane exited before checkpoint %s\n' "$name" >&2
      exit 1
    }
    history_size="$(sed -E 's/.*history=([0-9]+).*/\1/' <<<"$before")"
    capture_current >"$raw_root/$name.current"
    if ((history_size > 0)); then
      tmux -L "$socket_name" capture-pane -p -N -S - -E -1 -t "$pane_target" \
        >"$raw_root/$name.history"
    else
      : >"$raw_root/$name.history"
    fi
    tmux -L "$socket_name" capture-pane -p -N -S - -E - -t "$pane_target" \
      >"$raw_root/$name.full"
    after="$(tmux -L "$socket_name" display-message -p -t "$pane_target" \
      'geometry=#{pane_width}x#{pane_height} cursor=#{cursor_x},#{cursor_y} history=#{history_size} dead=#{pane_dead}')"
    if [[ "$before" == "$after" ]]; then
      printf '%s\n' "${before% dead=*}" >"$raw_root/$name.meta"
      return 0
    fi
    sleep 0.05
  done
  printf 'pane geometry did not stabilize for checkpoint %s\n' "$name" >&2
  return 1
}

count_fixed() {
  local needle="$1"
  local file="$2"
  local matches
  matches="$(grep -aFo -- "$needle" "$file" 2>/dev/null || true)"
  if [[ -z "$matches" ]]; then
    printf '0'
  else
    printf '%s\n' "$matches" | wc -l | tr -d ' '
  fi
}

assert_count() {
  local expected="$1"
  local needle="$2"
  local file="$3"
  local actual
  actual="$(count_fixed "$needle" "$file")"
  [[ "$actual" == "$expected" ]] || {
    printf 'expected %s occurrences of %q in %s, found %s\n' \
      "$expected" "$needle" "$file" "$actual" >&2
    exit 1
  }
}

assert_checkpoint_geometry() {
  local name="$1"
  local width="$2"
  local height="$3"
  local meta
  local cursor_y
  meta="$(<"$raw_root/$name.meta")"
  [[ "$meta" == geometry="$width"x"$height"* ]] || {
    printf 'unexpected geometry for %s: %s\n' "$name" "$meta" >&2
    exit 1
  }
  cursor_y="$(sed -E 's/.*cursor=[0-9]+,([0-9]+).*/\1/' <<<"$meta")"
  ((cursor_y < height)) || {
    printf 'cursor is outside checkpoint %s: %s\n' "$name" "$meta" >&2
    exit 1
  }
}

assert_startup_checkpoint() {
  local name="$1"
  assert_checkpoint_geometry "$name" 160 24
  assert_count 1 'prompt: new thread ready' "$raw_root/$name.current"
  assert_count 0 'notice: activity:' "$raw_root/$name.full"
  assert_count 0 'D4_RAW_ACTIVITY_SECRET' "$raw_root/$name.full"
}

assert_active_checkpoint() {
  local name="$1"
  local width="$2"
  local height="$3"
  local model_count="$4"
  assert_checkpoint_geometry "$name" "$width" "$height"
  assert_count 1 'notice: activity: cmd:1 lines | active:command' "$raw_root/$name.current"
  assert_count 1 'notice: activity:' "$raw_root/$name.current"
  assert_count 1 'active:command' "$raw_root/$name.current"
  assert_count 1 'Working (' "$raw_root/$name.current"
  assert_count 1 'prompt: turn running' "$raw_root/$name.current"
  assert_count "$model_count" 'model:gpt-5.6-synthetic' "$raw_root/$name.current"
  assert_count 0 'notice: activity:' "$raw_root/$name.history"
  assert_count 0 'active:command' "$raw_root/$name.history"
  assert_count 0 'prompt: turn running' "$raw_root/$name.history"
  assert_count 0 'D4_RAW_ACTIVITY_SECRET' "$raw_root/$name.full"
}

normalize_frame() {
  sed -E 's/Working \(([0-9]+h [0-9]+m|[0-9]+m [0-9]+s|[0-9]+s)/Working (<elapsed>/' "$1"
}

wait_for_file "$raw_pipe_ready" 200
: >"$launch_gate"
wait_for_geometry 160x24 100
wait_for_current 'prompt: new thread ready' 200
capture_checkpoint startup
assert_startup_checkpoint startup

tmux -L "$socket_name" send-keys -t "$pane_target" -l 'D4_CAPTURE_START'
tmux -L "$socket_name" send-keys -t "$pane_target" Enter
wait_for_current 'notice: activity: cmd:1 lines | active:command' 200
wait_for_current 'model:gpt-5.6-synthetic' 200
capture_checkpoint active_wide
assert_active_checkpoint active_wide 160 24 1

tmux -L "$socket_name" resize-window -t "$session_name:0" -x 48 -y 18
wait_for_geometry 48x18 100
sleep 0.15
wait_for_current 'notice: activity: cmd:1 lines | active:command' 100
capture_checkpoint active_narrow
assert_active_checkpoint active_narrow 48 18 0

tmux -L "$socket_name" resize-window -t "$session_name:0" -x 49 -y 18
wait_for_geometry 49x18 100
wait_for_current 'notice: activity: cmd:1 lines | active:command' 100
tmux -L "$socket_name" resize-window -t "$session_name:0" -x 48 -y 18
wait_for_geometry 48x18 100
sleep 0.15
wait_for_current 'notice: activity: cmd:1 lines | active:command' 100
capture_checkpoint active_narrow_repeat
assert_active_checkpoint active_narrow_repeat 48 18 0
normalize_frame "$raw_root/active_narrow.current" >"$raw_root/active_narrow.normalized"
normalize_frame "$raw_root/active_narrow_repeat.current" \
  >"$raw_root/active_narrow_repeat.normalized"
cmp -s "$raw_root/active_narrow.normalized" "$raw_root/active_narrow_repeat.normalized" || {
  printf 'repeated narrow resize did not restore an idempotent active frame\n' >&2
  exit 1
}
[[ "$(<"$raw_root/active_narrow.history")" == "$(<"$raw_root/active_narrow_repeat.history")" ]] || {
  printf 'repeated narrow resize changed host history\n' >&2
  exit 1
}

tmux -L "$socket_name" resize-window -t "$session_name:0" -x 160 -y 24
wait_for_geometry 160x24 100
sleep 0.15
wait_for_current 'model:gpt-5.6-synthetic' 100
capture_checkpoint active_restored
assert_active_checkpoint active_restored 160 24 1

: >"$release_file"
wait_for_current 'turn: idle' 200
wait_for_full 'D4_COMMITTED_OUTPUT_CANARY' 200
capture_checkpoint completed
assert_count 0 'notice: activity:' "$raw_root/completed.current"
assert_count 0 'notice: activity:' "$raw_root/completed.history"
assert_count 0 'active:command' "$raw_root/completed.current"
assert_count 0 'active:command' "$raw_root/completed.history"
assert_count 0 'prompt: turn running' "$raw_root/completed.history"
assert_count 0 'D4_RAW_ACTIVITY_SECRET' "$raw_root/completed.full"
assert_count 1 'D4_COMMITTED_OUTPUT_CANARY' "$raw_root/completed.full"
assert_count 1 'prompt: session ready' "$raw_root/completed.current"

tmux -L "$socket_name" pipe-pane -t "$pane_target"
wait_for_file "$raw_pipe_done" 200
[[ -s "$raw_pty" ]] || {
  printf 'raw PTY capture is empty\n' >&2
  exit 1
}
raw_committed_canary_count="$(count_fixed 'D4_COMMITTED_OUTPUT_CANARY' "$raw_pty")"
((raw_committed_canary_count > 0)) || {
  printf 'raw PTY capture did not reach the completed checkpoint\n' >&2
  exit 1
}
assert_count 0 'D4_RAW_ACTIVITY_SECRET' "$raw_pty"
raw_ansi_payload="$(printf '\033[31mD4_RAW_ACTIVITY_SECRET\033[0m')"
assert_count 0 "$raw_ansi_payload" "$raw_pty"

tmux -L "$socket_name" send-keys -t "$pane_target" C-q
for ((attempt = 1; attempt <= 200; attempt += 1)); do
  [[ "$(tmux -L "$socket_name" display-message -p -t "$pane_target" '#{pane_dead}')" == 1 ]] && break
  sleep 0.05
done
[[ "$(tmux -L "$socket_name" display-message -p -t "$pane_target" '#{pane_dead}')" == 1 ]] || {
  printf 'Akra did not exit after Ctrl+Q\n' >&2
  exit 1
}
runtime_exit_status="$(tmux -L "$socket_name" display-message -p -t "$pane_target" '#{pane_dead_status}')"
[[ "$runtime_exit_status" == 0 ]] || {
  printf 'Akra exited with status %s\n' "$runtime_exit_status" >&2
  exit 1
}
assert_clean_candidate after-capture

export AKRA_CAPTURE_RAW_ROOT="$raw_root"
export AKRA_CAPTURE_COMMIT="$candidate_commit"
export AKRA_CAPTURE_TREE="$candidate_tree"
export AKRA_CAPTURE_BINARY_SHA="$(sha256sum "$binary" | cut -d' ' -f1)"
export AKRA_CAPTURE_FAKE_SHA="$(sha256sum "$fake_codex" | cut -d' ' -f1)"
export AKRA_CAPTURE_RAW_PTY_SHA="$(sha256sum "$raw_pty" | cut -d' ' -f1)"
export AKRA_CAPTURE_RAW_PTY_BYTES="$(wc -c <"$raw_pty" | tr -d ' ')"
export AKRA_CAPTURE_RAW_CANARY_COUNT="$raw_committed_canary_count"
export AKRA_CAPTURE_TMUX_VERSION="$(tmux -V)"
export AKRA_CAPTURE_BASH_VERSION="${BASH_VERSION}"
export AKRA_CAPTURE_NODE_VERSION="$(node --version)"
export AKRA_CAPTURE_KERNEL="$(uname -srmo)"
export AKRA_CAPTURE_OS="$(. /etc/os-release; printf '%s' "$PRETTY_NAME")"
export AKRA_CAPTURE_REPO_ROOT="$repo_root"
export AKRA_CAPTURE_SOCKET="$socket_name"
export AKRA_CAPTURE_SESSION="$session_name"
export AKRA_CAPTURE_EXIT_STATUS="$runtime_exit_status"
export AKRA_CAPTURED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
export AKRA_CAPTURE_OPERATOR="$(git config user.name || printf 'not recorded')"
export AKRA_CAPTURE_ARTIFACT_ID="$(basename "$output_path")"

mkdir -p "$(dirname "$output_path")"
staging_path="$(mktemp "$(dirname "$output_path")/.typed-activity-rail-e2e.XXXXXX")"
chmod 600 "$staging_path"
node - "$staging_path" <<'NODE'
'use strict';

const crypto = require('crypto');
const fs = require('fs');

const outputPath = process.argv[2];
const root = process.env.AKRA_CAPTURE_RAW_ROOT;
const names = ['startup', 'active_wide', 'active_narrow', 'active_narrow_repeat', 'active_restored', 'completed'];
const read = (name, suffix) => fs.readFileSync(`${root}/${name}.${suffix}`);
const text = (name, suffix) => read(name, suffix).toString('utf8');
const count = (source, needle) => source.split(needle).length - 1;
const sha256 = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');

function rejectUnsafeText(label, bytes) {
  const value = bytes.toString('utf8');
  if (!Buffer.from(value, 'utf8').equals(bytes)) throw new Error(`${label} is not valid UTF-8`);
  rejectUnsafeString(label, value, true);
}

function rejectUnsafeString(label, value, allowNewline = false) {
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if ((codePoint < 0x20 && !(allowNewline && codePoint === 0x0a)) || (codePoint >= 0x7f && codePoint <= 0x9f)) {
      throw new Error(`${label} contains a forbidden control character`);
    }
  }
}

function rejectUnsafeStrings(value, label) {
  if (typeof value === 'string') {
    rejectUnsafeString(label, value);
  } else if (Array.isArray(value)) {
    value.forEach((item, index) => rejectUnsafeStrings(item, `${label}[${index}]`));
  } else if (value && typeof value === 'object') {
    Object.entries(value).forEach(([key, item]) => rejectUnsafeStrings(item, `${label}.${key}`));
  }
}

function checkpoint(name) {
  const meta = text(name, 'meta').trim();
  const match = /^geometry=(\d+)x(\d+) cursor=(\d+),(\d+) history=(\d+)$/.exec(meta);
  if (!match) throw new Error(`invalid checkpoint metadata: ${meta}`);
  const current = text(name, 'current');
  const history = text(name, 'history');
  const full = text(name, 'full');
  for (const [label, bytes] of [['current', read(name, 'current')], ['history', read(name, 'history')], ['full', read(name, 'full')]]) {
    rejectUnsafeText(`${name}.${label}`, bytes);
  }
  const visibleSource = name === 'completed' ? full : current;
  return {
    name,
    geometry: { width: Number(match[1]), height: Number(match[2]) },
    cursor: { x: Number(match[3]), y: Number(match[4]) },
    historyRows: Number(match[5]),
    counts: {
      currentActiveCommand: count(current, 'active:command'),
      currentActivityRail: count(current, 'notice: activity:'),
      currentWorkingState: count(current, 'Working ('),
      currentRunningPrompt: count(current, 'prompt: turn running'),
      currentReadyPrompt: count(current, 'prompt: new thread ready'),
      currentModelFact: count(current, 'model:gpt-5.6-synthetic'),
      historyActivityRail: count(history, 'notice: activity:'),
      historyActiveCommand: count(history, 'active:command'),
      historyRunningPrompt: count(history, 'prompt: turn running'),
      fullRawSecret: count(full, 'D4_RAW_ACTIVITY_SECRET'),
      fullCommittedCanary: count(full, 'D4_COMMITTED_OUTPUT_CANARY'),
    },
    digests: {
      currentSha256: sha256(read(name, 'current')),
      historySha256: sha256(read(name, 'history')),
      fullSha256: sha256(read(name, 'full')),
    },
    visibleEvidence: visibleSource
      .split('\n')
      .map((line) => line.trimEnd())
      .filter((line) => line.startsWith('Akra  |') || line.includes('Working (') || line.startsWith('notice: activity:') || line.startsWith('prompt:') || line.trim() === '>' || line.includes('D4_COMMITTED_OUTPUT_CANARY')),
  };
}

const artifact = {
  schema: 'akra-typed-activity-rail-e2e/v1',
  artifactId: process.env.AKRA_CAPTURE_ARTIFACT_ID,
  workItem: 'P0-D4 typed activity rail physical-resize regression',
  capturedAt: process.env.AKRA_CAPTURED_AT,
  operator: process.env.AKRA_CAPTURE_OPERATOR,
  reviewer: 'pending',
  captureRole: 'supplemental-unmatched',
  checkProfile: 'typed-activity-rail-e2e-linux-tmux',
  scenarioSet: 'typed-activity-rail-e2e-v1',
  approvalGrade: false,
  sourceBuild: true,
  syntheticAppServer: true,
  releasedRuntime: false,
  candidate: {
    commit: process.env.AKRA_CAPTURE_COMMIT,
    tree: process.env.AKRA_CAPTURE_TREE,
    cleanTree: true,
    cleanChecks: ['before-build', 'after-build', 'after-capture'],
    binarySha256: process.env.AKRA_CAPTURE_BINARY_SHA,
    syntheticCodexSha256: process.env.AKRA_CAPTURE_FAKE_SHA,
    runtimeExitStatus: Number(process.env.AKRA_CAPTURE_EXIT_STATUS),
  },
  environment: {
    os: process.env.AKRA_CAPTURE_OS,
    kernel: process.env.AKRA_CAPTURE_KERNEL,
    environmentClass: 'first-class',
    terminal: process.env.AKRA_CAPTURE_TMUX_VERSION,
    multiplexer: 'tmux detached PTY',
    shell: `bash ${process.env.AKRA_CAPTURE_BASH_VERSION}`,
    node: process.env.AKRA_CAPTURE_NODE_VERSION,
    term: 'tmux-256color',
    frontend: 'inline',
    inlineHistoryRenderMode: 'HostScrollback',
    historyInsertionMode: 'StandardScrollRegion',
    overrides: {
      CODEX_EXEC_LOOP_INLINE_HISTORY_MODE: 'scrollback',
      CODEX_EXEC_LOOP_HISTORY_INSERT_MODE: 'standard',
      CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART: '0',
      AKRA_APP_SERVER_PROMPT_LOG: '0',
      WT_SESSION: 'unset',
      traceVariables: 'unset',
    },
  },
  geometryRationale: '48x18 preserves the complete 16-row inline viewport while exercising the checked-in 48-column narrow layout; physical heights below the inline viewport are excluded.',
  checkpoints: names.map(checkpoint),
  rawPtyProof: {
    scope: 'from process launch through the completed checkpoint, excluding terminal teardown',
    bytes: Number(process.env.AKRA_CAPTURE_RAW_PTY_BYTES),
    sha256: process.env.AKRA_CAPTURE_RAW_PTY_SHA,
    committedCanaryOccurrences: Number(process.env.AKRA_CAPTURE_RAW_CANARY_COUNT),
    rawSecretOccurrences: 0,
    exactAnsiPayloadOccurrences: 0,
  },
  scenarioResults: {
    isolatedStartup: 'pass',
    activeWideRail: 'pass',
    narrowPhysicalResize: 'pass',
    repeatedNarrowResize: 'pass',
    wideRestore: 'pass',
    committedCompletion: 'pass',
    rawPtyPayloadNonDisclosure: 'pass',
    cleanExit: 'pass',
  },
  assertions: {
    startupReadyExactlyOnce: true,
    activeRailVisibleExactlyOnce: true,
    wideModelFactVisibleExactlyOnce: true,
    narrowRailDropsLowerPriorityModelFact: true,
    workingStateVisibleExactlyOnce: true,
    runningPromptVisibleExactlyOnce: true,
    activeRailAbsentFromHostHistory: true,
    runningPromptAbsentFromHostHistory: true,
    rawActivityPayloadAbsentFromRenderedAndRawPtyCapture: true,
    repeatedNarrowResizeIdempotent: true,
    restoredFrameInBounds: true,
    committedCompletionPresentExactlyOnce: true,
    activeRailRemovedAfterCompletion: true,
    runtimeExitedCleanly: true,
  },
  deviations: [
    'synthetic owner-only Codex app-server fixture',
    'source-build runtime rather than packaged release artifact',
    'raw PTY proof stops before terminal teardown after the completed checkpoint',
  ],
  nonClaims: [
    'not a released Akra artifact capture',
    'not a released Codex app-server activity capture',
    'not an allocator, RSS, latency, or throughput measurement',
    'not evidence for Admin, CLI, Telegram, parallel persistence, durable restart recovery, or reconciliation recovery',
    'not approval-grade E1-E4 primitive validation',
    'not evidence for physical terminal heights below the 16-row inline viewport',
  ],
};

rejectUnsafeStrings(artifact, 'artifact');
const serialized = `${JSON.stringify(artifact, null, 2)}\n`;
for (const forbidden of [
  '\\u001b',
  'D4_RAW_ACTIVITY_SECRET',
  '/dev/pts/',
  '/tmp/',
  process.env.HOME || '__unset_home__',
  process.env.AKRA_CAPTURE_REPO_ROOT,
  process.env.AKRA_CAPTURE_RAW_ROOT,
  process.env.AKRA_CAPTURE_SOCKET,
]) {
  if (forbidden && serialized.includes(forbidden)) throw new Error(`sanitized artifact contains forbidden value: ${forbidden}`);
}
if (/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/i.test(serialized)) throw new Error('sanitized artifact contains an email address');
rejectUnsafeText('serialized artifact', Buffer.from(serialized, 'utf8'));
fs.writeFileSync(outputPath, serialized, { flag: 'w', mode: 0o600 });
NODE

ln -- "$staging_path" "$output_path"
chmod 644 "$output_path"
rm -f "$staging_path"
staging_path=""
printf 'typed activity rail E2E capture written: %s\n' "$output_path"
