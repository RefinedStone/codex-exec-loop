#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app_dir="${repo_root}/src/adapter/inbound/tui/app"
status=0

if ! command -v rg >/dev/null 2>&1; then
  echo "ripgrep is required for TUI layering checks" >&2
  exit 2
fi

report_forbidden() {
  local description="$1"
  local pattern="$2"
  local allowed_regex="$3"
  local matches

  matches="$(rg -n --glob '*.rs' "${pattern}" "${app_dir}" || true)"
  if [[ -n "${matches}" && -n "${allowed_regex}" ]]; then
    matches="$(printf '%s\n' "${matches}" | grep -Ev "${allowed_regex}" || true)"
  fi

  if [[ -n "${matches}" ]]; then
    printf 'TUI layering violation: %s\n' "${description}" >&2
    printf '%s\n\n' "${matches}" >&2
    status=1
  fi
}

report_forbidden \
  "raw panel blocks belong in theme.rs; use AkraTheme::panel_block or add a semantic helper" \
  'Block::default\(' \
  '/theme\.rs:'

report_forbidden \
  "raw selected/list highlight symbols belong in AkraTheme::list_highlight_symbol" \
  'highlight_symbol\("' \
  '/theme\.rs:'

report_forbidden \
  "raw brand/status colors belong in theme.rs; use semantic AkraTheme styles" \
  'Color::|\.bg\(' \
  '/theme\.rs:|/history_insertion\.rs:|/shell_rendering_contract_tests\.rs:'

if [[ "${status}" -ne 0 ]]; then
  cat >&2 <<'MSG'
Move visual decisions into src/adapter/inbound/tui/app/theme.rs unless the match is a terminal
adapter escape hatch. See docs/design/07-tui-layered-architecture-and-aesthetic-contract.md.
MSG
  exit 1
fi

echo "tui layering check passed"
