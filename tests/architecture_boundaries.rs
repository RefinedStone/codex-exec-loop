use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy)]
struct BoundaryRule {
    name: &'static str,
    root: &'static str,
    forbidden_patterns: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct AllowedCrateReferenceRule {
    name: &'static str,
    root: &'static str,
    allowed_prefixes: &'static [&'static str],
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

const PARALLEL_CONTROL_PLANE_BYPASS_DEBTS: &[PatternDebtRule] = &[
    PatternDebtRule {
        path_suffix: "src/application/service/parallel_mode/control_plane/composition.rs",
        pattern: "pub fn parallel_mode_service(",
        reason: "composition still exposes the raw ParallelModeService, allowing callers to bypass the control-plane gate.",
    },
    PatternDebtRule {
        path_suffix: "src/application/service/parallel_mode/control_plane/composition.rs",
        pattern: ".run_orchestrator_tick(",
        reason: "manual orchestrator ticks still call ParallelModeService directly instead of entering through a control-plane command.",
    },
    PatternDebtRule {
        path_suffix: "src/application/service/parallel_mode/control_plane/controller.rs",
        pattern: ".has_actionable_queue_head(",
        reason: "controller still queries queue state while building a command instead of receiving all state through control-plane effects.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app/app_runtime.rs",
        pattern: "parallel_mode_service: composition.parallel_mode_service().clone()",
        reason: "TUI runtime still receives a raw ParallelModeService clone alongside the control-plane binding.",
    },
    PatternDebtRule {
        path_suffix: "src/adapter/inbound/tui/app/parallel_mode.rs",
        pattern: "fn parallel_mode_service(",
        reason: "TUI parallel binding still exposes raw service access, which should disappear behind the control-plane handle.",
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
];

const TUI_POST_TURN_PLANNING_BRIDGE_FORBIDDEN_PATTERNS: &[&str] = &[
    "PlanningLedgerRepairRequest",
    "PlanningOfficialCompletionRefreshRequest",
    "PlanningProposalPromotionRequest",
    "PlanningQueueRefreshRequest",
    "PlanningRuntimeWorkspaceStatus",
    "QueueIdlePolicy::",
    ".promote_top_proposal_to_ready_if_needed(",
    ".refresh_queue_from_official_completion(",
    ".refresh_queue_from_reply(",
    ".render_official_completion_refresh_prompt(",
    ".render_refresh_queue_prompt(",
    ".render_repair_task_authority_prompt(",
    ".repair_task_authority(",
];

const CORE_APP_APPLICATION_CONTRACT_FORBIDDEN_PATTERNS: &[&str] = &[
    "crate::application::",
    "ConversationStreamEvent",
    "ManualPromptPreparationRequest",
    "ManualPromptPreparationResult",
    "ParallelTurnSlotLeaseHandoff",
    "PlanningRuntimeProjection",
    "PlanningTurnExecutionSnapshotCapture",
    "PostTurnEvaluationExecution",
    "PostTurnEvaluationRequest",
];

const CORE_RUNTIME_RAW_APPLICATION_SERVICE_FORBIDDEN_PATTERNS: &[&str] = &[
    "use crate::application::service::",
    "crate::application::service::",
    "StartupService",
    "SessionService",
    "ConversationService",
    "PlanningServices",
    "PlanningRuntimeUseCases",
    "ParallelModeTurnService",
    "ManualPromptPreparationService",
    "PostTurnEvaluationService",
    "ParallelTurnStreamLaunchRequest",
    "PlanningTurnExecutionSnapshotCaptureRequest",
    ".run_checks(",
    ".load_session_catalog(",
    ".load_snapshot(",
    ".prepare_stream_launch(",
    ".capture_execution_snapshot(",
    ".start_stream(",
    ".prepare(",
    ".evaluate_with_timeout(",
];

#[test]
fn domain_layer_has_no_application_core_or_adapter_dependencies() {
    // Static guard: dependency direction is a source graph property, not a runtime behavior.
    assert_no_forbidden_references(BoundaryRule {
        name: "domain must stay independent from application, core, and adapters",
        root: "src/domain",
        forbidden_patterns: &["crate::application::", "crate::core::", "crate::adapter::"],
    });
}

#[test]
fn domain_layer_has_no_runtime_ui_or_io_dependencies() {
    // Static guard: pure domain code must not gain runtime/framework imports even if behavior tests still pass.
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
    });
}

#[test]
fn application_layer_has_no_concrete_adapter_dependencies() {
    // Static guard: application may depend on ports, but concrete adapter imports are architectural leaks.
    assert_no_forbidden_references(BoundaryRule {
        name: "application must depend on ports and domain, not concrete adapters",
        root: "src/application",
        forbidden_patterns: &["crate::adapter::"],
    });
}

#[test]
fn application_layer_has_no_core_runtime_dependencies() {
    // Static guard: core coordinates application services; application code must not call back into core.
    assert_no_forbidden_references(BoundaryRule {
        name: "application must not depend on the core app runtime boundary",
        root: "src/application",
        forbidden_patterns: &["crate::core::"],
    });
}

#[test]
fn application_layer_has_no_ui_framework_dependencies() {
    // Static guard: UI framework imports in application compile cleanly but invert the hexagonal boundary.
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
    });
}

#[test]
fn core_layer_only_depends_on_application_domain_and_core_modules() {
    // Static guard: core sits above application/domain and below inbound adapters. Any crate-local
    // dependency outside these prefixes is a new boundary that should be designed explicitly first.
    assert_only_allowed_crate_references(AllowedCrateReferenceRule {
        name: "core may only reference application, domain, and core modules",
        root: "src/core",
        allowed_prefixes: &["crate::application::", "crate::core::", "crate::domain::"],
    });
}

#[test]
#[ignore]
fn future_core_layer_is_application_independent() {
    /*
     * Disabled target: if core becomes the innermost app/kernel boundary instead
     * of a headless coordinator, application service types should move behind
     * core-owned ports/facades and disappear from the core source graph.
     */
    assert_no_forbidden_references(BoundaryRule {
        name: "future core layer must not depend directly on application modules",
        root: "src/core",
        forbidden_patterns: &["crate::application::"],
    });
}

#[test]
#[ignore]
fn future_core_app_contracts_are_application_dto_free() {
    /*
     * Disabled target: core/app should eventually expose core-owned commands,
     * effects, events, and snapshots instead of leaking application service DTOs
     * into the public headless runtime contract.
     */
    assert_no_forbidden_references(BoundaryRule {
        name: "future core app contracts must not depend on application DTOs",
        root: "src/core/app",
        forbidden_patterns: &["crate::application::"],
    });
}

#[test]
#[ignore]
fn future_core_app_public_contracts_are_core_owned() {
    /*
     * Disabled target: this is the strict version of the core/app boundary. Core
     * commands, effects, inputs, events, stream snapshots, and app snapshots
     * should be owned by core/domain contracts instead of reusing application
     * request/result/projection DTOs in their public shape.
     */
    assert_no_forbidden_references_in_paths(
        "future core/app public contracts must be core-owned and application DTO free",
        &[
            "src/core/app/command.rs",
            "src/core/app/effect.rs",
            "src/core/app/event.rs",
            "src/core/app/projection.rs",
            "src/core/app/snapshot.rs",
            "src/core/app/state.rs",
            "src/core/app/turn_stream.rs",
            "src/core/app/turn_submission.rs",
        ],
        CORE_APP_APPLICATION_CONTRACT_FORBIDDEN_PATTERNS,
    );
}

#[test]
fn core_layer_has_no_ui_transport_or_concrete_adapter_dependencies() {
    // Static guard: core is a headless application runtime. It may coordinate application services,
    // but it must not become a TUI, HTTP, Telegram, or concrete outbound adapter layer.
    assert_no_forbidden_references(BoundaryRule {
        name: "core must stay headless and depend on application contracts instead of adapters",
        root: "src/core",
        forbidden_patterns: &[
            "ratatui",
            "crossterm",
            "axum",
            "askama",
            "rusqlite",
            "tower",
            "which",
            "serde_json",
            "tracing_appender",
            "crate::adapter::inbound",
            "crate::adapter::outbound",
            "crate::composition::",
            "crate::diagnostics",
            "crate::subprocess",
            "crate::test_utils",
            "telegram_bot",
        ],
    });
}

#[test]
fn core_app_layer_has_no_effect_execution_dependencies() {
    // Static guard: core/app owns commands, events, snapshots, and reducer state only. Service
    // execution, threads, channels, ports, and IO belong in core/runtime or lower application ports.
    assert_no_forbidden_references(BoundaryRule {
        name: "core app must stay a pure contract and reducer layer",
        root: "src/core/app",
        forbidden_patterns: &[
            "std::thread",
            "thread::spawn",
            "std::sync::mpsc",
            "mpsc::",
            "tokio::",
            "std::fs",
            "std::process",
            "Command::new",
            "crate::core::runtime::",
            "CoreRuntime",
            "CoreEffectRunner",
            "CoreEffectExecutor",
            "StartupService",
            "SessionService",
            "ConversationService",
            "PlanningServices",
            "PlanningRuntimeUseCases",
            "ParallelModeTurnService",
            "ManualPromptPreparationService",
            "PostTurnEvaluationService",
            "crate::application::port::",
        ],
    });
}

#[test]
#[ignore]
fn future_core_runtime_does_not_hold_raw_application_services() {
    /*
     * Disabled target: core/runtime may keep the effect boundary, but the concrete
     * field set should collapse behind a narrow application-facing facade before
     * this becomes an always-on architecture gate.
     */
    assert_no_forbidden_references(BoundaryRule {
        name: "future core runtime must depend on a narrow application facade instead of raw services",
        root: "src/core/runtime",
        forbidden_patterns: &[
            "startup_service: StartupService",
            "session_service: SessionService",
            "conversation_service: ConversationService",
            "planning_runtime: PlanningRuntimeUseCases",
            "parallel_mode_turn_service: ParallelModeTurnService",
            "manual_prompt_preparation_service: ManualPromptPreparationService",
            "post_turn_evaluation_service: PostTurnEvaluationService",
        ],
    });
}

#[test]
#[ignore]
fn future_core_runtime_uses_application_facade_not_service_modules() {
    /*
     * Disabled target: core/runtime should eventually execute one narrow
     * application-facing facade instead of importing service modules, storing raw
     * services, or calling service methods directly from core worker code.
     */
    assert_no_forbidden_references(BoundaryRule {
        name: "future core runtime must call an application facade instead of raw service modules",
        root: "src/core/runtime",
        forbidden_patterns: CORE_RUNTIME_RAW_APPLICATION_SERVICE_FORBIDDEN_PATTERNS,
    });
}

#[test]
fn core_runtime_driver_stays_service_agnostic() {
    // Static guard: the runtime loop may dispatch effects through a trait, but concrete application
    // service wiring belongs to CoreEffectRunner so the command loop stays reusable and testable.
    assert_no_forbidden_references_in_paths(
        "core runtime driver must not own application service or domain dependencies",
        &["src/core/runtime/driver.rs"],
        &[
            "crate::application::",
            "crate::domain::",
            "crate::adapter::",
            "crate::composition::",
            "CoreEffectRunner",
            "StartupService",
            "SessionService",
            "ConversationService",
            "PlanningServices",
            "PlanningRuntimeUseCases",
            "ParallelModeTurnService",
            "ManualPromptPreparationService",
            "PostTurnEvaluationService",
            "std::thread",
            "thread::spawn",
        ],
    );
}

#[test]
fn core_runtime_has_no_concrete_boundary_or_framework_dependencies() {
    // Static guard: core/runtime may execute application services, but it must not instantiate
    // concrete adapters, ports, process commands, persistence, telemetry, or UI/web frameworks.
    assert_no_forbidden_references(BoundaryRule {
        name: "core runtime must execute effects without owning concrete infrastructure boundaries",
        root: "src/core/runtime",
        forbidden_patterns: &[
            "crate::adapter::",
            "crate::composition::",
            "crate::application::port::",
            "crate::diagnostics",
            "crate::subprocess",
            "std::fs",
            "std::process",
            "Command::new",
            "tokio::",
            "ratatui",
            "crossterm",
            "axum",
            "askama",
            "rusqlite",
            "Sqlite",
            "Filesystem",
            "CodexAppServer",
            "Github",
            "Telegram",
            "tower",
            "which",
            "serde_json",
            "tracing::",
        ],
    });
}

#[test]
fn core_runtime_worker_modules_stay_private_to_effect_boundary() {
    // Static guard: workers such as turn submission are implementation detail behind CoreEffectRunner.
    let repo_root = repo_root();
    let runtime_mod_path = repo_root.join("src/core/runtime/mod.rs");
    let source = fs::read_to_string(&runtime_mod_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", runtime_mod_path.display());
    });

    assert!(
        source.contains("mod turn_submission;"),
        "turn submission worker module must stay private to core/runtime"
    );
    assert!(
        !source.contains("pub mod turn_submission;"),
        "turn submission worker module must not become part of the public core runtime contract"
    );
}

#[test]
fn core_layer_does_not_bypass_parallel_control_plane_gate() {
    // Parallel mode already has an application single-writer gate. Core may eventually
    // expose a projection of that state, but it must not own the raw service, host
    // internals, runtime store, or gate-owned wake/effect machinery.
    assert_no_forbidden_references(BoundaryRule {
        name: "core must not bypass the parallel control-plane single-writer gate",
        root: "src/core",
        forbidden_patterns: &[
            "ParallelModeService",
            "ParallelModeControlPlaneService",
            "ParallelModeControlPlaneRuntime",
            "ParallelModeControlPlaneRuntimeStore",
            "ParallelModeControlPlaneWake",
            "ParallelModeControlPlaneEffectId",
        ],
    });
}

#[test]
fn tui_startup_checks_enter_through_core_runtime() {
    // Static guard for the startup migration: TUI may request startup checks, but execution belongs
    // to CoreRuntime/CoreEffectRunner so completion re-enters core before TUI state changes.
    assert_no_forbidden_references_in_paths(
        "TUI startup checks must be dispatched through core runtime, not StartupService directly",
        &["src/adapter/inbound/tui/app/app_runtime.rs"],
        &[".run_checks(", "NativeTuiStartupHandle"],
    );
}

#[test]
fn tui_session_catalog_loads_enter_through_core_runtime() {
    // Static guard for the session migration: TUI owns overlay state and selection, while session
    // catalog loading runs through CoreRuntime/CoreEffectRunner before TUI receives catalog state.
    assert_no_forbidden_references_in_paths(
        "TUI session catalog loads must be dispatched through core runtime, not SessionService directly",
        &["src/adapter/inbound/tui/app/app_runtime.rs"],
        &[".load_session_catalog(", "NativeTuiSessionCatalogHandle"],
    );
}

#[test]
fn tui_conversation_loads_enter_through_core_runtime() {
    // Static guard for the conversation lifecycle migration: TUI may keep presentation
    // state and reducers, but snapshot loading must enter CoreRuntime/CoreEffectRunner.
    assert_no_forbidden_references_in_paths(
        "TUI conversation loads must be dispatched through core runtime, not ConversationService directly",
        &["src/adapter/inbound/tui/app/app_runtime.rs"],
        &[".load_snapshot("],
    );
}

#[test]
fn tui_conversation_stream_events_enter_through_core_runtime() {
    // Static guard for turn-stream preparation: shell runtime should not feed app-server
    // stream events directly to the TUI conversation reducer.
    assert_no_forbidden_references_in_paths(
        "TUI conversation stream events must re-enter core before reducer application",
        &["src/adapter/inbound/tui/app/shell_runtime.rs"],
        &["ConversationRuntimeEvent::StreamUpdated"],
    );
}

#[test]
fn tui_conversation_turn_events_enter_through_core_runtime() {
    // Static guard for application-internal async conversation events. Turn completion and
    // runtime notices should re-enter core before TUI reducers observe them.
    assert_no_forbidden_references_in_paths(
        "TUI conversation turn events must re-enter core before reducer application",
        &["src/adapter/inbound/tui/app/shell_runtime.rs"],
        &[
            "ConversationRuntimeEvent::StreamTurnCompleted",
            "ConversationRuntimeEvent::StreamExecutionObserved",
        ],
    );
}

#[test]
fn tui_manual_prompt_preparation_enters_through_core_runtime() {
    // TUI owns editable prompt text and overlay state, while manual planning bootstrap and intake
    // execution must run as a core effect backed by application services.
    assert_no_forbidden_references_in_paths(
        "TUI manual prompt preparation must enter core instead of calling planning bootstrap/intake directly",
        &["src/adapter/inbound/tui/app/turn_submission_runtime.rs"],
        &[
            "ManualPromptIntakeRequest",
            ".prepare_manual_prompt_intake(",
            "fn ensure_manual_planning_workspace",
            ".stage_simple_mode_draft(",
            ".promote_staged_draft(",
        ],
    );
}

#[test]
fn inbound_adapters_only_wire_outbound_adapters_in_explicit_composition_roots() {
    // Static guard: R9 moved production wiring to crate::composition, so inbound adapter imports of outbound
    // implementations are now direct boundary regressions. Behavior smoke lives in production_composition tests.
    assert_no_forbidden_references(inbound_outbound_boundary_rule());
}

#[test]
fn inbound_adapters_do_not_mutate_parallel_durable_state_directly() {
    // Static guard: these method names are the durable mutation boundary. Flow tests exercise dispatch/recovery
    // behavior, while this check prevents inbound surfaces from reaching around the application service.
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
    });
}

#[test]
fn tui_post_turn_execution_uses_planning_post_turn_facade() {
    // Static guard retained as a supplement to post-turn behavior tests: exact low-level planning workflow
    // symbols must not reappear in the TUI executor even if the happy path still works.
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
    // Static guard: outbound adapters implement ports and should never depend on inbound transport/UI code.
    assert_no_forbidden_references(BoundaryRule {
        name: "outbound adapters must not depend on inbound adapters or core runtime",
        root: "src/adapter/outbound",
        forbidden_patterns: &["crate::adapter::inbound::", "crate::core::"],
    });
}

#[test]
fn outbound_port_modules_follow_port_naming_contract() {
    // Static guard: port module naming is a directory/API contract, not a behavior.
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
    // Static guard: R8 behavior tests cover TUI flow, but raw service fields in TUI state are a structural leak.
    let debts = collect_pattern_debts(TUI_RAW_APPLICATION_SERVICE_DEBTS);

    assert!(
        debts.is_empty(),
        "temporary TUI service-wiring debt remains. TUI production state should hold UI state, projection cache, and narrow application handles only:\n{}",
        format_temporary_debts(&debts)
    );
}

#[test]
fn temporary_parallel_runtime_store_has_been_made_private_to_single_writer() {
    // Static guard: public fields on the runtime store would bypass the single-writer facade by construction.
    let debts = collect_public_fields_in_struct(
        "src/application/service/parallel_mode/control_plane/mod.rs",
        "ParallelModeControlPlaneRuntimeStore",
        "runtime store fields are still public; single-writer gate state should be private and mutated only by the runtime.",
    );

    assert!(
        debts.is_empty(),
        "temporary parallel single-writer debt remains. Runtime store should not expose mutable state shape as public fields:\n{}",
        format_temporary_debts(&debts)
    );
}

#[test]
fn parallel_control_plane_host_uses_explicit_synchronous_single_writer_gate() {
    // Static guard retained for the R6 architecture decision. The behavior counterpart is
    // `synchronous_mutex_facade_covers_ordering_backpressure_and_stale_completion`.
    let repo_root = repo_root();
    let host_path = repo_root.join("src/application/service/parallel_mode/control_plane/host.rs");
    let source = fs::read_to_string(&host_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", host_path.display());
    });

    assert!(
        source.contains("R6 decision: this is intentionally a synchronous mutex-serialized facade"),
        "control-plane host must document the R6 mutex-facade decision"
    );
    assert!(
        source.contains("Mutex<ParallelModeControlPlaneService"),
        "control-plane host must keep one application-owned synchronous gate"
    );
    assert!(
        !source.contains("mpsc::") && !source.contains("tokio::sync::mpsc"),
        "R6 keeps the control-plane host synchronous; do not add a mailbox actor in this slice"
    );
}

#[test]
fn temporary_parallel_control_surfaces_no_longer_bypass_control_plane_gate() {
    // Static guard retained because a raw service escape hatch is visible in source before it shows up as
    // broken behavior. Control-plane command behavior is covered by the parallel_mode test suite.
    let debts = collect_pattern_debts(PARALLEL_CONTROL_PLANE_BYPASS_DEBTS);

    assert!(
        debts.is_empty(),
        "temporary parallel control-surface debt remains. Parallel control should enter through commands and gate-owned effects only:\n{}",
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
                if source_line.text.contains(pattern) {
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

fn assert_only_allowed_crate_references(rule: AllowedCrateReferenceRule) {
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

            for crate_reference in crate_references(&source_line.text) {
                if !rule
                    .allowed_prefixes
                    .iter()
                    .any(|allowed_prefix| crate_reference.starts_with(allowed_prefix))
                {
                    violations.push(BoundaryViolation {
                        rule: rule.name,
                        path: relative_path.clone(),
                        line: source_line.number,
                        pattern: "crate::",
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

fn crate_references(line: &str) -> Vec<&str> {
    let mut references = Vec::new();
    let mut start = 0;

    while let Some(offset) = line[start..].find("crate::") {
        let reference_start = start + offset;
        references.push(&line[reference_start..]);
        start = reference_start + "crate::".len();
    }

    references
}

fn inbound_outbound_boundary_rule() -> BoundaryRule {
    BoundaryRule {
        name: "inbound adapters must not pull outbound adapters outside explicit composition roots",
        root: "src/adapter/inbound",
        forbidden_patterns: &["crate::adapter::outbound::"],
    }
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
