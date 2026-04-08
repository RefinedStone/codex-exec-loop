#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/capture_native_validation.sh --frontend <inline|alternate> [options]

Options:
  --frontend <value>   Validation row frontend label. Required.
  --terminal <value>   Terminal app/version label. Default: detected from env.
  --shell <value>      Shell label. Default: detected from env.
  --term <value>       TERM value to record. Default: current TERM.
  --result <value>     Result label such as pass or blocker. Default: pending.
  --notes <value>      Free-form validation notes. Default: empty.
  --commit <value>     Commit SHA to record. Default: current HEAD.
  --os <value>         OS label. Default: detected from host.
  --date <value>       Date label. Default: today in local time.
  --output <path>      Write the report to a file instead of stdout.
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
  elif [[ -n "${TERM_PROGRAM-}" ]]; then
    printf '%s' "${TERM_PROGRAM}"
  elif [[ -n "${LC_TERMINAL-}" ]]; then
    printf '%s' "${LC_TERMINAL}"
  else
    printf 'unknown'
  fi
}

detect_shell() {
  if [[ -n "${SHELL-}" ]]; then
    basename "${SHELL}"
  else
    printf 'unknown'
  fi
}

detect_os() {
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

repo_root="$(git rev-parse --show-toplevel)"
frontend=""
terminal=""
shell_name=""
term_value="${TERM-}"
result="pending"
notes=""
commit_sha=""
os_value=""
date_value=""
output_path=""

while (($# > 0)); do
  case "$1" in
    --frontend)
      require_value "$1" "${2-}"
      frontend="$2"
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
  printf '--frontend is required\n' >&2
  usage >&2
  exit 1
fi

if [[ -z "${terminal}" ]]; then
  terminal="$(detect_terminal)"
fi

if [[ -z "${shell_name}" ]]; then
  shell_name="$(detect_shell)"
fi

if [[ -z "${commit_sha}" ]]; then
  commit_sha="$(git -C "${repo_root}" rev-parse HEAD)"
fi

if [[ -z "${os_value}" ]]; then
  os_value="$(detect_os)"
fi

if [[ -z "${date_value}" ]]; then
  date_value="$(date '+%Y-%m-%d')"
fi

report="$(cat <<EOF
date: ${date_value}
commit: ${commit_sha}
os: ${os_value}
terminal: ${terminal}
shell: ${shell_name}
frontend: ${frontend}
term: ${term_value}
checks:
- launch and exit
- frontend selection
- input editing
- overlay flow
- streaming visibility
- resize and scrollback
- failure and recovery
result: ${result}
notes: ${notes}
EOF
)"

if [[ -n "${output_path}" ]]; then
  mkdir -p "$(dirname "${output_path}")"
  printf '%s\n' "${report}" > "${output_path}"
else
  printf '%s\n' "${report}"
fi
