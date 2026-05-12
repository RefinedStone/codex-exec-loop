#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  bash scripts/summarize_native_validation.sh [options]

Options:
  --records-dir <path>    Validation record directory. Default: docs/validation
  --check-profile <value> Validation checklist profile. Default: terminal-baseline
  --format <text|markdown>
                          Output format. Default: text
  --fail-on-incomplete    Exit non-zero unless every required row is recorded as pass
  -h, --help              Show this help text.
EOF
}

slugify() {
  printf '%s' "$1" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//; s/-{2,}/-/g'
}

canonical_os_family() {
  local value
  value="$(slugify "$1")"
  case "${value}" in
    macos*|darwin*)
      printf 'macos'
      ;;
    microsoft-windows*|windows*|mingw*|msys*)
      printf 'windows'
      ;;
    ubuntu*|debian*|fedora*|arch*|pop-os*|red-hat*|rhel*|centos*|alpine*|linux*|gnu-linux*)
      printf 'linux'
      ;;
    *)
      printf '%s' "${value}"
      ;;
  esac
}

canonical_terminal() {
  local value
  value="$(slugify "$1")"
  case "${value}" in
    terminal-app*|apple-terminal*)
      printf 'terminal-app'
      ;;
    iterm2*)
      printf 'iterm2'
      ;;
    windows-terminal*)
      printf 'windows-terminal'
      ;;
    jetbrains-jediterm*|jetbrains-terminal*|jediterm*)
      printf 'jetbrains-terminal'
      ;;
    git-bash*|git-for-windows*|mingw64*|mingw32*)
      printf 'git-bash'
      ;;
    tmux*)
      printf 'tmux'
      ;;
    zellij*)
      printf 'zellij'
      ;;
    visual-studio-code*|vscode*|vs-code*)
      printf 'vscode'
      ;;
    linux-terminal*|direct-terminal*|bare-terminal*|gnome-terminal*|konsole*|xterm*|alacritty*|kitty*|wezterm*)
      printf 'linux-terminal'
      ;;
    *)
      printf '%s' "${value}"
      ;;
  esac
}

canonical_shell() {
  local value
  value="$(slugify "$1")"
  case "${value}" in
    zsh*)
      printf 'zsh'
      ;;
    powershell*|pwsh*)
      printf 'powershell'
      ;;
    wsl-bash*|wsl2-bash*|wsl-ubuntu-bash*|wsl*)
      printf 'wsl-bash'
      ;;
    bash)
      printf 'bash'
      ;;
    *)
      printf '%s' "${value}"
      ;;
  esac
}

canonical_frontend() {
  local value
  value="$(slugify "$1")"
  case "${value}" in
    alternate|alternate-screen|fullscreen|alt)
      printf 'alternate'
      ;;
    inline|inline-main-buffer|main-buffer)
      printf 'inline'
      ;;
    *)
      printf '%s' "${value}"
      ;;
  esac
}

read_field() {
  local file="$1"
  local field="$2"
  sed -n "s/^${field}:[[:space:]]*//p" "${file}" | head -n 1
}

row_key() {
  printf '%s|%s|%s|%s' "$1" "$2" "$3" "$4"
}

row_key_exists() {
  local needle="$1"
  local key
  for key in "${row_keys[@]}"; do
    if [[ "${key}" == "${needle}" ]]; then
      return 0
    fi
  done
  return 1
}

set_latest_for_row() {
  local key="$1"
  local result="$2"
  local file="$3"
  local index
  for index in "${!latest_keys[@]}"; do
    if [[ "${latest_keys[${index}]}" == "${key}" ]]; then
      latest_results[${index}]="${result}"
      latest_files[${index}]="${file}"
      return
    fi
  done
  latest_keys+=("${key}")
  latest_results+=("${result}")
  latest_files+=("${file}")
}

latest_row_value() {
  local key="$1"
  local field="$2"
  local default_value="$3"
  local index
  for index in "${!latest_keys[@]}"; do
    if [[ "${latest_keys[${index}]}" == "${key}" ]]; then
      case "${field}" in
        result)
          printf '%s' "${latest_results[${index}]}"
          ;;
        file)
          printf '%s' "${latest_files[${index}]}"
          ;;
      esac
      return
    fi
  done
  printf '%s' "${default_value}"
}

canonical_check_profile() {
  local value
  value="$(slugify "$1")"
  case "${value}" in
    ""|terminal-baseline)
      printf 'terminal-baseline'
      ;;
    phase1-operator-surface)
      printf 'phase1-operator-surface'
      ;;
    prompt-input-delay|prompt-input-delay-pty)
      printf 'prompt-input-delay-pty'
      ;;
    *)
      printf '%s' "${value}"
      ;;
  esac
}

load_row_specs() {
  local selected_profile="$1"

  case "${selected_profile}" in
    terminal-baseline|phase1-operator-surface)
      row_specs=(
        "required|macos|terminal-app|zsh|inline|macOS / Terminal.app / zsh / inline"
        "required|macos|terminal-app|zsh|alternate|macOS / Terminal.app / zsh / alternate"
        "required|macos|iterm2|zsh|inline|macOS / iTerm2 / zsh / inline"
        "required|macos|iterm2|zsh|alternate|macOS / iTerm2 / zsh / alternate"
        "required|windows|windows-terminal|powershell|inline|Windows / Windows Terminal / PowerShell / inline"
        "required|windows|windows-terminal|powershell|alternate|Windows / Windows Terminal / PowerShell / alternate"
        "required|windows|windows-terminal|wsl-bash|inline|Windows / Windows Terminal / WSL bash / inline"
        "required|windows|windows-terminal|wsl-bash|alternate|Windows / Windows Terminal / WSL bash / alternate"
        "optional|windows|git-bash|bash|inline|Windows / Git Bash / bash / inline"
        "optional|windows|jetbrains-terminal|wsl-bash|inline|Windows / JetBrains IDE terminal / WSL bash / inline"
        "optional|windows|jetbrains-terminal|wsl-bash|alternate|Windows / JetBrains IDE terminal / WSL bash / alternate"
      )
      ;;
    prompt-input-delay-pty)
      row_specs=(
        "required|linux|linux-terminal|bash|inline|Linux / direct terminal / bash / inline"
        "required|linux|tmux|bash|inline|Linux / tmux detached PTY / bash / inline"
        "required|linux|zellij|bash|inline|Linux / Zellij / bash / inline"
        "required|windows|windows-terminal|powershell|inline|Windows / Windows Terminal / PowerShell / inline"
        "required|windows|windows-terminal|wsl-bash|inline|Windows / Windows Terminal / WSL bash / inline"
        "optional|macos|terminal-app|zsh|inline|macOS / Terminal.app / zsh / inline"
        "optional|macos|iterm2|zsh|inline|macOS / iTerm2 / zsh / inline"
        "optional|windows|jetbrains-terminal|wsl-bash|inline|Windows / JetBrains IDE terminal / WSL bash / inline"
        "optional|linux|vscode|bash|inline|Linux / VS Code integrated terminal / bash / inline"
      )
      ;;
    *)
      printf 'unsupported --check-profile: %s\n' "${selected_profile}" >&2
      exit 1
      ;;
  esac
}

records_dir="docs/validation"
check_profile="terminal-baseline"
output_format="text"
fail_on_incomplete=0

while (($# > 0)); do
  case "$1" in
    --records-dir)
      if [[ -z "${2-}" ]]; then
        printf 'missing value for %s\n' "$1" >&2
        exit 1
      fi
      records_dir="$2"
      shift 2
      ;;
    --check-profile)
      if [[ -z "${2-}" ]]; then
        printf 'missing value for %s\n' "$1" >&2
        exit 1
      fi
      check_profile="$(canonical_check_profile "$2")"
      shift 2
      ;;
    --format)
      if [[ -z "${2-}" ]]; then
        printf 'missing value for %s\n' "$1" >&2
        exit 1
      fi
      output_format="$2"
      shift 2
      ;;
    --fail-on-incomplete)
      fail_on_incomplete=1
      shift
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

case "${output_format}" in
  text|markdown)
    ;;
  *)
    printf 'unsupported format: %s\n' "${output_format}" >&2
    exit 1
    ;;
esac

declare -a row_specs=()
load_row_specs "${check_profile}"

declare -a row_keys=()
declare -a latest_keys=()
declare -a latest_results=()
declare -a latest_files=()
declare -a unmatched_entries=()

for spec in "${row_specs[@]}"; do
  IFS='|' read -r kind os terminal shell_name frontend label <<<"${spec}"
  key="$(row_key "${os}" "${terminal}" "${shell_name}" "${frontend}")"
  row_keys+=("${key}")
done

if [[ -d "${records_dir}" ]]; then
  while IFS= read -r record_file; do
    os_value="$(read_field "${record_file}" "os")"
    terminal_value="$(read_field "${record_file}" "terminal")"
    shell_value="$(read_field "${record_file}" "shell")"
    frontend_value="$(read_field "${record_file}" "frontend")"
    profile_value="$(read_field "${record_file}" "check_profile")"
    result_value="$(read_field "${record_file}" "result")"
    record_profile="$(canonical_check_profile "${profile_value:-terminal-baseline}")"

    if [[ "${record_profile}" != "${check_profile}" ]]; then
      continue
    fi

    key="$(row_key \
      "$(canonical_os_family "${os_value}")" \
      "$(canonical_terminal "${terminal_value}")" \
      "$(canonical_shell "${shell_value}")" \
      "$(canonical_frontend "${frontend_value}")")"

    if row_key_exists "${key}"; then
      set_latest_for_row "${key}" "$(slugify "${result_value}")" "${record_file}"
    else
      unmatched_entries+=("${record_file}|${os_value}|${terminal_value}|${shell_value}|${frontend_value}|${result_value}")
    fi
  done < <(find "${records_dir}" -maxdepth 1 -type f -name '*.txt' | sort)
fi

required_total=0
required_pass=0
required_missing=0
required_non_pass=0
optional_total=0
optional_pass=0

for spec in "${row_specs[@]}"; do
  IFS='|' read -r kind os terminal shell_name frontend label <<<"${spec}"
  key="$(row_key "${os}" "${terminal}" "${shell_name}" "${frontend}")"
  result="$(latest_row_value "${key}" result missing)"

  if [[ "${kind}" == "required" ]]; then
    ((required_total += 1))
    if [[ "${result}" == "pass" ]]; then
      ((required_pass += 1))
    elif [[ "${result}" == "missing" ]]; then
      ((required_missing += 1))
    else
      ((required_non_pass += 1))
    fi
  else
    ((optional_total += 1))
    if [[ "${result}" == "pass" ]]; then
      ((optional_pass += 1))
    fi
  fi
done

if [[ "${output_format}" == "text" ]]; then
  printf 'Native Validation Summary\n'
  printf 'records dir: %s\n' "${records_dir}"
  printf 'check profile: %s\n' "${check_profile}"
  printf '\n'

  printf 'Required Rows\n'
  for spec in "${row_specs[@]}"; do
    IFS='|' read -r kind os terminal shell_name frontend label <<<"${spec}"
    key="$(row_key "${os}" "${terminal}" "${shell_name}" "${frontend}")"
    result="$(latest_row_value "${key}" result missing)"
    source_file="$(latest_row_value "${key}" file "")"

    if [[ "${kind}" == "required" ]]; then
      if [[ -n "${source_file}" ]]; then
        printf -- '- %-8s %s (%s)\n' "${result}" "${label}" "${source_file}"
      else
        printf -- '- %-8s %s\n' "${result}" "${label}"
      fi
    fi
  done

  printf '\n'
  printf 'Optional Rows\n'
  for spec in "${row_specs[@]}"; do
    IFS='|' read -r kind os terminal shell_name frontend label <<<"${spec}"
    key="$(row_key "${os}" "${terminal}" "${shell_name}" "${frontend}")"
    result="$(latest_row_value "${key}" result missing)"
    source_file="$(latest_row_value "${key}" file "")"

    if [[ "${kind}" == "optional" ]]; then
      if [[ -n "${source_file}" ]]; then
        printf -- '- %-8s %s (%s)\n' "${result}" "${label}" "${source_file}"
      else
        printf -- '- %-8s %s\n' "${result}" "${label}"
      fi
    fi
  done

  printf '\n'
  printf 'Counts\n'
  printf -- '- required pass: %d/%d\n' "${required_pass}" "${required_total}"
  printf -- '- required missing: %d\n' "${required_missing}"
  printf -- '- required non-pass: %d\n' "${required_non_pass}"
  printf -- '- optional pass: %d/%d\n' "${optional_pass}" "${optional_total}"

  if ((${#unmatched_entries[@]} > 0)); then
    printf '\n'
    printf 'Unmatched Records\n'
    for entry in "${unmatched_entries[@]}"; do
      IFS='|' read -r file os_value terminal_value shell_value frontend_value result_value <<<"${entry}"
      printf -- '- %s (os=%s; terminal=%s; shell=%s; frontend=%s; result=%s)\n' \
        "${file}" "${os_value}" "${terminal_value}" "${shell_value}" "${frontend_value}" "${result_value}"
    done
  fi
else
  printf '# Native Validation Summary\n\n'
  printf -- '- records dir: `%s`\n' "${records_dir}"
  printf -- '- check profile: `%s`\n' "${check_profile}"
  printf -- '- required pass: `%d/%d`\n' "${required_pass}" "${required_total}"
  printf -- '- required missing: `%d`\n' "${required_missing}"
  printf -- '- required non-pass: `%d`\n' "${required_non_pass}"
  printf -- '- optional pass: `%d/%d`\n' "${optional_pass}" "${optional_total}"
  printf '\n'
  printf '## Required Rows\n\n'
  printf '| Status | Row | Record |\n'
  printf '| --- | --- | --- |\n'
  for spec in "${row_specs[@]}"; do
    IFS='|' read -r kind os terminal shell_name frontend label <<<"${spec}"
    key="$(row_key "${os}" "${terminal}" "${shell_name}" "${frontend}")"
    result="$(latest_row_value "${key}" result missing)"
    source_file="$(latest_row_value "${key}" file "")"
    if [[ "${kind}" == "required" ]]; then
      if [[ -n "${source_file}" ]]; then
        printf '| `%s` | %s | `%s` |\n' "${result}" "${label}" "${source_file}"
      else
        printf '| `%s` | %s | - |\n' "${result}" "${label}"
      fi
    fi
  done

  printf '\n'
  printf '## Optional Rows\n\n'
  printf '| Status | Row | Record |\n'
  printf '| --- | --- | --- |\n'
  for spec in "${row_specs[@]}"; do
    IFS='|' read -r kind os terminal shell_name frontend label <<<"${spec}"
    key="$(row_key "${os}" "${terminal}" "${shell_name}" "${frontend}")"
    result="$(latest_row_value "${key}" result missing)"
    source_file="$(latest_row_value "${key}" file "")"
    if [[ "${kind}" == "optional" ]]; then
      if [[ -n "${source_file}" ]]; then
        printf '| `%s` | %s | `%s` |\n' "${result}" "${label}" "${source_file}"
      else
        printf '| `%s` | %s | - |\n' "${result}" "${label}"
      fi
    fi
  done

  if ((${#unmatched_entries[@]} > 0)); then
    printf '\n'
    printf '## Unmatched Records\n\n'
    for entry in "${unmatched_entries[@]}"; do
      IFS='|' read -r file os_value terminal_value shell_value frontend_value result_value <<<"${entry}"
      printf -- '- `%s` (`os=%s`; `terminal=%s`; `shell=%s`; `frontend=%s`; `result=%s`)\n' \
        "${file}" "${os_value}" "${terminal_value}" "${shell_value}" "${frontend_value}" "${result_value}"
    done
  fi
fi

if ((fail_on_incomplete == 1)) && ((required_missing > 0 || required_non_pass > 0)); then
  exit 1
fi
