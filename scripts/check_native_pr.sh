#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if [[ -f "${HOME}/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  . "${HOME}/.cargo/env"
fi

case "${1:-}" in
  --changed)
    shift
    exec node scripts/agent-plan.mjs --run "$@"
    ;;
  --full)
    shift
    if [[ "$#" -ne 0 ]]; then
      printf 'The full gate does not accept selective-plan arguments.\n' >&2
      exit 2
    fi
    ;;
  "") ;;
  *)
    printf 'Usage: check_native_pr.sh [--full | --changed <agent-plan arguments>]\n' >&2
    exit 2
    ;;
esac

run_step() {
  local label="$1"
  shift

  printf '\n==> %s\n' "${label}"
  "$@"
}

run_step "TUI layering" bash scripts/check_tui_layering.sh
run_step "Node surfaces" bash scripts/check_node_surfaces.sh
run_step "Crate boundaries" node scripts/check_crate_boundaries.mjs
run_step "Rust formatting" cargo fmt --all -- --check
run_step "Rust tests" cargo test --workspace --locked
run_step "Native confirmation process" node scripts/check_native_confirmation_process.mjs
run_step "Rust clippy" cargo clippy --workspace --locked --all-targets --all-features -- -D warnings
