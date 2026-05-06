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
legacy_html="${output_dir}/legacy.html"
dashboard_json="${output_dir}/dashboard.json"
events_json="${output_dir}/events.json"
events_incremental_json="${output_dir}/events-incremental.json"
events_error_json="${output_dir}/events-error.json"
sprites_svg="${output_dir}/admin-character-sprites.svg"
office_asset="${output_dir}/akra-office-background.png"
sprite_asset="${output_dir}/akra-object-sprites.png"
screenshot_path="${output_dir}/admin-graphic.png"

mkdir -p "${output_dir}"

require_contains() {
  local file="$1"
  local needle="$2"

  if ! grep -Fq "${needle}" "${file}"; then
    echo "missing expected visual contract token in ${file}: ${needle}" >&2
    return 1
  fi
}

require_not_contains() {
  local file="$1"
  local needle="$2"

  if grep -Fq "${needle}" "${file}"; then
    echo "unexpected visual contract token in ${file}: ${needle}" >&2
    return 1
  fi
}

find_browser() {
  for browser in chromium chromium-browser google-chrome google-chrome-stable microsoft-edge firefox; do
    if command -v "${browser}" >/dev/null 2>&1; then
      command -v "${browser}"
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

cargo run --quiet --bin akra-admin -- --port "${port}" >"${server_log}" 2>&1 &
server_pid="$!"

base_url="http://127.0.0.1:${port}"
wait_for_server "${base_url}/admin"

curl -fsS "${base_url}/admin" >"${admin_html}"
curl -fsS "${base_url}/admin/legacy" >"${legacy_html}"
curl -fsS "${base_url}/api/admin/akra/dashboard" >"${dashboard_json}"
curl -fsS "${base_url}/api/admin/akra/events?limit=50" >"${events_json}"
curl -fsS "${base_url}/api/admin/akra/events?afterSequence=0&limit=50" >"${events_incremental_json}"
curl -fsS "${base_url}/assets/admin/admin-character-sprites.svg" >"${sprites_svg}"
curl -fsS "${base_url}/admin/assets/graphics/akra-office-background.png" >"${office_asset}"
curl -fsS "${base_url}/admin/assets/graphics/akra-object-sprites.png" >"${sprite_asset}"
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
  'id="events"' \
  'id="pipeline"' \
  'id="metrics"' \
  'AKRA ADMIN CONTROL CENTER' \
  'Automation Epoch' \
  'Last Updated' \
  'akra_admin' \
  'Legacy Admin' \
  'data-admin-graphic' \
  'data-api-base' \
  'data-poll-interval-ms' \
  'data-focus-target="pipeline"' \
  'data-event-drawer' \
  'data-event-feed-status' \
  '/assets/admin/admin-character-sprites.svg' \
  'background-size: 240px 48px' \
  'akra-office-background.png' \
  'akra-object-sprites.png' \
  'prependEventRows' \
  'stale snapshot' \
  'skeleton-line' \
  'grid-template-columns: repeat(8' \
  'max-height: 540px' \
  'overflow: auto' \
  'text-overflow: ellipsis' \
  '@media (max-width: 860px)'; do
  require_contains "${admin_html}" "${token}"
done

for token in \
  '"workspace"' \
  '"kpis"' \
  '"pool"' \
  '"agents"' \
  '"distributor"' \
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
  '<svg xmlns="http://www.w3.org/2000/svg" width="240" height="48"' \
  'id="agent-normal"' \
  'id="agent-warning"' \
  'id="agent-danger"' \
  'id="distributor"' \
  'id="event-board"'; do
  require_contains "${sprites_svg}" "${token}"
done

for token in \
  '"error":"event_limit_too_large"' \
  '"operatorMessage":"Runtime event API limit must be 200 or less."'; do
  require_contains "${events_error_json}" "${token}"
done

require_contains "${legacy_html}" "Workspace Status"
require_contains "${legacy_html}" "Open Full Planning Draft"
require_not_contains "${legacy_html}" '<body class="akra-graphic">'

cmp -s assets/admin/graphics/akra-office-background.png "${office_asset}" || {
  echo "served office background asset does not match workspace asset" >&2
  exit 1
}
cmp -s assets/admin/graphics/akra-object-sprites.png "${sprite_asset}" || {
  echo "served object sprite asset does not match workspace asset" >&2
  exit 1
}

if [[ -f docs/gamification/img.png ]]; then
  sha256sum docs/gamification/img.png >"${output_dir}/reference-img.sha256"
fi

browser_path="$(find_browser || true)"
if [[ -n "${browser_path}" ]]; then
  if capture_with_browser "${browser_path}" "${base_url}/admin"; then
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
