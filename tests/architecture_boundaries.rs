use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy)]
struct TemporaryAllowance {
    path_suffix: &'static str,
    pattern: &'static str,
    reason: &'static str,
}

#[derive(Clone, Copy)]
struct BoundaryRule {
    name: &'static str,
    root: &'static str,
    forbidden_patterns: &'static [&'static str],
    temporary_allowances: &'static [TemporaryAllowance],
}

struct SourceLine {
    number: usize,
    text: String,
}

struct BoundaryViolation {
    rule: &'static str,
    path: String,
    line: usize,
    pattern: &'static str,
    text: String,
}

struct TemporaryDebt {
    path: String,
    line: usize,
    pattern: &'static str,
    text: String,
    reason: &'static str,
}

#[derive(Clone, Copy)]
struct PatternDebtRule {
    path_suffix: &'static str,
    pattern: &'static str,
    reason: &'static str,
}

const INBOUND_OUTBOUND_TEMPORARY_ALLOWANCES: &[TemporaryAllowance] = &[
    TemporaryAllowance {
        path_suffix: "src/adapter/inbound/cli.rs",
        pattern: "crate::adapter::outbound::",
        reason: "CLI still owns production wiring until application composition is centralized.",
    },
    TemporaryAllowance {
        path_suffix: "src/adapter/inbound/admin_api/mod.rs",
        pattern: "crate::adapter::outbound::",
        reason: "Admin API still owns production wiring until application composition is centralized.",
    },
    TemporaryAllowance {
        path_suffix: "src/adapter/inbound/telegram_bot/mod.rs",
        pattern: "crate::adapter::outbound::",
        reason: "Telegram bot still owns production wiring until application composition is centralized.",
    },
    TemporaryAllowance {
        path_suffix: "src/adapter/inbound/tui/app/shell_entrypoint.rs",
        pattern: "crate::adapter::outbound::",
        reason: "Native TUI shell entrypoint is the current production wiring boundary.",
    },
    TemporaryAllowance {
        path_suffix: "src/adapter/inbound/tui/app/github_polling.rs",
        pattern: "crate::adapter::outbound::",
        reason: "GitHub review polling still builds its adapter from the TUI edge.",
    },
];

const PARALLEL_CONTROL_PLANE_HOST_EVENT_LOOP_DEBTS: &[PatternDebtRule] = &[
    PatternDebtRule {
        path_suffix: "src/application/service/parallel_mode/control_plane/host.rs",
        pattern: "Mutex<ParallelModeControlPlaneService",
        reason: "control-plane host still protects a mutable service with a mutex instead of owning a mailbox/event loop.",
    },
    PatternDebtRule {
        path_suffix: "src/application/service/parallel_mode/control_plane/host.rs",
        pattern: "MutexGuard<'_, ParallelModeControlPlaneService",
        reason: "handle calls still borrow the raw service synchronously instead of enqueueing commands.",
    },
    PatternDebtRule {
        path_suffix: "src/application/service/parallel_mode/control_plane/host.rs",
        pattern: ".lock()",
        reason: "command processing is still caller-thread locking, not single loop ownership.",
    },
];

const PARALLEL_CONTROL_PLANE_BYPASS_DEBTS: &[PatternDebtRule] = &[
    PatternDebtRule {
        path_suffix: "src/application/service/parallel_mode/control_plane/composition.rs",
        pattern: "pub fn parallel_mode_service(",
        reason: "composition still exposes the raw ParallelModeService, allowing callers to bypass the control-plane handle.",
    },
    PatternDebtRule {
        path_suffix: "src/application/service/parallel_mode/control_plane/composition.rs",
        pattern: ".run_orchestrator_tick(",
        reason: "manual orchestrator ticks still call ParallelModeService directly instead of enqueueing a control-plane command.",
    },
    PatternDebtRule {
        path_suffix: "src/application/service/parallel_mode/control_plane/controller.rs",
        pattern: ".has_actionable_queue_head(",
        reason: "controller still queries queue state while building a command instead of receiving all state through loop events/effects.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app/app_runtime.rs",
        pattern: "parallel_mode_service: composition.parallel_mode_service().clone()",
        reason: "TUI runtime still receives a raw ParallelModeService clone alongside the control-plane binding.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app/parallel_mode.rs",
        pattern: "fn parallel_mode_service(",
        reason: "TUI parallel binding still exposes raw service access, which should disappear behind the event-loop handle.",
    },
];

const TUI_RAW_APPLICATION_SERVICE_DEBTS: &[PatternDebtRule] = &[
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app.rs",
        pattern: "startup_service: StartupService,",
        reason: "NativeTuiApp still owns a raw startup application service instead of an application-facing handle.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app.rs",
        pattern: "session_service: SessionService,",
        reason: "NativeTuiApp still owns a raw session application service instead of an application-facing handle.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app.rs",
        pattern: "conversation_service: ConversationService,",
        reason: "NativeTuiApp still owns a raw conversation application service instead of an application-facing handle.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app.rs",
        pattern: "parallel_mode_service: ParallelModeService,",
        reason: "NativeTuiApp still owns a raw parallel application service instead of an application-facing handle.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app.rs",
        pattern: "planning: PlanningServices,",
        reason: "NativeTuiApp still owns raw planning services instead of a narrow application-facing handle.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app/app_runtime.rs",
        pattern: "parallel_mode_service: ParallelModeService,",
        reason: "TUI runtime dependencies still carry raw parallel service wiring.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app/app_runtime.rs",
        pattern: "planning: PlanningServices,",
        reason: "TUI runtime dependencies still carry raw planning service wiring.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs",
        pattern: "planning: PlanningServices,",
        reason: "TUI post-turn execution still owns raw planning service wiring.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app/turn_submission_runtime/stream_execution.rs",
        pattern: "service: ConversationService,",
        reason: "TUI stream execution still owns raw conversation service wiring.",
    },
];

const TUI_POST_TURN_PLANNING_BRIDGE_FORBIDDEN_PATTERNS: &[&str] = &[
    "PlanningLedgerRepairRequest",
    "PlanningOfficialCompletionRefreshRequest",
    "PlanningProposalPromotionRequest",
    "PlanningQueueRefreshRequest",
    "PlanningRuntimeWorkspaceStatus",
    "QueueIdlePolicy",
    ".promote_top_proposal_to_ready_if_needed(",
    ".refresh_queue_from_official_completion(",
    ".refresh_queue_from_reply(",
    ".render_official_completion_refresh_prompt(",
    ".render_refresh_queue_prompt(",
    ".render_repair_task_authority_prompt(",
    ".repair_task_authority(",
];

#[test]
fn domain_layer_has_no_application_or_adapter_dependencies() {
    assert_no_forbidden_references(BoundaryRule {
        name: "domain must stay independent from application and adapters",
        root: "src/domain",
        forbidden_patterns: &["crate::application::", "crate::adapter::"],
        temporary_allowances: &[],
    });
}

#[test]
fn domain_layer_has_no_runtime_ui_or_io_dependencies() {
    assert_no_forbidden_references(BoundaryRule {
        name: "domain must stay pure from runtime, UI, and IO infrastructure",
        root: "src/domain",
        forbidden_patterns: &[
            "std::thread",
            "std::sync::mpsc",
            "tokio::",
            "ratatui",
            "crossterm",
            "std::fs",
            "std::process",
            "Command::new",
        ],
        temporary_allowances: &[],
    });
}

#[test]
fn application_layer_has_no_concrete_adapter_dependencies() {
    assert_no_forbidden_references(BoundaryRule {
        name: "application must depend on ports and domain, not concrete adapters",
        root: "src/application",
        forbidden_patterns: &["crate::adapter::"],
        temporary_allowances: &[],
    });
}

#[test]
fn application_layer_has_no_ui_framework_dependencies() {
    assert_no_forbidden_references(BoundaryRule {
        name: "application must not depend on TUI framework details",
        root: "src/application",
        forbidden_patterns: &[
            "ratatui",
            "crossterm",
            "crate::adapter::inbound::tui",
            "crate::adapter::inbound::admin_api",
            "crate::adapter::inbound::telegram_bot",
        ],
        temporary_allowances: &[],
    });
}

#[test]
fn inbound_adapters_only_wire_outbound_adapters_in_explicit_composition_roots() {
    assert_no_forbidden_references(inbound_outbound_boundary_rule());
}

#[test]
fn temporary_inbound_composition_debt_has_been_removed() {
    let debts = collect_temporarily_allowed_references(inbound_outbound_boundary_rule());

    assert!(
        debts.is_empty(),
        "temporary architecture debt remains. This failure is intentional until composition wiring is moved out of inbound adapters:\n{}",
        format_temporary_debts(&debts)
    );
}

#[test]
fn inbound_adapters_do_not_mutate_parallel_durable_state_directly() {
    assert_no_forbidden_references(BoundaryRule {
        name: "inbound adapters must not directly mutate parallel durable/runtime state",
        root: "src/adapter/inbound",
        forbidden_patterns: &[
            "abandon_next_official_refresh_order",
            "acquire_official_refresh_claim",
            "apply_parallel_pool_reset_report",
            "claim_next_dispatch_command",
            "cancel_runtime_dispatch_commands",
            "clear_parallel_runtime_projections",
            "clear_parallel_runtime_projections_for_tasks",
            "enqueue_runtime_dispatch_command",
            "mark_workspace_slot_running",
            "release_distributor_queue_claim",
            "release_official_refresh_claim",
            "release_workspace_slot_lease_after_failed_start",
            "reserve_next_official_refresh_order",
            "try_acquire_distributor_queue_claim",
            "try_claim_next_runtime_dispatch_command",
            "update_runtime_dispatch_command",
            "upsert_runtime_distributor_queue_record",
            "upsert_runtime_session_detail",
            "upsert_runtime_slot_lease",
            "upsert_runtime_task_dispatch_block",
            "write_slot_lease",
            "remove_slot_lease",
            "cleanup_slot(",
            "reset_slot_worktree_to_akra",
            "build_dispatch_plan(",
        ],
        temporary_allowances: &[],
    });
}

#[test]
fn tui_post_turn_execution_uses_planning_post_turn_facade() {
    assert_no_forbidden_references_in_paths(
        "TUI post-turn execution must call planning post-turn facade DTOs instead of composing low-level planning workflow",
        &[
            "src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs",
            "src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution",
        ],
        TUI_POST_TURN_PLANNING_BRIDGE_FORBIDDEN_PATTERNS,
    );
}

#[test]
fn outbound_adapters_do_not_depend_on_inbound_adapters() {
    assert_no_forbidden_references(BoundaryRule {
        name: "outbound adapters must not depend on inbound adapters",
        root: "src/adapter/outbound",
        forbidden_patterns: &["crate::adapter::inbound::"],
        temporary_allowances: &[],
    });
}

#[test]
fn outbound_port_modules_follow_port_naming_contract() {
    let repo_root = repo_root();
    let port_root = repo_root.join("src/application/port/outbound");
    let mut violations = Vec::new();

    for path in rust_files_under(&port_root) {
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if file_name == "mod.rs" || is_test_only_path(&path) {
            continue;
        }
        if !file_name.ends_with("_port.rs") {
            violations.push(format!(
                "{}: outbound port modules must use the *_port.rs suffix",
                relative_path(&repo_root, &path)
            ));
            continue;
        }

        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        if !source.contains("trait ") || !source.contains("Port") {
            violations.push(format!(
                "{}: outbound port module must define a Port trait contract",
                relative_path(&repo_root, &path)
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "outbound port naming contract violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn temporary_tui_raw_application_services_have_been_wrapped() {
    let debts = collect_pattern_debts(TUI_RAW_APPLICATION_SERVICE_DEBTS);

    assert!(
        debts.is_empty(),
        "temporary TUI service-wiring debt remains. TUI production state should hold UI state, projection cache, and narrow application handles only:\n{}",
        format_temporary_debts(&debts)
    );
}

#[test]
fn temporary_parallel_control_plane_host_has_been_moved_to_mailbox_event_loop() {
    let debts = collect_pattern_debts(PARALLEL_CONTROL_PLANE_HOST_EVENT_LOOP_DEBTS);

    assert!(
        debts.is_empty(),
        "temporary parallel event-loop debt remains. Control-plane host should become a mailbox-owned event loop:\n{}",
        format_temporary_debts(&debts)
    );
}

#[test]
fn temporary_parallel_runtime_store_has_been_made_private_to_single_writer() {
    let debts = collect_public_fields_in_struct(
        "src/application/service/parallel_mode/control_plane/mod.rs",
        "ParallelModeControlPlaneRuntimeStore",
        "runtime store fields are still public; single-writer event-loop state should be private and mutated only by the loop/runtime.",
    );

    assert!(
        debts.is_empty(),
        "temporary parallel single-writer debt remains. Runtime store should not expose mutable state shape as public fields:\n{}",
        format_temporary_debts(&debts)
    );
}

#[test]
fn temporary_parallel_control_surfaces_no_longer_bypass_event_loop() {
    let debts = collect_pattern_debts(PARALLEL_CONTROL_PLANE_BYPASS_DEBTS);

    assert!(
        debts.is_empty(),
        "temporary parallel control-surface debt remains. Parallel control should enter through commands and loop-owned effects only:\n{}",
        format_temporary_debts(&debts)
    );
}

fn assert_no_forbidden_references(rule: BoundaryRule) {
    let repo_root = repo_root();
    let root = repo_root.join(rule.root);
    let mut violations = Vec::new();

    for path in rust_files_under(&root) {
        if is_test_only_path(&path) {
            continue;
        }

        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        let relative_path = relative_path(&repo_root, &path);

        for source_line in production_lines(&source) {
            if is_comment_only_line(&source_line.text) {
                continue;
            }

            for pattern in rule.forbidden_patterns {
                if source_line.text.contains(pattern)
                    && !is_temporarily_allowed(rule, &relative_path, pattern)
                {
                    violations.push(BoundaryViolation {
                        rule: rule.name,
                        path: relative_path.clone(),
                        line: source_line.number,
                        pattern,
                        text: source_line.text.trim().to_string(),
                    });
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "architecture boundary violations:\n{}",
        format_violations(&violations)
    );
}

fn assert_no_forbidden_references_in_paths(
    rule_name: &'static str,
    path_suffixes: &[&str],
    forbidden_patterns: &[&'static str],
) {
    let repo_root = repo_root();
    let mut violations = Vec::new();

    for path_suffix in path_suffixes {
        let root = repo_root.join(path_suffix);
        for path in rust_files_for_path(&root) {
            if is_test_only_path(&path) {
                continue;
            }

            let source = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!("failed to read {}: {error}", path.display());
            });
            let relative_path = relative_path(&repo_root, &path);

            for source_line in production_lines(&source) {
                if is_comment_only_line(&source_line.text) {
                    continue;
                }

                for pattern in forbidden_patterns {
                    if source_line.text.contains(pattern) {
                        violations.push(BoundaryViolation {
                            rule: rule_name,
                            path: relative_path.clone(),
                            line: source_line.number,
                            pattern,
                            text: source_line.text.trim().to_string(),
                        });
                    }
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "architecture boundary violations:\n{}",
        format_violations(&violations)
    );
}

fn inbound_outbound_boundary_rule() -> BoundaryRule {
    BoundaryRule {
        name: "inbound adapters must not pull outbound adapters outside explicit composition roots",
        root: "src/adapter/inbound",
        forbidden_patterns: &["crate::adapter::outbound::"],
        temporary_allowances: INBOUND_OUTBOUND_TEMPORARY_ALLOWANCES,
    }
}

fn is_temporarily_allowed(rule: BoundaryRule, path: &str, pattern: &str) -> bool {
    temporary_allowance_for(rule, path, pattern).is_some()
}

fn temporary_allowance_for(
    rule: BoundaryRule,
    path: &str,
    pattern: &str,
) -> Option<TemporaryAllowance> {
    rule.temporary_allowances
        .iter()
        .copied()
        .find(|allowance| path.ends_with(allowance.path_suffix) && pattern == allowance.pattern)
}

fn collect_temporarily_allowed_references(rule: BoundaryRule) -> Vec<TemporaryDebt> {
    let repo_root = repo_root();
    let root = repo_root.join(rule.root);
    let mut debts = Vec::new();

    for path in rust_files_under(&root) {
        if is_test_only_path(&path) {
            continue;
        }

        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        let relative_path = relative_path(&repo_root, &path);

        for source_line in production_lines(&source) {
            if is_comment_only_line(&source_line.text) {
                continue;
            }

            for pattern in rule.forbidden_patterns {
                if source_line.text.contains(pattern)
                    && let Some(allowance) = temporary_allowance_for(rule, &relative_path, pattern)
                {
                    debts.push(TemporaryDebt {
                        path: relative_path.clone(),
                        line: source_line.number,
                        pattern,
                        text: source_line.text.trim().to_string(),
                        reason: allowance.reason,
                    });
                }
            }
        }
    }

    debts
}

fn collect_pattern_debts(rules: &[PatternDebtRule]) -> Vec<TemporaryDebt> {
    let repo_root = repo_root();
    let mut debts = Vec::new();

    for rule in rules {
        let path = repo_root.join(rule.path_suffix);
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        let relative_path = relative_path(&repo_root, &path);

        for source_line in production_lines(&source) {
            if is_comment_only_line(&source_line.text) {
                continue;
            }
            if source_line.text.contains(rule.pattern) {
                debts.push(TemporaryDebt {
                    path: relative_path.clone(),
                    line: source_line.number,
                    pattern: rule.pattern,
                    text: source_line.text.trim().to_string(),
                    reason: rule.reason,
                });
            }
        }
    }

    debts
}

fn collect_public_fields_in_struct(
    path_suffix: &'static str,
    struct_name: &'static str,
    reason: &'static str,
) -> Vec<TemporaryDebt> {
    let repo_root = repo_root();
    let path = repo_root.join(path_suffix);
    let source = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    });
    let relative_path = relative_path(&repo_root, &path);
    let mut debts = Vec::new();
    let mut inside_struct = false;
    let mut brace_depth = 0isize;
    let struct_marker = format!("struct {struct_name}");

    for source_line in production_lines(&source) {
        if is_comment_only_line(&source_line.text) {
            continue;
        }

        let trimmed = source_line.text.trim();
        if !inside_struct {
            if trimmed.contains(&struct_marker) {
                inside_struct = true;
                brace_depth = brace_delta(&source_line.text);
            }
            continue;
        }

        if (trimmed.starts_with("pub ") || trimmed.starts_with("pub(")) && trimmed.contains(':') {
            debts.push(TemporaryDebt {
                path: relative_path.clone(),
                line: source_line.number,
                pattern: "pub... <field>:",
                text: trimmed.to_string(),
                reason,
            });
        }

        brace_depth += brace_delta(&source_line.text);
        if brace_depth <= 0 {
            break;
        }
    }

    debts
}

fn format_violations(violations: &[BoundaryViolation]) -> String {
    violations
        .iter()
        .map(|violation| {
            format!(
                "{}:{}: {} matched `{}` in rule `{}`",
                violation.path, violation.line, violation.text, violation.pattern, violation.rule
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_temporary_debts(debts: &[TemporaryDebt]) -> String {
    debts
        .iter()
        .map(|debt| {
            format!(
                "{}:{}: {} matched `{}`; reason: {}",
                debt.path, debt.line, debt.text, debt.pattern, debt.reason
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn production_lines(source: &str) -> Vec<SourceLine> {
    let mut lines = Vec::new();
    let mut skip_cfg_test_item = false;
    let mut skipping_cfg_test_block = false;
    let mut skipping_block_comment = false;
    let mut cfg_test_block_depth = 0isize;

    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();

        if skipping_block_comment {
            if trimmed.contains("*/") {
                skipping_block_comment = false;
            }
            continue;
        }

        if trimmed.starts_with("/*") {
            if !trimmed.contains("*/") {
                skipping_block_comment = true;
            }
            continue;
        }

        if skipping_cfg_test_block {
            cfg_test_block_depth += brace_delta(line);
            if cfg_test_block_depth <= 0 {
                skipping_cfg_test_block = false;
                cfg_test_block_depth = 0;
            }
            continue;
        }

        if skip_cfg_test_item {
            if trimmed.is_empty() || trimmed.starts_with("#[") {
                continue;
            }

            if line.contains('{') {
                skipping_cfg_test_block = true;
                cfg_test_block_depth = brace_delta(line);
                if cfg_test_block_depth <= 0 {
                    skipping_cfg_test_block = false;
                    cfg_test_block_depth = 0;
                }
            } else if trimmed.ends_with(';') {
                skip_cfg_test_item = false;
            }
            continue;
        }

        if is_cfg_test_attribute(trimmed) {
            skip_cfg_test_item = true;
            continue;
        }

        lines.push(SourceLine {
            number: index + 1,
            text: line.to_string(),
        });
    }

    lines
}

fn is_cfg_test_attribute(line: &str) -> bool {
    line.starts_with("#[cfg(test")
        || line.starts_with("#[cfg_attr(test")
        || line.contains("cfg(test)")
}

fn brace_delta(line: &str) -> isize {
    line.chars()
        .fold(0isize, |depth, character| match character {
            '{' => depth + 1,
            '}' => depth - 1,
            _ => depth,
        })
}

fn is_comment_only_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*')
}

fn rust_files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files.sort();
    files
}

fn rust_files_for_path(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        if root.extension().and_then(|value| value.to_str()) == Some("rs") {
            return vec![root.to_path_buf()];
        }
        return Vec::new();
    }
    rust_files_under(root)
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    if !path.exists() {
        return;
    }

    if path.is_file() {
        if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path.to_path_buf());
        }
        return;
    }

    for entry in fs::read_dir(path).unwrap_or_else(|error| {
        panic!("failed to read directory {}: {error}", path.display());
    }) {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "failed to read directory entry in {}: {error}",
                path.display()
            );
        });
        collect_rust_files(&entry.path(), files);
    }
}

fn is_test_only_path(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    file_name == "tests.rs"
        || file_name == "test_helpers.rs"
        || file_name == "fixtures.rs"
        || file_name == "tui_testkit.rs"
        || file_name == "contract_tests.rs"
        || file_name.ends_with("_tests.rs")
        || path.components().any(|component| match component {
            Component::Normal(value) => {
                value == "tests"
                    || value == "snapshots"
                    || value == "fixtures"
                    || value.to_string_lossy().ends_with("_tests")
            }
            _ => false,
        })
}

fn relative_path(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
