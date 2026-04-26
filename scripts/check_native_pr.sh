#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if [[ -f "${HOME}/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  . "${HOME}/.cargo/env"
fi

run_step() {
  local label="$1"
  shift

  printf '\n==> %s\n' "${label}"
  "$@"
}

run_step "TUI layering" bash scripts/check_tui_layering.sh
run_step "Rust formatting" cargo fmt --all -- --check
run_step "Rust tests" cargo test
run_step "Rust clippy" cargo clippy --all-targets --all-features -- -D warnings
