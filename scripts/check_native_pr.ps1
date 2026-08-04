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
    Write-Host "`n==> repository Node script syntax"
    $scriptPaths = @(
        "scripts/capture_admin_graphic.mjs",
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

    Invoke-ExternalStep "Windows-safe npm launcher tests" "node" @(
        "--test",
        "npm/test/bin.test.js",
        "npm/test/platform.test.js",
        "npm/test/runtime.test.js",
        "npm/test/schema-normalizer.test.js"
    )
    Invoke-ExternalStep "admin game dependencies" "npm" @("--prefix", "assets/admin/game", "ci")
    Invoke-ExternalStep "admin game dependency audit" "npm" @(
        "--prefix", "assets/admin/game", "audit", "--audit-level=high"
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
        "check", "--locked", "--all-features", "--lib", "--bins", "--target", $windowsTarget
    )

    $testFilters = @(
        "process_liveness::tests::",
        "sqlite_planning_authority_adapter::tests::authority_store_",
        "windows_plain_workspace_uses_private_authority_for_active_and_draft_flows",
        "windows_plain_workspace_production_flow_initializes_edits_and_resets_private_authority",
        "telegram_bot::tests::telegram_config_windows_",
        "telegram_global_runner_lease::tests::",
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
        $testFilters = @(
            "trusted_executable::tests::native_",
            "reconcile_provisions_missing_slots_into_idle_baselines",
            "host_owned_worker_commit_"
        ) + $testFilters
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
            "scripts/agent-plan.test.mjs"
        )
    }
    if ($Mode -in @("Full", "Rust")) {
        Test-TuiLayering
        Invoke-ExternalStep "Rust formatting" "cargo" @("fmt", "--all", "--", "--check")
    }
    if ($Mode -in @("Full", "Admin")) {
        Invoke-NodeAdminChecks
    }
    if ($Mode -in @("Full", "Rust", "Portable")) {
        Invoke-WindowsPortableChecks -IncludeCleanHostContracts ($Mode -eq "Portable")
    }
    if ($Mode -in @("Full", "Rust")) {
        Invoke-ExternalStep "Rust clippy" "cargo" @(
            "clippy", "--locked", "--all-targets", "--all-features", "--", "-D", "warnings"
        )
    }
}
finally {
    Pop-Location
}

Write-Host "`nWindows native PR checks passed ($Mode mode)"
