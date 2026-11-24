#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

target="x86_64-pc-windows-msvc"

# Keep this CI path under Git Bash: clean-host ACL fixtures depend on the launcher.
# Check validates production targets; the runner builds the lib test artifact only once.
cargo check --locked --all-features --lib --bins --target "${target}"

node scripts/run_native_rust_contracts.mjs --all-features --target "${target}" --timings -- \
  'process_liveness::tests::' \
  'trusted_executable::tests::native_' \
  'sqlite_planning_authority_adapter::tests::authority_store_' \
  'windows_plain_workspace_uses_private_authority_for_active_and_draft_flows' \
  'windows_plain_workspace_production_flow_initializes_edits_and_resets_private_authority' \
  'telegram_bot::tests::telegram_config_windows_' \
  'telegram_global_runner_lease::tests::' \
  'private_sqlite::tests::' \
  'stable_user_data_root::tests::' \
  'sqlite_issue_automation_store::private_store::tests::' \
  'adapter::outbound::vault_key_store::' \
  'application::service::evidence_vault_unlock::' \
  'trace_event_log::tests::exact_trace_file_rejects_windows_' \
  'trace_event_log::tests::windows_trace_file_and_rolling_directory_receive_private_owner_acl' \
  'actual_windows_environment_filter_accepts_mixed_case_allowlisted_keys' \
  'admin_api::security::tests::' \
  'admin_router_rejects_non_local_host_on_read_only_routes' \
  'admin_router_requires_capability_and_exact_loopback_authority' \
  'admin_login_exchanges_capability_for_strict_http_only_session_cookie' \
  'admin_json_mutations_require_header_csrf_and_share_reset_guard' \
  'application::service::parallel_mode::pool::paths::tests::' \
  'reconcile_provisions_missing_slots_into_idle_baselines' \
  'host_owned_worker_commit_'
