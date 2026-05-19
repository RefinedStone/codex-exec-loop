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

#[derive(Clone, Copy)]
struct TuiCoverageSurface {
    name: &'static str,
    doc_marker: &'static str,
    source_prefixes: &'static [&'static str],
    test_entrypoints: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct TuiCoverageSourceException {
    path: &'static str,
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

const TUI_COVERAGE_SURFACES: &[TuiCoverageSurface] = &[
    TuiCoverageSurface {
        name: "Inline terminal, host scrollback, viewport, resize, redraw transaction",
        doc_marker: "| Inline terminal, host scrollback, viewport, resize, redraw transaction |",
        source_prefixes: &[
            "src/adapter/inbound/tui/app/inline_terminal_adapter",
            "src/adapter/inbound/tui/app/history_insertion.rs",
        ],
        test_entrypoints: &[
            "src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs",
            "src/adapter/inbound/tui/app/inline_terminal_adapter/tests/history_flush.rs",
            "src/adapter/inbound/tui/app/history_insertion.rs",
        ],
    },
    TuiCoverageSurface {
        name: "Parallel event stream, live-tail, prompt position, command hints",
        doc_marker: "| Parallel event stream, live-tail, prompt position, command hints |",
        source_prefixes: &[
            "src/adapter/inbound/tui/app/parallel_",
            "src/adapter/inbound/tui/app/parallel_mode",
            "src/adapter/inbound/tui/app/shell_presentation/overlays/popup/supersession",
            "src/adapter/inbound/tui/app/shell_presentation/status_panels",
            "src/adapter/inbound/tui/supersession_mud.rs",
        ],
        test_entrypoints: &[
            "src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs",
            "src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs",
            "src/adapter/inbound/tui/app/shell_runtime/tests/flows.rs",
            "src/adapter/inbound/tui/app/shell_runtime/tests/input.rs",
            "src/adapter/inbound/tui/app/parallel_peek_overlay_ui.rs",
        ],
    },
    TuiCoverageSurface {
        name: "Overlay surfaces: help, session, planning, model/view/language selection, parallel peek",
        doc_marker: "| Overlay surfaces: help, session, planning, model/view/language selection, parallel peek |",
        source_prefixes: &[
            "src/adapter/inbound/tui/app/language.rs",
            "src/adapter/inbound/tui/app/auto_follow_overlay_ui.rs",
            "src/adapter/inbound/tui/app/directions_maintenance_ui.rs",
            "src/adapter/inbound/tui/app/model_selection_overlay_ui.rs",
            "src/adapter/inbound/tui/app/planning",
            "src/adapter/inbound/tui/app/planning_",
            "src/adapter/inbound/tui/app/session_overlay_ui.rs",
            "src/adapter/inbound/tui/app/shell_presentation/overlays",
            "src/adapter/inbound/tui/app/view_selection_overlay_ui.rs",
            "src/adapter/inbound/tui/shell_chrome.rs",
        ],
        test_entrypoints: &[
            "src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs",
            "src/adapter/inbound/tui/app/shell_rendering_contract_tests/planning.rs",
            "src/adapter/inbound/tui/app/shell_rendering_tests.rs",
            "src/adapter/inbound/tui/app/language.rs",
            "src/adapter/inbound/tui/app/planning_draft_editor_ui/tests.rs",
            "src/adapter/inbound/tui/app/planning/controller.rs",
            "src/adapter/inbound/tui/app/session_overlay_ui.rs",
            "src/adapter/inbound/tui/app/model_selection_overlay_ui.rs",
            "src/adapter/inbound/tui/app/view_selection_overlay_ui.rs",
            "src/adapter/inbound/tui/shell_chrome.rs",
        ],
    },
    TuiCoverageSurface {
        name: "Shell runtime input flow: key events, command palette, submit, escape/cancel",
        doc_marker: "| Shell runtime input flow: key events, command palette, submit, escape/cancel |",
        source_prefixes: &[
            "src/adapter/inbound/tui/app/conversation",
            "src/adapter/inbound/tui/app/inline_shell_commands",
            "src/adapter/inbound/tui/app/parallel_mode_shell_command.rs",
            "src/adapter/inbound/tui/app/planning_overlay_shell_command.rs",
            "src/adapter/inbound/tui/app/planning_reset_shell_command.rs",
            "src/adapter/inbound/tui/app/planning_shell_command.rs",
            "src/adapter/inbound/tui/app/shell_runtime",
            "src/adapter/inbound/tui/app/turn_submission_runtime",
        ],
        test_entrypoints: &[
            "src/adapter/inbound/tui/app/shell_runtime/tests/input.rs",
            "src/adapter/inbound/tui/app/shell_runtime/tests/flows.rs",
            "src/adapter/inbound/tui/app/shell_runtime/tests/scheduler.rs",
            "src/adapter/inbound/tui/app/conversation_input.rs",
            "src/adapter/inbound/tui/app/conversation_intents.rs",
            "src/adapter/inbound/tui/app/inline_shell_commands/tests.rs",
        ],
    },
    TuiCoverageSurface {
        name: "Shell rendering snapshots plus targeted assertions",
        doc_marker: "| Shell rendering snapshots plus targeted assertions |",
        source_prefixes: &[
            "src/adapter/inbound/tui/app/shell_rendering",
            "src/adapter/inbound/tui/app/shell_layout.rs",
            "src/adapter/inbound/tui/app/shell_presentation.rs",
            "src/adapter/inbound/tui/app/shell_presentation",
            "src/adapter/inbound/tui/app/theme.rs",
            "src/adapter/inbound/tui/conversation_text.rs",
        ],
        test_entrypoints: &[
            "src/adapter/inbound/tui/app/shell_rendering_tests.rs",
            "src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs",
            "src/adapter/inbound/tui/app/shell_rendering_contract_tests/planning.rs",
            "src/adapter/inbound/tui/app/snapshots",
        ],
    },
    TuiCoverageSurface {
        name: "vt100 terminal path",
        doc_marker: "| vt100 terminal path |",
        source_prefixes: &[
            "src/adapter/inbound/tui/app/tui_testkit.rs",
            "src/adapter/inbound/tui/app/history_insertion.rs",
            "src/adapter/inbound/tui/app/inline_terminal_adapter",
            "src/adapter/inbound/tui/app/shell_rendering_tests.rs",
        ],
        test_entrypoints: &[
            "src/adapter/inbound/tui/app/tui_testkit.rs",
            "src/adapter/inbound/tui/app/history_insertion.rs",
            "src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs",
            "src/adapter/inbound/tui/app/shell_rendering_tests.rs",
        ],
    },
    TuiCoverageSurface {
        name: "Startup, session, conversation, auto-follow, planning control state",
        doc_marker: "| Startup, session, conversation, auto-follow, planning control state |",
        source_prefixes: &[
            "src/adapter/inbound/tui/app.rs",
            "src/adapter/inbound/tui/app/app_runtime.rs",
            "src/adapter/inbound/tui/app/auto_follow",
            "src/adapter/inbound/tui/app/auto_follow_controls.rs",
            "src/adapter/inbound/tui/app/conversation",
            "src/adapter/inbound/tui/app/github_polling",
            "src/adapter/inbound/tui/app/post_turn_continuation.rs",
            "src/adapter/inbound/tui/app/shell_controller.rs",
            "src/adapter/inbound/tui/app/shell_entrypoint.rs",
            "src/adapter/inbound/tui/app/shell_frontend.rs",
            "src/adapter/inbound/tui/app/ratatui_frontend.rs",
            "src/adapter/inbound/tui/app/session_shell_controller.rs",
            "src/adapter/inbound/tui/app/turn_submission_runtime",
        ],
        test_entrypoints: &[
            "src/adapter/inbound/tui/app.rs",
            "src/adapter/inbound/tui/app/auto_follow_controls.rs",
            "src/adapter/inbound/tui/app/auto_follow_overlay_ui.rs",
            "src/adapter/inbound/tui/app/conversation_model_tests.rs",
            "src/adapter/inbound/tui/app/conversation_runtime.rs",
            "src/adapter/inbound/tui/app/github_polling/tests.rs",
            "src/adapter/inbound/tui/app/turn_submission_runtime.rs",
            "src/adapter/inbound/tui/app/shell_entrypoint.rs",
        ],
    },
    TuiCoverageSurface {
        name: "TUI support and validation devices",
        doc_marker: "| TUI support and validation devices |",
        source_prefixes: &[
            "src/adapter/inbound/tui/app/test_helpers.rs",
            "src/adapter/inbound/tui/app/tui_testkit.rs",
            "tests/architecture_boundaries.rs",
            "tests/native_validation_scripts.rs",
        ],
        test_entrypoints: &[
            "src/adapter/inbound/tui/app/tui_testkit.rs",
            "tests/architecture_boundaries.rs",
            "tests/native_validation_scripts.rs",
            "docs/validation/tui-coverage-matrix.md",
        ],
    },
];

const TUI_COVERAGE_SOURCE_EXCEPTIONS: &[TuiCoverageSourceException] = &[
    TuiCoverageSourceException {
        path: "src/adapter/inbound/tui/mod.rs",
        reason: "module declaration glue only; behavior is covered by the mapped child TUI surfaces.",
    },
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
    // Static guard: workers such as turn submission are implementation detail behind the
    // composition-owned CoreEffectRunner, not public core/runtime API.
    let repo_root = repo_root();
    let runtime_mod_path = repo_root.join("src/core/runtime/mod.rs");
    let source = fs::read_to_string(&runtime_mod_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", runtime_mod_path.display());
    });
    let composition_mod_path = repo_root.join("src/composition/mod.rs");
    let composition_source = fs::read_to_string(&composition_mod_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", composition_mod_path.display());
    });

    assert!(
        !source.contains("turn_submission"),
        "turn submission worker module must not live under core/runtime"
    );
    assert!(
        composition_source.contains("pub(crate) mod core_turn_submission;"),
        "turn submission worker module must stay crate-private behind composition CoreEffectRunner"
    );
    assert!(
        !composition_source.contains("pub mod core_turn_submission;"),
        "turn submission worker module must not become part of a public composition contract"
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

#[test]
fn tui_temporal_regressions_use_shared_frame_recorder_contract() {
    // Static guard: redraw-order regressions need temporal evidence. Keep the
    // recorder in the shared TUI testkit so future TUI tests do not re-create
    // weaker one-off final-screen checks.
    let repo_root = repo_root();
    let docs_path = repo_root.join("docs/validation/terminal-ui-testing-methodology.md");
    let testkit_path = repo_root.join("src/adapter/inbound/tui/app/tui_testkit.rs");
    let inline_tests_path =
        repo_root.join("src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs");
    let docs = fs::read_to_string(&docs_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", docs_path.display());
    });
    let testkit = fs::read_to_string(&testkit_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", testkit_path.display());
    });
    let inline_tests = fs::read_to_string(&inline_tests_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", inline_tests_path.display());
    });

    for required_doc_text in [
        "direct frame recorder: store every rendered buffer",
        "Frame recorder assertions should include",
        "Parallel event stream | frame recorder proves",
    ] {
        assert!(
            docs.contains(required_doc_text),
            "TUI methodology must require frame-recorder coverage for redraw-order regressions: {required_doc_text}"
        );
    }

    for required_testkit_text in [
        "pub(super) struct InlineFrameRecorder",
        "pub(super) struct RecordedInlineFrame",
        "pub(super) fn draw_and_record",
        "pub(super) fn record_inline",
        "screen_text",
        "host_scrollback_text",
        "terminal_history_text",
        "app_event_stream_text",
    ] {
        assert!(
            testkit.contains(required_testkit_text),
            "shared TUI testkit must expose the direct frame-recorder contract: {required_testkit_text}"
        );
    }

    assert!(
        !inline_tests.contains("struct InlineFrameRecorder"),
        "inline terminal tests must use the shared tui_testkit recorder instead of a local one-off recorder"
    );
    assert!(
        inline_tests.contains("tui_testkit::InlineFrameRecorder::default()"),
        "inline terminal frame-recorder regressions must instantiate the shared tui_testkit recorder"
    );
    for required_regression in [
        "direct_frame_recorder_keeps_parallel_status_rows_across_runtime_redraw",
        "direct_frame_recorder_catches_wrapped_parallel_stream_split_at_live_boundary",
    ] {
        assert!(
            inline_tests.contains(required_regression),
            "parallel stream redraw regression must stay covered by a direct frame-recorder test: {required_regression}"
        );
    }
}

#[test]
fn tui_parallel_stream_continuity_is_architecture_contract() {
    // Static guard: append-only stream surfaces are more fragile than ordinary
    // panels because the visible rows can be split between host scrollback and
    // the live inline viewport. Keep the contract explicit so a future wording
    // or layout tweak cannot reintroduce title chrome in the middle of a stream.
    let repo_root = repo_root();
    let methodology_path = repo_root.join("docs/validation/terminal-ui-testing-methodology.md");
    let design_path =
        repo_root.join("docs/design/07-tui-layered-architecture-and-aesthetic-contract.md");
    let matrix_path = repo_root.join("docs/validation/tui-coverage-matrix.md");
    let renderer_path =
        repo_root.join("src/adapter/inbound/tui/app/shell_rendering/inline_inspection.rs");
    let layout_path =
        repo_root.join("src/adapter/inbound/tui/app/shell_rendering/inline_layout.rs");
    let inline_tests_path =
        repo_root.join("src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs");
    let methodology = fs::read_to_string(&methodology_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", methodology_path.display());
    });
    let design = fs::read_to_string(&design_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", design_path.display());
    });
    let matrix = fs::read_to_string(&matrix_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", matrix_path.display());
    });
    let renderer = fs::read_to_string(&renderer_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", renderer_path.display());
    });
    let layout = fs::read_to_string(&layout_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", layout_path.display());
    });
    let inline_tests = fs::read_to_string(&inline_tests_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", inline_tests_path.display());
    });

    for required_text in [
        "Architectural Guardrails",
        "no panel title may be inserted between durable scrollback rows and live rows",
        "dedicated stream renderer",
        "typed render surface API",
        "InlineAppendOnlyStream",
        "titleless live tail",
    ] {
        assert!(
            methodology.contains(required_text),
            "TUI methodology must document stream-continuity guardrail: {required_text}"
        );
    }

    for required_text in [
        "Append-only Stream Surfaces",
        "No panel title may be inserted between durable scrollback rows and live rows",
        "titleless live tail data only",
        "explicit render surface type",
        "InlineTitledPanel",
        "InlineScrolledPanel",
        "InlineAppendOnlyStream",
        "named stream renderer",
    ] {
        assert!(
            design.contains(required_text),
            "TUI design contract must document stream-continuity architecture: {required_text}"
        );
    }

    assert!(
        matrix.contains("split scrollback/live-tail streams render as a titleless live tail"),
        "TUI coverage matrix must name split-stream titleless live-tail behavior"
    );
    assert!(
        matrix.contains("typed render surface routing"),
        "TUI coverage matrix must name typed render surface routing"
    );

    for required_renderer_text in [
        "fn render_inline_parallel_event_stream",
        "Architecture contract: a split event stream is not a titled panel.",
        "InlineAppendOnlyStreamTitle::Hidden",
        "InlineAppendOnlyStream::new(title, lines, stream_scroll_offset).render(frame, area)",
    ] {
        assert!(
            renderer.contains(required_renderer_text),
            "parallel event stream renderer must keep titleless split-stream contract: {required_renderer_text}"
        );
    }
    for forbidden_renderer_text in [
        "render_inline_section(",
        "render_inline_scrolled_section(",
        "render_inline_scrolled_body(",
    ] {
        assert!(
            !renderer.contains(forbidden_renderer_text),
            "inline inspection must route through typed render surfaces instead of low-level helper: {forbidden_renderer_text}"
        );
    }
    assert!(
        !renderer.contains("Recent Parallel Events"),
        "parallel event stream renderer must not replace one misplaced stream title with another"
    );

    for required_layout_text in [
        "pub(super) struct InlineTitledPanel",
        "pub(super) struct InlineScrolledPanel",
        "pub(super) enum InlineAppendOnlyStreamTitle",
        "pub(super) struct InlineAppendOnlyStream",
    ] {
        assert!(
            layout.contains(required_layout_text),
            "inline layout must expose the typed render surface API: {required_layout_text}"
        );
    }
    for forbidden_layout_text in [
        "pub(super) fn render_inline_section(",
        "pub(super) fn render_inline_scrolled_section(",
        "pub(super) fn render_inline_scrolled_body(",
    ] {
        assert!(
            !layout.contains(forbidden_layout_text),
            "low-level inline layout helper must stay private behind typed render surfaces: {forbidden_layout_text}"
        );
    }

    for required_regression in [
        "parallel_live_tail_continues_scrollback_without_inline_title",
        "parallel_bootstrap_and_task_intake_stream_does_not_insert_tail_title",
        "direct_frame_recorder_keeps_parallel_status_rows_across_runtime_redraw",
    ] {
        assert!(
            inline_tests.contains(required_regression),
            "parallel stream continuity must stay covered by a named frame-recorder regression: {required_regression}"
        );
    }
}

#[test]
fn tui_system_korean_copy_is_localized_through_language_module() {
    // Static guard: production TUI Korean copy should be centralized so a new
    // localized row cannot bypass `:language` by embedding Hangul at a call site.
    let repo_root = repo_root();
    let language_path = repo_root.join("src/adapter/inbound/tui/app/language.rs");
    let language_source = fs::read_to_string(&language_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", language_path.display());
    });

    for required_text in [
        "pub(super) enum TuiLanguage",
        "LANGUAGE_SELECTION_OPTIONS",
        "parallel_board_refreshed",
        "parallel_history_summary",
        "startup_axis_row",
        "startup_diagnostics_summary_line",
        "LanguageSelectionOverlayUiState",
    ] {
        assert!(
            language_source.contains(required_text),
            "TUI localization module must own language copy and selection state: {required_text}"
        );
    }

    let tui_root = repo_root.join("src/adapter/inbound/tui");
    let mut violations = Vec::new();
    for path in rust_files_under(&tui_root) {
        if is_test_only_path(&path) {
            continue;
        }
        let relative = relative_path(&repo_root, &path);
        if relative == "src/adapter/inbound/tui/app/language.rs" {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        let production_source = production_source_before_inline_tests(&source);
        for literal in korean_string_literals(&production_source) {
            violations.push(format!("{relative}: \"{literal}\""));
        }
    }

    assert!(
        violations.is_empty(),
        "TUI production Korean string literals must go through app/language.rs:\n{}",
        violations.join("\n")
    );
}

#[test]
fn tui_coverage_matrix_maps_existing_sources_to_automated_entrypoints() {
    // Static guard: existing TUI code should not rely on tribal memory for test
    // coverage. Each production source file must belong to a documented surface
    // with at least one automated test entrypoint, or have a narrow exception.
    let repo_root = repo_root();
    let matrix_path = repo_root.join("docs/validation/tui-coverage-matrix.md");
    let methodology_path = repo_root.join("docs/validation/terminal-ui-testing-methodology.md");
    let matrix = fs::read_to_string(&matrix_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", matrix_path.display());
    });
    let methodology = fs::read_to_string(&methodology_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", methodology_path.display());
    });

    for required_text in [
        "terminal-ui-testing-methodology.md",
        "tui_testkit::InlineFrameRecorder",
        "Ratatui `TestBackend`",
        "vt100-backed tests",
        "architecture-test exception",
    ] {
        assert!(
            matrix.contains(required_text),
            "TUI coverage matrix must document the project-wide TUI testing rule: {required_text}"
        );
    }
    assert!(
        methodology.contains("docs/validation/tui-coverage-matrix.md"),
        "TUI methodology must link to the project-wide coverage matrix"
    );

    for surface in TUI_COVERAGE_SURFACES {
        assert!(
            matrix.contains(surface.doc_marker),
            "TUI coverage matrix must contain surface row: {}",
            surface.name
        );
        for entrypoint in surface.test_entrypoints {
            assert_tui_test_entrypoint_has_coverage(&repo_root, surface.name, entrypoint);
        }
    }

    let tui_root = repo_root.join("src/adapter/inbound/tui");
    let mut unmapped_sources = Vec::new();
    for path in rust_files_under(&tui_root) {
        if is_test_only_path(&path) {
            continue;
        }
        let relative = relative_path(&repo_root, &path);
        if TUI_COVERAGE_SOURCE_EXCEPTIONS
            .iter()
            .any(|exception| exception.path == relative)
        {
            continue;
        }
        if TUI_COVERAGE_SURFACES.iter().any(|surface| {
            surface
                .source_prefixes
                .iter()
                .any(|prefix| relative.starts_with(prefix))
        }) {
            continue;
        }
        unmapped_sources.push(relative);
    }

    assert!(
        unmapped_sources.is_empty(),
        "TUI source files must be mapped to the coverage matrix or an explicit exception:\n{}",
        unmapped_sources.join("\n")
    );

    for exception in TUI_COVERAGE_SOURCE_EXCEPTIONS {
        assert!(
            !exception.reason.trim().is_empty(),
            "TUI coverage exception must explain why {} is exempt",
            exception.path
        );
        assert!(
            repo_root.join(exception.path).exists(),
            "TUI coverage exception points at a missing path: {}",
            exception.path
        );
    }
}

#[test]
fn tui_coverage_matrix_lists_existing_tui_test_entrypoints() {
    // Static guard: the matrix is the TUI testing inventory, so every Rust file
    // that owns TUI tests must be listed there explicitly.
    let repo_root = repo_root();
    let matrix_path = repo_root.join("docs/validation/tui-coverage-matrix.md");
    let matrix = fs::read_to_string(&matrix_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", matrix_path.display());
    });

    let documented = tui_test_entrypoint_paths_from_matrix(&matrix);
    assert!(
        !documented.is_empty(),
        "TUI coverage matrix must list existing TUI test entrypoints"
    );

    let mut sorted_documented = documented.clone();
    sorted_documented.sort();
    assert_eq!(
        documented, sorted_documented,
        "TUI test entrypoint inventory must stay sorted by path"
    );

    let mut unique_documented = sorted_documented.clone();
    unique_documented.dedup();
    assert_eq!(
        sorted_documented, unique_documented,
        "TUI test entrypoint inventory must not contain duplicate paths"
    );

    let tui_root = repo_root.join("src/adapter/inbound/tui");
    let mut actual = Vec::new();
    for path in rust_files_under(&tui_root) {
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        if is_tui_test_entrypoint_source(&source) {
            actual.push(relative_path(&repo_root, &path));
        }
    }
    actual.sort();

    let missing = actual
        .iter()
        .filter(|path| !documented.contains(path))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "TUI test entrypoints must be listed in docs/validation/tui-coverage-matrix.md:\n{}",
        missing.join("\n")
    );

    let stale = documented
        .iter()
        .filter(|path| !actual.contains(path))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "TUI coverage matrix lists stale or missing TUI test entrypoints:\n{}",
        stale.join("\n")
    );
}

#[test]
fn tui_shared_test_devices_stay_in_tui_testkit() {
    // Static guard: reusable temporal/terminal devices belong in tui_testkit so
    // TUI tests share the same frame and backend contracts.
    let repo_root = repo_root();
    let tui_root = repo_root.join("src/adapter/inbound/tui");
    let mut violations = Vec::new();

    for path in rust_files_under(&tui_root) {
        let relative = relative_path(&repo_root, &path);
        if relative == "src/adapter/inbound/tui/app/tui_testkit.rs" {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        for forbidden in [
            "struct InlineFrameRecorder",
            "struct RecordedInlineFrame",
            "struct Vt100Backend",
            "struct Vt100Screen",
        ] {
            if source.contains(forbidden) {
                violations.push(format!(
                    "{relative}: local reusable TUI device `{forbidden}`"
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "reusable TUI test devices must stay centralized in tui_testkit:\n{}",
        violations.join("\n")
    );
}

fn assert_tui_test_entrypoint_has_coverage(repo_root: &Path, surface_name: &str, entrypoint: &str) {
    let path = repo_root.join(entrypoint);
    assert!(
        path.exists(),
        "TUI coverage surface `{surface_name}` references missing entrypoint: {entrypoint}"
    );

    if path.is_dir() {
        let mut covered_files = Vec::new();
        collect_files_with_extension(&path, "snap", &mut covered_files);
        assert!(
            !covered_files.is_empty(),
            "TUI coverage surface `{surface_name}` directory entrypoint has no snapshots: {entrypoint}"
        );
        return;
    }

    let source = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    });
    let has_coverage_marker = source.contains("#[test]")
        || source.contains("assert_snapshot!")
        || source.contains("# TUI Coverage Matrix");
    assert!(
        has_coverage_marker,
        "TUI coverage surface `{surface_name}` entrypoint must contain tests, snapshots, or matrix text: {entrypoint}"
    );
}

fn tui_test_entrypoint_paths_from_matrix(matrix: &str) -> Vec<String> {
    let marker = "## Test Entry Point Inventory";
    let section = matrix
        .split_once(marker)
        .unwrap_or_else(|| panic!("TUI coverage matrix must contain `{marker}`"))
        .1;
    let section = section
        .split_once("\n## ")
        .map_or(section, |(section, _)| section);
    section
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let path = trimmed.strip_prefix("- `")?;
            let path = path.split_once('`')?.0;
            path.starts_with("src/adapter/inbound/tui/")
                .then(|| path.to_string())
        })
        .collect()
}

fn is_tui_test_entrypoint_source(source: &str) -> bool {
    source.contains("#[test]")
        || source.contains("#[tokio::test]")
        || source.contains("assert_snapshot!")
}

fn production_source_before_inline_tests(source: &str) -> String {
    let test_module_index = source
        .find("#[cfg(test)]\nmod tests")
        .or_else(|| source.find("#[cfg(test)]\r\nmod tests"))
        .unwrap_or(source.len());
    source[..test_module_index].to_string()
}

fn korean_string_literals(source: &str) -> Vec<String> {
    let chars = source.chars().collect::<Vec<_>>();
    let mut literals = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '/' && chars.get(index + 1) == Some(&'/') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;
            while index + 1 < chars.len() && !(chars[index] == '*' && chars[index + 1] == '/') {
                index += 1;
            }
            index = (index + 2).min(chars.len());
            continue;
        }
        if chars[index] == 'r'
            && let Some((literal, next_index)) = parse_raw_rust_string_literal(&chars, index)
        {
            if contains_korean(&literal) {
                literals.push(literal);
            }
            index = next_index;
            continue;
        }
        if chars[index] == '"' {
            let (literal, next_index) = parse_quoted_rust_string_literal(&chars, index);
            if contains_korean(&literal) {
                literals.push(literal);
            }
            index = next_index;
            continue;
        }
        index += 1;
    }
    literals
}

fn parse_quoted_rust_string_literal(chars: &[char], start: usize) -> (String, usize) {
    let mut literal = String::new();
    let mut escaped = false;
    let mut index = start + 1;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            literal.push(ch);
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '"' {
            return (literal, index + 1);
        }
        literal.push(ch);
        index += 1;
    }
    (literal, index)
}

fn parse_raw_rust_string_literal(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut index = start + 1;
    let mut hashes = 0usize;
    while chars.get(index) == Some(&'#') {
        hashes += 1;
        index += 1;
    }
    if chars.get(index) != Some(&'"') {
        return None;
    }
    index += 1;
    let content_start = index;
    while index < chars.len() {
        if chars[index] == '"' && raw_string_hashes_match(chars, index + 1, hashes) {
            let literal = chars[content_start..index].iter().collect::<String>();
            return Some((literal, index + 1 + hashes));
        }
        index += 1;
    }
    None
}

fn raw_string_hashes_match(chars: &[char], start: usize, hashes: usize) -> bool {
    (0..hashes).all(|offset| chars.get(start + offset) == Some(&'#'))
}

fn contains_korean(text: &str) -> bool {
    text.chars()
        .any(|ch| ('\u{AC00}'..='\u{D7A3}').contains(&ch))
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

fn collect_files_with_extension(path: &Path, extension: &str, files: &mut Vec<PathBuf>) {
    if !path.exists() {
        return;
    }

    if path.is_file() {
        if path.extension().and_then(|value| value.to_str()) == Some(extension) {
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
        collect_files_with_extension(&entry.path(), extension, files);
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
