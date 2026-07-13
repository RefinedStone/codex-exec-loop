#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if [[ -f "${HOME}/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  . "${HOME}/.cargo/env"
fi

port="${ADMIN_GRAPHIC_PORT:-18444}"
capture_mode="${ADMIN_GRAPHIC_CAPTURE:-auto}"
output_dir="${ADMIN_GRAPHIC_OUTPUT_DIR:-target/admin-graphic-visual}"
server_log="${output_dir}/akra-admin.log"
admin_html="${output_dir}/admin.html"
metrics_html="${output_dir}/admin-metrics.html"
tasks_html="${output_dir}/admin-tasks.html"
akra_tasks_html="${output_dir}/admin-akra-tasks.html"
dashboard_json="${output_dir}/dashboard.json"
events_json="${output_dir}/events.json"
events_incremental_json="${output_dir}/events-incremental.json"
events_error_json="${output_dir}/events-error.json"
game_js="${output_dir}/akra-diorama.js"
admin_shell_js="${output_dir}/admin-shell.js"
dashboard_js="${output_dir}/akra-dashboard.js"
font_regular="${output_dir}/Galmuri11.woff2"
font_bold="${output_dir}/Galmuri11-Bold.woff2"
final_draft_map_asset="${output_dir}/final-draft-map-sprite.png"
final_draft_desk_asset="${output_dir}/sprite_fd_desk_1.png"
final_draft_tower_asset="${output_dir}/sprite_fd_event_log_tower.png"
agent_atlas_asset="${output_dir}/gamebaljeonguk_atlas_64x96.png"
agent_atlas_large_asset="${output_dir}/gamebaljeonguk_atlas_128x192.png"
screenshot_path="${output_dir}/admin-graphic.png"
mobile_screenshot_path="${output_dir}/admin-graphic-mobile.png"
admin_token=""
admin_host=""
auth_tmp_dir=""
cookie_jar=""

mkdir -p "${output_dir}"

curl_no_config() {
  if [[ -n "${admin_host}" ]]; then
    command curl -q --resolve "${admin_host}:${port}:127.0.0.1" "$@"
  else
    command curl -q "$@"
  fi
}

if [[ "${1:-}" == "--curl-config-probe" ]]; then
  curl_no_config --version >/dev/null
  exit 0
fi

case "${capture_mode}" in
  auto | always) ;;
  *)
    echo "ADMIN_GRAPHIC_CAPTURE must be auto or always" >&2
    exit 1
    ;;
esac

if [[ -n "${ADMIN_GRAPHIC_TOKEN:-}" ]]; then
  if [[ ! "${ADMIN_GRAPHIC_TOKEN}" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "ADMIN_GRAPHIC_TOKEN must be exactly 64 hexadecimal characters" >&2
    exit 1
  fi
  admin_token="${ADMIN_GRAPHIC_TOKEN}"
else
  if ! command -v node >/dev/null 2>&1; then
    echo "node is required to generate the admin visual capability token" >&2
    exit 1
  fi
  admin_token="$(node -e 'process.stdout.write(require("node:crypto").randomBytes(32).toString("hex"))')"
  if [[ ! "${admin_token}" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "failed to generate a 256-bit admin visual capability token" >&2
    exit 1
  fi
fi

require_contains() {
  local file="$1"
  local needle="$2"

  if ! grep -Fq -- "${needle}" "${file}"; then
    echo "missing expected visual contract token in ${file}: ${needle}" >&2
    return 1
  fi
}

require_not_contains() {
  local file="$1"
  local needle="$2"

  if grep -Fq -- "${needle}" "${file}"; then
    echo "unexpected visual contract token in ${file}: ${needle}" >&2
    return 1
  fi
}

is_supported_browser() {
  case "$(basename "$1")" in
    chromium | chromium-browser | chrome | chrome.exe | google-chrome | google-chrome-stable | headless_shell | microsoft-edge | msedge | msedge.exe | "Chromium" | "Google Chrome" | "Microsoft Edge")
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

if [[ -n "${ADMIN_GRAPHIC_BROWSER:-}" ]]; then
  if [[ ! -x "${ADMIN_GRAPHIC_BROWSER}" ]]; then
    echo "ADMIN_GRAPHIC_BROWSER is not executable" >&2
    exit 1
  fi
  if ! is_supported_browser "${ADMIN_GRAPHIC_BROWSER}"; then
    echo "ADMIN_GRAPHIC_BROWSER must point to a Chromium, Chrome, or Edge executable" >&2
    exit 1
  fi
fi

find_browser() {
  if [[ -n "${ADMIN_GRAPHIC_BROWSER:-}" ]]; then
    printf '%s\n' "${ADMIN_GRAPHIC_BROWSER}"
    return 0
  fi

  local browser
  for browser in chromium chromium-browser google-chrome google-chrome-stable microsoft-edge; do
    if command -v "${browser}" >/dev/null 2>&1; then
      command -v "${browser}"
      return 0
    fi
  done

  local candidate
  for candidate in \
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
    "${HOME}/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
    "/Applications/Chromium.app/Contents/MacOS/Chromium" \
    "${HOME}/Applications/Chromium.app/Contents/MacOS/Chromium" \
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge" \
    "${HOME}/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"; do
    if [[ -x "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done

  return 1
}

wait_for_server() {
  local url="$1"

  for _ in $(seq 1 80); do
    if curl_no_config -fsS "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done

  echo "admin server did not become ready; log follows" >&2
  cat "${server_log}" >&2 || true
  return 1
}

authenticated_curl() {
  curl_no_config -b "${cookie_jar}" "$@"
}

capture_with_browser() {
  local browser="$1"
  local url="$2"

  if ! node -e "require.resolve('@playwright/test')" >/dev/null 2>&1; then
    npm ci --ignore-scripts
  fi
  AKRA_ADMIN_VISUAL_TOKEN="${admin_token}" node scripts/capture_admin_graphic.mjs \
    --browser="${browser}" \
    --url="${url}" \
    --screenshot="${screenshot_path}" \
    --mobile-screenshot="${mobile_screenshot_path}"
}

cleanup() {
  if [[ -n "${server_pid:-}" ]]; then
    kill "${server_pid}" >/dev/null 2>&1 || true
    wait "${server_pid}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${auth_tmp_dir}" ]]; then
    rm -rf "${auth_tmp_dir}"
  fi
}
trap cleanup EXIT

if [[ "${ADMIN_GAME_BUILD:-1}" != "0" ]]; then
  if [[ ! -d assets/admin/game/node_modules ]]; then
    npm --prefix assets/admin/game ci
  fi
  npm --prefix assets/admin/game run check
  npm --prefix assets/admin/game run build
fi

auth_tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/akra-admin-visual-auth.XXXXXX")"
chmod 700 "${auth_tmp_dir}"
cookie_jar="${auth_tmp_dir}/cookies.txt"
login_form="${auth_tmp_dir}/login-form.txt"
printf 'token=%s' "${admin_token}" >"${login_form}"
chmod 600 "${login_form}"

# Keep the server-readiness deadline scoped to process startup. A cold Rust
# build can legitimately take longer than that deadline and otherwise leaves
# an empty server log that looks like an admin runtime failure.
cargo build --quiet --locked --bin akra-admin

AKRA_ADMIN_TOKEN="${admin_token}" \
  AKRA_HOME="${auth_tmp_dir}/akra-home" \
  cargo run --quiet --locked --bin akra-admin -- --port "${port}" >"${server_log}" 2>&1 &
server_pid="$!"

for _ in $(seq 1 80); do
  base_url="$(sed -n 's/^admin login: \(http:\/\/[^/]*\)\/admin\/login$/\1/p' "${server_log}" | tail -n 1)"
  if [[ -n "${base_url}" ]]; then
    break
  fi
  if ! kill -0 "${server_pid}" >/dev/null 2>&1; then
    echo "admin server exited before publishing its isolated origin; log follows" >&2
    cat "${server_log}" >&2 || true
    exit 1
  fi
  sleep 0.25
done
if [[ -z "${base_url:-}" ]]; then
  echo "admin server did not publish its isolated origin; log follows" >&2
  cat "${server_log}" >&2 || true
  exit 1
fi
admin_host="${base_url#http://}"
admin_host="${admin_host%:${port}}"
if [[ ! "${admin_host}" =~ ^akra-[0-9a-f]{32}\.[0-9a-f]{32}\.localhost$ ]]; then
  echo "admin server returned an invalid isolated localhost origin: ${base_url}" >&2
  exit 1
fi
graphic_url="${base_url}/admin/akra"
metrics_url="${base_url}/admin/akra/metrics"
tasks_url="${base_url}/admin/tasks"
akra_tasks_url="${base_url}/admin/akra/tasks"
wait_for_server "${base_url}/admin/login"

login_status="$({
  curl_no_config -sS \
    -o /dev/null \
    -c "${cookie_jar}" \
    -w "%{http_code}" \
    -H "Origin: ${base_url}" \
    -H "Referer: ${base_url}/admin/login" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-binary "@${login_form}" \
    "${base_url}/admin/login"
})"
if [[ "${login_status}" != "303" ]]; then
  echo "expected admin login to return 303, got ${login_status}" >&2
  exit 1
fi
chmod 600 "${cookie_jar}"

authenticated_curl -fsS "${graphic_url}" >"${admin_html}"
authenticated_curl -fsS "${metrics_url}" >"${metrics_html}"
authenticated_curl -fsS "${tasks_url}" >"${tasks_html}"
authenticated_curl -fsS "${akra_tasks_url}" >"${akra_tasks_html}"
authenticated_curl -fsS "${base_url}/api/admin/akra/dashboard" >"${dashboard_json}"
authenticated_curl -fsS "${base_url}/api/admin/akra/events?limit=50" >"${events_json}"
authenticated_curl -fsS "${base_url}/api/admin/akra/events?afterSequence=0&limit=50" >"${events_incremental_json}"
authenticated_curl -fsS "${base_url}/admin/assets/game/akra-diorama.js" >"${game_js}"
authenticated_curl -fsS "${base_url}/admin/assets/scripts/admin-shell.js" >"${admin_shell_js}"
authenticated_curl -fsS "${base_url}/admin/assets/scripts/akra-dashboard.js" >"${dashboard_js}"
authenticated_curl -fsS "${base_url}/admin/assets/fonts/Galmuri11.woff2" >"${font_regular}"
authenticated_curl -fsS "${base_url}/admin/assets/fonts/Galmuri11-Bold.woff2" >"${font_bold}"
authenticated_curl -fsS "${base_url}/admin/assets/graphics/final-draft-map-sprite.png" >"${final_draft_map_asset}"
authenticated_curl -fsS "${base_url}/admin/assets/graphics/sprite_fd_desk_1.png" >"${final_draft_desk_asset}"
authenticated_curl -fsS "${base_url}/admin/assets/graphics/sprite_fd_event_log_tower.png" >"${final_draft_tower_asset}"
authenticated_curl -fsS "${base_url}/admin/assets/graphics/gamebaljeonguk_atlas_64x96.png" >"${agent_atlas_asset}"
authenticated_curl -fsS "${base_url}/admin/assets/graphics/gamebaljeonguk_atlas_128x192.png" >"${agent_atlas_large_asset}"
events_error_status="$(authenticated_curl -sS -o "${events_error_json}" -w "%{http_code}" "${base_url}/api/admin/akra/events?limit=201")"
if [[ "${events_error_status}" != "400" ]]; then
  echo "expected event limit validation to return 400, got ${events_error_status}" >&2
  cat "${events_error_json}" >&2 || true
  exit 1
fi

for token in \
  '<body class="akra-graphic akra-dashboard-page">' \
  'aria-label="게임발전국 AKRA Admin Control Center"' \
  'class="office-board" id="agents"' \
  'class="game-panel pool-overlay" id="pool"' \
  'data-detail-title="워크트리 풀 · 슬롯' \
  'data-detail-slot="슬롯' \
  'data-detail-task="' \
  'data-detail-type="slot"' \
  'data-task-id="' \
  'data-detail-branch="' \
  'data-detail-worktree="' \
  'data-detail-owner="' \
  'title="슬롯' \
  'id="events"' \
  'id="notices"' \
  'id="system-mini"' \
  'id="pipeline"' \
  '운영 알림' \
  '시스템 상태 요약' \
  'AKRA ADMIN CONTROL CENTER' \
  'class="draft-nav" aria-label="AKRA dashboard navigation"' \
  'href="/admin/akra" aria-current="page"' \
  'href="/admin/akra/metrics"' \
  'href="/admin/controls"' \
  'href="/admin/akra/directions"' \
  'href="/admin/akra/tasks"' \
  '작전 방향' \
  'MISSION FLOW' \
  'stage-refresh-btn' \
  '--office-board-height: clamp(520px, 56vw, 650px)' \
  '/admin/assets/game/akra-diorama.js' \
  'data-admin-graphic' \
  'data-planning-revision' \
  'data-poll-interval-ms' \
  'data-focus-target="pipeline"' \
  'data-event-drawer' \
  'data-detail-drawer' \
  'data-refresh-dashboard' \
  'data-scene-actor-list' \
  'data-scene-diagnostics' \
  'data-event-feed-status' \
  'gamebaljeonguk_atlas_64x96.png' \
  'background-image: var(--agent-sprite-sheet)' \
  'background: var(--office-bg-image) 0 0 / 100% 100% no-repeat' \
  'final-draft-map-sprite.png' \
  'office-map-image' \
  'width: min(100%, 1040px)' \
  'background-size: 384px 504px' \
  'avatar-Artificer' \
  'skeleton-line' \
  'grid-template-columns: repeat(8' \
  'grid-template-columns: minmax(0, 1fr)' \
  'body.akra-graphic .admin-layout {' \
  'overflow: auto' \
  'text-overflow: ellipsis' \
  '@media (max-width: 860px)'; do
  require_contains "${admin_html}" "${token}"
done

for token in \
  'akraHashTabRoutes' \
  'directions: "/admin/akra/directions"' \
  'tasks: "/admin/akra/tasks"' \
  'hashchange'; do
  require_contains "${admin_shell_js}" "${token}"
done

for token in \
  'detailSourceKey(node) === nextKey' \
  'optionalText(distributor.bubbleLabel, "배포 파이프라인")' \
  'openDetailDrawer' \
  'openRefreshDetail' \
  'createActorButton' \
  'renderActors' \
  'sceneSignature' \
  'dashboardRequest' \
  'eventsRequest' \
  'agentAvatarClass' \
  'prependEventRows' \
  'stale snapshot'; do
  require_contains "${dashboard_js}" "${token}"
done

for token in \
  'AkraAdminGame' \
  'pixi-diorama' \
  'gamebaljeonguk_atlas_128x192.png' \
  'sprite_fd_desk_1.png' \
  'sprite_fd_event_log_tower.png' \
  'inspectScene' \
  'Promise.allSettled' \
  'PixiJS - The MIT License'; do
  require_contains "${game_js}" "${token}"
done

for token in \
  'class="akra-topbar"' \
  'class="ops-status"' \
  'id="metrics"' \
  '길드 성과' \
  '운영 지표' \
  'akra_admin' \
  'Last Updated' \
  'control tower is live in read-only supervisor mode' \
  'read-only 운영 관제' \
  '게임화 정책' \
  '도메인 매핑' \
  'blocked slot은 operator recovery' \
  'is-bursting' \
  'blocked-copy'; do
  require_not_contains "${admin_html}" "${token}"
done

for token in \
  'akra:mission-pulse' \
  'pulseStage' \
  'createSlotAgentButton' \
  'renderAgents(dashboard.pool)'; do
  require_not_contains "${dashboard_js}" "${token}"
done

for token in \
  'data-agent-progress' \
  'data-detail-title="슬롯 요원'; do
  require_not_contains "${admin_html}" "${token}"
done

for token in \
  '<body class="akra-graphic">' \
  'aria-label="AKRA detached metrics"' \
  'id="metrics"' \
  'id="system"' \
  '길드 성과' \
  '운영 지표' \
  '풀 활용률' \
  '지표 출처' \
  'Worktree 풀'; do
  require_contains "${metrics_html}" "${token}"
done

for token in \
  'href="/admin/tasks" class="active">Tasks</a>' \
  'href="/admin/akra">Graphic dashboard</a>' \
  '<summary>Add task</summary>' \
  'Task catalog view' \
  'Skipped tasks' \
  'class="toolbar"' \
  'class="create-panel"' \
  'class="metric-row"' \
  'class="list-panel"' \
  'class="entity-list" id="task-list"' \
  'Search tasks' \
  'id="task-list"' \
  'data-list-filter="task-list"' \
  '/admin/tasks/upsert' \
  '/admin/files/export' \
  '/admin/files/apply' \
  'Tasks'; do
  require_contains "${tasks_html}" "${token}"
done

require_not_contains "${tasks_html}" '<body class="akra-graphic">'
require_not_contains "${tasks_html}" 'aria-label="게임발전국 작업 관리"'
require_not_contains "${tasks_html}" 'class="akra-task-console"'
require_not_contains "${admin_html}" 'tasks: "/admin/tasks"'
require_not_contains "${admin_html}" 'href="/admin/tasks"'

for token in \
  '<body class="akra-graphic">' \
  'href="/admin/akra/tasks" class="active"><span class="nav-icon">T</span><span>작업 관리</span></a>' \
  '<summary>Add task</summary>' \
  'Task catalog view' \
  'Skipped tasks' \
  'class="toolbar"' \
  'class="create-panel"' \
  'class="metric-row"' \
  'class="list-panel"' \
  'class="entity-list" id="task-list"' \
  'Search tasks' \
  'id="task-list"' \
  'data-list-filter="task-list"' \
  '/admin/akra/tasks/upsert' \
  '/admin/files/export' \
  '/admin/files/apply' \
  '게임발전국 작업 관리'; do
  require_contains "${akra_tasks_html}" "${token}"
done

require_contains "templates/admin/tasks.html" 'action="{{ task_delete_path }}"'

require_not_contains "${akra_tasks_html}" 'href="/admin/tasks" class="active">Tasks</a>'
require_not_contains "${akra_tasks_html}" 'action="/admin/tasks/upsert"'
require_not_contains "${akra_tasks_html}" 'action="/admin/tasks/delete"'

for token in \
  '"workspace"' \
  '"kpis"' \
  '"pool"' \
  '"agents"' \
  '"scene"' \
  '"stations"' \
  '"actors"' \
  '"diagnostics"' \
  '"distributor"' \
  '"campaign"' \
  '"laneCards"' \
  '"intelCards"' \
  '"events"' \
  '"generatedTimeLabel"' \
  '"planningRevision"'; do
  require_contains "${dashboard_json}" "${token}"
done

for token in \
  '"feed"' \
  '"events"' \
  '"limit"' \
  '"totalEventCount"' \
  '"incremental"'; do
  require_contains "${events_json}" "${token}"
  require_contains "${events_incremental_json}" "${token}"
done

for token in \
  '"error":"event_limit_too_large"' \
  '"operatorMessage":"Runtime event API limit must be 200 or less."'; do
  require_contains "${events_error_json}" "${token}"
done

cmp -s assets/admin/graphics/final-draft-map-sprite.png "${final_draft_map_asset}" || {
  echo "served final draft map asset does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/graphics/sprite_fd_desk_1.png "${final_draft_desk_asset}" || {
  echo "served final draft desk sprite asset does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/graphics/sprite_fd_event_log_tower.png "${final_draft_tower_asset}" || {
  echo "served final draft event tower sprite asset does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/graphics/gamebaljeonguk_atlas_64x96.png "${agent_atlas_asset}" || {
  echo "served gamebaljeonguk agent atlas does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/graphics/gamebaljeonguk_atlas_128x192.png "${agent_atlas_large_asset}" || {
  echo "served large gamebaljeonguk agent atlas does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/game/akra-diorama.js "${game_js}" || {
  echo "served admin game diorama asset does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/scripts/admin-shell.js "${admin_shell_js}" || {
  echo "served admin shell script does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/scripts/akra-dashboard.js "${dashboard_js}" || {
  echo "served admin dashboard script does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/fonts/Galmuri11.woff2 "${font_regular}" || {
  echo "served regular admin font does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/fonts/Galmuri11-Bold.woff2 "${font_bold}" || {
  echo "served bold admin font does not match workspace asset" >&2
  exit 1
}

if [[ -f templates/admin/resources/main-sprite.png ]]; then
  sha256sum templates/admin/resources/main-sprite.png >"${output_dir}/reference-img.sha256"
fi

browser_path=""
if browser_path="$(find_browser)"; then
  if capture_with_browser "${browser_path}" "${graphic_url}"; then
    sha256sum "${screenshot_path}" >"${output_dir}/admin-graphic.sha256"
    sha256sum "${mobile_screenshot_path}" >"${output_dir}/admin-graphic-mobile.sha256"
    echo "admin graphic screenshots captured: ${screenshot_path}, ${mobile_screenshot_path}"
  else
    exit 1
  fi
else
  if [[ -n "${ADMIN_GRAPHIC_BROWSER:-}" ]]; then
    exit 1
  fi
  if [[ "${capture_mode}" == "always" ]]; then
    echo "ADMIN_GRAPHIC_CAPTURE=always requires Chromium, Chrome, or Edge on PATH" >&2
    exit 1
  fi
  echo "screenshot capture skipped: no supported browser found on PATH"
fi

echo "admin graphic visual contract ok"
