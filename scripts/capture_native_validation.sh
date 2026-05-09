#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/capture_native_validation.sh --frontend <inline|alternate> [options]

Options:
  --frontend <value>   Validation row frontend label. Required.
  --check-profile <value>
                       Validation checklist profile. Default: terminal-baseline.
  --terminal <value>   Terminal app/version label. Default: detected from env.
  --shell <value>      Shell label. Default: detected from env.
  --term <value>       TERM value to record. Default: current TERM.
  --result <value>     Result label such as pass or blocker. Default: pending.
  --notes <value>      Free-form validation notes. Default: empty.
  --commit <value>     Commit SHA to record. Default: current HEAD.
  --os <value>         OS label. Default: detected from host.
  --date <value>       Date label. Default: today in local time.
  --output <path>      Write the report to a file instead of stdout.
  --output-dir <path>  Generate a slugged report filename under this directory.
  -h, --help           Show this help text.
EOF
}

require_value() {
  local option="$1"
  local value="${2-}"
  if [[ -z "${value}" ]]; then
    printf 'missing value for %s\n' "${option}" >&2
    exit 1
  fi
}

detect_terminal() {
  if [[ -n "${WT_SESSION-}" ]]; then
    printf 'Windows Terminal'
  elif [[ -n "${TERMINAL_EMULATOR-}" ]]; then
    printf '%s' "${TERMINAL_EMULATOR}"
  elif [[ -n "${TERM_PROGRAM-}" ]]; then
    printf '%s' "${TERM_PROGRAM}"
  elif [[ -n "${LC_TERMINAL-}" ]]; then
    printf '%s' "${LC_TERMINAL}"
  else
    printf 'unknown'
  fi
}

detect_shell() {
  if [[ -n "${WSL_DISTRO_NAME-}" ]]; then
    if [[ -n "${SHELL-}" ]]; then
      printf 'WSL %s' "$(basename "${SHELL}")"
    else
      printf 'WSL shell'
    fi
    return
  fi

  if [[ -n "${SHELL-}" ]]; then
    basename "${SHELL}"
  else
    printf 'unknown'
  fi
}

detect_windows_host_os_from_wsl() {
  local powershell_path='/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe'
  local host_os

  if [[ ! -x "${powershell_path}" ]]; then
    return 1
  fi

  host_os="$(
    "${powershell_path}" -NoProfile -Command \
      '$instance = Get-CimInstance Win32_OperatingSystem; "$($instance.Caption) $($instance.Version)"' \
      2>/dev/null | tr -d '\r' | head -n 1
  )"

  if [[ -z "${host_os}" ]]; then
    return 1
  fi

  printf '%s' "${host_os}"
}

detect_os() {
  if [[ -n "${WSL_DISTRO_NAME-}" ]]; then
    local host_os=""
    host_os="$(detect_windows_host_os_from_wsl || true)"
    if [[ -n "${host_os}" ]]; then
      printf '%s / WSL %s' "${host_os}" "${WSL_DISTRO_NAME}"
      return
    fi
  fi

  if command -v sw_vers >/dev/null 2>&1; then
    printf 'macOS %s %s' "$(sw_vers -productVersion)" "$(sw_vers -buildVersion)"
    return
  fi

  if [[ -r /etc/os-release ]]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    if [[ -n "${PRETTY_NAME-}" ]]; then
      printf '%s' "${PRETTY_NAME}"
      return
    fi
  fi

  uname -sr
}

slugify() {
  printf '%s' "$1" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//; s/-{2,}/-/g'
}

render_checks() {
  local check_profile="$1"

  case "${check_profile}" in
    terminal-baseline)
      cat <<'EOF'
- launch and exit
- frontend selection
- input editing
- overlay flow
- streaming visibility
- resize and scrollback
- failure and recovery
EOF
      ;;
    phase1-operator-surface)
      cat <<'EOF'
- launch and exit
- frontend selection
- input editing
- overlay flow
- streaming visibility
- resize and scrollback
- failure and recovery
- status language and next action
- resumed session context
- queue and automation explanation
- lifecycle command parity
EOF
      ;;
    prompt-input-delay-pty)
      cat <<'EOF'
- launch with detached PTY backend
- prompt input echoes without visible delay
- multiline input stays editable before submit
- submitted prompt transitions into streaming output
- completion preserves prompt history and cursor recovery
- interrupt and recovery remain responsive after delayed input checks
EOF
      ;;
    *)
      printf 'unsupported --check-profile: %s\n' "${check_profile}" >&2
      exit 1
      ;;
  esac
}

frontend=""
check_profile="terminal-baseline"
terminal=""
shell_name=""
term_value="${TERM-}"
result="pending"
notes=""
commit_sha=""
os_value=""
date_value=""
output_path=""
output_dir=""

while (($# > 0)); do
  case "$1" in
    --frontend)
      require_value "$1" "${2-}"
      frontend="$2"
      shift 2
      ;;
    --check-profile)
      require_value "$1" "${2-}"
      check_profile="$2"
      shift 2
      ;;
    --terminal)
      require_value "$1" "${2-}"
      terminal="$2"
      shift 2
      ;;
    --shell)
      require_value "$1" "${2-}"
      shell_name="$2"
      shift 2
      ;;
    --term)
      require_value "$1" "${2-}"
      term_value="$2"
      shift 2
      ;;
    --result)
      require_value "$1" "${2-}"
      result="$2"
      shift 2
      ;;
    --notes)
      require_value "$1" "${2-}"
      notes="$2"
      shift 2
      ;;
    --commit)
      require_value "$1" "${2-}"
      commit_sha="$2"
      shift 2
      ;;
    --os)
      require_value "$1" "${2-}"
      os_value="$2"
      shift 2
      ;;
    --date)
      require_value "$1" "${2-}"
      date_value="$2"
      shift 2
      ;;
    --output)
      require_value "$1" "${2-}"
      output_path="$2"
      shift 2
      ;;
    --output-dir)
      require_value "$1" "${2-}"
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
      exit 1
      ;;
  esac
done

if [[ -z "${frontend}" ]]; then
  printf '%s\n' '--frontend is required' >&2
  usage >&2
  exit 1
fi

if [[ -n "${output_path}" && -n "${output_dir}" ]]; then
  printf '%s\n' '--output and --output-dir cannot be used together' >&2
  exit 1
fi

if [[ -z "${terminal}" ]]; then
  terminal="$(detect_terminal)"
fi

if [[ -z "${shell_name}" ]]; then
  shell_name="$(detect_shell)"
fi

if [[ -z "${commit_sha}" ]]; then
  repo_root="$(git rev-parse --show-toplevel)"
  commit_sha="$(git -C "${repo_root}" rev-parse HEAD)"
fi

if [[ -z "${os_value}" ]]; then
  os_value="$(detect_os)"
fi

if [[ -z "${date_value}" ]]; then
  date_value="$(date '+%Y-%m-%d')"
fi

if [[ -n "${output_dir}" ]]; then
  output_path="${output_dir}/${date_value}-$(slugify "${os_value}")-$(slugify "${terminal}")-$(slugify "${shell_name}")-$(slugify "${frontend}").txt"
fi

checks_block="$(render_checks "${check_profile}")"
report="$(cat <<EOF
date: ${date_value}
commit: ${commit_sha}
os: ${os_value}
terminal: ${terminal}
shell: ${shell_name}
frontend: ${frontend}
term: ${term_value}
check_profile: ${check_profile}
checks:
${checks_block}
result: ${result}
notes: ${notes}
EOF
)"

if [[ -n "${output_path}" ]]; then
  mkdir -p "$(dirname "${output_path}")"
  printf '%s\n' "${report}" > "${output_path}"
  printf 'wrote %s\n' "${output_path}"
else
  printf '%s\n' "${report}"
fi
