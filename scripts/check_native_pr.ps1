param(
    [ValidateSet("Full", "Rust", "Admin", "Portable")]
    [string]$Mode = "Full"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$windowsTarget = "x86_64-pc-windows-msvc"

function Invoke-ExternalStep {
    param(
        [string]$Label,
        [string]$FilePath,
        [string[]]$CommandArguments
    )

    Write-Host "`n==> $Label"
    & $FilePath @CommandArguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Label failed with exit code $LASTEXITCODE"
    }
}

function Test-TuiLayering {
    Write-Host "`n==> TUI layering"
    $appDirectory = Join-Path $repoRoot "src\adapter\inbound\tui\app"
    $rules = @(
        @{
            Description = "raw panel blocks belong in theme.rs"
            Pattern = 'Block::default\('
            Allowed = '[/\\]theme\.rs:'
        },
        @{
            Description = "raw selected/list highlight symbols belong in theme.rs"
            Pattern = '\.highlight_symbol\([^A]'
            Allowed = '[/\\]theme\.rs:'
        },
        @{
            Description = "raw brand/status colors belong in theme.rs"
            Pattern = 'Color::|\.bg\('
            Allowed = '[/\\](theme|history_insertion|shell_rendering_contract_tests)\.rs:'
        }
    )

    foreach ($rule in $rules) {
        $rawMatches = @(& rg -n --glob "*.rs" $rule.Pattern $appDirectory)
        $searchExit = $LASTEXITCODE
        if ($searchExit -gt 1) {
            throw "ripgrep failed while checking TUI layering"
        }
        $violations = @($rawMatches | Where-Object { $_ -notmatch $rule.Allowed })
        if ($violations.Count -gt 0) {
            $details = $violations -join "`n"
            throw "TUI layering violation: $($rule.Description)`n$details"
        }
    }
    Write-Host "tui layering check passed"
}

function Invoke-NodeAdminChecks {
    Invoke-ExternalStep "kernel skill matches spec" "node" @("scripts/generate-kernel-skill.mjs", "--check")
    Invoke-ExternalStep "validation tool argument checks" "node" @(
        "--test", "docs/plan/17-akra-desktop/scripts/client_count_validation.test.mjs"
    )
    Write-Host "`n==> repository Node script syntax"
    $scriptPaths = @(
        "scripts/capture_admin_graphic.mjs",
        "scripts/capture_admin_validation_evidence.mjs",
        "scripts/normalize_codex_app_server_schema.mjs",
        "scripts/agent-plan.mjs",
        "scripts/ci-scope.mjs",
        "npm/scripts/publish-package.mjs",
        "npm/scripts/stage-npm-packages.mjs",
        "npm/scripts/verify-native-release-assets.mjs",
        "npm/bin/akra.js"
    )
    foreach ($scriptPath in $scriptPaths) {
        Invoke-ExternalStep "node --check $scriptPath" "node" @("--check", $scriptPath)
    }

    Invoke-ExternalStep "PR validation approval package" "node" @(
        "--test",
        "scripts/pr-validation-approval-package.test.mjs"
    )

    Invoke-ExternalStep "Windows Vault manual launcher contracts" "node" @(
        "--test", "scripts/windows-vault-live.test.mjs"
    )

    Invoke-ExternalStep "admin browser behavior tests" "node" @(
        "--test", "assets/admin/scripts/*.test.mjs"
    )

    Invoke-ExternalStep "Windows-safe npm launcher tests" "node" @(
        "--test",
        "npm/test/bin.test.js",
        "npm/test/platform.test.js",
        "npm/test/runtime.test.js",
        "npm/test/schema-normalizer.test.js"
    )
    Invoke-ExternalStep "admin game dependencies" "npm" @("--prefix", "assets/admin/game", "ci")
    # High and critical advisories fail, except the dated exceptions in scripts/npm-audit-exceptions.json.
    Invoke-ExternalStep "admin game dependency audit" "node" @(
        "scripts/npm-audit-gate.mjs", "assets/admin/game"
    )
    Invoke-ExternalStep "admin game typecheck" "npm" @(
        "--prefix", "assets/admin/game", "run", "check"
    )

    $bundlePath = "assets/admin/game/akra-diorama.js"
    $bundleHashBefore = (& git hash-object $bundlePath).Trim()
    Invoke-ExternalStep "admin game reproducible build" "npm" @(
        "--prefix", "assets/admin/game", "run", "build"
    )
    $bundleHashAfter = (& git hash-object $bundlePath).Trim()
    if ($bundleHashBefore -ne $bundleHashAfter) {
        throw "admin game bundle was stale before the check"
    }

    # Type checks, unit tests, and bundling do not need the Electron runtime binary.
    $env:ELECTRON_SKIP_BINARY_DOWNLOAD = "1"
    try {
        Invoke-ExternalStep "desktop dependencies" "npm" @("--prefix", "desktop", "ci")
    } finally {
        Remove-Item Env:ELECTRON_SKIP_BINARY_DOWNLOAD -ErrorAction SilentlyContinue
    }
    # High and critical advisories fail, except the dated exceptions in scripts/npm-audit-exceptions.json.
    Invoke-ExternalStep "desktop dependency audit" "node" @(
        "scripts/npm-audit-gate.mjs", "desktop"
    )
    Invoke-ExternalStep "desktop typecheck" "npm" @("--prefix", "desktop", "run", "check")
    Invoke-ExternalStep "desktop tests" "npm" @("--prefix", "desktop", "test")
    Invoke-ExternalStep "desktop build" "npm" @("--prefix", "desktop", "run", "build")

    Write-Host "`n==> admin offline asset policy"
    $remoteAssets = @(& rg -n 'https?://|//(cdnjs|unpkg|esm\.sh|fastly\.jsdelivr)' `
        "templates/admin" "assets/admin/game/src" "assets/admin/scripts")
    $assetSearchExit = $LASTEXITCODE
    if ($assetSearchExit -eq 0) {
        throw "admin source contains a remote runtime asset reference`n$($remoteAssets -join "`n")"
    }
    if ($assetSearchExit -gt 1) {
        throw "ripgrep failed while checking the admin offline asset policy"
    }
}

function Invoke-WindowsPortableChecks {
    param(
        [bool]$IncludeCleanHostContracts
    )

    $commonTestArguments = @(
        "test", "--locked", "--all-features", "--lib", "--target", $windowsTarget
    )

    Invoke-ExternalStep "Windows native target" "cargo" @(
        "check", "--workspace", "--locked", "--all-features", "--lib", "--bins", "--target", $windowsTarget
    )

    $testFilters = @("process_liveness::tests::")
    if ($IncludeCleanHostContracts) {
        $testFilters += "trusted_executable::tests::native_"
    }
    $testFilters += @(
        "sqlite_planning_authority_adapter::tests::authority_store_",
        "windows_plain_workspace_uses_private_authority_for_active_and_draft_flows",
        "windows_plain_workspace_production_flow_initializes_edits_and_resets_private_authority",
        "telegram_bot::tests::telegram_config_windows_",
        "telegram_global_runner_lease::tests::",
        "private_sqlite::tests::",
        "private_fs::shared_directory_tests::",
        "adapter::outbound::issue_code_validator::windows::",
        "sqlite_issue_automation_restart_registry::tests::",
        "application::service::issue_coordinator::tests::autonomy::",
        "composition::issue_collection::restart_registry_tests::",
        "composition::repository_kernel::restart_tests::",
        "adapter::inbound::gateway::restart::tests::",
        "provider_credentials",
        "provider_work",
        "provider_http",
        "adapter::outbound::jira::issue_source::tests::",
        "adapter::outbound::jira::issue_feedback::tests::",
        "domain::issue_automation::source::tests::jira::",
        "domain::issue_automation::triage::tests::jira::",
        "application::service::issue_automation_policy::tests::jira",
        "application::service::issue_collection::tests::jira::",
        "sqlite_issue_source_claims::tests::",
        "application::service::issue_coordinator::tests::jira::",
        "application::service::issue_feedback::tests::jira::",
        "sqlite_issue_automation_store::feedback::tests::jira::",
        "adapter::inbound::kernel_protocol::issue_automation::triage::tests::",
        "application::port::outbound::planning_work_order_repository_port::tests::jira_",
        "application::service::parallel_mode::tests::pool::lease::malformed_jira_branch_keys_",
        "application::service::parallel_mode::distributor::delivery::github::tests::jira_pr_metadata_",
        "adapter::outbound::linear::",
        "application::service::issue_collection::tests::linear::",
        "application::service::issue_automation_policy::tests::linear::",
        "provider_key_store",
        "native_credential_prompt",
        "provider_credential_prompt",
        "provider_credential_migration",
        "private_ipc::credential",
        "private_ipc::owner",
        "filesystem::issue_publish_intent_journal::",
        "filesystem::issue_creation_journal::",
        "application::service::parallel_mode::tests::issue_automation_flow::",
        "filesystem::issue_template::",
        "issue_draft::tests::filesystem_template::",
        "gateway::tests::issue_shared_copy_flow::",
        "stable_user_data_root::tests::",
        "sqlite_issue_automation_store::private_store::tests::",
        "adapter::outbound::vault_key_store::",
        "application::service::evidence_vault_unlock::",
        "filesystem::vault_fs::",
        "filesystem::evidence_vault_store::",
        "filesystem::evidence_vault_backup::",
        "filesystem::vault_restore_workspace::",
        "adapter::outbound::native_vault_confirmation::",
        "application::port::outbound::vault_native_confirmation_port::",
        "adapter::inbound::desktop_restore_bridge::",
        "application::service::evidence_vault_restore::",
        "application::service::desktop_vault_restore::",
        "composition::desktop_restore::",
        "task_evidence_artifact::tests::",
        "sqlite_planning_authority_adapter::tests::task_evidence::",
        "domain::planning::verification::",
        "domain::kernel_event::task_verification::",
        "adapter::inbound::kernel_protocol::task_verification::",
        "kernel_events::tests::task_evidence_receipts_survive_ack_loss_retention_and_reject_changed_payload",
        "task_evidence_outbox::tests::host_verification_",
        "application::service::task_verification::tests::",
        "application::service::task_evidence::tests::verification::",
        "composition::native_client_runtime::task_verification_tests::",
        "composition::kernel_command_execution::tests::task_verification::",
        "composition::production::delivery_evidence::tests::",
        "domain::planning::delivery_evidence::",
        "application::service::task_delivery_evidence::tests::",
        "composition::kernel_host::simulation::issue_decision::human_confirmation::autonomy::",
        "sqlite_issue_automation_store::autonomy::tests::",
        "application::service::issue_coordinator::tests::auto_delivery::",
        "adapter::inbound::kernel_protocol::issue_autonomy::",
        "domain::issue_automation::arbiter::tests::arbiter_input_fingerprints_reject_missing_or_unbounded_provenance",
        "domain::issue_automation::arbiter::tests::arbiter_legacy_input_json_keeps_its_bytes_until_a_binding_generation_exists",
        "application::service::issue_binding_admission::tests::",
        "sqlite_issue_automation_store::binding_pause::tests::",
        "sqlite_issue_automation_store::admission::tests::binding_pause::",
        "sqlite_issue_automation_store::admission::tests::arbiter::binding_pause::",
        "sqlite_issue_automation_store::feedback::tests::binding_pause::",
        "application::service::issue_feedback::tests::binding_pause::",
        "application::service::issue_coordinator::tests::binding_pause::",
        "application::service::issue_intervention::tests::issue_intervention_reserves_internal_autonomy_and_binding_pause_command_ids",
        "sqlite_planning_authority_adapter::work_orders::tests::permits::action::",
        "adapter::outbound::issue_assessment_model::tests::denied_actual_dispatch_never_falls_back_to_an_unguarded_backend",
        "adapter::inbound::kernel_protocol::issue_binding_pause::",
        "composition::native_client_runtime::tests::binding_pause::",
        "composition::kernel_host::simulation::issue_decision::human_confirmation::binding_pause::",
        "kernel_events::tests::binding_pause_receipt_deduplicates_after_retention_and_rejects_changed_payload",
        "sqlite_planning_authority_adapter::work_orders::tests::permits::delivery_review::auto_delivery::",
        "sqlite_planning_authority_adapter::work_orders::tests::permits::delivery_evidence::",
        "subprocess::controlled::tests::",
        "trace_event_log::tests::exact_trace_file_rejects_windows_",
        "trace_event_log::tests::windows_trace_file_and_rolling_directory_receive_private_owner_acl",
        "actual_windows_environment_filter_accepts_mixed_case_allowlisted_keys",
        "admin_api::security::tests::",
        "admin_router_rejects_non_local_host_on_read_only_routes",
        "admin_router_requires_capability_and_exact_loopback_authority",
        "admin_login_exchanges_capability_for_strict_http_only_session_cookie",
        "admin_json_mutations_require_header_csrf_and_share_reset_guard",
        "application::service::parallel_mode::pool::paths::tests::"
    )
    if ($IncludeCleanHostContracts) {
        $testFilters += @(
            "reconcile_provisions_missing_slots_into_idle_baselines",
            "host_owned_worker_commit_",
            "application::service::parallel_mode::tests::pool::lease::jira_branch_",
            "application::service::parallel_mode::tests::distributor::work_permits::link_reference::jira_work_link_",
            "parallel_worker_commit::tree_snapshot::tests::",
            "configuration::tests::evidence::",
            "adapter::inbound::cli::tests::config_evidence_mode_",
            "application::service::parallel_mode::tests::distributor::evidence_gate::",
            "application::service::parallel_mode::tests::distributor::work_permits::evidence_conditions::",
            "adapter::outbound::github::automation::tests::frozen_integration::",
            "adapter::outbound::github::automation::tests::windows_temp::",
            "worker_evidence::",
            "parallel_mode::tests::pool::lease::task_evidence::",
            "trusted_executable::fingerprint::tests::",
            "adapter::outbound::git::task_verification::tests::",
            "parallel_mode::tests::pool::lease::task_verification::",
            "application::service::parallel_mode::orchestrator_loop::tests::automatic_verification::",
            "application::service::post_turn_evaluation::tests::automatic_verification::",
            "application::service::parallel_mode::tests::automatic_verification::"
        )
    }
    foreach ($testFilter in $testFilters) {
        Invoke-ExternalStep "Windows contract: $testFilter" "cargo" @(
            $commonTestArguments + @($testFilter, "--", "--nocapture")
        )
    }
}

Push-Location $repoRoot
try {
    if ($Mode -in @("Full", "Rust", "Admin")) {
        Invoke-ExternalStep "CI policy tests" "node" @(
            "--test",
            "scripts/ci-scope.test.mjs",
            "scripts/crate-boundaries.test.mjs",
            "scripts/agent-plan.test.mjs",
            "scripts/native-rust-contracts.test.mjs",
            "scripts/npm-audit-gate.test.mjs"
        )
    }
    if ($Mode -in @("Full", "Rust")) {
        Invoke-ExternalStep "Crate boundaries" "node" @("scripts/check_crate_boundaries.mjs")
        Test-TuiLayering
        Invoke-ExternalStep "Rust formatting" "cargo" @("fmt", "--all", "--", "--check")
    }
    if ($Mode -in @("Full", "Admin")) {
        Invoke-NodeAdminChecks
    }
    if ($Mode -in @("Full", "Rust", "Portable")) {
        Invoke-ExternalStep "Workspace member tests" "cargo" @("test", "--workspace", "--exclude", "akra-ade", "--locked", "--all-features", "--target", $windowsTarget)
        Invoke-WindowsPortableChecks -IncludeCleanHostContracts ($Mode -eq "Portable")
    }
    if ($Mode -in @("Full", "Rust")) {
        Invoke-ExternalStep "Rust clippy" "cargo" @(
            "clippy", "--workspace", "--locked", "--all-targets", "--all-features", "--", "-D", "warnings"
        )
    }
}
finally {
    Pop-Location
}

Write-Host "`nWindows native PR checks passed ($Mode mode)"
