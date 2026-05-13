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
office_asset="${output_dir}/akra-office-background.png"
sprite_asset="${output_dir}/akra-object-sprites.png"
agent_atlas_asset="${output_dir}/gamebaljeonguk_atlas_64x96.png"
agent_atlas_large_asset="${output_dir}/gamebaljeonguk_atlas_128x192.png"
screenshot_path="${output_dir}/admin-graphic.png"

mkdir -p "${output_dir}"

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

find_browser() {
  local browser
  for browser in chromium chromium-browser google-chrome google-chrome-stable microsoft-edge firefox; do
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
    "${HOME}/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge" \
    "/Applications/Firefox.app/Contents/MacOS/firefox" \
    "${HOME}/Applications/Firefox.app/Contents/MacOS/firefox"; do
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
    if curl -fsS "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done

  echo "admin server did not become ready; log follows" >&2
  cat "${server_log}" >&2 || true
  return 1
}

capture_with_browser() {
  local browser="$1"
  local url="$2"

  case "$(basename "${browser}")" in
    firefox)
      echo "screenshot capture skipped: firefox CLI capture is not supported by this script" >&2
      return 2
      ;;
    *)
      "${browser}" \
        --headless \
        --disable-gpu \
        --no-sandbox \
        --window-size=1600,1000 \
        --screenshot="${screenshot_path}" \
        "${url}" >/dev/null 2>&1
      ;;
  esac
}

cleanup() {
  if [[ -n "${server_pid:-}" ]]; then
    kill "${server_pid}" >/dev/null 2>&1 || true
    wait "${server_pid}" >/dev/null 2>&1 || true
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

cargo run --quiet --bin akra-admin -- --port "${port}" >"${server_log}" 2>&1 &
server_pid="$!"

base_url="http://127.0.0.1:${port}"
graphic_url="${base_url}/admin/akra"
metrics_url="${base_url}/admin/akra/metrics"
tasks_url="${base_url}/admin/tasks"
akra_tasks_url="${base_url}/admin/akra/tasks"
wait_for_server "${graphic_url}"

curl -fsS "${graphic_url}" >"${admin_html}"
curl -fsS "${metrics_url}" >"${metrics_html}"
curl -fsS "${tasks_url}" >"${tasks_html}"
curl -fsS "${akra_tasks_url}" >"${akra_tasks_html}"
curl -fsS "${base_url}/api/admin/akra/dashboard" >"${dashboard_json}"
curl -fsS "${base_url}/api/admin/akra/events?limit=50" >"${events_json}"
curl -fsS "${base_url}/api/admin/akra/events?afterSequence=0&limit=50" >"${events_incremental_json}"
curl -fsS "${base_url}/admin/assets/game/akra-diorama.js" >"${game_js}"
curl -fsS "${base_url}/admin/assets/graphics/akra-office-background.png" >"${office_asset}"
curl -fsS "${base_url}/admin/assets/graphics/akra-object-sprites.png" >"${sprite_asset}"
curl -fsS "${base_url}/admin/assets/graphics/gamebaljeonguk_atlas_64x96.png" >"${agent_atlas_asset}"
curl -fsS "${base_url}/admin/assets/graphics/gamebaljeonguk_atlas_128x192.png" >"${agent_atlas_large_asset}"
events_error_status="$(curl -sS -o "${events_error_json}" -w "%{http_code}" "${base_url}/api/admin/akra/events?limit=201")"
if [[ "${events_error_status}" != "400" ]]; then
  echo "expected event limit validation to return 400, got ${events_error_status}" >&2
  cat "${events_error_json}" >&2 || true
  exit 1
fi

for token in \
  '<body class="akra-graphic">' \
  'aria-label="게임발전국 AKRA Admin Control Center"' \
  'class="office-board" id="agents"' \
  'class="pool-overlay" id="pool"' \
  'data-detail-title="워크트리 풀 · 슬롯' \
  'data-detail-slot="슬롯' \
  'data-detail-task="' \
  'data-detail-type="slot"' \
  'data-task-id="' \
  'data-detail-branch="' \
  'data-detail-worktree="' \
  'data-detail-owner="' \
  'title="슬롯' \
  'class="scene-object object-sprite server-rack"' \
  'id="events"' \
  'id="campaign"' \
  'id="attempts"' \
  'id="intel"' \
  'id="pipeline"' \
  '시도 보드' \
  '최근 시도 로그' \
  '정보 카드' \
  'AKRA ADMIN CONTROL CENTER' \
  'akraHashTabRoutes' \
  'directions: "/admin/akra/directions"' \
  'tasks: "/admin/akra/tasks"' \
  'href="/admin/akra/directions"' \
  'href="/admin/akra/tasks"' \
  '작전 방향' \
  'hashchange' \
  'MISSION FLOW' \
  'stage-refresh-btn' \
  '--office-board-height: 720px' \
  '/admin/assets/game/akra-diorama.js' \
  'data-admin-graphic' \
  'data-api-base' \
  'data-poll-interval-ms' \
  'data-focus-target="pipeline"' \
  'data-event-drawer' \
  'data-detail-drawer' \
  'data-refresh-dashboard' \
  'detailSourceKey(node) === nextKey' \
  'optionalText(distributor.bubbleLabel, "배포 파이프라인")' \
  'openDetailDrawer' \
  'openRefreshDetail' \
  'akra:mission-pulse' \
  'pulseStage' \
  'is-bursting' \
  'data-event-feed-status' \
  'gamebaljeonguk_atlas_64x96.png' \
  'background-image: var(--object-sprite-sheet)' \
  'background-image: var(--agent-sprite-sheet)' \
  'var(--office-bg-image) center / cover no-repeat' \
  'akra-office-background.png' \
  'akra-object-sprites.png' \
  'background-size: 384px 504px' \
  'avatar-Artificer' \
  'agentAvatarClass' \
  'background-size: 627px 627px' \
  'prependEventRows' \
  'stale snapshot' \
  'skeleton-line' \
  'grid-template-columns: repeat(8' \
  'grid-template-columns: minmax(0, 1fr)' \
  'overflow: auto' \
  'text-overflow: ellipsis' \
  '@media (max-width: 860px)'; do
  require_contains "${admin_html}" "${token}"
done

for token in \
  'window.AkraAdminGame' \
  'mountDiorama' \
  'new PIXI.Application' \
  'PIXI.Assets.load' \
  'PIXI.Sprite' \
  'PIXI.Container' \
  'app.ticker.add' \
  'gamebaljeonguk_atlas_128x192.png' \
  'AGENT_FRAME_WIDTH' \
  'AGENT_SPRITE_SCALE' \
  'makePacket' \
  'statusPalette' \
  'chooseRoamPoint' \
  'updateRoamMotion' \
  'applyWalkFrame' \
  'buildAgentFrameSets' \
  'rebuildAgentUnits' \
  'akra:dashboard-rendered'; do
  require_contains "${game_js}" "${token}"
done

for token in \
  'class="akra-topbar"' \
  'class="ops-status"' \
  'class="right-stack"' \
  'id="metrics"' \
  'id="system"' \
  '길드 성과' \
  '운영 지표' \
  'akra_admin' \
  'Last Updated' \
  'control tower is live in read-only supervisor mode' \
  'read-only 운영 관제' \
  '게임화 정책' \
  '도메인 매핑' \
  'blocked slot은 operator recovery' \
  'blocked-copy'; do
  require_not_contains "${admin_html}" "${token}"
done

for token in \
  'data-agent-progress' \
  'data-detail-type="agent"' \
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
  '/admin/tasks/delete' \
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
  '/admin/akra/tasks/delete' \
  '/admin/files/export' \
  '/admin/files/apply' \
  '게임발전국 작업 관리'; do
  require_contains "${akra_tasks_html}" "${token}"
done

require_not_contains "${akra_tasks_html}" 'href="/admin/tasks" class="active">Tasks</a>'
require_not_contains "${akra_tasks_html}" 'action="/admin/tasks/upsert"'
require_not_contains "${akra_tasks_html}" 'action="/admin/tasks/delete"'

for token in \
  '"workspace"' \
  '"kpis"' \
  '"pool"' \
  '"agents"' \
  '"distributor"' \
  '"campaign"' \
  '"laneCards"' \
  '"intelCards"' \
  '"events"' \
  '"generatedTimeLabel"' \
  '"automationEpoch"'; do
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

cmp -s assets/admin/graphics/akra-office-background.png "${office_asset}" || {
  echo "served office background asset does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/graphics/akra-object-sprites.png "${sprite_asset}" || {
  echo "served object sprite asset does not match workspace asset" >&2
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

if [[ -f templates/admin/resources/main-sprite.png ]]; then
  sha256sum templates/admin/resources/main-sprite.png >"${output_dir}/reference-img.sha256"
fi

browser_path="$(find_browser || true)"
if [[ -n "${browser_path}" ]]; then
  if capture_with_browser "${browser_path}" "${graphic_url}"; then
    sha256sum "${screenshot_path}" >"${output_dir}/admin-graphic.sha256"
    echo "admin graphic screenshot captured: ${screenshot_path}"
  elif [[ "${capture_mode}" == "always" ]]; then
    exit 1
  fi
elif [[ "${capture_mode}" == "always" ]]; then
  echo "ADMIN_GRAPHIC_CAPTURE=always requires chromium, chrome, edge, or firefox on PATH" >&2
  exit 1
else
  echo "screenshot capture skipped: no supported browser found on PATH"
fi

echo "admin graphic visual contract ok"
