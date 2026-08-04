#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

target="x86_64-pc-windows-msvc"

run_contract() {
  local filter="$1"
  printf '\n==> Windows contract: %s\n' "${filter}"
  cargo test --locked --all-features --lib --target "${target}" \
    "${filter}" -- --nocapture
}

# Keep this CI path under Git Bash. GitHub-hosted Windows gives native ACL fixtures
# different inherited ownership when the same commands are launched from pwsh.
cargo check --locked --all-features --lib --bins --target "${target}"

run_contract 'process_liveness::tests::'
run_contract 'trusted_executable::tests::native_'
run_contract 'sqlite_planning_authority_adapter::tests::authority_store_'
run_contract 'windows_plain_workspace_uses_private_authority_for_active_and_draft_flows'
run_contract 'windows_plain_workspace_production_flow_initializes_edits_and_resets_private_authority'
run_contract 'telegram_bot::tests::telegram_config_windows_'
run_contract 'telegram_global_runner_lease::tests::'
run_contract 'trace_event_log::tests::exact_trace_file_rejects_windows_'
run_contract 'trace_event_log::tests::windows_trace_file_and_rolling_directory_receive_private_owner_acl'
run_contract 'actual_windows_environment_filter_accepts_mixed_case_allowlisted_keys'
run_contract 'admin_api::security::tests::'
run_contract 'admin_router_rejects_non_local_host_on_read_only_routes'
run_contract 'admin_router_requires_capability_and_exact_loopback_authority'
run_contract 'admin_login_exchanges_capability_for_strict_http_only_session_cookie'
run_contract 'admin_json_mutations_require_header_csrf_and_share_reset_guard'
run_contract 'application::service::parallel_mode::pool::paths::tests::'
run_contract 'reconcile_provisions_missing_slots_into_idle_baselines'
run_contract 'host_owned_worker_commit_'
