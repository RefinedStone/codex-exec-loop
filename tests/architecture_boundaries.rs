use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::visit_mut::{self, VisitMut};

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

#[derive(Debug, Eq, PartialEq)]
struct CrateReference {
    line: usize,
    path: String,
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
        name: "Overlay surfaces: help, session, planning, model/view/language selection, parallel peek, progressive activity",
        doc_marker: "| Overlay surfaces: help, session, planning, model/view/language selection, parallel peek, progressive activity |",
        source_prefixes: &[
            "src/adapter/inbound/tui/app/language.rs",
            "src/adapter/inbound/tui/app/auto_follow_overlay_ui.rs",
            "src/adapter/inbound/tui/app/directions_maintenance_ui.rs",
            "src/adapter/inbound/tui/app/model_selection_overlay_ui.rs",
            "src/adapter/inbound/tui/app/planning",
            "src/adapter/inbound/tui/app/planning_",
            "src/adapter/inbound/tui/app/progressive_activity_overlay_ui.rs",
            "src/adapter/inbound/tui/app/queue_overlay_controller.rs",
            "src/adapter/inbound/tui/app/queue_overlay_ui.rs",
            "src/adapter/inbound/tui/app/reviews_overlay_ui.rs",
            "src/adapter/inbound/tui/app/session_overlay_screen_model.rs",
            "src/adapter/inbound/tui/app/session_overlay_ui.rs",
            "src/adapter/inbound/tui/app/shell_presentation/overlays",
            "src/adapter/inbound/tui/app/view_selection_overlay_ui.rs",
            "src/adapter/inbound/tui/shell_chrome.rs",
        ],
        test_entrypoints: &[
            "src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs",
            "src/adapter/inbound/tui/app/shell_presentation/overlays/activity.rs",
            "src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs",
            "src/adapter/inbound/tui/app/shell_rendering_contract_tests/planning.rs",
            "src/adapter/inbound/tui/app/shell_rendering_tests.rs",
            "src/adapter/inbound/tui/app/language.rs",
            "src/adapter/inbound/tui/app/planning_draft_editor_ui/tests.rs",
            "src/adapter/inbound/tui/app/planning/controller.rs",
            "src/adapter/inbound/tui/app/progressive_activity_overlay_ui.rs",
            "src/adapter/inbound/tui/app/queue_overlay_ui.rs",
            "src/adapter/inbound/tui/app/shell_controller.rs",
            "src/adapter/inbound/tui/app/reviews_overlay_ui.rs",
            "src/adapter/inbound/tui/app/session_overlay_screen_model.rs",
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
            "src/adapter/inbound/tui/app/inline_frame_model.rs",
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
fn parallel_agent_profile_service_has_no_direct_filesystem_dependency() {
    assert_no_forbidden_references_in_paths(
        "parallel agent profile application service must use its repository port",
        &["src/application/service/parallel_agent_profile.rs"],
        &[
            "std::fs",
            "std::path",
            "Path::",
            "PathBuf",
            "File::",
            "OpenOptions",
        ],
    );
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
fn client_runtime_state_mutation_stays_behind_the_runtime_driver() {
    /*
     * Core is a client runtime outside the business hexagon. Adapters may dispatch
     * typed inputs and read snapshots, but direct controller/store mutation would
     * create a second ingress that bypasses effect ordering and correlation gates.
     */
    assert_no_forbidden_references(BoundaryRule {
        name: "inbound adapters must not construct or mutate client-runtime internals",
        root: "src/adapter/inbound",
        forbidden_patterns: &[
            "crate::core::app::CoreController",
            "crate::core::app::AppState",
            "crate::core::app::TurnStreamState",
            "crate::core::app::turn_stream::TurnStreamState",
        ],
    });

    let app_module = fs::read_to_string("src/core/app/mod.rs").unwrap();
    for forbidden in [
        "pub use controller::{CoreController",
        "pub use controller::CoreController",
        "pub use state::AppState",
        "pub use controller::state",
        "pub use turn_stream::TurnStreamState",
    ] {
        assert!(
            !app_module.contains(forbidden),
            "client-runtime mutation internals must not be publicly re-exported: {forbidden}"
        );
    }
    for required in [
        "pub(in crate::core) use controller::CoreController;",
        "pub(in crate::core) use turn_stream::TurnStreamState;",
    ] {
        assert!(
            app_module.contains(required),
            "client-runtime internal visibility contract is missing: {required}"
        );
    }

    let controller = fs::read_to_string("src/core/app/controller.rs").unwrap();
    assert!(
        controller.contains("pub(in crate::core) struct CoreController"),
        "CoreController must remain private to the client runtime"
    );
    assert!(
        controller.contains("pub(in crate::core) fn handle_input(&mut self, input: CoreInput)"),
        "only the client runtime driver may enter the root reducer"
    );

    let state = fs::read_to_string("src/core/app/controller/state.rs").unwrap();
    assert!(
        state.contains("pub(super) struct AppState"),
        "AppState must remain private to the root controller module"
    );
    assert!(
        controller.contains("mod state;") && controller.contains("use self::state::AppState;"),
        "the root reducer must own AppState in its private child module"
    );

    let turn_stream = fs::read_to_string("src/core/app/turn_stream.rs")
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        turn_stream.contains("pub(in crate::core) struct TurnStreamState"),
        "TurnStreamState must be impossible to name outside the client runtime"
    );
    assert!(
        turn_stream.contains("#[cfg(test)]\npub(crate) struct TurnStreamTestHarness"),
        "cross-module stream fixtures must be exposed only through a test harness"
    );

    for (path, internal_type) in [
        (
            "src/core/app/approval.rs",
            "ApprovalReviewPersistenceCoordinator",
        ),
        (
            "src/core/app/planning_runtime.rs",
            "PlanningRuntimeCoordinator",
        ),
        (
            "src/core/app/planning_workspace.rs",
            "PlanningWorkspaceOperationCoordinator",
        ),
        (
            "src/core/app/conversation_turn_reducer.rs",
            "ConversationTurnFeatureReducer",
        ),
        ("src/core/app/session_reducer.rs", "SessionFeatureReducer"),
    ] {
        let source = fs::read_to_string(path).unwrap();
        assert!(
            source.contains(&format!("pub(super) struct {internal_type}"))
                && !source.contains(&format!("pub(crate) struct {internal_type}"))
                && !source.contains(&format!("pub struct {internal_type}")),
            "{internal_type} must remain private to the root reducer"
        );
    }
    for forbidden_reexport in [
        "use approval::ApprovalReviewPersistenceCoordinator",
        "use conversation_turn_reducer::ConversationTurnFeatureReducer",
        "use planning_runtime::PlanningRuntimeCoordinator",
        "use planning_workspace::PlanningWorkspaceOperationCoordinator",
        "use session_reducer::SessionFeatureReducer",
    ] {
        assert!(
            !app_module.contains(forbidden_reexport),
            "mutation coordinator must not be re-exported: {forbidden_reexport}"
        );
    }

    let driver = fs::read_to_string("src/core/runtime/driver.rs").unwrap();
    assert!(
        driver.contains("    fn from_parts(")
            && !driver.contains("pub(crate) fn from_parts(")
            && !driver.contains("pub fn from_parts("),
        "injecting a raw CoreController must not be part of the public runtime API"
    );
}

#[test]
fn app_state_authority_is_sealed_inside_the_root_controller() {
    let app_module =
        fs::read_to_string("src/core/app/mod.rs").expect("core app module source should load");
    let controller = fs::read_to_string("src/core/app/controller.rs")
        .expect("core controller source should load");
    let state = fs::read_to_string("src/core/app/controller/state.rs")
        .expect("core controller state source should load");

    verify_app_state_controller_seal(&app_module, &controller, &state)
        .unwrap_or_else(|error| panic!("AppState authority must remain root-owned: {error}"));
}

#[test]
fn app_state_authority_analyzer_rejects_visibility_and_writer_escapes() {
    let app_module =
        fs::read_to_string("src/core/app/mod.rs").expect("core app module source should load");
    let controller = fs::read_to_string("src/core/app/controller.rs")
        .expect("core controller source should load");
    let state = fs::read_to_string("src/core/app/controller/state.rs")
        .expect("core controller state source should load");

    let sibling_state = app_module.replacen(
        "pub mod turn_interrupt;",
        "mod state;\npub mod turn_interrupt;",
        1,
    );
    let error = verify_app_state_controller_seal(&sibling_state, &controller, &state)
        .expect_err("a sibling AppState module must be rejected");
    assert!(
        error.contains("must not declare a sibling `state` module"),
        "unexpected sibling-state analyzer error: {error}"
    );

    let visible_child = controller.replacen("mod state;", "pub(super) mod state;", 1);
    let error = verify_app_state_controller_seal(&app_module, &visible_child, &state)
        .expect_err("a visible controller state child must be rejected");
    assert!(
        error.contains("private child"),
        "unexpected state-module visibility analyzer error: {error}"
    );

    let visible_state = state.replacen(
        "pub(super) struct AppState",
        "pub(crate) struct AppState",
        1,
    );
    let error = verify_app_state_controller_seal(&app_module, &controller, &visible_state)
        .expect_err("a crate-visible AppState must be rejected");
    assert!(
        error.contains("restricted to its parent CoreController"),
        "unexpected AppState visibility analyzer error: {error}"
    );

    let visible_writer = state.replacen("pub(super) fn new()", "pub(crate) fn new()", 1);
    let error = verify_app_state_controller_seal(&app_module, &controller, &visible_writer)
        .expect_err("a crate-visible AppState method must be rejected");
    assert!(
        error.contains("must remain private or pub(super)"),
        "unexpected AppState method visibility analyzer error: {error}"
    );

    let visible_field = state.replacen(
        "    current: Arc<AppSnapshot>,",
        "    pub(super) current: Arc<AppSnapshot>,",
        1,
    );
    let error = verify_app_state_controller_seal(&app_module, &controller, &visible_field)
        .expect_err("a visible AppState storage field must be rejected");
    assert!(
        error.contains("storage field must remain private"),
        "unexpected AppState field visibility analyzer error: {error}"
    );

    let nested_writer = controller.replacen(
        "mod state;",
        "mod state;\n\
         mod escaped_writer {\n\
             use super::state::AppState;\n\
             fn mutate(state: &mut AppState) {\n\
                 state.mark_startup_loading();\n\
             }\n\
         }",
        1,
    );
    let error = verify_app_state_controller_seal(&app_module, &nested_writer, &state)
        .expect_err("a production controller child module must not inherit AppState authority");
    assert!(
        error.contains("must not declare production child modules"),
        "unexpected nested-writer analyzer error: {error}"
    );

    let macro_writer = controller.replacen(
        "mod state;",
        "mod state;\n\
         macro_rules! escaped_writer {\n\
             () => {\n\
                 mod escaped_writer {\n\
                     use super::state::AppState;\n\
                     fn mutate(state: &mut AppState) {\n\
                         state.mark_startup_loading();\n\
                     }\n\
                 }\n\
             };\n\
         }\n\
         escaped_writer!();",
        1,
    );
    let error = verify_app_state_controller_seal(&app_module, &macro_writer, &state)
        .expect_err("a production item macro must not manufacture an AppState writer module");
    assert!(
        error.contains("must not contain production item macros"),
        "unexpected macro-writer analyzer error: {error}"
    );

    let statement_macro_writer = format!(
        "{controller}\n\
         fn invoke_writer_macro() {{\n\
             escaped_writer!();\n\
         }}\n"
    );
    let error = verify_app_state_controller_seal(&app_module, &statement_macro_writer, &state)
        .expect_err("a statement macro must not manufacture a local AppState writer module");
    assert!(
        error.contains("must not contain production item macros"),
        "unexpected statement-macro analyzer error: {error}"
    );

    let expression_macro_writer = format!(
        "{controller}\n\
         fn invoke_writer_macro() {{\n\
             let _writer = escaped_writer!();\n\
         }}\n"
    );
    let error = verify_app_state_controller_seal(&app_module, &expression_macro_writer, &state)
        .expect_err("an expression macro must not manufacture a local AppState writer module");
    assert!(
        error.contains("must not contain production item macros"),
        "unexpected expression-macro analyzer error: {error}"
    );

    let allowed_macro_writer = format!(
        "{controller}\n\
         fn invoke_writer_macro() {{\n\
             assert!({{\n\
                 mod escaped_writer {{\n\
                     use super::state::AppState;\n\
                     fn mutate(state: &mut AppState) {{\n\
                         state.mark_startup_loading();\n\
                     }}\n\
                 }}\n\
                 true\n\
             }});\n\
         }}\n"
    );
    let error = verify_app_state_controller_seal(&app_module, &allowed_macro_writer, &state)
        .expect_err("an allowed macro must not hide a local AppState writer item");
    assert!(
        error.contains("must not contain production item macros"),
        "unexpected allowed-macro analyzer error: {error}"
    );

    let nested_macro_writer = format!(
        "{controller}\n\
         fn invoke_writer_macro() {{\n\
             assert!(external_writer!());\n\
         }}\n"
    );
    let error = verify_app_state_controller_seal(&app_module, &nested_macro_writer, &state)
        .expect_err("an allowed macro must not hide a nested writer macro");
    assert!(
        error.contains("must not contain production item macros"),
        "unexpected nested-macro analyzer error: {error}"
    );

    let attribute_writer = controller.replacen(
        "#[derive(Debug, Clone, PartialEq, Eq)]",
        "#[escaped_writer]\n#[derive(Debug, Clone, PartialEq, Eq)]",
        1,
    );
    let error = verify_app_state_controller_seal(&app_module, &attribute_writer, &state)
        .expect_err("a production attribute macro must not manufacture an AppState writer");
    assert!(
        error.contains("procedural attribute `escaped_writer`"),
        "unexpected attribute-macro analyzer error: {error}"
    );

    let derive_writer = controller.replacen(
        "#[derive(Debug, Clone, PartialEq, Eq)]",
        "#[derive(Debug, Clone, PartialEq, Eq, EscapedWriter)]",
        1,
    );
    let error = verify_app_state_controller_seal(&app_module, &derive_writer, &state)
        .expect_err("a production derive macro must not manufacture an AppState writer");
    assert!(
        error.contains("procedural attribute `derive`"),
        "unexpected derive-macro analyzer error: {error}"
    );

    let allowed_attribute = controller.replacen(
        "#[derive(Debug, Clone, PartialEq, Eq)]",
        "#[allow(dead_code)]\n#[derive(Debug, Clone, PartialEq, Eq)]",
        1,
    );
    verify_app_state_controller_seal(&app_module, &allowed_attribute, &state)
        .expect("safe built-in attributes and derives must remain allowed");

    let nested_state_writer = format!(
        "{state}\n\
         mod escaped_writer {{\n\
             use super::AppState;\n\
             fn mutate(state: &mut AppState) {{\n\
                 state.mark_startup_loading();\n\
             }}\n\
         }}\n"
    );
    let error = verify_app_state_controller_seal(&app_module, &controller, &nested_state_writer)
        .expect_err("the AppState child file must not delegate writer authority");
    assert!(
        error.contains("state child must not declare production child modules"),
        "unexpected nested state-writer analyzer error: {error}"
    );
}

#[test]
fn raw_core_runtime_api_is_crate_private_and_native_facade_is_bounded() {
    let core_module =
        fs::read_to_string("src/core/mod.rs").expect("core module source should load");
    let runtime_module =
        fs::read_to_string("src/core/runtime/mod.rs").expect("runtime module source should load");
    let driver = fs::read_to_string("src/core/runtime/driver.rs")
        .expect("runtime driver source should load");
    let mailbox = fs::read_to_string("src/core/runtime/input_mailbox.rs")
        .expect("runtime mailbox source should load");
    let native_facade = fs::read_to_string("src/composition/native_client_runtime.rs")
        .expect("native client facade source should load");

    verify_client_runtime_api_boundary(
        &core_module,
        &runtime_module,
        &driver,
        &mailbox,
        &native_facade,
    )
    .unwrap_or_else(|error| panic!("client runtime API boundary must remain sealed: {error}"));

    let public_runtime_module =
        core_module.replacen("pub(crate) mod runtime;", "pub mod runtime;", 1);
    let error = verify_client_runtime_api_boundary(
        &public_runtime_module,
        &runtime_module,
        &driver,
        &mailbox,
        &native_facade,
    )
    .expect_err("making the raw runtime module public must fail");
    assert!(
        error.contains("core::runtime module"),
        "unexpected public runtime module error: {error}"
    );

    let public_runtime_type =
        driver.replacen("pub(crate) struct CoreRuntime", "pub struct CoreRuntime", 1);
    let error = verify_client_runtime_api_boundary(
        &core_module,
        &runtime_module,
        &public_runtime_type,
        &mailbox,
        &native_facade,
    )
    .expect_err("making CoreRuntime public must fail");
    assert!(
        error.contains("CoreRuntime visibility"),
        "unexpected public CoreRuntime error: {error}"
    );

    let public_mailbox = mailbox.replacen(
        "pub(crate) struct CoreInputSender",
        "pub struct CoreInputSender",
        1,
    );
    let error = verify_client_runtime_api_boundary(
        &core_module,
        &runtime_module,
        &driver,
        &public_mailbox,
        &native_facade,
    )
    .expect_err("making the completion sender public must fail");
    assert!(
        error.contains("CoreInputSender visibility"),
        "unexpected public mailbox error: {error}"
    );

    let expanded_facade = native_facade.replacen(
        "impl NativeClientRuntime {",
        "impl NativeClientRuntime {\n    pub(crate) fn runtime_mut(&mut self) {}",
        1,
    );
    let error = verify_client_runtime_api_boundary(
        &core_module,
        &runtime_module,
        &driver,
        &mailbox,
        &expanded_facade,
    )
    .expect_err("adding a second mutable facade capability must fail");
    assert!(
        error.contains("NativeClientRuntime API"),
        "unexpected expanded facade error: {error}"
    );
}

#[test]
fn native_tui_uses_one_composition_owned_client_runtime_ingress() {
    /*
     * The native adapter may name CoreInput and immutable projections, but it
     * must not assemble or drive the raw generic runtime. Keeping the concrete
     * mailbox/effect-runner graph behind one facade makes a new TUI feature use
     * the same reducer and completion ordering by construction.
     */
    assert_no_forbidden_references(BoundaryRule {
        name: "native TUI must use the composition-owned typed client-runtime ingress",
        root: "src/adapter/inbound/tui",
        forbidden_patterns: &[
            "crate::core::runtime",
            "CoreRuntime",
            "crate::composition::core_effect_runner",
            "CoreEffectRunner",
            "core_input_channel",
            ".dispatch_command(",
            ".dispatch_input(",
        ],
    });

    let composition_module = fs::read_to_string("src/composition/mod.rs")
        .expect("composition module source should load");
    assert!(
        composition_module.contains("pub(crate) mod native_client_runtime;"),
        "composition must expose the crate-private native client-runtime facade"
    );

    let facade_path = "src/composition/native_client_runtime.rs";
    let facade_source =
        fs::read_to_string(facade_path).expect("native client-runtime facade source should load");
    let production_facade = production_lines(&facade_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    let compact_facade = production_facade
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();

    for required in [
        "pub(crate)structNativeClientRuntime",
        "core:NativeCoreRuntime",
        "parallel_control_plane:ParallelModeControlPlaneHandle<NativeParallelModeControlPlaneEventSink>",
        "parallel_completion_rx:Receiver<ParallelModeControlPlaneBackgroundEvent>",
        "pub(crate)enumNativeClientEvent",
        "pub(crate)fncore(input:CoreInput)->Self",
        "Self::Core(Box::new(input))",
        "pub(crate)fndispatch_client_event(&mutself,event:NativeClientEvent,)->NativeClientDispatchOutcome",
        "NativeClientEvent::Core(input)=>self.dispatch_core_input(*input)",
    ] {
        assert!(
            compact_facade.contains(required),
            "NativeClientRuntime must retain its typed facade contract: {required}"
        );
    }
    assert_eq!(
        compact_facade
            .matches("NativeClientEvent::Core(input)=>")
            .count(),
        1,
        "NativeClientRuntime must route Core input through exactly one unified client-event arm"
    );
    let dispatch_core_input = top_level_impl_method_source(&facade_source, "dispatch_core_input");
    let compact_dispatch_core_input = rust_code_without_comments_and_literals(&dispatch_core_input)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert_eq!(
        production_callable_reference_lines(&facade_source, "dispatch_input").len(),
        1,
        "the unified client event must enter the raw core runtime exactly once"
    );
    assert!(
        compact_dispatch_core_input.contains(
            "letoutcome=self.core.runtime.dispatch_input(input);self.resolve_internal_post_turn_routes(outcome)"
        ),
        "the one raw Core ingress must immediately consume internal post-turn routing before returning to the TUI"
    );
    assert!(
        !compact_facade.contains("NativeClientEvent::ParallelCompletion"),
        "parallel worker completion must remain private to NativeClientRuntime polling"
    );
    for forbidden_api in [
        "pub(crate)fndispatch_command(",
        "pub(crate)fndispatch_input(",
        "pub(crate)fnruntime_mut(",
        "pub(crate)fneffect_runner(",
        "pub(crate)fninput_sender(",
    ] {
        assert!(
            !compact_facade.contains(forbidden_api),
            "NativeClientRuntime must not expose a second mutable ingress: {forbidden_api}"
        );
    }

    let expected_owner = facade_path.to_string();
    for assembly_call in [
        "core_input_channel()",
        "CoreEffectRunner::new(",
        "CoreRuntime::new(",
    ] {
        let mut owners = Vec::new();
        for path in rust_files_under(&repo_root().join("src/composition")) {
            if is_test_only_path(&path) {
                continue;
            }
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            let production_source = production_lines(&source)
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>()
                .join("\n");
            if production_source.contains(assembly_call) {
                owners.push(relative_path(&repo_root(), &path));
            }
        }
        assert_eq!(
            owners.as_slice(),
            std::slice::from_ref(&expected_owner),
            "raw native client-runtime assembly must have one composition owner: {assembly_call}"
        );
    }

    let tui_app_source =
        fs::read_to_string("src/adapter/inbound/tui/app.rs").expect("TUI app source should load");
    let compact_tui_app = production_lines(&tui_app_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("")
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_tui_app.contains("client_runtime:NativeClientRuntime"),
        "NativeTuiApp must retain only the opaque composition-owned client runtime"
    );
}

#[test]
fn native_client_event_routing_is_exhaustive_and_tui_background_lane_is_presentation_only() {
    let facade = fs::read_to_string("src/composition/native_client_runtime.rs")
        .expect("native client runtime source should load");
    let app_runtime = fs::read_to_string("src/adapter/inbound/tui/app/app_runtime.rs")
        .expect("TUI app runtime source should load")
        .replace("\r\n", "\n");

    verify_native_client_event_contract(&facade, &app_runtime)
        .unwrap_or_else(|error| panic!("single ClientEvent contract must remain closed: {error}"));

    let unrouted_event = facade.replacen(
        "pub(crate) enum NativeClientEvent {",
        "pub(crate) enum NativeClientEvent {\n    UnroutedSemanticMutation,",
        1,
    );
    let error = verify_native_client_event_contract(&unrouted_event, &app_runtime)
        .expect_err("a new event without an exact dispatch arm must fail");
    assert!(
        error.contains("exactly cover"),
        "unexpected unrouted event error: {error}"
    );

    let wildcard_router = facade.replacen(
        "NativeClientEvent::ClearParallelDispatchWithheldReason => {",
        "_ => {",
        1,
    );
    let error = verify_native_client_event_contract(&wildcard_router, &app_runtime)
        .expect_err("a wildcard client-event route must fail");
    assert!(
        error.contains("wildcard"),
        "unexpected wildcard router error: {error}"
    );

    let semantic_background_lane = app_runtime.replacen(
        "    #[cfg(test)]\n    ConversationRuntimeNotice(String),",
        "    ConversationRuntimeNotice(String),",
        1,
    );
    let error = verify_native_client_event_contract(&facade, &semantic_background_lane)
        .expect_err("a production semantic BackgroundMessage variant must fail");
    assert!(
        error.contains("presentation-only"),
        "unexpected semantic background-lane error: {error}"
    );
}

#[test]
fn native_tui_app_owns_exactly_four_typed_private_state_slices() {
    let app_source =
        fs::read_to_string("src/adapter/inbound/tui/app.rs").expect("TUI app source should load");
    let app_syntax = syn::parse_file(&app_source).expect("TUI app source should parse");

    let root_fields = named_struct_fields(&app_syntax, "NativeTuiApp");
    let expected_root = [
        ("shell", "NativeTuiShellState"),
        ("conversation", "NativeTuiConversationState"),
        ("planning", "NativeTuiPlanningState"),
        ("runtime", "NativeTuiRuntimeState"),
    ];
    assert_eq!(
        root_fields.len(),
        expected_root.len(),
        "NativeTuiApp must remain the four-slice host aggregate"
    );
    for (field, (expected_name, expected_type)) in root_fields.iter().zip(expected_root) {
        assert_eq!(
            field
                .ident
                .as_ref()
                .expect("NativeTuiApp field should be named"),
            expected_name,
            "NativeTuiApp slice order and names are architectural"
        );
        assert!(
            matches!(field.vis, syn::Visibility::Inherited),
            "NativeTuiApp slice {expected_name} must stay private"
        );
        assert!(
            is_named_path_type(&field.ty, expected_type),
            "NativeTuiApp.{expected_name} must use {expected_type}"
        );
    }

    let expected_slices: [(&str, &[(&str, &str)]); 4] = [
        (
            "NativeTuiShellState",
            &[
                ("chrome", "ShellChromeState"),
                ("supersession_mud_ui_state", "SupersessionMudUiState"),
                (
                    "parallel_peek_overlay_ui_state",
                    "ParallelPeekOverlayUiState",
                ),
                (
                    "progressive_activity_overlay_ui_state",
                    "ProgressiveActivityOverlayUiState",
                ),
                ("help_scroll_offset", "usize"),
                ("reviews_overlay_ui_state", "ReviewsOverlayUiState"),
                (
                    "parallel_supervisor_event_log",
                    "ParallelSupervisorEventLog",
                ),
                ("session_overlay_ui_state", "SessionOverlayUiState"),
                ("tui_language", "TuiLanguage"),
                (
                    "language_selection_overlay_ui_state",
                    "LanguageSelectionOverlayUiState",
                ),
                (
                    "model_selection_overlay_ui_state",
                    "ModelSelectionOverlayUiState",
                ),
                (
                    "view_selection_overlay_ui_state",
                    "ViewSelectionOverlayUiState",
                ),
                ("inline_history_render_mode", "InlineHistoryRenderMode"),
                ("history_insert_mode", "HistoryInsertionMode"),
                ("show_startup_ascii_art", "bool"),
            ],
        ),
        (
            "NativeTuiConversationState",
            &[
                ("lifecycle", "ConversationLifecycleState"),
                (
                    "pending_manual_prompt_preparation",
                    "Option<PendingManualPromptPreparation>",
                ),
                ("prompt_input_revision", "u64"),
                ("turn_steer_confirmation", "Option<TurnSteerUiIntent>"),
                ("pending_turn_steer", "Option<PendingTurnSteerUiIntent>"),
                ("conversation_history_identity_revision", "u64"),
                ("conversation_history_thread_id", "Option<String>"),
                ("turn_options", "ConversationTurnOptions"),
                ("conversation_view_mode", "ConversationViewMode"),
                ("auto_follow_overlay_ui_state", "AutoFollowOverlayUiState"),
            ],
        ),
        (
            "NativeTuiPlanningState",
            &[
                ("planning_ui_intent_revision", "u64"),
                (
                    "pending_resumed_session_planning_refresh",
                    "Option<PendingResumedSessionPlanningRefresh>",
                ),
                ("queue_overlay_ui_state", "QueueOverlayUiState"),
                ("queue_mutation_ui_state", "QueueMutationUiState"),
                (
                    "directions_maintenance_overlay_ui_state",
                    "DirectionsMaintenanceOverlayUiState",
                ),
                (
                    "planning_init_overlay_ui_state",
                    "PlanningInitOverlayUiState",
                ),
                (
                    "planning_runtime_refresh_ui_state",
                    "PlanningRuntimeRefreshUiState",
                ),
                (
                    "planning_workspace_operation_ui_state",
                    "PlanningWorkspaceOperationUiState",
                ),
                (
                    "planning_draft_editor_ui_state",
                    "PlanningDraftEditorUiState",
                ),
                (
                    "planning_worker_panel_state",
                    "CorePlanningWorkerPanelProjection",
                ),
                ("planning_worker_visibility", "PlanningWorkerVisibility"),
            ],
        ),
        (
            "NativeTuiRuntimeState",
            &[
                ("client_runtime", "NativeClientRuntime"),
                ("github_review_polling_state", "GithubReviewPollingState"),
                ("tx", "SyncSender<BackgroundMessage>"),
                ("rx", "Receiver<BackgroundMessage>"),
            ],
        ),
    ];
    for (slice_name, expected_fields) in expected_slices {
        let fields = named_struct_fields(&app_syntax, slice_name);
        assert_eq!(
            fields.len(),
            expected_fields.len(),
            "{slice_name} must keep its exact state-authority ledger"
        );
        for (field, (expected_name, expected_type)) in fields.iter().zip(expected_fields) {
            assert_eq!(
                field.ident.as_ref().expect("slice field should be named"),
                expected_name,
                "{slice_name} field order and names are architectural"
            );
            assert!(
                matches!(field.vis, syn::Visibility::Inherited),
                "{slice_name}.{expected_name} must stay private"
            );
            let exact_type = if let Some((outer, inner)) = expected_type.split_once('<') {
                is_single_generic_named_type(
                    &field.ty,
                    outer,
                    inner
                        .strip_suffix('>')
                        .expect("generic expected type should close"),
                )
            } else {
                is_named_path_type(&field.ty, expected_type)
            };
            assert!(
                exact_type,
                "{slice_name}.{expected_name} must use {expected_type}"
            );
        }
    }

    let protected_types = [
        "NativeTuiApp",
        "NativeTuiShellState",
        "NativeTuiConversationState",
        "NativeTuiPlanningState",
        "NativeTuiRuntimeState",
        "CorePlanningWorkerPanelProjection",
        "ShellRuntime",
    ];
    let forbidden_traits = [
        "Deref",
        "DerefMut",
        "AsRef",
        "AsMut",
        "Borrow",
        "BorrowMut",
        "Index",
        "IndexMut",
    ];
    let protected_reference_types = [
        "NativeTuiApp",
        "NativeTuiShellState",
        "NativeTuiConversationState",
        "NativeTuiPlanningState",
        "NativeTuiRuntimeState",
        "CorePlanningWorkerPanelProjection",
    ];
    let protected_reference_return = |output: &syn::ReturnType| {
        let syn::ReturnType::Type(_, ty) = output else {
            return None;
        };
        let syn::Type::Reference(reference) = ty.as_ref() else {
            return None;
        };
        protected_reference_types
            .iter()
            .copied()
            .find(|type_name| is_named_path_type(reference.elem.as_ref(), type_name))
    };
    let mut protected_source_paths =
        rust_files_under(&repo_root().join("src/adapter/inbound/tui/app"));
    protected_source_paths.push(repo_root().join("src/adapter/inbound/tui/app.rs"));
    protected_source_paths.sort();
    protected_source_paths.dedup();
    let mut violations = Vec::new();
    for path in protected_source_paths {
        if is_test_only_path(&path) {
            continue;
        }
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let syntax = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()));
        for item in &syntax.items {
            if item_is_test_only(item) {
                continue;
            }
            if let syn::Item::Fn(function) = item
                && let Some(type_name) = protected_reference_return(&function.sig.output)
            {
                violations.push(format!(
                    "{}:{}: {} returns a {type_name} reference",
                    relative_path(&repo_root(), &path),
                    function.sig.ident.span().start().line,
                    function.sig.ident
                ));
            }
            let syn::Item::Impl(item_impl) = item else {
                continue;
            };
            if attributes_are_test_only(&item_impl.attrs) {
                continue;
            }
            let protected_self = protected_types
                .iter()
                .any(|type_name| type_is_simple_path(item_impl.self_ty.as_ref(), &[*type_name]));
            if protected_self
                && let Some((_, trait_path, _)) = &item_impl.trait_
                && trait_path.segments.last().is_some_and(|segment| {
                    forbidden_traits
                        .iter()
                        .any(|forbidden| segment.ident == *forbidden)
                })
            {
                violations.push(format!(
                    "{}:{}: protected TUI state implements {}",
                    relative_path(&repo_root(), &path),
                    item_impl.impl_token.span.start().line,
                    trait_path
                        .segments
                        .last()
                        .expect("trait path should have a leaf")
                        .ident
                ));
            }
            for impl_item in &item_impl.items {
                let syn::ImplItem::Fn(method) = impl_item else {
                    continue;
                };
                if attributes_are_test_only(&method.attrs) {
                    continue;
                }
                if let Some(type_name) = protected_reference_return(&method.sig.output) {
                    violations.push(format!(
                        "{}:{}: {} returns a {type_name} reference",
                        relative_path(&repo_root(), &path),
                        method.sig.ident.span().start().line,
                        method.sig.ident
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "typed TUI slices must not expose aggregate escape hatches:\n{}",
        violations.join("\n")
    );
}

#[test]
fn native_tui_projects_core_snapshots_without_duplicate_gates_or_runtime_app_escape() {
    assert_no_forbidden_references_in_paths(
        "TUI must project correlation-accepted Core snapshots without a second gate",
        &["src/adapter/inbound/tui"],
        &[
            "pending_startup_check",
            "pending_conversation_load",
            "apply_correlated_startup_snapshot",
            "apply_correlated_conversation_snapshot",
            "StartupCheckCorrelation",
            "ConversationLoadCorrelation",
        ],
    );
    assert_no_production_callable_reference_named_in_paths(
        "production TUI code must not regain the mutable Runtime aggregate escape",
        &["src/adapter/inbound/tui"],
        "app_mut",
    );
    assert_no_forbidden_references_in_paths(
        "terminal/frontend code must consume typed Runtime projections, never NativeTuiApp",
        &[
            "src/adapter/inbound/tui/app/inline_terminal_adapter.rs",
            "src/adapter/inbound/tui/app/ratatui_frontend.rs",
        ],
        &[
            "NativeTuiApp",
            "ConversationProjectionSample::capture",
            "InlineConversationFrameProjection::from_app_with_sample",
        ],
    );

    let shell_runtime_source = fs::read_to_string("src/adapter/inbound/tui/app/shell_runtime.rs")
        .expect("shell runtime source should load");
    let shell_runtime_syntax =
        syn::parse_file(&shell_runtime_source).expect("shell runtime source should parse");
    for escape in ["app", "app_mut"] {
        assert!(
            inherent_impl_methods(&shell_runtime_syntax, "ShellRuntime", escape).is_empty(),
            "ShellRuntime::{escape} must remain test-only"
        );
    }

    let worker_projection_source =
        fs::read_to_string("src/adapter/inbound/tui/app/planning_worker_panel_projection.rs")
            .expect("planning worker projection source should load");
    let worker_projection_syntax = syn::parse_file(&worker_projection_source)
        .expect("planning worker projection source should parse");
    let production_methods = [
        "current",
        "apply_started",
        "apply_completed",
        "reset_for_conversation_lifecycle",
    ];
    for required in production_methods {
        assert_eq!(
            inherent_impl_methods(
                &worker_projection_syntax,
                "CorePlanningWorkerPanelProjection",
                required,
            )
            .len(),
            1,
            "Core planning worker projection must expose exactly one {required} transition"
        );
    }
    assert!(
        !worker_projection_source.contains("DerefMut")
            && !worker_projection_source.contains("state_mut"),
        "planning worker projection must not expose mutable domain state"
    );
    let expected_worker_mutator_sites = [
        (
            "apply_started",
            vec![("src/adapter/inbound/tui/app/app_runtime.rs", 1_usize)],
        ),
        (
            "apply_completed",
            vec![(
                "src/adapter/inbound/tui/app/post_turn_continuation.rs",
                1_usize,
            )],
        ),
        (
            "reset_for_conversation_lifecycle",
            vec![("src/adapter/inbound/tui/app/app_runtime.rs", 2_usize)],
        ),
    ];
    let app_source_root = repo_root().join("src/adapter/inbound/tui/app");
    for (mutator, expected_sites) in expected_worker_mutator_sites {
        let mut actual_sites = Vec::new();
        for path in rust_files_under(&app_source_root) {
            if is_test_only_path(&path) {
                continue;
            }
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            let call_count = production_callable_reference_lines(&source, mutator).len();
            if call_count > 0 {
                actual_sites.push((relative_path(&repo_root(), &path).to_string(), call_count));
            }
        }
        actual_sites.sort();
        let expected_sites = expected_sites
            .into_iter()
            .map(|(path, count)| (path.to_string(), count))
            .collect::<Vec<_>>();
        assert_eq!(
            actual_sites, expected_sites,
            "Core planning worker projection mutator `{mutator}` must stay on its accepted Core event settlement path"
        );
    }

    let app_runtime_source = fs::read_to_string("src/adapter/inbound/tui/app/app_runtime.rs")
        .expect("TUI runtime source should load");
    let intent_effect =
        top_level_impl_method_source(&app_runtime_source, "execute_conversation_intent_effect");
    assert!(
        !intent_effect.contains("reset_for_conversation_lifecycle"),
        "draft/session UI intent must not optimistically clear the Core worker projection"
    );
    let core_event_projection = top_level_impl_method_source(
        &app_runtime_source,
        "apply_core_event_with_previous_runtime",
    );
    let compact_core_event_projection =
        rust_code_without_comments_and_literals(&core_event_projection)
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
    assert!(
        compact_core_event_projection.contains(
            "AppEvent::ConversationRuntimeAuthorityChanged(snapshot)=>{ifprevious_runtime.post_turn.is_in_flight()&&!snapshot.post_turn.is_in_flight(){self.planning.planning_worker_panel_state.reset_for_conversation_lifecycle();}"
        ),
        "one worker reset must be gated by the exact Core post-turn in-flight to settled/cancelled authority transition"
    );
    let core_conversation_projection =
        top_level_impl_method_source(&app_runtime_source, "apply_core_conversation_snapshot");
    assert!(
        core_conversation_projection.contains("CoreConversationSnapshot::Idle")
            && core_conversation_projection.contains("CoreConversationSnapshot::Loading")
            && core_conversation_projection.contains("reset_for_conversation_lifecycle"),
        "the other worker reset must be gated by an accepted Core Idle/Loading conversation lifecycle snapshot"
    );
}

#[test]
fn parallel_runtime_notices_enter_the_typed_client_runtime() {
    let source = fs::read_to_string("src/adapter/inbound/tui/app/parallel_mode.rs")
        .expect("parallel TUI adapter source should load");
    let apply_action =
        top_level_impl_method_source(&source, "apply_parallel_mode_presentation_action");
    let compact_apply_action = apply_action
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();

    assert!(
        compact_apply_action.contains(
            "ParallelModePresentationAction::ObserveRuntimeNotice(notice)=>{self.dispatch_client_event(CoreInput::ConversationRuntimeNotice(notice));}"
        ),
        "ordinary parallel runtime notices must re-enter the root reducer through CoreInput"
    );
    for forbidden_bypass in [
        "ConversationRuntimeEvent::RuntimeNoticeObserved",
        "dispatch_conversation_runtime",
    ] {
        assert!(
            !apply_action.contains(forbidden_bypass),
            "parallel runtime notice must not bypass CoreInput through the TUI reducer: {forbidden_bypass}"
        );
    }
}

#[test]
fn global_parallel_cleanup_notices_are_owned_by_application_projection_and_pure_tail() {
    /*
     * A cleanup warning is semantic application state, not a TUI ledger. The
     * control plane publishes an owned projection; its event is only an
     * invalidation marker. Frame sampling then carries the projection into an
     * owned screen-model field before pure tail copy consumes it.
     */
    assert_no_semantic_references_in_paths(
        "production TUI must not regain a mutable global cleanup-notice ledger",
        &["src/adapter/inbound/tui"],
        &[],
        &[
            "GlobalRuntimeNoticeEntry",
            "GlobalRuntimeNoticeState",
            "record_global_runtime_notice",
            "clear_global_runtime_notice",
            "surface_global_runtime_notices_if_ready",
            "remove_ready_conversation_runtime_notice",
        ],
    );
    assert_no_forbidden_references_in_paths(
        "production TUI must not declare or access the retired global cleanup-notice ledger",
        &["src/adapter/inbound/tui"],
        &[
            "global_runtime_notice_state",
            "fn record_global_runtime_notice(",
            "fn clear_global_runtime_notice(",
            "fn surface_global_runtime_notices_if_ready(",
            "fn remove_ready_conversation_runtime_notice(",
        ],
    );

    let host_source = fs::read_to_string(
        repo_root().join("src/application/service/parallel_mode/control_plane/host.rs"),
    )
    .expect("parallel control-plane host source should load");
    let host_syntax =
        syn::parse_file(&host_source).expect("parallel control-plane host should parse");
    let presentation_fields = named_struct_fields(
        &host_syntax,
        "ParallelModeControlPlanePresentationProjection",
    );
    let global_runtime_notices = presentation_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "global_runtime_notices")
        })
        .expect("application presentation projection must own global runtime notices");
    assert!(
        is_single_generic_named_type(
            &global_runtime_notices.ty,
            "Vec",
            "ParallelModeGlobalRuntimeNoticeProjection",
        ),
        "application presentation projection must own Vec<ParallelModeGlobalRuntimeNoticeProjection>"
    );

    let control_plane_source = fs::read_to_string(
        repo_root().join("src/application/service/parallel_mode/control_plane/mod.rs"),
    )
    .expect("parallel control-plane runtime source should load");
    let control_plane_syntax = syn::parse_file(&control_plane_source)
        .expect("parallel control-plane runtime should parse");
    let notice_projection_fields = named_struct_fields(
        &control_plane_syntax,
        "ParallelModeGlobalRuntimeNoticeProjection",
    );
    assert!(
        !notice_projection_fields.is_empty()
            && notice_projection_fields
                .iter()
                .all(|field| !matches!(&field.ty, syn::Type::Reference(_))),
        "global runtime notice projection must contain owned fields without borrowed state"
    );
    assert!(
        notice_projection_fields
            .iter()
            .any(|field| is_named_path_type(&field.ty, "String")),
        "global runtime notice projection must own its rendered notice copy"
    );

    let presentation_projection =
        top_level_impl_method_source(&host_source, "presentation_projection");
    let compact_presentation_projection = presentation_projection
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_presentation_projection.contains("global_runtime_notices:service."),
        "one control-plane mutex snapshot must derive global notices from application authority"
    );

    let controller_source = fs::read_to_string(
        repo_root().join("src/application/service/parallel_mode/control_plane/controller.rs"),
    )
    .expect("parallel control-plane controller source should load");
    let controller_syntax = syn::parse_file(&controller_source)
        .expect("parallel control-plane controller should parse");
    let presentation_event = controller_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == "ParallelModeControlPlanePresentationEvent" => {
                Some(item)
            }
            _ => None,
        })
        .expect("parallel control-plane presentation event must exist");
    let marker = presentation_event
        .variants
        .iter()
        .find(|variant| variant.ident == "GlobalRuntimeNoticesChanged")
        .expect("control-plane presentation event must expose the invalidation marker");
    assert!(
        matches!(&marker.fields, syn::Fields::Unit),
        "GlobalRuntimeNoticesChanged must be a payload-free invalidation marker"
    );
    for retired_payload_variant in ["GlobalRuntimeNotice", "GlobalRuntimeNoticeCleared"] {
        assert!(
            presentation_event
                .variants
                .iter()
                .all(|variant| variant.ident != retired_payload_variant),
            "control-plane event must not publish retired payload writer {retired_payload_variant}"
        );
    }
    assert!(
        controller_source
            .matches("ParallelModeControlPlanePresentationEvent::GlobalRuntimeNoticesChanged")
            .count()
            >= 2,
        "cleanup failure and settlement must emit the payload-free presentation invalidation marker"
    );

    let bridge_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/parallel_mode/presentation_bridge.rs"),
    )
    .expect("parallel presentation bridge source should load");
    let bridge_syntax =
        syn::parse_file(&bridge_source).expect("parallel presentation bridge should parse");
    let presentation_action = bridge_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == "ParallelModePresentationAction" => Some(item),
            _ => None,
        })
        .expect("parallel presentation action must exist");
    let marker = presentation_action
        .variants
        .iter()
        .find(|variant| variant.ident == "GlobalRuntimeNoticesChanged")
        .expect("presentation bridge must retain the invalidation marker");
    assert!(
        matches!(&marker.fields, syn::Fields::Unit),
        "bridge invalidation marker must not regain cleanup payload authority"
    );
    for retired_payload_action in ["RecordGlobalRuntimeNotice", "ClearGlobalRuntimeNotice"] {
        assert!(
            presentation_action
                .variants
                .iter()
                .all(|variant| variant.ident != retired_payload_action),
            "presentation bridge must not retain payload writer {retired_payload_action}"
        );
    }
    let bridge_mapping = top_level_function_source(
        &bridge_source,
        "parallel_mode_presentation_actions_for_event",
    );
    for required_marker in [
        "ParallelModeControlPlanePresentationEvent::GlobalRuntimeNoticesChanged",
        "ParallelModePresentationAction::GlobalRuntimeNoticesChanged",
    ] {
        assert!(
            bridge_mapping.contains(required_marker),
            "presentation bridge must map the invalidation marker exhaustively: {required_marker}"
        );
    }

    let parallel_adapter_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/parallel_mode.rs"))
            .expect("parallel TUI adapter source should load");
    let apply_action = top_level_impl_method_source(
        &parallel_adapter_source,
        "apply_parallel_mode_presentation_action",
    );
    assert!(
        rust_semantic_references(&apply_action)
            .paths
            .iter()
            .any(|path| path
                .ends_with("ParallelModePresentationAction::GlobalRuntimeNoticesChanged")),
        "TUI action application must exhaustively accept the invalidation marker"
    );

    let shell_core_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_presentation/shell_core.rs"),
    )
    .expect("conversation shell-core source should load");
    let shell_core_syntax =
        syn::parse_file(&shell_core_source).expect("conversation shell-core should parse");
    let screen_model_fields = named_struct_fields(&shell_core_syntax, "ConversationScreenModel");
    let global_runtime_notices = screen_model_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "global_runtime_notices")
        })
        .expect("ConversationScreenModel must own projected global runtime notices");
    assert!(
        is_single_generic_named_type(&global_runtime_notices.ty, "Vec", "String"),
        "ConversationScreenModel.global_runtime_notices must be owned Vec<String>"
    );
    let sample_accessor = shell_core_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item) => Some(item),
            _ => None,
        })
        .filter(|item| {
            matches!(
                item.self_ty.as_ref(),
                syn::Type::Path(type_path)
                    if type_path.path.segments.last().is_some_and(|segment| {
                        segment.ident == "ConversationProjectionSample"
                    })
            )
        })
        .flat_map(|item| item.items.iter())
        .find_map(|item| match item {
            syn::ImplItem::Fn(method) if method.sig.ident == "global_runtime_notices" => {
                Some(method)
            }
            _ => None,
        })
        .expect("ConversationProjectionSample must expose global_runtime_notices");
    let mut sample_accessor_references = RustSemanticReferenceVisitor::default();
    sample_accessor_references.visit_block(&sample_accessor.block);
    assert!(
        sample_accessor_references
            .references
            .paths
            .iter()
            .any(|path| path == "global_runtime_notices"),
        "ConversationProjectionSample accessor must read the sampled application projection"
    );
    let screen_model_builder =
        top_level_impl_method_source(&shell_core_source, "from_app_with_sample");
    let screen_model_builder_references = rust_semantic_references(&screen_model_builder).paths;
    assert!(
        screen_model_builder_references
            .iter()
            .any(|path| path == "global_runtime_notices"),
        "ConversationScreenModel must derive owned notice copy through the sample accessor"
    );
    assert!(
        !screen_model_builder.contains("parallel_mode_control_plane")
            && !screen_model_builder.contains("presentation_projection"),
        "ConversationScreenModel construction must not reread application authority after sampling"
    );

    let tail_path = "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs";
    assert_no_semantic_references_in_paths(
        "pure tail copy must consume only screen-model notice data",
        &[tail_path],
        &[],
        &[
            "NativeTuiApp",
            "ParallelModeControlPlaneHandle",
            "ParallelModeControlPlanePresentationProjection",
            "ParallelModeGlobalRuntimeNoticeProjection",
        ],
    );
    assert_no_forbidden_references_in_paths(
        "pure tail copy must not pull global notices from application authority",
        &[tail_path],
        &["parallel_mode_control_plane", "presentation_projection"],
    );
    let tail_source = fs::read_to_string(repo_root().join(tail_path))
        .expect("inline tail copy source should load");
    let tail_content =
        top_level_function_source(&tail_source, "build_inline_tail_content_with_context");
    assert!(
        tail_content.contains("screen_model.global_runtime_notices"),
        "pure tail copy must read global cleanup notices only from ConversationScreenModel"
    );
}

#[test]
fn client_runtime_compile_dependencies_and_runtime_flow_are_documented_separately() {
    let english = fs::read_to_string("docs/reference/architecture.md").unwrap();
    for required in [
        "**compile-time source dependencies**",
        "**runtime flow**",
        "framework-free **Client Runtime**",
        "terminal transaction/adapter",
        "`CoreRuntime`, its effect executor, and its input mailbox are crate-private",
    ] {
        assert!(
            english.contains(required),
            "canonical architecture must document the client-runtime boundary: {required}"
        );
    }

    let korean = fs::read_to_string("docs/ko/reference/architecture.md").unwrap();
    for required in [
        "**컴파일 시점의 소스 의존성**",
        "**런타임 실행 흐름**",
        "framework 독립 **Client Runtime**",
        "terminal transaction/adapter",
        "`CoreRuntime`, effect executor, input mailbox는 library SDK가 아닌 crate-private",
    ] {
        assert!(
            korean.contains(required),
            "Korean architecture reference must document the client-runtime boundary: {required}"
        );
    }
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
     * Enforced boundary: the client runtime owns its contracts and composition
     * interprets effects, so application service types stay out of Core.
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
     * Enforced boundary: core/app exposes core-owned commands, effects, events,
     * and snapshots instead of application service DTOs.
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
     * Enforced strict boundary: Core commands, effects, inputs, events, stream
     * snapshots, and app snapshots use core/domain-owned contracts.
     */
    assert_no_forbidden_references_in_paths(
        "future core/app public contracts must be core-owned and application DTO free",
        &[
            "src/core/app/approval.rs",
            "src/core/app/command.rs",
            "src/core/app/effect.rs",
            "src/core/app/event.rs",
            "src/core/app/manual_prompt.rs",
            "src/core/app/projection.rs",
            "src/core/app/queue.rs",
            "src/core/app/review_center.rs",
            "src/core/app/snapshot.rs",
            "src/core/app/controller/state.rs",
            "src/core/app/turn_steer.rs",
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
     * Enforced boundary: core/runtime keeps only the effect contract and never
     * stores concrete application services.
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
     * Enforced boundary: composition implements the effect facade; core/runtime
     * does not import service modules or call application services directly.
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
fn core_effect_workers_share_one_redacted_panic_totality_boundary() {
    let runner = fs::read_to_string("src/composition/core_effect_runner.rs")
        .expect("core effect runner source should load");
    let turn_submission = fs::read_to_string("src/composition/core_turn_submission.rs")
        .expect("turn submission worker source should load");
    let worker = fs::read_to_string("src/composition/core_effect_worker.rs")
        .expect("shared core effect worker source should load");

    let mut raw_spawn_owners = Vec::new();
    for path in rust_files_under(&repo_root().join("src/composition")) {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for line in production_callable_reference_lines(&source, "spawn") {
            raw_spawn_owners.push(format!("{}:{line}", path.display()).replace('\\', "/"));
        }
    }
    assert!(
        raw_spawn_owners.len() == 1
            && raw_spawn_owners[0].contains("src/composition/core_effect_worker.rs:"),
        "the shared worker boundary must own the only production thread spawn in composition: {raw_spawn_owners:?}"
    );
    assert!(
        worker.contains("catch_redacted_worker_unwind(work)")
            && worker.contains("spawn_effect_completion_worker")
            && worker.contains("spawn_worker_with_panic_fallback")
            && worker.contains("spawn_joinable_redacted_worker"),
        "one-shot, outer turn, and joined stream workers must all use the shared redacted boundary"
    );

    let production_runner = production_source_before_inline_tests(&runner);
    assert!(
        !production_runner.contains("pub fn spawn_"),
        "Core effect worker entrypoints must stay private behind exhaustive CoreEffect dispatch"
    );
    assert!(
        production_runner.contains("failed_review_center_snapshot(")
            && production_runner.contains("queue_mutation_panic_completion(")
            && production_runner.contains("post_turn_evaluation_failure_execution("),
        "non-Result completion shapes must keep explicit typed panic settlements"
    );
    assert!(
        turn_submission.contains("settle_turn_submission_panic(")
            && turn_submission.contains("finalize_after_stream_completion(true)")
            && !turn_submission.contains("panic_payload_summary"),
        "turn panic recovery must settle the slot lifecycle without exposing panic payloads"
    );

    let post_turn = fs::read_to_string("src/application/service/post_turn_evaluation.rs")
        .expect("post-turn evaluation source should load");
    let production_post_turn = production_source_before_inline_tests(&post_turn);
    assert!(
        production_post_turn.contains("catch_redacted_worker_unwind(|| service.evaluate(request))")
            && !production_post_turn.contains("std::panic::catch_unwind"),
        "the nested post-turn evaluator must preserve the redacted panic boundary"
    );
}

#[test]
fn core_effect_dispatch_is_ast_exhaustive_and_completion_total() {
    let effect_source =
        fs::read_to_string("src/core/app/effect.rs").expect("CoreEffect source should load");
    let runner_source = fs::read_to_string("src/composition/core_effect_runner.rs")
        .expect("CoreEffectRunner source should load");
    let turn_submission_source = fs::read_to_string("src/composition/core_turn_submission.rs")
        .expect("turn submission worker source should load");

    verify_core_effect_totality_contract(&effect_source, &runner_source, &turn_submission_source)
        .unwrap_or_else(|error| panic!("CoreEffect totality contract violated: {error}"));
}

#[test]
fn core_effect_totality_analyzer_rejects_dispatch_and_sink_bypasses() {
    let effect_source =
        fs::read_to_string("src/core/app/effect.rs").expect("CoreEffect source should load");
    let runner_source = fs::read_to_string("src/composition/core_effect_runner.rs")
        .expect("CoreEffectRunner source should load");
    let turn_submission_source = fs::read_to_string("src/composition/core_turn_submission.rs")
        .expect("turn submission worker source should load");
    let reject_runner = |source: String, expected| {
        assert_core_effect_totality_rejects(
            &effect_source,
            &source,
            &turn_submission_source,
            expected,
        );
    };

    assert_core_effect_totality_rejects(
        &effect_source.replacen(
            "pub enum CoreEffect {",
            "pub enum CoreEffect {\n    Unsettled,",
            1,
        ),
        &runner_source,
        &turn_submission_source,
        "enum variants must exactly match",
    );
    reject_runner(
        runner_source.replacen("CoreEffect::RunStartupChecks { correlation } =>", "_ =>", 1),
        "one exact CoreEffect variant",
    );
    reject_runner(
        runner_source.replacen(
            "CoreEffect::RunStartupChecks { correlation } =>",
            "CoreEffect::RunStartupChecks { correlation } if true =>",
            1,
        ),
        "must not use a match guard",
    );
    reject_runner(
        runner_source.replacen(
            "self.spawn_startup_checks(correlation);",
            "if true { self.spawn_startup_checks(correlation); }",
            1,
        ),
        "one unconditional top-level launcher",
    );
    reject_runner(
        runner_source.replacen(
            "self.spawn_startup_checks(correlation);",
            "self.startup_service.run_checks(\".\");\n                self.spawn_startup_checks(correlation);",
            1,
        ),
        "exact audited dispatch shape",
    );
    reject_runner(
        runner_source.replacen(
            "spawn_effect_completion_worker(input_sender, panic_completion, move || {",
            "run_unsettled_worker(input_sender, panic_completion, move || {",
            1,
        ),
        "one audited top-level sink",
    );
    reject_runner(
        runner_source.replacen(
            "CoreEffectRunner::run_effect(self, effect)",
            "self.run_effect(effect)",
            1,
        ),
        "must delegate exactly",
    );
}

#[test]
fn manual_prompt_and_stop_provider_io_never_run_inline_in_core_effect_dispatch() {
    let runner = fs::read_to_string("src/composition/core_effect_runner.rs")
        .expect("core effect runner source should load");
    let run_effect = runner
        .split_once("pub fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {")
        .and_then(|(_, body)| body.split_once("fn spawn_session_catalog_load("))
        .map(|(body, _)| body)
        .expect("core effect dispatch should have a bounded source body");

    assert!(
        run_effect.contains("self.spawn_manual_prompt_preparation(*request, permit);")
            && run_effect
                .contains("self.spawn_stop_request_attempt(correlation, attempt, permit);"),
        "manual prompt preparation and stop requests must dispatch their background workers"
    );
    assert!(
        run_effect.contains("CoreEffect::CancelManualPromptPreparation { correlation }")
            && run_effect.contains("CoreEffect::InvalidateStopRequest { correlation }")
            && run_effect.contains(".invalidate(correlation.generation);"),
        "manual cancellation and stop lifecycle invalidation must reach the worker permits"
    );
    assert!(
        !run_effect.contains(".prepare(") && !run_effect.contains(".request_stop_all_sessions("),
        "core effect dispatch must not execute manual preparation or stop provider I/O inline"
    );

    let manual_worker = runner
        .split_once("fn spawn_manual_prompt_preparation(")
        .and_then(|(_, body)| body.split_once("fn spawn_stop_request_attempt("))
        .map(|(body, _)| body)
        .expect("manual preparation worker should have a bounded source body");
    let stop_worker = runner
        .split_once("fn spawn_stop_request_attempt(")
        .and_then(|(_, body)| body.split_once("fn spawn_approval_decision_submission("))
        .map(|(body, _)| body)
        .expect("stop request worker should have a bounded source body");
    assert!(
        manual_worker.contains("spawn_effect_completion_worker_with_recovery(")
            && manual_worker.contains("catch_redacted_worker_unwind(")
            && manual_worker.contains("service.prepare_guarded(request, &|| permit.is_active())"),
        "manual preparation must run behind its cancellation and panic boundary"
    );
    assert!(
        stop_worker.contains("spawn_effect_completion_worker_with_recovery(")
            && stop_worker.contains("catch_redacted_worker_unwind(")
            && stop_worker.contains("if !permit.is_active()")
            && stop_worker.contains("conversation_service.request_stop_all_sessions()"),
        "stop provider execution must run behind its lifecycle and panic boundary"
    );
}

#[test]
fn parallel_control_plane_effect_dispatch_is_ast_exhaustive_and_panic_total() {
    let effect_source =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/mod.rs")
            .expect("parallel control-plane effect source should load");
    let controller_source =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/controller.rs")
            .expect("parallel control-plane controller source should load")
            .replace("\r\n", "\n");
    let runner_source =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/effect_runner.rs")
            .expect("parallel control-plane effect runner source should load");

    verify_parallel_control_plane_effect_totality_contract(
        &effect_source,
        &controller_source,
        &runner_source,
    )
    .unwrap_or_else(|error| {
        panic!("parallel control-plane effect totality contract violated: {error}")
    });
}

#[test]
fn parallel_control_plane_effect_totality_analyzer_rejects_bypasses_without_fixture_noise() {
    let effect_source =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/mod.rs")
            .expect("parallel control-plane effect source should load");
    let controller_source =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/controller.rs")
            .expect("parallel control-plane controller source should load")
            .replace("\r\n", "\n");
    let runner_source =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/effect_runner.rs")
            .expect("parallel control-plane effect runner source should load")
            .replace("\r\n", "\n");
    let reject = |effect: &str, controller: &str, runner: &str, expected: &str| {
        let error =
            verify_parallel_control_plane_effect_totality_contract(effect, controller, runner)
                .expect_err("mutated parallel effect contract must be rejected");
        assert!(
            error.contains(expected),
            "unexpected parallel effect analyzer failure; expected `{expected}`, got `{error}`"
        );
    };

    reject(
        &effect_source.replacen(
            "pub enum ParallelModeControlPlaneEffect {",
            "pub enum ParallelModeControlPlaneEffect {\n    Unsettled,",
            1,
        ),
        &controller_source,
        &runner_source,
        "enum variants must exactly match",
    );
    reject(
        &effect_source,
        &controller_source.replacen(
            "ParallelModeControlPlaneEffect::EnterParallelMode {",
            "_ /* hidden bypass */ | ParallelModeControlPlaneEffect::EnterParallelMode {",
            1,
        ),
        &runner_source,
        "or-patterns are forbidden",
    );
    reject(
        &effect_source,
        &controller_source.replacen(
            "initial_pool_reset_required,\n            } => {",
            "initial_pool_reset_required,\n            } if true => {",
            1,
        ),
        &runner_source,
        "must not use a match guard",
    );
    reject(
        &effect_source,
        &controller_source.replacen(
            "                    return self.drain_outcome(outcome);",
            "                    return Vec::new();",
            1,
        ),
        &runner_source,
        "explicit synchronous settlement",
    );
    reject(
        &effect_source,
        &controller_source,
        &runner_source.replacen(
            "spawn_parallel_effect_completion_worker(event_sink, panic_completion, move || {",
            "spawn_unchecked_worker(event_sink, panic_completion, move || {",
            1,
        ),
        "shared panic-total completion sink",
    );

    let fixture_controller = format!(
        "{controller_source}\n#[cfg(test)] mod bypass_fixture {{ fn run_effect() {{ match () {{ _ if true => std::thread::spawn(|| {{}}), _ => unreachable!() }}; }} }}",
    );
    let fixture_runner = format!(
        "{runner_source}\n#[cfg(test)] fn spawn_unchecked_fixture() {{ std::thread::spawn(|| panic!(\"fixture\")); }}",
    );
    verify_parallel_control_plane_effect_totality_contract(
        &effect_source,
        &fixture_controller,
        &fixture_runner,
    )
    .expect("test-only bypass fixtures must not create production false positives");
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
fn parallel_supervisor_inspection_never_runs_inline_under_the_control_plane_mutex() {
    assert_no_forbidden_references_in_paths(
        "parallel supervisor inspection must dispatch a worker before touching filesystem, Git, or SQLite state",
        &["src/application/service/parallel_mode/control_plane/controller.rs"],
        &[
            ".inspect_supervisor(",
            ".load_runtime_projection_or_invalid(",
            ".inspect_readiness(",
            ".build_supervisor_snapshot(",
            ".reconcile_supervisor_snapshot(",
        ],
    );

    let runner =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/effect_runner.rs")
            .expect("parallel control-plane effect runner source should load")
            .replace("\r\n", "\n");
    let inspection_worker = runner
        .split_once("pub fn spawn_supervisor_inspection(")
        .and_then(|(_, body)| body.split_once("pub fn spawn_orchestrator_tick("))
        .map(|(body, _)| body)
        .expect("parallel supervisor inspection worker should have a bounded source body");
    assert!(
        runner.contains("pub fn spawn_supervisor_inspection(")
            && inspection_worker.contains(".reconcile_supervisor_snapshot_guarded(")
            && inspection_worker.contains(
                "ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted",
            ),
        "parallel supervisor inspection must reconcile through an epoch guard and complete through the background worker event path"
    );
    assert!(
        !inspection_worker.contains(".reconcile_supervisor_snapshot("),
        "the async inspection worker must not call the unguarded reconciliation path"
    );
    let parallel_service = fs::read_to_string("src/application/service/parallel_mode/mod.rs")
        .expect("parallel mode service source should load");
    assert!(
        parallel_service.contains("fn reconcile_supervisor_snapshot_guarded(")
            && parallel_service.contains("permit.with_active_commit(reconcile)"),
        "the guarded inspection reconcile must linearize its final pool mutation with epoch cancellation"
    );
}

#[test]
fn parallel_pending_dispatch_poll_never_reads_authority_under_the_control_plane_mutex() {
    assert_no_forbidden_references_in_paths(
        "parallel pending-dispatch polling must dispatch a worker before reading SQLite or filesystem authority",
        &[
            "src/application/service/parallel_mode/control_plane/controller.rs",
            "src/adapter/inbound/tui/app/parallel_mode.rs",
            "src/adapter/inbound/tui/app/shell_runtime.rs",
        ],
        &[
            ".pending_dispatch_wake(",
            ".load_runtime_projections(",
            ".load_runtime_projection_or_invalid(",
        ],
    );

    let controller =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/controller.rs")
            .expect("parallel control-plane controller source should load");
    assert!(
        controller.contains(".spawn_pending_dispatch_wake_poll(correlation);"),
        "the controller must dispatch pending-wake authority reads through the async effect runner"
    );
    let runner =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/effect_runner.rs")
            .expect("parallel control-plane effect runner source should load")
            .replace("\r\n", "\n");
    let poll_worker = runner
        .split_once("pub fn spawn_pending_dispatch_wake_poll(")
        .and_then(|(_, body)| body.split_once("pub fn spawn_dispatch_command_mutation("))
        .map(|(body, _)| body)
        .expect("pending dispatch poll worker should have a bounded source body");
    assert!(
        poll_worker.contains("spawn_parallel_effect_completion_worker(")
            && poll_worker.contains(".pending_dispatch_wake(")
            && poll_worker
                .contains("ParallelModeControlPlaneBackgroundEvent::PendingDispatchWakePolled"),
        "pending dispatch polling must perform authority I/O inside the panic-total completion worker"
    );
    let runtime = fs::read_to_string("src/application/service/parallel_mode/control_plane/mod.rs")
        .expect("parallel control-plane runtime source should load");
    assert!(
        runtime.contains("ParallelModePendingDispatchPollCorrelation")
            && runtime.contains("pending_dispatch_poll_in_flight"),
        "pending dispatch polling must retain typed operation/workspace/epoch correlation before worker dispatch"
    );
}

#[test]
fn parallel_dispatch_mutations_never_run_inline_under_the_control_plane_mutex() {
    assert_no_forbidden_references_in_paths(
        "parallel dispatch enqueue and cancellation must dispatch a worker before touching durable authority",
        &[
            "src/application/service/parallel_mode/control_plane/controller.rs",
            "src/application/service/parallel_mode/control_plane/host.rs",
            "src/adapter/inbound/tui/app/parallel_mode.rs",
        ],
        &[
            ".load_runtime_projection_or_invalid(",
            ".enqueue_dispatch_commands_for_event(",
            ".enqueue_dispatch_commands_for_trigger(",
            ".cancel_dispatch_commands(",
        ],
    );

    let runner =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/effect_runner.rs")
            .expect("parallel control-plane effect runner source should load")
            .replace("\r\n", "\n");
    let mutation_worker = runner
        .split_once("pub fn spawn_dispatch_command_mutation(")
        .and_then(|(_, body)| {
            body.split_once("\n    }\n}\n\nfn supervisor_refresh_started_payload")
        })
        .map(|(body, _)| body)
        .expect("parallel dispatch mutation worker should have a bounded source body");
    let (before_spawn, worker_closure) = mutation_worker
        .split_once("spawn_parallel_effect_completion_worker(")
        .expect("parallel dispatch mutation must use the audited completion worker");
    for forbidden in [
        ".load_runtime_projection_or_invalid(",
        ".enqueue_dispatch_commands_for_event(",
        ".enqueue_dispatch_commands_for_trigger(",
        ".cancel_dispatch_commands(",
    ] {
        assert!(
            !before_spawn.contains(forbidden),
            "parallel dispatch mutation must not perform {forbidden} before spawning its worker"
        );
    }
    for required in [
        ".load_runtime_projection_or_invalid(",
        ".enqueue_dispatch_commands_for_event(",
        ".enqueue_dispatch_commands_for_trigger(",
        ".cancel_dispatch_commands(",
        "ParallelModeControlPlaneBackgroundEvent::DispatchMutationCompleted",
    ] {
        assert!(
            worker_closure.contains(required),
            "parallel dispatch mutation closure must exclusively own {required}"
        );
    }

    let runtime = fs::read_to_string("src/application/service/parallel_mode/control_plane/mod.rs")
        .expect("parallel control-plane runtime source should load");
    for required in [
        "ParallelModeDispatchMutationCorrelation",
        "ParallelModeDispatchCleanupCorrelation",
        "dispatch_mutation_in_flight",
        "next_dispatch_mutation_operation_id",
        "pending_dispatch_mutations: VecDeque",
        "canonical_enqueue_trigger",
        "RetryUnsettledDispatchCleanup",
        "unsettled_dispatch_cleanups: VecDeque",
        "MAX_UNSETTLED_DISPATCH_CLEANUPS",
    ] {
        assert!(
            runtime.contains(required),
            "dispatch mutation ordering must retain {required}"
        );
    }

    let controller =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/controller.rs")
            .expect("parallel control-plane controller source should load");
    let pulse = controller
        .split_once("    pub fn tick(")
        .and_then(|(_, body)| body.split_once("    fn cleanup_retry_interval_due("))
        .map(|(body, _)| body)
        .expect("parallel control-plane pulse should have a bounded source body");
    let refresh_position = pulse
        .find("ParallelModeControlPlaneCommand::RefreshSupervisor")
        .expect("pulse should retain supervisor refresh");
    let poll_position = pulse
        .find("poll_pending_dispatch_wake")
        .expect("pulse should retain pending wake poll");
    let cleanup_fallback_position = pulse
        .rfind("start_cleanup_retry")
        .expect("pulse should retain a bounded cleanup fallback");
    assert!(
        pulse.contains("oldest_unsettled_dispatch_cleanup")
            && pulse.contains("cleanup_retry_deferred")
            && refresh_position < poll_position
            && poll_position < cleanup_fallback_position,
        "the production pulse must preserve refresh then poll priority before its fair cleanup fallback"
    );
    let cleanup_driver = controller
        .split_once("    fn start_cleanup_retry(")
        .and_then(|(_, body)| body.split_once("    pub fn supervisor_refresh_due("))
        .map(|(body, _)| body)
        .expect("cleanup retry driver should have a bounded source body");
    assert!(
        cleanup_driver.contains("RetryUnsettledDispatchCleanup")
            && cleanup_driver.contains("last_cleanup_retry_at")
            && !cleanup_driver.contains("last_orchestrator_wake_poll_at"),
        "typed cleanup retry cadence must not consume the pending-wake poll cadence"
    );

    let shell_runtime = fs::read_to_string("src/adapter/inbound/tui/app/shell_runtime.rs")
        .expect("TUI shell runtime source should load");
    assert!(
        shell_runtime
            .contains("tick_parallel_mode_control_plane(now, &parallel_presentation_sample)",),
        "the production TUI shell pulse must drive the parallel control plane"
    );
}

#[test]
fn parallel_post_turn_continuation_reuses_the_accepted_runtime_projection() {
    let runner =
        fs::read_to_string("src/application/service/parallel_mode/control_plane/effect_runner.rs")
            .expect("parallel control-plane effect runner source should load");
    assert!(
        !runner.contains("continue_post_turn_queue_command"),
        "post-turn continuation must not rebuild its command by rereading planning authority"
    );

    let post_turn_execution = fs::read_to_string(
        "src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs",
    )
    .expect("TUI post-turn execution source should load");
    assert!(
        post_turn_execution.contains("outcome.runtime_projection.has_actionable_queue_head()")
            && post_turn_execution.contains(".with_runtime_projection_routing("),
        "the exact accepted post-turn runtime projection must carry workspace and queue-head authority into routing"
    );

    let native_runtime = fs::read_to_string("src/composition/native_client_runtime.rs")
        .expect("native client runtime source should load");
    let route = top_level_impl_method_source(&native_runtime, "resolve_internal_post_turn_routes");
    let compact_route = rust_code_without_comments_and_literals(&route)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let exact_route =
        top_level_function_source(&native_runtime, "exact_single_post_turn_routing_request");
    let compact_exact_route = rust_code_without_comments_and_literals(&exact_route)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_route.contains(
            "exact_single_post_turn_routing_request(core_outcome.snapshot.as_ref(),&routing_requests"
        ) && compact_route.contains(
            "route_parallel_post_turn_for_exact_single(core_outcome.snapshot.as_ref(),&routing_requests"
        ) && compact_route.contains(
            "request.execution.evaluation.runtime_projection.has_actionable_queue_head()"
        ) && compact_exact_route.contains("let[request]=routing_requestselse{returnNone;}")
            && compact_exact_route.contains(
                "pending_post_turn_route_correlation(snapshot)==Some(&request.correlation)"
            ),
        "NativeClientRuntime must route only the exact Core AwaitingRoute correlation and reuse its accepted queue-head projection"
    );
    assert_eq!(
        production_callable_reference_lines(&native_runtime, "continue_post_turn_queue").len(),
        1,
        "the private native runtime may offer an exact post-turn route to the parallel control-plane at most once"
    );
    let parallel_adapter = fs::read_to_string("src/adapter/inbound/tui/app/parallel_mode.rs")
        .expect("TUI parallel adapter source should load");
    assert!(
        production_callable_reference_lines(&parallel_adapter, "continue_post_turn_queue")
            .is_empty(),
        "the TUI adapter must not regain direct post-turn parallel routing authority"
    );
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
fn native_tui_prompt_log_maintenance_runs_only_inside_the_startup_worker() {
    const FORBIDDEN_SYNCHRONOUS_MAINTENANCE_CALLS: &[&str] = &[
        "maintain_prompt_logs_best_effort",
        "maintain_app_server_prompt_logs",
        "clear_app_server_prompt_interaction_records",
        "purge_expired_app_server_prompt_interaction_records",
    ];

    let production = fs::read_to_string("src/composition/production.rs")
        .expect("production composition source should load");
    let native_builder_calls =
        reachable_callable_expression_names(&production, "build_native_tui_application");
    for forbidden in FORBIDDEN_SYNCHRONOUS_MAINTENANCE_CALLS {
        assert!(
            !native_builder_calls.iter().any(|call| call == forbidden),
            "native TUI production builder must not perform direct prompt-log storage maintenance: {forbidden}"
        );
    }
    assert!(
        native_builder_calls
            .iter()
            .any(|call| call == "build_shared_ports_for_prompt_logging")
            && native_builder_calls
                .iter()
                .any(|call| call == "with_prompt_log_maintenance"),
        "native TUI production builder must inject maintenance into StartupService"
    );

    let shell_entrypoint = fs::read_to_string("src/adapter/inbound/tui/app/shell_entrypoint.rs")
        .expect("TUI shell entrypoint source should load");
    let shell_entrypoint_calls = production_call_expression_names(&shell_entrypoint);
    for forbidden in FORBIDDEN_SYNCHRONOUS_MAINTENANCE_CALLS {
        assert!(
            !shell_entrypoint_calls.iter().any(|call| call == forbidden),
            "TUI entrypoints must not perform direct prompt-log storage maintenance: {forbidden}"
        );
    }

    let runner = fs::read_to_string("src/composition/core_effect_runner.rs")
        .expect("core effect runner source should load");
    let startup_worker_calls =
        named_function_call_expression_names(&runner, "spawn_startup_checks");
    assert!(
        startup_worker_calls
            .iter()
            .any(|call| call == "spawn_effect_completion_worker")
            && startup_worker_calls
                .iter()
                .any(|call| call == "guarded_startup_checks_completion"),
        "prompt-log maintenance and startup checks must be admitted through the background startup worker"
    );

    let startup_service = fs::read_to_string("src/application/service/startup_service.rs")
        .expect("startup service source should load");
    let startup_checks_calls = named_function_call_expression_names(&startup_service, "run_checks");
    assert!(
        startup_checks_calls
            .iter()
            .any(|call| call == "run_checks_with_local_prerequisites"),
        "production startup checks must use the orchestration path covered by the maintenance/probe order test"
    );
    let guarded_startup_calls =
        named_function_call_expression_names(&runner, "guarded_startup_checks_completion");
    assert!(
        guarded_startup_calls
            .iter()
            .any(|call| call == "catch_redacted_worker_unwind"),
        "startup provider execution must use the shared redacted worker panic boundary"
    );
}

#[test]
fn rust_call_graph_guard_ignores_text_but_follows_indirect_helpers() {
    let text_only_source = r#"
        fn entry() {
            // maintain_prompt_logs_best_effort();
            let _example = "maintain_prompt_logs_best_effort()";
        }
    "#;
    assert!(
        !reachable_callable_expression_names(text_only_source, "entry")
            .iter()
            .any(|call| call == "maintain_prompt_logs_best_effort")
    );

    let indirect_source = r#"
        fn entry() {
            shared_ports();
        }

        fn shared_ports() {
            privacy_helper();
        }

        fn privacy_helper() {
            maintain_prompt_logs_best_effort();
        }
    "#;
    assert!(
        reachable_callable_expression_names(indirect_source, "entry")
            .iter()
            .any(|call| call == "maintain_prompt_logs_best_effort"),
        "call graph inspection must catch synchronous maintenance hidden behind a shared-port helper"
    );

    let method_and_alias_source = r#"
        struct SharedPorts;

        impl SharedPorts {
            fn prepare(&self) {
                let cleanup = maintain_prompt_logs_best_effort;
                cleanup();
            }
        }

        fn entry() {
            SharedPorts.prepare();
        }
    "#;
    assert!(
        reachable_callable_expression_names(method_and_alias_source, "entry")
            .iter()
            .any(|call| call == "maintain_prompt_logs_best_effort"),
        "call graph inspection must catch maintenance reached through a method and function alias"
    );
}

#[test]
fn session_catalog_load_contract_keeps_admission_and_identity_in_core() {
    let request_source =
        fs::read_to_string("src/core/app/request.rs").expect("core request source should load");
    let request_syntax =
        syn::parse_file(&request_source).expect("core request source should parse");

    let load_mode = request_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == "SessionCatalogLoadMode" => Some(item),
            _ => None,
        })
        .expect("core request contracts must define SessionCatalogLoadMode");
    assert_eq!(
        load_mode
            .variants
            .iter()
            .map(|variant| variant.ident.to_string())
            .collect::<Vec<_>>(),
        vec!["EnsureLoaded".to_string(), "Refresh".to_string()],
        "catalog load mode must distinguish idempotent ensure from explicit refresh"
    );
    assert!(
        load_mode
            .variants
            .iter()
            .all(|variant| matches!(variant.fields, syn::Fields::Unit)),
        "catalog load modes must remain data-free policy markers"
    );

    let intent_fields = named_struct_fields(&request_syntax, "SessionCatalogLoadIntent");
    assert_eq!(
        intent_fields
            .iter()
            .map(|field| {
                field
                    .ident
                    .as_ref()
                    .expect("catalog intent field should be named")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        vec![
            "mode".to_string(),
            "limit".to_string(),
            "workspace_directory".to_string(),
        ],
        "catalog intent must carry mode and the complete provider target"
    );
    assert!(
        is_named_path_type(&intent_fields[0].ty, "SessionCatalogLoadMode")
            && is_named_path_type(&intent_fields[1].ty, "usize")
            && is_named_path_type(&intent_fields[2].ty, "String"),
        "catalog intent field types must remain mode/usize/String"
    );

    let correlation_fields = named_struct_fields(&request_syntax, "SessionCatalogLoadCorrelation");
    assert_eq!(
        correlation_fields
            .iter()
            .map(|field| {
                field
                    .ident
                    .as_ref()
                    .expect("catalog correlation field should be named")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        vec![
            "generation".to_string(),
            "limit".to_string(),
            "workspace_directory".to_string(),
        ],
        "catalog correlation must bind generation to the complete provider target"
    );
    assert!(
        is_named_path_type(&correlation_fields[0].ty, "u64")
            && is_named_path_type(&correlation_fields[1].ty, "usize")
            && is_named_path_type(&correlation_fields[2].ty, "String"),
        "catalog correlation field types must remain u64/usize/String"
    );
    let correlation = request_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "SessionCatalogLoadCorrelation" => Some(item),
            _ => None,
        })
        .expect("core request contracts must define SessionCatalogLoadCorrelation");
    let correlation_derives_copy = correlation
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("derive"))
        .any(|attribute| {
            attribute
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
                .expect("catalog correlation derive should parse")
                .iter()
                .any(|path| path.is_ident("Copy"))
        });
    let correlation_implements_copy = request_syntax.items.iter().any(|item| {
        let syn::Item::Impl(item) = item else {
            return false;
        };
        item.trait_
            .as_ref()
            .is_some_and(|(_, path, _)| path.is_ident("Copy"))
            && is_named_path_type(item.self_ty.as_ref(), "SessionCatalogLoadCorrelation")
    });
    assert!(
        !correlation_derives_copy && !correlation_implements_copy,
        "catalog correlation owns a String target and must not regain Copy semantics"
    );

    let command_source =
        fs::read_to_string("src/core/app/command.rs").expect("core command source should load");
    let command_syntax =
        syn::parse_file(&command_source).expect("core command source should parse");
    let load_command = command_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == "AppCommand" => item
                .variants
                .iter()
                .find(|variant| variant.ident == "LoadSessionCatalog"),
            _ => None,
        })
        .expect("AppCommand must retain a catalog-load variant");
    let syn::Fields::Unnamed(load_command_fields) = &load_command.fields else {
        panic!("catalog-load command must wrap one typed intent");
    };
    assert!(
        load_command_fields.unnamed.len() == 1
            && is_named_path_type(
                &load_command_fields
                    .unnamed
                    .first()
                    .expect("one catalog intent field")
                    .ty,
                "SessionCatalogLoadIntent",
            ),
        "AppCommand must carry exactly one SessionCatalogLoadIntent"
    );

    let effect_source =
        fs::read_to_string("src/core/app/effect.rs").expect("core effect source should load");
    let effect_syntax = syn::parse_file(&effect_source).expect("core effect source should parse");
    let load_effect = effect_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == "CoreEffect" => item
                .variants
                .iter()
                .find(|variant| variant.ident == "LoadSessionCatalog"),
            _ => None,
        })
        .expect("CoreEffect must retain a catalog-load variant");
    let syn::Fields::Named(load_effect_fields) = &load_effect.fields else {
        panic!("catalog-load effect must use one named correlation field");
    };
    assert!(
        load_effect_fields.named.len() == 1
            && load_effect_fields.named[0]
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "correlation")
            && is_named_path_type(
                &load_effect_fields.named[0].ty,
                "SessionCatalogLoadCorrelation",
            ),
        "CoreEffect must derive provider target data from one full catalog correlation"
    );

    let controller_source = fs::read_to_string("src/core/app/controller.rs")
        .expect("core controller source should load");
    let session_reducer_source = fs::read_to_string("src/core/app/session_reducer.rs")
        .expect("session feature reducer source should load");
    let session_reducer_syntax =
        syn::parse_file(&session_reducer_source).expect("session feature reducer should parse");
    let deferred_catalog_load =
        named_struct_fields(&session_reducer_syntax, "SessionFeatureReducer")
            .into_iter()
            .find(|field| {
                field
                    .ident
                    .as_ref()
                    .is_some_and(|ident| ident == "deferred_catalog_load")
            })
            .expect("SessionFeatureReducer must retain deferred catalog intent");
    assert!(
        is_single_generic_named_type(
            &deferred_catalog_load.ty,
            "Option",
            "SessionCatalogLoadIntent",
        ),
        "deferred catalog work must retain the typed intent, not an uncorrelated tuple"
    );
    let deferred_reads =
        top_level_impl_method_source(&controller_source, "start_deferred_session_reads");
    let compact_deferred_reads = rust_code_without_comments_and_literals(&deferred_reads)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_deferred_reads.contains("self.admit_session_catalog_load(intent)")
            && !compact_deferred_reads.contains("self.start_session_catalog_load("),
        "deferred catalog work must re-enter common Core admission instead of bypassing it"
    );
}

#[test]
fn core_session_feature_reducer_owns_only_the_session_lifecycle_slice() {
    let controller_source = fs::read_to_string("src/core/app/controller.rs")
        .expect("core controller source should load");
    let controller_syntax =
        syn::parse_file(&controller_source).expect("core controller source should parse");
    let controller_fields = named_struct_fields(&controller_syntax, "CoreController");
    let session_feature = controller_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "session_feature")
        })
        .expect("CoreController must own one SessionFeatureReducer slice");
    assert!(
        is_named_path_type(&session_feature.ty, "SessionFeatureReducer"),
        "session_feature must be the typed session reducer"
    );
    assert!(
        controller_fields.iter().all(|field| {
            field
                .ident
                .as_ref()
                .is_none_or(|ident| ident != "guarded_session_rename_stream")
        }),
        "the session-rename/turn-stream bridge belongs to the conversation-turn reducer"
    );
    for field in &controller_fields {
        let field_name = field
            .ident
            .as_ref()
            .expect("CoreController field should be named")
            .to_string();
        if field_name == "session_feature" {
            continue;
        }
        assert!(
            !field_name.contains("session"),
            "CoreController session state must live in session_feature; unexpected field: {field_name}"
        );
        for forbidden_type in [
            "SessionCatalogLoadCorrelation",
            "SessionCatalogLoadIntent",
            "SessionRenameCorrelation",
        ] {
            assert!(
                !type_mentions_named_path(&field.ty, forbidden_type),
                "CoreController field {field_name} must not hide session authority type {forbidden_type}"
            );
        }
    }

    let reducer_source = fs::read_to_string("src/core/app/session_reducer.rs")
        .expect("session feature reducer source should load");
    let reducer_syntax =
        syn::parse_file(&reducer_source).expect("session feature reducer should parse");
    let reducer_fields = named_struct_fields(&reducer_syntax, "SessionFeatureReducer")
        .into_iter()
        .map(|field| {
            field
                .ident
                .as_ref()
                .expect("session reducer field should be named")
                .to_string()
        })
        .collect::<HashSet<_>>();
    assert_eq!(
        reducer_fields,
        HashSet::from([
            "next_catalog_load_generation".to_string(),
            "active_catalog_load".to_string(),
            "next_rename_generation".to_string(),
            "active_rename".to_string(),
            "deferred_catalog_load".to_string(),
        ]),
        "SessionFeatureReducer must remain a cohesive session-only lifecycle slice"
    );
    for forbidden_dependency in [
        "AppState",
        "TurnStreamState",
        "CoreDispatchOutcome",
        "CoreEffect",
        "AppEvent",
    ] {
        assert!(
            !reducer_source.contains(forbidden_dependency),
            "session reducer must return typed reductions instead of mutating another feature: {forbidden_dependency}"
        );
    }
    let production_calls = top_level_impl_method_calls(&controller_source);
    for required_reduction in [
        "reduce_catalog_load",
        "reduce_rename",
        "accept_catalog_completion",
        "accept_rename_completion",
        "active_rename_matches_thread",
        "take_deferred_catalog_load",
        "has_active_rename",
    ] {
        let call_count = production_calls
            .iter()
            .flat_map(|(_, calls, _)| calls)
            .filter(|(called, _)| called == required_reduction)
            .count();
        assert_eq!(
            call_count, 1,
            "CoreController must call session reducer operation {required_reduction} exactly once in production"
        );
    }
    let catalog_admission =
        top_level_impl_method_source(&controller_source, "admit_session_catalog_load");
    let compact_catalog_admission = rust_code_without_comments_and_literals(&catalog_admission)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_catalog_admission.contains(
            "letreduction={letsnapshot=self.shared_snapshot();self.session_feature.reduce_catalog_load(intent,&snapshot.session_catalog)};matchreduction"
        ),
        "catalog admission must drop its read snapshot before AppState copy-on-write mutation"
    );
    let reduce_catalog_load = inherent_impl_methods(
        &reducer_syntax,
        "SessionFeatureReducer",
        "reduce_catalog_load",
    );
    let [reduce_catalog_load] = reduce_catalog_load.as_slice() else {
        panic!("SessionFeatureReducer must define one reduce_catalog_load method");
    };
    let reduction_inputs = reduce_catalog_load
        .sig
        .inputs
        .iter()
        .filter_map(|argument| match argument {
            syn::FnArg::Receiver(_) => None,
            syn::FnArg::Typed(argument) => Some(argument),
        })
        .collect::<Vec<_>>();
    assert!(
        matches!(
            reduction_inputs.as_slice(),
            [intent, current_catalog]
                if is_named_path_type(&intent.ty, "SessionCatalogLoadIntent")
                    && matches!(
                        current_catalog.ty.as_ref(),
                        syn::Type::Reference(reference)
                            if reference.mutability.is_none()
                                && is_named_path_type(
                                    reference.elem.as_ref(),
                                    "SessionCatalogSnapshot",
                                )
                    )
        ),
        "catalog admission policy must receive the typed intent and read-only catalog slice"
    );

    let production_controller = production_lines(&controller_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    let compact_controller = rust_code_without_comments_and_literals(&production_controller)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for forbidden_constructor in [
        "SessionCatalogLoadCorrelation::new(",
        "SessionRenameCorrelation::new(",
    ] {
        assert!(
            !compact_controller.contains(forbidden_constructor),
            "only SessionFeatureReducer may mint session correlations: {forbidden_constructor}"
        );
    }
    let handle_input_inner = top_level_impl_method_source(&controller_source, "handle_input_inner");
    let compact_handle_input = rust_code_without_comments_and_literals(&handle_input_inner)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for (acceptance_gate, state_mutation) in [
        (
            "accept_catalog_completion(&correlation)",
            "state.apply_session_catalog_result(result)",
        ),
        (
            "accept_rename_completion(&correlation)",
            "state.apply_session_rename(&correlation.request)",
        ),
    ] {
        let gate_position = compact_handle_input
            .find(acceptance_gate)
            .unwrap_or_else(|| panic!("missing session completion gate: {acceptance_gate}"));
        let mutation_position = compact_handle_input
            .find(state_mutation)
            .unwrap_or_else(|| panic!("missing gated session state mutation: {state_mutation}"));
        assert!(
            gate_position < mutation_position,
            "session completion gate {acceptance_gate} must run before {state_mutation}"
        );
    }
    let app_module =
        fs::read_to_string("src/core/app/mod.rs").expect("core app module source should load");
    assert!(
        app_module.contains("mod session_reducer;")
            && !app_module.contains("pub mod session_reducer;")
            && !app_module.contains("pub use session_reducer"),
        "the mutable session reducer must remain private to core/app"
    );
}

#[test]
fn core_conversation_turn_feature_reducer_owns_one_correlated_lifecycle_slice() {
    let controller_source = fs::read_to_string("src/core/app/controller.rs")
        .expect("core controller source should load");
    let controller_syntax =
        syn::parse_file(&controller_source).expect("core controller source should parse");
    let controller_fields = named_struct_fields(&controller_syntax, "CoreController");
    let conversation_turn = controller_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "conversation_turn")
        })
        .expect("CoreController must own one ConversationTurnFeatureReducer slice");
    assert!(
        is_named_path_type(&conversation_turn.ty, "ConversationTurnFeatureReducer"),
        "conversation_turn must be the typed conversation/turn reducer"
    );

    let reducer_authority_fields = [
        "turn_stream_state",
        "conversation_runtime",
        "guarded_session_rename_stream",
        "next_conversation_load_generation",
        "in_flight_conversation_load",
        "deferred_conversation_load",
        "next_turn_submission_generation",
        "next_post_turn_evaluation_generation",
        "post_turn_continuation_gate",
        "in_flight_post_turn_evaluation",
        "next_stop_request_generation",
        "active_stop_request",
        "next_turn_steer_generation",
        "active_turn_steer",
        "next_approval_decision_generation",
        "approval_review_persistence",
    ];
    let forbidden_root_fields = [
        "turn_stream_state",
        "conversation_runtime",
        "guarded_session_rename_stream",
        "next_conversation_load_generation",
        "in_flight_conversation_load",
        "deferred_conversation_load",
        "next_turn_submission_generation",
        "active_turn_submission",
        "next_post_turn_evaluation_generation",
        "post_turn_continuation_gate",
        "in_flight_post_turn_evaluation",
        "next_stop_request_generation",
        "active_stop_request",
        "next_turn_steer_generation",
        "active_turn_steer",
        "next_approval_decision_generation",
        "active_approval_decision",
        "approval_review_persistence",
    ];
    for field in &controller_fields {
        let field_name = field
            .ident
            .as_ref()
            .expect("CoreController field should be named")
            .to_string();
        assert!(
            field_name == "conversation_turn"
                || !forbidden_root_fields.contains(&field_name.as_str()),
            "conversation/turn authority must live in conversation_turn; unexpected root field: {field_name}"
        );
        if field_name == "conversation_turn" {
            continue;
        }
        for forbidden_type in [
            "TurnStreamState",
            "ConversationLoadCorrelation",
            "TurnSubmissionCorrelation",
            "StopRequestCorrelation",
            "TurnSteerCorrelation",
            "ApprovalDecisionCorrelation",
            "PostTurnEvaluationCorrelation",
            "ActiveStopRequest",
            "ActiveTurnSteer",
            "ActiveApprovalDecision",
            "ActivePostTurnEvaluation",
        ] {
            assert!(
                !type_mentions_named_path(&field.ty, forbidden_type),
                "CoreController field {field_name} must not hide conversation/turn authority type {forbidden_type}"
            );
        }
    }

    let reducer_source = fs::read_to_string("src/core/app/conversation_turn_reducer.rs")
        .expect("conversation/turn feature reducer source should load");
    let reducer_syntax =
        syn::parse_file(&reducer_source).expect("conversation/turn reducer should parse");
    let reducer_fields = named_struct_fields(&reducer_syntax, "ConversationTurnFeatureReducer")
        .into_iter()
        .map(|field| {
            field
                .ident
                .as_ref()
                .expect("conversation/turn reducer field should be named")
                .to_string()
        })
        .collect::<HashSet<_>>();
    assert_eq!(
        reducer_fields,
        reducer_authority_fields
            .into_iter()
            .map(str::to_string)
            .collect::<HashSet<_>>(),
        "ConversationTurnFeatureReducer must remain one cohesive correlated lifecycle slice"
    );
    let reducer_struct_fields =
        named_struct_fields(&reducer_syntax, "ConversationTurnFeatureReducer");
    let runtime_authority = reducer_struct_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "conversation_runtime")
        })
        .expect("conversation reducer must own one runtime authority");
    assert!(
        is_named_path_type(&runtime_authority.ty, "ConversationRuntimeAuthority"),
        "active-turn, approval, auto-follow, and post-turn authority must share one typed runtime owner"
    );
    let continuation_gate = reducer_struct_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "post_turn_continuation_gate")
        })
        .expect("conversation reducer must own the worker cancellation gate");
    assert!(
        is_named_path_type(&continuation_gate.ty, "PostTurnContinuationGate"),
        "the physical worker permit must remain beside the semantic post-turn authority"
    );
    let runtime_authority_source = fs::read_to_string("src/core/app/conversation_runtime.rs")
        .expect("conversation runtime authority source should load");
    let runtime_authority_syntax = syn::parse_file(&runtime_authority_source)
        .expect("conversation runtime authority source should parse");
    let runtime_fields =
        named_struct_fields(&runtime_authority_syntax, "ConversationRuntimeAuthority");
    assert!(
        matches!(
            runtime_fields.as_slice(),
            [snapshot, pending_route]
                if snapshot.ident.as_ref().is_some_and(|ident| ident == "snapshot")
                    && is_named_path_type(&snapshot.ty, "ConversationRuntimeSnapshot")
                    && pending_route
                        .ident
                        .as_ref()
                        .is_some_and(|ident| ident == "pending_post_turn_route")
                    && is_single_generic_named_type(
                        &pending_route.ty,
                        "Option",
                        "PendingPostTurnRoute",
                    )
        ),
        "ConversationRuntimeAuthority must retain one semantic snapshot plus one private exact route payload"
    );
    let production_reducer = production_source_before_inline_tests(&reducer_source);
    for forbidden_dependency in [
        "AppState",
        "PlanningRuntimeCoordinator",
        "SessionFeatureReducer",
        "CoreController",
        "CoreDispatchOutcome",
        "CoreEffect",
        "AppEvent",
        "NativeTuiApp",
        "crate::adapter",
        "crate::application",
    ] {
        assert!(
            !production_reducer.contains(forbidden_dependency),
            "conversation/turn reducer must return typed reductions instead of mutating another feature: {forbidden_dependency}"
        );
    }
    for item in &reducer_syntax.items {
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        if type_is_simple_path(
            item_impl.self_ty.as_ref(),
            &["ConversationTurnFeatureReducer"],
        ) && item_impl.trait_.as_ref().is_some_and(|(_, path, _)| {
            path.segments
                .last()
                .is_some_and(|segment| segment.ident == "DerefMut")
        }) {
            panic!("ConversationTurnFeatureReducer must not expose state through DerefMut");
        }
        if item_impl.trait_.is_some()
            || !type_is_simple_path(
                item_impl.self_ty.as_ref(),
                &["ConversationTurnFeatureReducer"],
            )
        {
            continue;
        }
        for item in &item_impl.items {
            let syn::ImplItem::Fn(method) = item else {
                continue;
            };
            if matches!(method.vis, syn::Visibility::Inherited) {
                continue;
            }
            let signature_mentions_stream_state = method.sig.inputs.iter().any(|argument| {
                matches!(
                    argument,
                    syn::FnArg::Typed(argument)
                        if type_mentions_named_path(argument.ty.as_ref(), "TurnStreamState")
                )
            }) || matches!(
                &method.sig.output,
                syn::ReturnType::Type(_, ty)
                    if type_mentions_named_path(ty.as_ref(), "TurnStreamState")
            );
            assert!(
                !signature_mentions_stream_state,
                "public reducer method {} must expose typed transitions, not raw TurnStreamState",
                method.sig.ident
            );
        }
    }

    let production_controller = production_source_before_inline_tests(&controller_source);
    let production_controller_syntax = syn::parse_file(&production_controller)
        .expect("production Core controller source should parse");
    for item in &production_controller_syntax.items {
        let syn::Item::Struct(item_struct) = item else {
            continue;
        };
        let syn::Fields::Named(fields) = &item_struct.fields else {
            continue;
        };
        for field in &fields.named {
            if item_struct.ident == "CoreController"
                && field
                    .ident
                    .as_ref()
                    .is_some_and(|ident| ident == "conversation_turn")
            {
                continue;
            }
            for forbidden_type in [
                "TurnStreamState",
                "ConversationLoadCorrelation",
                "TurnSubmissionCorrelation",
                "StopRequestCorrelation",
                "TurnSteerCorrelation",
                "ApprovalDecisionCorrelation",
                "PostTurnEvaluationCorrelation",
                "ActiveStopRequest",
                "ActiveTurnSteer",
                "ActiveApprovalDecision",
                "ActivePostTurnEvaluation",
                "ConversationRuntimeAuthority",
                "PostTurnContinuationGate",
            ] {
                assert!(
                    !type_mentions_named_path(&field.ty, forbidden_type),
                    "production controller struct {} must not hide lifecycle authority type {forbidden_type}",
                    item_struct.ident
                );
            }
        }
    }
    let compact_controller = rust_code_without_comments_and_literals(&production_controller)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for forbidden_constructor in [
        "ConversationLoadCorrelation::new(",
        "TurnSubmissionCorrelation::new(",
        "StopRequestCorrelation::new(",
        "TurnSteerCorrelation::new(",
        "ApprovalDecisionCorrelation::new(",
        "PostTurnEvaluationCorrelation::new(",
    ] {
        assert!(
            !compact_controller.contains(forbidden_constructor),
            "only ConversationTurnFeatureReducer may mint lifecycle correlations: {forbidden_constructor}"
        );
    }
    struct LifecycleStructLiteralVisitor {
        forbidden: HashSet<&'static str>,
        found: Vec<String>,
    }
    impl<'ast> Visit<'ast> for LifecycleStructLiteralVisitor {
        fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
            if expression
                .path
                .segments
                .last()
                .is_some_and(|segment| self.forbidden.contains(segment.ident.to_string().as_str()))
            {
                self.found.push(
                    expression
                        .path
                        .segments
                        .iter()
                        .map(|segment| segment.ident.to_string())
                        .collect::<Vec<_>>()
                        .join("::"),
                );
            }
            visit::visit_expr_struct(self, expression);
        }
    }
    let mut lifecycle_literals = LifecycleStructLiteralVisitor {
        forbidden: HashSet::from([
            "ConversationLoadCorrelation",
            "TurnSubmissionCorrelation",
            "StopRequestCorrelation",
            "TurnSteerCorrelation",
            "ApprovalDecisionCorrelation",
            "PostTurnEvaluationCorrelation",
        ]),
        found: Vec::new(),
    };
    lifecycle_literals.visit_file(&production_controller_syntax);
    assert!(
        lifecycle_literals.found.is_empty(),
        "CoreController must not mint lifecycle correlations through struct literals: {:?}",
        lifecycle_literals.found
    );

    let production_calls = top_level_impl_method_calls(&controller_source);
    for required_reduction in [
        "admit_conversation_load",
        "reduce_conversation_invalidation",
        "complete_conversation_load",
        "admit_turn_submission",
        "admit_stop_request",
        "complete_stop_request",
        "admit_turn_steer",
        "complete_turn_steer",
        "admit_approval_decision",
        "complete_approval_decision",
        "admit_post_turn_evaluation",
        "complete_post_turn_evaluation",
        "apply_correlated_turn_stream_event",
        "reduce_session_rename_projection",
    ] {
        let call_count = production_calls
            .iter()
            .flat_map(|(_, calls, _)| calls)
            .filter(|(called, _)| called == required_reduction)
            .count();
        assert_eq!(
            call_count, 1,
            "CoreController must route major lifecycle operation {required_reduction} through the feature reducer exactly once"
        );
    }
    let handle_input_inner = top_level_impl_method_source(&controller_source, "handle_input_inner");
    let compact_handle_input = rust_code_without_comments_and_literals(&handle_input_inner)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for (acceptance_gate, root_mutation) in [
        (
            "complete_conversation_load(&correlation,loaded_stream_identity)",
            "state.apply_conversation_result(result)",
        ),
        (
            "complete_post_turn_evaluation(&correlation,execution.as_ref())",
            "planning.accept_post_turn_worker_panel(execution.as_ref())",
        ),
    ] {
        let gate_position = compact_handle_input
            .find(acceptance_gate)
            .unwrap_or_else(|| {
                panic!("missing conversation/turn completion gate: {acceptance_gate}")
            });
        let mutation_position = compact_handle_input
            .find(root_mutation)
            .unwrap_or_else(|| panic!("missing gated root mutation: {root_mutation}"));
        assert!(
            gate_position < mutation_position,
            "feature reducer completion gate {acceptance_gate} must run before {root_mutation}"
        );
    }

    let app_module =
        fs::read_to_string("src/core/app/mod.rs").expect("core app module source should load");
    assert!(
        app_module.contains("mod conversation_turn_reducer;")
            && !app_module.contains("pub mod conversation_turn_reducer;")
            && !app_module.contains("pub use conversation_turn_reducer"),
        "the mutable conversation/turn reducer must remain private to core/app"
    );
}

#[test]
fn core_controller_owns_exactly_seven_private_typed_feature_slices() {
    let controller_source = fs::read_to_string("src/core/app/controller.rs")
        .expect("core controller source should load");
    let app_module_source =
        fs::read_to_string("src/core/app/mod.rs").expect("core app module source should load");
    let reducer_sources = core_feature_reducer_sources();

    verify_core_feature_reducer_contract(&controller_source, &app_module_source, &reducer_sources)
        .unwrap_or_else(|error| panic!("Core feature reducer contract violated: {error}"));
}

#[test]
fn core_feature_reducer_analyzer_rejects_raw_writers_without_test_fixture_noise() {
    let controller_source = fs::read_to_string("src/core/app/controller.rs")
        .expect("core controller source should load");
    let app_module_source =
        fs::read_to_string("src/core/app/mod.rs").expect("core app module source should load");
    let reducer_sources = core_feature_reducer_sources();

    let raw_controller = controller_source.replacen(
        "state: AppState,",
        "state: AppState,\n    next_escaped_generation: u64,",
        1,
    );
    let error =
        verify_core_feature_reducer_contract(&raw_controller, &app_module_source, &reducer_sources)
            .expect_err("a raw root generation writer must be rejected");
    assert!(
        error.contains("exactly the seven typed private slices"),
        "unexpected controller analyzer error: {error}"
    );

    let mut public_field_sources = reducer_sources.clone();
    let startup = public_field_sources
        .get_mut("startup_reducer")
        .expect("startup reducer fixture should exist");
    let field_name = first_named_struct_field_name(startup, "StartupFeatureReducer")
        .expect("startup reducer should retain an authority field");
    *startup = startup.replacen(
        &format!("    {field_name}:"),
        &format!("    pub(super) {field_name}:"),
        1,
    );
    let error = verify_core_feature_reducer_contract(
        &controller_source,
        &app_module_source,
        &public_field_sources,
    )
    .expect_err("a visible reducer authority field must be rejected");
    assert!(
        error.contains("authority fields must remain private"),
        "unexpected reducer visibility analyzer error: {error}"
    );

    let mut forbidden_dependency_sources = reducer_sources.clone();
    forbidden_dependency_sources
        .get_mut("read_model_reducer")
        .expect("read-model reducer fixture should exist")
        .push_str(
            "\nfn escaped_dependency(_: crate::application::service::PlanningServices, _: AppEvent) {}\n",
        );
    let error = verify_core_feature_reducer_contract(
        &controller_source,
        &app_module_source,
        &forbidden_dependency_sources,
    )
    .expect_err("a reducer dependency on application or AppEvent must be rejected");
    assert!(
        error.contains("forbidden dependency"),
        "unexpected reducer dependency analyzer error: {error}"
    );

    let mut harmless_fixture_sources = reducer_sources.clone();
    harmless_fixture_sources
        .get_mut("read_model_reducer")
        .expect("read-model reducer fixture should exist")
        .push_str(
            r#"
const ARCHITECTURE_EXAMPLE: &str = "crate::application::service::PlanningServices AppEvent";
#[cfg(test)]
mod architecture_fixture {
    use crate::application::service::PlanningServices;
    use super::super::AppEvent;
    fn fixture(_: PlanningServices, _: AppEvent) {}
}
"#,
        );
    verify_core_feature_reducer_contract(
        &controller_source,
        &app_module_source,
        &harmless_fixture_sources,
    )
    .expect("comments, strings, and test-only fixtures must not create reducer false positives");
}

#[test]
fn conversation_turn_runtime_authority_match_is_ast_exhaustive() {
    let update_source =
        fs::read_to_string("src/core/app/turn_stream.rs").expect("turn stream source should load");
    let reducer_source = fs::read_to_string("src/core/app/conversation_turn_reducer.rs")
        .expect("conversation/turn reducer source should load");

    verify_turn_stream_authority_match_contract(&update_source, &reducer_source)
        .unwrap_or_else(|error| panic!("TurnStreamUpdate authority match violated: {error}"));
}

#[test]
fn conversation_turn_runtime_authority_match_analyzer_rejects_silent_variants() {
    let update_source =
        fs::read_to_string("src/core/app/turn_stream.rs").expect("turn stream source should load");
    let reducer_source = fs::read_to_string("src/core/app/conversation_turn_reducer.rs")
        .expect("conversation/turn reducer source should load");

    let wildcard_reducer =
        reducer_source.replacen("TurnStreamUpdate::AttachmentObserved { .. }", "_", 1);
    let error = verify_turn_stream_authority_match_contract(&update_source, &wildcard_reducer)
        .expect_err("a wildcard authority arm must be rejected");
    assert!(
        error.contains("wildcard patterns are forbidden"),
        "unexpected TurnStreamUpdate wildcard analyzer error: {error}"
    );

    let guarded_reducer = reducer_source.replacen(
        "TurnStreamUpdate::TurnStarted { turn_id, .. } =>",
        "TurnStreamUpdate::TurnStarted { turn_id, .. } if true =>",
        1,
    );
    let error = verify_turn_stream_authority_match_contract(&update_source, &guarded_reducer)
        .expect_err("a guarded authority arm must be rejected");
    assert!(
        error.contains("must not use a match guard"),
        "unexpected TurnStreamUpdate guard analyzer error: {error}"
    );

    let expanded_updates = update_source.replacen(
        "pub enum TurnStreamUpdate {",
        "pub enum TurnStreamUpdate {\n    UnsettledAuthority,",
        1,
    );
    let error = verify_turn_stream_authority_match_contract(&expanded_updates, &reducer_source)
        .expect_err("a newly silent TurnStreamUpdate variant must be rejected");
    assert!(
        error.contains("must exactly cover the enum"),
        "unexpected TurnStreamUpdate coverage analyzer error: {error}"
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

    let shell_source = fs::read_to_string("src/adapter/inbound/tui/shell_chrome.rs")
        .expect("shell source should load");
    let compact_shell_production = production_lines(&shell_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("")
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for forbidden_local_admission in [
        ".session_state=SessionState::Loading",
        "session_state:SessionState::Loading",
        "queue_session_load_if_allowed",
        "queue_session_reload_if_allowed",
        "matches!(state.session_state,SessionState::",
    ] {
        assert!(
            !compact_shell_production.contains(forbidden_local_admission),
            "shell chrome must not regain projection-based catalog admission: {forbidden_local_admission}"
        );
    }

    let catalog_intent_helper = top_level_function_source(
        &shell_source,
        "queue_session_catalog_intent_if_startup_ready",
    );
    let compact_catalog_intent_helper =
        rust_code_without_comments_and_literals(&catalog_intent_helper)
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
    assert!(
        compact_catalog_intent_helper.contains("state.can_open_session_list()")
            && compact_catalog_intent_helper.contains("ShellChromeEffect::LoadSessionCatalog")
            && !compact_catalog_intent_helper.contains("session_state")
            && !compact_catalog_intent_helper.contains("SessionState::"),
        "shell catalog trigger may check startup readiness but must not suppress intent from catalog projection"
    );

    let app_runtime_source = fs::read_to_string("src/adapter/inbound/tui/app/app_runtime.rs")
        .expect("TUI app runtime source should load");
    let execute_effect =
        top_level_impl_method_source(&app_runtime_source, "execute_shell_chrome_effect");
    let compact_execute_effect = rust_code_without_comments_and_literals(&execute_effect)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_execute_effect.contains(
            "SessionCatalogLoadMode::EnsureLoaded=>{SessionCatalogLoadIntent::ensure_loaded(limit,workspace_directory)}",
        ) && compact_execute_effect.contains(
            "SessionCatalogLoadMode::Refresh=>{SessionCatalogLoadIntent::refresh(limit,workspace_directory)}",
        ),
        "TUI runtime must preserve ensure/refresh policy when mapping shell effects to Core intents"
    );
}

#[test]
fn shell_overlay_cleanup_is_owned_by_the_typed_reducer_transition() {
    let shell_source = fs::read_to_string("src/adapter/inbound/tui/shell_chrome.rs")
        .expect("shell chrome source should load");
    let compact_shell = rust_code_without_comments_and_literals(
        &production_source_before_inline_tests(&shell_source),
    )
    .chars()
    .filter(|character| !character.is_whitespace())
    .collect::<String>();
    assert!(
        compact_shell.contains("puboverlay_transition:Option<ShellOverlayTransition>"),
        "ShellChromeReduction must carry the reducer-owned typed overlay transition"
    );

    let shell_controller = fs::read_to_string("src/adapter/inbound/tui/app/shell_controller.rs")
        .expect("shell controller source should load");
    let close_overlay = top_level_impl_method_source(&shell_controller, "close_shell_overlay");
    let compact_close = rust_code_without_comments_and_literals(&close_overlay)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_close.contains("self.dispatch_shell_chrome(ShellChromeEvent::OverlayClosed);"),
        "close_shell_overlay must express only the typed close intent"
    );
    for forbidden in ["match", ".reset(", "::default(", "clear_loading("] {
        assert!(
            !compact_close.contains(forbidden),
            "close_shell_overlay must not regain direct overlay cleanup: {forbidden}"
        );
    }

    let app_runtime = fs::read_to_string("src/adapter/inbound/tui/app/app_runtime.rs")
        .expect("TUI app runtime source should load");
    let dispatch = top_level_impl_method_source(&app_runtime, "dispatch_shell_chrome");
    let compact_dispatch = rust_code_without_comments_and_literals(&dispatch)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_dispatch.contains(
            "ifletSome(transition)=overlay_transition{self.apply_shell_overlay_transition(transition);}",
        ),
        "dispatch_shell_chrome must pass the reducer transition to the single cleanup owner"
    );
    for forbidden_cleanup in [
        "reviews_overlay_ui_state",
        "queue_overlay_ui_state",
        "directions_maintenance_overlay_ui_state",
        "planning_draft_editor_ui_state",
        "parallel_peek_overlay_ui_state",
        "planning_init_overlay_ui_state",
    ] {
        assert!(
            !compact_dispatch.contains(forbidden_cleanup),
            "dispatch_shell_chrome must not restore its old cleanup if-chain: {forbidden_cleanup}"
        );
    }

    let cleanup = top_level_impl_method_source(&app_runtime, "apply_shell_overlay_transition");
    let compact_cleanup = rust_code_without_comments_and_literals(&cleanup)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_cleanup.contains("matchtransition.exit_mode{")
            && compact_cleanup.contains("ShellOverlayExitMode::Suspend=>")
            && compact_cleanup.contains("ShellOverlayExitMode::Exit=>matchtransition.from{"),
        "overlay cleanup owner must exhaustively distinguish suspend from exit"
    );
    for overlay in [
        "Hidden",
        "Startup",
        "Sessions",
        "ModelSelection",
        "ViewSelection",
        "LanguageSelection",
        "Supersession",
        "ParallelPeek",
        "Activity",
        "Help",
        "Reviews",
        "Queue",
        "DirectionsMaintenance",
        "PlanningInit",
        "Approval",
    ] {
        assert!(
            compact_cleanup.contains(&format!("ShellOverlay::{overlay}")),
            "overlay cleanup owner must explicitly classify ShellOverlay::{overlay}"
        );
    }
    assert!(
        !compact_cleanup.contains("_=>"),
        "overlay cleanup owner must not hide new overlay or exit-mode variants behind a wildcard"
    );
}

#[test]
fn shell_chrome_state_has_one_typed_reducer_writer() {
    let root = repo_root();
    let shell_source = fs::read_to_string(root.join("src/adapter/inbound/tui/shell_chrome.rs"))
        .expect("shell chrome source should load");
    verify_shell_chrome_event_reducer_contract(&shell_source)
        .unwrap_or_else(|error| panic!("ShellChromeEvent reducer contract violated: {error}"));
    let shell_syntax = syn::parse_file(&shell_source).expect("shell chrome source should parse");
    let actual_state_fields = named_struct_fields(&shell_syntax, "ShellChromeState")
        .into_iter()
        .map(|field| {
            field
                .ident
                .as_ref()
                .expect("ShellChromeState fields must be named")
                .to_string()
        })
        .collect::<HashSet<_>>();
    assert_eq!(
        actual_state_fields,
        SHELL_CHROME_REDUCER_FIELDS
            .iter()
            .map(|field| (*field).to_string())
            .collect(),
        "ShellChromeState field additions must extend the audited reducer-writer ledger"
    );

    let tui_module = fs::read_to_string(root.join("src/adapter/inbound/tui/mod.rs"))
        .expect("TUI module source should load");
    assert!(
        tui_module.contains("pub(crate) mod shell_chrome;")
            && !tui_module.contains("pub mod shell_chrome;"),
        "shell reducer internals must not be a public adapter API"
    );

    let mut violations = Vec::new();
    let mut reducer_fields = HashSet::new();
    let mut whole_state_writers = Vec::new();
    let declared_module_paths =
        rust_module_paths_from_declarations(&root.join("src/lib.rs"), &["crate".to_string()])
            .unwrap_or_else(|error| panic!("failed to resolve crate module declarations: {error}"));
    assert_eq!(
        declared_module_paths
            .get(&root.join("src/adapter/inbound/tui/app/parallel_mode/panel_controller.rs")),
        Some(&vec![
            "crate".to_string(),
            "adapter".to_string(),
            "inbound".to_string(),
            "tui".to_string(),
            "app".to_string(),
            "parallel_panel_controller".to_string(),
        ]),
        "#[path] modules must use their declared Rust identifier, not their disk directory"
    );
    for (relative, expected) in [
        ("src/lib.rs", vec!["crate"]),
        ("src/adapter/mod.rs", vec!["crate", "adapter"]),
        (
            "src/adapter/inbound/mod.rs",
            vec!["crate", "adapter", "inbound"],
        ),
        (
            "src/adapter/inbound/tui/mod.rs",
            vec!["crate", "adapter", "inbound", "tui"],
        ),
    ] {
        let expected = expected.into_iter().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(
            declared_module_paths.get(&root.join(relative)),
            Some(&expected),
            "shell authority registry must include declared crate-to-TUI ancestor `{relative}`"
        );
    }
    let tui_sources = rust_files_under(&root.join("src/adapter/inbound/tui"))
        .into_iter()
        .filter(|path| !is_test_only_path(path))
        .map(|path| {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            let module_path = declared_module_paths
                .get(&path)
                .cloned()
                .unwrap_or_else(|| {
                    panic!(
                        "production TUI source is not reachable from declared module graph: {}",
                        path.display()
                    )
                });
            (path, source, module_path)
        })
        .collect::<Vec<_>>();
    let mut known_struct_fields = ShellStructFields::new();
    let mut known_type_aliases = ShellQualifiedTypeAliases::new();
    let tui_root_module_path = [
        "crate".to_string(),
        "adapter".to_string(),
        "inbound".to_string(),
        "tui".to_string(),
    ];
    let mut registry_sources = declared_module_paths.iter().collect::<Vec<_>>();
    registry_sources.sort_by_key(|(path, _)| path.as_os_str().to_owned());
    for (path, module_path) in registry_sources {
        let source = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let syntax = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()));
        if tui_root_module_path.starts_with(module_path.as_slice())
            || module_path.starts_with(tui_root_module_path.as_slice())
        {
            known_struct_fields.extend(qualified_struct_fields_declared_in_file(
                &syntax,
                module_path,
            ));
        }
        known_type_aliases.extend(qualified_type_aliases_declared_in_file(
            &syntax,
            module_path,
        ));
    }
    for (path, source, module_path) in &tui_sources {
        let syntax = syn::parse_file(source)
            .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()));
        known_struct_fields.extend(qualified_struct_fields_declared_in_file(
            &syntax,
            module_path,
        ));
        known_type_aliases.extend(qualified_type_aliases_declared_in_file(
            &syntax,
            module_path,
        ));
    }

    for (path, source, module_path) in &tui_sources {
        let relative = relative_path(&root, path);
        let audit = shell_chrome_writer_audit_with_type_registry(
            source,
            relative == "src/adapter/inbound/tui/app/app_runtime.rs",
            &known_struct_fields,
            &known_type_aliases,
            module_path,
        )
        .unwrap_or_else(|error| panic!("failed to audit {}: {error}", path.display()));

        for write in audit.field_writes {
            if relative == "src/adapter/inbound/tui/shell_chrome.rs"
                && write.owner == "reduce_shell_chrome"
            {
                for &field in SHELL_CHROME_REDUCER_FIELDS {
                    if write.detail.contains(&format!("`{field}`")) {
                        reducer_fields.insert(field);
                    }
                }
            } else {
                violations.push(format!("{relative}:{}:{write}", write.line));
            }
        }
        violations.extend(
            audit
                .macro_escapes
                .into_iter()
                .map(|write| format!("{relative}:{}:{write}", write.line)),
        );
        whole_state_writers.extend(
            audit
                .whole_state_writes
                .into_iter()
                .map(|write| (relative.clone(), write.owner, write.line)),
        );
    }

    assert!(
        violations.is_empty(),
        "production shell chrome fields must be written only by reduce_shell_chrome:\n{}",
        violations.join("\n")
    );
    assert_eq!(
        reducer_fields,
        SHELL_CHROME_REDUCER_FIELDS.iter().copied().collect(),
        "the reducer must remain the explicit writer for every ShellChromeState field"
    );
    whole_state_writers.sort();
    assert_eq!(
        whole_state_writers
            .iter()
            .map(|(path, owner, _)| (path.as_str(), owner.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                "src/adapter/inbound/tui/app/app_runtime.rs",
                "apply_shell_chrome_state",
            ),
            (
                "src/adapter/inbound/tui/app/app_runtime.rs",
                "take_shell_chrome_state",
            ),
        ],
        "only the reducer dispatch seam may take and reinstall the whole shell chrome state"
    );
}

#[test]
fn shell_chrome_writer_and_router_analyzers_reject_escape_fixtures() {
    let harmless = r#"
fn render(app: &App) {
    let _ = (&app.shell.chrome.startup_state, app.shell.chrome.selected_session_index);
}
#[cfg(test)]
fn fixture(app: &mut App) {
    app.shell.chrome.startup_state = StartupState::Loading;
}
"#;
    let audit = shell_chrome_writer_audit(harmless).expect("harmless fixture should parse");
    assert!(
        audit.field_writes.is_empty() && audit.whole_state_writes.is_empty(),
        "reads and cfg(test) fixtures must not create shell writer false positives"
    );

    let unrelated_fields = r#"
struct OtherState {
    startup_state: usize,
}
struct Projection {
    session_state: usize,
}
struct Process {
    shell: Vec<u8>,
}
fn update_unrelated(
    other: &mut OtherState,
    projection: &mut Projection,
    process: &mut Process,
) {
    other.startup_state = 1;
    projection.session_state = 2;
    process.shell.clear();
    let _ = &mut process.shell;
}
"#;
    let audit =
        shell_chrome_writer_audit(unrelated_fields).expect("unrelated-field fixture should parse");
    assert!(
        audit.field_writes.is_empty() && audit.whole_state_writes.is_empty(),
        "matching field names outside shell.chrome must not create writer false positives"
    );

    let direct = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("direct shell writer fixture should parse");
    assert_eq!(direct.field_writes.len(), 1);

    let canonical_qualified_app = shell_chrome_writer_audit(
        "fn escape(\n\
             app: &mut crate::adapter::inbound::tui::app::NativeTuiApp,\n\
         ) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("canonical qualified NativeTuiApp fixture should parse");
    assert_eq!(
        canonical_qualified_app.field_writes.len(),
        1,
        "the canonical TUI module path must retain NativeTuiApp authority"
    );

    let relative_qualified_app = shell_chrome_writer_audit_with_struct_registry(
        "fn escape(app: &mut super::NativeTuiApp) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &ShellStructFields::new(),
        &[
            "crate".to_string(),
            "adapter".to_string(),
            "inbound".to_string(),
            "tui".to_string(),
            "app".to_string(),
            "child".to_string(),
        ],
    )
    .expect("relative qualified NativeTuiApp fixture should parse");
    assert_eq!(
        relative_qualified_app.field_writes.len(),
        1,
        "relative paths must resolve against their declared Rust module"
    );

    let module_alias_qualified_app = shell_chrome_writer_audit_with_struct_registry(
        "use super as app_alias;\n\
         fn escape(app: &mut app_alias::NativeTuiApp) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &ShellStructFields::new(),
        &[
            "crate".to_string(),
            "adapter".to_string(),
            "inbound".to_string(),
            "tui".to_string(),
            "app".to_string(),
            "child".to_string(),
        ],
    )
    .expect("module alias qualified NativeTuiApp fixture should parse");
    assert_eq!(
        module_alias_qualified_app.field_writes.len(),
        1,
        "module aliases must resolve against their declared Rust module"
    );

    let child_module_qualified_app = shell_chrome_writer_audit_with_struct_registry(
        "fn escape(app: &mut app::NativeTuiApp) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &ShellStructFields::new(),
        &[
            "crate".to_string(),
            "adapter".to_string(),
            "inbound".to_string(),
            "tui".to_string(),
        ],
    )
    .expect("child module qualified NativeTuiApp fixture should parse");
    assert_eq!(
        child_module_qualified_app.field_writes.len(),
        1,
        "child module paths must resolve against their current Rust module"
    );

    let unrelated_module_alias = shell_chrome_writer_audit_with_struct_registry(
        "use crate::unrelated as app_alias;\n\
         fn update(app: &mut app_alias::NativeTuiApp) {\n\
             app.shell.chrome.session_state = 1;\n\
         }",
        false,
        &ShellStructFields::new(),
        &[
            "crate".to_string(),
            "adapter".to_string(),
            "inbound".to_string(),
            "tui".to_string(),
            "app".to_string(),
            "child".to_string(),
        ],
    )
    .expect("unrelated module alias fixture should parse");
    assert!(
        unrelated_module_alias.field_writes.is_empty()
            && unrelated_module_alias.whole_state_writes.is_empty(),
        "module aliases to an unrelated same-named type must remain harmless"
    );

    let unrelated_qualified_app = shell_chrome_writer_audit(
        "fn update(app: &mut crate::unrelated::NativeTuiApp) {\n\
             app.shell.chrome.session_state = 1;\n\
         }",
    )
    .expect("unrelated qualified NativeTuiApp fixture should parse");
    assert!(
        unrelated_qualified_app.field_writes.is_empty()
            && unrelated_qualified_app.whole_state_writes.is_empty(),
        "a same-named type on an unrelated qualified path must not gain TUI authority"
    );

    let unrelated_qualified_import_alias = shell_chrome_writer_audit(
        "use crate::unrelated::NativeTuiApp as App;\n\
         fn update(app: &mut App) {\n\
             app.shell.chrome.session_state = 1;\n\
         }",
    )
    .expect("unrelated qualified NativeTuiApp import alias fixture should parse");
    assert!(
        unrelated_qualified_import_alias.field_writes.is_empty()
            && unrelated_qualified_import_alias
                .whole_state_writes
                .is_empty(),
        "an import alias must preserve its qualified path before authority classification"
    );

    let qualified_reexport = shell_chrome_writer_audit(
        "mod facade {\n\
             pub use crate::adapter::inbound::tui::app::NativeTuiApp;\n\
         }\n\
         fn escape(app: &mut crate::facade::NativeTuiApp) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("qualified NativeTuiApp re-export fixture should parse");
    assert_eq!(
        qualified_reexport.field_writes.len(),
        1,
        "qualified re-exports must resolve to the original TUI authority path"
    );

    let qualified_module_reexport = shell_chrome_writer_audit(
        "mod facade {\n\
             pub use crate::adapter::inbound::tui::app as app_alias;\n\
         }\n\
         fn escape(app: &mut crate::facade::app_alias::NativeTuiApp) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("qualified NativeTuiApp module re-export fixture should parse");
    assert_eq!(
        qualified_module_reexport.field_writes.len(),
        1,
        "qualified module re-exports must resolve path suffixes to TUI authority"
    );

    let qualified_glob_reexport = shell_chrome_writer_audit(
        "mod facade {\n\
             pub use crate::adapter::inbound::tui::shell_chrome::*;\n\
         }\n\
         fn escape(chrome: &mut crate::facade::ShellChromeState) {\n\
             chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("qualified ShellChromeState glob re-export fixture should parse");
    assert_eq!(
        qualified_glob_reexport.field_writes.len(),
        1,
        "qualified glob re-exports must resolve names from the original authority module"
    );
    let explicit_type_over_glob = shell_chrome_writer_audit(
        "mod facade {\n\
             pub use crate::adapter::inbound::tui::shell_chrome::*;\n\
             pub struct ShellChromeState {\n\
                 pub session_state: usize,\n\
             }\n\
         }\n\
         fn update(chrome: &mut crate::facade::ShellChromeState) {\n\
             chrome.session_state = 1;\n\
         }",
    )
    .expect("explicit type over glob re-export fixture should parse");
    assert!(
        explicit_type_over_glob.field_writes.is_empty()
            && explicit_type_over_glob.whole_state_writes.is_empty(),
        "an explicit local type must shadow a same-named glob re-export"
    );

    let chained_qualified_glob_reexport = shell_chrome_writer_audit(
        "mod first {\n\
             pub use crate::adapter::inbound::tui::shell_chrome::*;\n\
         }\n\
         mod facade {\n\
             pub use crate::first::*;\n\
         }\n\
         fn escape(chrome: &mut crate::facade::ShellChromeState) {\n\
             chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("chained ShellChromeState glob re-export fixture should parse");
    assert_eq!(
        chained_qualified_glob_reexport.field_writes.len(),
        1,
        "glob re-export chains must retain the original authority module"
    );

    let unrelated_qualified_reexport = shell_chrome_writer_audit(
        "mod facade {\n\
             pub use crate::unrelated::NativeTuiApp;\n\
         }\n\
         fn update(app: &mut crate::facade::NativeTuiApp) {\n\
             app.shell.chrome.session_state = 1;\n\
         }",
    )
    .expect("unrelated qualified re-export fixture should parse");
    assert!(
        unrelated_qualified_reexport.field_writes.is_empty()
            && unrelated_qualified_reexport.whole_state_writes.is_empty(),
        "same-named re-exports outside the TUI authority path must remain harmless"
    );

    let unrelated_qualified_module_reexport = shell_chrome_writer_audit(
        "mod facade {\n\
             pub use crate::unrelated as app_alias;\n\
         }\n\
         fn update(app: &mut crate::facade::app_alias::NativeTuiApp) {\n\
             app.shell.chrome.session_state = 1;\n\
         }",
    )
    .expect("unrelated qualified module re-export fixture should parse");
    assert!(
        unrelated_qualified_module_reexport.field_writes.is_empty()
            && unrelated_qualified_module_reexport
                .whole_state_writes
                .is_empty(),
        "module re-exports to unrelated same-named types must remain harmless"
    );

    let unrelated_qualified_glob_reexport = shell_chrome_writer_audit(
        "mod facade {\n\
             pub use crate::unrelated::*;\n\
         }\n\
         fn update(chrome: &mut crate::facade::ShellChromeState) {\n\
             chrome.session_state = 1;\n\
         }",
    )
    .expect("unrelated qualified glob re-export fixture should parse");
    assert!(
        unrelated_qualified_glob_reexport.field_writes.is_empty()
            && unrelated_qualified_glob_reexport
                .whole_state_writes
                .is_empty(),
        "glob re-exports from unrelated modules must remain harmless"
    );

    let wrapped_app = shell_chrome_writer_audit(
        "struct Context<'a> {\n\
             app: &'a mut NativeTuiApp,\n\
         }\n\
         fn escape(context: &mut Context<'_>) {\n\
             context.app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("wrapped native app fixture should parse");
    assert_eq!(
        wrapped_app.field_writes.len(),
        1,
        "user-defined wrapper fields must retain nested NativeTuiApp authority"
    );

    let nested_wrapped_app = shell_chrome_writer_audit(
        "struct Context<'a> {\n\
             app: &'a mut NativeTuiApp,\n\
         }\n\
         struct Outer<'a> {\n\
             context: Context<'a>,\n\
         }\n\
         fn escape(outer: &mut Outer<'_>) {\n\
             let context = &mut outer.context;\n\
             context.app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("nested wrapped native app fixture should parse");
    assert_eq!(
        nested_wrapped_app.field_writes.len(),
        1,
        "nested wrapper fields and local aliases must retain NativeTuiApp authority"
    );

    let authority_wrapper_file = syn::parse_file(
        "pub struct Context<'a> {\n\
             pub app: &'a mut NativeTuiApp,\n\
         }",
    )
    .expect("cross-file wrapper definition should parse");
    let unrelated_wrapper_file = syn::parse_file(
        "pub struct Context<'a> {\n\
             pub app: &'a mut OtherApp,\n\
         }",
    )
    .expect("same-named unrelated wrapper definition should parse");
    let mut known_struct_fields = qualified_struct_fields_declared_in_file(
        &authority_wrapper_file,
        &["crate".to_string(), "authority".to_string()],
    );
    known_struct_fields.extend(qualified_struct_fields_declared_in_file(
        &unrelated_wrapper_file,
        &["crate".to_string(), "unrelated".to_string()],
    ));
    let writer_module = ["crate".to_string(), "writer".to_string()];
    let cross_file_wrapped_app = shell_chrome_writer_audit_with_struct_registry(
        "use crate::authority::Context;\n\
         fn escape(context: &mut Context<'_>) {\n\
             context.app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &writer_module,
    )
    .expect("cross-file wrapped native app fixture should parse");
    assert_eq!(
        cross_file_wrapped_app.field_writes.len(),
        1,
        "the production-wide struct registry must retain authority across Rust files"
    );
    let cross_file_unrelated_app = shell_chrome_writer_audit_with_struct_registry(
        "use crate::unrelated::Context;\n\
         fn update(context: &mut Context<'_>) {\n\
             context.app.shell.chrome.session_state = 1;\n\
         }",
        false,
        &known_struct_fields,
        &writer_module,
    )
    .expect("same-named cross-file wrapper fixture should parse");
    assert!(
        cross_file_unrelated_app.field_writes.is_empty()
            && cross_file_unrelated_app.whole_state_writes.is_empty(),
        "qualified imports must distinguish same-named wrappers across modules"
    );

    let authority_alias_file = syn::parse_file("pub type AppRef<'a> = &'a mut NativeTuiApp;")
        .expect("cross-file authority alias definition should parse");
    let unrelated_alias_file = syn::parse_file("pub type AppRef<'a> = &'a mut OtherApp;")
        .expect("same-named unrelated alias definition should parse");
    let mut known_type_aliases = qualified_type_aliases_declared_in_file(
        &authority_alias_file,
        &["crate".to_string(), "authority_alias".to_string()],
    );
    known_type_aliases.extend(qualified_type_aliases_declared_in_file(
        &unrelated_alias_file,
        &["crate".to_string(), "unrelated_alias".to_string()],
    ));
    let cross_file_aliased_app = shell_chrome_writer_audit_with_type_registry(
        "use crate::authority_alias::AppRef;\n\
         fn escape(app: AppRef<'_>) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &known_type_aliases,
        &writer_module,
    )
    .expect("cross-file aliased native app fixture should parse");
    assert_eq!(
        cross_file_aliased_app.field_writes.len(),
        1,
        "the production-wide alias registry must retain authority across Rust files"
    );
    let qualified_chrome_alias_file = syn::parse_file(
        "pub type ChromeRef<'a> = &'a mut ShellChromeState;\n\
         pub type ChromeView<'a> = &'a ShellChromeState;\n\
         pub type ChromeGuard<'a> = std::sync::MutexGuard<'a, ShellChromeState>;\n\
         pub type ChromeResult<'a> = Result<&'a ShellChromeState, ChromeGuard<'a>>;\n\
         pub type MixedAuthorityResult<'a> = Result<&'a mut NativeTuiApp, ChromeGuard<'a>>;",
    )
    .expect("qualified Chrome authority aliases should parse");
    let qualified_chrome_aliases = qualified_type_aliases_declared_in_file(
        &qualified_chrome_alias_file,
        &["crate".to_string(), "chrome_aliases".to_string()],
    );
    let qualified_mutable_chrome_argument = shell_chrome_writer_audit_with_type_registry(
        "fn external<T>(_chrome: T) {}\n\
         fn escape(chrome: crate::chrome_aliases::ChromeRef<'_>) {\n\
             external(chrome);\n\
         }",
        false,
        &known_struct_fields,
        &qualified_chrome_aliases,
        &writer_module,
    )
    .expect("qualified mutable Chrome argument fixture should parse");
    assert_eq!(
        qualified_mutable_chrome_argument.whole_state_writes.len(),
        1,
        "qualified aliases must retain mutable Chrome authority at external call sites"
    );
    let qualified_read_only_chrome_argument = shell_chrome_writer_audit_with_type_registry(
        "fn external<T>(_chrome: T) {}\n\
         fn inspect(chrome: crate::chrome_aliases::ChromeView<'_>) {\n\
             external(chrome);\n\
         }",
        false,
        &known_struct_fields,
        &qualified_chrome_aliases,
        &writer_module,
    )
    .expect("qualified read-only Chrome argument fixture should parse");
    assert!(
        qualified_read_only_chrome_argument
            .whole_state_writes
            .is_empty(),
        "qualified read-only aliases must not manufacture mutable Chrome authority"
    );
    let qualified_chrome_guard_argument = shell_chrome_writer_audit_with_type_registry(
        "fn external<T>(_chrome: T) {}\n\
         fn escape(chrome: crate::chrome_aliases::ChromeGuard<'_>) {\n\
             external(chrome);\n\
         }",
        false,
        &known_struct_fields,
        &qualified_chrome_aliases,
        &writer_module,
    )
    .expect("qualified Chrome guard argument fixture should parse");
    assert_eq!(
        qualified_chrome_guard_argument.whole_state_writes.len(),
        1,
        "qualified mutable guard aliases must retain Chrome authority at external call sites"
    );
    let multi_argument_chrome_alias = shell_chrome_writer_audit_with_type_registry(
        "fn external<T>(_chrome: T) {}\n\
         fn escape(chrome: crate::chrome_aliases::ChromeResult<'_>) {\n\
             external(chrome);\n\
         }",
        false,
        &known_struct_fields,
        &qualified_chrome_aliases,
        &writer_module,
    )
    .expect("multi-argument Chrome alias fixture should parse");
    assert_eq!(
        multi_argument_chrome_alias.whole_state_writes.len(),
        1,
        "all generic arguments must contribute their mutable Chrome authority"
    );
    let mixed_authority_alias = shell_chrome_writer_audit_with_type_registry(
        "fn external<T>(_value: T) {}\n\
         fn escape(value: crate::chrome_aliases::MixedAuthorityResult<'_>) {\n\
             external(value);\n\
         }",
        false,
        &known_struct_fields,
        &qualified_chrome_aliases,
        &writer_module,
    )
    .expect("mixed authority alias fixture should parse");
    assert_eq!(
        mixed_authority_alias.whole_state_writes.len(),
        1,
        "mutable Shell or Chrome authority must outrank mutable App generic arguments"
    );
    let mixed_authority_variants = shell_chrome_writer_audit_with_type_registry(
        "fn escape(value: crate::chrome_aliases::MixedAuthorityResult<'_>) {\n\
             match value {\n\
                 Ok(app) => {\n\
                     app.shell.chrome.session_state = SessionState::Idle;\n\
                 }\n\
                 Err(mut chrome) => {\n\
                     chrome.session_state = SessionState::Idle;\n\
                 }\n\
             }\n\
         }",
        false,
        &known_struct_fields,
        &qualified_chrome_aliases,
        &writer_module,
    )
    .expect("mixed authority variant fixture should parse");
    assert_eq!(
        mixed_authority_variants.field_writes.len(),
        2,
        "variant patterns must bind each generic payload to its own authority kind"
    );
    let referenced_authority_variants = shell_chrome_writer_audit(
        "fn escape(value: &mut Result<NativeTuiApp, ShellChromeState>) {\n\
             match value {\n\
                 Ok(app) => {\n\
                     app.shell.chrome.session_state = SessionState::Idle;\n\
                 }\n\
                 Err(chrome) => {\n\
                     chrome.session_state = SessionState::Idle;\n\
                 }\n\
             }\n\
         }",
    )
    .expect("referenced authority variant fixture should parse");
    assert_eq!(
        referenced_authority_variants.field_writes.len(),
        2,
        "match ergonomics must preserve outer mutable access for each variant payload"
    );
    let aliased_authority_variants = shell_chrome_writer_audit_with_type_registry(
        "use std::result::Result::{Err as Failure, Ok as Success};\n\
         fn escape(value: crate::chrome_aliases::MixedAuthorityResult<'_>) {\n\
             match value {\n\
                 Success(app) => {\n\
                     app.shell.chrome.session_state = SessionState::Idle;\n\
                 }\n\
                 Failure(mut chrome) => {\n\
                     chrome.session_state = SessionState::Idle;\n\
                 }\n\
             }\n\
         }",
        false,
        &known_struct_fields,
        &qualified_chrome_aliases,
        &writer_module,
    )
    .expect("aliased authority variant fixture should parse");
    assert_eq!(
        aliased_authority_variants.field_writes.len(),
        2,
        "variant constructor aliases must resolve to their original payload positions"
    );
    let generic_authority_alias_file = syn::parse_file(
        "pub type Forward<T> = T;\n\
         pub type AppList<T> = Vec<T>;",
    )
    .expect("generic cross-file authority alias definition should parse");
    let mut generic_authority_aliases = qualified_type_aliases_declared_in_file(
        &generic_authority_alias_file,
        &["crate".to_string(), "generic_alias".to_string()],
    );
    let generic_cross_file_alias = shell_chrome_writer_audit_with_type_registry(
        "use crate::generic_alias::Forward;\n\
         fn escape(app: Forward<&mut NativeTuiApp>) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &generic_authority_aliases,
        &writer_module,
    )
    .expect("generic cross-file authority alias fixture should parse");
    assert_eq!(
        generic_cross_file_alias.field_writes.len(),
        1,
        "generic aliases must substitute actual authority type arguments"
    );
    let generic_reexport_file =
        syn::parse_file("pub use crate::generic_alias::Forward as AppForward;")
            .expect("generic authority re-export should parse");
    generic_authority_aliases.extend(qualified_type_aliases_declared_in_file(
        &generic_reexport_file,
        &["crate".to_string(), "generic_facade".to_string()],
    ));
    let generic_reexported_alias = shell_chrome_writer_audit_with_type_registry(
        "use crate::generic_facade::AppForward;\n\
         fn escape(app: AppForward<&mut NativeTuiApp>) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &generic_authority_aliases,
        &writer_module,
    )
    .expect("generic re-exported authority alias fixture should parse");
    assert_eq!(
        generic_reexported_alias.field_writes.len(),
        1,
        "transparent alias re-exports must forward actual type arguments"
    );
    let generic_collection_alias = shell_chrome_writer_audit(
        "type AppList<T> = Vec<T>;\n\
         fn escape(mut apps: AppList<NativeTuiApp>) {\n\
             apps[0].shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("generic authority collection alias fixture should parse");
    assert_eq!(
        generic_collection_alias.field_writes.len(),
        1,
        "collection inference must retain substituted generic alias elements"
    );
    let qualified_generic_collection_alias = shell_chrome_writer_audit_with_type_registry(
        "fn escape(\n\
             mut apps: crate::generic_alias::AppList<NativeTuiApp>,\n\
         ) {\n\
             apps[0].shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &generic_authority_aliases,
        &writer_module,
    )
    .expect("qualified generic authority collection alias fixture should parse");
    assert_eq!(
        qualified_generic_collection_alias.field_writes.len(),
        1,
        "qualified collection aliases must resolve through the production alias registry"
    );
    let collection_prefix_reexport_file =
        syn::parse_file("pub use crate::generic_alias as collections;")
            .expect("collection prefix re-export should parse");
    generic_authority_aliases.extend(qualified_type_aliases_declared_in_file(
        &collection_prefix_reexport_file,
        &["crate".to_string(), "collection_facade".to_string()],
    ));
    let prefix_reexported_collection_alias = shell_chrome_writer_audit_with_type_registry(
        "fn escape(\n\
             mut apps: crate::collection_facade::collections::AppList<NativeTuiApp>,\n\
         ) {\n\
             apps[0].shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &generic_authority_aliases,
        &writer_module,
    )
    .expect("prefix re-exported authority collection alias fixture should parse");
    assert_eq!(
        prefix_reexported_collection_alias.field_writes.len(),
        1,
        "collection aliases must follow prefix re-exports"
    );
    let collection_glob_reexport_file = syn::parse_file("pub use crate::generic_alias::*;")
        .expect("collection glob re-export should parse");
    generic_authority_aliases.extend(qualified_type_aliases_declared_in_file(
        &collection_glob_reexport_file,
        &["crate".to_string(), "collection_glob".to_string()],
    ));
    let glob_reexported_collection_alias = shell_chrome_writer_audit_with_type_registry(
        "fn escape(\n\
             mut apps: crate::collection_glob::AppList<NativeTuiApp>,\n\
         ) {\n\
             apps[0].shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &generic_authority_aliases,
        &writer_module,
    )
    .expect("glob re-exported authority collection alias fixture should parse");
    assert_eq!(
        glob_reexported_collection_alias.field_writes.len(),
        1,
        "collection aliases must follow glob re-exports"
    );
    let authority_collection_file = syn::parse_file("pub type AppList = Vec<NativeTuiApp>;")
        .expect("authority collection alias should parse");
    let unrelated_collection_file = syn::parse_file("pub type AppList = OtherAppList;")
        .expect("unrelated collection alias should parse");
    let collection_precedence_file = syn::parse_file(
        "pub use crate::authority_collection::*;\n\
         pub use crate::unrelated_collection::AppList;",
    )
    .expect("collection import precedence fixture should parse");
    let mut collection_precedence_aliases = qualified_type_aliases_declared_in_file(
        &authority_collection_file,
        &["crate".to_string(), "authority_collection".to_string()],
    );
    collection_precedence_aliases.extend(qualified_type_aliases_declared_in_file(
        &unrelated_collection_file,
        &["crate".to_string(), "unrelated_collection".to_string()],
    ));
    collection_precedence_aliases.extend(qualified_type_aliases_declared_in_file(
        &collection_precedence_file,
        &["crate".to_string(), "collection_precedence".to_string()],
    ));
    let explicitly_unrelated_collection_alias = shell_chrome_writer_audit_with_type_registry(
        "fn update(mut apps: crate::collection_precedence::AppList) {\n\
             apps[0].shell.chrome.session_state = 1;\n\
         }",
        false,
        &known_struct_fields,
        &collection_precedence_aliases,
        &writer_module,
    )
    .expect("explicitly unrelated collection alias fixture should parse");
    assert!(
        explicitly_unrelated_collection_alias
            .field_writes
            .is_empty()
            && explicitly_unrelated_collection_alias
                .whole_state_writes
                .is_empty(),
        "an explicit unrelated collection import must shadow authority glob re-exports"
    );

    let relative_authority_alias_file = syn::parse_file(
        "pub type BaseApp = super::NativeTuiApp;\n\
         pub type AppRef<'a> = &'a mut BaseApp;",
    )
    .expect("relative cross-file authority alias definition should parse");
    let relative_alias_module = [
        "crate".to_string(),
        "adapter".to_string(),
        "inbound".to_string(),
        "tui".to_string(),
        "app".to_string(),
        "aliases".to_string(),
    ];
    let relative_type_aliases = qualified_type_aliases_declared_in_file(
        &relative_authority_alias_file,
        &relative_alias_module,
    );
    let relative_writer_module = [
        "crate".to_string(),
        "adapter".to_string(),
        "inbound".to_string(),
        "tui".to_string(),
        "app".to_string(),
        "feature".to_string(),
        "writer".to_string(),
    ];
    let relative_cross_file_alias = shell_chrome_writer_audit_with_type_registry(
        "type BaseApp = OtherApp;\n\
         use crate::adapter::inbound::tui::app::aliases::AppRef;\n\
         fn escape(app: AppRef<'_>) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &relative_type_aliases,
        &relative_writer_module,
    )
    .expect("relative cross-file authority alias fixture should parse");
    assert_eq!(
        relative_cross_file_alias.field_writes.len(),
        1,
        "imported aliases must resolve relative RHS paths and nested aliases in their declaration module"
    );

    let module_alias_authority_file = syn::parse_file(
        "use super as app_alias;\n\
         pub type AppRef<'a> = &'a mut app_alias::NativeTuiApp;",
    )
    .expect("module-aliased cross-file authority definition should parse");
    let module_alias_type_aliases = qualified_type_aliases_declared_in_file(
        &module_alias_authority_file,
        &[
            "crate".to_string(),
            "adapter".to_string(),
            "inbound".to_string(),
            "tui".to_string(),
            "app".to_string(),
            "aliases".to_string(),
        ],
    );
    let module_alias_cross_file = shell_chrome_writer_audit_with_type_registry(
        "use crate::adapter::inbound::tui::app::aliases::AppRef;\n\
         fn escape(app: AppRef<'_>) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &module_alias_type_aliases,
        &relative_writer_module,
    )
    .expect("module-aliased cross-file authority fixture should parse");
    assert_eq!(
        module_alias_cross_file.field_writes.len(),
        1,
        "cross-file aliases must resolve module aliases in their declaration scope"
    );

    let cross_file_unrelated_alias = shell_chrome_writer_audit_with_type_registry(
        "use crate::unrelated_alias::AppRef;\n\
         fn update(app: AppRef<'_>) {\n\
             app.shell.chrome.session_state = 1;\n\
         }",
        false,
        &known_struct_fields,
        &known_type_aliases,
        &writer_module,
    )
    .expect("same-named cross-file unrelated alias fixture should parse");
    assert!(
        cross_file_unrelated_alias.field_writes.is_empty()
            && cross_file_unrelated_alias.whole_state_writes.is_empty(),
        "qualified imports must distinguish same-named aliases across modules"
    );

    let ancestor_reexport_file =
        syn::parse_file("pub use crate::adapter::inbound::tui::app::NativeTuiApp as App;")
            .expect("ancestor authority re-export should parse");
    let ancestor_reexports = qualified_type_aliases_declared_in_file(
        &ancestor_reexport_file,
        &[
            "crate".to_string(),
            "adapter".to_string(),
            "inbound".to_string(),
        ],
    );
    let ancestor_reexported_app = shell_chrome_writer_audit_with_type_registry(
        "fn escape(app: &mut crate::adapter::inbound::App) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &ancestor_reexports,
        &relative_writer_module,
    )
    .expect("ancestor re-exported native app fixture should parse");
    assert_eq!(
        ancestor_reexported_app.field_writes.len(),
        1,
        "crate-to-TUI ancestor re-exports must retain NativeTuiApp authority"
    );
    let sibling_reexports = qualified_type_aliases_declared_in_file(
        &ancestor_reexport_file,
        &["crate".to_string(), "facade".to_string()],
    );
    let sibling_reexported_app = shell_chrome_writer_audit_with_type_registry(
        "use crate::facade::App;\n\
         fn escape(app: &mut App) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &sibling_reexports,
        &relative_writer_module,
    )
    .expect("sibling re-exported native app fixture should parse");
    assert_eq!(
        sibling_reexported_app.field_writes.len(),
        1,
        "sibling module re-exports must retain NativeTuiApp authority"
    );

    let crate_alias_file = syn::parse_file("extern crate self as akra;")
        .expect("self-crate alias definition should parse");
    let crate_aliases =
        qualified_type_aliases_declared_in_file(&crate_alias_file, &["crate".to_string()]);
    let self_crate_aliased_app = shell_chrome_writer_audit_with_type_registry(
        "fn escape(\n\
             app: &mut akra::adapter::inbound::tui::app::NativeTuiApp,\n\
         ) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &crate_aliases,
        &relative_writer_module,
    )
    .expect("self-crate aliased native app fixture should parse");
    assert_eq!(
        self_crate_aliased_app.field_writes.len(),
        1,
        "crate-root `extern crate self` aliases must retain NativeTuiApp authority"
    );
    let shadowed_self_crate_alias = shell_chrome_writer_audit_with_type_registry(
        "mod akra {}\n\
         fn update(\n\
             app: &mut akra::adapter::inbound::tui::app::NativeTuiApp,\n\
         ) {\n\
             app.shell.chrome.session_state = 1;\n\
         }",
        false,
        &known_struct_fields,
        &crate_aliases,
        &relative_writer_module,
    )
    .expect("shadowed self-crate alias fixture should parse");
    assert!(
        shadowed_self_crate_alias.field_writes.is_empty()
            && shadowed_self_crate_alias.whole_state_writes.is_empty(),
        "a local type-namespace item must shadow a crate-root self alias"
    );
    let absolute_self_crate_alias = shell_chrome_writer_audit_with_type_registry(
        "mod akra {}\n\
         fn escape(\n\
             app: &mut ::akra::adapter::inbound::tui::app::NativeTuiApp,\n\
         ) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &crate_aliases,
        &relative_writer_module,
    )
    .expect("absolute self-crate alias fixture should parse");
    assert_eq!(
        absolute_self_crate_alias.field_writes.len(),
        1,
        "an absolute self-crate alias must bypass lexical path shadows"
    );
    let self_crate_alias_type_file =
        syn::parse_file("pub type App = akra::adapter::inbound::tui::app::NativeTuiApp;")
            .expect("cross-file self-crate alias type should parse");
    let mut self_crate_alias_types = crate_aliases.clone();
    self_crate_alias_types.extend(qualified_type_aliases_declared_in_file(
        &self_crate_alias_type_file,
        &relative_alias_module,
    ));
    let cross_file_self_crate_alias = shell_chrome_writer_audit_with_type_registry(
        "use crate::adapter::inbound::tui::app::aliases::App;\n\
         fn escape(app: &mut App) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &self_crate_alias_types,
        &relative_writer_module,
    )
    .expect("cross-file self-crate alias fixture should parse");
    assert_eq!(
        cross_file_self_crate_alias.field_writes.len(),
        1,
        "cross-file aliases must resolve self-crate paths in their declaration scope"
    );
    let shadowed_self_crate_alias_type_file = syn::parse_file(
        "mod akra {}\n\
         pub type App = akra::adapter::inbound::tui::app::NativeTuiApp;",
    )
    .expect("shadowed cross-file self-crate alias type should parse");
    let mut shadowed_self_crate_alias_types = crate_aliases.clone();
    shadowed_self_crate_alias_types.extend(qualified_type_aliases_declared_in_file(
        &shadowed_self_crate_alias_type_file,
        &relative_alias_module,
    ));
    let shadowed_cross_file_alias = shell_chrome_writer_audit_with_type_registry(
        "use crate::adapter::inbound::tui::app::aliases::App;\n\
         fn update(app: &mut App) {\n\
             app.shell.chrome.session_state = 1;\n\
         }",
        false,
        &known_struct_fields,
        &shadowed_self_crate_alias_types,
        &relative_writer_module,
    )
    .expect("shadowed cross-file self-crate alias fixture should parse");
    assert!(
        shadowed_cross_file_alias.field_writes.is_empty()
            && shadowed_cross_file_alias.whole_state_writes.is_empty(),
        "cross-file alias RHS paths must preserve declaration-module shadows"
    );
    let import_shadowed_self_crate_alias_file = syn::parse_file(
        "use crate::unrelated as akra;\n\
         pub type App = akra::adapter::inbound::tui::app::NativeTuiApp;",
    )
    .expect("import-shadowed cross-file self-crate alias type should parse");
    let mut import_shadowed_self_crate_aliases = crate_aliases.clone();
    import_shadowed_self_crate_aliases.extend(qualified_type_aliases_declared_in_file(
        &import_shadowed_self_crate_alias_file,
        &relative_alias_module,
    ));
    let import_shadowed_cross_file_alias = shell_chrome_writer_audit_with_type_registry(
        "use crate::adapter::inbound::tui::app::aliases::App;\n\
         fn update(app: &mut App) {\n\
             app.shell.chrome.session_state = 1;\n\
         }",
        false,
        &known_struct_fields,
        &import_shadowed_self_crate_aliases,
        &relative_writer_module,
    )
    .expect("import-shadowed cross-file self-crate alias fixture should parse");
    assert!(
        import_shadowed_cross_file_alias.field_writes.is_empty()
            && import_shadowed_cross_file_alias
                .whole_state_writes
                .is_empty(),
        "cross-file alias RHS paths must preserve declaration-module imports"
    );
    let absolute_imported_self_crate_alias_file = syn::parse_file(
        "mod akra {}\n\
         use ::akra as root_akra;\n\
         pub type App = root_akra::adapter::inbound::tui::app::NativeTuiApp;",
    )
    .expect("absolute-imported cross-file self-crate alias type should parse");
    let mut absolute_imported_self_crate_aliases = crate_aliases.clone();
    absolute_imported_self_crate_aliases.extend(qualified_type_aliases_declared_in_file(
        &absolute_imported_self_crate_alias_file,
        &relative_alias_module,
    ));
    let absolute_imported_cross_file_alias = shell_chrome_writer_audit_with_type_registry(
        "use crate::adapter::inbound::tui::app::aliases::App;\n\
         fn escape(app: &mut App) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &absolute_imported_self_crate_aliases,
        &relative_writer_module,
    )
    .expect("absolute-imported cross-file self-crate alias fixture should parse");
    assert_eq!(
        absolute_imported_cross_file_alias.field_writes.len(),
        1,
        "absolute imports in alias declarations must bypass declaration-module shadows"
    );
    let direct_absolute_reexport_file = syn::parse_file(
        "mod akra {}\n\
         pub use ::akra::adapter::inbound::tui::app::NativeTuiApp as App;",
    )
    .expect("direct absolute self-crate re-export should parse");
    let mut direct_absolute_reexports = crate_aliases.clone();
    direct_absolute_reexports.extend(qualified_type_aliases_declared_in_file(
        &direct_absolute_reexport_file,
        &relative_alias_module,
    ));
    let direct_absolute_reexported_alias = shell_chrome_writer_audit_with_type_registry(
        "use crate::adapter::inbound::tui::app::aliases::App;\n\
         fn escape(app: &mut App) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &direct_absolute_reexports,
        &relative_writer_module,
    )
    .expect("direct absolute self-crate re-export fixture should parse");
    assert_eq!(
        direct_absolute_reexported_alias.field_writes.len(),
        1,
        "direct absolute re-exports must preserve their root path origin"
    );
    let absolute_glob_reexport_file = syn::parse_file(
        "mod akra {}\n\
         pub use ::akra::adapter::inbound::tui::app::*;",
    )
    .expect("absolute self-crate glob re-export should parse");
    let mut absolute_glob_reexports = crate_aliases.clone();
    absolute_glob_reexports.extend(qualified_type_aliases_declared_in_file(
        &absolute_glob_reexport_file,
        &relative_alias_module,
    ));
    let absolute_glob_reexported_alias = shell_chrome_writer_audit_with_type_registry(
        "use crate::adapter::inbound::tui::app::aliases::NativeTuiApp;\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &absolute_glob_reexports,
        &relative_writer_module,
    )
    .expect("absolute self-crate glob re-export fixture should parse");
    assert_eq!(
        absolute_glob_reexported_alias.field_writes.len(),
        1,
        "absolute glob re-exports must preserve their root path origin"
    );
    let block_absolute_alias_import = shell_chrome_writer_audit_with_type_registry(
        "fn escape() {\n\
             use ::akra::adapter::inbound::tui::app::aliases::App;\n\
             let app: &mut App = unsafe { std::mem::zeroed() };\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &direct_absolute_reexports,
        &relative_writer_module,
    )
    .expect("block absolute alias import fixture should parse");
    assert_eq!(
        block_absolute_alias_import.field_writes.len(),
        1,
        "block-scoped absolute imports must normalize before alias registry lookup"
    );
    let absolute_struct_import = shell_chrome_writer_audit_with_type_registry(
        "mod akra {}\n\
         use ::akra::authority::Context;\n\
         fn escape(context: &mut Context<'_>) {\n\
             context.app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &crate_aliases,
        &writer_module,
    )
    .expect("absolute struct import fixture should parse");
    assert_eq!(
        absolute_struct_import.field_writes.len(),
        1,
        "absolute struct imports must normalize before wrapper registry lookup"
    );
    let direct_absolute_struct_path = shell_chrome_writer_audit_with_type_registry(
        "mod akra {}\n\
         fn escape(context: &mut ::akra::authority::Context<'_>) {\n\
             context.app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
        false,
        &known_struct_fields,
        &crate_aliases,
        &writer_module,
    )
    .expect("direct absolute struct path fixture should parse");
    assert_eq!(
        direct_absolute_struct_path.field_writes.len(),
        1,
        "direct absolute struct paths must retain wrapper authority"
    );

    let unrelated_wrapper = shell_chrome_writer_audit(
        "struct OtherChrome { session_state: usize }\n\
         struct OtherShell { chrome: OtherChrome }\n\
         struct OtherApp { shell: OtherShell }\n\
         struct Context<'a> { app: &'a mut OtherApp }\n\
         fn update(context: &mut Context<'_>) {\n\
             context.app.shell.chrome.session_state = 1;\n\
         }",
    )
    .expect("unrelated wrapper fixture should parse");
    assert!(
        unrelated_wrapper.field_writes.is_empty()
            && unrelated_wrapper.whole_state_writes.is_empty(),
        "same-shaped wrapper fields without NativeTuiApp authority must remain harmless"
    );

    let enum_payload = shell_chrome_writer_audit(
        "enum Holder<'a> { App(&'a mut NativeTuiApp), Empty }\n\
         fn escape(holder: Holder<'_>) {\n\
             if let Holder::App(app) = holder {\n\
                 app.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("enum payload authority fixture should parse");
    assert_eq!(
        enum_payload.field_writes.len(),
        1,
        "enum tuple payloads must retain nested NativeTuiApp authority"
    );

    let named_enum_payload = shell_chrome_writer_audit(
        "enum Holder<'a> { App { app: &'a mut NativeTuiApp }, Empty }\n\
         fn escape(holder: Holder<'_>) {\n\
             if let Holder::App { app } = holder {\n\
                 app.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("named enum payload authority fixture should parse");
    assert_eq!(
        named_enum_payload.field_writes.len(),
        1,
        "enum named payloads must retain nested NativeTuiApp authority"
    );
    let aliased_named_enum_payload = shell_chrome_writer_audit(
        "enum Holder<'a> { App { app: &'a mut NativeTuiApp }, Empty }\n\
         use Holder::App as Authority;\n\
         fn escape(holder: Holder<'_>) {\n\
             if let Authority { app } = holder {\n\
                 app.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("aliased named enum payload fixture should parse");
    assert_eq!(
        aliased_named_enum_payload.field_writes.len(),
        1,
        "named variant aliases must resolve to their registered payload authority"
    );
    let generic_enum_payload = shell_chrome_writer_audit(
        "enum Holder<T> { App(T), Empty }\n\
         fn escape(holder: Holder<NativeTuiApp>) {\n\
             if let Holder::App(mut app) = holder {\n\
                 app.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("generic enum payload fixture should parse");
    assert_eq!(
        generic_enum_payload.field_writes.len(),
        1,
        "generic tuple variant payloads must substitute their actual authority type"
    );
    let generic_named_enum_payload = shell_chrome_writer_audit(
        "enum Holder<T> { App { app: T }, Empty }\n\
         fn escape(holder: Holder<NativeTuiApp>) {\n\
             if let Holder::App { mut app } = holder {\n\
                 app.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("generic named enum payload fixture should parse");
    assert_eq!(
        generic_named_enum_payload.field_writes.len(),
        1,
        "generic named variant payloads must substitute their actual authority type"
    );
    let wrapped_generic_struct = shell_chrome_writer_audit(
        "struct Holder<T> { app: T }\n\
         fn escape(mut holder: Box<Holder<NativeTuiApp>>) {\n\
             holder.app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("wrapped generic struct fixture should parse");
    assert_eq!(
        wrapped_generic_struct.field_writes.len(),
        1,
        "wrapper resolution must retain the concrete inner struct arguments"
    );
    let reordered_generic_struct_alias = shell_chrome_writer_audit(
        "struct Holder<T> { app: T }\n\
         type Reordered<A, B> = Holder<B>;\n\
         fn escape(mut holder: Reordered<OtherState, NativeTuiApp>) {\n\
             holder.app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("reordered generic struct alias fixture should parse");
    assert_eq!(
        reordered_generic_struct_alias.field_writes.len(),
        1,
        "alias resolution must retain reordered concrete struct arguments"
    );
    let qualified_reordered_struct_alias = shell_chrome_writer_audit(
        "struct Holder<T> { app: T }\n\
         mod facade {\n\
             pub type Reordered<A, B> = super::Holder<B>;\n\
         }\n\
         fn escape(mut holder: facade::Reordered<OtherState, NativeTuiApp>) {\n\
             holder.app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("qualified reordered generic struct alias fixture should parse");
    assert_eq!(
        qualified_reordered_struct_alias.field_writes.len(),
        1,
        "qualified aliases must retain reordered concrete struct arguments"
    );
    let reexported_reordered_struct_aliases = shell_chrome_writer_audit(
        "struct Holder<T> { app: T }\n\
         mod defs {\n\
             pub type Reordered<A, B> = super::Holder<B>;\n\
         }\n\
         mod facade {\n\
             pub use super::defs as aliases;\n\
             pub use super::defs::*;\n\
         }\n\
         fn escape_prefix(\n\
             mut holder: facade::aliases::Reordered<OtherState, NativeTuiApp>,\n\
         ) {\n\
             holder.app.shell.chrome.session_state = SessionState::Idle;\n\
         }\n\
         fn escape_glob(\n\
             mut holder: facade::Reordered<OtherState, NativeTuiApp>,\n\
         ) {\n\
             holder.app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("re-exported reordered generic struct alias fixtures should parse");
    assert_eq!(
        reexported_reordered_struct_aliases.field_writes.len(),
        2,
        "prefix and glob re-exports must retain reordered concrete struct arguments"
    );
    let split_namespace_reexport = shell_chrome_writer_audit(
        "struct Holder<T> { app: T }\n\
         mod types {\n\
             pub type App = super::Holder<NativeTuiApp>;\n\
         }\n\
         mod values {\n\
             #[allow(non_snake_case)]\n\
             pub fn App() {}\n\
         }\n\
         mod facade {\n\
             pub use super::types::*;\n\
             pub use super::values::App;\n\
         }\n\
         fn escape(mut holder: facade::App) {\n\
             holder.app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("split type/value namespace re-export fixture should parse");
    assert_eq!(
        split_namespace_reexport.field_writes.len(),
        1,
        "a value-only explicit import must not hide a same-named type glob re-export"
    );
    let split_namespace_authority_reexport = shell_chrome_writer_audit(
        "mod types {\n\
             pub type Box = NativeTuiApp;\n\
         }\n\
         mod values {\n\
             #[allow(non_snake_case)]\n\
             pub fn Box() {}\n\
         }\n\
         mod facade {\n\
             pub use super::types::*;\n\
             pub use super::values::Box;\n\
         }\n\
         fn escape(mut app: facade::Box) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("split type/value authority re-export fixture should parse");
    assert_eq!(
        split_namespace_authority_reexport.field_writes.len(),
        1,
        "authority binding must continue to a type glob after a value-only exact import"
    );
    let recursively_reordered_struct = shell_chrome_writer_audit(
        "struct Flip<A, B> {\n\
             child: Box<Flip<B, A>>,\n\
             current: A,\n\
         }\n\
         fn escape<'a>(\n\
             value: Flip<OtherState, &'a mut ShellChromeState>,\n\
         ) -> Flip<OtherState, &'a mut ShellChromeState> {\n\
             value\n\
         }",
    )
    .expect("recursively reordered generic struct fixture should parse");
    assert_eq!(
        recursively_reordered_struct.whole_state_writes.len(),
        1,
        "recursive visits must distinguish concrete generic instantiations"
    );
    let custom_result_payload = shell_chrome_writer_audit(
        "enum Result<T, E> { Ok(E), Err(ShellChromeState, T) }\n\
         fn escape(value: Result<NativeTuiApp, OtherState>) {\n\
             match value {\n\
                 Result::Err(mut chrome, _) => {\n\
                     chrome.session_state = SessionState::Idle;\n\
                 }\n\
                 Result::Ok(_) => {}\n\
             }\n\
         }",
    )
    .expect("custom Result payload fixture should parse");
    assert_eq!(
        custom_result_payload.field_writes.len(),
        1,
        "registered custom Result payloads must outrank standard generic positions"
    );

    let unrelated_enum_payload = shell_chrome_writer_audit(
        "enum Holder<'a> { App(&'a mut OtherApp), Empty }\n\
         fn update(holder: Holder<'_>) {\n\
             if let Holder::App(app) = holder {\n\
                 app.shell.chrome.session_state = 1;\n\
             }\n\
         }",
    )
    .expect("unrelated enum payload fixture should parse");
    assert!(
        unrelated_enum_payload.field_writes.is_empty()
            && unrelated_enum_payload.whole_state_writes.is_empty(),
        "same-named enum variants without NativeTuiApp authority must remain harmless"
    );

    let type_alias = shell_chrome_writer_audit(
        "type App = NativeTuiApp;\n\
         fn escape(app: &mut App) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("type-aliased native app fixture should parse");
    assert_eq!(
        type_alias.field_writes.len(),
        1,
        "type aliases of NativeTuiApp must retain shell chrome authority"
    );

    let self_qualified_type_alias = shell_chrome_writer_audit(
        "type App = NativeTuiApp;\n\
         fn escape(app: &mut self::App) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("self-qualified native app alias fixture should parse");
    assert_eq!(
        self_qualified_type_alias.field_writes.len(),
        1,
        "self-qualified aliases must resolve in their declaration module"
    );

    let cross_module_qualified_type_alias = shell_chrome_writer_audit(
        "mod authority {\n\
             pub type BaseApp = NativeTuiApp;\n\
             pub type App = BaseApp;\n\
         }\n\
         fn escape(app: &mut crate::authority::App) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("cross-module qualified native app alias fixture should parse");
    assert_eq!(
        cross_module_qualified_type_alias.field_writes.len(),
        1,
        "qualified aliases must resolve through the production alias registry"
    );

    let unrelated_qualified_type_alias = shell_chrome_writer_audit(
        "mod unrelated {\n\
             pub type App = OtherApp;\n\
         }\n\
         fn update(app: &mut crate::unrelated::App) {\n\
             app.shell.chrome.session_state = 1;\n\
         }",
    )
    .expect("unrelated qualified alias fixture should parse");
    assert!(
        unrelated_qualified_type_alias.field_writes.is_empty()
            && unrelated_qualified_type_alias.whole_state_writes.is_empty(),
        "qualified aliases without TUI authority must remain harmless"
    );

    let chained_import_alias = shell_chrome_writer_audit(
        "use crate::adapter::inbound::tui::app::NativeTuiApp as BaseApp;\n\
         type App = BaseApp;\n\
         fn escape(app: &mut App) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("chained import and type alias fixture should parse");
    assert_eq!(
        chained_import_alias.field_writes.len(),
        1,
        "import renames and chained type aliases must retain shell chrome authority"
    );

    let scoped_type_alias = shell_chrome_writer_audit(
        "struct Other;\n\
         type App = NativeTuiApp;\n\
         mod unrelated {\n\
             type App = super::Other;\n\
             fn read(_app: &App) {}\n\
         }\n\
         fn escape(app: &mut App) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("lexically scoped type alias fixture should parse");
    assert_eq!(
        scoped_type_alias.field_writes.len(),
        1,
        "a nested module alias must not overwrite the enclosing App authority alias"
    );

    let boxed_type_alias = shell_chrome_writer_audit(
        "type App = NativeTuiApp;\n\
         fn escape(app: &mut Box<App>) {\n\
             (*app).shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("boxed type-aliased native app fixture should parse");
    assert_eq!(
        boxed_type_alias.field_writes.len(),
        1,
        "dereference wrappers around aliased NativeTuiApp must retain shell chrome authority"
    );

    let destructured_alias = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             let NativeTuiShellState { chrome, .. } = &mut app.shell;\n\
             chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("destructured shell chrome alias fixture should parse");
    assert!(
        !destructured_alias.whole_state_writes.is_empty(),
        "a mutable borrow of the shell container must reject destructured chrome aliases"
    );

    let ergonomic_pattern_alias = shell_chrome_writer_audit(
        "fn escape(\n\
             NativeTuiApp {\n\
                 shell: NativeTuiShellState { chrome, .. },\n\
                 ..\n\
             }: &mut NativeTuiApp,\n\
         ) {\n\
             chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("ergonomic shell chrome pattern alias fixture should parse");
    assert_eq!(
        ergonomic_pattern_alias.field_writes.len(),
        1,
        "destructuring shell/chrome authority must retain the aliased field writer even without an explicit mutable reference expression"
    );

    let match_pattern_alias = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             match app {\n\
                 NativeTuiApp {\n\
                     shell: NativeTuiShellState { chrome, .. },\n\
                     ..\n\
                 } => chrome.session_state = SessionState::Idle,\n\
             }\n\
         }",
    )
    .expect("match shell chrome pattern alias fixture should parse");
    assert_eq!(
        match_pattern_alias.field_writes.len(),
        1,
        "match ergonomics must retain mutable shell chrome alias authority"
    );

    let let_chain_alias = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp, enabled: bool) {\n\
             if let NativeTuiApp {\n\
                 shell: NativeTuiShellState { chrome, .. },\n\
                 ..\n\
             } = app && enabled {\n\
                 chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("let-chain shell chrome alias fixture should parse");
    assert_eq!(
        let_chain_alias.field_writes.len(),
        1,
        "Rust 2024 let-chain patterns must retain shell chrome authority"
    );

    let option_pattern_alias = shell_chrome_writer_audit(
        "fn escape(maybe: Option<&mut NativeTuiApp>) {\n\
             if let Some(app) = maybe {\n\
                 app.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("Option shell authority pattern fixture should parse");
    assert_eq!(
        option_pattern_alias.field_writes.len(),
        1,
        "generic Option patterns must retain their inner mutable app authority"
    );

    let read_only_pattern = shell_chrome_writer_audit(
        "fn render(app: &NativeTuiApp) {\n\
             let NativeTuiShellState { chrome, .. } = &app.shell;\n\
             let _ = &chrome.session_state;\n\
         }",
    )
    .expect("read-only shell chrome pattern fixture should parse");
    assert!(
        read_only_pattern.field_writes.is_empty()
            && read_only_pattern.whole_state_writes.is_empty(),
        "read-only shell chrome destructuring must not be classified as a writer"
    );

    let typed_chrome_alias = shell_chrome_writer_audit(
        "fn escape(chrome: &mut ShellChromeState) {\n\
             chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("typed shell chrome alias fixture should parse");
    assert_eq!(
        typed_chrome_alias.field_writes.len(),
        1,
        "a typed mutable ShellChromeState parameter must retain writer authority"
    );

    let shell_impl_alias = shell_chrome_writer_audit(
        "impl NativeTuiShellState {\n\
             fn escape(&mut self) {\n\
                 self.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("NativeTuiShellState impl alias fixture should parse");
    assert_eq!(
        shell_impl_alias.field_writes.len(),
        1,
        "NativeTuiShellState mutable self must resolve through chrome authority"
    );

    let explicit_mutable_self = shell_chrome_writer_audit(
        "impl NativeTuiApp {\n\
             fn escape(self: &mut Self) {\n\
                 self.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("explicit mutable self fixture should parse");
    assert_eq!(
        explicit_mutable_self.field_writes.len(),
        1,
        "explicit receiver types must retain mutable self authority"
    );

    let mutable_impl_self = shell_chrome_writer_audit(
        "trait Mutate {\n\
             fn mutate(self);\n\
         }\n\
         impl Mutate for &mut NativeTuiApp {\n\
             fn mutate(self) {\n\
                 self.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("mutable impl Self authority fixture should parse");
    assert_eq!(
        mutable_impl_self.field_writes.len(),
        1,
        "a by-value Self receiver must inherit mutability from an &mut impl target"
    );

    let shared_mutable_impl_self = shell_chrome_writer_audit(
        "trait Inspect {\n\
             fn inspect(&self);\n\
         }\n\
         impl Inspect for &mut NativeTuiApp {\n\
             fn inspect(&self) {\n\
                 self.shell.chrome.session_state.read_only_probe();\n\
                 external_read!(self.shell.chrome.session_state);\n\
             }\n\
         }",
    )
    .expect("shared mutable impl Self authority fixture should parse");
    assert!(
        shared_mutable_impl_self.field_writes.is_empty()
            && shared_mutable_impl_self.whole_state_writes.is_empty()
            && shared_mutable_impl_self.macro_escapes.is_empty(),
        "&self must remain read-only even when Self is implemented for &mut NativeTuiApp"
    );

    let mutable_wrapper_impl_self = shell_chrome_writer_audit(
        "struct Context<'a> {\n\
             app: &'a mut NativeTuiApp,\n\
         }\n\
         impl Context<'_> {\n\
             fn escape(&mut self) {\n\
                 self.app.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("mutable wrapper impl Self authority fixture should parse");
    assert_eq!(
        mutable_wrapper_impl_self.field_writes.len(),
        1,
        "receiver types using Self must still resolve through their concrete impl target"
    );

    let take_method_result = shell_chrome_writer_audit(
        "impl NativeTuiApp {\n\
             fn take_shell_chrome_state(&mut self) -> ShellChromeState {\n\
                 todo!()\n\
             }\n\
             fn escape(&mut self) {\n\
                 let mut state = self.take_shell_chrome_state();\n\
                 state.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("take shell chrome method result fixture should parse");
    assert_eq!(
        take_method_result.field_writes.len(),
        1,
        "the allowed take seam result must retain Chrome authority in local bindings"
    );

    let tuple_alias = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             let (alias,) = (app,);\n\
             alias.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("tuple shell authority alias fixture should parse");
    assert_eq!(
        tuple_alias.field_writes.len(),
        1,
        "tuple expressions and patterns must recursively retain shell chrome authority"
    );

    let typed_tuple_alias = shell_chrome_writer_audit(
        "fn escape((app,): (&mut NativeTuiApp,)) {\n\
             app.shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("typed tuple shell authority alias fixture should parse");
    assert_eq!(
        typed_tuple_alias.field_writes.len(),
        1,
        "typed tuple parameters must recursively retain shell chrome authority"
    );

    let match_tuple_alias = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             match (app,) {\n\
                 (alias,) => alias.shell.chrome.session_state = SessionState::Idle,\n\
             }\n\
         }",
    )
    .expect("match tuple shell authority alias fixture should parse");
    assert_eq!(
        match_tuple_alias.field_writes.len(),
        1,
        "match tuple patterns must recursively retain shell chrome authority"
    );

    let allowed_dispatch_seam = shell_chrome_writer_audit_with_policy(
        "impl NativeTuiApp {\n\
             fn dispatch_shell_chrome(&mut self) {\n\
                 let state = self.take_shell_chrome_state();\n\
                 self.apply_shell_chrome_state(state);\n\
             }\n\
         }",
        true,
    )
    .expect("exact NativeTuiApp dispatch seam fixture should parse");
    assert!(
        allowed_dispatch_seam.whole_state_writes.is_empty(),
        "only the exact NativeTuiApp dispatch method may call the take/apply seam"
    );

    let same_named_free_function = shell_chrome_writer_audit_with_policy(
        "fn dispatch_shell_chrome(app: &mut NativeTuiApp) {\n\
             let state = app.take_shell_chrome_state();\n\
             app.apply_shell_chrome_state(state);\n\
         }",
        true,
    )
    .expect("same-named free function fixture should parse");
    assert_eq!(
        same_named_free_function.whole_state_writes.len(),
        2,
        "a same-named free function must not inherit the NativeTuiApp dispatch exception"
    );

    let same_named_other_impl = shell_chrome_writer_audit_with_policy(
        "impl Other {\n\
             fn dispatch_shell_chrome(&mut self, app: &mut NativeTuiApp) {\n\
                 let state = app.take_shell_chrome_state();\n\
                 app.apply_shell_chrome_state(state);\n\
             }\n\
         }",
        true,
    )
    .expect("same-named unrelated impl fixture should parse");
    assert_eq!(
        same_named_other_impl.whole_state_writes.len(),
        2,
        "a same-named method on another type must not inherit the dispatch exception"
    );

    let wrong_dispatch_receiver = shell_chrome_writer_audit_with_policy(
        "impl NativeTuiApp {\n\
             fn dispatch_shell_chrome(&mut self, app: &mut NativeTuiApp) {\n\
                 let state = app.take_shell_chrome_state();\n\
                 app.apply_shell_chrome_state(state);\n\
             }\n\
         }",
        true,
    )
    .expect("wrong dispatch receiver fixture should parse");
    assert_eq!(
        wrong_dispatch_receiver.whole_state_writes.len(),
        2,
        "the dispatch exception must apply to the exact mutable self receiver only"
    );

    let unlisted_mutator = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             app.shell.chrome.approval_return_overlay.get_or_insert(Default::default());\n\
         }",
    )
    .expect("unlisted mutable method fixture should parse");
    assert_eq!(
        unlisted_mutator.field_writes.len(),
        1,
        "new methods on mutable shell chrome fields must be rejected unless explicitly read-only"
    );

    let explicit_read = shell_chrome_writer_audit(
        "fn inspect(app: &mut NativeTuiApp) {\n\
             let _ = app.shell.chrome.session_state.clone();\n\
             let _ = app.shell.chrome.shell_overlay.prompt_input_has_focus(false, false);\n\
         }",
    )
    .expect("explicit read-only shell method fixture should parse");
    assert!(
        explicit_read.field_writes.is_empty() && explicit_read.whole_state_writes.is_empty(),
        "audited read-only methods must not create shell writer false positives"
    );

    let macro_writer = shell_chrome_writer_audit(
        "macro_rules! write_shell_chrome {\n\
             ($app:expr) => {\n\
                 $app.shell.chrome.session_state = SessionState::Idle;\n\
             };\n\
         }\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             write_shell_chrome!(app);\n\
         }",
    )
    .expect("shell chrome macro writer fixture should parse");
    assert!(
        !macro_writer.macro_escapes.is_empty(),
        "production macro definitions and authority calls must not hide shell chrome writes"
    );

    let external_macro_writer = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             external_shell_writer!(app);\n\
         }",
    )
    .expect("external shell writer macro fixture should parse");
    assert!(
        !external_macro_writer.macro_escapes.is_empty(),
        "unknown macros receiving mutable shell authority must fail closed"
    );

    let macro_read = shell_chrome_writer_audit(
        "fn inspect(app: &mut NativeTuiApp) {\n\
             let _ = matches!(app.shell.chrome.session_state, SessionState::Idle);\n\
         }",
    )
    .expect("read-only shell macro fixture should parse");
    assert!(
        macro_read.field_writes.is_empty()
            && macro_read.whole_state_writes.is_empty()
            && macro_read.macro_escapes.is_empty(),
        "known read-only macros must not create shell writer false positives"
    );

    let qualified_standard_macro_read = shell_chrome_writer_audit(
        "fn inspect(app: &mut NativeTuiApp) {\n\
             let _ = std::format_args!(\"{}\", app.shell.chrome.session_state);\n\
             let _ = alloc::format!(\"{:?}\", app.shell.chrome.session_state);\n\
             core::assert_eq!(\n\
                 app.shell.chrome.selected_session_index,\n\
                 Some(0),\n\
             );\n\
         }",
    )
    .expect("qualified standard read-only macro fixture should parse");
    assert!(
        qualified_standard_macro_read.field_writes.is_empty()
            && qualified_standard_macro_read.whole_state_writes.is_empty()
            && qualified_standard_macro_read.macro_escapes.is_empty(),
        "std/core-qualified read-only macros must not create writer false positives"
    );

    let shadowed_standard_macro = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             use crate::evil as std;\n\
             let _ = std::format_args!(\"{}\", app.shell.chrome.session_state);\n\
         }",
    )
    .expect("import-shadowed standard macro fixture should parse");
    assert!(
        !shadowed_standard_macro.macro_escapes.is_empty(),
        "a local import named `std` must not make an authority-bearing macro trusted"
    );
    let module_shadowed_standard_macro = shell_chrome_writer_audit(
        "mod alloc {}\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             let _ = alloc::format!(\"{:?}\", app.shell.chrome.session_state);\n\
         }",
    )
    .expect("module-shadowed standard macro fixture should parse");
    assert!(
        !module_shadowed_standard_macro.macro_escapes.is_empty(),
        "a local module named `alloc` must not make an authority-bearing macro trusted"
    );
    let imported_lookalike_read_macro = shell_chrome_writer_audit(
        "use crate::evil::matches;\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             let _ = matches!(app.shell.chrome.session_state, SessionState::Idle);\n\
         }",
    )
    .expect("import-shadowed read macro fixture should parse");
    assert!(
        !imported_lookalike_read_macro.macro_escapes.is_empty(),
        "an imported lookalike must not inherit the read-only macro allowlist"
    );
    let imported_standard_read_macro = shell_chrome_writer_audit(
        "use std::matches;\n\
         fn inspect(app: &mut NativeTuiApp) {\n\
             let _ = matches!(app.shell.chrome.session_state, SessionState::Idle);\n\
         }",
    )
    .expect("imported standard read macro fixture should parse");
    assert!(
        imported_standard_read_macro.field_writes.is_empty()
            && imported_standard_read_macro.whole_state_writes.is_empty()
            && imported_standard_read_macro.macro_escapes.is_empty(),
        "an explicitly imported standard read-only macro must remain harmless"
    );

    let qualified_unknown_macro = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             std::external_shell_writer!(app);\n\
             alloc::external_shell_writer!(app);\n\
         }",
    )
    .expect("qualified unknown macro fixture should parse");
    assert!(
        !qualified_unknown_macro.macro_escapes.is_empty(),
        "std qualification must not whitelist an unknown macro receiving mutable authority"
    );

    let nested_macro_writer = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             assert!(external_shell_writer!(app));\n\
         }",
    )
    .expect("nested shell writer macro fixture should parse");
    assert!(
        !nested_macro_writer.macro_escapes.is_empty(),
        "a known read-only macro must not hide an unknown nested authority macro"
    );

    let nested_read_macros = shell_chrome_writer_audit(
        "fn inspect(app: &mut NativeTuiApp) {\n\
             assert!(matches!(\n\
                 app.shell.chrome.session_state,\n\
                 SessionState::Idle if !(false)\n\
             ));\n\
         }",
    )
    .expect("nested read-only shell macros fixture should parse");
    assert!(
        nested_read_macros.field_writes.is_empty()
            && nested_read_macros.whole_state_writes.is_empty()
            && nested_read_macros.macro_escapes.is_empty(),
        "nested known read-only macros must remain harmless"
    );
    let nested_absolute_read_macro = shell_chrome_writer_audit(
        "fn inspect(app: &mut NativeTuiApp) {\n\
             mod std {}\n\
             assert!(::std::matches!(\n\
                 app.shell.chrome.session_state,\n\
                 SessionState::Idle\n\
             ));\n\
         }",
    )
    .expect("nested absolute standard macro fixture should parse");
    assert!(
        nested_absolute_read_macro.field_writes.is_empty()
            && nested_absolute_read_macro.whole_state_writes.is_empty()
            && nested_absolute_read_macro.macro_escapes.is_empty(),
        "absolute nested standard macros must bypass lexical and crate-root shadows"
    );
    let renamed_nested_read_macro = shell_chrome_writer_audit(
        "use std::matches as is_match;\n\
         fn inspect(app: &mut NativeTuiApp) {\n\
             assert!(is_match!(\n\
                 app.shell.chrome.session_state,\n\
                 SessionState::Idle\n\
             ));\n\
         }",
    )
    .expect("renamed nested standard macro fixture should parse");
    assert!(
        renamed_nested_read_macro.field_writes.is_empty()
            && renamed_nested_read_macro.whole_state_writes.is_empty()
            && renamed_nested_read_macro.macro_escapes.is_empty(),
        "renamed nested standard macros must be classified by their import target"
    );
    let absolute_renamed_nested_read_macro = shell_chrome_writer_audit(
        "mod std {}\n\
         use ::std::matches as is_match;\n\
         fn inspect(app: &mut NativeTuiApp) {\n\
             assert!(is_match!(\n\
                 app.shell.chrome.session_state,\n\
                 SessionState::Idle\n\
             ));\n\
         }",
    )
    .expect("absolute renamed nested standard macro fixture should parse");
    assert!(
        absolute_renamed_nested_read_macro.field_writes.is_empty()
            && absolute_renamed_nested_read_macro
                .whole_state_writes
                .is_empty()
            && absolute_renamed_nested_read_macro.macro_escapes.is_empty(),
        "absolute imported read-only macros must bypass lexical root shadows"
    );
    let renamed_nested_lookalike_macro = shell_chrome_writer_audit(
        "use crate::evil::matches as is_match;\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             assert!(is_match!(app.shell.chrome.session_state));\n\
         }",
    )
    .expect("renamed nested lookalike macro fixture should parse");
    assert!(
        !renamed_nested_lookalike_macro.macro_escapes.is_empty(),
        "renamed lookalikes must not inherit the standard read-only macro allowlist"
    );

    let unrelated_nested_macro = shell_chrome_writer_audit(
        "fn inspect(app: &mut NativeTuiApp) {\n\
             assert!(\n\
                 cfg!(debug_assertions)\n\
                     && matches!(\n\
                         app.shell.chrome.session_state,\n\
                         SessionState::Idle\n\
                     )\n\
             );\n\
         }",
    )
    .expect("unrelated nested macro fixture should parse");
    assert!(
        unrelated_nested_macro.field_writes.is_empty()
            && unrelated_nested_macro.whole_state_writes.is_empty()
            && unrelated_nested_macro.macro_escapes.is_empty(),
        "an unknown nested macro must reference mutable authority itself before failing closed"
    );

    let akra_event_writer = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             crate::akra_event!(\n\
                 tracing::Level::DEBUG,\n\
                 \"writer\",\n\
                 value = {\n\
                     app.shell.chrome.session_state = SessionState::Idle;\n\
                     0\n\
                 },\n\
             );\n\
         }",
    )
    .expect("akra_event writer fixture should parse");
    assert!(
        !akra_event_writer.macro_escapes.is_empty(),
        "akra_event named-value expressions must not hide shell chrome assignments"
    );

    let akra_event_read = shell_chrome_writer_audit(
        "fn inspect(app: &mut NativeTuiApp) {\n\
             crate::akra_event!(\n\
                 tracing::Level::DEBUG,\n\
                 \"read\",\n\
                 value = app.shell.chrome.selected_session_index,\n\
             );\n\
         }",
    )
    .expect("akra_event read fixture should parse");
    assert!(
        akra_event_read.macro_escapes.is_empty(),
        "akra_event top-level key separators must not be mistaken for assignments"
    );

    let returned_chrome_alias = shell_chrome_writer_audit(
        "fn expose(\n\
             NativeTuiApp {\n\
                 shell: NativeTuiShellState { chrome, .. },\n\
                 ..\n\
             }: &mut NativeTuiApp,\n\
         ) -> &mut ShellChromeState {\n\
             chrome\n\
         }",
    )
    .expect("returned mutable chrome alias fixture should parse");
    assert!(
        !returned_chrome_alias.whole_state_writes.is_empty(),
        "mutable shell chrome authority must not escape through a function return"
    );

    let explicit_returned_chrome_alias = shell_chrome_writer_audit(
        "fn expose(chrome: &mut ShellChromeState) -> &mut ShellChromeState {\n\
             return chrome;\n\
         }",
    )
    .expect("explicit returned mutable chrome alias fixture should parse");
    assert!(
        !explicit_returned_chrome_alias.whole_state_writes.is_empty(),
        "explicit returns must not expose mutable shell chrome authority"
    );

    let returned_owned_chrome = shell_chrome_writer_audit(
        "fn expose(app: NativeTuiApp) -> ShellChromeState {\n\
             app.shell.chrome\n\
         }",
    )
    .expect("returned owned chrome fixture should parse");
    assert!(
        !returned_owned_chrome.whole_state_writes.is_empty(),
        "owned ShellChromeState authority must not escape through a function return"
    );

    let inferred_closure_parameter = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             let write = |value| {\n\
                 value.shell.chrome.session_state = SessionState::Idle;\n\
             };\n\
             write(app);\n\
         }",
    )
    .expect("inferred closure parameter fixture should parse");
    assert_eq!(
        inferred_closure_parameter.field_writes.len(),
        1,
        "untyped closure parameters must conservatively retain shell chrome authority paths"
    );

    let boxed_inferred_closure = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             let write: Box<dyn FnOnce(&mut NativeTuiApp)> = Box::new(|value| {\n\
                 value.shell.chrome.session_state = SessionState::Idle;\n\
             });\n\
             write(app);\n\
         }",
    )
    .expect("boxed inferred closure parameter fixture should parse");
    assert_eq!(
        boxed_inferred_closure.field_writes.len(),
        1,
        "callable wrappers must preserve local closure argument inference"
    );
    let imported_arc_inferred_closure = shell_chrome_writer_audit(
        "use std::sync::Arc as Callable;\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             let write = Callable::new(|value| {\n\
                 value.shell.chrome.session_state = SessionState::Idle;\n\
             });\n\
             write(app);\n\
         }",
    )
    .expect("import-aliased Arc closure parameter fixture should parse");
    assert_eq!(
        imported_arc_inferred_closure.field_writes.len(),
        1,
        "canonical imported callable wrappers must preserve closure authority"
    );
    let unrelated_boxed_closure = shell_chrome_writer_audit(
        "fn update(other: &mut OtherApp) {\n\
             let write: Box<dyn FnOnce(&mut OtherApp)> = Box::new(|value| {\n\
                 value.shell.chrome.session_state = 1;\n\
             });\n\
             write(other);\n\
         }",
    )
    .expect("unrelated boxed closure fixture should parse");
    assert!(
        unrelated_boxed_closure.field_writes.is_empty()
            && unrelated_boxed_closure.whole_state_writes.is_empty(),
        "callable wrappers must not infer TUI authority from unrelated arguments"
    );
    let shadowed_standard_wrapper = shell_chrome_writer_audit(
        "mod std {\n\
             pub mod boxed {\n\
                 pub struct Box;\n\
                 impl Box {\n\
                     pub fn new<F>(_closure: F) -> impl FnOnce(&mut NativeTuiApp) {\n\
                         |_| {}\n\
                     }\n\
                 }\n\
             }\n\
         }\n\
         fn update(app: &mut NativeTuiApp) {\n\
             let write = std::boxed::Box::new(|value| {\n\
                 value.shell.chrome.session_state = SessionState::Idle;\n\
             });\n\
             write(app);\n\
         }",
    )
    .expect("shadowed standard callable wrapper fixture should parse");
    assert!(
        shadowed_standard_wrapper.field_writes.is_empty()
            && shadowed_standard_wrapper.whole_state_writes.is_empty(),
        "a relative standard-looking wrapper must respect lexical root shadows"
    );
    let absolute_standard_wrapper = shell_chrome_writer_audit(
        "mod std {}\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             let write = ::std::boxed::Box::new(|value| {\n\
                 value.shell.chrome.session_state = SessionState::Idle;\n\
             });\n\
             write(app);\n\
         }",
    )
    .expect("absolute standard callable wrapper fixture should parse");
    assert_eq!(
        absolute_standard_wrapper.field_writes.len(),
        1,
        "an absolute standard wrapper must bypass lexical root shadows"
    );
    let absolute_imported_standard_wrapper = shell_chrome_writer_audit(
        "mod std {}\n\
         use ::std::boxed::Box as Callable;\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             let write = Callable::new(|value| {\n\
                 value.shell.chrome.session_state = SessionState::Idle;\n\
             });\n\
             write(app);\n\
         }",
    )
    .expect("absolute imported standard callable wrapper fixture should parse");
    assert_eq!(
        absolute_imported_standard_wrapper.field_writes.len(),
        1,
        "absolute imported callable wrappers must bypass lexical root shadows"
    );
    let absolute_imported_identity_wrapper = shell_chrome_writer_audit(
        "mod std {}\n\
         use ::std::convert::identity as keep;\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             let write = keep(|value| {\n\
                 value.shell.chrome.session_state = SessionState::Idle;\n\
             });\n\
             write(app);\n\
         }",
    )
    .expect("absolute imported identity wrapper fixture should parse");
    assert_eq!(
        absolute_imported_identity_wrapper.field_writes.len(),
        1,
        "absolute imported identity calls must preserve closure authority"
    );

    let referenced_inferred_closure = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             let write = |value| {\n\
                 value.shell.chrome.session_state = SessionState::Idle;\n\
             };\n\
             (&write)(app);\n\
         }",
    )
    .expect("referenced inferred closure parameter fixture should parse");
    assert_eq!(
        referenced_inferred_closure.field_writes.len(),
        1,
        "closure argument inference must follow transparent call-target references"
    );

    let aliased_referenced_inferred_closure = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             let write = |value| {\n\
                 value.shell.chrome.session_state = SessionState::Idle;\n\
             };\n\
             let forwarded = &write;\n\
             forwarded(app);\n\
         }",
    )
    .expect("aliased closure reference fixture should parse");
    assert_eq!(
        aliased_referenced_inferred_closure.field_writes.len(),
        1,
        "closure inference must follow references stored in local aliases"
    );

    let inline_inferred_closure = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             (|value| {\n\
                 value.shell.chrome.session_state = SessionState::Idle;\n\
             })(app);\n\
         }",
    )
    .expect("inline inferred closure parameter fixture should parse");
    assert_eq!(
        inline_inferred_closure.field_writes.len(),
        1,
        "inline closure calls must infer shell authority from their arguments"
    );

    let inferred_placeholder_type = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             let write = |app: _| {\n\
                 app.shell.chrome.session_state = SessionState::Idle;\n\
             };\n\
             write(app);\n\
         }",
    )
    .expect("inferred placeholder closure type fixture should parse");
    assert_eq!(
        inferred_placeholder_type.field_writes.len(),
        1,
        "unresolved closure parameter types must use the same conservative authority path"
    );

    let unrelated_inferred_closure = shell_chrome_writer_audit(
        "fn update() {\n\
             let increment = |value| value + 1;\n\
             let _ = increment(1);\n\
         }",
    )
    .expect("unrelated inferred closure fixture should parse");
    assert!(
        unrelated_inferred_closure.field_writes.is_empty()
            && unrelated_inferred_closure.whole_state_writes.is_empty(),
        "untyped closures without shell authority paths must remain harmless"
    );

    let unrelated_aliased_closure = shell_chrome_writer_audit(
        "fn update(other: &mut OtherApp) {\n\
             let write = |value| {\n\
                 value.shell.chrome.session_state = 1;\n\
             };\n\
             let forwarded = &write;\n\
             forwarded(other);\n\
         }",
    )
    .expect("unrelated aliased closure fixture should parse");
    assert!(
        unrelated_aliased_closure.field_writes.is_empty()
            && unrelated_aliased_closure.whole_state_writes.is_empty(),
        "closure aliases must not infer TUI authority from unrelated arguments"
    );

    let unrelated_inline_closure = shell_chrome_writer_audit(
        "fn update(other: &mut OtherApp) {\n\
             (|value| {\n\
                 value.shell.chrome.session_state = 1;\n\
             })(other);\n\
         }",
    )
    .expect("unrelated inline closure fixture should parse");
    assert!(
        unrelated_inline_closure.field_writes.is_empty()
            && unrelated_inline_closure.whole_state_writes.is_empty(),
        "inline closure inference must not infer TUI authority from field names alone"
    );

    let unrelated_shell_shaped_inferred_closure = shell_chrome_writer_audit(
        "fn update(mut other: OtherState) {\n\
             let write = |value| value.shell.chrome.session_state = 1;\n\
             write(&mut other);\n\
         }",
    )
    .expect("unrelated shell-shaped inferred closure fixture should parse");
    assert!(
        unrelated_shell_shaped_inferred_closure
            .field_writes
            .is_empty()
            && unrelated_shell_shaped_inferred_closure
                .whole_state_writes
                .is_empty(),
        "untyped closure parameters must not gain NativeTuiApp authority from field names alone"
    );

    let for_loop_alias = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             for alias in std::iter::once(app) {\n\
                 alias.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("for-loop shell authority alias fixture should parse");
    assert_eq!(
        for_loop_alias.field_writes.len(),
        1,
        "for-loop patterns must retain authority from their iterator element"
    );

    let collection_loop_alias = shell_chrome_writer_audit(
        "fn escape(apps: Vec<&mut NativeTuiApp>) {\n\
             for app in apps {\n\
                 app.shell.chrome.session_state = SessionState::Idle;\n\
             }\n\
         }",
    )
    .expect("collection loop authority fixture should parse");
    assert_eq!(
        collection_loop_alias.field_writes.len(),
        1,
        "collection element types must retain mutable NativeTuiApp authority"
    );

    let shared_collection_loop = shell_chrome_writer_audit(
        "fn inspect(apps: Vec<&mut NativeTuiApp>) {\n\
             for app in apps.iter() {\n\
                 let _ = &app.shell.chrome.session_state;\n\
             }\n\
         }",
    )
    .expect("shared collection loop fixture should parse");
    assert!(
        shared_collection_loop.field_writes.is_empty()
            && shared_collection_loop.whole_state_writes.is_empty(),
        "shared collection iteration must not manufacture mutable authority"
    );

    let unrelated_collection_loop = shell_chrome_writer_audit(
        "fn update(values: Vec<&mut OtherApp>) {\n\
             for value in values {\n\
                 value.shell.chrome.session_state = 1;\n\
             }\n\
         }",
    )
    .expect("unrelated collection loop fixture should parse");
    assert!(
        unrelated_collection_loop.field_writes.is_empty()
            && unrelated_collection_loop.whole_state_writes.is_empty(),
        "unrelated collection elements must not gain NativeTuiApp authority"
    );

    let indexed_collection = shell_chrome_writer_audit(
        "fn escape(mut apps: Vec<NativeTuiApp>) {\n\
             apps[0].shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("indexed collection authority fixture should parse");
    assert_eq!(
        indexed_collection.field_writes.len(),
        1,
        "mutable index expressions must retain collection element authority"
    );

    let first_mut_collection = shell_chrome_writer_audit(
        "fn escape(mut apps: Vec<NativeTuiApp>) {\n\
             apps.first_mut().unwrap().shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("first_mut collection authority fixture should parse");
    assert_eq!(
        first_mut_collection.field_writes.len(),
        1,
        "mutable collection accessors must retain element authority"
    );
    let option_as_mut = shell_chrome_writer_audit(
        "fn escape(mut app: Option<NativeTuiApp>) {\n\
             app.as_mut().unwrap().shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("Option::as_mut authority fixture should parse");
    assert_eq!(
        option_as_mut.field_writes.len(),
        1,
        "mutable wrapper accessors must retain payload authority through unwrap"
    );
    let result_error_as_mut = shell_chrome_writer_audit(
        "fn escape(mut result: Result<(), NativeTuiApp>) {\n\
             result.as_mut().unwrap_err().shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("Result::as_mut error authority fixture should parse");
    assert_eq!(
        result_error_as_mut.field_writes.len(),
        1,
        "Result error extractors must retain the second payload authority"
    );
    let owned_wrapper_payloads = shell_chrome_writer_audit(
        "fn escape_option(app: Option<NativeTuiApp>) {\n\
             app.unwrap().shell.chrome.session_state = SessionState::Idle;\n\
         }\n\
         fn escape_result(result: Result<(), NativeTuiApp>) {\n\
             result.unwrap_err().shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("owned wrapper payload authority fixtures should parse");
    assert_eq!(
        owned_wrapper_payloads.field_writes.len(),
        2,
        "owned success and error payloads must grant mutable temporary authority"
    );

    let read_only_index = shell_chrome_writer_audit(
        "fn inspect(apps: Vec<NativeTuiApp>) {\n\
             let _ = &apps[0].shell.chrome.session_state;\n\
         }",
    )
    .expect("read-only indexed collection fixture should parse");
    assert!(
        read_only_index.field_writes.is_empty() && read_only_index.whole_state_writes.is_empty(),
        "read-only index expressions must not manufacture mutable authority"
    );

    let unrelated_index = shell_chrome_writer_audit(
        "fn update(mut apps: Vec<OtherApp>) {\n\
             apps[0].shell.chrome.session_state = 1;\n\
         }",
    )
    .expect("unrelated indexed collection fixture should parse");
    assert!(
        unrelated_index.field_writes.is_empty() && unrelated_index.whole_state_writes.is_empty(),
        "unrelated indexed elements must not gain NativeTuiApp authority"
    );

    let unrelated_for_loop = shell_chrome_writer_audit(
        "struct OtherState { startup_state: usize }\n\
         fn update(values: Vec<OtherState>) {\n\
             for mut value in values {\n\
                 value.startup_state = 1;\n\
             }\n\
         }",
    )
    .expect("unrelated for-loop fixture should parse");
    assert!(
        unrelated_for_loop.field_writes.is_empty()
            && unrelated_for_loop.whole_state_writes.is_empty(),
        "inferred loops must not treat unrelated same-name fields as Chrome authority"
    );

    let mutable = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             let _ = &mut app.shell.chrome.approval_return_overlay;\n\
         }",
    )
    .expect("mutable shell borrow fixture should parse");
    assert_eq!(mutable.field_writes.len(), 1);

    let whole = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp, state: ShellChromeState) {\n\
             app.shell.chrome = state;\n\
         }",
    )
    .expect("whole shell writer fixture should parse");
    assert_eq!(whole.whole_state_writes.len(), 1);

    let function_argument = shell_chrome_writer_audit(
        "fn escape(chrome: &mut ShellChromeState) {\n\
             std::mem::replace(chrome, Default::default());\n\
         }",
    )
    .expect("mutable function argument fixture should parse");
    assert_eq!(
        function_argument.whole_state_writes.len(),
        1,
        "general function arguments must not expose mutable Chrome authority"
    );

    let method_argument = shell_chrome_writer_audit(
        "fn escape(sink: &mut Sink, chrome: &mut ShellChromeState) {\n\
             sink.replace(chrome);\n\
         }",
    )
    .expect("mutable method argument fixture should parse");
    assert_eq!(
        method_argument.whole_state_writes.len(),
        1,
        "method arguments must not expose mutable Chrome authority"
    );

    let call_returned_app = shell_chrome_writer_audit(
        "fn escape(app: &mut NativeTuiApp) {\n\
             std::convert::identity(app).shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("call-returned mutable app fixture should parse");
    assert_eq!(
        call_returned_app.field_writes.len(),
        1,
        "transparent calls must retain mutable NativeTuiApp authority"
    );

    let imported_call_returned_app = shell_chrome_writer_audit(
        "use std::convert::identity as retain;\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             retain(app).shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("imported call-returned mutable app fixture should parse");
    assert_eq!(
        imported_call_returned_app.field_writes.len(),
        1,
        "imported transparent calls must retain mutable NativeTuiApp authority"
    );

    let locally_returned_app = shell_chrome_writer_audit(
        "fn retain(app: &mut NativeTuiApp) -> &mut NativeTuiApp { app }\n\
         fn escape(app: &mut NativeTuiApp) {\n\
             retain(app).shell.chrome.session_state = SessionState::Idle;\n\
         }",
    )
    .expect("locally returned mutable app fixture should parse");
    assert_eq!(
        locally_returned_app.field_writes.len(),
        1,
        "declared mutable NativeTuiApp returns must retain authority at their call sites"
    );

    let unrelated_call_return = shell_chrome_writer_audit(
        "struct OtherChrome { session_state: usize }\n\
         struct OtherShell { chrome: OtherChrome }\n\
         struct OtherState { shell: OtherShell }\n\
         fn project(_: &mut NativeTuiApp) -> OtherState { todo!() }\n\
         fn update(app: &mut NativeTuiApp) {\n\
             project(app).shell.chrome.session_state = 1;\n\
         }",
    )
    .expect("unrelated call return fixture should parse");
    assert!(
        unrelated_call_return.field_writes.is_empty(),
        "a call argument alone must not retype an unrelated return as NativeTuiApp authority"
    );

    let read_only_argument = shell_chrome_writer_audit(
        "fn inspect(chrome: &ShellChromeState) {\n\
             inspect_state(chrome);\n\
         }",
    )
    .expect("read-only function argument fixture should parse");
    assert!(
        read_only_argument.whole_state_writes.is_empty(),
        "read-only Chrome arguments must remain harmless"
    );

    let shell_source = fs::read_to_string("src/adapter/inbound/tui/shell_chrome.rs")
        .expect("shell chrome source should load");
    let wildcard = shell_source.replacen("ShellChromeEvent::StartupCheckRequested =>", "_ =>", 1);
    let error = verify_shell_chrome_event_reducer_contract(&wildcard)
        .expect_err("a wildcard ShellChromeEvent arm must be rejected");
    assert!(
        error.contains("wildcard patterns are forbidden"),
        "unexpected ShellChromeEvent wildcard analyzer error: {error}"
    );

    let expanded = shell_source.replacen(
        "pub enum ShellChromeEvent {",
        "pub enum ShellChromeEvent {\n    UnroutedProjection,",
        1,
    );
    let error = verify_shell_chrome_event_reducer_contract(&expanded)
        .expect_err("a newly unrouted ShellChromeEvent variant must be rejected");
    assert!(
        error.contains("must exactly cover the enum"),
        "unexpected ShellChromeEvent coverage analyzer error: {error}"
    );
}

#[test]
fn tui_session_renames_enter_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI session renames must be dispatched through core runtime, not a local worker or SessionService handle",
        &["src/adapter/inbound/tui"],
        &[
            ".rename_session(",
            "NativeTuiSessionHandle",
            "BackgroundMessage::SessionRenameCompleted",
            "ConversationLifecycleEvent::SessionRenamed",
        ],
    );

    let overlay_state =
        fs::read_to_string("src/adapter/inbound/tui/app/session_overlay_ui.rs").unwrap();
    let compact_overlay_state: String = overlay_state
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(
        compact_overlay_state.contains("pending_correlation:Option<SessionRenameCorrelation>",),
        "TUI rename pending state must retain the exact Core admission correlation"
    );
    assert!(
        !compact_overlay_state.contains("pending_request:Option<SessionRenameRequest>"),
        "TUI rename pending state must not fall back to request-only matching"
    );

    let controller =
        fs::read_to_string("src/adapter/inbound/tui/app/session_shell_controller.rs").unwrap();
    let compact_controller: String = controller
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(
        compact_controller.contains("SessionRenameAdmissionResolved"),
        "TUI must bind rename pending state from the explicit Core admission event"
    );
    assert!(
        compact_controller.contains("pending_rename_matches(&correlation)"),
        "TUI rename completion must match the full Core correlation"
    );
    assert!(
        !compact_controller.contains("pending_rename_matches(&correlation.request)"),
        "TUI rename completion must not match request fields without generation"
    );

    let completion = top_level_impl_method_source(&controller, "apply_session_rename_completion");
    let compact_completion = rust_code_without_comments_and_literals(&completion)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let catalog_projection = compact_completion
        .find("self.apply_session_catalog_projection(accepted.session_catalog)")
        .expect("accepted rename must project the Core catalog");
    let stream_projection = compact_completion
        .find("self.dispatch_conversation_runtime(")
        .expect("accepted rename must project the Core turn stream");
    let success_pending_gate = compact_completion
        .find("if!exact_pending{return;}")
        .expect("rename success presentation must require the exact local receipt");
    let success_settlement = compact_completion
        .find("self.shell.session_overlay_ui_state.finish_rename_success()")
        .expect("exact rename success must settle local editor presentation");
    let failure_pending_gate = compact_completion
        .rfind("if!exact_pending{return;}")
        .expect("rename failure presentation must require the exact local receipt");
    let failure_settlement = compact_completion
        .find("self.shell.session_overlay_ui_state.finish_rename_failure(")
        .expect("exact rename failure must settle local editor presentation");
    assert!(
        catalog_projection < stream_projection
            && stream_projection < success_pending_gate
            && success_pending_gate < success_settlement,
        "Core-accepted rename semantics must project before local pending gates presentation settlement"
    );
    assert!(
        success_pending_gate < failure_pending_gate && failure_pending_gate < failure_settlement,
        "mismatched failure receipts must not settle a newer local rename editor"
    );
    assert_eq!(
        compact_completion.matches("exact_pending").count(),
        3,
        "exact local pending may be read once and used only by success/failure presentation gates"
    );
}

#[test]
fn tui_planning_runtime_projection_refreshes_enter_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI planning runtime refreshes must dispatch a Core command instead of reading the application service",
        &["src/adapter/inbound/tui"],
        &[
            ".load_runtime_projection_or_invalid(",
            "fn load_planning_runtime_projection(",
            ".inspect_workspace(",
            ".has_planning_workspace(",
            "PLANNING_RUNTIME_LOADING_STATUS",
        ],
    );

    let controller =
        fs::read_to_string("src/adapter/inbound/tui/app/conversation/controller.rs").unwrap();
    assert!(
        controller.contains("AppCommand::RefreshPlanningRuntime {"),
        "TUI planning runtime refreshes must positively enter through the typed Core command"
    );
    let planning_controller =
        fs::read_to_string("src/adapter/inbound/tui/app/planning/controller.rs").unwrap();
    assert!(
        planning_controller.contains("PlanningRuntimeRefreshOperation::Doctor")
            && planning_controller.contains("PlanningRuntimeRefreshOperation::ResetRecovery"),
        "Planning doctor and reset recovery must share the typed Core refresh effect"
    );
    let core_controller = fs::read_to_string("src/core/app/controller.rs").unwrap();
    let planning_reducer = fs::read_to_string("src/core/app/planning_reducer.rs").unwrap();
    assert!(
        core_controller.contains("planning: PlanningFeatureReducer")
            && planning_reducer.contains("runtime_refresh: PlanningRuntimeCoordinator"),
        "CoreController must delegate planning runtime generation and active-correlation ownership to its planning feature reducer"
    );
    assert!(
        !core_controller.contains("next_planning_runtime_refresh_generation:")
            && !core_controller.contains(
                "active_planning_runtime_refresh: Option<PlanningRuntimeRefreshCorrelation>",
            ),
        "CoreController must not restore raw planning runtime generation or active-correlation fields"
    );
    assert!(
        !core_controller.contains(".settle_workspace(")
            && core_controller.contains(".restart_runtime_refresh_if_matches(")
            && planning_reducer.contains(".restart_if_matches("),
        "planning projection writers must supersede and replace, never settle, an exact refresh operation"
    );
    let effect_runner = fs::read_to_string("src/composition/core_effect_runner.rs").unwrap();
    assert!(
        effect_runner.contains(".inspect_runtime_projection(")
            && !effect_runner.contains(".has_planning_workspace("),
        "the Core planning refresh worker must use one coherent inspection use case"
    );
    let refresh_ui =
        fs::read_to_string("src/adapter/inbound/tui/app/planning_runtime_refresh_ui.rs").unwrap();
    assert!(
        refresh_ui.contains("presentation_revision") && refresh_ui.contains("fn rebind("),
        "planning runtime completion must be gated by a typed presentation revision and rebind replacement inspections"
    );
}

#[test]
fn tui_planning_reset_enters_through_one_core_owned_async_coordinator() {
    for (callable_name, operation) in [
        ("reset_workspace", "reset"),
        ("stage_simple_mode_draft", "simple draft stage"),
        (
            "stage_manual_editor_session",
            "planning manual editor stage",
        ),
        (
            "stage_detail_doc_editor_session",
            "direction detail editor stage",
        ),
        (
            "stage_queue_idle_prompt_editor_session",
            "queue-idle prompt editor stage",
        ),
        ("load_manual_editor_session", "simple editor load"),
        ("promote_staged_draft", "simple draft promotion"),
        ("save_draft_editor_files", "editor save"),
        ("promote_draft_editor_files", "editor promotion"),
    ] {
        assert_no_production_callable_reference_named_in_paths(
            &format!(
                "TUI planning {operation} must dispatch a Core command instead of invoking the workspace service"
            ),
            &["src/adapter/inbound/tui"],
            callable_name,
        );
    }
    assert_no_forbidden_references_in_paths(
        "TUI planning reset controller must leave worker ownership in composition",
        &["src/adapter/inbound/tui/app/planning/controller.rs"],
        &["std::thread::spawn"],
    );

    let planning_controller =
        fs::read_to_string("src/adapter/inbound/tui/app/planning/controller.rs").unwrap();
    let planning_editor_controller =
        fs::read_to_string("src/adapter/inbound/tui/app/planning/controller/editor.rs").unwrap();
    let compact_planning_controller = planning_controller
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let compact_planning_editor_controller = planning_editor_controller
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_planning_controller.contains(
            "reduce_core_client_event(CoreInput::Command(AppCommand::ResetPlanningWorkspace("
        ) && compact_planning_controller.contains(
            "reduce_core_client_event(CoreInput::Command(AppCommand::StageSimplePlanningDraft"
        ) && compact_planning_editor_controller.contains(
            "reduce_core_client_event(CoreInput::Command(AppCommand::StagePlanningEditor"
        ) && compact_planning_controller.contains(
            "reduce_core_client_event(CoreInput::Command(AppCommand::LoadSimplePlanningEditor"
        ) && compact_planning_controller.contains(
            "reduce_core_client_event(CoreInput::Command(AppCommand::PromoteSimplePlanningDraft"
        ) && planning_controller.contains("PlanningWorkspaceOperationUiSettlement::Applied"),
        "TUI reset and simple authoring must enter through the typed client event and gate presentation on exact UI settlement"
    );
    assert!(
        planning_controller
            .contains("fn reset_busy_gate_and_exact_completion_survive_workspace_aba()")
            && planning_controller.contains(
                "fn successful_reset_refreshes_runtime_under_newer_busy_status_without_blocking_dispatch()"
            )
            && planning_controller.contains(
                "fn failed_reset_refreshes_runtime_and_preserves_newer_status_presentation()"
            )
            && planning_controller.contains(
                "fn simple_authoring_completions_do_not_reopen_after_close_or_workspace_drift()"
            )
            && planning_controller.contains(
                "fn stale_simple_promotion_refreshes_authority_without_closing_or_replacing_newer_status()"
            )
            && planning_controller.contains(
                "fn late_simple_editor_load_cannot_replace_a_newer_editor_session_identity()"
            )
            && planning_controller.contains(
                "fn coalesced_planning_editor_retry_rebinds_current_revision_and_reopens_loading()"
            )
            && planning_controller.contains(
                "fn approval_during_direction_editor_staging_restores_confirm_without_opening_editor()"
            )
            && planning_controller
                .contains("app.sync_draft_shell_workspace(workspace_b.path_str());")
            && planning_controller
                .contains("app.sync_draft_shell_workspace(workspace_a.path_str());"),
        "TUI integration coverage must prove the active reset gate and exact completion across A→B→A workspace drift"
    );
    let operation_ui =
        fs::read_to_string("src/adapter/inbound/tui/app/planning_workspace_operation_ui.rs")
            .unwrap();
    assert!(
        operation_ui.contains("presentation_revision")
            && operation_ui.contains("current_workspace_directory")
            && operation_ui.contains("pending.correlation != *correlation")
            && operation_ui
                .contains("PlanningWorkspaceOperationUiSettlement::PresentationSuperseded"),
        "TUI reset settlement must reject stale generation, workspace, and presentation intent"
    );
    let editor_stage_settlement = planning_controller
        .split_once("fn apply_planning_editor_stage_completion(")
        .and_then(|(_, body)| body.split_once("fn apply_planning_editor_mutation_completion("))
        .map(|(body, _)| body)
        .expect("planning editor stage settlement should have a bounded source body");
    for forbidden in [
        "pause_post_turn_continuation_after_authority_mutation",
        "refresh_ready_conversation_planning_runtime_projection",
        "begin_planning_runtime_projection_refresh",
    ] {
        assert!(
            !editor_stage_settlement.contains(forbidden),
            "editor staging must not trigger runtime refresh or post-turn pause: {forbidden}"
        );
    }
    let editor_save_settlement = planning_controller
        .split_once("fn apply_planning_editor_save_completion(")
        .and_then(|(_, body)| body.split_once("fn apply_planning_editor_promote_completion("))
        .map(|(body, _)| body)
        .expect("planning editor save settlement should have a bounded source body");
    for forbidden in [
        "pause_post_turn_continuation_after_authority_mutation",
        "refresh_ready_conversation_planning_runtime_projection",
        "close_shell_overlay",
        "start_directions_maintenance_overview_load",
    ] {
        assert!(
            !editor_save_settlement.contains(forbidden),
            "editor save must preserve the overlay and skip runtime refresh/pause: {forbidden}"
        );
    }
    let editor_promote_settlement = planning_controller
        .split_once("fn apply_planning_editor_promote_completion(")
        .and_then(|(_, body)| body.split_once("fn planning_editor_mutation_target_is_current("))
        .map(|(body, _)| body)
        .expect("planning editor promote settlement should have a bounded source body");
    for required in [
        "pause_post_turn_continuation_after_authority_mutation",
        "refresh_ready_conversation_planning_runtime_projection_for_workspace",
        "PlanningWorkspaceOperationUiSettlement::Applied",
    ] {
        assert!(
            editor_promote_settlement.contains(required),
            "editor promotion settlement is missing its authority or exact-presentation contract: {required}"
        );
    }
    for required in [
        "fn planning_workspace_operation_busy_label(",
        "fn editor_mutation_late_save_reconciles_only_the_matching_session_and_revision()",
        "fn editor_mutation_exact_promote_always_refreshes_but_only_success_closes()",
        "fn editor_mutation_late_promote_never_closes_a_newer_or_suspended_editor()",
        "fn editor_mutation_promote_refreshes_after_workspace_aba_but_not_in_workspace_b()",
        "fn editor_mutation_coalesced_retry_rebinds_current_presentation_revision()",
        "fn planning_editor_workspace_drift_blocks_mutation_without_clearing_close_confirmation()",
        "fn directions_detail_doc_editor_promotes_back_to_maintenance_overview()",
    ] {
        assert!(
            planning_controller.contains(required),
            "TUI editor mutation temporal coverage is missing: {required}"
        );
    }

    let core_controller = fs::read_to_string("src/core/app/controller.rs").unwrap();
    let planning_reducer = fs::read_to_string("src/core/app/planning_reducer.rs").unwrap();
    assert!(
        core_controller.contains("planning: PlanningFeatureReducer")
            && planning_reducer
                .contains("workspace_operations: PlanningWorkspaceOperationCoordinator")
            && core_controller.contains(".begin_workspace_operation(intent)")
            && core_controller.contains(".accept_workspace_operation(&correlation)")
            && planning_reducer.contains(".begin(intent)")
            && planning_reducer.contains(".accept(correlation)")
            && core_controller.contains("correlation.reset_target() == Some(snapshot.target)")
            && core_controller.contains("PlanningWorkspaceOperationKind::StageSimpleDraft")
            && core_controller.contains("PlanningWorkspaceOperationKind::StageEditor")
            && core_controller.contains("PlanningWorkspaceOperationKind::MutateEditor")
            && core_controller.contains("AppCommand::MutatePlanningEditor")
            && core_controller.contains("PlanningWorkspaceOperationKind::LoadSimpleEditor")
            && core_controller.contains("PlanningWorkspaceOperationKind::PromoteSimpleDraft"),
        "CoreController must delegate reset and simple-authoring admission plus exact settlement to one coordinator"
    );
    let coordinator = fs::read_to_string("src/core/app/planning_workspace.rs").unwrap();
    for required in [
        "PlanningWorkspaceOperationAdmission::Coalesced",
        "PlanningWorkspaceOperationAdmission::Busy",
        "PlanningEditorSessionIdentity",
        "PlanningEditorStageTarget",
        "PlanningEditorMutationIdentity",
        "PlanningEditorMutationRequest",
        "StageEditor",
        "MutateEditor",
        "LoadSimpleEditor",
        "PromoteSimpleDraft",
        "checked_add(1)",
    ] {
        assert!(
            coordinator.contains(required),
            "planning workspace coordinator is missing contract: {required}"
        );
    }
    for forbidden in [
        "VecDeque",
        "cancel(",
        "actor",
        "saving…",
        "promoting…",
        "planning editor save",
        "planning editor promotion",
    ] {
        assert!(
            !coordinator.contains(forbidden),
            "planning workspace coordinator must not add queue/actor/cancellation machinery: {forbidden}"
        );
    }

    let effect_runner = fs::read_to_string("src/composition/core_effect_runner.rs").unwrap();
    let reset_worker = effect_runner
        .split_once("fn spawn_planning_workspace_reset(")
        .and_then(|(_, body)| body.split_once("fn spawn_simple_planning_draft_stage("))
        .map(|(body, _)| body)
        .expect("planning reset worker should have a bounded source body");
    assert!(
        reset_worker.contains("spawn_effect_completion_worker(")
            && reset_worker.contains("catch_redacted_worker_unwind")
            && reset_worker.contains("PlanningWorkspaceResetCompleted"),
        "planning reset provider I/O and redacted panic conversion must stay in the Core effect worker"
    );
    for required in [
        "fn spawn_simple_planning_draft_stage(",
        "fn spawn_planning_editor_stage(",
        "fn spawn_planning_editor_mutation(",
        "fn spawn_simple_planning_editor_load(",
        "fn spawn_simple_planning_draft_promotion(",
        "PlanningSimpleDraftStaged",
        "PlanningEditorStaged",
        "PlanningEditorMutationCompleted",
        "PlanningSimpleEditorLoaded",
        "PlanningSimpleDraftPromoted",
        "planning simple draft stage worker panicked",
        "planning editor stage worker panicked",
        "planning editor mutation worker panicked",
        "planning simple editor load worker panicked",
        "planning simple draft promotion worker panicked",
    ] {
        assert!(
            effect_runner.contains(required),
            "simple authoring effect runner is missing typed async contract: {required}"
        );
    }
    assert!(
        effect_runner.contains(
            "fn simple_draft_stage_dispatch_returns_within_300ms_while_provider_stays_gated()"
        ) && effect_runner.contains(
            "fn planning_editor_stage_dispatch_is_non_blocking_and_exact_duplicates_coalesce()"
        ) && effect_runner.contains(
            "fn planning_editor_stage_runner_rejects_wrong_operation_target_session_and_draft()"
        ) && effect_runner.contains(
            "fn planning_editor_mutation_rejects_every_identity_mismatch_before_provider_io()"
        ) && effect_runner.contains(
            "fn planning_editor_mutation_dispatch_is_non_blocking_and_coordinates_duplicates()"
        ) && effect_runner.contains(
            "fn planning_editor_stage_worker_panic_is_redacted_and_reopens_admission()"
        ) && effect_runner.contains(
            "fn planning_editor_save_and_promote_panics_are_redacted_once_and_reopen_admission()"
        ) && effect_runner.contains(
            "fn simple_editor_worker_panic_returns_one_redacted_completion_and_reopens_admission()"
        ) && effect_runner
            .contains("fn mismatched_simple_authoring_source_session_never_calls_the_provider()"),
        "simple authoring workers must prove bounded dispatch, fail-closed source identity, and panic settlement"
    );

    let tui_root = repo_root().join("src/adapter/inbound/tui");
    let mut direct_workspace_calls = 0;
    for path in rust_files_under(&tui_root) {
        if is_test_only_path(&path) {
            continue;
        }
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for (_, calls, _) in top_level_impl_method_calls(&source) {
            direct_workspace_calls += calls.iter().filter(|(name, _)| name == "workspace").count();
        }
    }
    assert_eq!(
        direct_workspace_calls, 0,
        "production TUI must not expose PlanningWorkspaceUseCases through `.workspace()`"
    );
}

#[test]
fn planning_workspace_ast_guard_rejects_all_callable_forms_and_ignored_busy_results() {
    for callable_name in [
        "reset_workspace",
        "stage_simple_mode_draft",
        "stage_manual_editor_session",
        "stage_detail_doc_editor_session",
        "stage_queue_idle_prompt_editor_session",
        "load_manual_editor_session",
        "promote_staged_draft",
        "save_draft_editor_files",
        "promote_draft_editor_files",
    ] {
        let text_only = format!(
            r#"
                fn sample() {{
                    // PlanningWorkspaceUseCases::{callable_name}(...)
                    let _copy = "{callable_name}";
                    stringify!("{callable_name}");
                }}

                #[cfg(test)]
                fn test_only(service: &PlanningWorkspaceUseCases) {{
                    invoke!(PlanningWorkspaceUseCases::{callable_name});
                    service.{callable_name}();
                }}
            "#
        );
        assert!(
            production_callable_reference_lines(&text_only, callable_name).is_empty(),
            "comments, literals, and cfg(test) references must not create a production violation for {callable_name}"
        );

        for callable_source in [
            format!(
                "fn sample(service: &PlanningWorkspaceUseCases) {{ service.{callable_name}(); }}"
            ),
            format!(
                "fn sample(service: &PlanningWorkspaceUseCases) {{ PlanningWorkspaceUseCases::{callable_name}(service); }}"
            ),
            format!(
                "fn sample() {{ let callable = PlanningWorkspaceUseCases::{callable_name}; use_it(callable); }}"
            ),
            format!("fn sample() {{ invoke!(PlanningWorkspaceUseCases::{callable_name}); }}"),
        ] {
            assert!(
                !production_callable_reference_lines(&callable_source, callable_name).is_empty(),
                "method, UFCS, function-pointer, path, and macro-token references must be detected for {callable_name}: {callable_source}"
            );
        }
    }

    let guarded = r#"
        impl App {
            fn mutate(&mut self) {
                if self
                    .planning_workspace_operation_blocks_direct_mutation()
                {
                    return;
                }
                self.application.planning().workspace().save();
            }
        }
    "#;
    let guarded_methods = top_level_impl_method_calls(guarded);
    assert_eq!(guarded_methods.len(), 1);
    assert!(guarded_methods[0].2);

    for bypass in [
        r#"
            impl App {
                fn mutate(&mut self) {
                    self.planning_workspace_operation_blocks_direct_mutation();
                    self.application.planning().workspace().save();
                }
            }
        "#,
        r#"
            impl App {
                fn mutate(&mut self) {
                    if self.planning_workspace_operation_blocks_direct_mutation() {
                        observe_only();
                    }
                    self.application.planning().workspace().save();
                }
            }
        "#,
        r#"
            impl App {
                fn mutate(&mut self) {
                    self.application.planning().workspace().save();
                    if self.planning_workspace_operation_blocks_direct_mutation() {
                        return;
                    }
                }
            }
        "#,
    ] {
        let methods = top_level_impl_method_calls(bypass);
        assert_eq!(methods.len(), 1);
        assert!(
            !methods[0].2,
            "ignored, non-returning, or late busy guards must not satisfy the boundary"
        );
    }
}

#[test]
fn tui_directions_maintenance_loads_enter_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI directions maintenance reads must dispatch a Core command instead of loading planning authority directly",
        &["src/adapter/inbound/tui"],
        &[".load_summary("],
    );

    let controller =
        fs::read_to_string("src/adapter/inbound/tui/app/planning/controller.rs").unwrap();
    assert!(
        controller.contains("AppCommand::LoadDirectionsMaintenance {"),
        "TUI directions maintenance loads must positively enter through the typed Core command"
    );
}

#[test]
fn tui_review_presentation_reads_screen_model_without_effects() {
    // Review Center authority loading belongs to the controller/effect path. Presentation must
    // remain a pure projection of the request-correlated immutable screen model.
    assert_no_forbidden_references_in_paths(
        "TUI Review Center presentation must consume its screen model without service or I/O effects",
        &["src/adapter/inbound/tui/app/shell_presentation/overlays/popup/reviews.rs"],
        &[
            "NativeTuiApp",
            "crate::application::",
            ".application",
            "load_review_center_",
            "ReviewCenterRepositoryPort",
            "std::fs",
            "std::thread",
            "std::sync",
        ],
    );
}

#[test]
fn tui_review_center_loads_enter_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI Review Center authority reads must run as core effects from every TUI ownership module",
        &["src/adapter/inbound/tui/app"],
        &[
            "load_reviews_overlay_authority",
            ".load_review_center_thread_reviews_for_workspace(",
            ".load_review_center_pending_inbox_for_workspace(",
            ".load_review_center_recent_history_for_workspace(",
            "BackgroundMessage::ReviewsOverlayLoaded",
            "next_request_id",
        ],
    );
    assert_no_forbidden_references_in_paths(
        "TUI Review Center controller must not spawn its own authority worker",
        &["src/adapter/inbound/tui/app/shell_controller.rs"],
        &["std::thread::spawn"],
    );

    let controller = fs::read_to_string("src/adapter/inbound/tui/app/shell_controller.rs").unwrap();
    assert!(
        controller.contains("AppCommand::LoadReviewCenter {"),
        "TUI Review Center loads must positively enter through AppCommand::LoadReviewCenter"
    );
}

#[test]
fn tui_queue_presentation_reads_screen_model_without_effects() {
    // Queue authority loading and mutation belong to controller/effect paths. Presentation must
    // remain a pure projection of the request-correlated immutable screen model.
    assert_no_forbidden_references_in_paths(
        "TUI queue presentation must consume its screen model without service or I/O effects",
        &["src/adapter/inbound/tui/app/shell_presentation/overlays/popup/queue.rs"],
        &[
            "NativeTuiApp",
            "NativeTuiApplicationHandle",
            "NativeTuiPlanningHandle",
            ".application",
            ".planning()",
            ".queue()",
            "execute_queue_mutation",
            "load_authority_snapshot",
            "load_queue_authority",
            "refresh_queue_",
            "std::fs",
            "std::thread",
            "std::sync",
        ],
    );
    assert_no_forbidden_references_in_paths(
        "TUI queue presentation facade must only project app state into the pure screen model",
        &["src/adapter/inbound/tui/app/shell_presentation.rs"],
        &[
            ".application",
            ".planning()",
            ".queue()",
            "execute_queue_mutation",
            "load_authority_snapshot",
            "load_queue_authority",
            "refresh_queue_",
            "std::fs",
            "std::thread",
            "std::sync",
        ],
    );
    let facade =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/shell_presentation.rs"))
            .expect("queue presentation facade source should load");
    let syntax = syn::parse_file(&facade).expect("queue presentation facade should parse");
    let function = syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "build_queue_overlay_view" => {
                Some(function)
            }
            _ => None,
        })
        .expect("queue presentation facade should expose build_queue_overlay_view");
    assert!(
        matches!(
            function.block.stmts.as_slice(),
            [syn::Stmt::Expr(syn::Expr::Call(call), None)]
                if matches!(
                    call.func.as_ref(),
                    syn::Expr::Path(path)
                        if path.path.is_ident("build_queue_overlay_view_from_screen_model")
                )
                    && matches!(
                        call.args.iter().collect::<Vec<_>>().as_slice(),
                        [syn::Expr::MethodCall(method)]
                            if method.method == "queue_overlay_screen_model"
                                && method.args.is_empty()
                                && matches!(
                                    method.receiver.as_ref(),
                                    syn::Expr::Path(receiver)
                                        if receiver.path.is_ident("app")
                                )
                    )
        ),
        "queue presentation facade must be one pure screen-model delegation"
    );
}

#[test]
fn tui_queue_authority_loads_enter_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI Queue authority loads must run as core effects",
        &["src/adapter/inbound/tui/app"],
        &[
            ".load_coherent_authority(",
            "BackgroundMessage::QueueOverlayAuthorityLoaded",
            "next_authority_request_id",
            "QueueOverlayAuthorityLoadResult",
        ],
    );

    let controller =
        fs::read_to_string("src/adapter/inbound/tui/app/queue_overlay_controller.rs").unwrap();
    assert!(
        controller.contains("AppCommand::LoadQueueAuthority {"),
        "TUI Queue authority loads must positively enter through AppCommand::LoadQueueAuthority"
    );
}

#[test]
fn tui_queue_mutations_enter_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI Queue mutations must not schedule or execute their application transaction",
        &["src/adapter/inbound/tui/app"],
        &[
            "thread::spawn",
            ".execute_cancellation_transaction(",
            "PlanningQueueCancellationRequest",
            "PlanningQueueCancellationTarget",
        ],
    );
    assert_no_forbidden_references_in_paths(
        "TUI Queue mutation correlation and completion must be core-owned",
        &["src/adapter/inbound/tui/app"],
        &[
            "BackgroundMessage::QueueMutationCompleted",
            "QueueMutationOperation",
            "QueueMutationWorkerResult",
            "next_operation_id",
        ],
    );

    let controller =
        fs::read_to_string("src/adapter/inbound/tui/app/queue_overlay_controller.rs").unwrap();
    let compact_controller = controller
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        controller.contains("QueueMutationIntent {")
            && compact_controller.contains(
                ".reduce_core_client_event(CoreInput::Command(AppCommand::SubmitQueueMutation("
            )
            && compact_controller.contains("Box::new(intent)"),
        "TUI Queue mutations must positively enter through AppCommand::SubmitQueueMutation"
    );
}

#[test]
fn tui_generic_shell_controller_does_not_own_queue_mutation_effects() {
    assert_no_forbidden_references_in_paths(
        "TUI generic shell controller must delegate queue mutation effects",
        &["src/adapter/inbound/tui/app/shell_controller.rs"],
        &[
            "execute_queue_mutation",
            "load_authority_snapshot",
            "load_queue_authority",
            ".planning()",
            ".queue()",
            ".cancel_tasks(",
            "PlanningQueueCancellationRequest",
            "PlanningQueueCancellationTarget",
            "QueueMutationOperation",
            "QueueOverlayAuthorityLoad",
            "QueueMutationWorkerResult",
            "cancel_selected_queue_task",
            "start_queue_cancellation",
            "apply_queue_mutation_completion",
        ],
    );
}

#[test]
fn tui_queue_adapter_does_not_own_cancellation_authority_transaction() {
    assert_no_forbidden_references_in_paths(
        "TUI queue controller must not schedule or compose the application-owned cancellation and authority transaction",
        &["src/adapter/inbound/tui/app/queue_overlay_controller.rs"],
        &[
            ".cancel_tasks(",
            ".load_authority_snapshot(",
            ".load_runtime_projection_or_invalid(",
            "RevisionsKeptChanging",
        ],
    );
    assert_no_forbidden_references_in_paths(
        "TUI planning handle must not rebuild the queue cancellation and authority transaction",
        &["src/adapter/inbound/tui/app/app_runtime.rs"],
        &[
            "fn execute_queue_mutation",
            "fn load_queue_authority",
            "QueueMutationAuthorityRefreshError",
            "QueueMutationAuthoritySnapshot",
        ],
    );

    let use_cases =
        fs::read_to_string(repo_root().join("src/application/service/planning/use_cases.rs"))
            .expect("planning use-case source should load");
    let production_source = production_lines(&use_cases)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    for required_method in [
        "pub fn execute_cancellation_transaction(",
        "pub fn load_coherent_authority(",
    ] {
        assert!(
            production_source.contains(required_method),
            "PlanningQueueUseCases must own queue cancellation and coherent authority readback through {required_method}"
        );
    }
}

#[test]
fn tui_shell_renderer_consumes_one_owned_frame_without_app_or_effects() {
    const RENDERER_PATHS: &[&str] = &[
        "src/adapter/inbound/tui/app/shell_rendering.rs",
        "src/adapter/inbound/tui/app/shell_rendering",
    ];

    assert_no_semantic_references_in_paths(
        "TUI shell renderer must not import Core, application, outbound, terminal, or I/O authority",
        RENDERER_PATHS,
        &[
            "crate::application",
            "crate::composition",
            "crate::core",
            "crate::adapter::outbound",
            "crossterm",
            "reqwest",
            "sqlx",
            "std::fs",
            "std::io",
            "std::net",
            "std::process",
            "std::sync",
            "std::thread",
            "std::time",
            "tokio",
        ],
        &[],
    );

    let repo_root = repo_root();
    let mut violations = Vec::new();
    for path_suffix in RENDERER_PATHS {
        for path in rust_files_for_path(&repo_root.join(path_suffix)) {
            if is_test_only_path(&path) {
                continue;
            }
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            for (line, violation) in renderer_production_boundary_violations(&source) {
                violations.push(format!(
                    "{}:{line}: {violation}",
                    relative_path(&repo_root, &path)
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "TUI shell renderer must receive only owned/immutable frame data; production app, authority, glob-import, and non-Frame mutable-input violations:\n{}",
        violations.join("\n")
    );

    let frame_model_path = repo_root.join("src/adapter/inbound/tui/app/inline_frame_model.rs");
    assert_no_semantic_references_in_paths(
        "inline frame capture may sample UI state but must not reacquire Core, application, control-plane, or outbound authority",
        &["src/adapter/inbound/tui/app/inline_frame_model.rs"],
        &[
            "crate::composition",
            "crate::core",
            "crate::adapter::outbound",
        ],
        &[],
    );
    let frame_model_source =
        fs::read_to_string(&frame_model_path).expect("owned inline frame model source should load");
    let frame_model_syntax =
        syn::parse_file(&frame_model_source).expect("owned inline frame model source should parse");
    let capture_authority_references = rust_semantic_references(&frame_model_source)
        .paths
        .into_iter()
        .filter(|reference| {
            reference.split("::").any(|segment| {
                matches!(
                    segment,
                    "CoreController"
                        | "CoreRuntime"
                        | "NativeClientRuntime"
                        | "NativeTuiApplicationHandle"
                ) || segment.contains("ControlPlane")
                    || segment.ends_with("Service")
                    || segment.ends_with("Services")
                    || segment.ends_with("Port")
                    || segment.ends_with("Repository")
                    || segment.ends_with("Handle")
                    || segment.ends_with("UseCases")
                    || segment.ends_with("Effect")
                    || segment.ends_with("EffectRunner")
            })
        })
        .collect::<Vec<_>>();
    assert!(
        capture_authority_references.is_empty(),
        "inline frame capture may carry application read DTOs but no service, port, handle, effect, Core, or control-plane authority: {capture_authority_references:?}"
    );
    for forbidden_field in [
        "application",
        "client_runtime",
        "core_runtime",
        "parallel_mode_control_plane",
    ] {
        let mut field_reads = NamedFieldAccessVisitor::new(forbidden_field);
        field_reads.visit_file(&frame_model_syntax);
        assert!(
            field_reads.lines.is_empty(),
            "inline frame capture must not reacquire `{forbidden_field}` authority: {:?}",
            field_reads.lines
        );
    }
    for forbidden_call in [
        "build_parallel_peek_overlay_view",
        "build_planning_init_overlay_view",
        "build_queue_overlay_view",
        "dispatch_client_event",
        "dispatch_core_command",
        "planning_runtime_projection_snapshot",
        "poll_pending_client_event",
        "presentation_projection",
        "revisioned_planning_parallel_projection",
        "snapshot",
    ] {
        assert!(
            production_callable_reference_lines(&frame_model_source, forbidden_call).is_empty(),
            "inline frame capture must not call `{forbidden_call}`; the terminal transaction supplies its sampled authority projection"
        );
    }
    let capture_source =
        top_level_function_source(&frame_model_source, "capture_inline_shell_frame_model");
    for sampled_builder in [
        "build_parallel_peek_overlay_view_from_snapshot",
        "build_planning_init_overlay_view_from_projection",
        "build_queue_overlay_view_from_projection",
    ] {
        assert_eq!(
            production_callable_reference_lines(&capture_source, sampled_builder).len(),
            1,
            "inline frame capture must use one sampled `{sampled_builder}` call"
        );
    }
    for (sampled_field, expected_reads) in [
        ("projection.sampled_parallel_supervisor", 1usize),
        ("projection.sampled_planning_runtime_projection", 2usize),
    ] {
        assert_eq!(
            capture_source.matches(sampled_field).count(),
            expected_reads,
            "overlay builders must consume the transaction-owned `{sampled_field}` sample"
        );
    }
    for type_name in [
        "InlineShellFrameModel",
        "InlineInspectionFrameModel",
        "InlineFrameRenderReceipt",
    ] {
        let item = frame_model_syntax
            .items
            .iter()
            .find(|item| match item {
                syn::Item::Struct(item) => item.ident == type_name,
                syn::Item::Enum(item) => item.ident == type_name,
                _ => false,
            })
            .unwrap_or_else(|| panic!("inline frame boundary must define {type_name}"));
        let generics = match item {
            syn::Item::Struct(item) => &item.generics,
            syn::Item::Enum(item) => &item.generics,
            _ => unreachable!("matched only owned frame structs/enums"),
        };
        assert!(
            generics.params.is_empty(),
            "{type_name} must be owned and must not carry a borrowed lifetime"
        );
    }
    for documentation_path in [
        "docs/design/07-tui-layered-architecture-and-aesthetic-contract.md",
        "docs/ko/reference/tui-contract.md",
        "docs/reference/architecture.md",
        "docs/ko/reference/architecture.md",
        "docs/validation/terminal-ui-testing-methodology.md",
        "docs/validation/tui-coverage-matrix.md",
    ] {
        let documentation = fs::read_to_string(repo_root.join(documentation_path))
            .unwrap_or_else(|error| panic!("{documentation_path} should load: {error}"));
        for contract_name in [
            "InlineShellFrameModel",
            "InlineInspectionFrameModel",
            "InlineFrameRenderReceipt",
        ] {
            assert!(
                documentation.contains(contract_name),
                "{documentation_path} must document the owned frame contract `{contract_name}`"
            );
        }
    }

    let capture = top_level_function(&frame_model_syntax, "capture_inline_shell_frame_model");
    assert_eq!(
        capture.sig.inputs.len(),
        4,
        "frame capture must receive app, frontend mode, area, and the sampled conversation projection"
    );
    let capture_inputs = capture.sig.inputs.iter().collect::<Vec<_>>();
    assert!(
        matches!(
            capture_inputs[0],
            syn::FnArg::Typed(argument)
                if matches!(
                    argument.ty.as_ref(),
                    syn::Type::Reference(reference)
                        if reference.mutability.is_none()
                            && is_named_path_type(reference.elem.as_ref(), "NativeTuiApp")
                )
        ),
        "frame capture must receive NativeTuiApp through an immutable reference"
    );
    assert!(
        matches!(
            &capture.sig.output,
            syn::ReturnType::Type(_, ty) if is_named_path_type(ty, "InlineShellFrameModel")
        ),
        "frame capture must return InlineShellFrameModel"
    );
    let apply = top_level_function(&frame_model_syntax, "apply_inline_frame_render_receipt");
    assert_eq!(
        apply.sig.inputs.len(),
        2,
        "receipt application must receive only app and the delivered frame receipt"
    );

    let rendering_path = repo_root.join("src/adapter/inbound/tui/app/shell_rendering.rs");
    let rendering_source =
        fs::read_to_string(&rendering_path).expect("shell rendering source should load");
    let rendering_syntax =
        syn::parse_file(&rendering_source).expect("shell rendering source should parse");
    let draw = top_level_function(&rendering_syntax, "draw_projected");
    assert_eq!(
        draw.sig.inputs.len(),
        3,
        "draw_projected must receive only Frame, ShellFrontendMode, and InlineShellFrameModel"
    );
    let draw_inputs = draw.sig.inputs.iter().collect::<Vec<_>>();
    assert!(
        matches!(
            draw_inputs[0],
            syn::FnArg::Typed(argument)
                if matches!(
                    argument.ty.as_ref(),
                    syn::Type::Reference(reference)
                        if reference.mutability.is_some()
                            && is_renderer_frame_type(&reference.elem)
                )
        ),
        "draw_projected first input must be &mut Frame"
    );
    for (index, expected_type) in [
        (1usize, "ShellFrontendMode"),
        (2usize, "InlineShellFrameModel"),
    ] {
        assert!(
            matches!(
                draw_inputs[index],
                syn::FnArg::Typed(argument)
                    if is_named_path_type(&argument.ty, expected_type)
            ),
            "draw_projected input {index} must be {expected_type}"
        );
    }
    assert!(
        matches!(
            &draw.sig.output,
            syn::ReturnType::Type(_, ty) if is_named_path_type(ty, "InlineFrameRenderReceipt")
        ),
        "draw_projected must return InlineFrameRenderReceipt"
    );

    let terminal_source = fs::read_to_string(
        repo_root.join("src/adapter/inbound/tui/app/inline_terminal_adapter.rs"),
    )
    .expect("inline terminal adapter source should load");
    let terminal_function = top_level_function_source(&terminal_source, "draw_inline_frame");
    let draw_lines = production_callable_reference_lines(&terminal_function, "draw_projected");
    let stable_lines =
        production_callable_reference_lines(&terminal_function, "matches_resize_snapshot");
    let commit_lines =
        production_callable_reference_lines(&terminal_function, "commit_frame_render_receipt");
    assert_eq!(
        draw_lines.len(),
        1,
        "one terminal frame must invoke the pure renderer exactly once"
    );
    assert_eq!(
        commit_lines.len(),
        1,
        "one stable terminal frame must commit its render receipt exactly once"
    );
    assert!(
        stable_lines
            .iter()
            .any(|stable_line| draw_lines[0] < *stable_line && *stable_line < commit_lines[0]),
        "render receipt must apply only after the post-draw resize snapshot remains stable"
    );
    assert!(
        production_callable_reference_lines(
            &terminal_function,
            "apply_inline_frame_render_receipt"
        )
        .is_empty(),
        "draw_inline_frame must commit through the attempt-aware receipt gate"
    );
    let commit_receipt =
        top_level_impl_method_source(&terminal_source, "commit_frame_render_receipt");
    assert_eq!(
        production_callable_reference_lines(&commit_receipt, "commit_receipt").len(),
        1,
        "the attempt-aware gate must invoke one typed receipt commit callback"
    );
    for required in [
        "pending.attempt != current_attempt",
        "pending.attempt <= last_committed",
        "if !commit_receipt(pending.receipt)",
    ] {
        assert!(
            commit_receipt.contains(required),
            "the render receipt gate is missing required fail-closed behavior: {required}"
        );
    }
    let receipt_apply_index = commit_receipt
        .find("if !commit_receipt(pending.receipt)")
        .expect("receipt application must remain fail-closed");
    let commit_index = commit_receipt
        .find("last_committed_frame_render_attempt = Some(pending.attempt)")
        .expect("successful receipt application must record its attempt");
    assert!(
        receipt_apply_index < commit_index,
        "the render attempt must be recorded only after atomic receipt application succeeds"
    );
    let runtime_source =
        fs::read_to_string(repo_root.join("src/adapter/inbound/tui/app/shell_runtime.rs"))
            .expect("shell runtime source should load");
    let runtime_receipt =
        top_level_impl_method_source(&runtime_source, "commit_inline_frame_render_receipt");
    assert_eq!(
        production_callable_reference_lines(&runtime_receipt, "apply_inline_frame_render_receipt")
            .len(),
        1,
        "ShellRuntime must apply the delivered owned receipt exactly once"
    );

    let terminal_syntax =
        syn::parse_file(&terminal_source).expect("inline terminal adapter source should parse");
    let terminal_draw = top_level_function(&terminal_syntax, "draw_inline_frame");
    let mut closure_references = TerminalDrawClosureReferenceVisitor::default();
    closure_references.visit_block(&terminal_draw.block);
    assert_eq!(
        closure_references.references.len(),
        1,
        "draw_inline_frame must have one Terminal::draw closure"
    );
    for forbidden in ["NativeTuiApp", "app_mut", "runtime"] {
        assert!(
            !closure_references.references[0]
                .iter()
                .any(|reference| reference.split("::").any(|segment| segment == forbidden)),
            "Terminal::draw closure must not access `{forbidden}`; it consumes only the owned frame model"
        );
    }
}

#[test]
fn tui_owned_frame_capture_keeps_app_projection_wrappers_test_only() {
    let repo_root = repo_root();
    for (path, function_name) in [
        (
            "src/adapter/inbound/tui/app/shell_presentation.rs",
            "build_queue_overlay_view",
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/overlays/popup/parallel_peek.rs",
            "build_parallel_peek_overlay_view",
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning.rs",
            "build_planning_init_overlay_view",
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_init_router.rs",
            "build_planning_init_overlay_view_for_app",
        ),
    ] {
        let source = fs::read_to_string(repo_root.join(path))
            .unwrap_or_else(|error| panic!("{path} should load: {error}"));
        let syntax =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("{path} should parse: {error}"));
        let production_definitions = syntax
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Fn(function)
                    if function.sig.ident == function_name
                        && !attributes_are_test_only(&function.attrs) =>
                {
                    Some(function.sig.ident.to_string())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            production_definitions.is_empty(),
            "{path} must not compile the app-based `{function_name}` wrapper in production"
        );
    }

    let queue_source =
        fs::read_to_string(repo_root.join("src/adapter/inbound/tui/app/queue_overlay_ui.rs"))
            .expect("queue overlay UI source should load");
    let queue_syntax =
        syn::parse_file(&queue_source).expect("queue overlay UI source should parse");
    let production_queue_screen_model_wrappers = queue_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item) if !attributes_are_test_only(&item.attrs) => Some(item),
            _ => None,
        })
        .flat_map(|item| item.items.iter())
        .filter(|item| {
            matches!(
                item,
                syn::ImplItem::Fn(function)
                    if function.sig.ident == "queue_overlay_screen_model"
                        && !attributes_are_test_only(&function.attrs)
            )
        })
        .count();
    assert_eq!(
        production_queue_screen_model_wrappers, 0,
        "the queue app-based screen-model wrapper must stay test-only"
    );

    let shell_core_source = fs::read_to_string(
        repo_root.join("src/adapter/inbound/tui/app/shell_presentation/shell_core.rs"),
    )
    .expect("conversation screen-model source should load");
    let from_app_with_sample =
        top_level_impl_method_source(&shell_core_source, "from_app_with_sample");
    assert!(
        production_callable_reference_lines(&from_app_with_sample, "queue_receipt_undo_task_count")
            .is_empty(),
        "ConversationScreenModel must not reread parallel mode through the raw queue receipt helper"
    );
    assert_eq!(
        production_callable_reference_lines(
            &from_app_with_sample,
            "queue_receipt_undo_task_count_for_parallel_mode"
        )
        .len(),
        1,
        "ConversationScreenModel must derive queue receipt copy from the sampled parallel-mode fact"
    );
    let compact_from_app = from_app_with_sample
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_from_app
            .contains("queue_receipt_undo_task_count_for_parallel_mode(parallel_mode_enabled)"),
        "the sampled queue receipt helper must receive the transaction-owned parallel-mode value"
    );
}

#[test]
fn tui_conversation_tail_reads_one_immutable_screen_model_without_effects() {
    assert_no_forbidden_references_in_paths(
        "TUI conversation tail must consume ConversationScreenModel without app, service, I/O, or clock access",
        &[
            "src/adapter/inbound/tui/app/shell_presentation/status_panels",
            "src/adapter/inbound/tui/app/shell_presentation/runtime_status_copy.rs",
            "src/adapter/inbound/tui/app/planning/presentation.rs",
            "src/adapter/inbound/tui/app/planning/status_projection.rs",
        ],
        &[
            "NativeTuiApp",
            "NativeTuiApplicationHandle",
            ".application",
            ".planning()",
            ".runtime()",
            "CoreRuntime",
            "core_runtime",
            "NativeClientRuntime",
            "client_runtime",
            "dispatch_client_event",
            "poll_pending_client_event",
            "std::fs",
            "std::thread",
            "std::sync",
            "SystemTime::now",
            "Instant::now",
        ],
    );

    let screen_model_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_presentation/shell_core.rs"),
    )
    .expect("conversation screen-model source should load");
    let production_source = production_lines(&screen_model_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    // Keep frame projection reads narrow so the rendering boundary does not
    // become coupled to unrelated Core state through a full AppSnapshot.
    assert_eq!(
        production_source
            .matches("revisioned_planning_parallel_projection()")
            .count(),
        1,
        "ConversationProjectionSample must read the revisioned planning/parallel projection exactly once"
    );
    assert_eq!(
        production_source
            .matches("client_runtime.snapshot()")
            .count(),
        0,
        "ConversationProjectionSample must not rebuild the frame from the full core snapshot"
    );
    assert_eq!(
        production_source.matches("AppSnapshot").count(),
        0,
        "TUI shell_core must not import or use AppSnapshot for frame projection"
    );
    for required in [
        "conversation_history_identity_revision: u64",
        "fn conversation_history_identity_revision(",
    ] {
        assert!(
            production_source.contains(required),
            "ConversationProjectionSample must own the terminal history identity fact: {required}"
        );
    }
    let compact_production = production_source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_production.contains(
            "conversation_history_identity_revision:app.conversation.conversation_history_identity_revision"
        ),
        "ConversationProjectionSample must capture the typed conversation history identity"
    );
    for forbidden in [
        ".application",
        ".planning()",
        ".runtime()",
        "std::fs",
        "std::thread",
        "std::sync",
    ] {
        assert!(
            !production_source.contains(forbidden),
            "ConversationScreenModel choke point must not execute services or I/O: {forbidden}"
        );
    }

    let terminal_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/inline_terminal_adapter.rs"),
    )
    .expect("inline terminal adapter source should load");
    assert!(
        terminal_source.contains("frame_projection: &InlineConversationFrameProjection"),
        "frame cache must accept the immutable frame projection instead of NativeTuiApp"
    );
    assert!(
        terminal_source.contains("capture_inline_shell_frame_model("),
        "the terminal transaction must materialize one owned frame before drawing"
    );

    let syntax =
        syn::parse_file(&terminal_source).expect("inline terminal adapter source should parse");
    let transaction = syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "sync_inline_viewport_transaction" => {
                Some(function)
            }
            _ => None,
        })
        .expect("inline terminal adapter should expose sync_inline_viewport_transaction");
    let span = transaction.span();
    let transaction_source = terminal_source
        .lines()
        .skip(span.start().line.saturating_sub(1))
        .take(
            span.end()
                .line
                .saturating_sub(span.start().line)
                .saturating_add(1),
        )
        .collect::<String>();
    let transaction_source = transaction_source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert_eq!(
        transaction_source
            .matches("runtime.capture_inline_terminal_projection_sample()")
            .count(),
        1,
        "one terminal sync transaction must capture conversation projection facts exactly once"
    );
    assert_eq!(
        transaction_source
            .matches("observe_conversation_history_identity_revision(")
            .count(),
        1,
        "one terminal sync transaction must observe the sampled conversation history identity exactly once"
    );
}

#[test]
fn tui_transcript_handoff_ack_requires_an_exact_terminal_delivery_receipt() {
    let model_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/conversation_model/view_model/messages.rs"),
    )
    .expect("conversation message mutation source should load");
    let terminal_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/inline_terminal_adapter.rs"),
    )
    .expect("inline terminal adapter source should load");
    let runtime_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/shell_runtime.rs"))
            .expect("shell runtime source should load");
    let flush_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/inline_terminal_adapter/history_flush.rs"),
    )
    .expect("history flush source should load");

    for required in [
        "viewport_transcript_handoff_correlation",
        "correlation: &super::TranscriptHandoffCorrelation",
        "self.viewport_transcript_handoff_correlation().as_ref() != Some(correlation)",
    ] {
        assert!(
            model_source.contains(required),
            "conversation model must reject an uncorrelated transcript ACK: {required}"
        );
    }
    for required in [
        "committed_handoff: Option<TranscriptHandoffDeliveryToken>",
        "fn committed_handoff(",
    ] {
        assert!(
            flush_source.contains(required),
            "history flush receipt must retain the sampled handoff token: {required}"
        );
    }
    for required in [
        "history_sync.committed_handoff()",
        "handoff_sync.committed_handoff()",
    ] {
        assert!(
            terminal_source.contains(required),
            "terminal ACK must consume only an exact committed receipt: {required}"
        );
    }
    assert!(
        runtime_source.contains("delivery_token.matches_current(&self.app)")
            && terminal_source
                .contains("runtime.acknowledge_transcript_handoff_after_delivery(delivery_token)"),
        "ShellRuntime must validate the exact delivery token before mutating transcript state"
    );
}

#[test]
fn tui_turn_steer_confirmation_draws_one_owned_screen_model() {
    let screen_model_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_presentation/shell_core.rs"),
    )
    .expect("conversation screen-model source should load");
    let screen_model_production = production_lines(&screen_model_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    let screen_model_compact = screen_model_production
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for required in [
        "structTurnSteerConfirmationScreenModel",
        "turn_steer_confirmation:Option<TurnSteerConfirmationScreenModel>",
        "request:intent.request.clone()",
    ] {
        assert!(
            screen_model_compact.contains(required),
            "turn-steer projection must capture its owned confirmation fact: {required}"
        );
    }

    let rendering_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/shell_rendering.rs"))
            .expect("shell rendering source should load");
    let draw = top_level_function_source(&rendering_source, "draw_turn_steer_confirmation");
    let draw_compact = draw
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        draw_compact.contains("confirmation:&TurnSteerConfirmationScreenModel"),
        "turn-steer renderer must receive the immutable confirmation screen model"
    );
    for forbidden in [
        "NativeTuiApp",
        "app.turn_steer_confirmation",
        "app.tui_language",
    ] {
        assert!(
            !draw.contains(forbidden),
            "turn-steer renderer must not reread live app state: {forbidden}"
        );
    }
}

#[test]
fn tui_session_overlay_is_captured_once_before_pure_draw() {
    let model_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/session_overlay_screen_model.rs"),
    )
    .expect("session overlay screen-model source should load");
    let model_production = production_lines(&model_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        model_production
            .matches("build_session_browser_page(")
            .count(),
        1,
        "one session screen-model capture must project filtering, paging, and selection exactly once"
    );
    assert_eq!(
        model_production
            .matches("current_workspace_directory()")
            .count(),
        1,
        "one session screen-model capture must sample workspace context exactly once"
    );
    for required in [
        "selected_session_id: Option<String>",
        "selected_index: Option<usize>",
    ] {
        assert!(
            model_production.contains(required),
            "session screen model must keep stable identity separate from page-local selection: {required}"
        );
    }
    for forbidden in [
        ".application",
        "CoreRuntime",
        "core_runtime",
        "NativeClientRuntime",
        "client_runtime",
        "dispatch_core_command",
        "dispatch_client_event",
        "poll_pending_client_event",
        "SessionService",
        "SessionCatalogPort",
        "load_session_catalog",
        "rename_session",
        "std::fs",
        "std::thread",
        "std::sync",
        "SystemTime::now",
        "Instant::now",
    ] {
        assert!(
            !model_production.contains(forbidden),
            "session screen-model capture must not execute services, catalog I/O, or clocks: {forbidden}"
        );
    }

    let presentation_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_presentation/session_browser.rs"),
    )
    .expect("session presentation source should load");
    let presentation_production = production_lines(&presentation_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "NativeTuiApp",
        "SessionState",
        "CoreRuntime",
        "core_runtime",
        "NativeClientRuntime",
        "client_runtime",
        "dispatch_client_event",
        "poll_pending_client_event",
        "build_session_browser_page(",
        ".session_overlay_ui_state",
        ".current_workspace_directory()",
        ".application",
        "std::fs",
        "std::thread",
        "std::sync",
        "SystemTime::now",
        "Instant::now",
    ] {
        assert!(
            !presentation_production.contains(forbidden),
            "session presentation must consume only the immutable screen model: {forbidden}"
        );
    }

    let popup_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_presentation/overlays/popup/base.rs"),
    )
    .expect("popup assembly source should load");
    let popup_builder = top_level_function_source(&popup_source, "build_session_overlay_view");
    assert!(
        popup_builder.contains("screen_model: &SessionOverlayScreenModel"),
        "session popup assembly must receive the immutable screen model"
    );
    assert!(
        !popup_builder.contains("NativeTuiApp"),
        "session popup assembly must not reread NativeTuiApp"
    );

    let frame_model_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/inline_frame_model.rs"))
            .expect("inline frame-model source should load");
    let capture =
        top_level_function_source(&frame_model_source, "capture_inline_shell_frame_model");
    let compact_capture = capture
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert_eq!(
        compact_capture
            .matches("SessionOverlayScreenModel::capture(app)")
            .count(),
        1,
        "one frame capture must materialize one immutable session screen model"
    );
    assert_eq!(
        compact_capture
            .matches("build_session_overlay_view(&screen_model)")
            .count(),
        1,
        "one frame capture must build every session section from the same screen model"
    );
    let capture_index = compact_capture
        .find("SessionOverlayScreenModel::capture(app)")
        .expect("frame capture should capture a session screen model");
    let view_index = compact_capture
        .find("build_session_overlay_view(&screen_model)")
        .expect("frame capture should build an owned session overlay view");
    let list_state_index = compact_capture
        .find("list_state.select(")
        .expect("frame capture should prepare the owned Ratatui list state");
    assert!(
        capture_index < view_index && view_index < list_state_index,
        "frame capture must finish the immutable session projection before preparing local ListState"
    );

    let rendering_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_rendering/inline_inspection.rs"),
    )
    .expect("inline inspection source should load");
    for forbidden in [
        "SessionOverlayScreenModel::capture",
        "build_session_overlay_view",
        "NativeTuiApp",
    ] {
        assert!(
            !rendering_source.contains(forbidden),
            "session renderer must consume the owned frame model without recapturing `{forbidden}`"
        );
    }
}

#[test]
fn core_revisioned_planning_parallel_projection_stays_narrow() {
    let state_source = fs::read_to_string(repo_root().join("src/core/app/controller/state.rs"))
        .expect("core app state source should load");
    let state_method =
        top_level_impl_method_source(&state_source, "revisioned_planning_parallel_projection");
    let compact_state_method = state_method
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for required in [
        "RevisionedPlanningParallelProjection{",
        "revision:self.current.revision",
        "planning_parallel:self.current.planning_parallel.clone()",
    ] {
        assert!(
            compact_state_method.contains(required),
            "AppState must construct the narrow projection directly: {required}"
        );
    }
    for forbidden in [
        "snapshot(",
        "AppSnapshot",
        "self.startup",
        "self.session_catalog",
        "self.conversation",
    ] {
        assert!(
            !compact_state_method.contains(forbidden),
            "the narrow AppState projection must not materialize unrelated state: {forbidden}"
        );
    }

    for (path, delegate) in [
        (
            "src/core/app/controller.rs",
            "self.state.revisioned_planning_parallel_projection()",
        ),
        (
            "src/core/runtime/driver.rs",
            "self.controller.revisioned_planning_parallel_projection()",
        ),
    ] {
        let source = fs::read_to_string(repo_root().join(path))
            .unwrap_or_else(|error| panic!("{path} should load: {error}"));
        let method =
            top_level_impl_method_source(&source, "revisioned_planning_parallel_projection");
        let compact_method = method
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        assert!(
            compact_method.contains(delegate),
            "{path} must delegate the narrow projection without rebuilding it"
        );
        assert!(
            !compact_method.contains("snapshot(") && !compact_method.contains("AppSnapshot"),
            "{path} narrow projection wrapper must not fall back to AppSnapshot"
        );
    }
}

#[test]
fn core_dispatch_snapshots_share_one_copy_on_write_authority() {
    let state_source = fs::read_to_string(repo_root().join("src/core/app/controller/state.rs"))
        .expect("core app state source should load");
    assert!(
        state_source.contains("current: Arc<AppSnapshot>"),
        "AppState must keep one shared AppSnapshot authority"
    );
    assert!(
        state_source.contains("Arc::make_mut(&mut self.current)"),
        "AppState mutations must copy on write so retained dispatch snapshots stay immutable"
    );
    for duplicate_authority in [
        "startup: StartupState",
        "session_catalog: SessionCatalogState",
        "conversation: ConversationState",
        "cached_snapshot",
        "snapshot_cache",
    ] {
        assert!(
            !state_source.contains(duplicate_authority),
            "AppState must not retain a second full snapshot authority: {duplicate_authority}"
        );
    }

    let controller_source = fs::read_to_string(repo_root().join("src/core/app/controller.rs"))
        .expect("core controller source should load");
    let production_controller = production_lines(&controller_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        production_controller.contains("pub snapshot: Arc<AppSnapshot>"),
        "dispatch outcomes must expose the exact shared snapshot for their transition"
    );
    assert_eq!(
        production_controller
            .matches("self.state.snapshot()")
            .count(),
        1,
        "only the explicit owned snapshot() pull may deep-clone AppState"
    );
    assert!(
        !production_controller.contains("self.snapshot()"),
        "dispatch construction and helpers must use shared_snapshot() instead of an owned snapshot"
    );

    let event_source = fs::read_to_string(repo_root().join("src/core/app/event.rs"))
        .expect("core event source should load");
    assert!(
        event_source.contains("SnapshotChanged(Arc<AppSnapshot>)"),
        "SnapshotChanged and CoreDispatchOutcome must share the same snapshot allocation"
    );

    let tui_runtime_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/app_runtime.rs"))
            .expect("TUI app runtime source should load");
    let apply_outcome =
        top_level_impl_method_source(&tui_runtime_source, "apply_core_dispatch_outcome");
    assert!(
        apply_outcome.contains("for event in outcome.events"),
        "TUI must continue applying every dispatch event in order"
    );
    for forbidden_short_circuit in ["ptr_eq", "snapshot.revision"] {
        assert!(
            !apply_outcome.contains(forbidden_short_circuit),
            "shared snapshot identity is not a dispatch revision and must not suppress events: {forbidden_short_circuit}"
        );
    }
    let app_runtime_syntax =
        syn::parse_file(&tui_runtime_source).expect("TUI app runtime source should parse");
    let apply_outcome_methods = inherent_impl_methods(
        &app_runtime_syntax,
        "NativeTuiApp",
        "apply_core_dispatch_outcome",
    );
    let [apply_outcome_method] = apply_outcome_methods.as_slice() else {
        panic!("NativeTuiApp must define one apply_core_dispatch_outcome method");
    };
    let mut snapshot_reads = NamedFieldAccessVisitor::new("snapshot");
    snapshot_reads.visit_block(&apply_outcome_method.block);
    assert_eq!(
        snapshot_reads.lines.len(),
        1,
        "dispatch application may field-read the outcome snapshot exactly once, only to install the authoritative runtime projection"
    );
    let compact_apply_outcome = rust_code_without_comments_and_literals(&apply_outcome)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let authority_install = compact_apply_outcome
        .find(
            "self.apply_conversation_runtime_projection(outcome.snapshot.conversation_runtime.clone())",
        )
        .expect("TUI must install the final Core conversation authority before presentation events");
    let event_reduction = compact_apply_outcome
        .find("foreventinoutcome.events")
        .expect("TUI must reduce all events");
    let live_authority_restore = compact_apply_outcome
        .rfind("self.runtime.client_runtime.snapshot().conversation_runtime")
        .expect("nested dispatches must restore the latest facade authority");
    assert!(
        authority_install < event_reduction && event_reduction < live_authority_restore,
        "final authority install, unconditional event reduction, and nested-dispatch restore must keep their transaction order"
    );
}

#[test]
fn production_tui_cannot_write_conversation_runtime_semantic_authority() {
    let root = repo_root();
    let mut violations = Vec::new();
    let mut whole_replacements = Vec::new();
    let mut replacement_calls = Vec::new();
    let mut snapshot_literals = Vec::new();

    for path in rust_files_under(&root.join("src/adapter/inbound/tui")) {
        if is_test_only_path(&path) {
            continue;
        }
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let audit = conversation_runtime_writer_audit(&source)
            .unwrap_or_else(|error| panic!("failed to audit {}: {error}", path.display()));
        let relative = relative_path(&root, &path);
        violations.extend(
            audit
                .semantic_writes
                .into_iter()
                .map(|write| format!("{relative}:{}:{write}", write.line)),
        );
        whole_replacements.extend(
            audit
                .whole_replacements
                .into_iter()
                .map(|write| (relative.clone(), write.owner, write.line)),
        );
        replacement_calls.extend(
            audit
                .replacement_calls
                .into_iter()
                .map(|write| (relative.clone(), write.owner, write.line)),
        );
        snapshot_literals.extend(
            audit
                .snapshot_literals
                .into_iter()
                .map(|write| format!("{relative}:{}:{write}", write.line)),
        );
    }

    assert!(
        violations.is_empty(),
        "production TUI must treat ConversationRuntimeSnapshot semantic fields as read-only:\n{}",
        violations.join("\n")
    );
    assert!(
        snapshot_literals.is_empty(),
        "production TUI must not mint ConversationRuntimeSnapshot authority:\n{}",
        snapshot_literals.join("\n")
    );
    assert_eq!(
        whole_replacements
            .iter()
            .map(|(path, owner, _)| (path.as_str(), owner.as_str()))
            .collect::<Vec<_>>(),
        [(
            "src/adapter/inbound/tui/app/conversation_model/view_model.rs",
            "apply_runtime_snapshot",
        )],
        "one opaque ConversationViewModel replacement assignment is the only TUI-owned projection writer"
    );
    assert_eq!(
        replacement_calls
            .iter()
            .map(|(path, owner, _)| (path.as_str(), owner.as_str()))
            .collect::<Vec<_>>(),
        [(
            "src/adapter/inbound/tui/app/app_runtime.rs",
            "apply_conversation_runtime_projection",
        )],
        "only NativeTuiApp's Core projection installer may replace the immutable runtime snapshot"
    );
}

#[test]
fn conversation_runtime_writer_analyzer_ignores_reads_and_test_fixtures_but_rejects_writes() {
    let harmless = r#"
fn render(snapshot: &ConversationRuntimeSnapshot) {
    let _ = (&snapshot.active_turn, &snapshot.approval, &snapshot.auto_follow.phase);
    let _example = "snapshot.active_turn = None";
}
#[cfg(test)]
fn install_fixture(mut snapshot: ConversationRuntimeSnapshot) {
    snapshot.active_turn = None;
    snapshot.auto_follow.phase = AutoFollowPhase::Idle;
    let _ = ConversationRuntimeSnapshot {
        active_turn: None,
        approval: None,
        approval_review: None,
        auto_follow: AutoFollowAuthoritySnapshot::default(),
        post_turn: PostTurnAuthoritySnapshot::Idle,
        planning_handoff: None,
    };
}
"#;
    let harmless_audit =
        conversation_runtime_writer_audit(harmless).expect("harmless fixture should parse");
    assert!(
        harmless_audit.semantic_writes.is_empty()
            && harmless_audit.whole_replacements.is_empty()
            && harmless_audit.replacement_calls.is_empty()
            && harmless_audit.snapshot_literals.is_empty(),
        "reads, strings, and test-only writers must not create false positives"
    );

    let direct_write = conversation_runtime_writer_audit(
        "fn escape(mut snapshot: ConversationRuntimeSnapshot) { snapshot.active_turn = None; }",
    )
    .expect("direct-write fixture should parse");
    assert_eq!(direct_write.semantic_writes.len(), 1);

    let nested_write = conversation_runtime_writer_audit(
        "fn escape(mut snapshot: ConversationRuntimeSnapshot) { snapshot.auto_follow.phase = AutoFollowPhase::Idle; }",
    )
    .expect("nested-write fixture should parse");
    assert_eq!(nested_write.semantic_writes.len(), 1);

    let mutable_borrow = conversation_runtime_writer_audit(
        "fn escape(snapshot: &mut ConversationRuntimeSnapshot) { let _ = &mut snapshot.approval; }",
    )
    .expect("mutable-borrow fixture should parse");
    assert_eq!(mutable_borrow.semantic_writes.len(), 1);

    let replacement = conversation_runtime_writer_audit(
        "fn escape(view: &mut View, snapshot: ConversationRuntimeSnapshot) { view.runtime_snapshot = snapshot; view.apply_runtime_snapshot(snapshot); }",
    )
    .expect("replacement fixture should parse");
    assert_eq!(replacement.whole_replacements.len(), 1);
    assert_eq!(replacement.replacement_calls.len(), 1);

    let literal = conversation_runtime_writer_audit(
        "fn escape() { let _ = ConversationRuntimeSnapshot { active_turn: None }; }",
    )
    .expect("snapshot literal fixture should parse");
    assert_eq!(literal.snapshot_literals.len(), 1);
}

#[test]
fn tui_parallel_frame_uses_one_control_plane_and_event_projection_sample() {
    assert_no_forbidden_references_in_paths(
        "TUI parallel presentation and rendering must not reread mutable control-plane state",
        &[
            "src/adapter/inbound/tui/app/shell_presentation",
            "src/adapter/inbound/tui/app/shell_rendering.rs",
            "src/adapter/inbound/tui/app/shell_rendering",
            "src/adapter/inbound/tui/app/inline_terminal_adapter.rs",
        ],
        &[
            ".mode_enabled()",
            ".control_effect_in_flight()",
            "build_supersession_overlay_view(app",
        ],
    );
    assert_no_forbidden_references_in_paths(
        "Supersession presentation must consume the sampled animation clock",
        &[
            "src/adapter/inbound/tui/app/shell_presentation/overlays",
            "src/adapter/inbound/tui/app/shell_rendering.rs",
            "src/adapter/inbound/tui/app/shell_rendering",
        ],
        &["SystemTime::now"],
    );

    let host_source = fs::read_to_string(
        repo_root().join("src/application/service/parallel_mode/control_plane/host.rs"),
    )
    .expect("parallel control-plane host source should load");
    for required in [
        "pub struct ParallelModeControlPlanePresentationProjection",
        "pub fn presentation_projection(&self)",
        "let service = self.service();",
        "supervisor_inspection_state: service.supervisor_inspection_state().clone()",
        "last_dispatch_withheld_reason: service",
    ] {
        assert!(
            host_source.contains(required),
            "one host mutex acquisition must capture every parallel presentation fact: {required}"
        );
    }

    let sample_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_presentation/shell_core.rs"),
    )
    .expect("conversation projection sample source should load");
    for required in [
        "pub(in crate::adapter::inbound::tui::app) struct ParallelPanelProjectionSample",
        "parallel_control_plane: ParallelModeControlPlanePresentationProjection",
        "RevisionedPlanningParallelProjection {",
        "PlanningParallelProjection {",
        "parallel_panel: ParallelPanelProjectionSample",
        "parallel_panel: ParallelPanelProjectionSample::from_parts(",
        "let parallel_control_plane = app",
        ".parallel_control_plane_projection()",
        "parallel_supervisor_events: app.shell.parallel_supervisor_event_log.projection()",
    ] {
        assert!(
            sample_source.contains(required),
            "ConversationProjectionSample must own the parallel frame fact: {required}"
        );
    }
    let conversation_capture_start = sample_source
        .find("impl ConversationProjectionSample")
        .expect("conversation projection sample implementation should exist");
    let conversation_capture_end = sample_source[conversation_capture_start..]
        .find("pub(in crate::adapter::inbound::tui::app) fn parallel_mode_enabled")
        .map(|offset| conversation_capture_start + offset)
        .expect("conversation projection capture boundary should exist");
    let conversation_capture = &sample_source[conversation_capture_start..conversation_capture_end];
    assert_eq!(
        conversation_capture
            .matches("revisioned_planning_parallel_projection()")
            .count(),
        1,
        "conversation frame must capture one revisioned planning/parallel projection"
    );
    assert!(
        !conversation_capture.contains("parallel_mode_projection()"),
        "conversation frame must move parallel state out of the revisioned projection instead of rereading Core"
    );

    let terminal_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/inline_terminal_adapter.rs"),
    )
    .expect("inline terminal adapter source should load");
    for required in [
        "InlineTerminalSyncProjection",
        "InlineTerminalSyncPolicy::from_sample(&projection_sample)",
        "sampled_parallel_frame_projection",
    ] {
        assert!(
            terminal_source.contains(required),
            "parallel policy, history, and frame drawing must share one sample: {required}"
        );
    }

    let frame_model_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/inline_frame_model.rs"))
            .expect("owned frame model source should load");
    for required in [
        "supersession_overlay_view: Option<Box<SupersessionOverlayView>>",
        "parallel frame projection must own the supervisor view",
        "sample.parallel_supervisor_event_scrollback_lines_before_live_tail(",
    ] {
        assert!(
            frame_model_source.contains(required),
            "owned frame capture must materialize one supersession view: {required}"
        );
    }
    let rendering_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/shell_rendering.rs"))
            .expect("shell rendering source should load");
    assert!(
        rendering_source
            .contains("inline_inspection::parallel_event_stream_visible_rows(view, layout[0])"),
        "Supersession row planning and drawing must consume the owned view"
    );

    let runtime_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/shell_runtime.rs"))
            .expect("shell runtime source should load");
    for required in [
        "let parallel_presentation_sample = ParallelPanelProjectionSample::capture(&self.app);",
        ".live_activity_pulse_with_sample(now, &parallel_presentation_sample)",
        ".tick_parallel_mode_control_plane(now, &parallel_presentation_sample)",
    ] {
        assert!(
            runtime_source.contains(required),
            "pulse and tick must agree on one sampled parallel panel state: {required}"
        );
    }
    let scheduler_source =
        top_level_impl_method_source(&runtime_source, "poll_background_messages_at");
    assert!(
        !scheduler_source.contains("ConversationProjectionSample::capture")
            && !scheduler_source.contains("capture_inline_terminal_projection_sample"),
        "the 100ms scheduler must not clone the full conversation and event-stream sample"
    );
    for path in [
        "src/adapter/inbound/tui/app/parallel_mode.rs",
        "src/adapter/inbound/tui/app/auto_follow/controller.rs",
    ] {
        let source = fs::read_to_string(repo_root().join(path))
            .unwrap_or_else(|error| panic!("{path} should load: {error}"));
        assert!(
            !source.contains("ConversationProjectionSample::capture"),
            "prompt and pulse checks must use the lightweight parallel panel sample: {path}"
        );
    }

    let event_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/parallel_supervisor_events.rs"),
    )
    .expect("parallel event source should load");
    let inspection_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_rendering/inline_inspection.rs"),
    )
    .expect("inline inspection source should load");
    for required in [
        "rendered_parallel_event_line_rows",
        "rendered_parallel_event_tail_start_index",
    ] {
        for source in [&event_source, &inspection_source] {
            assert!(
                source.contains(required),
                "scrollback splitting and live rendering must share Ratatui word-wrap boundary helper: {required}"
            );
        }
    }
}

#[test]
fn tui_tail_compaction_uses_typed_priority_without_parsing_localized_copy() {
    let source = fs::read_to_string(repo_root().join(
        "src/adapter/inbound/tui/app/shell_presentation/status_panels/live_status_layout.rs",
    ))
    .expect("live-status layout source should load");
    let syntax = syn::parse_file(&source).expect("live-status layout source should parse");
    let function = syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "compact_inspection_tail_lines" => {
                Some(function)
            }
            _ => None,
        })
        .expect("live-status layout should expose compact_inspection_tail_lines");
    let span = function.span();
    let compaction = source
        .lines()
        .skip(span.start().line.saturating_sub(1))
        .take(
            span.end()
                .line
                .saturating_sub(span.start().line)
                .saturating_add(1),
        )
        .collect::<Vec<_>>()
        .join("\n");

    for required in ["Vec<InlineTailLine>", ".priority"] {
        assert!(
            compaction.contains(required),
            "tail compaction must consume typed semantic priority: {required}"
        );
    }
    let production = production_lines(&source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !production.contains("fn compact_tail_priority("),
        "live-status layout must not restore rendered-copy priority parsing"
    );
    for forbidden in [
        ".to_string()",
        ".starts_with(",
        ".strip_prefix(",
        ".contains(",
        ".eq_ignore_ascii_case(",
    ] {
        assert!(
            !compaction.contains(forbidden),
            "tail compaction must not infer priority from rendered/localized copy: {forbidden}"
        );
    }
}

#[test]
fn conversation_state_model_has_no_presentation_or_planning_projection_cache() {
    assert_no_forbidden_references_in_paths(
        "conversation semantic state must not own shell presentation or planning runtime projections",
        &[
            "src/adapter/inbound/tui/app/conversation_model.rs",
            "src/adapter/inbound/tui/app/conversation_model",
        ],
        &[
            "ratatui",
            "shell_presentation",
            "cached_conversation_lines",
            "refresh_conversation_lines",
            "PlanningRuntimeProjection",
            "RuntimeProjection",
            "planning_runtime_projection",
            "projection_cache",
            "reducer_event_projection_cache",
        ],
    );
}

#[test]
fn conversation_input_reducer_is_isolated_to_composer_state() {
    let composer_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/conversation_model/composer_state.rs"),
    )
    .expect("conversation composer state source should load");
    let composer_syntax =
        syn::parse_file(&composer_source).expect("conversation composer state should parse");
    let composer_fields = named_struct_fields(&composer_syntax, "ConversationComposerState");
    let composer_field_types = composer_fields
        .iter()
        .map(|field| {
            (
                field
                    .ident
                    .as_ref()
                    .expect("composer field should be named")
                    .to_string(),
                &field.ty,
            )
        })
        .collect::<HashMap<_, _>>();
    let expected_composer_field_names = [
        "input_buffer",
        "input_cursor_byte_index",
        "inline_shell_command_palette_state",
        "startup_submit_armed",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<HashSet<_>>();
    assert_eq!(
        composer_field_types.keys().cloned().collect::<HashSet<_>>(),
        expected_composer_field_names,
        "ConversationComposerState must own exactly the mutable composer surface"
    );
    assert!(
        is_named_path_type(composer_field_types["input_buffer"], "String"),
        "ConversationComposerState.input_buffer must be String"
    );
    assert!(
        is_single_generic_named_type(
            composer_field_types["input_cursor_byte_index"],
            "Option",
            "usize",
        ),
        "ConversationComposerState.input_cursor_byte_index must be Option<usize>"
    );
    assert!(
        is_named_path_type(
            composer_field_types["inline_shell_command_palette_state"],
            "InlineShellCommandPaletteState",
        ),
        "ConversationComposerState.inline_shell_command_palette_state must use the palette state"
    );
    assert!(
        is_named_path_type(composer_field_types["startup_submit_armed"], "bool"),
        "ConversationComposerState.startup_submit_armed must be bool"
    );

    let view_model_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/conversation_model/view_model.rs"),
    )
    .expect("conversation view-model source should load");
    let view_model_syntax =
        syn::parse_file(&view_model_source).expect("conversation view model should parse");
    let view_model_fields = named_struct_fields(&view_model_syntax, "ConversationViewModel");
    let composer_field = view_model_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "composer")
        })
        .expect("ConversationViewModel must own one composer field");
    assert!(
        matches!(
            &composer_field.ty,
            syn::Type::Path(type_path)
                if type_path.path.segments.last().is_some_and(|segment| {
                    segment.ident == "ConversationComposerState"
                })
        ),
        "ConversationViewModel.composer must use ConversationComposerState"
    );
    for flat_field_name in &expected_composer_field_names {
        assert!(
            view_model_fields.iter().all(|field| {
                field
                    .ident
                    .as_ref()
                    .is_none_or(|ident| ident != flat_field_name)
            }),
            "ConversationViewModel must not retain flat composer field {flat_field_name}"
        );
    }

    let input_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/conversation_input.rs"))
            .expect("conversation input source should load");
    let input_syntax =
        syn::parse_file(&input_source).expect("conversation input source should parse");
    let production_references = rust_semantic_references(&input_source).paths;
    let references_identifier = |references: &[String], identifier: &str| {
        references
            .iter()
            .any(|path| path.split("::").any(|segment| segment == identifier))
    };
    for forbidden_reference in [
        "ConversationViewModel",
        "ConversationMessage",
        "record_manual_preparation_failure",
        "record_status_message",
    ] {
        assert!(
            !references_identifier(&production_references, forbidden_reference),
            "production conversation_input.rs must not reference {forbidden_reference}"
        );
    }
    let reduce_source = top_level_function_source(&input_source, "reduce_conversation_input");
    let reduce_syntax =
        syn::parse_file(&reduce_source).expect("conversation input reducer should parse");
    let reduce_function = reduce_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "reduce_conversation_input" => {
                Some(function)
            }
            _ => None,
        })
        .expect("conversation input reducer should exist");
    let reducer_parameter_types = reduce_function
        .sig
        .inputs
        .iter()
        .map(|argument| match argument {
            syn::FnArg::Typed(argument) => argument.ty.as_ref(),
            syn::FnArg::Receiver(_) => panic!("conversation input reducer must be a free function"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reducer_parameter_types.len(),
        2,
        "conversation input reducer must accept only composer state and one composer event"
    );
    assert!(
        is_named_path_type(reducer_parameter_types[0], "ConversationComposerState"),
        "conversation input reducer first parameter must be ConversationComposerState"
    );
    assert!(
        is_named_path_type(reducer_parameter_types[1], "ConversationComposerEvent"),
        "conversation input reducer second parameter must be ConversationComposerEvent"
    );
    let syn::ReturnType::Type(_, reducer_return_type) = &reduce_function.sig.output else {
        panic!("conversation input reducer must return ConversationComposerReduction");
    };
    assert!(
        is_named_path_type(reducer_return_type, "ConversationComposerReduction"),
        "conversation input reducer must return ConversationComposerReduction"
    );

    let reduction_fields = named_struct_fields(&input_syntax, "ConversationComposerReduction");
    let reduction_field_types = reduction_fields
        .iter()
        .map(|field| {
            (
                field
                    .ident
                    .as_ref()
                    .expect("composer reduction field should be named")
                    .to_string(),
                &field.ty,
            )
        })
        .collect::<HashMap<_, _>>();
    assert_eq!(
        reduction_field_types
            .keys()
            .cloned()
            .collect::<HashSet<_>>(),
        ["state", "effects"]
            .into_iter()
            .map(str::to_string)
            .collect::<HashSet<_>>(),
        "ConversationComposerReduction must expose only state and effects"
    );
    assert!(
        is_named_path_type(reduction_field_types["state"], "ConversationComposerState"),
        "composer reduction state must remain ConversationComposerState"
    );
    assert!(
        is_single_generic_named_type(
            reduction_field_types["effects"],
            "Vec",
            "ConversationComposerEffect",
        ),
        "composer reduction effects must remain Vec<ConversationComposerEffect>"
    );

    let effect = input_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == "ConversationComposerEffect" => Some(item),
            _ => None,
        })
        .expect("conversation input source should define ConversationComposerEffect");
    assert_eq!(
        effect.variants.len(),
        1,
        "composer effects must not gain semantic conversation mutations"
    );
    let replace_status = &effect.variants[0];
    assert_eq!(replace_status.ident, "ReplaceStatus");
    let syn::Fields::Named(replace_status_fields) = &replace_status.fields else {
        panic!("ReplaceStatus must use one named status_text field");
    };
    assert_eq!(replace_status_fields.named.len(), 1);
    let status_text_field = &replace_status_fields.named[0];
    assert!(
        status_text_field
            .ident
            .as_ref()
            .is_some_and(|ident| ident == "status_text")
            && is_named_path_type(&status_text_field.ty, "String"),
        "ReplaceStatus must carry only status_text: String"
    );

    let reduce_references = rust_semantic_references(&reduce_source).paths;
    for forbidden_reference in ["ConversationInputEvent", "ConversationViewModel"] {
        assert!(
            !references_identifier(&reduce_references, forbidden_reference),
            "reduce_conversation_input must not reference {forbidden_reference}"
        );
    }
}

#[test]
fn conversation_prompt_projection_uses_one_narrow_composer_screen_model() {
    let shell_core_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_presentation/shell_core.rs"),
    )
    .expect("shell presentation core source should load");
    let shell_core_syntax =
        syn::parse_file(&shell_core_source).expect("shell presentation core should parse");
    let composer_fields =
        named_struct_fields(&shell_core_syntax, "ConversationComposerScreenModel");
    let composer_field_types = composer_fields
        .iter()
        .map(|field| {
            (
                field
                    .ident
                    .as_ref()
                    .expect("composer screen-model field should be named")
                    .to_string(),
                &field.ty,
            )
        })
        .collect::<HashMap<_, _>>();
    assert_eq!(
        composer_field_types.keys().cloned().collect::<HashSet<_>>(),
        [
            "state",
            "input_state",
            "post_turn_settlement_in_flight",
            "auto_follow_has_live_activity",
            "viewport_transcript_handoff_pending",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<HashSet<_>>(),
        "composer screen model must stay limited to prompt presentation facts"
    );
    assert!(
        is_shared_reference_to_named_type_with_lifetime(
            composer_field_types["state"],
            "ConversationComposerState",
            "a",
        ),
        "composer screen model must borrow ConversationComposerState without cloning it"
    );
    assert!(
        is_named_path_type(
            composer_field_types["input_state"],
            "ConversationInputState"
        ),
        "composer screen model must retain the typed submit state"
    );
    for boolean_field in [
        "post_turn_settlement_in_flight",
        "auto_follow_has_live_activity",
        "viewport_transcript_handoff_pending",
    ] {
        assert!(
            is_named_path_type(composer_field_types[boolean_field], "bool"),
            "composer screen-model fact {boolean_field} must stay boolean"
        );
    }

    let screen_model_fields = named_struct_fields(&shell_core_syntax, "ConversationScreenModel");
    let composer_projection = screen_model_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "composer")
        })
        .expect("ConversationScreenModel must retain one composer projection");
    assert!(
        is_option_of_single_lifetime_named_type(
            &composer_projection.ty,
            "ConversationComposerScreenModel",
            "a",
        ),
        "ConversationScreenModel.composer must be Option<ConversationComposerScreenModel>"
    );

    let references_identifier = |references: &[String], identifier: &str| {
        references
            .iter()
            .any(|path| path.split("::").any(|segment| segment == identifier))
    };
    let prompt_composer_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_presentation/prompt_composer.rs"),
    )
    .expect("prompt composer source should load");
    let prompt_composer_references = rust_semantic_references(&prompt_composer_source).paths;
    assert!(
        references_identifier(
            &prompt_composer_references,
            "ConversationComposerScreenModel"
        ),
        "prompt composer must consume ConversationComposerScreenModel"
    );
    for forbidden_reference in ["ConversationViewModel", "ConversationComposerState"] {
        assert!(
            !references_identifier(&prompt_composer_references, forbidden_reference),
            "production prompt_composer.rs must not consume {forbidden_reference} directly"
        );
    }

    let prompt_composer_syntax =
        syn::parse_file(&prompt_composer_source).expect("prompt composer source should parse");
    for function_name in [
        "build_shell_command_palette_lines",
        "build_prompt_cursor_offset",
        "locate_prompt_cursor_with_word_wrap",
        "build_prompt_buffer_view",
    ] {
        let function = top_level_function(&prompt_composer_syntax, function_name);
        let Some(syn::FnArg::Typed(first_argument)) = function.sig.inputs.first() else {
            panic!("{function_name} must accept a composer screen model first");
        };
        assert!(
            is_shared_reference_to_single_lifetime_named_type(
                &first_argument.ty,
                "ConversationComposerScreenModel",
                "_",
            ),
            "{function_name} must accept &ConversationComposerScreenModel<'_> first"
        );
    }

    let consumer_contracts = [
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs",
            "build_inline_ready_prompt_lines",
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels/live_status_layout.rs",
            "build_inline_prompt_cursor_offset_for_lines",
        ),
    ];
    for (path, function_name) in consumer_contracts {
        let source = fs::read_to_string(repo_root().join(path))
            .unwrap_or_else(|error| panic!("{path} should load: {error}"));
        let syntax =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("{path} should parse: {error}"));
        let function = top_level_function(&syntax, function_name);
        if function_name == "build_inline_ready_prompt_lines" {
            let Some(syn::FnArg::Typed(first_argument)) = function.sig.inputs.first() else {
                panic!("{function_name} must accept a composer screen model first");
            };
            assert!(
                is_shared_reference_to_single_lifetime_named_type(
                    &first_argument.ty,
                    "ConversationComposerScreenModel",
                    "_",
                ),
                "{function_name} must accept &ConversationComposerScreenModel<'_> first"
            );
        }
        let function_source = top_level_function_source(&source, function_name);
        let references = rust_semantic_references(&function_source).paths;
        for forbidden_reference in [
            "ConversationViewModel",
            "ConversationComposerState",
            "ShellConversationState",
        ] {
            assert!(
                !references_identifier(&references, forbidden_reference),
                "{function_name} must not consume {forbidden_reference}"
            );
        }
    }

    for (path, function_name, called_function, argument_count) in [
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs",
            "build_inline_tail_prompt_lines_with_context",
            "build_inline_ready_prompt_lines",
            3,
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs",
            "build_inline_ready_prompt_lines",
            "build_prompt_buffer_view",
            1,
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs",
            "build_inline_ready_prompt_lines",
            "build_shell_command_palette_lines",
            2,
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels/live_status_layout.rs",
            "build_inline_prompt_cursor_offset_for_lines",
            "build_prompt_cursor_offset",
            2,
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/prompt_composer.rs",
            "build_prompt_cursor_offset",
            "locate_prompt_cursor_with_word_wrap",
            2,
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/prompt_composer.rs",
            "locate_prompt_cursor_with_word_wrap",
            "build_prompt_buffer_view",
            1,
        ),
    ] {
        let source = fs::read_to_string(repo_root().join(path))
            .unwrap_or_else(|error| panic!("{path} should load: {error}"));
        let syntax =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("{path} should parse: {error}"));
        let function = top_level_function(&syntax, function_name);
        assert_direct_function_call(function, called_function, argument_count);
    }

    for path in [
        "src/adapter/inbound/tui/app/shell_presentation/prompt_composer.rs",
        "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs",
        "src/adapter/inbound/tui/app/shell_presentation/status_panels/live_status_layout.rs",
    ] {
        let source = fs::read_to_string(repo_root().join(path))
            .unwrap_or_else(|error| panic!("{path} should load: {error}"));
        let production_references = rust_semantic_references(&source).paths;
        assert!(
            !references_identifier(&production_references, "from_conversation"),
            "{path} must consume the prebuilt composer projection instead of rebuilding it"
        );
        let syntax =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("{path} should parse: {error}"));
        let mut composer_fields = ComposerFieldAccessVisitor::default();
        composer_fields.visit_file(&syntax);
        assert!(
            composer_fields.lines.is_empty(),
            "{path} must not bypass ConversationScreenModel with direct .composer field reads; invalid lines: {:?}",
            composer_fields.lines
        );
    }
}

#[test]
fn conversation_runtime_status_projection_uses_one_narrow_screen_model() {
    let shell_core_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_presentation/shell_core.rs"),
    )
    .expect("shell presentation core source should load");
    let shell_core_syntax =
        syn::parse_file(&shell_core_source).expect("shell presentation core should parse");
    let runtime_status_fields =
        named_struct_fields(&shell_core_syntax, "ConversationRuntimeStatusScreenModel");
    assert_eq!(
        runtime_status_fields
            .iter()
            .map(|field| {
                field
                    .ident
                    .as_ref()
                    .expect("runtime status field should be named")
                    .to_string()
            })
            .collect::<HashSet<_>>(),
        [
            "working_started_at",
            "post_turn_settlement_in_flight",
            "auto_follow_phase",
            "auto_follow_max_turns_label",
            "input_state",
            "live_agent_message_present",
            "interrupt_support_label",
            "wait_status",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<HashSet<_>>(),
        "runtime status screen model must stay limited to working-line presentation facts"
    );

    let shell_core_production = production_lines(&shell_core_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        shell_core_production
            .contains("runtime_status:Option<ConversationRuntimeStatusScreenModel>"),
        "ConversationScreenModel must retain one ready-only runtime status projection"
    );

    let runtime_copy_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/shell_presentation/runtime_status_copy.rs"),
    )
    .expect("runtime status copy source should load");
    let production_copy = production_lines(&runtime_copy_source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(production_copy.contains("ConversationRuntimeStatusScreenModel"));
    assert!(
        !production_copy.contains("ConversationViewModel"),
        "runtime status copy must consume the narrow projection instead of the conversation view model"
    );

    let tail_source = fs::read_to_string(
        repo_root()
            .join("src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs"),
    )
    .expect("inline tail copy source should load");
    let ready_tail =
        top_level_function_source(&tail_source, "build_inline_tail_content_with_context")
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
    assert!(
        ready_tail.contains("runtime_status()")
            && ready_tail.contains("build_working_line(runtime_status,"),
        "inline tail must pass the prebuilt runtime status projection to working-line copy"
    );
}

#[test]
fn conversation_live_transcript_projection_uses_one_narrow_screen_model() {
    const SHELL_CORE: &str = "src/adapter/inbound/tui/app/shell_presentation/shell_core.rs";
    const LIVE_CONSUMERS: &[&str] = &[
        "src/adapter/inbound/tui/app/shell_presentation.rs",
        "src/adapter/inbound/tui/app/shell_presentation/status_panels.rs",
        "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_shared.rs",
        "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs",
    ];

    let shell_core_source =
        fs::read_to_string(repo_root().join(SHELL_CORE)).expect("shell core source should load");
    let shell_core_syntax =
        syn::parse_file(&shell_core_source).expect("shell core source should parse");
    named_struct_fields(&shell_core_syntax, "ConversationLiveTranscriptScreenModel");
    let shell_core_compact = shell_core_source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        shell_core_compact.contains("recent_tail_messages:[Option<&'aConversationMessage>;2],"),
        "live transcript projection must retain only two recent tail message references"
    );

    let screen_model_fields = named_struct_fields(&shell_core_syntax, "ConversationScreenModel");
    let live_projection = screen_model_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "live_transcript")
        })
        .expect("ConversationScreenModel must retain one live transcript projection");
    assert!(
        is_option_of_single_lifetime_named_type(
            &live_projection.ty,
            "ConversationLiveTranscriptScreenModel",
            "a",
        ),
        "ConversationScreenModel.live_transcript must retain the narrow projection"
    );

    for (path, function_name) in [
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels.rs",
            "current_live_agent_lines",
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_shared.rs",
            "current_live_agent_lines",
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs",
            "build_recent_transcript_summary_lines",
        ),
    ] {
        let source = fs::read_to_string(repo_root().join(path))
            .unwrap_or_else(|error| panic!("{path} should load: {error}"));
        let syntax =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("{path} should parse: {error}"));
        let function = top_level_function(&syntax, function_name);
        assert_eq!(
            function.sig.inputs.len(),
            1,
            "{path}::{function_name} must accept only the live transcript projection"
        );
        let Some(syn::FnArg::Typed(first_argument)) = function.sig.inputs.first() else {
            panic!("{path}::{function_name} must accept a projection first");
        };
        assert!(
            is_shared_reference_to_single_lifetime_named_type(
                &first_argument.ty,
                "ConversationLiveTranscriptScreenModel",
                "_",
            ),
            "{path}::{function_name} must accept only the narrow projection"
        );
    }

    for callable in [
        "viewport_transcript_handoff_messages",
        "viewport_transcript_handoff_release_messages",
        "has_pending_viewport_transcript_handoff",
    ] {
        assert_no_production_callable_reference_named_in_paths(
            "live transcript consumers must use the prebuilt screen projection",
            LIVE_CONSUMERS,
            callable,
        );
    }

    let references_identifier = |references: &[String], identifier: &str| {
        references
            .iter()
            .any(|path| path.split("::").any(|segment| segment == identifier))
    };
    for (path, function_name) in [
        (
            "src/adapter/inbound/tui/app/shell_presentation.rs",
            "build_inline_live_transcript_lines",
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels.rs",
            "current_live_agent_lines",
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_shared.rs",
            "current_live_agent_lines",
        ),
        (
            "src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs",
            "build_recent_transcript_summary_lines",
        ),
    ] {
        let source = fs::read_to_string(repo_root().join(path))
            .unwrap_or_else(|error| panic!("{path} should load: {error}"));
        let function_source = top_level_function_source(&source, function_name);
        let references = rust_semantic_references(&function_source).paths;
        for forbidden in ["ConversationViewModel", "ShellConversationState"] {
            assert!(
                !references_identifier(&references, forbidden),
                "{path}::{function_name} must not consume {forbidden}"
            );
        }
    }

    for (function_source, boundary) in [
        (
            top_level_function_source(
                &fs::read_to_string(repo_root().join(LIVE_CONSUMERS[0]))
                    .expect("shell presentation source should load"),
                "build_inline_live_transcript_lines",
            ),
            "live transcript copy",
        ),
        (
            top_level_impl_method_source(&shell_core_source, "renders_viewport_transcript_handoff"),
            "viewport handoff ACK",
        ),
    ] {
        let compact = function_source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        assert!(
            compact.contains("live_transcript()"),
            "{boundary} must consume the narrow live transcript projection"
        );
        for forbidden in [".conversation_state", "ready_conversation("] {
            assert!(
                !compact.contains(forbidden),
                "{boundary} must not bypass the live transcript projection: {forbidden}"
            );
        }
    }
}

#[test]
fn tui_planning_worker_state_uses_the_domain_contract_without_round_trip_mappers() {
    assert_no_forbidden_references_in_paths(
        "TUI planning worker diagnostics must store the domain snapshot directly",
        &["src/adapter/inbound/tui"],
        &[
            "enum PlanningWorkerStatus",
            "struct PlanningWorkerPanelState",
            "fn application_planning_worker_panel_state(",
            "fn application_planning_worker_status(",
            "fn tui_planning_worker_panel_state(",
            "fn tui_planning_worker_status(",
        ],
    );
}

#[test]
fn auto_follow_overlay_keeps_only_an_active_turn_budget_draft() {
    assert_no_forbidden_references_in_paths(
        "closed auto-follow presentation must read the canonical conversation policy without mirroring it",
        &[
            "src/adapter/inbound/tui/app/auto_follow_overlay_ui.rs",
            "src/adapter/inbound/tui/app/app_runtime.rs",
            "src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_copy.rs",
            "src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_simple_review_inputs.rs",
        ],
        &[
            "ContentReset",
            "MaxAutoTurnsValueSynced",
            "MaxAutoTurnsEditCommitted",
            "MaxAutoTurnsEditCanceled",
            "max_auto_turns_editor.",
            "max_auto_turns_editor:",
            "is_turn_budget_editing",
            "turn_budget_buffer",
        ],
    );

    let overlay_source = fs::read_to_string(
        repo_root().join("src/adapter/inbound/tui/app/auto_follow_overlay_ui.rs"),
    )
    .expect("auto-follow overlay source should load");
    let overlay_syntax =
        syn::parse_file(&overlay_source).expect("auto-follow overlay source should parse");
    let overlay_fields = named_struct_fields(&overlay_syntax, "AutoFollowOverlayUiState");
    assert!(
        overlay_fields
            .iter()
            .map(|field| field
                .ident
                .as_ref()
                .expect("field should be named")
                .to_string())
            .eq(["max_auto_turns_edit_buffer".to_string()]),
        "auto-follow overlay must own exactly one editor-local field"
    );
    assert!(
        is_option_string_type(&overlay_fields[0].ty),
        "auto-follow editor ownership and its raw draft must be represented by one Option<String>"
    );

    let copy_source = fs::read_to_string(
        repo_root()
            .join("src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_copy.rs"),
    )
    .expect("planning copy source should load");
    let copy_syntax = syn::parse_file(&copy_source).expect("planning copy source should parse");
    let copy_fields = named_struct_fields(&copy_syntax, "PlanningSimpleReviewCopy");
    let copy_field_names = copy_fields
        .iter()
        .map(|field| {
            field
                .ident
                .as_ref()
                .expect("field should be named")
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        copy_field_names,
        [
            "draft_name",
            "staged_file_count",
            "validation_ok",
            "first_error",
            "max_auto_turns_label",
            "turn_budget_edit_buffer",
        ],
        "simple-review copy must not add a second editing flag or budget mirror"
    );
    let edit_buffer = copy_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "turn_budget_edit_buffer")
        })
        .expect("simple-review copy should expose its active edit draft");
    assert!(
        is_option_string_type(&edit_buffer.ty),
        "simple-review presentation must project editor ownership and its raw draft atomically"
    );
}

#[test]
fn tui_conversation_loads_enter_through_core_runtime() {
    // Static guard for the conversation lifecycle migration: TUI may keep presentation
    // state and reducers, but snapshot loading must enter CoreRuntime/CoreEffectRunner.
    assert_no_forbidden_references_in_paths(
        "TUI conversation loads must be dispatched through core runtime, not ConversationService directly",
        &[
            "src/adapter/inbound/tui/app/app_runtime.rs",
            "src/adapter/inbound/tui/app/parallel_peek.rs",
        ],
        &[".load_snapshot(", ".load_conversation_snapshot("],
    );
}

#[test]
fn tui_parallel_peek_load_correlation_is_core_owned() {
    assert_no_forbidden_references_in_paths(
        "TUI parallel peek may project accepted loads but must not own request correlation",
        &[
            "src/adapter/inbound/tui/app/parallel_peek.rs",
            "src/adapter/inbound/tui/app/parallel_peek_overlay_ui.rs",
        ],
        &[
            "PendingParallelPeekConversationLoad",
            "next_load_request_id",
            "pending_load",
            ".begin_conversation_load(",
        ],
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
fn tui_approval_review_persistence_enters_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI approval review updates must not persist review authority on the UI thread",
        &["src/adapter/inbound/tui"],
        &[
            "PersistApprovalReview",
            ".persist_review_center_approval_review_for_workspace(",
        ],
    );
    let controller_source = fs::read_to_string(repo_root().join("src/core/app/controller.rs"))
        .expect("core controller source should be readable");
    assert!(
        controller_source.contains("CoreEffect::PersistApprovalReview { correlation }"),
        "approval review persistence must positively enter through the typed Core effect"
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
fn tui_stop_requests_enter_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI stop requests must not own provider execution, admission, or TurnStarted resend state",
        &["src/adapter/inbound/tui"],
        &[
            ".request_stop_all_sessions(",
            "interrupt_request_pending",
            "mark_interrupt_requested_once",
            "clear_interrupt_request",
            "ResendPendingInterrupt",
        ],
    );
    let controller_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/shell_controller.rs"))
            .expect("shell controller source should be readable");
    assert!(
        controller_source.contains(
            "self.dispatch_client_event(CoreInput::Command(AppCommand::RequestStopAllSessions));"
        ),
        "TUI stop intent must positively enter through AppCommand::RequestStopAllSessions"
    );
}

#[test]
fn tui_active_turn_steering_enters_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI active-turn steering must not own a service worker or completion mailbox",
        &[
            "src/adapter/inbound/tui/app/app_runtime.rs",
            "src/adapter/inbound/tui/app/shell_controller.rs",
            "src/adapter/inbound/tui/app/shell_runtime.rs",
        ],
        &[
            ".steer_turn(",
            "BackgroundMessage::TurnSteerCompleted",
            "TurnSteerCompleted {\n        request_id:",
        ],
    );
    let controller_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/shell_controller.rs"))
            .expect("shell controller source should be readable");
    assert!(
        controller_source.contains("AppCommand::SteerTurn(request)"),
        "TUI steer confirmation must dispatch the typed core command"
    );
}

#[test]
fn tui_approval_decisions_enter_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI approval decisions must enter core instead of calling the provider directly",
        &["src/adapter/inbound/tui"],
        &[
            ".resolve_approval_request(",
            "ConversationRuntimeEffect::ResolveApprovalRequest",
        ],
    );
    let controller_source =
        fs::read_to_string(repo_root().join("src/adapter/inbound/tui/app/shell_controller.rs"))
            .expect("shell controller source should be readable");
    let compact_controller_source = controller_source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_controller_source.contains(
            ".reduce_core_client_event(CoreInput::Command(AppCommand::SubmitApprovalDecision"
        ),
        "TUI approval decisions must positively enter through AppCommand::SubmitApprovalDecision"
    );
}

#[test]
fn tui_github_review_polling_enters_through_core_runtime() {
    assert_no_forbidden_references_in_paths(
        "TUI GitHub review polling must not own worker, cursor, or completion authority",
        &[
            "src/adapter/inbound/tui/app/github_polling.rs",
            "src/adapter/inbound/tui/app/app_runtime.rs",
            "src/adapter/inbound/tui/app/shell_runtime.rs",
            "src/adapter/inbound/tui/app/shell_entrypoint.rs",
        ],
        &["thread::spawn", ".poll("],
    );
    assert_no_forbidden_references_in_paths(
        "TUI GitHub review polling must not regain cursor or completion authority",
        &["src/adapter/inbound/tui"],
        &[
            "GithubPullRequestPollState",
            "GithubReviewPollLoaded",
            "next_github_review_poll_generation",
            "github_review_poll_generation",
        ],
    );
    assert_no_forbidden_references_in_paths(
        "NativeTuiApp must not retain the raw GitHub review poller service",
        &["src/adapter/inbound/tui/app.rs"],
        &["github_review_poller_service:"],
    );
    let polling_path = repo_root().join("src/adapter/inbound/tui/app/github_polling.rs");
    let polling_source =
        fs::read_to_string(&polling_path).expect("GitHub polling source should be readable");
    let polling_references = rust_semantic_references(&polling_source);
    assert!(
        polling_references
            .paths
            .iter()
            .any(|path| path.ends_with("AppCommand::SetupGithubReviewPolling"))
            && polling_references
                .paths
                .iter()
                .any(|path| path.ends_with("AppCommand::PollGithubReview")),
        "TUI GitHub setup and polling must positively enter through typed Core commands"
    );

    let forbidden_identifiers = [
        "GithubReviewPollerService",
        "GithubReviewPollerAdapter",
        "GithubAutomationAdapter",
        "GitParallelModeRuntimeAdapter",
        "discover_github_review_poller_service_for_current_branch",
        "build_github_review_poller_service",
        "parallel_mode_integration_branch_for_repo",
    ];
    let forbidden_paths = ["std::process", "std::thread", "thread::spawn"];
    for path in [
        "src/adapter/inbound/tui/app.rs",
        "src/adapter/inbound/tui/app/app_runtime.rs",
        "src/adapter/inbound/tui/app/github_polling.rs",
        "src/adapter/inbound/tui/app/inline_terminal_adapter.rs",
        "src/adapter/inbound/tui/app/shell_entrypoint.rs",
        "src/adapter/inbound/tui/app/shell_runtime.rs",
    ] {
        let source_path = repo_root().join(path);
        let source = fs::read_to_string(&source_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", source_path.display()));
        let references = rust_semantic_references(&source);
        let violations = references
            .paths
            .iter()
            .filter(|reference| {
                forbidden_identifiers
                    .iter()
                    .any(|forbidden| reference.split("::").any(|segment| segment == *forbidden))
                    || forbidden_paths.iter().any(|forbidden| {
                        reference == forbidden || reference.starts_with(&format!("{forbidden}::"))
                    })
            })
            .collect::<Vec<_>>();
        assert!(
            violations.is_empty(),
            "production TUI GitHub flow must not own service, adapter, discovery, process, or thread paths in {path}: {violations:?}"
        );
    }

    for path in [
        "src/adapter/inbound/tui/app/github_polling.rs",
        "src/adapter/inbound/tui/app/inline_terminal_adapter.rs",
        "src/adapter/inbound/tui/app/shell_entrypoint.rs",
        "src/adapter/inbound/tui/app/shell_runtime.rs",
    ] {
        let source = fs::read_to_string(repo_root().join(path)).unwrap();
        assert!(
            !rust_semantic_references(&source)
                .paths
                .iter()
                .any(|reference| reference
                    .split("::")
                    .any(|segment| segment == "NativeTuiApplicationHandle")),
            "GitHub setup flow must not receive a raw NativeTuiApplicationHandle in {path}"
        );
    }
}

#[test]
fn rust_semantic_guard_ignores_comments_strings_and_test_only_items() {
    let harmless = r#"
        fn production() {
            let _ = "GithubReviewPollerService std::thread::spawn";
            // std::process::Command::new("git");
        }

        #[cfg(test)]
        fn test_only() {
            std::process::Command::new("git");
            std::thread::spawn(|| {});
        }
    "#;
    let harmless_references = rust_semantic_references(harmless);
    assert!(
        harmless_references
            .paths
            .iter()
            .all(|path| !path.starts_with("std::process") && !path.starts_with("std::thread"))
    );

    let real = r#"fn production() { std::process::Command::new("git"); }"#;
    assert!(
        rust_semantic_references(real)
            .paths
            .iter()
            .any(|path| path.starts_with("std::process::Command"))
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
    assert_no_forbidden_references_in_paths(
        "TUI manual prompt correlation authority must stay core-owned",
        &["src/adapter/inbound/tui"],
        &[
            "next_manual_prompt_preparation_request_id",
            "manual_prompt_preparation_generation",
            "next_manual_prompt_preparation_correlation",
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
            "replace_runtime_slot_lease_if_matches",
            "upsert_runtime_task_dispatch_block",
            "transition_slot_lease",
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
    assert_no_forbidden_references_in_paths(
        "TUI post-turn execution must not own raw providers, process, threads, filesystem, database, Git, or GitHub access",
        &[
            "src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs",
            "src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution",
        ],
        &[
            "crate::adapter::outbound::",
            "crate::application::port::",
            "std::process",
            "std::thread",
            "std::fs",
            "Sqlite",
            "Filesystem",
            "GitAdapter",
            "Github",
        ],
    );
    let semantic_forbidden_prefixes = [
        "crate::adapter::outbound",
        "crate::application::port",
        "std::process",
        "std::thread",
        "std::fs",
    ];
    let semantic_forbidden_segments = [
        "Sqlite",
        "Filesystem",
        "GitAdapter",
        "Github",
        "NativeTuiApplicationHandle",
        "NativeTuiPlanningHandle",
        "PlanningWorkspaceUseCases",
    ];
    assert_no_semantic_references_in_paths(
        "TUI post-turn execution must not alias raw providers, process, threads, filesystem, database, Git, GitHub, or raw application handles",
        &[
            "src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs",
            "src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution",
        ],
        &semantic_forbidden_prefixes,
        &semantic_forbidden_segments,
    );
    for source in [
        "fn sample() { std::thread::spawn(|| {}); }",
        "use std::{fs, process, thread}; fn sample() { thread::spawn(|| {}); }",
        "use std::thread as worker; fn sample() { worker::spawn(|| {}); }",
        "use std as system; fn sample() { system::process::Command::new(\"git\"); }",
        "use crate::adapter as adapters; fn sample() { adapters::outbound::filesystem::FilesystemPlanningWorkspaceAdapter::new(); }",
        "use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter as Workspace; fn sample() { Workspace::new(); }",
    ] {
        assert!(
            rust_semantic_references(source).paths.iter().any(|path| {
                semantic_path_is_forbidden(
                    path,
                    &semantic_forbidden_prefixes,
                    &semantic_forbidden_segments,
                )
            }),
            "semantic post-turn guard must detect direct, grouped, and aliased imports: {source}"
        );
    }

    assert_no_forbidden_references_in_paths(
        "production TUI must not retain raw application/planning handles or expose planning workspace use cases",
        &["src/adapter/inbound/tui"],
        &[
            "NativeTuiApplicationHandle",
            "NativeTuiPlanningHandle",
            "PlanningWorkspaceUseCases",
        ],
    );
    let tui_app_source = fs::read_to_string("src/adapter/inbound/tui/app.rs").unwrap();
    let tui_app_syntax =
        syn::parse_file(&tui_app_source).expect("NativeTuiApp source should parse");
    assert!(
        named_struct_fields(&tui_app_syntax, "NativeTuiApp")
            .iter()
            .all(|field| field
                .ident
                .as_ref()
                .is_none_or(|ident| ident != "application")),
        "NativeTuiApp must not retain a raw application field"
    );

    let tui_post_turn_source = fs::read_to_string(
        "src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs",
    )
    .expect("TUI post-turn execution source should load");
    let execute_post_turn_evaluation =
        top_level_impl_method_source(&tui_post_turn_source, "execute_post_turn_evaluation");
    let execute_syntax = syn::parse_file(&format!(
        "impl NativeTuiApp {{\n{execute_post_turn_evaluation}\n}}"
    ))
    .expect("TUI post-turn request method should parse");
    let mut panel_field_reads = NamedFieldAccessVisitor::new("planning_worker_panel_state");
    panel_field_reads.visit_file(&execute_syntax);
    assert!(
        panel_field_reads.lines.is_empty()
            && !execute_post_turn_evaluation.contains("planning_worker_panel_state"),
        "TUI post-turn request admission must not seed Core from its projected panel state; field references: {:?}",
        panel_field_reads.lines
    );
    let field_guard_sample = syn::parse_file(
        "fn sample(app: &NativeTuiApp) { let _ = app.planning_worker_panel_state.clone(); }",
    )
    .expect("field guard sample should parse");
    let mut sample_field_reads = NamedFieldAccessVisitor::new("planning_worker_panel_state");
    sample_field_reads.visit_file(&field_guard_sample);
    assert_eq!(
        sample_field_reads.lines.len(),
        1,
        "planning-worker seed guard must detect a direct TUI field read"
    );

    let application_post_turn_request =
        top_level_function_source(&tui_post_turn_source, "application_post_turn_request");
    let compact_application_post_turn_request = application_post_turn_request
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_application_post_turn_request
            .contains("planning_worker_panel_state:Default::default()"),
        "the adapter request DTO must carry only a neutral placeholder until Core binds its history seed"
    );

    assert_no_production_callable_reference_named_in_paths(
        "production TUI must not call or retain the old post-turn panel-state helper",
        &["src/adapter/inbound/tui"],
        "post_turn_worker_panel_start_state",
    );
    for source in [
        "fn sample(service: &PlanningRuntimeUseCases) { service.post_turn_worker_panel_start_state(); }",
        "fn sample(service: &PlanningRuntimeUseCases) { PlanningRuntimeUseCases::post_turn_worker_panel_start_state(service); }",
        "fn sample() { let callable = PlanningRuntimeUseCases::post_turn_worker_panel_start_state; use_it(callable); }",
        "fn sample() { invoke!(PlanningRuntimeUseCases::post_turn_worker_panel_start_state); }",
    ] {
        assert!(
            !production_callable_reference_lines(source, "post_turn_worker_panel_start_state")
                .is_empty(),
            "old helper guard must detect direct, UFCS, function-pointer, path, and macro references: {source}"
        );
    }

    let core_controller = fs::read_to_string("src/core/app/controller.rs").unwrap();
    let conversation_turn_reducer =
        fs::read_to_string("src/core/app/conversation_turn_reducer.rs").unwrap();
    let planning_reducer = fs::read_to_string("src/core/app/planning_reducer.rs").unwrap();
    let core_event = fs::read_to_string("src/core/app/event.rs").unwrap();
    let core_runtime = fs::read_to_string("src/core/runtime/driver.rs").unwrap();
    let tui_runtime = fs::read_to_string("src/adapter/inbound/tui/app/app_runtime.rs").unwrap();
    let tui_tests =
        fs::read_to_string("src/adapter/inbound/tui/app/shell_runtime/tests.rs").unwrap();

    let planning_syntax =
        syn::parse_file(&planning_reducer).expect("planning reducer source should parse");
    let planning_fields = named_struct_fields(&planning_syntax, "PlanningFeatureReducer");
    let history_seed = planning_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "worker_panel_history_seed")
        })
        .expect("PlanningFeatureReducer must own the planning-worker panel history seed");
    assert!(
        is_named_path_type(&history_seed.ty, "PlanningWorkerPanelState"),
        "Core planning-worker history seed must retain the complete domain panel state"
    );

    let start_state =
        top_level_impl_method_source(&planning_reducer, "begin_post_turn_worker_panel");
    let compact_start_state = start_state
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_start_state.contains("request:&PostTurnRequest")
            && compact_start_state.contains("letmutstate=self.worker_panel_history_seed.clone()")
            && !compact_start_state.contains("request.planning_worker_panel_state"),
        "planning reducer start-state policy must derive history from its own seed, never the inbound request field"
    );

    let handle_input_inner = top_level_impl_method_source(&core_controller, "handle_input_inner");
    let compact_handle_input = handle_input_inner
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_handle_input
            .contains("self.planning.begin_post_turn_worker_panel(request.as_ref())")
            && compact_handle_input.contains("request.as_ref()")
            && compact_handle_input.contains(
                "request.planning_worker_panel_state=planning_worker_panel_state.clone()"
            ),
        "Core admission must overwrite the effect request with the state derived from Core history"
    );
    let accepted_reducer_settlement = compact_handle_input
        .find("conversation_turn.complete_post_turn_evaluation(&correlation,execution.as_ref())")
        .expect("Core must delegate exact post-turn settlement to the feature reducer");
    let history_seed_commit = compact_handle_input
        .find("self.planning.accept_post_turn_worker_panel(execution.as_ref())")
        .expect("Core must retain the accepted completion as the next history seed");
    assert!(
        accepted_reducer_settlement < history_seed_commit,
        "Core history may change only after the feature reducer accepts exact settlement"
    );
    let post_turn_settlement =
        top_level_impl_method_source(&conversation_turn_reducer, "complete_post_turn_evaluation");
    let compact_post_turn_settlement = post_turn_settlement
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let exact_correlation_guard = compact_post_turn_settlement
        .find("active_post_turn_evaluation_correlation()!=Some(correlation)")
        .expect("feature reducer must reject a completion outside the exact active lease");
    let exact_execution_guard = compact_post_turn_settlement
        .find("!correlation.matches_execution(execution)")
        .expect("feature reducer must reject a mismatched execution identity");
    let accepted_turn_guard = compact_post_turn_settlement
        .find("accept_post_turn_evaluation_completion(execution)")
        .expect("feature reducer must let turn authority accept exact completion");
    assert!(
        exact_correlation_guard < accepted_turn_guard
            && exact_execution_guard < accepted_turn_guard,
        "turn acceptance must follow exact correlation and execution identity gates"
    );

    let reset_history_seed =
        top_level_impl_method_source(&planning_reducer, "reset_worker_panel_history");
    let compact_reset_history_seed = reset_history_seed
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_reset_history_seed
            .contains("self.worker_panel_history_seed=PlanningWorkerPanelState::default()"),
        "conversation lifecycle reset must discard the prior planning-worker panel history"
    );
    let reset_call_lines =
        production_callable_reference_lines(&core_controller, "reset_worker_panel_history");
    assert_eq!(
        reset_call_lines.len(),
        2,
        "Core must reset planning-worker history exactly once for invalidation and once for common conversation-load admission; calls: {reset_call_lines:?}"
    );

    for required in [
        "fn post_turn_start_state_obeys_priority_and_preserves_panel_detail()",
        "fn post_turn_panel_history_updates_only_after_exact_accepted_completion()",
        "fn conversation_lifecycle_resets_post_turn_panel_history()",
    ] {
        assert!(
            core_controller.contains(required),
            "Core post-turn panel-state ownership is missing: {required}"
        );
    }
    assert!(
        core_event.contains("PostTurnEvaluationStarted(PlanningWorkerPanelState)"),
        "Core must publish the full post-turn panel state before dispatching evaluation"
    );
    assert!(
        core_runtime.contains("fn immediate_post_turn_completion_keeps_started_event_first()"),
        "Core runtime must prove Started precedes an immediate completion"
    );
    assert!(
        tui_runtime.contains("AppEvent::PostTurnEvaluationStarted(state)")
            && tui_runtime.contains(".planning_worker_panel_state")
            && tui_runtime.contains(".apply_started(state);"),
        "TUI must apply the Core-started panel state through its sealed projection"
    );
    assert!(
        tui_tests
            .contains("fn post_turn_evaluation_started_event_ignores_forged_tui_history_seed()"),
        "TUI must prove a forged presentation history cannot seed Core admission"
    );
}

#[test]
fn post_turn_evaluation_uses_one_core_owned_exact_correlation() {
    let request =
        fs::read_to_string("src/core/app/request.rs").expect("Core request source should load");
    for required in [
        "pub struct PostTurnEvaluationCorrelation",
        "pub generation: u64",
        "pub thread_id: String",
        "pub completed_turn_id: String",
        "pub turn_workspace_directory: String",
        "pub planning_workspace_directory: String",
        "pub fn matches_execution",
        "execution.evaluation.provenance.completed_turn_id",
        "queue_mutation_receipt",
    ] {
        assert!(
            request.contains(required),
            "post-turn exact correlation contract is missing: {required}"
        );
    }

    let effect =
        fs::read_to_string("src/core/app/effect.rs").expect("Core effect source should load");
    let event = fs::read_to_string("src/core/app/event.rs").expect("Core event source should load");
    assert!(
        effect.contains("EvaluatePostTurn {")
            && effect.contains("correlation: PostTurnEvaluationCorrelation")
            && effect.contains("request: Box<PostTurnRequest>"),
        "post-turn effect must carry the Core-owned correlation beside the domain request"
    );
    assert!(
        event.contains("PostTurnEvaluationCompleted {")
            && event.contains("correlation: PostTurnEvaluationCorrelation")
            && event.contains("execution: Box<PostTurnExecution>"),
        "post-turn completion must return the same Core-owned correlation"
    );

    let controller = fs::read_to_string("src/core/app/controller.rs")
        .expect("Core controller source should load");
    let reducer = fs::read_to_string("src/core/app/conversation_turn_reducer.rs")
        .expect("conversation/turn reducer source should load");
    let production_controller = production_source_before_inline_tests(&controller);
    let production_reducer = production_source_before_inline_tests(&reducer);
    assert!(
        production_reducer.contains("struct ActivePostTurnEvaluation")
            && production_reducer.contains("correlation: PostTurnEvaluationCorrelation")
            && production_reducer.contains("continuation_permit: PostTurnContinuationPermit")
            && production_reducer
                .contains("in_flight_post_turn_evaluation: Option<ActivePostTurnEvaluation>")
            && production_reducer.contains("next_post_turn_evaluation_generation: u64")
            && production_reducer
                .contains("if self.active_post_turn_evaluation_correlation() != Some(correlation)")
            && production_reducer.contains("|| !correlation.matches_execution(execution)")
            && production_reducer.contains("fn prune_post_turn_evaluation_for_lifecycle")
            && production_reducer.contains("fn cancel_active_post_turn_evaluation")
            && production_reducer.contains("active.continuation_permit.invalidate_if_current()")
            && !production_reducer
                .contains("in_flight_post_turn_evaluation: Option<(String, String)>")
            && production_controller.contains("conversation_turn: ConversationTurnFeatureReducer")
            && !production_controller.contains("in_flight_post_turn_evaluation:"),
        "ConversationTurnFeatureReducer must bind exact post-turn correlation and worker authority in one lifecycle-pruned active lease"
    );
    for behavior_test in [
        "fn post_turn_completion_requires_latest_exact_correlation_once_across_aba()",
        "fn lifecycle_only_aba_prunes_the_old_post_turn_lease_before_completion()",
        "fn deferred_conversation_load_cancels_active_post_turn_authority_immediately()",
        "fn forged_post_turn_payload_cannot_settle_the_exact_lease()",
        "fn post_turn_evaluation_generation_exhaustion_fails_before_admission()",
    ] {
        assert!(
            controller.contains(behavior_test),
            "Core post-turn correlation behavior proof is missing: {behavior_test}"
        );
    }

    let runner = fs::read_to_string("src/composition/core_effect_runner.rs")
        .expect("Core effect runner source should load");
    let production_runner = production_source_before_inline_tests(&runner);
    assert!(
        production_runner.contains("correlation: correlation.clone()")
            && production_runner.contains(
                "post_turn_evaluation_completion(correlation, &fallback_request, execution)"
            )
            && production_runner
                .contains("\"post-turn evaluation worker returned a mismatched target\"",)
            && production_runner.contains("request.continuation_permit.invalidate_if_current()"),
        "normal, malformed, and panic post-turn worker outcomes must preserve exact correlation and settle safely"
    );
    assert!(
        runner.contains(
            "fn mismatched_post_turn_execution_becomes_one_correlated_safe_failure_with_local_revoke()",
        ),
        "composition must prove malformed post-turn output becomes one correlated safe failure without invalidating a newer permit"
    );

    let planning_contracts = fs::read_to_string("src/domain/planning/runtime_contracts.rs")
        .expect("planning runtime contracts should load");
    assert!(
        planning_contracts.contains("request_valid: std::sync::Arc<std::sync::atomic::AtomicBool>")
            && planning_contracts.contains("&& self.request_valid.load(Ordering::SeqCst)")
            && planning_contracts
                .contains("std::sync::Arc::ptr_eq(&self.request_valid, &other.request_valid)")
            && planning_contracts.contains("newer_request.is_current()"),
        "worker failure must invalidate only its request-local permit, not a newer request in the shared gate generation"
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
fn production_tui_receives_only_opaque_runtime_capabilities() {
    let repo_root = repo_root();
    let mut violations = Vec::new();
    for path in rust_files_under(&repo_root.join("src/adapter/inbound/tui")) {
        if is_test_only_path(&path) {
            continue;
        }
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for reference in forbidden_tui_runtime_capability_references(&source) {
            violations.push(format!("{}: {reference}", relative_path(&repo_root, &path)));
        }
    }

    assert!(
        violations.is_empty(),
        "production TUI must not receive raw services, ports, repositories, workers, process/thread capabilities, or outbound adapters:\n{}",
        violations.join("\n")
    );

    let shell_entrypoint = fs::read_to_string("src/adapter/inbound/tui/app/shell_entrypoint.rs")
        .expect("native shell entrypoint should load");
    let shell_references = rust_semantic_references(&shell_entrypoint);
    assert!(
        shell_references
            .paths
            .iter()
            .any(|path| path.ends_with("production::build_native_tui_application"))
            && shell_references
                .paths
                .iter()
                .any(|path| path.ends_with("NativeTuiApp::new_with_github_review_polling")),
        "production shell must receive one opaque native application composition"
    );

    let composition_source = fs::read_to_string("src/composition/native_client_runtime.rs")
        .expect("native client composition should load");
    let composition_syntax =
        syn::parse_file(&composition_source).expect("native client composition must parse as Rust");
    for struct_name in [
        "NativeTuiApplicationComposition",
        "BoundNativeTuiApplication",
    ] {
        assert!(
            named_struct_fields(&composition_syntax, struct_name)
                .iter()
                .all(|field| matches!(field.vis, syn::Visibility::Inherited)),
            "{struct_name} must keep its raw capabilities private"
        );
    }

    let from_services = inherent_impl_methods(
        &composition_syntax,
        "NativeTuiApplicationComposition",
        "from_services",
    );
    assert_eq!(
        from_services.len(),
        1,
        "native TUI composition must define one from_services constructor"
    );
    assert!(
        matches!(
            &from_services[0].vis,
            syn::Visibility::Restricted(visibility)
                if path_is_simple(&visibility.path, &["crate", "composition"])
        ),
        "raw-service construction must be visible only inside crate::composition"
    );
}

#[test]
fn tui_runtime_capability_guard_ignores_fixtures_and_rejects_real_escapes() {
    let harmless = r#"
        fn production(app: NativeClientRuntime) {
            let _ = "ConversationService std::thread::spawn";
            app.snapshot();
        }

        #[cfg(test)]
        fn fixture(service: ConversationService) {
            std::thread::spawn(move || service.run());
        }
    "#;
    assert!(
        forbidden_tui_runtime_capability_references(harmless).is_empty(),
        "comments, literals, and cfg(test) fixtures must not trigger the production guard"
    );

    let raw_service = "fn production(service: ConversationService) { service.run(); }";
    assert!(
        forbidden_tui_runtime_capability_references(raw_service)
            .iter()
            .any(|path| path.ends_with("ConversationService"))
    );

    let raw_worker = "fn production() { std::thread::spawn(|| mutate_provider()); }";
    assert!(
        forbidden_tui_runtime_capability_references(raw_worker)
            .iter()
            .any(|path| path.starts_with("std::thread"))
    );

    let inferred_raw_service = "fn production() { let _ = production::build_planning_services(); }";
    assert!(
        forbidden_tui_runtime_capability_references(inferred_raw_service)
            .iter()
            .any(|path| path.ends_with("production::build_planning_services")),
        "return-type inference must not hide a raw production service builder"
    );

    let forged_completion =
        "fn production(completion: Value) { CoreInput::EffectCompleted(completion); }";
    assert!(
        forbidden_tui_runtime_capability_references(forged_completion)
            .iter()
            .any(|path| path.ends_with("CoreInput::EffectCompleted")),
        "production TUI must not forge effect completions"
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
        "## Current-Stack Default And Compatibility Ownership",
        "## Compatibility-Tier Ownership Table",
        "Stay on the current Ratatui/Crossterm stack by default.",
        "Option A proof hardening is the default",
        "blocked unless the Decision Record explicitly proves the Round 6 trigger evidence",
        "Default `InlineHistoryRenderMode`",
        "Default `HistoryInsertionMode`",
        "Terminal primitive behavior ownership",
        "Reviewer gate / release semantics",
        "## Manual Capture Contract",
        "### Reviewer gate",
        "Manual capture is required **only** for primitive-sensitive changes",
    ] {
        assert!(
            docs.contains(required_doc_text),
            "TUI methodology must require frame-recorder coverage and concrete proof-contract markers: {required_doc_text}"
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

    for required_matrix_text in [
        "split scrollback/live-tail streams render as a titleless live tail",
        "typed render surface routing",
        "## Primary Proof Matrix — Invariant × First-Class Environment",
        "## Linked Branch-Family Applicability Table — Invariant × Branch Family",
        "I1 | History/live-tail separation and no duplicate replay",
        "I2 | Resize leaves no stale rows or duplicated live tail",
        "I3 | Scrollback insertion / clear-reset restores clean header and viewport",
        "I4 | Thread/session switch does not leak transcript or deferred history",
        "I5 | `ViewportReplay` remains explicit-only and does not write committed history to host scrollback",
        "I6 | Standard and fallback insertion modes each preserve viewport state correctly",
        "B1 HostScrollback",
        "B2 ViewportReplay",
        "B3 StandardScrollRegion",
        "B4 NewlineFallback",
        "activate Option B.",
    ] {
        assert!(
            matrix.contains(required_matrix_text),
            "TUI coverage matrix must name the durable proof-contract marker: {required_matrix_text}"
        );
    }

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
        "## Proof Contract Markers",
        "## Primary Proof Matrix — Invariant × First-Class Environment",
        "## Linked Branch-Family Applicability Table — Invariant × Branch Family",
        "## Joined Proof Shape",
        "I5 | `ViewportReplay` remains explicit-only and does not write committed history to host scrollback",
    ] {
        assert!(
            matrix.contains(required_text),
            "TUI coverage matrix must document the project-wide TUI testing rule: {required_text}"
        );
    }
    for required_methodology_text in [
        "docs/validation/tui-coverage-matrix.md",
        "## Current-Stack Default And Compatibility Ownership",
        "## Compatibility-Tier Ownership Table",
        "### First-class environment key",
        "### Branch-family key",
        "## Responsibility Candidate Summary",
        "`NativeTuiApp`",
        "Thin terminal layer",
        "Render/layout boundary",
        "Shared render transaction model",
        "## Manual Capture Contract",
        "### Reviewer gate",
        "### When all four first-class environments are required",
        "### When a smaller representative set is sufficient",
    ] {
        assert!(
            methodology.contains(required_methodology_text),
            "TUI methodology must document the concrete proof contract marker: {required_methodology_text}"
        );
    }

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
fn native_runtime_validation_proof_contract_is_documented_in_repo_guards() {
    // Static guard: native runtime validation evidence should stay contract-first.
    // The repo-facing docs must keep the proof schema, environment keys, branch
    // family keys, and primitive-sensitive manual-capture rule explicit.
    let repo_root = repo_root();
    let matrix_path = repo_root.join("docs/validation/tui-coverage-matrix.md");
    let methodology_path = repo_root.join("docs/validation/terminal-ui-testing-methodology.md");
    let matrix = fs::read_to_string(&matrix_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", matrix_path.display());
    });
    let methodology = fs::read_to_string(&methodology_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", methodology_path.display());
    });

    let common_required_texts = [
        "**E1** = Windows Terminal + WSL bash + inline",
        "**E2** = Windows Terminal + PowerShell + inline",
        "**E3** = tmux detached PTY + inline",
        "**E4** = direct Linux terminal + inline",
        "bug-class recurrence across compatibility boundaries",
        "fallback masking risk",
        "future test-growth cost",
        "maintainability cost",
        "Current owner / source",
        "Decision point",
        "First-class default",
        "Fallback / experimental handling",
        "Override mechanism",
        "Downgrade semantics",
        "Proof obligation",
        "Manual terminal capture stays primitive-sensitive only",
        "escape sequences",
        "viewport mode",
        "clear or restore behavior",
        "host scrollback behavior",
    ];
    let methodology_specific_texts = [
        "## Current-Stack Default And Compatibility Ownership",
        "invariant × first-class environment × branch family",
        "`HostScrollback`, `ViewportReplay`, `StandardScrollRegion`, `NewlineFallback`",
        "current stack remains the default posture",
        "docs/plan/12-platform-validation-matrix.md",
        "macOS Terminal.app and iTerm2",
    ];
    for required_text in common_required_texts
        .iter()
        .copied()
        .chain(methodology_specific_texts.iter().copied())
    {
        assert!(
            methodology.contains(required_text),
            "TUI methodology must document native runtime validation proof contract text: {required_text}"
        );
    }

    let matrix_specific_texts = [
        "## Proof Contract Markers",
        "## Primary Proof Matrix — Invariant × First-Class Environment",
        "## Linked Branch-Family Applicability Table — Invariant × Branch Family",
        "## Joined Proof Shape",
        "Branch-family keys: `HostScrollback`, `ViewportReplay`, `StandardScrollRegion`, `NewlineFallback`.",
        "current stack as the default posture",
    ];
    for required_text in common_required_texts
        .iter()
        .copied()
        .chain(matrix_specific_texts.iter().copied())
    {
        assert!(
            matrix.contains(required_text),
            "TUI coverage matrix must document native runtime validation proof contract text: {required_text}"
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

#[test]
fn rust_aware_crate_reference_scan_handles_groups_aliases_and_test_modules() {
    let source = r###"
use crate::{
    application::service::Runner,
    domain::Model as DomainModel,
};

const DISPLAY_ONLY: &str = "crate::adapter::outbound::NotADependency";

fn production_path() {
    let _ = crate::
        core::Runtime;
    // crate::diagnostics::CommentOnly
}

macro_rules! emit_runtime {
    () => { crate::adapter::outbound::MacroAdapter::new() };
}

wire!(crate::{adapter::outbound::GroupedAdapter, domain::GroupedModel});
invoke!(crate::composition::Builder);

struct Fixture;

impl Fixture {
    #[cfg(test)]
    fn test_only_macro() {
        invoke!(crate::adapter::outbound::TestOnlyMacro);
    }

    #[cfg_attr(test, allow(dead_code))]
    fn production_macro() {
        invoke!(crate::core::CfgAttrRuntime);
    }
}

#[cfg(test)]
mod tests {
    use crate::adapter::outbound::TestOnlyAdapter;
}
"###;

    let references = rust_crate_references(source)
        .into_iter()
        .map(|reference| reference.path)
        .collect::<Vec<_>>();

    assert_eq!(
        references,
        vec![
            "crate::application::service::Runner",
            "crate::domain::Model",
            "crate::core::Runtime",
            "crate::adapter::outbound::MacroAdapter::new",
            "crate::adapter::outbound::GroupedAdapter",
            "crate::domain::GroupedModel",
            "crate::composition::Builder",
            "crate::core::CfgAttrRuntime",
        ]
    );
}

#[test]
fn production_pattern_scan_ignores_comments_and_rust_literals() {
    let source = r####"
const NORMAL: &str = "Command::new and crate::adapter::outbound::Fake";
const RAW: &str = r#"std::process and crate::core::Fake"#;
// StartupService and crate::application::Fake
/* nested /* PlanningServices */ crossterm */

fn production() {
    Command::new("akra");
}

#[cfg_attr(test, allow(dead_code))]
fn cfg_attr_item_stays_in_production_scan() {
    std::process::id();
}

struct Fixture;

impl Fixture {
    #[cfg_attr(test, allow(dead_code))]
    fn cfg_attr_associated_item_stays_in_production_scan() {
        AssociatedProductionMarker::run();
    }

    #[cfg(test)]
    fn test_only_associated_item() {
        AssociatedTestMarker::run();
    }
}

#[cfg(test)]
fn test_only() {
    std::process::Command::new("ignored");
}
"####;
    let production = production_lines(source)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(production.contains("Command::new"));
    assert!(production.contains("std::process::id"));
    assert!(production.contains("AssociatedProductionMarker::run"));
    assert!(!production.contains("crate::adapter"));
    assert!(!production.contains("std::process::Command"));
    assert!(!production.contains("AssociatedTestMarker"));
    assert!(!production.contains("StartupService"));
    assert!(!production.contains("PlanningServices"));
}

#[test]
fn renderer_boundary_scan_ignores_fixtures_and_rejects_production_escape_hatches() {
    let fixture_only = r#"
        use ratatui::Frame;

        const EXAMPLE: &str = "NativeTuiApp client_runtime snapshot";

        fn draw_owned(frame: &mut Frame<'_>, model: InlineShellFrameModel) {
            // NativeTuiApp and AppSnapshot are architecture examples only.
            render(frame, model);
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            fn legacy_fixture(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
                app.client_runtime.snapshot();
                render(frame, app);
            }
        }
    "#;
    assert!(
        renderer_production_boundary_violations(fixture_only).is_empty(),
        "comments, literals, and cfg(test) fixtures must not create renderer violations"
    );

    let production_escape_hatches = r#"
        use super::*;

        #[cfg_attr(test, allow(dead_code))]
        fn draw_live(
            frame: &mut Frame<'_>,
            app: &mut NativeTuiApp,
            state: &mut SessionOverlayUiState,
        ) {
            app.client_runtime.snapshot();
            render(frame, state);
        }
    "#;
    let violations = renderer_production_boundary_violations(production_escape_hatches)
        .into_iter()
        .map(|(_, violation)| violation)
        .collect::<Vec<_>>();
    for expected in [
        "glob import",
        "NativeTuiApp",
        "mutable reference other than &mut Frame",
        "client_runtime",
        "snapshot",
    ] {
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "renderer boundary scan must catch `{expected}`: {violations:?}"
        );
    }
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
    let test_module_index = ["#[cfg(test)]\nmod ", "#[cfg(test)]\r\nmod "]
        .into_iter()
        .filter_map(|marker| source.find(marker))
        .min()
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

        for crate_reference in rust_crate_references(&source) {
            for pattern in rule
                .forbidden_patterns
                .iter()
                .filter(|pattern| pattern.starts_with("crate::"))
            {
                if crate_reference_matches_prefix(&crate_reference.path, pattern) {
                    violations.push(BoundaryViolation {
                        rule: rule.name,
                        path: relative_path.clone(),
                        line: crate_reference.line,
                        pattern,
                        text: source_line(&source, crate_reference.line),
                    });
                }
            }
        }

        for source_line in production_lines(&source) {
            for pattern in rule
                .forbidden_patterns
                .iter()
                .filter(|pattern| !pattern.starts_with("crate::"))
            {
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

        for crate_reference in rust_crate_references(&source) {
            if !rule.allowed_prefixes.iter().any(|allowed_prefix| {
                crate_reference_matches_prefix(&crate_reference.path, allowed_prefix)
            }) {
                violations.push(BoundaryViolation {
                    rule: rule.name,
                    path: relative_path.clone(),
                    line: crate_reference.line,
                    pattern: "crate::",
                    text: source_line(&source, crate_reference.line),
                });
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

            for crate_reference in rust_crate_references(&source) {
                for pattern in forbidden_patterns
                    .iter()
                    .filter(|pattern| pattern.starts_with("crate::"))
                {
                    if crate_reference_matches_prefix(&crate_reference.path, pattern) {
                        violations.push(BoundaryViolation {
                            rule: rule_name,
                            path: relative_path.clone(),
                            line: crate_reference.line,
                            pattern,
                            text: source_line(&source, crate_reference.line),
                        });
                    }
                }
            }

            for source_line in production_lines(&source) {
                for pattern in forbidden_patterns
                    .iter()
                    .filter(|pattern| !pattern.starts_with("crate::"))
                {
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

fn assert_no_semantic_references_in_paths(
    rule_name: &'static str,
    path_suffixes: &[&str],
    forbidden_prefixes: &[&str],
    forbidden_segments: &[&str],
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
            for reference in
                rust_semantic_references(&source)
                    .paths
                    .into_iter()
                    .filter(|reference| {
                        semantic_path_is_forbidden(
                            reference,
                            forbidden_prefixes,
                            forbidden_segments,
                        )
                    })
            {
                violations.push(format!("{relative_path}: {reference}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "{rule_name}:\n{}",
        violations.join("\n")
    );
}

fn semantic_path_is_forbidden(
    path: &str,
    forbidden_prefixes: &[&str],
    forbidden_segments: &[&str],
) -> bool {
    forbidden_prefixes
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}::")))
        || path.split("::").any(|segment| {
            forbidden_segments
                .iter()
                .any(|forbidden| segment.contains(forbidden))
        })
}

fn production_call_expression_names(source: &str) -> Vec<String> {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("architecture source must parse as Rust: {error}"));
    let mut visitor = CallExpressionVisitor::default();
    visitor.visit_file(&syntax);
    normalized_call_names(visitor.names)
}

fn named_function_call_expression_names(source: &str, function_name: &str) -> Vec<String> {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("architecture source must parse as Rust: {error}"));
    let mut visitor = CallExpressionVisitor::default();
    let mut match_count = 0;

    for item in &syntax.items {
        match item {
            syn::Item::Fn(function)
                if function.sig.ident == function_name && !item_is_test_only(item) =>
            {
                match_count += 1;
                visitor.visit_block(&function.block);
            }
            syn::Item::Impl(item_impl) if !item_is_test_only(item) => {
                for impl_item in &item_impl.items {
                    let syn::ImplItem::Fn(function) = impl_item else {
                        continue;
                    };
                    if function.sig.ident == function_name
                        && !impl_item_attributes(impl_item).is_some_and(attributes_are_test_only)
                    {
                        match_count += 1;
                        visitor.visit_block(&function.block);
                    }
                }
            }
            _ => {}
        }
    }

    assert_eq!(
        match_count, 1,
        "architecture source should define one production function named {function_name}"
    );
    normalized_call_names(visitor.names)
}

fn reachable_callable_expression_names(source: &str, entrypoint: &str) -> Vec<String> {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("architecture source must parse as Rust: {error}"));
    let mut callables = HashMap::<String, Vec<&syn::Block>>::new();
    for item in &syntax.items {
        match item {
            syn::Item::Fn(function) if !item_is_test_only(item) => {
                callables
                    .entry(function.sig.ident.to_string())
                    .or_default()
                    .push(&function.block);
            }
            syn::Item::Impl(item_impl) if !item_is_test_only(item) => {
                for impl_item in &item_impl.items {
                    let syn::ImplItem::Fn(function) = impl_item else {
                        continue;
                    };
                    if !impl_item_attributes(impl_item).is_some_and(attributes_are_test_only) {
                        callables
                            .entry(function.sig.ident.to_string())
                            .or_default()
                            .push(&function.block);
                    }
                }
            }
            _ => {}
        }
    }
    assert!(
        callables.contains_key(entrypoint),
        "architecture source should define production entrypoint {entrypoint}"
    );

    let mut visited = HashSet::new();
    let mut calls = Vec::new();
    collect_reachable_callable_expressions(entrypoint, &callables, &mut visited, &mut calls);
    normalized_call_names(calls)
}

fn collect_reachable_callable_expressions(
    callable_name: &str,
    callables: &HashMap<String, Vec<&syn::Block>>,
    visited: &mut HashSet<String>,
    calls: &mut Vec<String>,
) {
    let blocks = callables
        .get(callable_name)
        .unwrap_or_else(|| panic!("reachable callable {callable_name} should exist"));
    for (index, block) in blocks.iter().enumerate() {
        if !visited.insert(format!("{callable_name}#{index}")) {
            continue;
        }
        let mut visitor = CallExpressionVisitor::default();
        visitor.visit_block(block);
        let CallExpressionVisitor {
            names,
            callable_candidates,
        } = visitor;
        calls.extend(names);

        for candidate in callable_candidates {
            if callables.contains_key(&candidate) {
                collect_reachable_callable_expressions(&candidate, callables, visited, calls);
            }
        }
    }
}

fn normalized_call_names(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}

#[derive(Default)]
struct CallExpressionVisitor {
    names: Vec<String>,
    callable_candidates: Vec<String>,
}

#[derive(Default)]
struct ComposerFieldAccessVisitor {
    lines: Vec<usize>,
}

struct NamedFieldAccessVisitor<'a> {
    field_name: &'a str,
    lines: Vec<usize>,
}

impl<'a> NamedFieldAccessVisitor<'a> {
    fn new(field_name: &'a str) -> Self {
        Self {
            field_name,
            lines: Vec::new(),
        }
    }
}

#[derive(Default)]
struct DirectFunctionCallVisitor {
    calls: Vec<(String, usize)>,
}

impl<'ast> Visit<'ast> for DirectFunctionCallVisitor {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(function) = call.func.as_ref() {
            self.calls.push((
                function
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::"),
                call.args.len(),
            ));
        }
        visit::visit_expr_call(self, call);
    }
}

impl<'ast> Visit<'ast> for ComposerFieldAccessVisitor {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if item_is_test_only(item) {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if matches!(&field.member, syn::Member::Named(member) if member == "composer") {
            self.lines.push(field.member.span().start().line);
        }
        visit::visit_expr_field(self, field);
    }
}

impl<'ast> Visit<'ast> for NamedFieldAccessVisitor<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if item_is_test_only(item) {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if matches!(&field.member, syn::Member::Named(member) if member == self.field_name) {
            self.lines.push(field.member.span().start().line);
        }
        visit::visit_expr_field(self, field);
    }
}

impl<'ast> Visit<'ast> for CallExpressionVisitor {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if item_is_test_only(item) {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if impl_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_impl_item(self, item);
    }

    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        if let syn::Expr::Path(function_path) = expression.func.as_ref()
            && let Some(segment) = function_path.path.segments.last()
        {
            let name = segment.ident.to_string();
            self.names.push(name.clone());
            self.callable_candidates.push(name);
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        let name = expression.method.to_string();
        self.names.push(name.clone());
        self.callable_candidates.push(name);
        visit::visit_expr_method_call(self, expression);
    }

    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        if let Some(segment) = expression.path.segments.last() {
            let name = segment.ident.to_string();
            self.names.push(name.clone());
            self.callable_candidates.push(name);
        }
        visit::visit_expr_path(self, expression);
    }
}

fn rust_crate_references(source: &str) -> Vec<CrateReference> {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("architecture source must parse as Rust: {error}"));
    let mut visitor = CrateReferenceVisitor::default();
    visitor.visit_file(&syntax);
    visitor.references.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.path.cmp(&right.path))
    });
    visitor.references.dedup();
    visitor.references
}

#[derive(Default)]
struct RustSemanticReferences {
    paths: Vec<String>,
}

fn rust_semantic_references(source: &str) -> RustSemanticReferences {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("architecture source must parse as Rust: {error}"));
    let mut visitor = RustSemanticReferenceVisitor::default();
    visitor.visit_file(&syntax);
    expand_semantic_alias_paths(&mut visitor.references.paths, &visitor.aliases);
    visitor.references.paths.sort();
    visitor.references.paths.dedup();
    visitor.references
}

fn forbidden_tui_runtime_capability_references(source: &str) -> Vec<String> {
    rust_semantic_references(source)
        .paths
        .into_iter()
        .filter(|reference| {
            let identifier = reference.rsplit("::").next().unwrap_or(reference);
            matches!(
                identifier,
                "CoreEffectRunner"
                    | "CoreEffectCompletion"
                    | "CoreRuntime"
                    | "NativeTuiParallelModeBinding"
                    | "ParallelModeControlPlaneComposition"
            ) || identifier.ends_with("Service")
                || identifier.ends_with("Services")
                || identifier.ends_with("Port")
                || identifier.ends_with("Repository")
                || identifier.ends_with("_service")
                || identifier.ends_with("_services")
                || identifier.ends_with("_port")
                || identifier.ends_with("_repository")
                || reference.ends_with("CoreInput::EffectCompleted")
                || (reference.contains("production::build_")
                    && !reference.ends_with("production::build_native_tui_application"))
                || reference == "crate::adapter::outbound"
                || reference.starts_with("crate::adapter::outbound::")
                || reference == "std::process"
                || reference.starts_with("std::process::")
                || reference == "std::thread"
                || reference.starts_with("std::thread::")
                || reference == "tokio::spawn"
                || reference == "tokio::task"
                || reference.starts_with("tokio::task::")
        })
        .collect()
}

fn expand_semantic_alias_paths(paths: &mut Vec<String>, aliases: &[(String, String)]) {
    for _ in 0..=aliases.len() {
        let mut additions = Vec::new();
        for path in paths.iter() {
            let (root, suffix) = path
                .split_once("::")
                .map_or((path.as_str(), None), |(root, suffix)| (root, Some(suffix)));
            for (alias, original) in aliases {
                if root != alias {
                    continue;
                }
                let expanded = suffix.map_or_else(
                    || original.clone(),
                    |suffix| format!("{original}::{suffix}"),
                );
                if !paths.contains(&expanded) && !additions.contains(&expanded) {
                    additions.push(expanded);
                }
            }
        }
        if additions.is_empty() {
            break;
        }
        paths.extend(additions);
    }
}

#[derive(Default)]
struct RustSemanticReferenceVisitor {
    references: RustSemanticReferences,
    aliases: Vec<(String, String)>,
}

impl<'ast> Visit<'ast> for RustSemanticReferenceVisitor {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if item_is_test_only(item) {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if attributes_are_test_only(&arm.attrs) {
            return;
        }
        visit::visit_arm(self, arm);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if impl_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if trait_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_trait_item(self, item);
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        if foreign_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_foreign_item(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        collect_semantic_use_paths(
            &item.tree,
            &mut Vec::new(),
            &mut self.references.paths,
            &mut self.aliases,
        );
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.references.paths.push(
            path.segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>()
                .join("::"),
        );
        visit::visit_path(self, path);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.references.paths.push(call.method.to_string());
        visit::visit_expr_method_call(self, call);
    }
}

#[derive(Default)]
struct TerminalDrawClosureReferenceVisitor {
    references: Vec<Vec<String>>,
}

impl<'ast> Visit<'ast> for TerminalDrawClosureReferenceVisitor {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "draw" {
            for argument in &call.args {
                let syn::Expr::Closure(closure) = argument else {
                    continue;
                };
                let mut visitor = RustSemanticReferenceVisitor::default();
                visitor.visit_expr(&closure.body);
                expand_semantic_alias_paths(&mut visitor.references.paths, &visitor.aliases);
                visitor.references.paths.sort();
                visitor.references.paths.dedup();
                self.references.push(visitor.references.paths);
            }
        }
        visit::visit_expr_method_call(self, call);
    }
}

fn collect_semantic_use_paths(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    references: &mut Vec<String>,
    aliases: &mut Vec<(String, String)>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_semantic_use_paths(&path.tree, prefix, references, aliases);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            prefix.push(name.ident.to_string());
            references.push(prefix.join("::"));
            prefix.pop();
        }
        syn::UseTree::Rename(rename) => {
            prefix.push(rename.ident.to_string());
            let original = prefix.join("::");
            references.push(original.clone());
            aliases.push((rename.rename.to_string(), original));
            prefix.pop();
        }
        syn::UseTree::Glob(_) => {
            references.push(prefix.join("::"));
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_semantic_use_paths(item, prefix, references, aliases);
            }
        }
    }
}

#[derive(Default)]
struct CrateReferenceVisitor {
    references: Vec<CrateReference>,
}

impl<'ast> Visit<'ast> for CrateReferenceVisitor {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if item_is_test_only(item) {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if impl_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if trait_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_trait_item(self, item);
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        if foreign_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_foreign_item(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        collect_use_tree_references(&item.tree, &mut Vec::new(), &mut self.references);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if path.leading_colon.is_none()
            && segments.len() > 1
            && segments.first().is_some_and(|root| root == "crate")
        {
            self.references.push(CrateReference {
                line: path.span().start().line,
                path: segments.join("::"),
            });
        }
        visit::visit_path(self, path);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        collect_macro_token_references(&mac.tokens, &mut self.references);
        visit::visit_macro(self, mac);
    }
}

fn collect_macro_token_references(tokens: &TokenStream, references: &mut Vec<CrateReference>) {
    let tokens = tokens.clone().into_iter().collect::<Vec<_>>();

    for token in &tokens {
        if let TokenTree::Group(group) = token {
            collect_macro_token_references(&group.stream(), references);
        }
    }

    for (index, token) in tokens.iter().enumerate() {
        let TokenTree::Ident(root) = token else {
            continue;
        };
        if root != "crate" {
            continue;
        }

        collect_rooted_token_path(
            &tokens,
            index + 1,
            vec![root.to_string()],
            root.span().start().line,
            references,
        );
    }
}

fn collect_rooted_token_path(
    tokens: &[TokenTree],
    mut cursor: usize,
    mut segments: Vec<String>,
    line: usize,
    references: &mut Vec<CrateReference>,
) {
    while token_pair_is_path_separator(tokens, cursor) {
        match tokens.get(cursor + 2) {
            Some(TokenTree::Ident(segment)) => {
                segments.push(segment.to_string());
                cursor += 3;
            }
            Some(TokenTree::Punct(punct)) if punct.as_char() == '*' => {
                segments.push("*".to_string());
                cursor += 3;
            }
            Some(TokenTree::Group(group)) => {
                collect_grouped_token_paths(&group.stream(), &segments, line, references);
                return;
            }
            _ => break,
        }
    }

    if segments.len() > 1 {
        references.push(CrateReference {
            line,
            path: segments.join("::"),
        });
    }
}

fn collect_grouped_token_paths(
    tokens: &TokenStream,
    prefix: &[String],
    line: usize,
    references: &mut Vec<CrateReference>,
) {
    let tokens = tokens.clone().into_iter().collect::<Vec<_>>();
    let mut branch_start = 0usize;

    for branch_end in (0..=tokens.len()).filter(|index| {
        *index == tokens.len()
            || matches!(tokens.get(*index), Some(TokenTree::Punct(punct)) if punct.as_char() == ',')
    }) {
        let branch = &tokens[branch_start..branch_end];
        if let Some(TokenTree::Ident(segment)) = branch.first() {
            let mut segments = prefix.to_vec();
            if segment != "self" {
                segments.push(segment.to_string());
            }
            collect_rooted_token_path(branch, 1, segments, line, references);
        } else if matches!(branch.first(), Some(TokenTree::Punct(punct)) if punct.as_char() == '*')
        {
            let mut segments = prefix.to_vec();
            segments.push("*".to_string());
            references.push(CrateReference {
                line,
                path: segments.join("::"),
            });
        }
        branch_start = branch_end + 1;
    }
}

fn token_pair_is_path_separator(tokens: &[TokenTree], start: usize) -> bool {
    matches!(tokens.get(start), Some(TokenTree::Punct(punct)) if punct.as_char() == ':')
        && matches!(tokens.get(start + 1), Some(TokenTree::Punct(punct)) if punct.as_char() == ':')
}

fn collect_use_tree_references(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    references: &mut Vec<CrateReference>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_tree_references(&path.tree, prefix, references);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            prefix.push(name.ident.to_string());
            push_use_reference(prefix, name.ident.span().start().line, references);
            prefix.pop();
        }
        syn::UseTree::Rename(rename) => {
            prefix.push(rename.ident.to_string());
            push_use_reference(prefix, rename.ident.span().start().line, references);
            prefix.pop();
        }
        syn::UseTree::Glob(glob) => {
            prefix.push("*".to_string());
            push_use_reference(prefix, glob.star_token.span.start().line, references);
            prefix.pop();
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree_references(item, prefix, references);
            }
        }
    }
}

fn push_use_reference(segments: &[String], line: usize, references: &mut Vec<CrateReference>) {
    if segments.first().is_some_and(|root| root == "crate") {
        references.push(CrateReference {
            line,
            path: segments.join("::"),
        });
    }
}

fn verify_native_client_event_contract(
    native_facade: &str,
    tui_app_runtime: &str,
) -> Result<(), String> {
    let facade_syntax = syn::parse_file(native_facade)
        .map_err(|error| format!("native client facade must parse: {error}"))?;
    let client_events = facade_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Enum(item)
                if item.ident == "NativeClientEvent" && !attributes_are_test_only(&item.attrs) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [client_events] = client_events.as_slice() else {
        return Err(format!(
            "expected one production NativeClientEvent enum, found {}",
            client_events.len()
        ));
    };
    let expected_routes = client_events
        .variants
        .iter()
        .filter(|variant| !attributes_are_test_only(&variant.attrs))
        .map(|variant| variant.ident.to_string())
        .collect::<HashSet<_>>();

    let dispatch_methods = inherent_impl_methods(
        &facade_syntax,
        "NativeClientRuntime",
        "dispatch_client_event",
    );
    let [dispatch] = dispatch_methods.as_slice() else {
        return Err(format!(
            "expected one NativeClientRuntime::dispatch_client_event, found {}",
            dispatch_methods.len()
        ));
    };
    let event_matches = dispatch
        .block
        .stmts
        .iter()
        .filter_map(|statement| match statement {
            syn::Stmt::Expr(syn::Expr::Match(expression), None)
                if expression_is_simple_path(expression.expr.as_ref(), &["event"]) =>
            {
                Some(expression)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [event_match] = event_matches.as_slice() else {
        return Err(format!(
            "dispatch_client_event must contain one direct `match event`, found {}",
            event_matches.len()
        ));
    };
    let mut actual_routes = HashSet::new();
    for arm in &event_match.arms {
        if arm.guard.is_some() {
            return Err(format!(
                "NativeClientEvent arm at line {} must not use a match guard",
                arm.span().start().line
            ));
        }
        for variant in exact_enum_pattern_variants(&arm.pat, "NativeClientEvent")
            .map_err(|error| format!("NativeClientEvent router {error}"))?
        {
            if !actual_routes.insert(variant.clone()) {
                return Err(format!(
                    "NativeClientEvent::{variant} must appear in exactly one dispatch arm"
                ));
            }
        }
    }
    if actual_routes != expected_routes {
        return Err(format!(
            "NativeClientEvent dispatch arms must exactly cover the enum ({})",
            core_effect_set_difference(&actual_routes, &expected_routes)
        ));
    }

    let app_syntax = syn::parse_file(tui_app_runtime)
        .map_err(|error| format!("TUI app runtime must parse: {error}"))?;
    let background_messages = app_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Enum(item)
                if item.ident == "BackgroundMessage" && !attributes_are_test_only(&item.attrs) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [background_messages] = background_messages.as_slice() else {
        return Err(format!(
            "expected one production BackgroundMessage enum, found {}",
            background_messages.len()
        ));
    };
    let production_variants = background_messages
        .variants
        .iter()
        .filter(|variant| !attributes_are_test_only(&variant.attrs))
        .collect::<Vec<_>>();
    let [operator_alert] = production_variants.as_slice() else {
        return Err(format!(
            "production BackgroundMessage must remain presentation-only with one OperatorAlert variant; found {:?}",
            production_variants
                .iter()
                .map(|variant| variant.ident.to_string())
                .collect::<Vec<_>>()
        ));
    };
    let syn::Fields::Unnamed(fields) = &operator_alert.fields else {
        return Err(
            "production BackgroundMessage::OperatorAlert must carry one typed payload".to_string(),
        );
    };
    if operator_alert.ident != "OperatorAlert"
        || fields.unnamed.len() != 1
        || !is_named_path_type(&fields.unnamed[0].ty, "OperatorAlert")
    {
        return Err(
            "production BackgroundMessage must remain presentation-only as OperatorAlert(OperatorAlert)"
                .to_string(),
        );
    }

    Ok(())
}

fn verify_client_runtime_api_boundary(
    core_module: &str,
    runtime_module: &str,
    driver: &str,
    mailbox: &str,
    native_facade: &str,
) -> Result<(), String> {
    let core_syntax =
        syn::parse_file(core_module).map_err(|error| format!("core module must parse: {error}"))?;
    let runtime_item = core_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Mod(item) if item.ident == "runtime" => Some(item),
            _ => None,
        })
        .ok_or_else(|| "core module must declare runtime".to_string())?;
    if !visibility_is_restricted_to(&runtime_item.vis, &["crate"]) {
        return Err("core::runtime module must be restricted to pub(crate)".to_string());
    }

    let runtime_syntax = syn::parse_file(runtime_module)
        .map_err(|error| format!("runtime module must parse: {error}"))?;
    let driver_module = runtime_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Mod(item) if item.ident == "driver" => Some(item),
            _ => None,
        })
        .ok_or_else(|| "runtime module must declare driver".to_string())?;
    if !matches!(driver_module.vis, syn::Visibility::Inherited) {
        return Err("raw runtime driver module must remain private".to_string());
    }
    let runtime_reexports = runtime_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Use(item) => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    if runtime_reexports.len() != 2
        || runtime_reexports
            .iter()
            .any(|item| !visibility_is_restricted_to(&item.vis, &["crate"]))
    {
        return Err(
            "raw runtime types and mailbox must have exactly two pub(crate) re-exports".to_string(),
        );
    }

    let driver_syntax =
        syn::parse_file(driver).map_err(|error| format!("runtime driver must parse: {error}"))?;
    let runtime_struct = driver_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "CoreRuntime" => Some(item),
            _ => None,
        })
        .ok_or_else(|| "runtime driver must define CoreRuntime".to_string())?;
    if !visibility_is_restricted_to(&runtime_struct.vis, &["crate"]) {
        return Err("CoreRuntime visibility must remain pub(crate)".to_string());
    }
    let executor_trait = driver_syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Trait(item) if item.ident == "CoreEffectExecutor" => Some(item),
            _ => None,
        })
        .ok_or_else(|| "runtime driver must define CoreEffectExecutor".to_string())?;
    if !visibility_is_restricted_to(&executor_trait.vis, &["crate"]) {
        return Err("CoreEffectExecutor visibility must remain pub(crate)".to_string());
    }

    let expected_runtime_api = [
        "dispatch_command",
        "dispatch_input",
        "new",
        "parallel_mode_projection",
        "poll_pending_input",
        "revisioned_planning_parallel_projection",
        "snapshot",
    ];
    let mut actual_runtime_api = Vec::new();
    for item in &driver_syntax.items {
        if item_is_test_only(item) {
            continue;
        }
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        let implements_core_runtime = matches!(
            item_impl.self_ty.as_ref(),
            syn::Type::Path(type_path)
                if type_path.qself.is_none()
                    && type_path.path.segments.last().is_some_and(|segment| {
                        segment.ident == "CoreRuntime"
                    })
        );
        if item_impl.trait_.is_some() || !implements_core_runtime {
            continue;
        }
        for impl_item in &item_impl.items {
            let syn::ImplItem::Fn(method) = impl_item else {
                continue;
            };
            if attributes_are_test_only(&method.attrs) {
                continue;
            }
            if matches!(method.vis, syn::Visibility::Public(_)) {
                return Err(format!(
                    "CoreRuntime::{} must not be publicly reachable",
                    method.sig.ident
                ));
            }
            if visibility_is_restricted_to(&method.vis, &["crate"]) {
                actual_runtime_api.push(method.sig.ident.to_string());
            }
        }
    }
    actual_runtime_api.sort();
    if actual_runtime_api != expected_runtime_api {
        return Err(format!(
            "CoreRuntime crate API must stay bounded; expected {expected_runtime_api:?}, found {actual_runtime_api:?}"
        ));
    }

    let mailbox_syntax =
        syn::parse_file(mailbox).map_err(|error| format!("runtime mailbox must parse: {error}"))?;
    for type_name in ["CoreInputSender", "CoreInputReceiver"] {
        let item = mailbox_syntax
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == type_name => Some(item),
                _ => None,
            })
            .ok_or_else(|| format!("runtime mailbox must define {type_name}"))?;
        if !visibility_is_restricted_to(&item.vis, &["crate"]) {
            return Err(format!("{type_name} visibility must remain pub(crate)"));
        }
    }
    let channel = top_level_function(&mailbox_syntax, "core_input_channel");
    if !visibility_is_restricted_to(&channel.vis, &["crate"]) {
        return Err("core_input_channel visibility must remain pub(crate)".to_string());
    }
    let sender_methods = inherent_impl_methods(&mailbox_syntax, "CoreInputSender", "send");
    if sender_methods.len() != 1 || !visibility_is_restricted_to(&sender_methods[0].vis, &["crate"])
    {
        return Err("CoreInputSender::send visibility must remain pub(crate)".to_string());
    }
    if mailbox_syntax.items.iter().any(|item| {
        if item_is_test_only(item) {
            return false;
        }
        let visibility = match item {
            syn::Item::Const(item) => Some(&item.vis),
            syn::Item::Fn(item) => Some(&item.vis),
            syn::Item::Struct(item) => Some(&item.vis),
            _ => None,
        };
        visibility.is_some_and(|visibility| matches!(visibility, syn::Visibility::Public(_)))
    }) {
        return Err("runtime mailbox must expose no public top-level item".to_string());
    }

    let facade_syntax = syn::parse_file(native_facade)
        .map_err(|error| format!("native client facade must parse: {error}"))?;
    let expected_facade_api = [
        "current_parallel_epoch_id_for_workspace",
        "dispatch_client_event",
        "parallel_control_plane_projection",
        "parallel_epoch_snapshot",
        "parallel_mode_enabled",
        "parallel_mode_projection",
        "poll_pending_client_event",
        "revisioned_planning_parallel_projection",
        "snapshot",
    ];
    let read_only_facade_api = [
        "current_parallel_epoch_id_for_workspace",
        "parallel_control_plane_projection",
        "parallel_epoch_snapshot",
        "parallel_mode_enabled",
        "parallel_mode_projection",
        "revisioned_planning_parallel_projection",
        "snapshot",
    ];
    let mut actual_facade_api = Vec::new();
    for item in &facade_syntax.items {
        if item_is_test_only(item) {
            continue;
        }
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        if item_impl.trait_.is_some()
            || !type_is_simple_path(item_impl.self_ty.as_ref(), &["NativeClientRuntime"])
        {
            continue;
        }
        for impl_item in &item_impl.items {
            let syn::ImplItem::Fn(method) = impl_item else {
                continue;
            };
            if attributes_are_test_only(&method.attrs) {
                continue;
            }
            if matches!(method.vis, syn::Visibility::Public(_)) {
                return Err(format!(
                    "NativeClientRuntime::{} must remain crate-private",
                    method.sig.ident
                ));
            }
            if !visibility_is_restricted_to(&method.vis, &["crate"]) {
                continue;
            }
            let method_name = method.sig.ident.to_string();
            if read_only_facade_api.contains(&method_name.as_str()) {
                let receiver = method.sig.receiver().ok_or_else(|| {
                    format!("NativeClientRuntime::{method_name} must receive &self")
                })?;
                if receiver.reference.is_none() || receiver.mutability.is_some() {
                    return Err(format!(
                        "NativeClientRuntime::{method_name} must be read-only"
                    ));
                }
                if matches!(
                    &method.sig.output,
                    syn::ReturnType::Type(_, ty) if matches!(ty.as_ref(), syn::Type::Reference(_))
                ) {
                    return Err(format!(
                        "NativeClientRuntime::{method_name} must return an owned projection"
                    ));
                }
            }
            actual_facade_api.push(method_name);
        }
    }
    actual_facade_api.sort();
    if actual_facade_api != expected_facade_api {
        return Err(format!(
            "NativeClientRuntime API must be dispatch/poll or owned read-only projection only; expected {expected_facade_api:?}, found {actual_facade_api:?}"
        ));
    }

    Ok(())
}

fn verify_app_state_controller_seal(
    app_module: &str,
    controller: &str,
    state: &str,
) -> Result<(), String> {
    let app_syntax = syn::parse_file(app_module)
        .map_err(|error| format!("core app module must parse: {error}"))?;
    reject_production_item_macros("core/app", &app_syntax)?;
    if app_syntax.items.iter().any(|item| {
        matches!(
            item,
            syn::Item::Mod(module)
                if module.ident == "state" && !attributes_are_test_only(&module.attrs)
        )
    }) {
        return Err(
            "core/app must not declare a sibling `state` module beside CoreController".to_string(),
        );
    }
    if app_syntax.items.iter().any(|item| {
        matches!(
            item,
            syn::Item::Use(item)
                if !attributes_are_test_only(&item.attrs)
                    && use_tree_mentions_identifier(&item.tree, "AppState")
        )
    }) {
        return Err("core/app must not import or re-export AppState".to_string());
    }

    let controller_syntax = syn::parse_file(controller)
        .map_err(|error| format!("CoreController source must parse: {error}"))?;
    reject_production_item_macros("CoreController", &controller_syntax)?;
    let production_modules = production_module_paths(&controller_syntax);
    let unexpected_modules = production_modules
        .iter()
        .filter(|path| path.as_str() != "state")
        .cloned()
        .collect::<Vec<_>>();
    if !unexpected_modules.is_empty() {
        return Err(format!(
            "CoreController must not declare production child modules beside its private `state` child: {unexpected_modules:?}"
        ));
    }
    let state_modules = controller_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(module)
                if module.ident == "state" && !attributes_are_test_only(&module.attrs) =>
            {
                Some(module)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [state_module] = state_modules.as_slice() else {
        return Err(format!(
            "CoreController must declare exactly one state child, found {}",
            state_modules.len()
        ));
    };
    if !matches!(state_module.vis, syn::Visibility::Inherited) || state_module.content.is_some() {
        return Err("CoreController state must remain a private child file module".to_string());
    }

    let state_imports = controller_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Use(item)
                if !attributes_are_test_only(&item.attrs)
                    && use_tree_mentions_identifier(&item.tree, "AppState") =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [state_import] = state_imports.as_slice() else {
        return Err(format!(
            "CoreController must import AppState exactly once, found {} imports",
            state_imports.len()
        ));
    };
    if !matches!(state_import.vis, syn::Visibility::Inherited)
        || !use_tree_is_simple_path(&state_import.tree, &["self", "state", "AppState"])
    {
        return Err(
            "CoreController must privately import AppState from self::state::AppState".to_string(),
        );
    }

    let state_syntax =
        syn::parse_file(state).map_err(|error| format!("AppState source must parse: {error}"))?;
    reject_production_item_macros("AppState state child", &state_syntax)?;
    let state_production_modules = production_module_paths(&state_syntax);
    if !state_production_modules.is_empty() {
        return Err(format!(
            "AppState state child must not declare production child modules: {state_production_modules:?}"
        ));
    }
    let app_states = state_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item)
                if item.ident == "AppState" && !attributes_are_test_only(&item.attrs) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [app_state] = app_states.as_slice() else {
        return Err(format!(
            "state child must define exactly one production AppState, found {}",
            app_states.len()
        ));
    };
    if !visibility_is_restricted_to(&app_state.vis, &["super"]) {
        return Err(
            "AppState visibility must be restricted to its parent CoreController".to_string(),
        );
    }
    let syn::Fields::Named(fields) = &app_state.fields else {
        return Err("AppState must use one named storage field".to_string());
    };
    if fields.named.len() != 1 {
        return Err("AppState must retain exactly one snapshot authority field".to_string());
    }
    let current = fields
        .named
        .first()
        .expect("one AppState field was checked above");
    if current
        .ident
        .as_ref()
        .is_none_or(|ident| ident != "current")
        || !type_is_single_generic_path(&current.ty, "Arc", "AppSnapshot")
    {
        return Err("AppState authority must remain `current: Arc<AppSnapshot>`".to_string());
    }
    if !matches!(current.vis, syn::Visibility::Inherited) {
        return Err("AppState storage field must remain private".to_string());
    }

    for item in &state_syntax.items {
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        if item_impl.trait_.is_some()
            || attributes_are_test_only(&item_impl.attrs)
            || !type_is_simple_path(item_impl.self_ty.as_ref(), &["AppState"])
        {
            continue;
        }
        for item in &item_impl.items {
            let syn::ImplItem::Fn(method) = item else {
                continue;
            };
            if attributes_are_test_only(&method.attrs) {
                continue;
            }
            if !matches!(method.vis, syn::Visibility::Inherited)
                && !visibility_is_restricted_to(&method.vis, &["super"])
            {
                return Err(format!(
                    "AppState::{} must remain private or pub(super)",
                    method.sig.ident
                ));
            }
        }
    }

    Ok(())
}

fn visibility_is_restricted_to(visibility: &syn::Visibility, expected: &[&str]) -> bool {
    matches!(
        visibility,
        syn::Visibility::Restricted(restricted)
            if path_is_simple(&restricted.path, expected)
    )
}

fn item_is_test_only(item: &syn::Item) -> bool {
    item_attributes(item).is_some_and(attributes_are_test_only)
}

fn production_module_paths(syntax: &syn::File) -> Vec<String> {
    let mut visitor = ProductionModulePathVisitor::default();
    visitor.visit_file(syntax);
    visitor.paths
}

fn reject_production_item_macros(label: &str, syntax: &syn::File) -> Result<(), String> {
    let mut visitor = ProductionItemMacroVisitor::default();
    visitor.visit_file(syntax);
    if visitor.names.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{label} must not contain production item macros: {:?}",
            visitor.names
        ))
    }
}

#[derive(Default)]
struct ProductionItemMacroVisitor {
    names: Vec<String>,
}

const SEALED_SOURCE_ALLOWED_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "eprintln",
    "format",
    "matches",
    "panic",
    "println",
    "todo",
    "unimplemented",
    "unreachable",
    "vec",
];

const SEALED_SOURCE_ALLOWED_ATTRIBUTES: &[&str] = &[
    "allow",
    "cfg",
    "cold",
    "deny",
    "deprecated",
    "doc",
    "expect",
    "forbid",
    "inline",
    "must_use",
    "non_exhaustive",
    "repr",
    "warn",
];

const SEALED_SOURCE_ALLOWED_DERIVES: &[&str] = &[
    "Clone",
    "Copy",
    "Debug",
    "Default",
    "Eq",
    "Hash",
    "Ord",
    "PartialEq",
    "PartialOrd",
];

fn macro_name(expression: &syn::Macro) -> String {
    expression
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_else(|| "<anonymous>".to_string())
}

fn syn_path_name(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn sealed_source_macro_is_allowed(expression: &syn::Macro) -> bool {
    expression.path.leading_colon.is_none()
        && expression.path.segments.len() == 1
        && SEALED_SOURCE_ALLOWED_MACROS.contains(&macro_name(expression).as_str())
        && !macro_tokens_contain_sealed_source_escape(&expression.tokens)
}

fn sealed_source_attribute_is_allowed(attribute: &syn::Attribute) -> bool {
    if attribute.path().is_ident("derive") {
        return attribute
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|derives| {
                derives.iter().all(|derive| {
                    derive.leading_colon.is_none()
                        && derive.segments.len() == 1
                        && SEALED_SOURCE_ALLOWED_DERIVES
                            .contains(&derive.segments[0].ident.to_string().as_str())
                })
            });
    }
    attribute.path().leading_colon.is_none()
        && attribute.path().segments.len() == 1
        && SEALED_SOURCE_ALLOWED_ATTRIBUTES
            .contains(&attribute.path().segments[0].ident.to_string().as_str())
}

fn macro_tokens_contain_sealed_source_escape(tokens: &TokenStream) -> bool {
    let token_trees = tokens.clone().into_iter().collect::<Vec<_>>();
    if token_trees.windows(3).any(|window| {
        matches!(
            window,
            [
                TokenTree::Ident(_),
                TokenTree::Punct(bang),
                next,
            ] if bang.as_char() == '!'
                && !matches!(next, TokenTree::Punct(equals) if equals.as_char() == '=')
        )
    }) || matches!(
        token_trees.as_slice(),
        [.., TokenTree::Ident(_), TokenTree::Punct(bang)] if bang.as_char() == '!'
    ) {
        return true;
    }
    for token in tokens.clone() {
        let TokenTree::Group(group) = token else {
            continue;
        };
        let group_tokens = TokenStream::from(TokenTree::Group(group.clone()));
        if group.delimiter() == proc_macro2::Delimiter::Brace
            && let Ok(block) = syn::parse2::<syn::Block>(group_tokens)
        {
            if block.stmts.iter().any(
                |statement| matches!(statement, syn::Stmt::Item(item) if !item_is_test_only(item)),
            ) {
                return true;
            }
            let mut nested_macros = ProductionItemMacroVisitor::default();
            nested_macros.visit_block(&block);
            if !nested_macros.names.is_empty() {
                return true;
            }
        }
        if macro_tokens_contain_sealed_source_escape(&group.stream()) {
            return true;
        }
    }
    macro_token_identifiers(tokens).contains("AppState")
}

fn use_tree_can_shadow_allowed_macro(tree: &syn::UseTree) -> bool {
    match tree {
        syn::UseTree::Path(path) => use_tree_can_shadow_allowed_macro(path.tree.as_ref()),
        syn::UseTree::Name(name) => {
            SEALED_SOURCE_ALLOWED_MACROS.contains(&name.ident.to_string().as_str())
        }
        syn::UseTree::Rename(rename) => {
            SEALED_SOURCE_ALLOWED_MACROS.contains(&rename.rename.to_string().as_str())
        }
        syn::UseTree::Group(group) => group.items.iter().any(use_tree_can_shadow_allowed_macro),
        syn::UseTree::Glob(_) => true,
    }
}

impl<'ast> Visit<'ast> for ProductionItemMacroVisitor {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if item_is_test_only(item) {
            return;
        }
        if let Some(attributes) = item_attributes(item) {
            for attribute in attributes {
                if !sealed_source_attribute_is_allowed(attribute) {
                    self.names.push(format!(
                        "procedural attribute `{}`",
                        syn_path_name(attribute.path())
                    ));
                }
            }
        }
        visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if impl_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        if let Some(attributes) = impl_item_attributes(item) {
            for attribute in attributes {
                if !sealed_source_attribute_is_allowed(attribute) {
                    self.names.push(format!(
                        "procedural attribute `{}`",
                        syn_path_name(attribute.path())
                    ));
                }
            }
        }
        visit::visit_impl_item(self, item);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        let name = item
            .ident
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| macro_name(&item.mac));
        self.names.push(name);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if use_tree_can_shadow_allowed_macro(&item.tree) {
            self.names.push("macro-shadowing use".to_string());
        }
    }

    fn visit_stmt_macro(&mut self, statement: &'ast syn::StmtMacro) {
        if attributes_are_test_only(&statement.attrs) {
            return;
        }
        if !sealed_source_macro_is_allowed(&statement.mac) {
            self.names.push(macro_name(&statement.mac));
        }
    }

    fn visit_expr_macro(&mut self, expression: &'ast syn::ExprMacro) {
        if attributes_are_test_only(&expression.attrs) {
            return;
        }
        if !sealed_source_macro_is_allowed(&expression.mac) {
            self.names.push(macro_name(&expression.mac));
        }
    }
}

#[derive(Default)]
struct ProductionModulePathVisitor {
    parents: Vec<String>,
    paths: Vec<String>,
}

impl<'ast> Visit<'ast> for ProductionModulePathVisitor {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        if attributes_are_test_only(&module.attrs) {
            return;
        }
        self.parents.push(module.ident.to_string());
        self.paths.push(self.parents.join("::"));
        visit::visit_item_mod(self, module);
        self.parents.pop();
    }
}

fn attributes_are_test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string() == "test")
    })
}

fn item_attributes(item: &syn::Item) -> Option<&[syn::Attribute]> {
    Some(match item {
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::ExternCrate(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::ForeignMod(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Macro(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::TraitAlias(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        syn::Item::Use(item) => &item.attrs,
        syn::Item::Verbatim(_) => return None,
        _ => return None,
    })
}

fn impl_item_attributes(item: &syn::ImplItem) -> Option<&[syn::Attribute]> {
    Some(match item {
        syn::ImplItem::Const(item) => &item.attrs,
        syn::ImplItem::Fn(item) => &item.attrs,
        syn::ImplItem::Type(item) => &item.attrs,
        syn::ImplItem::Macro(item) => &item.attrs,
        syn::ImplItem::Verbatim(_) => return None,
        _ => return None,
    })
}

fn trait_item_attributes(item: &syn::TraitItem) -> Option<&[syn::Attribute]> {
    Some(match item {
        syn::TraitItem::Const(item) => &item.attrs,
        syn::TraitItem::Fn(item) => &item.attrs,
        syn::TraitItem::Type(item) => &item.attrs,
        syn::TraitItem::Macro(item) => &item.attrs,
        syn::TraitItem::Verbatim(_) => return None,
        _ => return None,
    })
}

fn foreign_item_attributes(item: &syn::ForeignItem) -> Option<&[syn::Attribute]> {
    Some(match item {
        syn::ForeignItem::Fn(item) => &item.attrs,
        syn::ForeignItem::Static(item) => &item.attrs,
        syn::ForeignItem::Type(item) => &item.attrs,
        syn::ForeignItem::Macro(item) => &item.attrs,
        syn::ForeignItem::Verbatim(_) => return None,
        _ => return None,
    })
}

fn crate_reference_matches_prefix(reference: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches(':');
    reference == prefix || reference.starts_with(&format!("{prefix}::"))
}

fn source_line(source: &str, line: usize) -> String {
    source
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or_default()
        .trim()
        .to_string()
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

fn named_struct_fields<'a>(syntax: &'a syn::File, struct_name: &str) -> Vec<&'a syn::Field> {
    let item = syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == struct_name => Some(item),
            _ => None,
        })
        .unwrap_or_else(|| panic!("source should define named struct {struct_name}"));
    let syn::Fields::Named(fields) = &item.fields else {
        panic!("{struct_name} should use named fields");
    };
    fields.named.iter().collect()
}

fn inherent_impl_methods<'a>(
    syntax: &'a syn::File,
    type_name: &str,
    method_name: &str,
) -> Vec<&'a syn::ImplItemFn> {
    syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item)
                if item.trait_.is_none()
                    && !attributes_are_test_only(&item.attrs)
                    && type_is_simple_path(item.self_ty.as_ref(), &[type_name]) =>
            {
                Some(item)
            }
            _ => None,
        })
        .flat_map(|item| item.items.iter())
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method)
                if method.sig.ident == method_name && !attributes_are_test_only(&method.attrs) =>
            {
                Some(method)
            }
            _ => None,
        })
        .collect()
}

fn top_level_function<'a>(syntax: &'a syn::File, function_name: &str) -> &'a syn::ItemFn {
    syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function)
                if !attributes_are_test_only(&function.attrs)
                    && function.sig.ident == function_name =>
            {
                Some(function)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected one non-test function named {function_name}"))
}

fn assert_direct_function_call(
    function: &syn::ItemFn,
    called_function: &str,
    argument_count: usize,
) {
    let mut calls = DirectFunctionCallVisitor::default();
    calls.visit_block(&function.block);
    let matching_calls = calls
        .calls
        .iter()
        .filter(|(called, _)| called == called_function)
        .collect::<Vec<_>>();
    assert_eq!(
        matching_calls.len(),
        1,
        "{} must call the direct path {called_function} exactly once",
        function.sig.ident
    );
    assert_eq!(
        matching_calls[0].1, argument_count,
        "{} must call {called_function} with {argument_count} arguments",
        function.sig.ident
    );
}

fn is_shared_reference_to_named_type_with_lifetime(
    ty: &syn::Type,
    expected_name: &str,
    expected_lifetime: &str,
) -> bool {
    let syn::Type::Reference(reference) = ty else {
        return false;
    };
    reference.mutability.is_none()
        && reference
            .lifetime
            .as_ref()
            .is_some_and(|lifetime| lifetime.ident == expected_lifetime)
        && matches!(
            reference.elem.as_ref(),
            syn::Type::Path(type_path)
                if type_path.qself.is_none()
                    && type_path.path.is_ident(expected_name)
        )
}

fn is_shared_reference_to_single_lifetime_named_type(
    ty: &syn::Type,
    expected_name: &str,
    expected_lifetime: &str,
) -> bool {
    let syn::Type::Reference(reference) = ty else {
        return false;
    };
    reference.mutability.is_none()
        && reference.lifetime.is_none()
        && is_single_lifetime_named_type(&reference.elem, expected_name, expected_lifetime)
}

fn is_option_of_single_lifetime_named_type(
    ty: &syn::Type,
    expected_inner_name: &str,
    expected_lifetime: &str,
) -> bool {
    let syn::Type::Path(option_type) = ty else {
        return false;
    };
    if option_type.qself.is_some() || option_type.path.segments.len() != 1 {
        return false;
    }
    let option_segment = option_type
        .path
        .segments
        .first()
        .expect("one Option segment");
    let syn::PathArguments::AngleBracketed(arguments) = &option_segment.arguments else {
        return false;
    };
    let Some(syn::GenericArgument::Type(inner)) = arguments.args.first() else {
        return false;
    };
    option_segment.ident == "Option"
        && arguments.args.len() == 1
        && is_single_lifetime_named_type(inner, expected_inner_name, expected_lifetime)
}

fn is_single_lifetime_named_type(
    ty: &syn::Type,
    expected_name: &str,
    expected_lifetime: &str,
) -> bool {
    let syn::Type::Path(type_path) = ty else {
        return false;
    };
    if type_path.qself.is_some() || type_path.path.segments.len() != 1 {
        return false;
    }
    let segment = type_path.path.segments.first().expect("one type segment");
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    arguments.args.len() == 1
        && matches!(
            arguments.args.first(),
            Some(syn::GenericArgument::Lifetime(lifetime))
                if segment.ident == expected_name && lifetime.ident == expected_lifetime
        )
}

fn is_named_path_type(ty: &syn::Type, expected_name: &str) -> bool {
    matches!(
        ty,
        syn::Type::Path(type_path)
            if type_path.qself.is_none()
                && type_path.path.segments.last().is_some_and(|segment| {
                    segment.ident == expected_name
                        && matches!(segment.arguments, syn::PathArguments::None)
                })
    )
}

fn type_mentions_named_path(ty: &syn::Type, expected_name: &str) -> bool {
    struct NamedTypeVisitor<'a> {
        expected_name: &'a str,
        found: bool,
    }

    impl<'ast> Visit<'ast> for NamedTypeVisitor<'_> {
        fn visit_type_path(&mut self, type_path: &'ast syn::TypePath) {
            if type_path
                .path
                .segments
                .iter()
                .any(|segment| segment.ident == self.expected_name)
            {
                self.found = true;
                return;
            }
            visit::visit_type_path(self, type_path);
        }
    }

    let mut visitor = NamedTypeVisitor {
        expected_name,
        found: false,
    };
    visitor.visit_type(ty);
    visitor.found
}

fn is_single_generic_named_type(ty: &syn::Type, outer_name: &str, inner_name: &str) -> bool {
    let syn::Type::Path(outer) = ty else {
        return false;
    };
    let Some(outer_segment) = outer.path.segments.last() else {
        return false;
    };
    if outer.qself.is_some() || outer_segment.ident != outer_name {
        return false;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &outer_segment.arguments else {
        return false;
    };
    let Some(syn::GenericArgument::Type(inner)) = arguments.args.first() else {
        return false;
    };
    arguments.args.len() == 1 && is_named_path_type(inner, inner_name)
}

fn is_option_string_type(ty: &syn::Type) -> bool {
    let syn::Type::Path(outer) = ty else {
        return false;
    };
    if outer.qself.is_some() || outer.path.segments.len() != 1 {
        return false;
    }
    let outer_segment = outer.path.segments.first().expect("one outer segment");
    if outer_segment.ident != "Option" {
        return false;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &outer_segment.arguments else {
        return false;
    };
    let Some(syn::GenericArgument::Type(syn::Type::Path(inner))) = arguments.args.first() else {
        return false;
    };
    arguments.args.len() == 1 && inner.qself.is_none() && inner.path.is_ident("String")
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
    let code_only_source = rust_code_without_comments_and_literals(source);
    let test_only_ranges = test_only_item_line_ranges(source);

    code_only_source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let number = index + 1;
            (!test_only_ranges
                .iter()
                .any(|(start, end)| (*start..=*end).contains(&number)))
            .then(|| SourceLine {
                number,
                text: line.to_string(),
            })
        })
        .collect()
}

fn assert_no_production_callable_reference_named_in_paths(
    message: &str,
    roots: &[&str],
    callable_name: &str,
) {
    let mut violations = Vec::new();
    for root in roots {
        for path in rust_files_under(&repo_root().join(root)) {
            if is_test_only_path(&path) {
                continue;
            }
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            for line in production_callable_reference_lines(&source, callable_name) {
                violations.push(format!("{}:{line}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "{message}\n{}",
        violations.join("\n")
    );
}

fn production_callable_reference_lines(source: &str, callable_name: &str) -> Vec<usize> {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("architecture source must parse as Rust: {error}"));
    let mut visitor = ProductionCallableReferenceVisitor {
        callable_name,
        lines: Vec::new(),
    };
    visitor.visit_file(&syntax);
    visitor.lines.sort_unstable();
    visitor.lines.dedup();
    visitor.lines
}

struct ProductionCallableReferenceVisitor<'a> {
    callable_name: &'a str,
    lines: Vec<usize>,
}

impl<'ast> Visit<'ast> for ProductionCallableReferenceVisitor<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if impl_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if trait_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_trait_item(self, item);
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        if foreign_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_foreign_item(self, item);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == self.callable_name {
            self.lines.push(call.method.span().start().line);
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == self.callable_name)
        {
            self.lines.push(path.path.span().start().line);
        }
        visit::visit_expr_path(self, path);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        collect_named_macro_token_lines(&mac.tokens, self.callable_name, &mut self.lines);
        visit::visit_macro(self, mac);
    }
}

fn collect_named_macro_token_lines(
    tokens: &TokenStream,
    callable_name: &str,
    lines: &mut Vec<usize>,
) {
    for token in tokens.clone() {
        match token {
            TokenTree::Ident(identifier) if identifier == callable_name => {
                lines.push(identifier.span().start().line);
            }
            TokenTree::Group(group) => {
                collect_named_macro_token_lines(&group.stream(), callable_name, lines);
            }
            TokenTree::Ident(_) | TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
}

fn renderer_production_boundary_violations(source: &str) -> Vec<(usize, String)> {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("renderer architecture source must parse as Rust: {error}"));
    let mut visitor = RendererProductionBoundaryVisitor::default();
    visitor.visit_file(&syntax);
    visitor
        .violations
        .sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    visitor.violations.dedup();
    visitor.violations
}

#[derive(Default)]
struct RendererProductionBoundaryVisitor {
    violations: Vec<(usize, String)>,
}

impl RendererProductionBoundaryVisitor {
    fn record(&mut self, line: usize, message: impl Into<String>) {
        self.violations.push((line, message.into()));
    }

    fn inspect_signature(&mut self, signature: &syn::Signature) {
        for input in &signature.inputs {
            match input {
                syn::FnArg::Receiver(receiver)
                    if receiver.reference.is_some() && receiver.mutability.is_some() =>
                {
                    self.record(
                        receiver.span().start().line,
                        format!(
                            "{} accepts mutable self; renderer state must stay local to the call",
                            signature.ident
                        ),
                    );
                }
                syn::FnArg::Typed(argument) => {
                    let mut mutable_inputs = RendererMutableInputVisitor {
                        function_name: signature.ident.to_string(),
                        violations: &mut self.violations,
                    };
                    mutable_inputs.visit_type(&argument.ty);
                }
                syn::FnArg::Receiver(_) => {}
            }
        }
    }

    fn inspect_use_tree(&mut self, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(path) => {
                self.inspect_identifier(&path.ident);
                self.inspect_use_tree(&path.tree);
            }
            syn::UseTree::Name(name) => self.inspect_identifier(&name.ident),
            syn::UseTree::Rename(rename) => {
                self.inspect_identifier(&rename.ident);
                self.inspect_identifier(&rename.rename);
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.inspect_use_tree(item);
                }
            }
            syn::UseTree::Glob(glob) => self.record(
                glob.star_token.span.start().line,
                "production renderer imports must be explicit; glob import found",
            ),
        }
    }

    fn inspect_identifier(&mut self, identifier: &syn::Ident) {
        let identifier_text = identifier.to_string();
        if renderer_identifier_is_forbidden(&identifier_text) {
            self.record(
                identifier.span().start().line,
                format!("renderer depends on forbidden authority/effect type `{identifier_text}`"),
            );
        }
    }

    fn inspect_macro_tokens(&mut self, tokens: &TokenStream) {
        for token in tokens.clone() {
            match token {
                TokenTree::Ident(identifier) => {
                    let identifier_text = identifier.to_string();
                    if renderer_identifier_is_forbidden(&identifier_text)
                        || renderer_callable_is_forbidden(&identifier_text)
                        || renderer_field_is_forbidden(&identifier_text)
                    {
                        self.record(
                            identifier.span().start().line,
                            format!(
                                "renderer macro references forbidden authority/effect identifier `{identifier_text}`"
                            ),
                        );
                    }
                }
                TokenTree::Group(group) => self.inspect_macro_tokens(&group.stream()),
                TokenTree::Punct(_) | TokenTree::Literal(_) => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for RendererProductionBoundaryVisitor {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if item_is_test_only(item) {
            return;
        }
        match item {
            syn::Item::Fn(function) => self.inspect_signature(&function.sig),
            syn::Item::Use(item_use) => self.inspect_use_tree(&item_use.tree),
            _ => {}
        }
        visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if impl_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        if let syn::ImplItem::Fn(function) = item {
            self.inspect_signature(&function.sig);
        }
        visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if trait_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        if let syn::TraitItem::Fn(function) = item {
            self.inspect_signature(&function.sig);
        }
        visit::visit_trait_item(self, item);
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        if foreign_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        visit::visit_foreign_item(self, item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        for segment in &path.segments {
            self.inspect_identifier(&segment.ident);
        }
        visit::visit_path(self, path);
    }

    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if let syn::Member::Named(identifier) = &field.member {
            let identifier_text = identifier.to_string();
            if renderer_field_is_forbidden(&identifier_text) {
                self.record(
                    identifier.span().start().line,
                    format!("renderer reads forbidden authority field `{identifier_text}`"),
                );
            }
        }
        visit::visit_expr_field(self, field);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if renderer_callable_is_forbidden(&method) {
            self.record(
                call.method.span().start().line,
                format!("renderer calls forbidden authority/effect method `{method}`"),
            );
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.inspect_macro_tokens(&mac.tokens);
        visit::visit_macro(self, mac);
    }
}

struct RendererMutableInputVisitor<'a> {
    function_name: String,
    violations: &'a mut Vec<(usize, String)>,
}

impl<'ast> Visit<'ast> for RendererMutableInputVisitor<'_> {
    fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
        if reference.mutability.is_some() && !is_renderer_frame_type(&reference.elem) {
            self.violations.push((
                reference.span().start().line,
                format!(
                    "{} accepts a mutable reference other than &mut Frame",
                    self.function_name
                ),
            ));
        }
        visit::visit_type_reference(self, reference);
    }
}

fn is_renderer_frame_type(ty: &syn::Type) -> bool {
    matches!(
        ty,
        syn::Type::Path(type_path)
            if type_path.qself.is_none()
                && type_path
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "Frame")
    )
}

fn renderer_identifier_is_forbidden(identifier: &str) -> bool {
    matches!(
        identifier,
        "NativeTuiApp"
            | "NativeTuiShellState"
            | "NativeTuiConversationState"
            | "NativeTuiPlanningState"
            | "NativeTuiRuntimeState"
            | "CorePlanningWorkerPanelProjection"
            | "NativeTuiApplicationHandle"
            | "NativeClientRuntime"
            | "AppState"
            | "AppSnapshot"
            | "AppCommand"
            | "AppEvent"
            | "CoreController"
            | "CoreDispatchOutcome"
            | "CoreInput"
            | "CoreRuntime"
            | "SystemTime"
            | "Instant"
            | "Cell"
            | "RefCell"
            | "Mutex"
            | "RwLock"
            | "File"
            | "OpenOptions"
            | "Command"
            | "Terminal"
            | "TcpListener"
            | "TcpStream"
            | "UdpSocket"
    ) || identifier.contains("ControlPlane")
        || identifier.ends_with("Service")
        || identifier.ends_with("Port")
        || identifier.ends_with("Repository")
        || identifier.ends_with("Handle")
        || identifier.ends_with("Runtime")
        || identifier.ends_with("Snapshot")
}

fn renderer_field_is_forbidden(field: &str) -> bool {
    matches!(
        field,
        "application" | "client_runtime" | "core_runtime" | "parallel_mode_control_plane"
    )
}

fn renderer_callable_is_forbidden(callable: &str) -> bool {
    matches!(
        callable,
        "block_on"
            | "dispatch_client_event"
            | "dispatch_core_command"
            | "lock"
            | "planning_runtime_projection_snapshot"
            | "poll_pending_client_event"
            | "presentation_projection"
            | "recv"
            | "revisioned_planning_parallel_projection"
            | "snapshot"
            | "spawn"
            | "spawn_blocking"
            | "try_recv"
    )
}

const CORE_CONTROLLER_SLICE_CONTRACTS: &[(&str, &str)] = &[
    ("state", "AppState"),
    ("startup", "StartupFeatureReducer"),
    ("session_feature", "SessionFeatureReducer"),
    ("conversation_turn", "ConversationTurnFeatureReducer"),
    ("read_models", "ReadModelFeatureReducer"),
    ("planning", "PlanningFeatureReducer"),
    ("github_review", "GithubReviewFeatureReducer"),
];

const CORE_FEATURE_REDUCER_CONTRACTS: &[(&str, &str, &str)] = &[
    (
        "startup_reducer",
        "src/core/app/startup_reducer.rs",
        "StartupFeatureReducer",
    ),
    (
        "session_reducer",
        "src/core/app/session_reducer.rs",
        "SessionFeatureReducer",
    ),
    (
        "conversation_turn_reducer",
        "src/core/app/conversation_turn_reducer.rs",
        "ConversationTurnFeatureReducer",
    ),
    (
        "read_model_reducer",
        "src/core/app/read_model_reducer.rs",
        "ReadModelFeatureReducer",
    ),
    (
        "planning_reducer",
        "src/core/app/planning_reducer.rs",
        "PlanningFeatureReducer",
    ),
    (
        "github_review_reducer",
        "src/core/app/github_review_reducer.rs",
        "GithubReviewFeatureReducer",
    ),
];

fn core_feature_reducer_sources() -> HashMap<String, String> {
    CORE_FEATURE_REDUCER_CONTRACTS
        .iter()
        .map(|(module_name, path, _)| {
            (
                (*module_name).to_string(),
                fs::read_to_string(path)
                    .unwrap_or_else(|error| panic!("{path} should load: {error}")),
            )
        })
        .collect()
}

fn verify_core_feature_reducer_contract(
    controller_source: &str,
    app_module_source: &str,
    reducer_sources: &HashMap<String, String>,
) -> Result<(), String> {
    let controller_syntax = syn::parse_file(controller_source)
        .map_err(|error| format!("CoreController source must parse as Rust: {error}"))?;
    let controller = controller_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item)
                if item.ident == "CoreController" && !attributes_are_test_only(&item.attrs) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [controller] = controller.as_slice() else {
        return Err(format!(
            "expected one production CoreController, found {}",
            controller.len()
        ));
    };
    let syn::Fields::Named(controller_fields) = &controller.fields else {
        return Err("CoreController must use named fields".to_string());
    };
    let actual_fields = controller_fields
        .named
        .iter()
        .map(|field| {
            let name = field
                .ident
                .as_ref()
                .ok_or_else(|| "CoreController field must be named".to_string())?
                .to_string();
            let type_name = simple_unqualified_type_name(&field.ty).ok_or_else(|| {
                format!("CoreController.{name} must use one unqualified typed slice")
            })?;
            if !matches!(field.vis, syn::Visibility::Inherited) {
                return Err(format!(
                    "CoreController.{name} must remain a private slice field"
                ));
            }
            Ok((name, type_name))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let expected_fields = CORE_CONTROLLER_SLICE_CONTRACTS
        .iter()
        .map(|(name, type_name)| ((*name).to_string(), (*type_name).to_string()))
        .collect::<Vec<_>>();
    if actual_fields != expected_fields {
        return Err(format!(
            "CoreController must own exactly the seven typed private slices; expected={expected_fields:?}, actual={actual_fields:?}"
        ));
    }

    let app_module_syntax = syn::parse_file(app_module_source)
        .map_err(|error| format!("core app module source must parse as Rust: {error}"))?;
    for &(module_name, _, reducer_type) in CORE_FEATURE_REDUCER_CONTRACTS {
        let declarations = app_module_syntax
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Mod(item) if item.ident == module_name => Some(item),
                _ => None,
            })
            .collect::<Vec<_>>();
        let [declaration] = declarations.as_slice() else {
            return Err(format!(
                "core app must declare exactly one private {module_name} module"
            ));
        };
        if !matches!(declaration.vis, syn::Visibility::Inherited) {
            return Err(format!(
                "mutable reducer module {module_name} must remain private to core/app"
            ));
        }
        for item in &app_module_syntax.items {
            let syn::Item::Use(item_use) = item else {
                continue;
            };
            if matches!(item_use.vis, syn::Visibility::Inherited) {
                continue;
            }
            if use_tree_mentions_identifier(&item_use.tree, module_name)
                || use_tree_mentions_identifier(&item_use.tree, reducer_type)
            {
                return Err(format!(
                    "mutable reducer {reducer_type} must not be publicly re-exported"
                ));
            }
        }

        let source = reducer_sources
            .get(module_name)
            .ok_or_else(|| format!("missing reducer source for {module_name}"))?;
        let syntax = syn::parse_file(source)
            .map_err(|error| format!("{module_name} source must parse as Rust: {error}"))?;
        let reducers = syntax
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Struct(item)
                    if item.ident == reducer_type && !attributes_are_test_only(&item.attrs) =>
                {
                    Some(item)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let [reducer] = reducers.as_slice() else {
            return Err(format!(
                "{module_name} must define exactly one production {reducer_type}"
            ));
        };
        let syn::Fields::Named(fields) = &reducer.fields else {
            return Err(format!("{reducer_type} must use named authority fields"));
        };
        if fields.named.is_empty() {
            return Err(format!(
                "{reducer_type} must own at least one typed authority field"
            ));
        }
        for field in &fields.named {
            if !matches!(field.vis, syn::Visibility::Inherited) {
                let field_name = field
                    .ident
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "<unnamed>".to_string());
                return Err(format!(
                    "{reducer_type} authority fields must remain private; found visible {field_name}"
                ));
            }
        }

        let mut references = RustSemanticReferenceVisitor::default();
        references.visit_file(&syntax);
        expand_semantic_alias_paths(&mut references.references.paths, &references.aliases);
        for reference in references.references.paths {
            let identifier = reference.rsplit("::").next().unwrap_or(reference.as_str());
            let cross_reducer = CORE_FEATURE_REDUCER_CONTRACTS
                .iter()
                .any(|(_, _, other_type)| *other_type != reducer_type && *other_type == identifier);
            let forbidden_type = matches!(
                identifier,
                "AppState" | "AppEvent" | "CoreEffect" | "CoreDispatchOutcome"
            );
            let forbidden_layer = crate_reference_matches_prefix(&reference, "crate::adapter")
                || crate_reference_matches_prefix(&reference, "crate::application");
            if forbidden_type || forbidden_layer || cross_reducer {
                return Err(format!(
                    "{reducer_type} has forbidden dependency `{reference}`; reducers must return typed local reductions"
                ));
            }
        }

        for item in &syntax.items {
            let syn::Item::Impl(item_impl) = item else {
                continue;
            };
            if attributes_are_test_only(&item_impl.attrs)
                || !type_path_ends_with_ident(item_impl.self_ty.as_ref(), reducer_type)
            {
                continue;
            }
            if item_impl.trait_.as_ref().is_some_and(|(_, path, _)| {
                path.segments
                    .last()
                    .is_some_and(|segment| segment.ident == "Deref" || segment.ident == "DerefMut")
            }) {
                return Err(format!(
                    "{reducer_type} must not expose raw authority through Deref/DerefMut"
                ));
            }
            for item in &item_impl.items {
                let syn::ImplItem::Fn(method) = item else {
                    continue;
                };
                if attributes_are_test_only(&method.attrs) {
                    continue;
                }
                if return_type_contains_mutable_reference(&method.sig.output) {
                    return Err(format!(
                        "{reducer_type}::{} must not return mutable raw authority",
                        method.sig.ident
                    ));
                }
            }
        }
    }
    Ok(())
}

fn simple_unqualified_type_name(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(type_path) = ty else {
        return None;
    };
    if type_path.qself.is_some()
        || type_path.path.leading_colon.is_some()
        || type_path.path.segments.len() != 1
        || !matches!(
            type_path.path.segments[0].arguments,
            syn::PathArguments::None
        )
    {
        return None;
    }
    Some(type_path.path.segments[0].ident.to_string())
}

fn first_named_struct_field_name(source: &str, struct_name: &str) -> Option<String> {
    let syntax = syn::parse_file(source).ok()?;
    let item = syntax.items.iter().find_map(|item| match item {
        syn::Item::Struct(item) if item.ident == struct_name => Some(item),
        _ => None,
    })?;
    let syn::Fields::Named(fields) = &item.fields else {
        return None;
    };
    fields
        .named
        .first()?
        .ident
        .as_ref()
        .map(ToString::to_string)
}

fn use_tree_mentions_identifier(tree: &syn::UseTree, expected: &str) -> bool {
    match tree {
        syn::UseTree::Path(path) => {
            path.ident == expected || use_tree_mentions_identifier(&path.tree, expected)
        }
        syn::UseTree::Name(name) => name.ident == expected,
        syn::UseTree::Rename(rename) => rename.ident == expected || rename.rename == expected,
        syn::UseTree::Group(group) => group
            .items
            .iter()
            .any(|tree| use_tree_mentions_identifier(tree, expected)),
        syn::UseTree::Glob(_) => false,
    }
}

fn use_tree_is_simple_path(tree: &syn::UseTree, expected: &[&str]) -> bool {
    let Some((head, tail)) = expected.split_first() else {
        return false;
    };
    match tree {
        syn::UseTree::Path(path) => {
            path.ident == *head && use_tree_is_simple_path(&path.tree, tail)
        }
        syn::UseTree::Name(name) => tail.is_empty() && name.ident == *head,
        _ => false,
    }
}

fn type_path_ends_with_ident(ty: &syn::Type, expected: &str) -> bool {
    matches!(
        ty,
        syn::Type::Path(type_path)
            if type_path.qself.is_none()
                && type_path
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == expected)
    )
}

fn type_is_single_generic_path(ty: &syn::Type, outer: &str, inner: &str) -> bool {
    let syn::Type::Path(type_path) = ty else {
        return false;
    };
    if type_path.qself.is_some()
        || type_path.path.leading_colon.is_some()
        || type_path.path.segments.len() != 1
    {
        return false;
    }
    let segment = &type_path.path.segments[0];
    if segment.ident != outer {
        return false;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    matches!(
        arguments.args.iter().collect::<Vec<_>>().as_slice(),
        [syn::GenericArgument::Type(argument)] if type_is_simple_path(argument, &[inner])
    )
}

fn return_type_contains_mutable_reference(output: &syn::ReturnType) -> bool {
    struct MutableReferenceVisitor {
        found: bool,
    }
    impl<'ast> Visit<'ast> for MutableReferenceVisitor {
        fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
            if reference.mutability.is_some() {
                self.found = true;
                return;
            }
            visit::visit_type_reference(self, reference);
        }
    }
    let syn::ReturnType::Type(_, ty) = output else {
        return false;
    };
    let mut visitor = MutableReferenceVisitor { found: false };
    visitor.visit_type(ty);
    visitor.found
}

fn verify_turn_stream_authority_match_contract(
    update_source: &str,
    reducer_source: &str,
) -> Result<(), String> {
    let update_syntax = syn::parse_file(update_source)
        .map_err(|error| format!("TurnStreamUpdate source must parse as Rust: {error}"))?;
    let reducer_syntax = syn::parse_file(reducer_source)
        .map_err(|error| format!("conversation/turn reducer source must parse as Rust: {error}"))?;
    let update_enums = update_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Enum(item)
                if item.ident == "TurnStreamUpdate" && !attributes_are_test_only(&item.attrs) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [updates] = update_enums.as_slice() else {
        return Err(format!(
            "expected one production TurnStreamUpdate enum, found {}",
            update_enums.len()
        ));
    };
    let expected = updates
        .variants
        .iter()
        .map(|variant| variant.ident.to_string())
        .collect::<HashSet<_>>();

    let methods = inherent_impl_methods(
        &reducer_syntax,
        "ConversationTurnFeatureReducer",
        "apply_correlated_turn_stream_event",
    );
    let [method] = methods.as_slice() else {
        return Err(format!(
            "expected one ConversationTurnFeatureReducer::apply_correlated_turn_stream_event, found {}",
            methods.len()
        ));
    };
    let authority_matches = method
        .block
        .stmts
        .iter()
        .filter_map(|statement| match statement {
            syn::Stmt::Expr(syn::Expr::Match(expression), None)
                if expression_is_shared_stream_update(&expression.expr) =>
            {
                Some(expression)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [authority_match] = authority_matches.as_slice() else {
        return Err(format!(
            "expected one direct `match &stream_snapshot.update` authority transition, found {}",
            authority_matches.len()
        ));
    };
    let mut actual = HashSet::new();
    for arm in &authority_match.arms {
        if arm.guard.is_some() {
            return Err(format!(
                "TurnStreamUpdate authority arm at line {} must not use a match guard",
                arm.span().start().line
            ));
        }
        for variant in exact_enum_pattern_variants(&arm.pat, "TurnStreamUpdate")? {
            if !actual.insert(variant.clone()) {
                return Err(format!(
                    "TurnStreamUpdate::{variant} must appear in exactly one authority arm"
                ));
            }
        }
    }
    if actual != expected {
        return Err(format!(
            "TurnStreamUpdate authority match must exactly cover the enum ({})",
            core_effect_set_difference(&actual, &expected)
        ));
    }
    Ok(())
}

fn expression_is_shared_stream_update(expression: &syn::Expr) -> bool {
    matches!(
        expression,
        syn::Expr::Reference(reference)
            if reference.mutability.is_none()
                && expression_is_field_path(
                    reference.expr.as_ref(),
                    "stream_snapshot",
                    "update",
                )
    )
}

fn exact_enum_pattern_variants(pattern: &syn::Pat, enum_name: &str) -> Result<Vec<String>, String> {
    match pattern {
        syn::Pat::Or(pattern) => {
            let mut variants = Vec::new();
            for case in &pattern.cases {
                variants.extend(exact_enum_pattern_variants(case, enum_name)?);
            }
            Ok(variants)
        }
        syn::Pat::Wild(_) => Err("wildcard patterns are forbidden".to_string()),
        syn::Pat::Struct(pattern) => {
            exact_two_segment_enum_variant(&pattern.path, enum_name).map(|variant| vec![variant])
        }
        syn::Pat::TupleStruct(pattern) => {
            exact_two_segment_enum_variant(&pattern.path, enum_name).map(|variant| vec![variant])
        }
        syn::Pat::Path(pattern) if pattern.qself.is_none() => {
            exact_two_segment_enum_variant(&pattern.path, enum_name).map(|variant| vec![variant])
        }
        _ => Err(format!(
            "{enum_name} authority arms may only use exact variant or or-patterns"
        )),
    }
}

fn exact_two_segment_enum_variant(path: &syn::Path, enum_name: &str) -> Result<String, String> {
    if path.leading_colon.is_some() || path.segments.len() != 2 {
        return Err(format!(
            "pattern path must be exactly {enum_name}::<Variant>"
        ));
    }
    let mut segments = path.segments.iter();
    let owner = segments.next().expect("path length checked");
    let variant = segments.next().expect("path length checked");
    if owner.ident != enum_name
        || !matches!(owner.arguments, syn::PathArguments::None)
        || !matches!(variant.arguments, syn::PathArguments::None)
    {
        return Err(format!(
            "pattern path must be exactly {enum_name}::<Variant>"
        ));
    }
    Ok(variant.ident.to_string())
}

#[derive(Debug)]
struct RuntimeWriterFinding {
    owner: String,
    line: usize,
    detail: String,
}

impl std::fmt::Display for RuntimeWriterFinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.owner, self.detail)
    }
}

#[derive(Default)]
struct ConversationRuntimeWriterAudit {
    semantic_writes: Vec<RuntimeWriterFinding>,
    whole_replacements: Vec<RuntimeWriterFinding>,
    replacement_calls: Vec<RuntimeWriterFinding>,
    snapshot_literals: Vec<RuntimeWriterFinding>,
}

fn conversation_runtime_writer_audit(
    source: &str,
) -> Result<ConversationRuntimeWriterAudit, String> {
    let syntax = syn::parse_file(source)
        .map_err(|error| format!("conversation runtime writer source must parse: {error}"))?;
    let mut visitor = ConversationRuntimeWriterVisitor::default();
    visitor.visit_file(&syntax);
    Ok(visitor.audit)
}

#[derive(Default)]
struct ConversationRuntimeWriterVisitor {
    owner: Option<String>,
    audit: ConversationRuntimeWriterAudit,
}

impl ConversationRuntimeWriterVisitor {
    fn finding(&self, line: usize, detail: impl Into<String>) -> RuntimeWriterFinding {
        RuntimeWriterFinding {
            owner: self.owner.clone().unwrap_or_else(|| "<module>".to_string()),
            line,
            detail: detail.into(),
        }
    }

    fn inspect_write_target(&mut self, expression: &syn::Expr, kind: &str) {
        let fields = expression_field_chain(expression);
        if let Some(field) = fields
            .iter()
            .find(|field| conversation_runtime_semantic_field(field))
        {
            self.audit.semantic_writes.push(self.finding(
                expression.span().start().line,
                format!("{kind} reaches semantic field `{field}`"),
            ));
        }
        if fields
            .last()
            .is_some_and(|field| field == "runtime_snapshot")
        {
            self.audit.whole_replacements.push(self.finding(
                expression.span().start().line,
                format!("{kind} replaces the whole runtime snapshot"),
            ));
        }
    }
}

impl<'ast> Visit<'ast> for ConversationRuntimeWriterVisitor {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if item_is_test_only(item) {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        if attributes_are_test_only(&function.attrs) {
            return;
        }
        let previous = self.owner.replace(function.sig.ident.to_string());
        visit::visit_item_fn(self, function);
        self.owner = previous;
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if impl_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        let syn::ImplItem::Fn(function) = item else {
            visit::visit_impl_item(self, item);
            return;
        };
        let previous = self.owner.replace(function.sig.ident.to_string());
        visit::visit_impl_item_fn(self, function);
        self.owner = previous;
    }

    fn visit_expr_assign(&mut self, expression: &'ast syn::ExprAssign) {
        self.inspect_write_target(expression.left.as_ref(), "assignment");
        visit::visit_expr_assign(self, expression);
    }

    fn visit_expr_binary(&mut self, expression: &'ast syn::ExprBinary) {
        if matches!(
            expression.op,
            syn::BinOp::AddAssign(_)
                | syn::BinOp::SubAssign(_)
                | syn::BinOp::MulAssign(_)
                | syn::BinOp::DivAssign(_)
                | syn::BinOp::RemAssign(_)
                | syn::BinOp::BitXorAssign(_)
                | syn::BinOp::BitAndAssign(_)
                | syn::BinOp::BitOrAssign(_)
                | syn::BinOp::ShlAssign(_)
                | syn::BinOp::ShrAssign(_)
        ) {
            self.inspect_write_target(expression.left.as_ref(), "compound assignment");
        }
        visit::visit_expr_binary(self, expression);
    }

    fn visit_expr_reference(&mut self, expression: &'ast syn::ExprReference) {
        if expression.mutability.is_some() {
            self.inspect_write_target(expression.expr.as_ref(), "mutable borrow");
        }
        visit::visit_expr_reference(self, expression);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "apply_runtime_snapshot" {
            self.audit.replacement_calls.push(self.finding(
                call.method.span().start().line,
                "calls the whole runtime snapshot replacement path",
            ));
        }
        if matches!(
            call.method.to_string().as_str(),
            "as_mut" | "borrow_mut" | "get_mut" | "replace" | "take"
        ) && expression_field_chain(call.receiver.as_ref())
            .iter()
            .any(|field| conversation_runtime_semantic_field(field))
        {
            self.audit.semantic_writes.push(self.finding(
                call.method.span().start().line,
                format!(
                    "mutable accessor `{}` reaches a semantic runtime field",
                    call.method
                ),
            ));
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
        if expression
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "ConversationRuntimeSnapshot")
        {
            self.audit.snapshot_literals.push(self.finding(
                expression.path.span().start().line,
                "constructs ConversationRuntimeSnapshot in the TUI",
            ));
        }
        visit::visit_expr_struct(self, expression);
    }
}

fn expression_field_chain(expression: &syn::Expr) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = expression;
    while let syn::Expr::Field(field) = current {
        if let syn::Member::Named(member) = &field.member {
            fields.push(member.to_string());
        }
        current = field.base.as_ref();
    }
    fields.reverse();
    fields
}

fn conversation_runtime_semantic_field(field: &str) -> bool {
    matches!(
        field,
        "active_turn"
            | "approval"
            | "approval_review"
            | "auto_follow"
            | "post_turn"
            | "planning_handoff"
    )
}

const SHELL_CHROME_REDUCER_FIELDS: &[&str] = &[
    "shell_overlay",
    "approval_return_overlay",
    "exit_confirmation_state",
    "startup_state",
    "session_state",
    "selected_session_index",
];

const SHELL_CHROME_READ_ONLY_METHODS: &[&str] = &["clone", "prompt_input_has_focus"];

const SHELL_CHROME_AUTHORITY_READ_MACROS: &[&str] = &[
    "akra_event",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "format",
    "format_args",
    "matches",
    "panic",
    "unreachable",
];

const SHELL_CHROME_MACRO_MUTATION_IDENTIFIERS: &[&str] = &[
    "as_mut",
    "borrow_mut",
    "clear",
    "get_mut",
    "get_or_insert",
    "get_or_insert_default",
    "get_or_insert_with",
    "insert",
    "mut",
    "push",
    "remove",
    "replace",
    "take",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellAuthorityKind {
    Unknown,
    App,
    Shell,
    Chrome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShellAuthorityBinding {
    kind: ShellAuthorityKind,
    mutable: bool,
}

#[derive(Clone)]
struct ShellTypeBinding {
    ty: syn::Type,
    mutable: bool,
}

struct ShellResolvedStructType {
    name: String,
    arguments: syn::PathArguments,
}

impl ShellResolvedStructType {
    fn visit_key(&self) -> String {
        format!(
            "{}{}",
            self.name,
            shell_path_arguments_identity(&self.arguments)
        )
    }
}

fn shell_path_arguments_identity(arguments: &syn::PathArguments) -> String {
    match arguments {
        syn::PathArguments::None => String::new(),
        syn::PathArguments::AngleBracketed(arguments) => {
            let arguments = arguments
                .args
                .iter()
                .map(|argument| match argument {
                    syn::GenericArgument::Lifetime(lifetime) => {
                        format!("'{}", lifetime.ident)
                    }
                    syn::GenericArgument::Type(ty) => shell_type_identity(ty),
                    syn::GenericArgument::Const(_) => "const".to_string(),
                    syn::GenericArgument::AssocType(association) => {
                        format!(
                            "{}={}",
                            association.ident,
                            shell_type_identity(&association.ty)
                        )
                    }
                    syn::GenericArgument::AssocConst(association) => {
                        format!("{}=const", association.ident)
                    }
                    syn::GenericArgument::Constraint(constraint) => {
                        format!("{}:constraint", constraint.ident)
                    }
                    _ => "argument".to_string(),
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("<{arguments}>")
        }
        syn::PathArguments::Parenthesized(arguments) => {
            let inputs = arguments
                .inputs
                .iter()
                .map(shell_type_identity)
                .collect::<Vec<_>>()
                .join(",");
            format!("({inputs})")
        }
    }
}

fn shell_type_identity(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(path) if path.qself.is_none() => path
            .path
            .segments
            .iter()
            .map(|segment| {
                format!(
                    "{}{}",
                    segment.ident,
                    shell_path_arguments_identity(&segment.arguments)
                )
            })
            .collect::<Vec<_>>()
            .join("::"),
        syn::Type::Reference(reference) => format!(
            "&{}{}",
            if reference.mutability.is_some() {
                "mut "
            } else {
                ""
            },
            shell_type_identity(reference.elem.as_ref())
        ),
        syn::Type::Ptr(pointer) => format!(
            "*{} {}",
            if pointer.mutability.is_some() {
                "mut"
            } else {
                "const"
            },
            shell_type_identity(pointer.elem.as_ref())
        ),
        syn::Type::Group(group) => shell_type_identity(group.elem.as_ref()),
        syn::Type::Paren(paren) => format!("({})", shell_type_identity(paren.elem.as_ref())),
        syn::Type::Slice(slice) => format!("[{}]", shell_type_identity(slice.elem.as_ref())),
        syn::Type::Array(array) => format!("[{};_]", shell_type_identity(array.elem.as_ref())),
        syn::Type::Tuple(tuple) => format!(
            "({})",
            tuple
                .elems
                .iter()
                .map(shell_type_identity)
                .collect::<Vec<_>>()
                .join(",")
        ),
        syn::Type::Never(_) => "!".to_string(),
        syn::Type::Infer(_) => "_".to_string(),
        _ => "type".to_string(),
    }
}

#[derive(Clone, Copy)]
struct ShellTypeResolutionScope<'a> {
    implicit_self: Option<ShellAuthorityKind>,
    type_aliases: &'a ShellTypeAliases,
    qualified_type_aliases: &'a ShellQualifiedTypeAliases,
    imports: &'a ShellStructImports,
    absolute_imports: &'a HashSet<String>,
    path_shadows: &'a HashSet<String>,
    module_path: &'a [String],
    allow_local_aliases: bool,
    allow_local_imports: bool,
}

fn normalized_shell_type_path(
    path: &syn::Path,
    scope: ShellTypeResolutionScope<'_>,
) -> Vec<String> {
    let mut absolute = path.leading_colon.is_some();
    let mut raw_path = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    if scope.allow_local_imports && path.leading_colon.is_none() {
        let mut resolving_imports = HashSet::new();
        while let Some(first) = raw_path.first().cloned()
            && let Some(target) = scope.imports.get(&first)
            && resolving_imports.insert(first.clone())
        {
            absolute |= scope.absolute_imports.contains(&first);
            let mut expanded = target.clone();
            expanded.extend(raw_path.iter().skip(1).cloned());
            raw_path = expanded;
        }
    }
    if raw_path.len() <= 1 {
        return raw_path;
    }

    let mut normalized = scope.module_path.to_vec();
    let mut index = 0;
    let first_is_visible_crate_alias = raw_path.first().is_some_and(|segment| {
        scope.qualified_type_aliases.is_crate_alias(segment)
            && (absolute || !scope.path_shadows.contains(segment))
    });
    if raw_path.first().is_some_and(|segment| segment == "crate") || first_is_visible_crate_alias {
        normalized.clear();
        normalized.push("crate".to_string());
        index = 1;
    } else if raw_path.first().is_some_and(|segment| segment == "self") {
        index = 1;
    }
    while raw_path
        .get(index)
        .is_some_and(|segment| segment == "super")
    {
        if normalized.len() > 1 {
            normalized.pop();
        }
        index += 1;
    }
    normalized.extend(raw_path[index..].iter().cloned());
    normalized
}

fn shell_authority_kind_from_path(
    path: &syn::Path,
    scope: ShellTypeResolutionScope<'_>,
) -> Option<ShellAuthorityKind> {
    let normalized = normalized_shell_type_path(path, scope);
    match normalized.as_slice() {
        [name] if name == "NativeTuiApp" => Some(ShellAuthorityKind::App),
        [name] if name == "NativeTuiShellState" => Some(ShellAuthorityKind::Shell),
        [name] if name == "ShellChromeState" => Some(ShellAuthorityKind::Chrome),
        [name] if name == "Self" => scope.implicit_self,
        [root, adapter, inbound, tui, app, name]
            if root == "crate"
                && adapter == "adapter"
                && inbound == "inbound"
                && tui == "tui"
                && app == "app"
                && name == "NativeTuiApp" =>
        {
            Some(ShellAuthorityKind::App)
        }
        [root, adapter, inbound, tui, app, name]
            if root == "crate"
                && adapter == "adapter"
                && inbound == "inbound"
                && tui == "tui"
                && app == "app"
                && name == "NativeTuiShellState" =>
        {
            Some(ShellAuthorityKind::Shell)
        }
        [root, adapter, inbound, tui, shell_chrome, name]
            if root == "crate"
                && adapter == "adapter"
                && inbound == "inbound"
                && tui == "tui"
                && shell_chrome == "shell_chrome"
                && name == "ShellChromeState" =>
        {
            Some(ShellAuthorityKind::Chrome)
        }
        _ => None,
    }
}

fn shell_type_is_bare_self(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Path(path) => {
            path.qself.is_none()
                && path.path.leading_colon.is_none()
                && path.path.segments.len() == 1
                && path.path.segments[0].ident == "Self"
        }
        syn::Type::Group(group) => shell_type_is_bare_self(group.elem.as_ref()),
        syn::Type::Paren(paren) => shell_type_is_bare_self(paren.elem.as_ref()),
        _ => false,
    }
}

fn shell_type_path(ty: &syn::Type) -> Option<&syn::TypePath> {
    match ty {
        syn::Type::Path(path) if path.qself.is_none() => Some(path),
        syn::Type::Group(group) => shell_type_path(group.elem.as_ref()),
        syn::Type::Paren(paren) => shell_type_path(paren.elem.as_ref()),
        _ => None,
    }
}

fn shell_authority_binding_from_type(
    ty: &syn::Type,
    scope: ShellTypeResolutionScope<'_>,
    resolving_aliases: &mut HashSet<String>,
) -> Option<ShellAuthorityBinding> {
    match ty {
        syn::Type::Path(path) if path.qself.is_none() => {
            let segment = path.path.segments.last()?;
            let name = segment.ident.to_string();
            let normalized_path = if path.path.segments.len() == 1 {
                let mut qualified = scope.module_path.to_vec();
                qualified.push(name.clone());
                qualified
            } else {
                normalized_shell_type_path(&path.path, scope)
            };
            let qualified_name = normalized_path.join("::");
            let alias_module_path = normalized_path
                .get(..normalized_path.len().saturating_sub(1))
                .unwrap_or_default();
            let local_alias = (scope.allow_local_aliases
                && (path.path.segments.len() == 1 || alias_module_path == scope.module_path))
                .then(|| scope.type_aliases.get(&name))
                .flatten();
            let alias = local_alias.or_else(|| scope.qualified_type_aliases.get(&qualified_name));
            if let Some(alias) = alias {
                if !resolving_aliases.insert(qualified_name.clone()) {
                    return None;
                }
                let alias_module_path = if local_alias.is_some() {
                    scope.module_path
                } else {
                    alias_module_path
                };
                let alias_path_shadows = if local_alias.is_some() {
                    scope.path_shadows
                } else {
                    scope
                        .qualified_type_aliases
                        .path_shadows_for_module(alias_module_path)
                };
                let alias_imports = if local_alias.is_some() {
                    scope.imports
                } else {
                    scope
                        .qualified_type_aliases
                        .imports_for_module(alias_module_path)
                };
                let alias_absolute_imports = if local_alias.is_some() {
                    scope.absolute_imports
                } else {
                    scope
                        .qualified_type_aliases
                        .absolute_imports_for_module(alias_module_path)
                };
                let instantiated_alias = alias.instantiate(&segment.arguments);
                let alias_scope = ShellTypeResolutionScope {
                    module_path: alias_module_path,
                    imports: alias_imports,
                    absolute_imports: alias_absolute_imports,
                    path_shadows: alias_path_shadows,
                    allow_local_aliases: local_alias.is_some(),
                    allow_local_imports: true,
                    ..scope
                };
                let resolved = shell_authority_binding_from_type(
                    &instantiated_alias,
                    alias_scope,
                    resolving_aliases,
                );
                resolving_aliases.remove(&qualified_name);
                if resolved.is_some() {
                    return resolved;
                }
                let expanded_alias = shell_type_with_expanded_aliases(
                    &instantiated_alias,
                    alias_scope,
                    &mut HashSet::new(),
                );
                if !alias.forwards_arguments
                    || shell_alias_target_is_type(&instantiated_alias, alias_scope)
                    || shell_alias_target_is_type(&expanded_alias, alias_scope)
                {
                    return None;
                }
            }
            for prefix_len in (1..normalized_path.len()).rev() {
                let prefix = normalized_path[..prefix_len].join("::");
                let Some(alias) = scope.qualified_type_aliases.get(&prefix) else {
                    continue;
                };
                let instantiated_alias = alias.instantiate(&syn::PathArguments::None);
                let Some(alias_path) = shell_type_path(&instantiated_alias) else {
                    continue;
                };
                if !resolving_aliases.insert(prefix.clone()) {
                    return None;
                }
                let mut expanded_path = alias_path.clone();
                for segment in &normalized_path[prefix_len..] {
                    expanded_path
                        .path
                        .segments
                        .push(syn::PathSegment::from(syn::Ident::new(
                            segment,
                            proc_macro2::Span::call_site(),
                        )));
                }
                if let Some(expanded_segment) = expanded_path.path.segments.last_mut() {
                    expanded_segment.arguments = segment.arguments.clone();
                }
                let alias_module_path = &normalized_path[..prefix_len.saturating_sub(1)];
                let alias_path_shadows = scope
                    .qualified_type_aliases
                    .path_shadows_for_module(alias_module_path);
                let alias_imports = scope
                    .qualified_type_aliases
                    .imports_for_module(alias_module_path);
                let alias_absolute_imports = scope
                    .qualified_type_aliases
                    .absolute_imports_for_module(alias_module_path);
                let resolved = shell_authority_binding_from_type(
                    &syn::Type::Path(expanded_path),
                    ShellTypeResolutionScope {
                        module_path: alias_module_path,
                        imports: alias_imports,
                        absolute_imports: alias_absolute_imports,
                        path_shadows: alias_path_shadows,
                        allow_local_aliases: false,
                        allow_local_imports: true,
                        ..scope
                    },
                    resolving_aliases,
                );
                resolving_aliases.remove(&prefix);
                if resolved.is_some() {
                    return resolved;
                }
            }
            if !scope
                .qualified_type_aliases
                .explicitly_declares_type(&qualified_name)
            {
                for prefix_len in (1..normalized_path.len()).rev() {
                    let glob_module = normalized_path[..prefix_len].join("::");
                    let shadowed_item = format!("{glob_module}::{}", normalized_path[prefix_len]);
                    if scope
                        .qualified_type_aliases
                        .explicitly_declares_type(&shadowed_item)
                    {
                        continue;
                    }
                    for (target_index, target) in scope
                        .qualified_type_aliases
                        .glob_targets(&glob_module)
                        .iter()
                        .enumerate()
                    {
                        let Some(target_path) = shell_type_path(target) else {
                            continue;
                        };
                        let resolution_key = format!("{glob_module}::*#{target_index}");
                        if !resolving_aliases.insert(resolution_key.clone()) {
                            continue;
                        }
                        let mut expanded_path = target_path.clone();
                        for segment in &normalized_path[prefix_len..] {
                            expanded_path.path.segments.push(syn::PathSegment::from(
                                syn::Ident::new(segment, proc_macro2::Span::call_site()),
                            ));
                        }
                        if let Some(expanded_segment) = expanded_path.path.segments.last_mut() {
                            expanded_segment.arguments = segment.arguments.clone();
                        }
                        let glob_module_path = &normalized_path[..prefix_len];
                        let glob_path_shadows = scope
                            .qualified_type_aliases
                            .path_shadows_for_module(glob_module_path);
                        let glob_imports = scope
                            .qualified_type_aliases
                            .imports_for_module(glob_module_path);
                        let glob_absolute_imports = scope
                            .qualified_type_aliases
                            .absolute_imports_for_module(glob_module_path);
                        let resolved = shell_authority_binding_from_type(
                            &syn::Type::Path(expanded_path),
                            ShellTypeResolutionScope {
                                module_path: glob_module_path,
                                imports: glob_imports,
                                absolute_imports: glob_absolute_imports,
                                path_shadows: glob_path_shadows,
                                allow_local_aliases: false,
                                allow_local_imports: true,
                                ..scope
                            },
                            resolving_aliases,
                        );
                        resolving_aliases.remove(&resolution_key);
                        if resolved.is_some() {
                            return resolved;
                        }
                    }
                }
            }
            let direct_kind = shell_authority_kind_from_path(&path.path, scope);
            if let Some(kind) = direct_kind {
                return Some(ShellAuthorityBinding {
                    kind,
                    mutable: false,
                });
            }
            if matches!(
                name.as_str(),
                "Box" | "MutexGuard" | "Option" | "Pin" | "RefMut" | "Result" | "RwLockWriteGuard"
            ) && let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments
            {
                let wrapper_is_mutable =
                    matches!(name.as_str(), "MutexGuard" | "RefMut" | "RwLockWriteGuard");
                let priority = |authority: ShellAuthorityBinding| match (
                    authority.mutable,
                    matches!(
                        authority.kind,
                        ShellAuthorityKind::Shell | ShellAuthorityKind::Chrome
                    ),
                ) {
                    (true, true) => 4,
                    (true, false) => 3,
                    (false, true) => 2,
                    (false, false) => 1,
                };
                let mut preferred_authority = None;
                for argument in &arguments.args {
                    let syn::GenericArgument::Type(inner) = argument else {
                        continue;
                    };
                    let Some(authority) =
                        shell_authority_binding_from_type(inner, scope, resolving_aliases)
                    else {
                        continue;
                    };
                    let authority = ShellAuthorityBinding {
                        mutable: authority.mutable || wrapper_is_mutable,
                        ..authority
                    };
                    if preferred_authority
                        .is_none_or(|preferred| priority(authority) > priority(preferred))
                    {
                        preferred_authority = Some(authority);
                    }
                }
                return preferred_authority;
            }
            None
        }
        syn::Type::Reference(reference) => {
            let authority = shell_authority_binding_from_type(
                reference.elem.as_ref(),
                scope,
                resolving_aliases,
            )?;
            Some(ShellAuthorityBinding {
                mutable: reference.mutability.is_some(),
                ..authority
            })
        }
        syn::Type::Group(group) => {
            shell_authority_binding_from_type(group.elem.as_ref(), scope, resolving_aliases)
        }
        syn::Type::Paren(paren) => {
            shell_authority_binding_from_type(paren.elem.as_ref(), scope, resolving_aliases)
        }
        syn::Type::Ptr(pointer) => {
            let authority =
                shell_authority_binding_from_type(pointer.elem.as_ref(), scope, resolving_aliases)?;
            Some(ShellAuthorityBinding {
                mutable: pointer.mutability.is_some(),
                ..authority
            })
        }
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShellAliasNamespace {
    Type,
    Value,
}

fn shell_type_with_expanded_aliases(
    ty: &syn::Type,
    scope: ShellTypeResolutionScope<'_>,
    resolving_aliases: &mut HashSet<String>,
) -> syn::Type {
    shell_type_with_expanded_aliases_in_namespace(
        ty,
        scope,
        resolving_aliases,
        ShellAliasNamespace::Type,
    )
}

fn shell_value_path_with_expanded_aliases(
    ty: &syn::Type,
    scope: ShellTypeResolutionScope<'_>,
    resolving_aliases: &mut HashSet<String>,
) -> syn::Type {
    shell_type_with_expanded_aliases_in_namespace(
        ty,
        scope,
        resolving_aliases,
        ShellAliasNamespace::Value,
    )
}

fn shell_alias_target_is_type(ty: &syn::Type, scope: ShellTypeResolutionScope<'_>) -> bool {
    match ty {
        syn::Type::Path(path) if path.qself.is_none() => {
            let Some(segment) = path.path.segments.last() else {
                return false;
            };
            let name = segment.ident.to_string();
            let normalized = if path.path.segments.len() == 1 {
                let mut qualified = scope.module_path.to_vec();
                qualified.push(name.clone());
                qualified
            } else {
                normalized_shell_type_path(&path.path, scope)
            };
            let qualified_name = normalized.join("::");
            scope
                .qualified_type_aliases
                .explicitly_declares_type(&qualified_name)
                || scope
                    .qualified_type_aliases
                    .get(&qualified_name)
                    .is_some_and(|alias| !alias.forwards_arguments)
                || shell_authority_kind_from_path(&path.path, scope).is_some()
                || (path.path.leading_colon.is_none()
                    && path.path.segments.len() == 1
                    && matches!(
                        name.as_str(),
                        "Box"
                            | "HashMap"
                            | "HashSet"
                            | "MutexGuard"
                            | "Option"
                            | "Pin"
                            | "Rc"
                            | "RefMut"
                            | "Result"
                            | "RwLockWriteGuard"
                            | "String"
                            | "Vec"
                            | "VecDeque"
                            | "bool"
                            | "char"
                            | "f32"
                            | "f64"
                            | "i8"
                            | "i16"
                            | "i32"
                            | "i64"
                            | "i128"
                            | "isize"
                            | "str"
                            | "u8"
                            | "u16"
                            | "u32"
                            | "u64"
                            | "u128"
                            | "usize"
                    ))
        }
        syn::Type::Reference(reference) => {
            shell_alias_target_is_type(reference.elem.as_ref(), scope)
        }
        syn::Type::Ptr(pointer) => shell_alias_target_is_type(pointer.elem.as_ref(), scope),
        syn::Type::Group(group) => shell_alias_target_is_type(group.elem.as_ref(), scope),
        syn::Type::Paren(paren) => shell_alias_target_is_type(paren.elem.as_ref(), scope),
        syn::Type::Array(_)
        | syn::Type::BareFn(_)
        | syn::Type::ImplTrait(_)
        | syn::Type::Infer(_)
        | syn::Type::Never(_)
        | syn::Type::Slice(_)
        | syn::Type::TraitObject(_)
        | syn::Type::Tuple(_) => true,
        _ => false,
    }
}

fn shell_type_with_expanded_aliases_in_namespace(
    ty: &syn::Type,
    scope: ShellTypeResolutionScope<'_>,
    resolving_aliases: &mut HashSet<String>,
    namespace: ShellAliasNamespace,
) -> syn::Type {
    match ty {
        syn::Type::Path(path) if path.qself.is_none() => {
            let Some(segment) = path.path.segments.last() else {
                return ty.clone();
            };
            let name = segment.ident.to_string();
            let normalized_path = if path.path.segments.len() == 1 {
                let mut qualified = scope.module_path.to_vec();
                qualified.push(name.clone());
                qualified
            } else {
                normalized_shell_type_path(&path.path, scope)
            };
            let qualified_name = normalized_path.join("::");
            let alias_module_path = normalized_path
                .get(..normalized_path.len().saturating_sub(1))
                .unwrap_or_default();
            let local_alias = (scope.allow_local_aliases
                && (path.path.segments.len() == 1 || alias_module_path == scope.module_path))
                .then(|| scope.type_aliases.get(&name))
                .flatten();
            let alias = local_alias.or_else(|| scope.qualified_type_aliases.get(&qualified_name));
            if let Some(alias) = alias {
                if !resolving_aliases.insert(qualified_name.clone()) {
                    return ty.clone();
                }
                let declaration_module = if local_alias.is_some() {
                    scope.module_path.to_vec()
                } else {
                    alias_module_path.to_vec()
                };
                let declaration_imports = if local_alias.is_some() {
                    scope.imports
                } else {
                    scope
                        .qualified_type_aliases
                        .imports_for_module(&declaration_module)
                };
                let declaration_absolute_imports = if local_alias.is_some() {
                    scope.absolute_imports
                } else {
                    scope
                        .qualified_type_aliases
                        .absolute_imports_for_module(&declaration_module)
                };
                let declaration_path_shadows = if local_alias.is_some() {
                    scope.path_shadows
                } else {
                    scope
                        .qualified_type_aliases
                        .path_shadows_for_module(&declaration_module)
                };
                let instantiated = alias.instantiate(&segment.arguments);
                let declaration_scope = ShellTypeResolutionScope {
                    module_path: &declaration_module,
                    imports: declaration_imports,
                    absolute_imports: declaration_absolute_imports,
                    path_shadows: declaration_path_shadows,
                    allow_local_aliases: local_alias.is_some(),
                    allow_local_imports: true,
                    ..scope
                };
                let expanded = shell_type_with_expanded_aliases_in_namespace(
                    &instantiated,
                    declaration_scope,
                    resolving_aliases,
                    namespace,
                );
                resolving_aliases.remove(&qualified_name);
                let valid_namespace = namespace == ShellAliasNamespace::Value
                    || !alias.forwards_arguments
                    || shell_alias_target_is_type(&instantiated, declaration_scope)
                    || shell_alias_target_is_type(&expanded, declaration_scope);
                if valid_namespace {
                    return expanded;
                }
            }

            for prefix_len in (1..normalized_path.len()).rev() {
                let prefix = normalized_path[..prefix_len].join("::");
                let Some(alias) = scope.qualified_type_aliases.get(&prefix) else {
                    continue;
                };
                let instantiated = alias.instantiate(&syn::PathArguments::None);
                let Some(alias_path) = shell_type_path(&instantiated) else {
                    continue;
                };
                if !resolving_aliases.insert(prefix.clone()) {
                    return ty.clone();
                }
                let mut expanded_path = alias_path.clone();
                for segment in &normalized_path[prefix_len..] {
                    expanded_path
                        .path
                        .segments
                        .push(syn::PathSegment::from(syn::Ident::new(
                            segment,
                            proc_macro2::Span::call_site(),
                        )));
                }
                if let Some(expanded_segment) = expanded_path.path.segments.last_mut() {
                    expanded_segment.arguments = segment.arguments.clone();
                }
                let declaration_module = normalized_path[..prefix_len.saturating_sub(1)].to_vec();
                let expanded = shell_type_with_expanded_aliases_in_namespace(
                    &syn::Type::Path(expanded_path),
                    ShellTypeResolutionScope {
                        module_path: &declaration_module,
                        imports: scope
                            .qualified_type_aliases
                            .imports_for_module(&declaration_module),
                        absolute_imports: scope
                            .qualified_type_aliases
                            .absolute_imports_for_module(&declaration_module),
                        path_shadows: scope
                            .qualified_type_aliases
                            .path_shadows_for_module(&declaration_module),
                        allow_local_aliases: false,
                        allow_local_imports: true,
                        ..scope
                    },
                    resolving_aliases,
                    namespace,
                );
                resolving_aliases.remove(&prefix);
                return expanded;
            }

            if !scope
                .qualified_type_aliases
                .explicitly_declares_type(&qualified_name)
            {
                for prefix_len in (1..normalized_path.len()).rev() {
                    let glob_module = normalized_path[..prefix_len].join("::");
                    let shadowed_item = format!("{glob_module}::{}", normalized_path[prefix_len]);
                    if scope
                        .qualified_type_aliases
                        .explicitly_declares_type(&shadowed_item)
                    {
                        continue;
                    }
                    for (target_index, target) in scope
                        .qualified_type_aliases
                        .glob_targets(&glob_module)
                        .iter()
                        .enumerate()
                    {
                        let Some(target_path) = shell_type_path(target) else {
                            continue;
                        };
                        let resolution_key = format!("{glob_module}::*#{target_index}");
                        if !resolving_aliases.insert(resolution_key.clone()) {
                            continue;
                        }
                        let mut expanded_path = target_path.clone();
                        for segment in &normalized_path[prefix_len..] {
                            expanded_path.path.segments.push(syn::PathSegment::from(
                                syn::Ident::new(segment, proc_macro2::Span::call_site()),
                            ));
                        }
                        if let Some(expanded_segment) = expanded_path.path.segments.last_mut() {
                            expanded_segment.arguments = segment.arguments.clone();
                        }
                        let declaration_module = normalized_path[..prefix_len].to_vec();
                        let candidate_scope = ShellTypeResolutionScope {
                            module_path: &declaration_module,
                            imports: scope
                                .qualified_type_aliases
                                .imports_for_module(&declaration_module),
                            absolute_imports: scope
                                .qualified_type_aliases
                                .absolute_imports_for_module(&declaration_module),
                            path_shadows: scope
                                .qualified_type_aliases
                                .path_shadows_for_module(&declaration_module),
                            allow_local_aliases: false,
                            allow_local_imports: true,
                            ..scope
                        };
                        let candidate = syn::Type::Path(expanded_path);
                        let candidate_key = shell_type_path(&candidate)
                            .map(|path| normalized_shell_type_path(&path.path, candidate_scope))
                            .unwrap_or_default()
                            .join("::");
                        let expanded = shell_type_with_expanded_aliases_in_namespace(
                            &candidate,
                            candidate_scope,
                            resolving_aliases,
                            namespace,
                        );
                        resolving_aliases.remove(&resolution_key);
                        let target_declared =
                            scope.qualified_type_aliases.contains_key(&candidate_key)
                                || scope
                                    .qualified_type_aliases
                                    .explicitly_declares_type(&candidate_key);
                        if target_declared
                            || shell_type_identity(&expanded) != shell_type_identity(&candidate)
                        {
                            return expanded;
                        }
                    }
                }
            }

            let mut expanded = path.clone();
            for segment in &mut expanded.path.segments {
                let syn::PathArguments::AngleBracketed(arguments) = &mut segment.arguments else {
                    continue;
                };
                for argument in &mut arguments.args {
                    let syn::GenericArgument::Type(inner) = argument else {
                        continue;
                    };
                    *inner = shell_type_with_expanded_aliases_in_namespace(
                        inner,
                        scope,
                        resolving_aliases,
                        namespace,
                    );
                }
            }
            syn::Type::Path(expanded)
        }
        syn::Type::Reference(reference) => {
            let mut expanded = reference.clone();
            expanded.elem = Box::new(shell_type_with_expanded_aliases_in_namespace(
                reference.elem.as_ref(),
                scope,
                resolving_aliases,
                namespace,
            ));
            syn::Type::Reference(expanded)
        }
        syn::Type::Ptr(pointer) => {
            let mut expanded = pointer.clone();
            expanded.elem = Box::new(shell_type_with_expanded_aliases_in_namespace(
                pointer.elem.as_ref(),
                scope,
                resolving_aliases,
                namespace,
            ));
            syn::Type::Ptr(expanded)
        }
        syn::Type::Group(group) => {
            let mut expanded = group.clone();
            expanded.elem = Box::new(shell_type_with_expanded_aliases_in_namespace(
                group.elem.as_ref(),
                scope,
                resolving_aliases,
                namespace,
            ));
            syn::Type::Group(expanded)
        }
        syn::Type::Paren(paren) => {
            let mut expanded = paren.clone();
            expanded.elem = Box::new(shell_type_with_expanded_aliases_in_namespace(
                paren.elem.as_ref(),
                scope,
                resolving_aliases,
                namespace,
            ));
            syn::Type::Paren(expanded)
        }
        syn::Type::Slice(slice) => {
            let mut expanded = slice.clone();
            expanded.elem = Box::new(shell_type_with_expanded_aliases_in_namespace(
                slice.elem.as_ref(),
                scope,
                resolving_aliases,
                namespace,
            ));
            syn::Type::Slice(expanded)
        }
        syn::Type::Array(array) => {
            let mut expanded = array.clone();
            expanded.elem = Box::new(shell_type_with_expanded_aliases_in_namespace(
                array.elem.as_ref(),
                scope,
                resolving_aliases,
                namespace,
            ));
            syn::Type::Array(expanded)
        }
        syn::Type::Tuple(tuple) => {
            let mut expanded = tuple.clone();
            for element in &mut expanded.elems {
                *element = shell_type_with_expanded_aliases_in_namespace(
                    element,
                    scope,
                    resolving_aliases,
                    namespace,
                );
            }
            syn::Type::Tuple(expanded)
        }
        _ => ty.clone(),
    }
}

#[derive(Clone)]
struct ShellTypeParameter {
    name: String,
    default: Option<syn::Type>,
}

#[derive(Clone)]
enum ShellAliasGenericParameter {
    Lifetime,
    Type(Box<ShellTypeParameter>),
    Const,
}

fn shell_alias_generic_parameters(generics: &syn::Generics) -> Vec<ShellAliasGenericParameter> {
    generics
        .params
        .iter()
        .map(|parameter| match parameter {
            syn::GenericParam::Lifetime(_) => ShellAliasGenericParameter::Lifetime,
            syn::GenericParam::Type(parameter) => {
                ShellAliasGenericParameter::Type(Box::new(ShellTypeParameter {
                    name: parameter.ident.to_string(),
                    default: parameter.default.clone(),
                }))
            }
            syn::GenericParam::Const(_) => ShellAliasGenericParameter::Const,
        })
        .collect()
}

#[derive(Clone)]
struct ShellTypeAlias {
    ty: syn::Type,
    generic_parameters: Vec<ShellAliasGenericParameter>,
    forwards_arguments: bool,
}

impl ShellTypeAlias {
    fn declared(item: &syn::ItemType) -> Self {
        Self {
            ty: item.ty.as_ref().clone(),
            generic_parameters: shell_alias_generic_parameters(&item.generics),
            forwards_arguments: false,
        }
    }

    fn transparent(mut ty: syn::Type, absolute: bool) -> Self {
        if absolute {
            mark_shell_type_path_absolute(&mut ty);
        }
        Self {
            ty,
            generic_parameters: Vec::new(),
            forwards_arguments: true,
        }
    }

    fn instantiate(&self, arguments: &syn::PathArguments) -> syn::Type {
        let mut instantiated = self.ty.clone();
        if self.generic_parameters.is_empty() {
            if self.forwards_arguments {
                attach_shell_alias_arguments(&mut instantiated, arguments);
            }
            return instantiated;
        }
        let actual_arguments = match arguments {
            syn::PathArguments::AngleBracketed(arguments) => {
                arguments.args.iter().collect::<Vec<_>>()
            }
            syn::PathArguments::None | syn::PathArguments::Parenthesized(_) => Vec::new(),
        };
        let mut argument_index = 0;
        let mut replacements = HashMap::new();
        for parameter in &self.generic_parameters {
            match parameter {
                ShellAliasGenericParameter::Lifetime => {
                    if actual_arguments
                        .get(argument_index)
                        .is_some_and(|argument| {
                            matches!(argument, syn::GenericArgument::Lifetime(_))
                        })
                    {
                        argument_index += 1;
                    }
                }
                ShellAliasGenericParameter::Const => {
                    if actual_arguments
                        .get(argument_index)
                        .is_some_and(|argument| {
                            !matches!(argument, syn::GenericArgument::Lifetime(_))
                        })
                    {
                        argument_index += 1;
                    }
                }
                ShellAliasGenericParameter::Type(parameter) => {
                    let replacement = actual_arguments
                        .get(argument_index)
                        .and_then(|argument| {
                            let syn::GenericArgument::Type(ty) = argument else {
                                return None;
                            };
                            argument_index += 1;
                            Some((*ty).clone())
                        })
                        .or_else(|| {
                            let mut default = parameter.default.clone()?;
                            ShellTypeParameterSubstituter {
                                replacements: &replacements,
                            }
                            .visit_type_mut(&mut default);
                            Some(default)
                        });
                    if let Some(replacement) = replacement {
                        replacements.insert(parameter.name.clone(), replacement);
                    }
                }
            }
        }
        ShellTypeParameterSubstituter {
            replacements: &replacements,
        }
        .visit_type_mut(&mut instantiated);
        instantiated
    }
}

fn mark_shell_type_path_absolute(ty: &mut syn::Type) {
    match ty {
        syn::Type::Group(group) => mark_shell_type_path_absolute(group.elem.as_mut()),
        syn::Type::Paren(paren) => mark_shell_type_path_absolute(paren.elem.as_mut()),
        syn::Type::Path(path) if path.qself.is_none() => {
            path.path.leading_colon = Some(Default::default());
        }
        _ => {}
    }
}

struct ShellTypeParameterSubstituter<'a> {
    replacements: &'a HashMap<String, syn::Type>,
}

impl VisitMut for ShellTypeParameterSubstituter<'_> {
    fn visit_type_mut(&mut self, ty: &mut syn::Type) {
        if let syn::Type::Path(path) = ty
            && path.qself.is_none()
            && path.path.leading_colon.is_none()
            && path.path.segments.len() == 1
            && matches!(path.path.segments[0].arguments, syn::PathArguments::None)
            && let Some(replacement) = self
                .replacements
                .get(&path.path.segments[0].ident.to_string())
        {
            *ty = replacement.clone();
            return;
        }
        visit_mut::visit_type_mut(self, ty);
    }
}

fn attach_shell_alias_arguments(ty: &mut syn::Type, arguments: &syn::PathArguments) {
    if matches!(arguments, syn::PathArguments::None) {
        return;
    }
    match ty {
        syn::Type::Group(group) => attach_shell_alias_arguments(group.elem.as_mut(), arguments),
        syn::Type::Paren(paren) => attach_shell_alias_arguments(paren.elem.as_mut(), arguments),
        syn::Type::Path(path)
            if path.qself.is_none()
                && path.path.segments.last().is_some_and(|segment| {
                    matches!(segment.arguments, syn::PathArguments::None)
                }) =>
        {
            path.path
                .segments
                .last_mut()
                .expect("a checked alias path has a final segment")
                .arguments = arguments.clone();
        }
        _ => {}
    }
}

type ShellTypeAliases = HashMap<String, ShellTypeAlias>;

fn collect_use_type_renames(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    aliases: &mut ShellTypeAliases,
    absolute: bool,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_type_renames(path.tree.as_ref(), prefix, aliases, absolute);
            prefix.pop();
        }
        syn::UseTree::Rename(rename) => {
            let mut target = prefix.clone();
            if rename.ident != "self" {
                target.push(rename.ident.to_string());
            }
            if let Ok(ty) = syn::parse_str::<syn::Type>(&target.join("::")) {
                aliases.insert(
                    rename.rename.to_string(),
                    ShellTypeAlias::transparent(ty, absolute),
                );
            }
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_type_renames(item, prefix, aliases, absolute);
            }
        }
        syn::UseTree::Glob(_) | syn::UseTree::Name(_) => {}
    }
}

fn collect_item_type_alias(item: &syn::Item, aliases: &mut ShellTypeAliases) {
    if item_is_test_only(item) {
        return;
    }
    match item {
        syn::Item::Type(item) => {
            aliases.insert(item.ident.to_string(), ShellTypeAlias::declared(item));
        }
        syn::Item::Use(item) => {
            collect_use_type_renames(
                &item.tree,
                &mut Vec::new(),
                aliases,
                item.leading_colon.is_some(),
            );
        }
        _ => {}
    }
}

fn type_aliases_declared_in_items(items: &[syn::Item]) -> ShellTypeAliases {
    let mut aliases = HashMap::new();
    for item in items {
        collect_item_type_alias(item, &mut aliases);
    }
    aliases
}

fn type_aliases_declared_in_statements(statements: &[syn::Stmt]) -> ShellTypeAliases {
    let mut aliases = HashMap::new();
    for statement in statements {
        if let syn::Stmt::Item(item) = statement {
            collect_item_type_alias(item, &mut aliases);
        }
    }
    aliases
}

#[derive(Clone, Default)]
struct ShellQualifiedTypeAliases {
    aliases: ShellTypeAliases,
    glob_imports: HashMap<String, Vec<syn::Type>>,
    crate_aliases: HashSet<String>,
    explicit_type_paths: HashSet<String>,
    module_imports: HashMap<String, ShellStructImports>,
    module_absolute_imports: HashMap<String, HashSet<String>>,
    module_path_shadows: HashMap<String, HashSet<String>>,
    root_path_shadows: HashSet<String>,
}

impl ShellQualifiedTypeAliases {
    fn new() -> Self {
        Self::default()
    }

    fn insert(&mut self, name: String, alias: ShellTypeAlias) {
        self.aliases.insert(name, alias);
    }

    fn insert_glob(&mut self, module: String, target: syn::Type) {
        self.glob_imports.entry(module).or_default().push(target);
    }

    fn insert_crate_alias(&mut self, alias: String) {
        self.crate_aliases.insert(alias);
    }

    fn insert_explicit_type(&mut self, path: String) {
        self.explicit_type_paths.insert(path);
    }

    fn insert_module_path_shadows(&mut self, module: String, shadows: HashSet<String>) {
        self.module_path_shadows
            .entry(module)
            .or_default()
            .extend(shadows);
    }

    fn insert_module_imports(&mut self, module: String, imports: ShellStructImports) {
        self.module_imports
            .entry(module)
            .or_default()
            .extend(imports);
    }

    fn insert_module_absolute_imports(&mut self, module: String, imports: HashSet<String>) {
        self.module_absolute_imports
            .entry(module)
            .or_default()
            .extend(imports);
    }

    fn insert_root_path_shadow(&mut self, name: String) {
        self.root_path_shadows.insert(name);
    }

    fn extend(&mut self, other: Self) {
        self.aliases.extend(other.aliases);
        for (module, targets) in other.glob_imports {
            self.glob_imports.entry(module).or_default().extend(targets);
        }
        self.crate_aliases.extend(other.crate_aliases);
        self.explicit_type_paths.extend(other.explicit_type_paths);
        for (module, imports) in other.module_imports {
            self.module_imports
                .entry(module)
                .or_default()
                .extend(imports);
        }
        for (module, imports) in other.module_absolute_imports {
            self.module_absolute_imports
                .entry(module)
                .or_default()
                .extend(imports);
        }
        for (module, shadows) in other.module_path_shadows {
            self.module_path_shadows
                .entry(module)
                .or_default()
                .extend(shadows);
        }
        self.root_path_shadows.extend(other.root_path_shadows);
    }

    fn get(&self, name: &str) -> Option<&ShellTypeAlias> {
        self.aliases.get(name)
    }

    fn contains_key(&self, name: &str) -> bool {
        self.aliases.contains_key(name)
    }

    fn glob_targets(&self, module: &str) -> &[syn::Type] {
        self.glob_imports.get(module).map_or(&[], Vec::as_slice)
    }

    fn is_crate_alias(&self, name: &str) -> bool {
        self.crate_aliases.contains(name)
    }

    fn explicitly_declares_type(&self, path: &str) -> bool {
        self.explicit_type_paths.contains(path)
    }

    fn path_shadows_for_module(&self, module_path: &[String]) -> &HashSet<String> {
        self.module_path_shadows
            .get(&module_path.join("::"))
            .unwrap_or_else(|| {
                static EMPTY: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
                EMPTY.get_or_init(HashSet::new)
            })
    }

    fn imports_for_module(&self, module_path: &[String]) -> &ShellStructImports {
        self.module_imports
            .get(&module_path.join("::"))
            .unwrap_or_else(|| {
                static EMPTY: std::sync::OnceLock<ShellStructImports> = std::sync::OnceLock::new();
                EMPTY.get_or_init(ShellStructImports::new)
            })
    }

    fn absolute_imports_for_module(&self, module_path: &[String]) -> &HashSet<String> {
        self.module_absolute_imports
            .get(&module_path.join("::"))
            .unwrap_or_else(|| {
                static EMPTY: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
                EMPTY.get_or_init(HashSet::new)
            })
    }

    fn root_path_is_shadowed(&self, name: &str) -> bool {
        self.root_path_shadows.contains(name)
    }
}

fn collect_use_glob_targets(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    targets: &mut Vec<Vec<String>>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_glob_targets(path.tree.as_ref(), prefix, targets);
            prefix.pop();
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_glob_targets(item, prefix, targets);
            }
        }
        syn::UseTree::Glob(_) => targets.push(prefix.clone()),
        syn::UseTree::Name(_) | syn::UseTree::Rename(_) => {}
    }
}

fn collect_qualified_type_aliases(
    items: &[syn::Item],
    module_path: &mut Vec<String>,
    aliases: &mut ShellQualifiedTypeAliases,
) {
    aliases.insert_module_path_shadows(
        module_path.join("::"),
        path_shadows_declared_in_items(items),
    );
    aliases.insert_module_imports(
        module_path.join("::"),
        struct_imports_declared_in_items(items),
    );
    aliases.insert_module_absolute_imports(
        module_path.join("::"),
        absolute_imports_declared_in_items(items),
    );
    for item in items {
        if item_is_test_only(item) {
            continue;
        }
        if module_path.as_slice() == ["crate"] {
            match item {
                syn::Item::ExternCrate(extern_crate) => {
                    let local_name = extern_crate
                        .rename
                        .as_ref()
                        .map_or(&extern_crate.ident, |(_, rename)| rename)
                        .to_string();
                    if extern_crate.ident == "self" {
                        aliases.insert_crate_alias(local_name.clone());
                        if matches!(local_name.as_str(), "std" | "core" | "alloc") {
                            aliases.insert_root_path_shadow(local_name);
                        }
                    } else if extern_crate.ident != local_name {
                        aliases.insert_root_path_shadow(local_name);
                    }
                }
                syn::Item::Mod(module) => {
                    aliases.insert_root_path_shadow(module.ident.to_string());
                }
                syn::Item::Use(import) => {
                    let mut imports = ShellStructImports::new();
                    collect_use_struct_imports(&import.tree, &mut Vec::new(), &mut imports);
                    for (local_name, target) in imports {
                        if matches!(local_name.as_str(), "std" | "core" | "alloc")
                            && target.as_slice() != [local_name.as_str()]
                        {
                            aliases.insert_root_path_shadow(local_name);
                        }
                    }
                }
                _ => {}
            }
        }
        let explicit_type_name = match item {
            syn::Item::Enum(item) => Some(item.ident.to_string()),
            syn::Item::ExternCrate(item) => Some(
                item.rename
                    .as_ref()
                    .map_or(&item.ident, |(_, rename)| rename)
                    .to_string(),
            ),
            syn::Item::Mod(item) => Some(item.ident.to_string()),
            syn::Item::Struct(item) => Some(item.ident.to_string()),
            syn::Item::Trait(item) => Some(item.ident.to_string()),
            syn::Item::TraitAlias(item) => Some(item.ident.to_string()),
            syn::Item::Type(item) => Some(item.ident.to_string()),
            syn::Item::Union(item) => Some(item.ident.to_string()),
            _ => None,
        };
        if let Some(name) = explicit_type_name {
            aliases.insert_explicit_type(format!("{}::{name}", module_path.join("::")));
        }
        if let syn::Item::Type(alias) = item {
            aliases.insert(
                format!("{}::{}", module_path.join("::"), alias.ident),
                ShellTypeAlias::declared(alias),
            );
        }
        if let syn::Item::Use(import) = item {
            let mut imports = ShellStructImports::new();
            collect_use_struct_imports(&import.tree, &mut Vec::new(), &mut imports);
            for (local_name, target) in imports {
                if let Ok(ty) = syn::parse_str::<syn::Type>(&target.join("::")) {
                    aliases.insert(
                        format!("{}::{local_name}", module_path.join("::")),
                        ShellTypeAlias::transparent(ty, import.leading_colon.is_some()),
                    );
                }
            }
            let mut glob_targets = Vec::new();
            collect_use_glob_targets(&import.tree, &mut Vec::new(), &mut glob_targets);
            for target in glob_targets {
                if let Ok(mut ty) = syn::parse_str::<syn::Type>(&target.join("::")) {
                    if import.leading_colon.is_some() {
                        mark_shell_type_path_absolute(&mut ty);
                    }
                    aliases.insert_glob(module_path.join("::"), ty);
                }
            }
        }
        if let syn::Item::Mod(module) = item
            && let Some((_, nested_items)) = &module.content
        {
            module_path.push(module.ident.to_string());
            collect_qualified_type_aliases(nested_items, module_path, aliases);
            module_path.pop();
        }
    }
}

fn qualified_type_aliases_declared_in_file(
    file: &syn::File,
    module_path: &[String],
) -> ShellQualifiedTypeAliases {
    let mut aliases = ShellQualifiedTypeAliases::new();
    collect_qualified_type_aliases(&file.items, &mut module_path.to_vec(), &mut aliases);
    aliases
}

type ShellFunctionReturns = HashMap<String, syn::Type>;

fn collect_item_function_return(item: &syn::Item, returns: &mut ShellFunctionReturns) {
    let syn::Item::Fn(function) = item else {
        return;
    };
    if attributes_are_test_only(&function.attrs) {
        return;
    }
    if let syn::ReturnType::Type(_, ty) = &function.sig.output {
        returns.insert(function.sig.ident.to_string(), ty.as_ref().clone());
    }
}

fn function_returns_declared_in_items(items: &[syn::Item]) -> ShellFunctionReturns {
    let mut returns = HashMap::new();
    for item in items {
        collect_item_function_return(item, &mut returns);
    }
    returns
}

fn function_returns_declared_in_statements(statements: &[syn::Stmt]) -> ShellFunctionReturns {
    let mut returns = HashMap::new();
    for statement in statements {
        if let syn::Stmt::Item(item) = statement {
            collect_item_function_return(item, &mut returns);
        }
    }
    returns
}

#[derive(Clone)]
struct ShellStructDefinition {
    fields: HashMap<String, syn::Type>,
    generic_parameters: Vec<ShellAliasGenericParameter>,
}

impl ShellStructDefinition {
    fn declared(fields: HashMap<String, syn::Type>, generics: &syn::Generics) -> Self {
        Self {
            fields,
            generic_parameters: shell_alias_generic_parameters(generics),
        }
    }

    fn instantiated_field(&self, name: &str, arguments: &syn::PathArguments) -> Option<syn::Type> {
        let ty = self.fields.get(name)?.clone();
        Some(
            ShellTypeAlias {
                ty,
                generic_parameters: self.generic_parameters.clone(),
                forwards_arguments: false,
            }
            .instantiate(arguments),
        )
    }

    fn instantiated_fields(&self, arguments: &syn::PathArguments) -> Vec<syn::Type> {
        self.fields
            .keys()
            .filter_map(|name| self.instantiated_field(name, arguments))
            .collect()
    }
}

type ShellStructFields = HashMap<String, ShellStructDefinition>;

fn shell_member_key(member: &syn::Member) -> String {
    match member {
        syn::Member::Named(member) => member.to_string(),
        syn::Member::Unnamed(member) => member.index.to_string(),
    }
}

fn collect_item_struct_fields(item: &syn::Item, structs: &mut ShellStructFields) {
    match item {
        syn::Item::Struct(item) if !attributes_are_test_only(&item.attrs) => {
            let fields = item
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    (
                        field
                            .ident
                            .as_ref()
                            .map_or_else(|| index.to_string(), ToString::to_string),
                        field.ty.clone(),
                    )
                })
                .collect();
            structs.insert(
                item.ident.to_string(),
                ShellStructDefinition::declared(fields, &item.generics),
            );
        }
        syn::Item::Enum(item) if !attributes_are_test_only(&item.attrs) => {
            let mut all_payloads = HashMap::new();
            for variant in &item.variants {
                if attributes_are_test_only(&variant.attrs) {
                    continue;
                }
                let fields = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(index, field)| {
                        (
                            field
                                .ident
                                .as_ref()
                                .map_or_else(|| index.to_string(), ToString::to_string),
                            field.ty.clone(),
                        )
                    })
                    .collect::<HashMap<_, _>>();
                for (field, ty) in &fields {
                    all_payloads.insert(format!("{}::{field}", variant.ident), ty.clone());
                }
                structs.insert(
                    format!("{}::{}", item.ident, variant.ident),
                    ShellStructDefinition::declared(fields, &item.generics),
                );
            }
            structs.insert(
                item.ident.to_string(),
                ShellStructDefinition::declared(all_payloads, &item.generics),
            );
        }
        _ => {}
    }
}

fn struct_fields_declared_in_items(items: &[syn::Item]) -> ShellStructFields {
    let mut structs = HashMap::new();
    for item in items {
        collect_item_struct_fields(item, &mut structs);
    }
    structs
}

fn struct_fields_declared_in_statements(statements: &[syn::Stmt]) -> ShellStructFields {
    let mut structs = HashMap::new();
    for statement in statements {
        if let syn::Stmt::Item(item) = statement {
            collect_item_struct_fields(item, &mut structs);
        }
    }
    structs
}

fn collect_qualified_struct_fields(
    items: &[syn::Item],
    module_path: &mut Vec<String>,
    structs: &mut ShellStructFields,
) {
    for item in items {
        if item_is_test_only(item) {
            continue;
        }
        let mut declared = ShellStructFields::new();
        collect_item_struct_fields(item, &mut declared);
        for (name, fields) in declared {
            structs.insert(format!("{}::{name}", module_path.join("::")), fields);
        }
        if let syn::Item::Mod(module) = item
            && let Some((_, nested_items)) = &module.content
        {
            module_path.push(module.ident.to_string());
            collect_qualified_struct_fields(nested_items, module_path, structs);
            module_path.pop();
        }
    }
}

fn qualified_struct_fields_declared_in_file(
    file: &syn::File,
    module_path: &[String],
) -> ShellStructFields {
    let mut structs = HashMap::new();
    collect_qualified_struct_fields(&file.items, &mut module_path.to_vec(), &mut structs);
    structs
}

type ShellStructImports = HashMap<String, Vec<String>>;

fn collect_use_struct_imports(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    imports: &mut ShellStructImports,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_struct_imports(path.tree.as_ref(), prefix, imports);
            prefix.pop();
        }
        syn::UseTree::Name(name) if name.ident == "self" => {
            if let Some(local_name) = prefix.last() {
                imports.insert(local_name.clone(), prefix.clone());
            }
        }
        syn::UseTree::Name(name) => {
            let mut target = prefix.clone();
            target.push(name.ident.to_string());
            imports.insert(name.ident.to_string(), target);
        }
        syn::UseTree::Rename(rename) => {
            let mut target = prefix.clone();
            if rename.ident != "self" {
                target.push(rename.ident.to_string());
            }
            imports.insert(rename.rename.to_string(), target);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_struct_imports(item, prefix, imports);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

fn collect_item_struct_imports(item: &syn::Item, imports: &mut ShellStructImports) {
    if item_is_test_only(item) {
        return;
    }
    if let syn::Item::Use(item) = item {
        collect_use_struct_imports(&item.tree, &mut Vec::new(), imports);
    }
}

fn struct_imports_declared_in_items(items: &[syn::Item]) -> ShellStructImports {
    let mut imports = HashMap::new();
    for item in items {
        collect_item_struct_imports(item, &mut imports);
    }
    imports
}

fn struct_imports_declared_in_statements(statements: &[syn::Stmt]) -> ShellStructImports {
    let mut imports = HashMap::new();
    for statement in statements {
        if let syn::Stmt::Item(item) = statement {
            collect_item_struct_imports(item, &mut imports);
        }
    }
    imports
}

fn collect_item_absolute_imports(item: &syn::Item, imports: &mut HashSet<String>) {
    if item_is_test_only(item) {
        return;
    }
    let syn::Item::Use(item) = item else {
        return;
    };
    if item.leading_colon.is_none() {
        return;
    }
    let mut bindings = ShellStructImports::new();
    collect_use_struct_imports(&item.tree, &mut Vec::new(), &mut bindings);
    imports.extend(bindings.into_keys());
}

fn absolute_imports_declared_in_items(items: &[syn::Item]) -> HashSet<String> {
    let mut imports = HashSet::new();
    for item in items {
        collect_item_absolute_imports(item, &mut imports);
    }
    imports
}

fn absolute_imports_declared_in_statements(statements: &[syn::Stmt]) -> HashSet<String> {
    let mut imports = HashSet::new();
    for statement in statements {
        if let syn::Stmt::Item(item) = statement {
            collect_item_absolute_imports(item, &mut imports);
        }
    }
    imports
}

fn extend_struct_import_scope(
    imports: &mut ShellStructImports,
    absolute_imports: &mut HashSet<String>,
    declared_imports: ShellStructImports,
    declared_absolute_imports: HashSet<String>,
) {
    for name in declared_imports.keys() {
        absolute_imports.remove(name);
    }
    imports.extend(declared_imports);
    absolute_imports.extend(declared_absolute_imports);
}

fn collect_item_path_shadows(item: &syn::Item, shadows: &mut HashSet<String>) {
    if item_is_test_only(item) {
        return;
    }
    match item {
        syn::Item::Mod(module) => {
            shadows.insert(module.ident.to_string());
        }
        syn::Item::ExternCrate(extern_crate) if extern_crate.ident != "self" => {
            let local_name = extern_crate
                .rename
                .as_ref()
                .map_or(&extern_crate.ident, |(_, rename)| rename);
            shadows.insert(local_name.to_string());
        }
        syn::Item::Enum(item) => {
            shadows.insert(item.ident.to_string());
        }
        syn::Item::Struct(item) => {
            shadows.insert(item.ident.to_string());
        }
        syn::Item::Trait(item) => {
            shadows.insert(item.ident.to_string());
        }
        syn::Item::TraitAlias(item) => {
            shadows.insert(item.ident.to_string());
        }
        syn::Item::Type(item) => {
            shadows.insert(item.ident.to_string());
        }
        syn::Item::Union(item) => {
            shadows.insert(item.ident.to_string());
        }
        _ => {}
    }
}

fn path_shadows_declared_in_items(items: &[syn::Item]) -> HashSet<String> {
    let mut shadows = HashSet::new();
    for item in items {
        collect_item_path_shadows(item, &mut shadows);
    }
    shadows
}

fn path_shadows_declared_in_statements(statements: &[syn::Stmt]) -> HashSet<String> {
    let mut shadows = HashSet::new();
    for statement in statements {
        if let syn::Stmt::Item(item) = statement {
            collect_item_path_shadows(item, &mut shadows);
        }
    }
    shadows
}

fn pattern_has_mutable_binding(pattern: &syn::Pat) -> bool {
    match pattern {
        syn::Pat::Ident(pattern) => {
            pattern.mutability.is_some()
                || pattern
                    .subpat
                    .as_ref()
                    .is_some_and(|(_, pattern)| pattern_has_mutable_binding(pattern))
        }
        syn::Pat::Or(pattern) => pattern.cases.iter().any(pattern_has_mutable_binding),
        syn::Pat::Paren(pattern) => pattern_has_mutable_binding(pattern.pat.as_ref()),
        syn::Pat::Reference(pattern) => {
            pattern.mutability.is_some() || pattern_has_mutable_binding(pattern.pat.as_ref())
        }
        syn::Pat::Struct(pattern) => pattern
            .fields
            .iter()
            .any(|field| pattern_has_mutable_binding(field.pat.as_ref())),
        syn::Pat::Type(pattern) => pattern_has_mutable_binding(pattern.pat.as_ref()),
        _ => false,
    }
}

#[derive(Default)]
struct PatternBindingNameVisitor {
    names: Vec<String>,
}

impl<'ast> Visit<'ast> for PatternBindingNameVisitor {
    fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
        self.names.push(pattern.ident.to_string());
        visit::visit_pat_ident(self, pattern);
    }
}

fn pattern_binding_names(pattern: &syn::Pat) -> Vec<String> {
    let mut visitor = PatternBindingNameVisitor::default();
    visitor.visit_pat(pattern);
    visitor.names
}

fn child_shell_authority(
    parent: ShellAuthorityKind,
    member: &syn::Member,
) -> Option<ShellAuthorityKind> {
    let syn::Member::Named(member) = member else {
        return None;
    };
    match (parent, member.to_string().as_str()) {
        (ShellAuthorityKind::App, "shell") => Some(ShellAuthorityKind::Shell),
        (ShellAuthorityKind::Shell, "chrome") => Some(ShellAuthorityKind::Chrome),
        _ => None,
    }
}

fn macro_token_identifiers(tokens: &proc_macro2::TokenStream) -> HashSet<String> {
    fn collect(tokens: proc_macro2::TokenStream, identifiers: &mut HashSet<String>) {
        for token in tokens {
            match token {
                proc_macro2::TokenTree::Group(group) => collect(group.stream(), identifiers),
                proc_macro2::TokenTree::Ident(ident) => {
                    identifiers.insert(ident.to_string());
                }
                proc_macro2::TokenTree::Literal(_) | proc_macro2::TokenTree::Punct(_) => {}
            }
        }
    }

    let mut identifiers = HashSet::new();
    collect(tokens.clone(), &mut identifiers);
    identifiers
}

struct MacroTokenInvocation {
    path: String,
    absolute: bool,
    tokens: proc_macro2::TokenStream,
}

fn macro_token_invocations(tokens: &proc_macro2::TokenStream) -> Vec<MacroTokenInvocation> {
    fn collect(tokens: proc_macro2::TokenStream, invocations: &mut Vec<MacroTokenInvocation>) {
        let token_trees = tokens.into_iter().collect::<Vec<_>>();
        for token in &token_trees {
            let proc_macro2::TokenTree::Group(group) = token else {
                continue;
            };
            collect(group.stream(), invocations);
        }
        for index in 0..token_trees.len().saturating_sub(1) {
            let proc_macro2::TokenTree::Ident(ident) = &token_trees[index] else {
                continue;
            };
            if syn::parse_str::<syn::Ident>(&ident.to_string()).is_err() {
                continue;
            }
            let proc_macro2::TokenTree::Punct(bang) = &token_trees[index + 1] else {
                continue;
            };
            if bang.as_char() != '!'
                || !matches!(
                    token_trees.get(index + 2),
                    Some(proc_macro2::TokenTree::Group(_))
                )
            {
                continue;
            }

            let mut path = vec![ident.to_string()];
            let mut path_index = index;
            while path_index >= 3
                && matches!(
                    (&token_trees[path_index - 2], &token_trees[path_index - 1]),
                    (
                        proc_macro2::TokenTree::Punct(first),
                        proc_macro2::TokenTree::Punct(second),
                    ) if first.as_char() == ':' && second.as_char() == ':'
                )
            {
                let proc_macro2::TokenTree::Ident(parent) = &token_trees[path_index - 3] else {
                    break;
                };
                path.push(parent.to_string());
                path_index -= 3;
            }
            path.reverse();
            let absolute = path_index >= 2
                && matches!(
                    (&token_trees[path_index - 2], &token_trees[path_index - 1]),
                    (
                        proc_macro2::TokenTree::Punct(first),
                        proc_macro2::TokenTree::Punct(second),
                    ) if first.as_char() == ':' && second.as_char() == ':'
                );
            let Some(proc_macro2::TokenTree::Group(arguments)) = token_trees.get(index + 2) else {
                continue;
            };
            invocations.push(MacroTokenInvocation {
                path: path.join("::"),
                absolute,
                tokens: arguments.stream(),
            });
        }
    }

    let mut invocations = Vec::new();
    collect(tokens.clone(), &mut invocations);
    invocations
}

fn shell_chrome_read_macro_name(name: &str) -> bool {
    SHELL_CHROME_AUTHORITY_READ_MACROS.contains(&name)
}

fn macro_tokens_have_assignment(tokens: &proc_macro2::TokenStream) -> bool {
    let compact = tokens
        .to_string()
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<Vec<_>>();
    compact.iter().enumerate().any(|(index, character)| {
        if *character != '=' {
            return false;
        }
        let previous = index
            .checked_sub(1)
            .and_then(|index| compact.get(index))
            .copied();
        let next = compact.get(index + 1).copied();
        !matches!(previous, Some('=' | '!' | '<' | '>')) && !matches!(next, Some('=' | '>'))
    })
}

fn akra_event_value_tokens_have_assignment(tokens: &proc_macro2::TokenStream) -> bool {
    let mut segments = vec![Vec::new()];
    for token in tokens.clone() {
        if matches!(&token, proc_macro2::TokenTree::Punct(punct) if punct.as_char() == ',') {
            segments.push(Vec::new());
        } else {
            segments
                .last_mut()
                .expect("one macro argument segment is always present")
                .push(token);
        }
    }
    segments.into_iter().any(|segment| {
        let value_start = segment
            .iter()
            .position(
                |token| matches!(token, proc_macro2::TokenTree::Punct(punct) if punct.as_char() == '='),
            )
            .map_or(0, |index| index + 1);
        let value_tokens = segment
            .into_iter()
            .skip(value_start)
            .collect::<proc_macro2::TokenStream>();
        macro_tokens_have_assignment(&value_tokens)
    })
}

#[derive(Default)]
struct ShellChromeWriterAudit {
    field_writes: Vec<RuntimeWriterFinding>,
    whole_state_writes: Vec<RuntimeWriterFinding>,
    macro_escapes: Vec<RuntimeWriterFinding>,
}

fn shell_chrome_writer_audit(source: &str) -> Result<ShellChromeWriterAudit, String> {
    shell_chrome_writer_audit_with_policy(source, false)
}

fn shell_chrome_writer_audit_with_policy(
    source: &str,
    allow_native_app_dispatch_seam: bool,
) -> Result<ShellChromeWriterAudit, String> {
    shell_chrome_writer_audit_with_struct_fields(
        source,
        allow_native_app_dispatch_seam,
        &ShellStructFields::new(),
    )
}

fn shell_chrome_writer_audit_with_struct_fields(
    source: &str,
    allow_native_app_dispatch_seam: bool,
    known_struct_fields: &ShellStructFields,
) -> Result<ShellChromeWriterAudit, String> {
    shell_chrome_writer_audit_with_struct_registry(
        source,
        allow_native_app_dispatch_seam,
        known_struct_fields,
        &["crate".to_string()],
    )
}

fn shell_chrome_writer_audit_with_struct_registry(
    source: &str,
    allow_native_app_dispatch_seam: bool,
    known_struct_fields: &ShellStructFields,
    module_path: &[String],
) -> Result<ShellChromeWriterAudit, String> {
    shell_chrome_writer_audit_with_type_registry(
        source,
        allow_native_app_dispatch_seam,
        known_struct_fields,
        &ShellQualifiedTypeAliases::new(),
        module_path,
    )
}

fn shell_chrome_writer_audit_with_type_registry(
    source: &str,
    allow_native_app_dispatch_seam: bool,
    known_struct_fields: &ShellStructFields,
    known_type_aliases: &ShellQualifiedTypeAliases,
    module_path: &[String],
) -> Result<ShellChromeWriterAudit, String> {
    let syntax = syn::parse_file(source)
        .map_err(|error| format!("shell chrome writer source must parse: {error}"))?;
    let mut struct_fields = known_struct_fields.clone();
    struct_fields.extend(qualified_struct_fields_declared_in_file(
        &syntax,
        module_path,
    ));
    let mut qualified_type_aliases = known_type_aliases.clone();
    qualified_type_aliases.extend(qualified_type_aliases_declared_in_file(
        &syntax,
        module_path,
    ));
    let mut visitor = ShellChromeWriterVisitor {
        allow_native_app_dispatch_seam,
        struct_fields,
        qualified_type_aliases,
        module_path: module_path.to_vec(),
        ..Default::default()
    };
    visitor.visit_file(&syntax);
    Ok(visitor.audit)
}

#[derive(Default)]
struct ShellChromeWriterVisitor {
    owner: Option<String>,
    impl_authority: Option<ShellAuthorityKind>,
    impl_authority_mutable: bool,
    impl_type: Option<syn::Type>,
    impl_is_inherent: bool,
    allow_native_app_dispatch_seam: bool,
    authority_bindings: HashMap<String, ShellAuthorityBinding>,
    type_bindings: HashMap<String, ShellTypeBinding>,
    type_aliases: ShellTypeAliases,
    qualified_type_aliases: ShellQualifiedTypeAliases,
    function_returns: ShellFunctionReturns,
    closure_bindings: HashMap<String, syn::ExprClosure>,
    struct_fields: ShellStructFields,
    struct_imports: ShellStructImports,
    absolute_imports: HashSet<String>,
    path_shadows: HashSet<String>,
    module_path: Vec<String>,
    audit: ShellChromeWriterAudit,
}

impl ShellChromeWriterVisitor {
    fn finding(&self, line: usize, detail: impl Into<String>) -> RuntimeWriterFinding {
        RuntimeWriterFinding {
            owner: self.owner.clone().unwrap_or_else(|| "<module>".to_string()),
            line,
            detail: detail.into(),
        }
    }

    fn type_resolution_scope(
        &self,
        implicit_self: Option<ShellAuthorityKind>,
    ) -> ShellTypeResolutionScope<'_> {
        ShellTypeResolutionScope {
            implicit_self,
            type_aliases: &self.type_aliases,
            qualified_type_aliases: &self.qualified_type_aliases,
            imports: &self.struct_imports,
            absolute_imports: &self.absolute_imports,
            path_shadows: &self.path_shadows,
            module_path: &self.module_path,
            allow_local_aliases: true,
            allow_local_imports: true,
        }
    }

    fn binding_from_type(
        &self,
        pattern: &syn::Pat,
        ty: &syn::Type,
    ) -> Option<ShellAuthorityBinding> {
        let mut resolving_aliases = HashSet::new();
        let authority = shell_authority_binding_from_type(
            ty,
            self.type_resolution_scope(self.impl_authority),
            &mut resolving_aliases,
        )?;
        Some(ShellAuthorityBinding {
            mutable: authority.mutable || pattern_has_mutable_binding(pattern),
            ..authority
        })
    }

    fn type_grants_mutable_access(&self, ty: &syn::Type) -> bool {
        let mut resolving_authority_aliases = HashSet::new();
        if let Some(authority) = shell_authority_binding_from_type(
            ty,
            self.type_resolution_scope(self.impl_authority),
            &mut resolving_authority_aliases,
        ) {
            return authority.mutable;
        }

        fn resolve(
            ty: &syn::Type,
            aliases: &ShellTypeAliases,
            resolving_aliases: &mut HashSet<String>,
        ) -> bool {
            match ty {
                syn::Type::Reference(reference) => reference.mutability.is_some(),
                syn::Type::Ptr(pointer) => pointer.mutability.is_some(),
                syn::Type::Group(group) => resolve(group.elem.as_ref(), aliases, resolving_aliases),
                syn::Type::Paren(paren) => resolve(paren.elem.as_ref(), aliases, resolving_aliases),
                syn::Type::Path(path) if path.qself.is_none() => {
                    let Some(segment) = path.path.segments.last() else {
                        return false;
                    };
                    let name = segment.ident.to_string();
                    if resolving_aliases.insert(name.clone()) {
                        if let Some(alias) = aliases.get(&name) {
                            let instantiated = alias.instantiate(&segment.arguments);
                            let mutable = resolve(&instantiated, aliases, resolving_aliases);
                            resolving_aliases.remove(&name);
                            return mutable;
                        }
                        resolving_aliases.remove(&name);
                    }
                    if matches!(name.as_str(), "MutexGuard" | "RefMut" | "RwLockWriteGuard") {
                        return true;
                    }
                    if matches!(name.as_str(), "Box" | "Option" | "Pin" | "Result")
                        && let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments
                    {
                        return arguments.args.iter().any(|argument| {
                            let syn::GenericArgument::Type(inner) = argument else {
                                return false;
                            };
                            resolve(inner, aliases, resolving_aliases)
                        });
                    }
                    false
                }
                _ => false,
            }
        }

        resolve(ty, &self.type_aliases, &mut HashSet::new())
    }

    fn imported_type_aliases(&self) -> ShellTypeAliases {
        self.struct_imports
            .iter()
            .filter_map(|(local_name, target)| {
                let absolute = self.absolute_imports.contains(local_name);
                let path = syn::parse_str::<syn::Path>(&format!(
                    "{}{}",
                    if absolute { "::" } else { "" },
                    target.join("::")
                ))
                .ok()?;
                let key = normalized_shell_type_path(
                    &path,
                    self.type_resolution_scope(self.impl_authority),
                )
                .join("::");
                if !self.qualified_type_aliases.contains_key(&key) {
                    return None;
                }
                let ty = syn::parse_str::<syn::Type>(&key).ok()?;
                Some((local_name.clone(), ShellTypeAlias::transparent(ty, false)))
            })
            .collect()
    }

    fn extend_imported_type_aliases(&mut self) {
        self.type_aliases.extend(self.imported_type_aliases());
    }

    fn registered_struct_key(&self, path: &syn::Path) -> Option<String> {
        let raw_path = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let first = raw_path.first()?;
        let normalized =
            normalized_shell_type_path(path, self.type_resolution_scope(self.impl_authority))
                .join("::");
        if self.struct_fields.contains_key(&normalized) {
            return Some(normalized);
        }
        if raw_path.len() == 1 && self.struct_fields.contains_key(first) {
            return Some(first.clone());
        }
        if raw_path.len() == 1 {
            let mut scoped_path = self.module_path.clone();
            scoped_path.push(first.clone());
            let scoped_key = scoped_path.join("::");
            if self.struct_fields.contains_key(&scoped_key) {
                return Some(scoped_key);
            }
        }
        if !matches!(
            raw_path.first().map(String::as_str),
            Some("crate" | "self" | "super")
        ) {
            let mut crate_path = vec!["crate".to_string()];
            crate_path.extend(raw_path.iter().cloned());
            let crate_key = crate_path.join("::");
            if self.struct_fields.contains_key(&crate_key) {
                return Some(crate_key);
            }
        }
        let suffix = format!("::{}", raw_path.last()?);
        let mut matching_keys = self
            .struct_fields
            .keys()
            .filter(|key| key.ends_with(&suffix))
            .cloned();
        let only_match = matching_keys.next()?;
        matching_keys.next().is_none().then_some(only_match)
    }

    fn resolved_struct_type(&self, ty: &syn::Type) -> Option<ShellResolvedStructType> {
        fn resolve(
            visitor: &ShellChromeWriterVisitor,
            ty: &syn::Type,
            resolving_aliases: &mut HashSet<String>,
        ) -> Option<ShellResolvedStructType> {
            match ty {
                syn::Type::Reference(reference) => {
                    resolve(visitor, reference.elem.as_ref(), resolving_aliases)
                }
                syn::Type::Ptr(pointer) => {
                    resolve(visitor, pointer.elem.as_ref(), resolving_aliases)
                }
                syn::Type::Group(group) => resolve(visitor, group.elem.as_ref(), resolving_aliases),
                syn::Type::Paren(paren) => resolve(visitor, paren.elem.as_ref(), resolving_aliases),
                syn::Type::Path(path) if path.qself.is_none() => {
                    if shell_type_is_bare_self(ty) {
                        if !resolving_aliases.insert("Self".to_string()) {
                            return None;
                        }
                        let resolved = visitor
                            .impl_type
                            .as_ref()
                            .and_then(|impl_type| resolve(visitor, impl_type, resolving_aliases));
                        resolving_aliases.remove("Self");
                        return resolved;
                    }
                    let segment = path.path.segments.last()?;
                    let name = segment.ident.to_string();
                    if matches!(
                        name.as_str(),
                        "Box" | "MutexGuard" | "Pin" | "RefMut" | "RwLockWriteGuard"
                    ) && let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments
                    {
                        return arguments.args.iter().find_map(|argument| {
                            let syn::GenericArgument::Type(inner) = argument else {
                                return None;
                            };
                            resolve(visitor, inner, resolving_aliases)
                        });
                    }
                    let raw_path = path
                        .path
                        .segments
                        .iter()
                        .map(|segment| segment.ident.to_string())
                        .collect::<Vec<_>>();
                    if visitor.struct_imports.contains_key(&raw_path[0])
                        && let Some(key) = visitor.registered_struct_key(&path.path)
                    {
                        return Some(ShellResolvedStructType {
                            name: key,
                            arguments: segment.arguments.clone(),
                        });
                    }
                    if path.path.segments.len() == 1 && resolving_aliases.insert(name.clone()) {
                        if let Some(alias) = visitor.type_aliases.get(&name) {
                            let instantiated = alias.instantiate(&segment.arguments);
                            let resolved = resolve(visitor, &instantiated, resolving_aliases);
                            resolving_aliases.remove(&name);
                            return resolved;
                        }
                        resolving_aliases.remove(&name);
                    }
                    visitor
                        .registered_struct_key(&path.path)
                        .map(|name| ShellResolvedStructType {
                            name,
                            arguments: segment.arguments.clone(),
                        })
                }
                _ => None,
            }
        }

        let expanded = shell_type_with_expanded_aliases(
            ty,
            self.type_resolution_scope(self.impl_authority),
            &mut HashSet::new(),
        );
        resolve(self, &expanded, &mut HashSet::new())
    }

    fn registered_struct_field_type(
        &self,
        struct_name: &str,
        arguments: &syn::PathArguments,
        field: &str,
    ) -> Option<syn::Type> {
        self.struct_fields
            .get(struct_name)?
            .instantiated_field(field, arguments)
    }

    fn struct_field_type(&self, ty: &syn::Type, member: &syn::Member) -> Option<syn::Type> {
        let resolved = self.resolved_struct_type(ty)?;
        self.registered_struct_field_type(
            &resolved.name,
            &resolved.arguments,
            &shell_member_key(member),
        )
    }

    fn tuple_struct_pattern_field_type(
        &self,
        ty: &syn::Type,
        path: &syn::Path,
        index: usize,
    ) -> Option<syn::Type> {
        let expanded_type = shell_type_with_expanded_aliases(
            ty,
            self.type_resolution_scope(self.impl_authority),
            &mut HashSet::new(),
        );
        let mut container_type = &expanded_type;
        let mut reference_mutability = None;
        loop {
            match container_type {
                syn::Type::Reference(reference) => {
                    let mutable = reference.mutability.is_some();
                    reference_mutability =
                        Some(reference_mutability.map_or(mutable, |outer| outer && mutable));
                    container_type = reference.elem.as_ref();
                }
                syn::Type::Group(group) => container_type = group.elem.as_ref(),
                syn::Type::Paren(paren) => container_type = paren.elem.as_ref(),
                _ => break,
            }
        }
        let expanded_variant = shell_value_path_with_expanded_aliases(
            &syn::Type::Path(syn::TypePath {
                qself: None,
                path: path.clone(),
            }),
            self.type_resolution_scope(self.impl_authority),
            &mut HashSet::new(),
        );
        let variant = shell_type_path(&expanded_variant)?
            .path
            .segments
            .last()?
            .ident
            .to_string();
        if let Some(parent) = self.resolved_struct_type(container_type) {
            let variant_key = format!("{}::{variant}", parent.name);
            let payload = self
                .registered_struct_field_type(&variant_key, &parent.arguments, &index.to_string())
                .or_else(|| {
                    self.struct_field_type(
                        container_type,
                        &syn::Member::Unnamed(syn::Index::from(index)),
                    )
                });
            if let Some(payload) = payload {
                return Some(reference_mutability.map_or(payload.clone(), |mutable| {
                    Self::referenced_type(payload, mutable)
                }));
            }
        }
        if index == 0
            && let syn::Type::Path(parent_path) = container_type
            && parent_path.qself.is_none()
        {
            let parent = parent_path.path.segments.last()?;
            let argument_index = match (parent.ident.to_string().as_str(), variant.as_str()) {
                ("Option", "Some") | ("Result", "Ok") => Some(0),
                ("Result", "Err") => Some(1),
                _ => None,
            };
            if let Some(argument_index) = argument_index
                && let syn::PathArguments::AngleBracketed(arguments) = &parent.arguments
                && let Some(payload) = arguments
                    .args
                    .iter()
                    .filter_map(|argument| {
                        let syn::GenericArgument::Type(ty) = argument else {
                            return None;
                        };
                        Some(ty)
                    })
                    .nth(argument_index)
            {
                return Some(reference_mutability.map_or_else(
                    || payload.clone(),
                    |mutable| Self::referenced_type(payload.clone(), mutable),
                ));
            }
        }
        None
    }

    fn struct_pattern_field_type(
        &self,
        ty: &syn::Type,
        path: &syn::Path,
        member: &syn::Member,
    ) -> Option<syn::Type> {
        let expanded_type = shell_type_with_expanded_aliases(
            ty,
            self.type_resolution_scope(self.impl_authority),
            &mut HashSet::new(),
        );
        let mut container_type = &expanded_type;
        let mut reference_mutability = None;
        loop {
            match container_type {
                syn::Type::Reference(reference) => {
                    let mutable = reference.mutability.is_some();
                    reference_mutability =
                        Some(reference_mutability.map_or(mutable, |outer| outer && mutable));
                    container_type = reference.elem.as_ref();
                }
                syn::Type::Group(group) => container_type = group.elem.as_ref(),
                syn::Type::Paren(paren) => container_type = paren.elem.as_ref(),
                _ => break,
            }
        }
        let expanded_variant = shell_value_path_with_expanded_aliases(
            &syn::Type::Path(syn::TypePath {
                qself: None,
                path: path.clone(),
            }),
            self.type_resolution_scope(self.impl_authority),
            &mut HashSet::new(),
        );
        let variant = shell_type_path(&expanded_variant)?
            .path
            .segments
            .last()?
            .ident
            .to_string();
        let parent = self.resolved_struct_type(container_type)?;
        let variant_key = format!("{}::{variant}", parent.name);
        let payload = self
            .registered_struct_field_type(
                &variant_key,
                &parent.arguments,
                &shell_member_key(member),
            )
            .or_else(|| self.struct_field_type(container_type, member))?;
        Some(reference_mutability.map_or(payload.clone(), |mutable| {
            Self::referenced_type(payload, mutable)
        }))
    }

    fn bind_type_pattern(&mut self, pattern: &syn::Pat, binding: ShellTypeBinding) {
        match pattern {
            syn::Pat::Ident(pattern) => {
                let name = pattern.ident.to_string();
                let typed_binding = ShellTypeBinding {
                    mutable: binding.mutable || pattern.mutability.is_some(),
                    ..binding.clone()
                };
                if let Some(mut authority) =
                    self.binding_from_type(&syn::Pat::Ident(pattern.clone()), &typed_binding.ty)
                {
                    authority.mutable |= typed_binding.mutable;
                    self.authority_bindings.insert(name.clone(), authority);
                }
                self.type_bindings.insert(name, typed_binding);
                if let Some((_, subpattern)) = &pattern.subpat {
                    self.bind_type_pattern(subpattern, binding);
                }
            }
            syn::Pat::Or(pattern) => {
                for case in &pattern.cases {
                    self.bind_type_pattern(case, binding.clone());
                }
            }
            syn::Pat::Paren(pattern) => self.bind_type_pattern(pattern.pat.as_ref(), binding),
            syn::Pat::Reference(pattern) => {
                let ty = match &binding.ty {
                    syn::Type::Reference(reference) => reference.elem.as_ref().clone(),
                    _ => binding.ty.clone(),
                };
                self.bind_type_pattern(
                    pattern.pat.as_ref(),
                    ShellTypeBinding {
                        ty,
                        mutable: binding.mutable && pattern.mutability.is_some(),
                    },
                );
            }
            syn::Pat::Type(pattern) => {
                let ty = pattern.ty.as_ref().clone();
                self.bind_type_pattern(
                    pattern.pat.as_ref(),
                    ShellTypeBinding {
                        mutable: binding.mutable || self.type_grants_mutable_access(&ty),
                        ty,
                    },
                );
            }
            syn::Pat::Tuple(pattern) => {
                let syn::Type::Tuple(tuple) = &binding.ty else {
                    return;
                };
                let children = pattern
                    .elems
                    .iter()
                    .zip(&tuple.elems)
                    .map(|(pattern, ty)| {
                        (
                            pattern,
                            ShellTypeBinding {
                                mutable: binding.mutable || self.type_grants_mutable_access(ty),
                                ty: ty.clone(),
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                for (pattern, binding) in children {
                    self.bind_type_pattern(pattern, binding);
                }
            }
            syn::Pat::Struct(pattern) => {
                let children = pattern
                    .fields
                    .iter()
                    .filter_map(|field| {
                        self.struct_pattern_field_type(&binding.ty, &pattern.path, &field.member)
                            .map(|ty| {
                                (
                                    field.pat.as_ref(),
                                    ShellTypeBinding {
                                        mutable: binding.mutable
                                            || self.type_grants_mutable_access(&ty),
                                        ty,
                                    },
                                )
                            })
                    })
                    .collect::<Vec<_>>();
                for (pattern, binding) in children {
                    self.bind_type_pattern(pattern, binding);
                }
            }
            syn::Pat::TupleStruct(pattern) => {
                let children = pattern
                    .elems
                    .iter()
                    .enumerate()
                    .filter_map(|(index, child_pattern)| {
                        self.tuple_struct_pattern_field_type(&binding.ty, &pattern.path, index)
                            .map(|ty| {
                                (
                                    child_pattern,
                                    ShellTypeBinding {
                                        mutable: binding.mutable
                                            || self.type_grants_mutable_access(&ty),
                                        ty,
                                    },
                                )
                            })
                    })
                    .collect::<Vec<_>>();
                for (pattern, binding) in children {
                    self.bind_type_pattern(pattern, binding);
                }
            }
            _ => {}
        }
    }

    fn bind_type_pattern_from_type(&mut self, pattern: &syn::Pat, ty: &syn::Type) {
        self.bind_type_pattern(
            pattern,
            ShellTypeBinding {
                ty: ty.clone(),
                mutable: self.type_grants_mutable_access(ty)
                    || pattern_has_mutable_binding(pattern),
            },
        );
    }

    fn resolve_type_binding(&self, expression: &syn::Expr) -> Option<ShellTypeBinding> {
        match expression {
            syn::Expr::Path(path)
                if path.qself.is_none()
                    && path.path.leading_colon.is_none()
                    && path.path.segments.len() == 1 =>
            {
                self.type_bindings
                    .get(&path.path.segments.first()?.ident.to_string())
                    .cloned()
            }
            syn::Expr::Field(field) => {
                let parent = self.resolve_type_binding(field.base.as_ref())?;
                let ty = self.struct_field_type(&parent.ty, &field.member)?;
                Some(ShellTypeBinding {
                    mutable: parent.mutable || self.type_grants_mutable_access(&ty),
                    ty,
                })
            }
            syn::Expr::Group(group) => self.resolve_type_binding(group.expr.as_ref()),
            syn::Expr::Paren(paren) => self.resolve_type_binding(paren.expr.as_ref()),
            syn::Expr::Reference(reference) => {
                let binding = self.resolve_type_binding(reference.expr.as_ref())?;
                Some(ShellTypeBinding {
                    mutable: binding.mutable && reference.mutability.is_some(),
                    ..binding
                })
            }
            syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Deref(_)) => {
                let binding = self.resolve_type_binding(unary.expr.as_ref())?;
                let ty = match &binding.ty {
                    syn::Type::Reference(reference) => reference.elem.as_ref().clone(),
                    syn::Type::Ptr(pointer) => pointer.elem.as_ref().clone(),
                    _ => binding.ty.clone(),
                };
                Some(ShellTypeBinding { ty, ..binding })
            }
            syn::Expr::Index(index) => {
                let parent = self.resolve_type_binding(index.expr.as_ref())?;
                let ty = self.collection_element_type(&parent.ty)?;
                Some(ShellTypeBinding {
                    ty,
                    mutable: parent.mutable,
                })
            }
            syn::Expr::MethodCall(call)
                if matches!(
                    call.method.to_string().as_str(),
                    "first_mut" | "get_mut" | "last_mut"
                ) =>
            {
                let parent = self.resolve_type_binding(call.receiver.as_ref())?;
                let ty = self.collection_element_type(&parent.ty)?;
                Some(ShellTypeBinding {
                    ty,
                    mutable: parent.mutable,
                })
            }
            syn::Expr::MethodCall(call)
                if matches!(call.method.to_string().as_str(), "as_deref_mut" | "as_mut") =>
            {
                let parent = self.resolve_type_binding(call.receiver.as_ref())?;
                Some(ShellTypeBinding {
                    mutable: true,
                    ..parent
                })
            }
            syn::Expr::MethodCall(call)
                if matches!(call.method.to_string().as_str(), "expect" | "unwrap") =>
            {
                let parent = self.resolve_type_binding(call.receiver.as_ref())?;
                if matches!(
                    call.receiver.as_ref(),
                    syn::Expr::MethodCall(accessor)
                        if matches!(
                            accessor.method.to_string().as_str(),
                            "first_mut" | "get_mut" | "last_mut"
                        )
                ) {
                    return Some(parent);
                }
                let ty = self.collection_element_type(&parent.ty)?;
                Some(ShellTypeBinding {
                    mutable: parent.mutable || self.extracted_payload_is_mutable(&ty),
                    ty,
                })
            }
            syn::Expr::MethodCall(call)
                if matches!(
                    call.method.to_string().as_str(),
                    "expect_err" | "unwrap_err"
                ) =>
            {
                let parent = self.resolve_type_binding(call.receiver.as_ref())?;
                let ty = self.result_error_type(&parent.ty)?;
                Some(ShellTypeBinding {
                    mutable: parent.mutable || self.extracted_payload_is_mutable(&ty),
                    ty,
                })
            }
            syn::Expr::Call(call) => {
                if let Some(argument) = self.transparent_call_argument(call) {
                    return self.resolve_type_binding(argument);
                }
                let syn::Expr::Path(path) = call.func.as_ref() else {
                    return None;
                };
                if path.qself.is_some()
                    || path.path.leading_colon.is_some()
                    || path.path.segments.len() != 1
                {
                    return None;
                }
                let ty = self
                    .function_returns
                    .get(&path.path.segments.first()?.ident.to_string())?
                    .clone();
                Some(ShellTypeBinding {
                    mutable: self.type_grants_mutable_access(&ty),
                    ty,
                })
            }
            _ => None,
        }
    }

    fn transparent_call_argument<'a>(&self, call: &'a syn::ExprCall) -> Option<&'a syn::Expr> {
        if call.args.len() != 1 {
            return None;
        }
        let syn::Expr::Path(path) = call.func.as_ref() else {
            return None;
        };
        if path.qself.is_some() {
            return None;
        }
        let raw_path = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let imported_path = (raw_path.len() == 1)
            .then(|| self.struct_imports.get(&raw_path[0]).cloned())
            .flatten();
        let absolute = path.path.leading_colon.is_some()
            || (imported_path.is_some() && self.absolute_imports.contains(&raw_path[0]));
        let resolved_path = imported_path.as_ref().unwrap_or(&raw_path);
        matches!(
            resolved_path.as_slice(),
            [root, convert, identity]
                if matches!(root.as_str(), "std" | "core")
                    && convert == "convert"
                    && identity == "identity"
                    && !self.standard_path_root_is_shadowed(root, absolute)
        )
        .then(|| call.args.first())
        .flatten()
    }

    fn bind_type_pattern_from_expression(&mut self, pattern: &syn::Pat, expression: &syn::Expr) {
        if let Some(mut binding) = self.resolve_type_binding(expression) {
            binding.mutable |= pattern_has_mutable_binding(pattern);
            self.bind_type_pattern(pattern, binding);
        }
    }

    fn type_contains_mutable_authority(
        &self,
        ty: &syn::Type,
        inherited_mutability: bool,
        resolving_structs: &mut HashSet<String>,
    ) -> bool {
        let mut resolving_aliases = HashSet::new();
        if let Some(authority) = shell_authority_binding_from_type(
            ty,
            self.type_resolution_scope(self.impl_authority),
            &mut resolving_aliases,
        ) {
            return inherited_mutability || authority.mutable;
        }
        let Some(struct_type) = self.resolved_struct_type(ty) else {
            return false;
        };
        if resolving_structs.len() >= 64 {
            return true;
        }
        let visit_key = struct_type.visit_key();
        if !resolving_structs.insert(visit_key.clone()) {
            return false;
        }
        let fields = self
            .struct_fields
            .get(&struct_type.name)
            .map(|definition| definition.instantiated_fields(&struct_type.arguments))
            .unwrap_or_default();
        let contains = fields.iter().any(|field| {
            self.type_contains_mutable_authority(
                field,
                inherited_mutability || self.type_grants_mutable_access(field),
                resolving_structs,
            )
        });
        resolving_structs.remove(&visit_key);
        contains
    }

    fn binding_contains_mutable_authority(&self, binding: &ShellTypeBinding) -> bool {
        self.type_contains_mutable_authority(&binding.ty, binding.mutable, &mut HashSet::new())
    }

    fn clear_pattern_bindings(&mut self, pattern: &syn::Pat) {
        for name in pattern_binding_names(pattern) {
            self.authority_bindings.remove(&name);
            self.type_bindings.remove(&name);
            self.closure_bindings.remove(&name);
        }
    }

    fn bind_pattern(&mut self, pattern: &syn::Pat, binding: ShellAuthorityBinding) {
        match pattern {
            syn::Pat::Ident(pattern) => {
                self.authority_bindings
                    .insert(pattern.ident.to_string(), binding);
                if let Some((_, subpattern)) = &pattern.subpat {
                    self.bind_pattern(subpattern, binding);
                }
            }
            syn::Pat::Or(pattern) => {
                for case in &pattern.cases {
                    self.bind_pattern(case, binding);
                }
            }
            syn::Pat::Paren(pattern) => self.bind_pattern(pattern.pat.as_ref(), binding),
            syn::Pat::Tuple(pattern) => {
                for element in &pattern.elems {
                    self.bind_pattern(element, binding);
                }
            }
            syn::Pat::TupleStruct(pattern) => {
                for element in &pattern.elems {
                    self.bind_pattern(element, binding);
                }
            }
            syn::Pat::Slice(pattern) => {
                for element in &pattern.elems {
                    self.bind_pattern(element, binding);
                }
            }
            syn::Pat::Reference(pattern) => self.bind_pattern(
                pattern.pat.as_ref(),
                ShellAuthorityBinding {
                    mutable: binding.mutable && pattern.mutability.is_some(),
                    ..binding
                },
            ),
            syn::Pat::Struct(pattern) => {
                for field in &pattern.fields {
                    if let Some(kind) = child_shell_authority(binding.kind, &field.member) {
                        self.bind_pattern(
                            field.pat.as_ref(),
                            ShellAuthorityBinding {
                                kind,
                                mutable: binding.mutable,
                            },
                        );
                    }
                }
            }
            syn::Pat::Type(pattern) => {
                let typed_binding = self
                    .binding_from_type(pattern.pat.as_ref(), pattern.ty.as_ref())
                    .unwrap_or(binding);
                self.bind_pattern(pattern.pat.as_ref(), typed_binding);
            }
            _ => {}
        }
    }

    fn bind_pattern_from_type(&mut self, pattern: &syn::Pat, ty: &syn::Type) -> bool {
        self.bind_type_pattern_from_type(pattern, ty);
        let mut resolving_aliases = HashSet::new();
        self.bind_pattern_from_type_inner(pattern, ty, &mut resolving_aliases)
    }

    fn bind_pattern_from_type_inner(
        &mut self,
        pattern: &syn::Pat,
        ty: &syn::Type,
        resolving_aliases: &mut HashSet<String>,
    ) -> bool {
        match pattern {
            syn::Pat::Type(pattern) => {
                return self.bind_pattern_from_type_inner(
                    pattern.pat.as_ref(),
                    pattern.ty.as_ref(),
                    resolving_aliases,
                );
            }
            syn::Pat::Paren(pattern) => {
                return self.bind_pattern_from_type_inner(
                    pattern.pat.as_ref(),
                    ty,
                    resolving_aliases,
                );
            }
            _ => {}
        }

        match ty {
            syn::Type::Group(group) => {
                return self.bind_pattern_from_type_inner(
                    pattern,
                    group.elem.as_ref(),
                    resolving_aliases,
                );
            }
            syn::Type::Paren(paren) => {
                return self.bind_pattern_from_type_inner(
                    pattern,
                    paren.elem.as_ref(),
                    resolving_aliases,
                );
            }
            syn::Type::Path(path) if path.qself.is_none() => {
                if let Some(segment) = path.path.segments.last() {
                    let name = segment.ident.to_string();
                    if resolving_aliases.insert(name.clone()) {
                        if let Some(alias) = self.type_aliases.get(&name).cloned() {
                            let instantiated = alias.instantiate(&segment.arguments);
                            let bound = self.bind_pattern_from_type_inner(
                                pattern,
                                &instantiated,
                                resolving_aliases,
                            );
                            resolving_aliases.remove(&name);
                            return bound;
                        }
                        resolving_aliases.remove(&name);
                    }
                }
            }
            syn::Type::Tuple(tuple) => {
                if let syn::Pat::Tuple(pattern) = pattern
                    && pattern.elems.len() == tuple.elems.len()
                {
                    return pattern.elems.iter().zip(&tuple.elems).fold(
                        false,
                        |bound, (pattern, ty)| {
                            self.bind_pattern_from_type_inner(pattern, ty, resolving_aliases)
                                || bound
                        },
                    );
                }
            }
            _ => {}
        }

        if let Some(binding) = self.binding_from_type(pattern, ty) {
            self.bind_pattern(pattern, binding);
            true
        } else {
            false
        }
    }

    fn bind_pattern_from_expression(&mut self, pattern: &syn::Pat, expression: &syn::Expr) -> bool {
        self.bind_type_pattern_from_expression(pattern, expression);
        let typed_authorities = pattern_binding_names(pattern)
            .into_iter()
            .filter_map(|name| {
                self.authority_bindings
                    .get(&name)
                    .copied()
                    .map(|authority| (name, authority))
            })
            .collect::<Vec<_>>();
        match pattern {
            syn::Pat::Type(pattern) => {
                self.bind_type_pattern_from_type(pattern.pat.as_ref(), pattern.ty.as_ref());
                if self.bind_pattern_from_expression(pattern.pat.as_ref(), expression) {
                    return true;
                }
                return self.bind_pattern_from_type(pattern.pat.as_ref(), pattern.ty.as_ref());
            }
            syn::Pat::Paren(pattern) => {
                return self.bind_pattern_from_expression(pattern.pat.as_ref(), expression);
            }
            _ => {}
        }

        match expression {
            syn::Expr::Group(group) => {
                return self.bind_pattern_from_expression(pattern, group.expr.as_ref());
            }
            syn::Expr::Paren(paren) => {
                return self.bind_pattern_from_expression(pattern, paren.expr.as_ref());
            }
            syn::Expr::Tuple(tuple) => {
                if let syn::Pat::Tuple(pattern) = pattern
                    && pattern.elems.len() == tuple.elems.len()
                {
                    return pattern.elems.iter().zip(&tuple.elems).fold(
                        false,
                        |bound, (pattern, expression)| {
                            self.bind_pattern_from_expression(pattern, expression) || bound
                        },
                    );
                }
            }
            _ => {}
        }

        if let Some(mut authority) = self.resolve_authority(expression) {
            authority.mutable |= pattern_has_mutable_binding(pattern);
            self.bind_pattern(pattern, authority);
            for (name, authority) in typed_authorities {
                self.authority_bindings.insert(name, authority);
            }
            true
        } else {
            false
        }
    }

    fn collection_element_type(&self, ty: &syn::Type) -> Option<syn::Type> {
        fn resolve(
            ty: &syn::Type,
            scope: ShellTypeResolutionScope<'_>,
            resolving_aliases: &mut HashSet<String>,
        ) -> Option<syn::Type> {
            match ty {
                syn::Type::Array(array) => Some(array.elem.as_ref().clone()),
                syn::Type::Slice(slice) => Some(slice.elem.as_ref().clone()),
                syn::Type::Group(group) => resolve(group.elem.as_ref(), scope, resolving_aliases),
                syn::Type::Paren(paren) => resolve(paren.elem.as_ref(), scope, resolving_aliases),
                syn::Type::Reference(reference) => {
                    let element = resolve(reference.elem.as_ref(), scope, resolving_aliases)?;
                    Some(syn::Type::Reference(syn::TypeReference {
                        and_token: Default::default(),
                        lifetime: None,
                        mutability: reference.mutability,
                        elem: Box::new(element),
                    }))
                }
                syn::Type::Path(path) if path.qself.is_none() => {
                    let segment = path.path.segments.last()?;
                    let name = segment.ident.to_string();
                    let normalized_path = if path.path.segments.len() == 1 {
                        let mut qualified = scope.module_path.to_vec();
                        qualified.push(name.clone());
                        qualified
                    } else {
                        normalized_shell_type_path(&path.path, scope)
                    };
                    let qualified_name = normalized_path.join("::");
                    let alias_module_path = normalized_path
                        .get(..normalized_path.len().saturating_sub(1))
                        .unwrap_or_default();
                    let local_alias = (scope.allow_local_aliases
                        && (path.path.segments.len() == 1
                            || alias_module_path == scope.module_path))
                        .then(|| scope.type_aliases.get(&name))
                        .flatten();
                    let alias =
                        local_alias.or_else(|| scope.qualified_type_aliases.get(&qualified_name));
                    if let Some(alias) = alias {
                        if !resolving_aliases.insert(qualified_name.clone()) {
                            return None;
                        }
                        let declaration_module = if local_alias.is_some() {
                            scope.module_path
                        } else {
                            alias_module_path
                        };
                        let declaration_imports = if local_alias.is_some() {
                            scope.imports
                        } else {
                            scope
                                .qualified_type_aliases
                                .imports_for_module(declaration_module)
                        };
                        let declaration_absolute_imports = if local_alias.is_some() {
                            scope.absolute_imports
                        } else {
                            scope
                                .qualified_type_aliases
                                .absolute_imports_for_module(declaration_module)
                        };
                        let declaration_path_shadows = if local_alias.is_some() {
                            scope.path_shadows
                        } else {
                            scope
                                .qualified_type_aliases
                                .path_shadows_for_module(declaration_module)
                        };
                        let instantiated = alias.instantiate(&segment.arguments);
                        let resolved = resolve(
                            &instantiated,
                            ShellTypeResolutionScope {
                                module_path: declaration_module,
                                imports: declaration_imports,
                                absolute_imports: declaration_absolute_imports,
                                path_shadows: declaration_path_shadows,
                                allow_local_aliases: local_alias.is_some(),
                                allow_local_imports: true,
                                ..scope
                            },
                            resolving_aliases,
                        );
                        resolving_aliases.remove(&qualified_name);
                        return resolved;
                    }
                    for prefix_len in (1..normalized_path.len()).rev() {
                        let prefix = normalized_path[..prefix_len].join("::");
                        let Some(alias) = scope.qualified_type_aliases.get(&prefix) else {
                            continue;
                        };
                        let instantiated = alias.instantiate(&syn::PathArguments::None);
                        let Some(alias_path) = shell_type_path(&instantiated) else {
                            continue;
                        };
                        if !resolving_aliases.insert(prefix.clone()) {
                            continue;
                        }
                        let mut expanded_path = alias_path.clone();
                        for segment in &normalized_path[prefix_len..] {
                            expanded_path.path.segments.push(syn::PathSegment::from(
                                syn::Ident::new(segment, proc_macro2::Span::call_site()),
                            ));
                        }
                        if let Some(expanded_segment) = expanded_path.path.segments.last_mut() {
                            expanded_segment.arguments = segment.arguments.clone();
                        }
                        let declaration_module = &normalized_path[..prefix_len.saturating_sub(1)];
                        let resolved = resolve(
                            &syn::Type::Path(expanded_path),
                            ShellTypeResolutionScope {
                                module_path: declaration_module,
                                imports: scope
                                    .qualified_type_aliases
                                    .imports_for_module(declaration_module),
                                absolute_imports: scope
                                    .qualified_type_aliases
                                    .absolute_imports_for_module(declaration_module),
                                path_shadows: scope
                                    .qualified_type_aliases
                                    .path_shadows_for_module(declaration_module),
                                allow_local_aliases: false,
                                allow_local_imports: true,
                                ..scope
                            },
                            resolving_aliases,
                        );
                        resolving_aliases.remove(&prefix);
                        if resolved.is_some() {
                            return resolved;
                        }
                    }
                    if !scope
                        .qualified_type_aliases
                        .explicitly_declares_type(&qualified_name)
                    {
                        for prefix_len in (1..normalized_path.len()).rev() {
                            let glob_module = normalized_path[..prefix_len].join("::");
                            let shadowed_item =
                                format!("{glob_module}::{}", normalized_path[prefix_len]);
                            if scope
                                .qualified_type_aliases
                                .explicitly_declares_type(&shadowed_item)
                            {
                                continue;
                            }
                            for (target_index, target) in scope
                                .qualified_type_aliases
                                .glob_targets(&glob_module)
                                .iter()
                                .enumerate()
                            {
                                let Some(target_path) = shell_type_path(target) else {
                                    continue;
                                };
                                let resolution_key = format!("{glob_module}::*#{target_index}");
                                if !resolving_aliases.insert(resolution_key.clone()) {
                                    continue;
                                }
                                let mut expanded_path = target_path.clone();
                                for segment in &normalized_path[prefix_len..] {
                                    expanded_path.path.segments.push(syn::PathSegment::from(
                                        syn::Ident::new(segment, proc_macro2::Span::call_site()),
                                    ));
                                }
                                if let Some(expanded_segment) =
                                    expanded_path.path.segments.last_mut()
                                {
                                    expanded_segment.arguments = segment.arguments.clone();
                                }
                                let declaration_module = &normalized_path[..prefix_len];
                                let resolved = resolve(
                                    &syn::Type::Path(expanded_path),
                                    ShellTypeResolutionScope {
                                        module_path: declaration_module,
                                        imports: scope
                                            .qualified_type_aliases
                                            .imports_for_module(declaration_module),
                                        absolute_imports: scope
                                            .qualified_type_aliases
                                            .absolute_imports_for_module(declaration_module),
                                        path_shadows: scope
                                            .qualified_type_aliases
                                            .path_shadows_for_module(declaration_module),
                                        allow_local_aliases: false,
                                        allow_local_imports: true,
                                        ..scope
                                    },
                                    resolving_aliases,
                                );
                                resolving_aliases.remove(&resolution_key);
                                if resolved.is_some() {
                                    return resolved;
                                }
                            }
                        }
                    }
                    if !matches!(
                        name.as_str(),
                        "BTreeSet"
                            | "BinaryHeap"
                            | "Box"
                            | "HashSet"
                            | "IntoIter"
                            | "Iter"
                            | "IterMut"
                            | "LinkedList"
                            | "Option"
                            | "Result"
                            | "Vec"
                            | "VecDeque"
                    ) {
                        return None;
                    }
                    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                        return None;
                    };
                    let element = arguments.args.iter().find_map(|argument| {
                        let syn::GenericArgument::Type(ty) = argument else {
                            return None;
                        };
                        Some(ty.clone())
                    })?;
                    match name.as_str() {
                        "Iter" => Some(ShellChromeWriterVisitor::referenced_type(element, false)),
                        "IterMut" => Some(ShellChromeWriterVisitor::referenced_type(element, true)),
                        _ => Some(element),
                    }
                }
                _ => None,
            }
        }

        resolve(
            ty,
            self.type_resolution_scope(self.impl_authority),
            &mut HashSet::new(),
        )
    }

    fn result_error_type(&self, ty: &syn::Type) -> Option<syn::Type> {
        fn path(ty: &syn::Type) -> Option<&syn::TypePath> {
            match ty {
                syn::Type::Reference(reference) => path(reference.elem.as_ref()),
                syn::Type::Group(group) => path(group.elem.as_ref()),
                syn::Type::Paren(paren) => path(paren.elem.as_ref()),
                syn::Type::Path(path) if path.qself.is_none() => Some(path),
                _ => None,
            }
        }

        let expanded = shell_type_with_expanded_aliases(
            ty,
            self.type_resolution_scope(self.impl_authority),
            &mut HashSet::new(),
        );
        let result_path = path(&expanded)?;
        let raw_path = result_path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let standard_result = matches!(raw_path.as_slice(), [name] if name == "Result")
            || matches!(
                raw_path.as_slice(),
                [root, module, name]
                    if matches!(root.as_str(), "std" | "core")
                        && module == "result"
                        && name == "Result"
            );
        let registered_result = if raw_path.len() == 1 {
            let mut scoped_path = self.module_path.clone();
            scoped_path.push("Result".to_string());
            self.struct_fields.contains_key("Result")
                || self.struct_fields.contains_key(&scoped_path.join("::"))
        } else {
            let normalized = normalized_shell_type_path(
                &result_path.path,
                self.type_resolution_scope(self.impl_authority),
            )
            .join("::");
            self.struct_fields.contains_key(&normalized)
        };
        if !standard_result || registered_result {
            return None;
        }
        let syn::PathArguments::AngleBracketed(arguments) =
            &result_path.path.segments.last()?.arguments
        else {
            return None;
        };
        arguments
            .args
            .iter()
            .filter_map(|argument| {
                let syn::GenericArgument::Type(ty) = argument else {
                    return None;
                };
                Some(ty.clone())
            })
            .nth(1)
    }

    fn extracted_payload_is_mutable(&self, ty: &syn::Type) -> bool {
        fn resolve(ty: &syn::Type) -> bool {
            match ty {
                syn::Type::Reference(reference) => reference.mutability.is_some(),
                syn::Type::Ptr(pointer) => pointer.mutability.is_some(),
                syn::Type::Group(group) => resolve(group.elem.as_ref()),
                syn::Type::Paren(paren) => resolve(paren.elem.as_ref()),
                _ => true,
            }
        }

        let expanded = shell_type_with_expanded_aliases(
            ty,
            self.type_resolution_scope(self.impl_authority),
            &mut HashSet::new(),
        );
        resolve(&expanded)
    }

    fn referenced_type(ty: syn::Type, mutable: bool) -> syn::Type {
        syn::Type::Reference(syn::TypeReference {
            and_token: Default::default(),
            lifetime: None,
            mutability: mutable.then(Default::default),
            elem: Box::new(ty),
        })
    }

    fn iterator_element_type_from_expression(&self, expression: &syn::Expr) -> Option<syn::Type> {
        match expression {
            syn::Expr::Group(group) => {
                self.iterator_element_type_from_expression(group.expr.as_ref())
            }
            syn::Expr::Paren(paren) => {
                self.iterator_element_type_from_expression(paren.expr.as_ref())
            }
            syn::Expr::Reference(reference) => {
                let binding = self.resolve_type_binding(reference.expr.as_ref())?;
                let element = self.collection_element_type(&binding.ty)?;
                Some(Self::referenced_type(
                    element,
                    reference.mutability.is_some(),
                ))
            }
            syn::Expr::MethodCall(call)
                if matches!(
                    call.method.to_string().as_str(),
                    "into_iter" | "iter" | "iter_mut"
                ) =>
            {
                let binding = self.resolve_type_binding(call.receiver.as_ref())?;
                let element = self.collection_element_type(&binding.ty)?;
                match call.method.to_string().as_str() {
                    "iter" => Some(Self::referenced_type(element, false)),
                    "iter_mut" => Some(Self::referenced_type(element, true)),
                    "into_iter" => Some(element),
                    _ => None,
                }
            }
            _ => {
                let binding = self.resolve_type_binding(expression)?;
                self.collection_element_type(&binding.ty)
            }
        }
    }

    fn bind_for_loop_pattern(&mut self, pattern: &syn::Pat, iterator: &syn::Expr) {
        if let Some(element) = self.iterator_element_type_from_expression(iterator)
            && self.bind_pattern_from_type(pattern, &element)
        {
            return;
        }
        let bound = match iterator {
            syn::Expr::Group(group) => {
                self.bind_for_loop_pattern(pattern, group.expr.as_ref());
                return;
            }
            syn::Expr::Paren(paren) => {
                self.bind_for_loop_pattern(pattern, paren.expr.as_ref());
                return;
            }
            syn::Expr::Array(array) => {
                let mut bound = false;
                for expression in &array.elems {
                    bound = self.bind_pattern_from_expression(pattern, expression) || bound;
                }
                bound
            }
            syn::Expr::Call(call)
                if matches!(
                    call.func.as_ref(),
                    syn::Expr::Path(path)
                        if path.path.segments.last().is_some_and(|segment| {
                            matches!(
                                segment.ident.to_string().as_str(),
                                "once" | "Some" | "Ok"
                            )
                        })
                ) =>
            {
                call.args.first().is_some_and(|expression| {
                    self.bind_pattern_from_expression(pattern, expression)
                })
            }
            syn::Expr::MethodCall(call) if call.method == "into_iter" => {
                self.bind_for_loop_pattern(pattern, call.receiver.as_ref());
                return;
            }
            _ => self.bind_pattern_from_expression(pattern, iterator),
        };
        if !bound {
            self.bind_pattern(
                pattern,
                ShellAuthorityBinding {
                    kind: ShellAuthorityKind::Unknown,
                    mutable: true,
                },
            );
        }
    }

    fn visit_let_chain_condition(&mut self, condition: &syn::Expr) {
        match condition {
            syn::Expr::Binary(binary) if matches!(binary.op, syn::BinOp::And(_)) => {
                self.visit_let_chain_condition(binary.left.as_ref());
                self.visit_let_chain_condition(binary.right.as_ref());
            }
            syn::Expr::Let(condition) => {
                self.visit_expr(condition.expr.as_ref());
                self.clear_pattern_bindings(condition.pat.as_ref());
                self.bind_pattern_from_expression(condition.pat.as_ref(), condition.expr.as_ref());
            }
            syn::Expr::Group(group) => self.visit_let_chain_condition(group.expr.as_ref()),
            syn::Expr::Paren(paren) => self.visit_let_chain_condition(paren.expr.as_ref()),
            _ => self.visit_expr(condition),
        }
    }

    fn seed_signature(&mut self, signature: &syn::Signature) {
        for input in &signature.inputs {
            match input {
                syn::FnArg::Receiver(receiver) => {
                    let owns_impl_target = shell_type_is_bare_self(receiver.ty.as_ref());
                    if self.impl_type.is_some() {
                        self.type_bindings.insert(
                            "self".to_string(),
                            ShellTypeBinding {
                                ty: receiver.ty.as_ref().clone(),
                                mutable: receiver.mutability.is_some()
                                    || self.type_grants_mutable_access(receiver.ty.as_ref())
                                    || (owns_impl_target && self.impl_authority_mutable),
                            },
                        );
                    }
                    if let Some(kind) = self.impl_authority {
                        let mut resolving_aliases = HashSet::new();
                        let typed_receiver = shell_authority_binding_from_type(
                            receiver.ty.as_ref(),
                            self.type_resolution_scope(Some(kind)),
                            &mut resolving_aliases,
                        );
                        self.authority_bindings.insert(
                            "self".to_string(),
                            ShellAuthorityBinding {
                                kind: typed_receiver
                                    .map(|authority| authority.kind)
                                    .unwrap_or(kind),
                                mutable: receiver.mutability.is_some()
                                    || typed_receiver.is_some_and(|authority| authority.mutable)
                                    || (owns_impl_target && self.impl_authority_mutable),
                            },
                        );
                    }
                }
                syn::FnArg::Typed(argument) => {
                    self.clear_pattern_bindings(argument.pat.as_ref());
                    self.bind_pattern_from_type(argument.pat.as_ref(), argument.ty.as_ref());
                }
            }
        }
    }

    fn resolve_authority(&self, expression: &syn::Expr) -> Option<ShellAuthorityBinding> {
        match expression {
            syn::Expr::Path(path)
                if path.qself.is_none()
                    && path.path.leading_colon.is_none()
                    && path.path.segments.len() == 1 =>
            {
                let name = path.path.segments.first()?.ident.to_string();
                if let Some(authority) = self.authority_bindings.get(&name) {
                    return Some(*authority);
                }
                let binding = self.type_bindings.get(&name)?;
                let mut resolving_aliases = HashSet::new();
                let authority = shell_authority_binding_from_type(
                    &binding.ty,
                    self.type_resolution_scope(self.impl_authority),
                    &mut resolving_aliases,
                )?;
                Some(ShellAuthorityBinding {
                    mutable: binding.mutable || authority.mutable,
                    ..authority
                })
            }
            syn::Expr::Field(field) => {
                if let Some(parent) = self.resolve_authority(field.base.as_ref())
                    && let Some(kind) = child_shell_authority(parent.kind, &field.member)
                {
                    return Some(ShellAuthorityBinding {
                        kind,
                        mutable: parent.mutable,
                    });
                }
                let binding = self.resolve_type_binding(expression)?;
                let mut resolving_aliases = HashSet::new();
                let authority = shell_authority_binding_from_type(
                    &binding.ty,
                    self.type_resolution_scope(self.impl_authority),
                    &mut resolving_aliases,
                )?;
                Some(ShellAuthorityBinding {
                    mutable: binding.mutable || authority.mutable,
                    ..authority
                })
            }
            syn::Expr::MethodCall(call) if call.method == "take_shell_chrome_state" => {
                let receiver = self.resolve_authority(call.receiver.as_ref())?;
                (receiver.kind == ShellAuthorityKind::App && receiver.mutable).then_some(
                    ShellAuthorityBinding {
                        kind: ShellAuthorityKind::Chrome,
                        mutable: false,
                    },
                )
            }
            syn::Expr::Group(group) => self.resolve_authority(group.expr.as_ref()),
            syn::Expr::Paren(paren) => self.resolve_authority(paren.expr.as_ref()),
            syn::Expr::Reference(reference) => {
                let authority = self.resolve_authority(reference.expr.as_ref())?;
                Some(ShellAuthorityBinding {
                    mutable: authority.mutable && reference.mutability.is_some(),
                    ..authority
                })
            }
            syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Deref(_)) => {
                self.resolve_authority(unary.expr.as_ref())
            }
            syn::Expr::Index(_) | syn::Expr::MethodCall(_) => {
                let binding = self.resolve_type_binding(expression)?;
                let mut resolving_aliases = HashSet::new();
                let authority = shell_authority_binding_from_type(
                    &binding.ty,
                    self.type_resolution_scope(self.impl_authority),
                    &mut resolving_aliases,
                )?;
                Some(ShellAuthorityBinding {
                    mutable: binding.mutable,
                    ..authority
                })
            }
            syn::Expr::Call(call) => {
                if let Some(argument) = self.transparent_call_argument(call) {
                    return self.resolve_authority(argument);
                }
                let binding = self.resolve_type_binding(expression)?;
                let mut resolving_aliases = HashSet::new();
                let authority = shell_authority_binding_from_type(
                    &binding.ty,
                    self.type_resolution_scope(self.impl_authority),
                    &mut resolving_aliases,
                )?;
                Some(ShellAuthorityBinding {
                    mutable: binding.mutable || authority.mutable,
                    ..authority
                })
            }
            _ => None,
        }
    }

    fn audited_field_write(&self, expression: &syn::Expr) -> Option<String> {
        let syn::Expr::Field(field) = expression else {
            return None;
        };
        let syn::Member::Named(member) = &field.member else {
            return None;
        };
        let field_name = member.to_string();
        if !SHELL_CHROME_REDUCER_FIELDS.contains(&field_name.as_str()) {
            return None;
        }
        let authority = self.resolve_authority(field.base.as_ref())?;
        (authority.kind == ShellAuthorityKind::Chrome && authority.mutable).then_some(field_name)
    }

    fn inspect_write_target(&mut self, expression: &syn::Expr, kind: &str) {
        if let Some(field) = self.audited_field_write(expression) {
            self.audit.field_writes.push(self.finding(
                expression.span().start().line,
                format!("{kind} reaches shell chrome field `{field}`"),
            ));
            return;
        }
        if let Some(authority) = self.resolve_authority(expression)
            && authority.mutable
            && matches!(
                authority.kind,
                ShellAuthorityKind::Shell | ShellAuthorityKind::Chrome
            )
        {
            self.audit.whole_state_writes.push(self.finding(
                expression.span().start().line,
                format!("{kind} replaces or exposes typed shell chrome authority"),
            ));
        }
    }

    fn inspect_return_escape(&mut self, expression: &syn::Expr, kind: &str) {
        if let Some(authority) = self.resolve_authority(expression)
            && authority.kind != ShellAuthorityKind::Unknown
            && (authority.mutable
                || matches!(
                    authority.kind,
                    ShellAuthorityKind::Shell | ShellAuthorityKind::Chrome
                ))
        {
            self.audit.whole_state_writes.push(self.finding(
                expression.span().start().line,
                format!("{kind} exposes {:?} authority", authority.kind),
            ));
            return;
        }
        if let Some(binding) = self.resolve_type_binding(expression)
            && self.binding_contains_mutable_authority(&binding)
        {
            self.audit.whole_state_writes.push(self.finding(
                expression.span().start().line,
                format!("{kind} exposes a typed wrapper containing shell chrome authority"),
            ));
        }
    }

    fn inspect_call_argument(&mut self, expression: &syn::Expr, kind: &str) {
        if let Some(authority) = self.resolve_authority(expression)
            && authority.mutable
            && self
                .resolve_type_binding(expression)
                .is_some_and(|binding| self.type_grants_mutable_access(&binding.ty))
            && matches!(
                authority.kind,
                ShellAuthorityKind::Shell | ShellAuthorityKind::Chrome
            )
        {
            self.audit.whole_state_writes.push(self.finding(
                expression.span().start().line,
                format!("{kind} passes mutable {:?} authority", authority.kind),
            ));
        }
    }

    fn identifiers_mention_mutable_authority(&self, identifiers: &HashSet<String>) -> bool {
        identifiers.iter().any(|identifier| {
            self.authority_bindings
                .get(identifier)
                .is_some_and(|authority| {
                    authority.mutable && authority.kind != ShellAuthorityKind::Unknown
                })
                || self
                    .type_bindings
                    .get(identifier)
                    .is_some_and(|binding| self.binding_contains_mutable_authority(binding))
        })
    }

    fn standard_path_root_is_shadowed(&self, root: &str, absolute: bool) -> bool {
        !absolute
            && (self.qualified_type_aliases.root_path_is_shadowed(root)
                || self.path_shadows.contains(root)
                || self
                    .struct_imports
                    .get(root)
                    .is_some_and(|target| target.as_slice() != [root]))
    }

    fn imported_read_macro_name(&self, target: &[String], absolute: bool) -> Option<String> {
        match target {
            [root, target_name]
                if matches!(root.as_str(), "std" | "core" | "alloc")
                    && shell_chrome_read_macro_name(target_name)
                    && !self.standard_path_root_is_shadowed(root, absolute) =>
            {
                Some(target_name.clone())
            }
            [root, target_name] if root == "crate" && target_name == "akra_event" => {
                Some(target_name.clone())
            }
            _ => None,
        }
    }

    fn shell_chrome_read_macro_path(&self, path: &str, absolute: bool) -> Option<String> {
        let segments = path.split("::").collect::<Vec<_>>();
        match segments.as_slice() {
            [name] => match self.struct_imports.get(*name) {
                Some(target) => {
                    self.imported_read_macro_name(target, self.absolute_imports.contains(*name))
                }
                None => shell_chrome_read_macro_name(name).then(|| (*name).to_string()),
            },
            [root, name]
                if matches!(*root, "std" | "core" | "alloc")
                    && shell_chrome_read_macro_name(name) =>
            {
                (!self.standard_path_root_is_shadowed(root, absolute)).then(|| (*name).to_string())
            }
            ["crate", "akra_event"] => Some("akra_event".to_string()),
            _ => None,
        }
    }

    fn inspect_macro(&mut self, expression: &syn::Macro, item_position: bool) {
        let name = expression
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_else(|| "<anonymous>".to_string());
        let identifiers = macro_token_identifiers(&expression.tokens);
        let mentions_audited_field = SHELL_CHROME_REDUCER_FIELDS
            .iter()
            .any(|field| identifiers.contains(*field));
        let mentions_mutable_authority = self.identifiers_mention_mutable_authority(&identifiers);
        let read_macro_name = self.shell_chrome_read_macro_path(
            &syn_path_name(&expression.path),
            expression.path.leading_colon.is_some(),
        );
        let has_mutation_syntax = (if read_macro_name.as_deref() == Some("akra_event") {
            akra_event_value_tokens_have_assignment(&expression.tokens)
        } else {
            macro_tokens_have_assignment(&expression.tokens)
        }) || SHELL_CHROME_MACRO_MUTATION_IDENTIFIERS
            .iter()
            .any(|identifier| identifiers.contains(*identifier));
        let has_untrusted_nested_authority_macro = macro_token_invocations(&expression.tokens)
            .iter()
            .any(|invocation| {
                self.shell_chrome_read_macro_path(&invocation.path, invocation.absolute)
                    .is_none()
                    && self.identifiers_mention_mutable_authority(&macro_token_identifiers(
                        &invocation.tokens,
                    ))
            });
        if item_position
            || (mentions_audited_field && has_mutation_syntax)
            || (mentions_mutable_authority
                && (read_macro_name.is_none()
                    || has_mutation_syntax
                    || has_untrusted_nested_authority_macro))
        {
            self.audit.macro_escapes.push(self.finding(
                expression.path.span().start().line,
                format!("macro `{name}` may hide a mutable shell chrome authority escape"),
            ));
        }
    }

    fn visit_scoped_block(&mut self, block: &syn::Block, inspect_tail_return: bool) {
        let previous_bindings = self.authority_bindings.clone();
        let previous_type_bindings = self.type_bindings.clone();
        let previous_aliases = self.type_aliases.clone();
        let previous_returns = self.function_returns.clone();
        let previous_closures = self.closure_bindings.clone();
        let previous_structs = self.struct_fields.clone();
        let previous_imports = self.struct_imports.clone();
        let previous_absolute_imports = self.absolute_imports.clone();
        let previous_path_shadows = self.path_shadows.clone();
        extend_struct_import_scope(
            &mut self.struct_imports,
            &mut self.absolute_imports,
            struct_imports_declared_in_statements(&block.stmts),
            absolute_imports_declared_in_statements(&block.stmts),
        );
        self.path_shadows
            .extend(path_shadows_declared_in_statements(&block.stmts));
        self.type_aliases
            .extend(type_aliases_declared_in_statements(&block.stmts));
        self.extend_imported_type_aliases();
        self.function_returns
            .extend(function_returns_declared_in_statements(&block.stmts));
        self.struct_fields
            .extend(struct_fields_declared_in_statements(&block.stmts));
        for statement in &block.stmts {
            self.visit_stmt(statement);
        }
        if inspect_tail_return && let Some(syn::Stmt::Expr(expression, None)) = block.stmts.last() {
            self.inspect_return_escape(expression, "tail return");
        }
        self.authority_bindings = previous_bindings;
        self.type_bindings = previous_type_bindings;
        self.type_aliases = previous_aliases;
        self.function_returns = previous_returns;
        self.closure_bindings = previous_closures;
        self.struct_fields = previous_structs;
        self.struct_imports = previous_imports;
        self.absolute_imports = previous_absolute_imports;
        self.path_shadows = previous_path_shadows;
    }

    fn is_exact_native_app_dispatch_receiver(&self, receiver: &syn::Expr) -> bool {
        let receiver_is_self = match receiver {
            syn::Expr::Path(path) => {
                path.qself.is_none()
                    && path.path.leading_colon.is_none()
                    && path.path.segments.len() == 1
                    && path.path.segments[0].ident == "self"
            }
            syn::Expr::Group(group) => {
                self.is_exact_native_app_dispatch_receiver(group.expr.as_ref())
            }
            syn::Expr::Paren(paren) => {
                self.is_exact_native_app_dispatch_receiver(paren.expr.as_ref())
            }
            _ => false,
        };
        receiver_is_self
            && self.resolve_authority(receiver).is_some_and(|authority| {
                authority.kind == ShellAuthorityKind::App && authority.mutable
            })
    }

    fn inspect_shell_state_seam_call(
        &mut self,
        method: &str,
        receiver: Option<&syn::Expr>,
        line: usize,
    ) {
        if !matches!(
            method,
            "take_shell_chrome_state" | "apply_shell_chrome_state"
        ) {
            return;
        }
        let exact_dispatch_seam = self.allow_native_app_dispatch_seam
            && self.impl_is_inherent
            && self.impl_authority == Some(ShellAuthorityKind::App)
            && self.owner.as_deref() == Some("dispatch_shell_chrome")
            && receiver
                .is_some_and(|receiver| self.is_exact_native_app_dispatch_receiver(receiver));
        if !exact_dispatch_seam {
            self.audit.whole_state_writes.push(self.finding(
                line,
                format!(
                    "calls shell chrome take/apply seam outside `dispatch_shell_chrome`: `{method}`"
                ),
            ));
        }
    }

    fn closure_for_call_target(&self, expression: &syn::Expr) -> Option<syn::ExprClosure> {
        match expression {
            syn::Expr::Closure(closure) => Some(closure.clone()),
            syn::Expr::Path(path)
                if path.qself.is_none()
                    && path.path.leading_colon.is_none()
                    && path.path.segments.len() == 1 =>
            {
                self.closure_bindings
                    .get(&path.path.segments[0].ident.to_string())
                    .cloned()
            }
            syn::Expr::Group(group) => self.closure_for_call_target(group.expr.as_ref()),
            syn::Expr::Paren(paren) => self.closure_for_call_target(paren.expr.as_ref()),
            syn::Expr::Reference(reference) => {
                self.closure_for_call_target(reference.expr.as_ref())
            }
            syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Deref(_)) => {
                self.closure_for_call_target(unary.expr.as_ref())
            }
            syn::Expr::Call(call) => self
                .transparent_closure_wrapper_argument(call)
                .and_then(|argument| self.closure_for_call_target(argument)),
            _ => None,
        }
    }

    fn transparent_closure_wrapper_argument<'a>(
        &self,
        call: &'a syn::ExprCall,
    ) -> Option<&'a syn::Expr> {
        if call.args.len() != 1 {
            return None;
        }
        if let Some(argument) = self.transparent_call_argument(call) {
            return Some(argument);
        }
        let syn::Expr::Path(path) = call.func.as_ref() else {
            return None;
        };
        if path.qself.is_some() {
            return None;
        }
        let mut absolute = path.path.leading_colon.is_some();
        let mut resolved_path = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let mut resolving_imports = HashSet::new();
        while let Some(first) = resolved_path.first().cloned()
            && let Some(imported) = self.struct_imports.get(&first)
            && resolving_imports.insert(first.clone())
        {
            absolute |= self.absolute_imports.contains(&first);
            let mut expanded = imported.clone();
            expanded.extend(resolved_path.iter().skip(1).cloned());
            resolved_path = expanded;
        }
        let known_wrapper = match resolved_path.as_slice() {
            [wrapper, constructor]
                if wrapper == "Box"
                    && matches!(constructor.as_str(), "from" | "new" | "pin")
                    && !self.path_shadows.contains(wrapper) =>
            {
                true
            }
            [root, module, wrapper, constructor]
                if matches!(root.as_str(), "alloc" | "std")
                    && module == "boxed"
                    && wrapper == "Box"
                    && matches!(constructor.as_str(), "from" | "new" | "pin")
                    && !self.standard_path_root_is_shadowed(root, absolute) =>
            {
                true
            }
            [root, module, wrapper, constructor]
                if matches!(root.as_str(), "alloc" | "std")
                    && module == "rc"
                    && wrapper == "Rc"
                    && matches!(constructor.as_str(), "from" | "new")
                    && !self.standard_path_root_is_shadowed(root, absolute) =>
            {
                true
            }
            [root, module, wrapper, constructor]
                if matches!(root.as_str(), "alloc" | "std")
                    && module == "sync"
                    && wrapper == "Arc"
                    && matches!(constructor.as_str(), "from" | "new")
                    && !self.standard_path_root_is_shadowed(root, absolute) =>
            {
                true
            }
            [root, module, wrapper, constructor]
                if matches!(root.as_str(), "core" | "std")
                    && module == "pin"
                    && wrapper == "Pin"
                    && matches!(constructor.as_str(), "new" | "new_unchecked")
                    && !self.standard_path_root_is_shadowed(root, absolute) =>
            {
                true
            }
            _ => false,
        };
        known_wrapper.then(|| call.args.first()).flatten()
    }

    fn visit_closure_with_arguments(
        &mut self,
        expression: &syn::ExprClosure,
        arguments: Option<&syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>>,
    ) {
        let previous = self.authority_bindings.clone();
        let previous_type_bindings = self.type_bindings.clone();
        let previous_closures = self.closure_bindings.clone();
        for (index, pattern) in expression.inputs.iter().enumerate() {
            let argument = arguments.and_then(|arguments| arguments.iter().nth(index));
            let argument_authority = argument.and_then(|argument| self.resolve_authority(argument));
            let argument_type = argument.and_then(|argument| self.resolve_type_binding(argument));
            self.clear_pattern_bindings(pattern);
            if let syn::Pat::Type(pattern) = pattern {
                let typed = self.bind_pattern_from_type(pattern.pat.as_ref(), pattern.ty.as_ref());
                if !typed {
                    if let Some(mut binding) = argument_type {
                        binding.mutable |= pattern_has_mutable_binding(pattern.pat.as_ref());
                        self.bind_type_pattern(pattern.pat.as_ref(), binding);
                    }
                    if let Some(mut authority) = argument_authority {
                        authority.mutable |= pattern_has_mutable_binding(pattern.pat.as_ref());
                        self.bind_pattern(pattern.pat.as_ref(), authority);
                    }
                }
            } else {
                if let Some(mut binding) = argument_type {
                    binding.mutable |= pattern_has_mutable_binding(pattern);
                    self.bind_type_pattern(pattern, binding);
                }
                if let Some(mut authority) = argument_authority {
                    authority.mutable |= pattern_has_mutable_binding(pattern);
                    self.bind_pattern(pattern, authority);
                }
            }
        }
        if let syn::Expr::Block(block) = expression.body.as_ref() {
            self.visit_scoped_block(&block.block, true);
        } else {
            self.visit_expr(expression.body.as_ref());
            self.inspect_return_escape(expression.body.as_ref(), "closure return");
        }
        self.authority_bindings = previous;
        self.type_bindings = previous_type_bindings;
        self.closure_bindings = previous_closures;
    }
}

impl<'ast> Visit<'ast> for ShellChromeWriterVisitor {
    fn visit_file(&mut self, file: &'ast syn::File) {
        let previous_aliases = self.type_aliases.clone();
        let previous_returns = self.function_returns.clone();
        let previous_structs = self.struct_fields.clone();
        let previous_imports = self.struct_imports.clone();
        let previous_absolute_imports = self.absolute_imports.clone();
        let previous_path_shadows = self.path_shadows.clone();
        extend_struct_import_scope(
            &mut self.struct_imports,
            &mut self.absolute_imports,
            struct_imports_declared_in_items(&file.items),
            absolute_imports_declared_in_items(&file.items),
        );
        self.path_shadows
            .extend(path_shadows_declared_in_items(&file.items));
        self.type_aliases
            .extend(type_aliases_declared_in_items(&file.items));
        self.extend_imported_type_aliases();
        self.function_returns
            .extend(function_returns_declared_in_items(&file.items));
        self.struct_fields
            .extend(struct_fields_declared_in_items(&file.items));
        for item in &file.items {
            self.visit_item(item);
        }
        self.type_aliases = previous_aliases;
        self.function_returns = previous_returns;
        self.struct_fields = previous_structs;
        self.struct_imports = previous_imports;
        self.absolute_imports = previous_absolute_imports;
        self.path_shadows = previous_path_shadows;
    }

    fn visit_item(&mut self, item: &'ast syn::Item) {
        if item_is_test_only(item) {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        self.inspect_macro(&item.mac, true);
    }

    fn visit_macro(&mut self, expression: &'ast syn::Macro) {
        self.inspect_macro(expression, false);
    }

    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        if attributes_are_test_only(&module.attrs) {
            return;
        }
        let Some((_, items)) = &module.content else {
            return;
        };
        let previous_aliases = self.type_aliases.clone();
        let previous_returns = self.function_returns.clone();
        let previous_structs = self.struct_fields.clone();
        let previous_imports = self.struct_imports.clone();
        let previous_absolute_imports = self.absolute_imports.clone();
        let previous_path_shadows = self.path_shadows.clone();
        self.module_path.push(module.ident.to_string());
        self.struct_imports = struct_imports_declared_in_items(items);
        self.absolute_imports = absolute_imports_declared_in_items(items);
        self.path_shadows = path_shadows_declared_in_items(items);
        self.type_aliases
            .extend(type_aliases_declared_in_items(items));
        self.extend_imported_type_aliases();
        self.function_returns
            .extend(function_returns_declared_in_items(items));
        self.struct_fields
            .extend(struct_fields_declared_in_items(items));
        for item in items {
            self.visit_item(item);
        }
        self.type_aliases = previous_aliases;
        self.function_returns = previous_returns;
        self.struct_fields = previous_structs;
        self.struct_imports = previous_imports;
        self.absolute_imports = previous_absolute_imports;
        self.path_shadows = previous_path_shadows;
        self.module_path.pop();
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if attributes_are_test_only(&item.attrs) {
            return;
        }
        let previous_authority = self.impl_authority;
        let previous_authority_mutable = self.impl_authority_mutable;
        let previous_type = self.impl_type.clone();
        let previous_is_inherent = self.impl_is_inherent;
        let mut resolving_aliases = HashSet::new();
        let impl_authority = shell_authority_binding_from_type(
            item.self_ty.as_ref(),
            self.type_resolution_scope(self.impl_authority),
            &mut resolving_aliases,
        );
        self.impl_authority = impl_authority.map(|authority| authority.kind);
        self.impl_authority_mutable = impl_authority.is_some_and(|authority| authority.mutable);
        self.impl_type = Some(item.self_ty.as_ref().clone());
        self.impl_is_inherent = item.trait_.is_none();
        visit::visit_item_impl(self, item);
        self.impl_authority = previous_authority;
        self.impl_authority_mutable = previous_authority_mutable;
        self.impl_type = previous_type;
        self.impl_is_inherent = previous_is_inherent;
    }

    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        if attributes_are_test_only(&function.attrs) {
            return;
        }
        let previous = self.owner.replace(function.sig.ident.to_string());
        let previous_bindings = std::mem::take(&mut self.authority_bindings);
        let previous_type_bindings = std::mem::take(&mut self.type_bindings);
        let previous_closures = std::mem::take(&mut self.closure_bindings);
        self.seed_signature(&function.sig);
        visit::visit_signature(self, &function.sig);
        self.visit_scoped_block(&function.block, true);
        self.authority_bindings = previous_bindings;
        self.type_bindings = previous_type_bindings;
        self.closure_bindings = previous_closures;
        self.owner = previous;
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if impl_item_attributes(item).is_some_and(attributes_are_test_only) {
            return;
        }
        let syn::ImplItem::Fn(function) = item else {
            visit::visit_impl_item(self, item);
            return;
        };
        let previous = self.owner.replace(function.sig.ident.to_string());
        let previous_bindings = std::mem::take(&mut self.authority_bindings);
        let previous_type_bindings = std::mem::take(&mut self.type_bindings);
        let previous_closures = std::mem::take(&mut self.closure_bindings);
        self.seed_signature(&function.sig);
        visit::visit_signature(self, &function.sig);
        self.visit_scoped_block(&function.block, true);
        self.authority_bindings = previous_bindings;
        self.type_bindings = previous_type_bindings;
        self.closure_bindings = previous_closures;
        self.owner = previous;
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.visit_scoped_block(block, false);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        let closure_initializer = local
            .init
            .as_ref()
            .and_then(|init| self.closure_for_call_target(init.expr.as_ref()));
        if let Some(init) = &local.init {
            self.visit_expr(init.expr.as_ref());
            if let Some((_, diverge)) = &init.diverge {
                self.visit_expr(diverge.as_ref());
            }
        }
        self.clear_pattern_bindings(&local.pat);
        let bound_from_initializer = local
            .init
            .as_ref()
            .is_some_and(|init| self.bind_pattern_from_expression(&local.pat, init.expr.as_ref()));
        if !bound_from_initializer && let syn::Pat::Type(pattern) = &local.pat {
            self.bind_pattern_from_type(pattern.pat.as_ref(), pattern.ty.as_ref());
        }
        if let Some(closure) = closure_initializer {
            let names = pattern_binding_names(&local.pat);
            if let [name] = names.as_slice() {
                self.closure_bindings.insert(name.clone(), closure);
            }
        }
    }

    fn visit_expr_assign(&mut self, expression: &'ast syn::ExprAssign) {
        self.inspect_write_target(expression.left.as_ref(), "assignment");
        visit::visit_expr_assign(self, expression);
    }

    fn visit_expr_binary(&mut self, expression: &'ast syn::ExprBinary) {
        if matches!(
            expression.op,
            syn::BinOp::AddAssign(_)
                | syn::BinOp::SubAssign(_)
                | syn::BinOp::MulAssign(_)
                | syn::BinOp::DivAssign(_)
                | syn::BinOp::RemAssign(_)
                | syn::BinOp::BitXorAssign(_)
                | syn::BinOp::BitAndAssign(_)
                | syn::BinOp::BitOrAssign(_)
                | syn::BinOp::ShlAssign(_)
                | syn::BinOp::ShrAssign(_)
        ) {
            self.inspect_write_target(expression.left.as_ref(), "compound assignment");
        }
        visit::visit_expr_binary(self, expression);
    }

    fn visit_expr_reference(&mut self, expression: &'ast syn::ExprReference) {
        if expression.mutability.is_some() {
            self.inspect_write_target(expression.expr.as_ref(), "mutable borrow");
        }
        visit::visit_expr_reference(self, expression);
    }

    fn visit_expr_return(&mut self, expression: &'ast syn::ExprReturn) {
        if let Some(value) = &expression.expr {
            self.inspect_return_escape(value.as_ref(), "explicit return");
        }
        visit::visit_expr_return(self, expression);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        self.inspect_shell_state_seam_call(
            &method,
            Some(call.receiver.as_ref()),
            call.method.span().start().line,
        );
        if !SHELL_CHROME_READ_ONLY_METHODS.contains(&method.as_str()) {
            self.inspect_write_target(call.receiver.as_ref(), "mutable method");
        }
        for argument in &call.args {
            self.inspect_call_argument(argument, "method argument");
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref()
            && let Some(method) = path.path.segments.last()
        {
            self.inspect_shell_state_seam_call(
                &method.ident.to_string(),
                None,
                method.ident.span().start().line,
            );
        }
        let invoked_closure = self.closure_for_call_target(call.func.as_ref());
        if let Some(closure) = &invoked_closure {
            self.visit_closure_with_arguments(closure, Some(&call.args));
        }
        for argument in &call.args {
            self.inspect_call_argument(argument, "function argument");
        }
        if invoked_closure.is_some() {
            for argument in &call.args {
                self.visit_expr(argument);
            }
        } else {
            visit::visit_expr_call(self, call);
        }
    }

    fn visit_expr_match(&mut self, expression: &'ast syn::ExprMatch) {
        self.visit_expr(expression.expr.as_ref());
        for arm in &expression.arms {
            let previous = self.authority_bindings.clone();
            let previous_type_bindings = self.type_bindings.clone();
            self.clear_pattern_bindings(&arm.pat);
            self.bind_pattern_from_expression(&arm.pat, expression.expr.as_ref());
            if let Some((_, guard)) = &arm.guard {
                self.visit_expr(guard.as_ref());
            }
            self.visit_expr(arm.body.as_ref());
            self.authority_bindings = previous;
            self.type_bindings = previous_type_bindings;
        }
    }

    fn visit_expr_if(&mut self, expression: &'ast syn::ExprIf) {
        let previous = self.authority_bindings.clone();
        let previous_type_bindings = self.type_bindings.clone();
        self.visit_let_chain_condition(expression.cond.as_ref());
        self.visit_block(&expression.then_branch);
        self.authority_bindings = previous;
        self.type_bindings = previous_type_bindings;
        if let Some((_, otherwise)) = &expression.else_branch {
            self.visit_expr(otherwise.as_ref());
        }
    }

    fn visit_expr_while(&mut self, expression: &'ast syn::ExprWhile) {
        let previous = self.authority_bindings.clone();
        let previous_type_bindings = self.type_bindings.clone();
        self.visit_let_chain_condition(expression.cond.as_ref());
        self.visit_block(&expression.body);
        self.authority_bindings = previous;
        self.type_bindings = previous_type_bindings;
    }

    fn visit_expr_for_loop(&mut self, expression: &'ast syn::ExprForLoop) {
        self.visit_expr(expression.expr.as_ref());
        let previous = self.authority_bindings.clone();
        let previous_type_bindings = self.type_bindings.clone();
        self.clear_pattern_bindings(expression.pat.as_ref());
        self.bind_for_loop_pattern(expression.pat.as_ref(), expression.expr.as_ref());
        self.visit_block(&expression.body);
        self.authority_bindings = previous;
        self.type_bindings = previous_type_bindings;
    }

    fn visit_expr_closure(&mut self, expression: &'ast syn::ExprClosure) {
        self.visit_closure_with_arguments(expression, None);
    }
}

fn verify_shell_chrome_event_reducer_contract(source: &str) -> Result<(), String> {
    let syntax =
        syn::parse_file(source).map_err(|error| format!("shell chrome must parse: {error}"))?;
    let event_enums = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Enum(item)
                if item.ident == "ShellChromeEvent" && !attributes_are_test_only(&item.attrs) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [event_enum] = event_enums.as_slice() else {
        return Err(format!(
            "expected one production ShellChromeEvent enum, found {}",
            event_enums.len()
        ));
    };
    let expected = event_enum
        .variants
        .iter()
        .filter(|variant| !attributes_are_test_only(&variant.attrs))
        .map(|variant| variant.ident.to_string())
        .collect::<HashSet<_>>();

    let reducer = top_level_function(&syntax, "reduce_shell_chrome");
    let event_matches = reducer
        .block
        .stmts
        .iter()
        .filter_map(|statement| match statement {
            syn::Stmt::Expr(syn::Expr::Match(expression), _)
                if expression_is_simple_path(expression.expr.as_ref(), &["event"]) =>
            {
                Some(expression)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [event_match] = event_matches.as_slice() else {
        return Err(format!(
            "reduce_shell_chrome must contain one direct `match event`, found {}",
            event_matches.len()
        ));
    };

    let mut actual = HashSet::new();
    for arm in &event_match.arms {
        if arm.guard.is_some() {
            return Err(format!(
                "ShellChromeEvent arm at line {} must not use a match guard",
                arm.span().start().line
            ));
        }
        for variant in exact_enum_pattern_variants(&arm.pat, "ShellChromeEvent")
            .map_err(|error| format!("ShellChromeEvent reducer {error}"))?
        {
            if !actual.insert(variant.clone()) {
                return Err(format!(
                    "ShellChromeEvent::{variant} must appear in exactly one reducer arm"
                ));
            }
        }
    }
    if actual != expected {
        return Err(format!(
            "ShellChromeEvent reducer arms must exactly cover the enum ({})",
            core_effect_set_difference(&actual, &expected)
        ));
    }

    Ok(())
}

const PARALLEL_CONTROL_PLANE_EFFECT_CONTRACTS: &[(&str, &str)] = &[
    ("EnterParallelMode", "spawn_entry"),
    ("RefreshSupervisor", "spawn_supervisor_snapshot_refresh"),
    ("InspectSupervisor", "spawn_supervisor_inspection"),
    ("RunOrchestrator", "spawn_orchestrator_wake"),
    ("RunOrchestratorTick", "spawn_orchestrator_tick"),
    (
        "PollPendingDispatchWake",
        "spawn_pending_dispatch_wake_poll",
    ),
    ("MutateDispatchCommands", "spawn_dispatch_command_mutation"),
];

fn verify_parallel_control_plane_effect_totality_contract(
    effect_source: &str,
    controller_source: &str,
    runner_source: &str,
) -> Result<(), String> {
    let effect_syntax = syn::parse_file(effect_source)
        .map_err(|error| format!("parallel effect source must parse as Rust: {error}"))?;
    let controller_syntax = syn::parse_file(controller_source)
        .map_err(|error| format!("parallel controller source must parse as Rust: {error}"))?;
    let runner_syntax = syn::parse_file(runner_source)
        .map_err(|error| format!("parallel effect runner source must parse as Rust: {error}"))?;

    let effect_enums = effect_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Enum(item)
                if item.ident == "ParallelModeControlPlaneEffect"
                    && !attributes_are_test_only(&item.attrs) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [effect_enum] = effect_enums.as_slice() else {
        return Err(format!(
            "expected one production ParallelModeControlPlaneEffect enum, found {}",
            effect_enums.len()
        ));
    };
    let enum_variants = effect_enum
        .variants
        .iter()
        .map(|variant| variant.ident.to_string())
        .collect::<HashSet<_>>();
    let expected_variants = PARALLEL_CONTROL_PLANE_EFFECT_CONTRACTS
        .iter()
        .map(|(variant, _)| (*variant).to_string())
        .collect::<HashSet<_>>();
    if enum_variants != expected_variants {
        return Err(format!(
            "parallel effect enum variants must exactly match audited totality contracts ({})",
            core_effect_set_difference(&enum_variants, &expected_variants)
        ));
    }

    let run_effect = find_inherent_method_ending_type(
        &controller_syntax,
        "ParallelModeControlPlaneController",
        "run_effect",
    )?;
    let dispatch = match run_effect.block.stmts.as_slice() {
        [syn::Stmt::Expr(syn::Expr::Match(dispatch), None)]
            if expression_is_simple_path(dispatch.expr.as_ref(), &["effect"]) =>
        {
            dispatch
        }
        _ => {
            return Err(
                "parallel controller run_effect must be exactly one `match effect` expression"
                    .to_string(),
            );
        }
    };
    let contracts = PARALLEL_CONTROL_PLANE_EFFECT_CONTRACTS
        .iter()
        .copied()
        .collect::<HashMap<_, _>>();
    let mut arms = HashMap::new();
    for arm in &dispatch.arms {
        if arm.guard.is_some() {
            return Err(format!(
                "parallel effect dispatch arm at line {} must not use a match guard",
                arm.span().start().line
            ));
        }
        let variant = exact_parallel_effect_pattern_variant(&arm.pat)?;
        if arms.insert(variant.clone(), arm).is_some() {
            return Err(format!(
                "ParallelModeControlPlaneEffect::{variant} must have exactly one dispatch arm"
            ));
        }
    }
    if arms.keys().cloned().collect::<HashSet<_>>() != enum_variants {
        return Err(format!(
            "parallel effect dispatch arms must exactly match the enum ({})",
            core_effect_set_difference(
                &arms.keys().cloned().collect::<HashSet<_>>(),
                &enum_variants,
            )
        ));
    }
    for (variant, arm) in arms {
        let launcher = contracts
            .get(variant.as_str())
            .ok_or_else(|| format!("missing parallel effect contract for {variant}"))?;
        let syn::Expr::Block(body) = arm.body.as_ref() else {
            return Err(format!(
                "ParallelModeControlPlaneEffect::{variant} dispatch must use an explicit block"
            ));
        };
        verify_parallel_effect_dispatch_arm(&variant, &body.block, launcher)?;
    }

    verify_parallel_panic_total_sink(&runner_syntax)?;
    for &(_, launcher) in PARALLEL_CONTROL_PLANE_EFFECT_CONTRACTS {
        let method = find_inherent_method_ending_type(
            &runner_syntax,
            "ParallelModeControlPlaneEffectRunner",
            launcher,
        )?;
        verify_parallel_effect_launcher(method, launcher)?;
    }
    Ok(())
}

fn find_inherent_method_ending_type<'a>(
    syntax: &'a syn::File,
    type_name: &str,
    method_name: &str,
) -> Result<&'a syn::ImplItemFn, String> {
    let methods = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item)
                if item.trait_.is_none()
                    && !attributes_are_test_only(&item.attrs)
                    && type_path_ends_with_ident(item.self_ty.as_ref(), type_name) =>
            {
                Some(item)
            }
            _ => None,
        })
        .flat_map(|item| item.items.iter())
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method)
                if method.sig.ident == method_name && !attributes_are_test_only(&method.attrs) =>
            {
                Some(method)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if methods.len() != 1 {
        return Err(format!(
            "expected one production {type_name}::{method_name}, found {}",
            methods.len()
        ));
    }
    Ok(methods[0])
}

fn exact_parallel_effect_pattern_variant(pattern: &syn::Pat) -> Result<String, String> {
    match pattern {
        syn::Pat::Or(_) => Err("or-patterns are forbidden in parallel effect dispatch".to_string()),
        syn::Pat::Wild(_) => {
            Err("wildcard patterns are forbidden in parallel effect dispatch".to_string())
        }
        syn::Pat::Struct(pattern) => {
            exact_two_segment_enum_variant(&pattern.path, "ParallelModeControlPlaneEffect")
        }
        syn::Pat::TupleStruct(pattern) => {
            exact_two_segment_enum_variant(&pattern.path, "ParallelModeControlPlaneEffect")
        }
        syn::Pat::Path(pattern) if pattern.qself.is_none() => {
            exact_two_segment_enum_variant(&pattern.path, "ParallelModeControlPlaneEffect")
        }
        _ => Err(
            "parallel effect dispatch must use one exact ParallelModeControlPlaneEffect variant"
                .to_string(),
        ),
    }
}

fn verify_parallel_effect_dispatch_arm(
    variant: &str,
    body: &syn::Block,
    launcher: &str,
) -> Result<(), String> {
    let top_level_launchers = body
        .stmts
        .iter()
        .filter_map(direct_parallel_effect_runner_launcher)
        .collect::<Vec<_>>();
    if top_level_launchers.as_slice() != [launcher] {
        return Err(format!(
            "ParallelModeControlPlaneEffect::{variant} must map to one unconditional top-level self.effect_runner.{launcher}(...); found {top_level_launchers:?}"
        ));
    }
    let mut audit = ParallelEffectDispatchVisitor::default();
    audit.visit_block(body);
    if audit.launchers.as_slice() != [launcher] {
        return Err(format!(
            "ParallelModeControlPlaneEffect::{variant} must not launch through a bypass; found {:?}",
            audit.launchers
        ));
    }
    if audit.try_count != 0 {
        return Err(format!(
            "ParallelModeControlPlaneEffect::{variant} must not use ? before completion"
        ));
    }
    if variant == "RefreshSupervisor" {
        if audit.returns != 1 || !refresh_arm_has_exact_sync_settlement(body) {
            return Err(
                "RefreshSupervisor's no-snapshot path must use one explicit synchronous settlement"
                    .to_string(),
            );
        }
    } else if audit.returns != 0 {
        return Err(format!(
            "ParallelModeControlPlaneEffect::{variant} must not return before its panic-total worker"
        ));
    }
    Ok(())
}

fn direct_parallel_effect_runner_launcher(statement: &syn::Stmt) -> Option<String> {
    let syn::Stmt::Expr(syn::Expr::MethodCall(call), Some(_)) = statement else {
        return None;
    };
    if !expression_is_self_field(call.receiver.as_ref(), "effect_runner")
        || !call.method.to_string().starts_with("spawn_")
    {
        return None;
    }
    Some(call.method.to_string())
}

#[derive(Default)]
struct ParallelEffectDispatchVisitor {
    launchers: Vec<String>,
    returns: usize,
    try_count: usize,
}

impl<'ast> Visit<'ast> for ParallelEffectDispatchVisitor {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if expression_is_self_field(call.receiver.as_ref(), "effect_runner")
            && call.method.to_string().starts_with("spawn_")
        {
            self.launchers.push(call.method.to_string());
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_return(&mut self, expression: &'ast syn::ExprReturn) {
        self.returns += 1;
        visit::visit_expr_return(self, expression);
    }

    fn visit_expr_try(&mut self, expression: &'ast syn::ExprTry) {
        self.try_count += 1;
        visit::visit_expr_try(self, expression);
    }
}

fn refresh_arm_has_exact_sync_settlement(body: &syn::Block) -> bool {
    let mut visitor = RefreshSyncSettlementVisitor::default();
    visitor.visit_block(body);
    visitor.completion_commands == 1
        && visitor.runtime_handle_calls == 1
        && visitor.drain_outcome_returns == 1
}

#[derive(Default)]
struct RefreshSyncSettlementVisitor {
    completion_commands: usize,
    runtime_handle_calls: usize,
    drain_outcome_returns: usize,
}

impl<'ast> Visit<'ast> for RefreshSyncSettlementVisitor {
    fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
        if path_is_simple(
            &expression.path,
            &[
                "ParallelModeControlPlaneCommand",
                "SupervisorSnapshotRefreshCompleted",
            ],
        ) {
            self.completion_commands += 1;
        }
        visit::visit_expr_struct(self, expression);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "handle"
            && matches!(
                call.receiver.as_ref(),
                syn::Expr::Field(runtime)
                    if expression_is_simple_path(runtime.base.as_ref(), &["self"])
                        && matches!(
                            &runtime.member,
                            syn::Member::Named(member) if member == "runtime"
                        )
            )
        {
            self.runtime_handle_calls += 1;
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_return(&mut self, expression: &'ast syn::ExprReturn) {
        if matches!(
            expression.expr.as_deref(),
            Some(syn::Expr::MethodCall(call))
                if call.method == "drain_outcome"
                    && expression_is_simple_path(call.receiver.as_ref(), &["self"])
                    && matches!(
                        call.args.iter().collect::<Vec<_>>().as_slice(),
                        [argument] if expression_is_simple_path(argument, &["outcome"])
                    )
        ) {
            self.drain_outcome_returns += 1;
        }
        visit::visit_expr_return(self, expression);
    }
}

fn verify_parallel_panic_total_sink(runner_syntax: &syn::File) -> Result<(), String> {
    let sink = find_production_function(runner_syntax, "spawn_parallel_effect_completion_worker")?;
    let mut visitor = ParallelPanicSinkVisitor::default();
    visitor.visit_block(&sink.block);
    if visitor.thread_spawns != 1
        || visitor.redacted_catches != 2
        || visitor.unwrap_or_calls != 1
        || visitor.completion_sends != 1
        || visitor.returns != 0
        || visitor.try_count != 0
    {
        return Err(format!(
            "shared parallel panic-total sink must catch work and publication exactly once; spawn={}, catch={}, fallback={}, send={}, return={}, ?={}",
            visitor.thread_spawns,
            visitor.redacted_catches,
            visitor.unwrap_or_calls,
            visitor.completion_sends,
            visitor.returns,
            visitor.try_count,
        ));
    }
    Ok(())
}

#[derive(Default)]
struct ParallelPanicSinkVisitor {
    thread_spawns: usize,
    redacted_catches: usize,
    unwrap_or_calls: usize,
    completion_sends: usize,
    returns: usize,
    try_count: usize,
}

impl<'ast> Visit<'ast> for ParallelPanicSinkVisitor {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if expression_is_simple_path(call.func.as_ref(), &["thread", "spawn"]) {
            self.thread_spawns += 1;
        }
        if expression_is_simple_path(call.func.as_ref(), &["catch_redacted_worker_unwind"]) {
            self.redacted_catches += 1;
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "unwrap_or" {
            self.unwrap_or_calls += 1;
        }
        if call.method == "send_control_plane_event" {
            self.completion_sends += 1;
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_return(&mut self, expression: &'ast syn::ExprReturn) {
        self.returns += 1;
        visit::visit_expr_return(self, expression);
    }

    fn visit_expr_try(&mut self, expression: &'ast syn::ExprTry) {
        self.try_count += 1;
        visit::visit_expr_try(self, expression);
    }
}

fn verify_parallel_effect_launcher(method: &syn::ImplItemFn, launcher: &str) -> Result<(), String> {
    if !matches!(method.sig.output, syn::ReturnType::Default) {
        return Err(format!("{launcher} must return unit"));
    }
    let direct_sinks = method
        .block
        .stmts
        .iter()
        .filter(|statement| {
            matches!(
                statement,
                syn::Stmt::Expr(syn::Expr::Call(call), Some(_))
                    if expression_is_simple_path(
                        call.func.as_ref(),
                        &["spawn_parallel_effect_completion_worker"],
                    )
                        && matches!(
                            call.args.iter().collect::<Vec<_>>().as_slice(),
                            [_, panic_completion, syn::Expr::Closure(_)]
                                if expression_is_simple_path(
                                    panic_completion,
                                    &["panic_completion"],
                                )
                        )
            )
        })
        .count();
    if direct_sinks != 1
        || !method.block.stmts.last().is_some_and(|statement| {
            matches!(
                statement,
                syn::Stmt::Expr(syn::Expr::Call(call), Some(_))
                    if expression_is_simple_path(
                        call.func.as_ref(),
                        &["spawn_parallel_effect_completion_worker"],
                    )
            )
        })
    {
        return Err(format!(
            "{launcher} must end in one unconditional shared panic-total completion sink"
        ));
    }
    let mut bypass = ParallelLauncherOuterVisitor::default();
    bypass.visit_block(&method.block);
    if bypass.shared_sinks != 1
        || bypass.returns != 0
        || bypass.try_count != 0
        || bypass.forbidden_calls != 0
    {
        return Err(format!(
            "{launcher} must reach the shared panic-total completion sink without synchronous work or bypass (sink={}, return={}, ?={}, forbidden_calls={})",
            bypass.shared_sinks, bypass.returns, bypass.try_count, bypass.forbidden_calls
        ));
    }
    Ok(())
}

#[derive(Default)]
struct ParallelLauncherOuterVisitor {
    shared_sinks: usize,
    returns: usize,
    try_count: usize,
    forbidden_calls: usize,
}

impl<'ast> Visit<'ast> for ParallelLauncherOuterVisitor {
    fn visit_expr_closure(&mut self, _expression: &'ast syn::ExprClosure) {}

    fn visit_expr_async(&mut self, _expression: &'ast syn::ExprAsync) {}

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if expression_is_simple_path(
            call.func.as_ref(),
            &["spawn_parallel_effect_completion_worker"],
        ) {
            self.shared_sinks += 1;
        } else if !expression_is_simple_path(call.func.as_ref(), &["effect_failed"])
            && !expression_is_simple_path(call.func.as_ref(), &["Err"])
        {
            self.forbidden_calls += 1;
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method != "clone" && call.method != "to_string" {
            self.forbidden_calls += 1;
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_return(&mut self, expression: &'ast syn::ExprReturn) {
        self.returns += 1;
        visit::visit_expr_return(self, expression);
    }

    fn visit_expr_try(&mut self, expression: &'ast syn::ExprTry) {
        self.try_count += 1;
        visit::visit_expr_try(self, expression);
    }
}

const CORE_EFFECT_LAUNCH_CONTRACTS: &[(&str, &str, &str)] = &[
    (
        "RunStartupChecks",
        "spawn_startup_checks",
        "spawn_effect_completion_worker",
    ),
    (
        "LoadSessionCatalog",
        "spawn_session_catalog_load",
        "spawn_effect_completion_worker",
    ),
    (
        "RenameSession",
        "spawn_session_rename",
        "spawn_effect_completion_worker",
    ),
    (
        "LoadConversation",
        "spawn_conversation_load",
        "spawn_effect_completion_worker",
    ),
    (
        "LoadParallelPeekConversation",
        "spawn_parallel_peek_conversation_load",
        "spawn_effect_completion_worker",
    ),
    (
        "LoadReviewCenter",
        "spawn_review_center_load",
        "spawn_effect_completion_worker",
    ),
    (
        "LoadQueueAuthority",
        "spawn_queue_authority_load",
        "spawn_effect_completion_worker",
    ),
    (
        "LoadDirectionsMaintenance",
        "spawn_directions_maintenance_load",
        "spawn_effect_completion_worker",
    ),
    (
        "LoadPlanningRuntime",
        "spawn_planning_runtime_projection_load",
        "spawn_effect_completion_worker",
    ),
    (
        "ResetPlanningWorkspace",
        "spawn_planning_workspace_reset",
        "spawn_effect_completion_worker",
    ),
    (
        "StageSimplePlanningDraft",
        "spawn_simple_planning_draft_stage",
        "spawn_effect_completion_worker",
    ),
    (
        "StagePlanningEditor",
        "spawn_planning_editor_stage",
        "spawn_effect_completion_worker",
    ),
    (
        "MutatePlanningEditor",
        "spawn_planning_editor_mutation",
        "spawn_effect_completion_worker",
    ),
    (
        "LoadSimplePlanningEditor",
        "spawn_simple_planning_editor_load",
        "spawn_effect_completion_worker",
    ),
    (
        "PromoteSimplePlanningDraft",
        "spawn_simple_planning_draft_promotion",
        "spawn_effect_completion_worker",
    ),
    (
        "ExecuteQueueMutation",
        "spawn_queue_mutation",
        "spawn_effect_completion_worker",
    ),
    (
        "SetupGithubReviewPolling",
        "spawn_github_review_polling_setup",
        "spawn_effect_completion_worker_with_recovery",
    ),
    (
        "PollGithubReview",
        "spawn_github_review_poll",
        "spawn_effect_completion_worker",
    ),
    (
        "PrepareManualPrompt",
        "spawn_manual_prompt_preparation",
        "spawn_effect_completion_worker_with_recovery",
    ),
    (
        "SubmitApprovalDecision",
        "spawn_approval_decision_submission",
        "spawn_effect_completion_worker",
    ),
    (
        "PersistApprovalReview",
        "spawn_approval_review_persistence",
        "spawn_effect_completion_worker",
    ),
    (
        "SubmitTurn",
        "spawn_turn_submission",
        "core_turn_submission::spawn_turn_submission_worker",
    ),
    (
        "RequestStopAllSessions",
        "spawn_stop_request_attempt",
        "spawn_effect_completion_worker_with_recovery",
    ),
    (
        "SteerTurn",
        "spawn_turn_steer",
        "spawn_effect_completion_worker",
    ),
    (
        "EvaluatePostTurn",
        "spawn_post_turn_evaluation",
        "spawn_effect_completion_worker_with_recovery",
    ),
];

const CORE_EFFECT_INVALIDATION_CONTRACTS: &[(&str, &str)] = &[
    ("CancelManualPromptPreparation", "manual_prompt_workers"),
    ("InvalidateStopRequest", "stop_request_workers"),
];

fn verify_core_effect_totality_contract(
    effect_source: &str,
    runner_source: &str,
    turn_submission_source: &str,
) -> Result<(), String> {
    let effect_syntax = syn::parse_file(effect_source)
        .map_err(|error| format!("CoreEffect source must parse as Rust: {error}"))?;
    let runner_syntax = syn::parse_file(runner_source)
        .map_err(|error| format!("CoreEffectRunner source must parse as Rust: {error}"))?;
    let turn_submission_syntax = syn::parse_file(turn_submission_source)
        .map_err(|error| format!("turn submission source must parse as Rust: {error}"))?;

    let core_effect_enums = effect_syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Enum(item)
                if item.ident == "CoreEffect" && !attributes_are_test_only(&item.attrs) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if core_effect_enums.len() != 1 {
        return Err(format!(
            "expected one production CoreEffect enum, found {}",
            core_effect_enums.len()
        ));
    }
    let enum_variants = core_effect_enums[0]
        .variants
        .iter()
        .map(|variant| variant.ident.to_string())
        .collect::<HashSet<_>>();

    let mut launch_contracts = HashMap::new();
    for &(variant, launcher, sink) in CORE_EFFECT_LAUNCH_CONTRACTS {
        if launch_contracts
            .insert(variant.to_string(), (launcher, sink))
            .is_some()
        {
            return Err(format!(
                "duplicate launch contract for CoreEffect::{}",
                variant
            ));
        }
    }
    let mut invalidation_contracts = HashMap::new();
    for &(variant, worker_registry) in CORE_EFFECT_INVALIDATION_CONTRACTS {
        if invalidation_contracts
            .insert(variant.to_string(), worker_registry)
            .is_some()
        {
            return Err(format!(
                "duplicate invalidation contract for CoreEffect::{}",
                variant
            ));
        }
    }
    if launch_contracts
        .keys()
        .any(|variant| invalidation_contracts.contains_key(variant))
    {
        return Err("a CoreEffect variant cannot be both launched and invalidated".to_string());
    }
    let expected_variants = launch_contracts
        .keys()
        .chain(invalidation_contracts.keys())
        .cloned()
        .collect::<HashSet<_>>();
    if enum_variants != expected_variants {
        return Err(format!(
            "CoreEffect enum variants must exactly match the audited totality contracts ({})",
            core_effect_set_difference(&enum_variants, &expected_variants)
        ));
    }

    let run_effect = find_core_effect_runner_method(&runner_syntax, None, "run_effect", true)?;
    verify_core_effect_executor_signature(run_effect)?;
    let dispatch_match = match run_effect.block.stmts.as_slice() {
        [syn::Stmt::Expr(syn::Expr::Match(dispatch), None)]
            if expression_is_simple_path(dispatch.expr.as_ref(), &["effect"]) =>
        {
            dispatch
        }
        _ => {
            return Err(
                "CoreEffectRunner::run_effect must contain exactly one `match effect` expression"
                    .to_string(),
            );
        }
    };

    let mut dispatch_arms = HashMap::new();
    for arm in &dispatch_match.arms {
        if arm.guard.is_some() {
            return Err(format!(
                "CoreEffect dispatch arm at line {} must not use a match guard",
                arm.span().start().line
            ));
        }
        let variant = exact_core_effect_pattern_variant(&arm.pat).map_err(|error| {
            format!(
                "CoreEffect dispatch arm at line {} must use one exact CoreEffect variant pattern: {error}",
                arm.pat.span().start().line
            )
        })?;
        if dispatch_arms.insert(variant.clone(), arm).is_some() {
            return Err(format!(
                "CoreEffect::{variant} must have exactly one dispatch arm"
            ));
        }
    }
    let dispatched_variants = dispatch_arms.keys().cloned().collect::<HashSet<_>>();
    if dispatched_variants != enum_variants {
        return Err(format!(
            "CoreEffect match arms must exactly match the enum ({})",
            core_effect_set_difference(&dispatched_variants, &enum_variants)
        ));
    }

    for (variant, arm) in dispatch_arms {
        let syn::Expr::Block(body) = arm.body.as_ref() else {
            return Err(format!(
                "CoreEffect::{variant} dispatch must use an explicit block"
            ));
        };
        if let Some(contract) = invalidation_contracts.get(&variant) {
            verify_core_effect_invalidation_arm(&variant, &body.block, contract)?;
        } else {
            let contract = launch_contracts
                .get(&variant)
                .ok_or_else(|| format!("missing launch contract for CoreEffect::{variant}"))?;
            verify_core_effect_launch_arm(&variant, &body.block, contract.0)?;
        }
    }

    for &(launcher_name, sink) in launch_contracts.values() {
        let launcher = find_core_effect_runner_method(&runner_syntax, None, launcher_name, false)?;
        verify_core_effect_launcher(launcher, launcher_name, sink)?;
    }

    let turn_submission_worker =
        find_production_function(&turn_submission_syntax, "spawn_turn_submission_worker")?;
    verify_unconditional_worker_sink(
        "core_turn_submission::spawn_turn_submission_worker",
        &turn_submission_worker.block,
        &["spawn_worker_with_panic_fallback"],
        false,
    )?;

    let trait_run_effect = find_core_effect_runner_method(
        &runner_syntax,
        Some("CoreEffectExecutor"),
        "run_effect",
        false,
    )?;
    verify_core_effect_executor_signature(trait_run_effect)?;
    verify_exact_core_effect_trait_delegate(trait_run_effect)?;

    Ok(())
}

fn assert_core_effect_totality_rejects(
    effect_source: &str,
    runner_source: &str,
    turn_submission_source: &str,
    expected_error: &str,
) {
    let error =
        verify_core_effect_totality_contract(effect_source, runner_source, turn_submission_source)
            .expect_err("mutated CoreEffect contract must be rejected");
    assert!(
        error.contains(expected_error),
        "unexpected CoreEffect analyzer failure; expected `{expected_error}`, got `{error}`"
    );
}

fn core_effect_set_difference(actual: &HashSet<String>, expected: &HashSet<String>) -> String {
    let mut unexpected = actual.difference(expected).cloned().collect::<Vec<_>>();
    let mut missing = expected.difference(actual).cloned().collect::<Vec<_>>();
    unexpected.sort();
    missing.sort();
    format!("unexpected={unexpected:?}, missing={missing:?}")
}

fn find_core_effect_runner_method<'a>(
    syntax: &'a syn::File,
    trait_name: Option<&str>,
    method_name: &str,
    require_public: bool,
) -> Result<&'a syn::ImplItemFn, String> {
    let methods = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item)
                if !attributes_are_test_only(&item.attrs)
                    && type_is_simple_path(item.self_ty.as_ref(), &["CoreEffectRunner"])
                    && match (trait_name, item.trait_.as_ref()) {
                        (None, None) => true,
                        (Some(expected), Some((_, path, _))) => path_is_simple(path, &[expected]),
                        _ => false,
                    } =>
            {
                Some(item)
            }
            _ => None,
        })
        .flat_map(|item| item.items.iter())
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method)
                if !attributes_are_test_only(&method.attrs)
                    && method.sig.ident == method_name
                    && (!require_public || matches!(method.vis, syn::Visibility::Public(_))) =>
            {
                Some(method)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if methods.len() != 1 {
        let owner = trait_name.unwrap_or("CoreEffectRunner");
        return Err(format!(
            "expected one production {owner}::{method_name} method, found {}",
            methods.len()
        ));
    }
    Ok(methods[0])
}

fn find_production_function<'a>(
    syntax: &'a syn::File,
    function_name: &str,
) -> Result<&'a syn::ItemFn, String> {
    let functions = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function)
                if !attributes_are_test_only(&function.attrs)
                    && function.sig.ident == function_name =>
            {
                Some(function)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if functions.len() != 1 {
        return Err(format!(
            "expected one production {function_name} function, found {}",
            functions.len()
        ));
    }
    Ok(functions[0])
}

fn verify_core_effect_executor_signature(method: &syn::ImplItemFn) -> Result<(), String> {
    let mut inputs = method.sig.inputs.iter();
    let Some(syn::FnArg::Receiver(receiver)) = inputs.next() else {
        return Err("run_effect must receive &self first".to_string());
    };
    if receiver.reference.is_none() || receiver.mutability.is_some() {
        return Err("run_effect must receive immutable &self".to_string());
    }
    let Some(syn::FnArg::Typed(effect)) = inputs.next() else {
        return Err("run_effect must receive effect: CoreEffect second".to_string());
    };
    if inputs.next().is_some()
        || !matches!(
            effect.pat.as_ref(),
            syn::Pat::Ident(ident)
                if ident.ident == "effect"
                    && ident.by_ref.is_none()
                    && ident.mutability.is_none()
                    && ident.subpat.is_none()
        )
        || !type_is_simple_path(effect.ty.as_ref(), &["CoreEffect"])
    {
        return Err("run_effect signature must be (&self, effect: CoreEffect)".to_string());
    }
    if !type_is_option_of(&method.sig.output, "CoreInput") {
        return Err("run_effect must return Option<CoreInput>".to_string());
    }
    Ok(())
}

fn type_is_option_of(output: &syn::ReturnType, inner_type: &str) -> bool {
    let syn::ReturnType::Type(_, ty) = output else {
        return false;
    };
    let syn::Type::Path(option) = ty.as_ref() else {
        return false;
    };
    if option.qself.is_some()
        || option.path.leading_colon.is_some()
        || option.path.segments.len() != 1
    {
        return false;
    }
    let segment = &option.path.segments[0];
    if segment.ident != "Option" {
        return false;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    matches!(
        arguments.args.iter().collect::<Vec<_>>().as_slice(),
        [syn::GenericArgument::Type(inner)] if type_is_simple_path(inner, &[inner_type])
    )
}

fn type_is_simple_path(ty: &syn::Type, segments: &[&str]) -> bool {
    matches!(
        ty,
        syn::Type::Path(path) if path.qself.is_none() && path_is_simple(&path.path, segments)
    )
}

fn path_is_simple(path: &syn::Path, expected: &[&str]) -> bool {
    path.leading_colon.is_none()
        && path.segments.len() == expected.len()
        && path
            .segments
            .iter()
            .zip(expected)
            .all(|(segment, expected)| {
                segment.ident == *expected && matches!(segment.arguments, syn::PathArguments::None)
            })
}

fn expression_is_simple_path(expression: &syn::Expr, expected: &[&str]) -> bool {
    matches!(
        expression,
        syn::Expr::Path(path) if path.qself.is_none() && path_is_simple(&path.path, expected)
    )
}

fn exact_core_effect_pattern_variant(pattern: &syn::Pat) -> Result<String, &'static str> {
    let path = match pattern {
        syn::Pat::Struct(pattern) => &pattern.path,
        syn::Pat::TupleStruct(pattern) => &pattern.path,
        syn::Pat::Path(pattern) if pattern.qself.is_none() => &pattern.path,
        syn::Pat::Or(_) => return Err("or-patterns are forbidden"),
        syn::Pat::Wild(_) => return Err("wildcard patterns are forbidden"),
        _ => return Err("only struct, tuple-struct, or unit variant patterns are allowed"),
    };
    if path.leading_colon.is_some() || path.segments.len() != 2 {
        return Err("the pattern path must be exactly CoreEffect::<Variant>");
    }
    let mut segments = path.segments.iter();
    let core_effect = segments.next().expect("path length checked");
    let variant = segments.next().expect("path length checked");
    if core_effect.ident != "CoreEffect"
        || !matches!(core_effect.arguments, syn::PathArguments::None)
        || !matches!(variant.arguments, syn::PathArguments::None)
    {
        return Err("the pattern path must be exactly CoreEffect::<Variant>");
    }
    Ok(variant.ident.to_string())
}

fn verify_core_effect_invalidation_arm(
    variant: &str,
    body: &syn::Block,
    worker_registry: &str,
) -> Result<(), String> {
    let [invalidate_statement, none_statement] = body.stmts.as_slice() else {
        return Err(format!(
            "CoreEffect::{variant} must contain exact invalidate + None statements"
        ));
    };
    let syn::Stmt::Expr(syn::Expr::MethodCall(invalidate), Some(_)) = invalidate_statement else {
        return Err(format!(
            "CoreEffect::{variant} must call its worker-registry invalidate method directly"
        ));
    };
    if invalidate.method != "invalidate"
        || !expression_is_self_field(invalidate.receiver.as_ref(), worker_registry)
        || !matches!(
            invalidate.args.iter().collect::<Vec<_>>().as_slice(),
            [argument] if expression_is_field_path(argument, "correlation", "generation")
        )
    {
        return Err(format!(
            "CoreEffect::{variant} must call self.{}.invalidate(correlation.generation) exactly",
            worker_registry
        ));
    }
    if !statement_is_final_none(none_statement) {
        return Err(format!("CoreEffect::{variant} must end with None"));
    }
    let mut audit = CoreEffectDispatchVisitor::default();
    audit.visit_block(body);
    if !audit.spawn_calls.is_empty()
        || !audit.returns.is_empty()
        || audit.try_count != 0
        || audit.effect_completed_calls != 0
    {
        return Err(format!(
            "CoreEffect::{variant} invalidation must not launch, return, fail, or complete"
        ));
    }
    Ok(())
}

fn verify_core_effect_launch_arm(
    variant: &str,
    body: &syn::Block,
    launcher: &str,
) -> Result<(), String> {
    if !body.stmts.last().is_some_and(statement_is_final_none) {
        return Err(format!("CoreEffect::{variant} must end with None"));
    }

    let direct_launchers = body
        .stmts
        .iter()
        .filter_map(direct_self_spawn_statement)
        .collect::<Vec<_>>();
    if direct_launchers.len() != 1 || direct_launchers[0] != launcher {
        return Err(format!(
            "CoreEffect::{variant} must have one unconditional top-level launcher `self.{}(...)`; found {direct_launchers:?}",
            launcher
        ));
    }
    verify_core_effect_dispatch_shape(variant, body, launcher)?;

    let mut audit = CoreEffectDispatchVisitor::default();
    audit.visit_block(body);
    if audit.spawn_calls.len() != 1 || audit.spawn_calls[0] != launcher {
        return Err(format!(
            "CoreEffect::{variant} must call only its audited launcher `self.{}(...)`; found {:?}",
            launcher, audit.spawn_calls
        ));
    }
    if audit.try_count != 0 {
        return Err(format!(
            "CoreEffect::{variant} dispatch must not use ? before settlement"
        ));
    }

    if variant == "PollGithubReview" {
        if audit.returns.as_slice() != [CoreEffectReturnKind::EffectCompletedSome]
            || audit.effect_completed_calls != 1
            || body
                .stmts
                .iter()
                .filter(|statement| statement_is_exact_early_effect_completion(statement))
                .count()
                != 1
        {
            return Err(
                "CoreEffect::PollGithubReview may only use one exact early Some(CoreInput::EffectCompleted(...)) from a top-level let-else"
                    .to_string(),
            );
        }
    } else {
        if !audit.returns.is_empty() {
            return Err(format!(
                "CoreEffect::{variant} must not return from dispatch; completion must re-enter through the worker sink"
            ));
        }
        if audit.effect_completed_calls != 0 {
            return Err(format!(
                "CoreEffect::{variant} must not construct inline CoreInput::EffectCompleted"
            ));
        }
    }
    Ok(())
}

fn verify_core_effect_dispatch_shape(
    variant: &str,
    body: &syn::Block,
    launcher: &str,
) -> Result<(), String> {
    let shape_is_exact = match (variant, body.stmts.as_slice()) {
        ("SetupGithubReviewPolling", [setup_statement, launch_statement, none_statement]) => {
            statement_is_exact_github_review_setup_begin(setup_statement)
                && statement_is_exact_launcher(launch_statement, launcher)
                && statement_is_final_none(none_statement)
        }
        ("PollGithubReview", [lookup_statement, launch_statement, none_statement]) => {
            statement_is_exact_github_poll_service_lookup(lookup_statement)
                && statement_is_exact_launcher(launch_statement, launcher)
                && statement_is_final_none(none_statement)
        }
        ("PrepareManualPrompt", [register_statement, launch_statement, none_statement]) => {
            statement_is_exact_worker_registration(
                register_statement,
                "manual_prompt_workers",
                "request",
                &["correlation", "generation"],
            ) && statement_is_exact_launcher(launch_statement, launcher)
                && statement_is_final_none(none_statement)
        }
        ("RequestStopAllSessions", [register_statement, launch_statement, none_statement]) => {
            statement_is_exact_worker_registration(
                register_statement,
                "stop_request_workers",
                "correlation",
                &["generation"],
            ) && statement_is_exact_launcher(launch_statement, launcher)
                && statement_is_final_none(none_statement)
        }
        (_, [launch_statement, none_statement]) => {
            statement_is_exact_launcher(launch_statement, launcher)
                && statement_is_final_none(none_statement)
        }
        _ => false,
    };
    if !shape_is_exact {
        return Err(format!(
            "CoreEffect::{variant} must use its exact audited dispatch shape before completion"
        ));
    }
    Ok(())
}

fn statement_is_exact_launcher(statement: &syn::Stmt, launcher: &str) -> bool {
    direct_self_spawn_statement(statement).as_deref() == Some(launcher)
}

fn statement_is_exact_github_review_setup_begin(statement: &syn::Stmt) -> bool {
    let syn::Stmt::Expr(syn::Expr::MethodCall(begin), Some(_)) = statement else {
        return false;
    };
    begin.method == "begin"
        && expression_is_self_field(begin.receiver.as_ref(), "github_review_polling_services")
        && matches!(
            begin.args.iter().collect::<Vec<_>>().as_slice(),
            [syn::Expr::MethodCall(clone)]
                if clone.method == "clone"
                    && clone.args.is_empty()
                    && expression_is_simple_path(clone.receiver.as_ref(), &["correlation"])
        )
}

fn statement_is_exact_worker_registration(
    statement: &syn::Stmt,
    worker_registry: &str,
    correlation_base: &str,
    correlation_fields: &[&str],
) -> bool {
    let syn::Stmt::Local(local) = statement else {
        return false;
    };
    if !matches!(
        &local.pat,
        syn::Pat::Ident(permit)
            if permit.ident == "permit"
                && permit.by_ref.is_none()
                && permit.mutability.is_none()
                && permit.subpat.is_none()
    ) {
        return false;
    }
    let Some(initializer) = &local.init else {
        return false;
    };
    if initializer.diverge.is_some() {
        return false;
    }
    let syn::Expr::MethodCall(register) = initializer.expr.as_ref() else {
        return false;
    };
    register.method == "register"
        && expression_is_self_field(register.receiver.as_ref(), worker_registry)
        && matches!(
            register.args.iter().collect::<Vec<_>>().as_slice(),
            [generation]
                if expression_is_field_chain(
                    generation,
                    correlation_base,
                    correlation_fields,
                )
        )
}

fn statement_is_exact_github_poll_service_lookup(statement: &syn::Stmt) -> bool {
    let syn::Stmt::Local(local) = statement else {
        return false;
    };
    if !matches!(
        &local.pat,
        syn::Pat::TupleStruct(some)
            if path_is_simple(&some.path, &["Some"])
                && matches!(
                    some.elems.iter().collect::<Vec<_>>().as_slice(),
                    [syn::Pat::Ident(service)]
                        if service.ident == "service"
                            && service.by_ref.is_none()
                            && service.mutability.is_none()
                            && service.subpat.is_none()
                )
    ) {
        return false;
    }
    let Some(initializer) = &local.init else {
        return false;
    };
    let syn::Expr::MethodCall(service_for) = initializer.expr.as_ref() else {
        return false;
    };
    service_for.method == "service_for"
        && expression_is_self_field(
            service_for.receiver.as_ref(),
            "github_review_polling_services",
        )
        && matches!(
            service_for.args.iter().collect::<Vec<_>>().as_slice(),
            [syn::Expr::Reference(reference)]
                if reference.mutability.is_none()
                    && expression_is_simple_path(
                        reference.expr.as_ref(),
                        &["setup_correlation"],
                    )
        )
        && statement_is_exact_early_effect_completion(statement)
}

fn direct_self_spawn_statement(statement: &syn::Stmt) -> Option<String> {
    let syn::Stmt::Expr(syn::Expr::MethodCall(call), Some(_)) = statement else {
        return None;
    };
    if !expression_is_simple_path(call.receiver.as_ref(), &["self"])
        || !call.method.to_string().starts_with("spawn_")
    {
        return None;
    }
    Some(call.method.to_string())
}

fn statement_is_final_none(statement: &syn::Stmt) -> bool {
    matches!(
        statement,
        syn::Stmt::Expr(expression, None)
            if expression_is_simple_path(expression, &["None"])
    )
}

fn expression_is_self_field(expression: &syn::Expr, expected_field: &str) -> bool {
    matches!(
        expression,
        syn::Expr::Field(field)
            if expression_is_simple_path(field.base.as_ref(), &["self"])
                && matches!(&field.member, syn::Member::Named(member) if member == expected_field)
    )
}

fn expression_is_field_path(
    expression: &syn::Expr,
    expected_base: &str,
    expected_field: &str,
) -> bool {
    matches!(
        expression,
        syn::Expr::Field(field)
            if expression_is_simple_path(field.base.as_ref(), &[expected_base])
                && matches!(&field.member, syn::Member::Named(member) if member == expected_field)
    )
}

fn expression_is_field_chain(
    expression: &syn::Expr,
    expected_base: &str,
    expected_fields: &[&str],
) -> bool {
    let Some((expected_field, parent_fields)) = expected_fields.split_last() else {
        return expression_is_simple_path(expression, &[expected_base]);
    };
    matches!(
        expression,
        syn::Expr::Field(field)
            if matches!(&field.member, syn::Member::Named(member) if member == *expected_field)
                && expression_is_field_chain(
                    field.base.as_ref(),
                    expected_base,
                    parent_fields,
                )
    )
}

fn statement_is_exact_early_effect_completion(statement: &syn::Stmt) -> bool {
    let syn::Stmt::Local(local) = statement else {
        return false;
    };
    let Some(initializer) = &local.init else {
        return false;
    };
    let Some((_, diverge)) = &initializer.diverge else {
        return false;
    };
    let syn::Expr::Block(diverge) = diverge.as_ref() else {
        return false;
    };
    matches!(
        diverge.block.stmts.as_slice(),
        [syn::Stmt::Expr(syn::Expr::Return(return_expression), Some(_))]
            if return_expression.expr.as_deref().is_some_and(expression_is_effect_completed_some)
    )
}

fn expression_is_effect_completed_some(expression: &syn::Expr) -> bool {
    let syn::Expr::Call(some) = expression else {
        return false;
    };
    if !expression_is_simple_path(some.func.as_ref(), &["Some"]) || some.args.len() != 1 {
        return false;
    }
    let Some(syn::Expr::Call(completed)) = some.args.first() else {
        return false;
    };
    expression_is_simple_path(completed.func.as_ref(), &["CoreInput", "EffectCompleted"])
        && completed.args.len() == 1
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoreEffectReturnKind {
    None,
    EffectCompletedSome,
    Other,
}

#[derive(Default)]
struct CoreEffectDispatchVisitor {
    spawn_calls: Vec<String>,
    returns: Vec<CoreEffectReturnKind>,
    try_count: usize,
    effect_completed_calls: usize,
}

impl<'ast> Visit<'ast> for CoreEffectDispatchVisitor {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            if let Some(last) = path.path.segments.last() {
                let name = last.ident.to_string();
                if worker_launch_name(&name) {
                    self.spawn_calls.push(name);
                }
            }
            if path.qself.is_none() && path_is_simple(&path.path, &["CoreInput", "EffectCompleted"])
            {
                self.effect_completed_calls += 1;
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let name = call.method.to_string();
        if worker_launch_name(&name) {
            self.spawn_calls.push(name);
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_return(&mut self, expression: &'ast syn::ExprReturn) {
        self.returns.push(match expression.expr.as_deref() {
            Some(value) if expression_is_simple_path(value, &["None"]) => {
                CoreEffectReturnKind::None
            }
            Some(value) if expression_is_effect_completed_some(value) => {
                CoreEffectReturnKind::EffectCompletedSome
            }
            _ => CoreEffectReturnKind::Other,
        });
        visit::visit_expr_return(self, expression);
    }

    fn visit_expr_try(&mut self, expression: &'ast syn::ExprTry) {
        self.try_count += 1;
        visit::visit_expr_try(self, expression);
    }
}

fn worker_launch_name(name: &str) -> bool {
    name == "spawn" || name.starts_with("spawn_")
}

fn verify_core_effect_launcher(
    method: &syn::ImplItemFn,
    launcher: &str,
    sink: &str,
) -> Result<(), String> {
    if !matches!(method.sig.output, syn::ReturnType::Default) {
        return Err(format!("CoreEffect launcher {launcher} must return unit"));
    }
    let expected_path = sink.split("::").collect::<Vec<_>>();
    verify_unconditional_worker_sink(
        launcher,
        &method.block,
        &expected_path,
        sink != "core_turn_submission::spawn_turn_submission_worker",
    )
}

fn verify_unconditional_worker_sink(
    owner: &str,
    body: &syn::Block,
    expected_path: &[&str],
    forbid_direct_completion: bool,
) -> Result<(), String> {
    let direct_sinks = body
        .stmts
        .iter()
        .filter_map(direct_function_call_statement_path)
        .filter(|path| {
            path.len() == expected_path.len()
                && path
                    .iter()
                    .zip(expected_path)
                    .all(|(actual, expected)| actual == expected)
        })
        .count();

    let mut launch_audit = WorkerSinkVisitor::default();
    launch_audit.visit_block(body);
    let expected_path_text = expected_path.join("::");
    if direct_sinks != 1
        || launch_audit.launch_calls.len() != 1
        || launch_audit.launch_calls[0] != expected_path_text
    {
        return Err(format!(
            "{owner} must call one audited top-level sink `{expected_path_text}(...)`; found direct={direct_sinks}, all={:?}",
            launch_audit.launch_calls
        ));
    }
    if forbid_direct_completion
        && (launch_audit.send_calls != 0 || launch_audit.effect_completed_calls != 0)
    {
        return Err(format!(
            "{owner} must not publish around its exactly-once completion sink"
        ));
    }

    let mut bypass_audit = OuterWorkerBypassVisitor::default();
    bypass_audit.visit_block(body);
    if bypass_audit.return_count != 0 || bypass_audit.try_count != 0 {
        return Err(format!(
            "{owner} must reach its top-level worker sink unconditionally (return={}, ?={})",
            bypass_audit.return_count, bypass_audit.try_count
        ));
    }
    Ok(())
}

fn direct_function_call_statement_path(statement: &syn::Stmt) -> Option<Vec<String>> {
    let syn::Stmt::Expr(syn::Expr::Call(call), Some(_)) = statement else {
        return None;
    };
    let syn::Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    if path.qself.is_some()
        || path.path.leading_colon.is_some()
        || path
            .path
            .segments
            .iter()
            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
    {
        return None;
    }
    Some(
        path.path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect(),
    )
}

#[derive(Default)]
struct WorkerSinkVisitor {
    launch_calls: Vec<String>,
    send_calls: usize,
    effect_completed_calls: usize,
}

impl<'ast> Visit<'ast> for WorkerSinkVisitor {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            let path_text = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            if path
                .path
                .segments
                .last()
                .is_some_and(|segment| worker_launch_name(&segment.ident.to_string()))
            {
                self.launch_calls.push(path_text);
            }
            if path.qself.is_none() && path_is_simple(&path.path, &["CoreInput", "EffectCompleted"])
            {
                self.effect_completed_calls += 1;
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let name = call.method.to_string();
        if worker_launch_name(&name) {
            self.launch_calls.push(format!("<method>::{name}"));
        }
        if name == "send" {
            self.send_calls += 1;
        }
        visit::visit_expr_method_call(self, call);
    }
}

#[derive(Default)]
struct OuterWorkerBypassVisitor {
    return_count: usize,
    try_count: usize,
}

impl<'ast> Visit<'ast> for OuterWorkerBypassVisitor {
    fn visit_expr_closure(&mut self, _expression: &'ast syn::ExprClosure) {}

    fn visit_expr_async(&mut self, _expression: &'ast syn::ExprAsync) {}

    fn visit_expr_return(&mut self, expression: &'ast syn::ExprReturn) {
        self.return_count += 1;
        visit::visit_expr_return(self, expression);
    }

    fn visit_expr_try(&mut self, expression: &'ast syn::ExprTry) {
        self.try_count += 1;
        visit::visit_expr_try(self, expression);
    }
}

fn verify_exact_core_effect_trait_delegate(method: &syn::ImplItemFn) -> Result<(), String> {
    let [syn::Stmt::Expr(syn::Expr::Call(delegate), None)] = method.block.stmts.as_slice() else {
        return Err(
            "CoreEffectExecutor::run_effect must delegate exactly with one tail expression"
                .to_string(),
        );
    };
    if !expression_is_simple_path(delegate.func.as_ref(), &["CoreEffectRunner", "run_effect"])
        || !matches!(
            delegate.args.iter().collect::<Vec<_>>().as_slice(),
            [receiver, effect]
                if expression_is_simple_path(receiver, &["self"])
                    && expression_is_simple_path(effect, &["effect"])
        )
    {
        return Err(
            "CoreEffectExecutor::run_effect must delegate exactly to CoreEffectRunner::run_effect(self, effect)"
                .to_string(),
        );
    }
    Ok(())
}

type MethodCallLocation = (String, usize);
type ImplMethodCallSummary = (String, Vec<MethodCallLocation>, bool);

fn top_level_impl_method_source(source: &str, method_name: &str) -> String {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("architecture source must parse as Rust: {error}"));
    let methods = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item) if !attributes_are_test_only(&item.attrs) => Some(item),
            _ => None,
        })
        .flat_map(|item| item.items.iter())
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method)
                if !attributes_are_test_only(&method.attrs) && method.sig.ident == method_name =>
            {
                Some(method)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        methods.len(),
        1,
        "expected one non-test impl method named {method_name}"
    );
    let span = methods[0].span();
    source
        .lines()
        .skip(span.start().line.saturating_sub(1))
        .take(
            span.end()
                .line
                .saturating_sub(span.start().line)
                .saturating_add(1),
        )
        .collect::<Vec<_>>()
        .join("\n")
}

fn top_level_function_source(source: &str, function_name: &str) -> String {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("architecture source must parse as Rust: {error}"));
    let functions = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function)
                if !attributes_are_test_only(&function.attrs)
                    && function.sig.ident == function_name =>
            {
                Some(function)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        functions.len(),
        1,
        "expected one non-test function named {function_name}"
    );
    let span = functions[0].span();
    source
        .lines()
        .skip(span.start().line.saturating_sub(1))
        .take(
            span.end()
                .line
                .saturating_sub(span.start().line)
                .saturating_add(1),
        )
        .collect::<Vec<_>>()
        .join("\n")
}

fn top_level_impl_method_calls(source: &str) -> Vec<ImplMethodCallSummary> {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("architecture source must parse as Rust: {error}"));
    let mut methods = Vec::new();
    for item in syntax.items {
        let syn::Item::Impl(item) = item else {
            continue;
        };
        if attributes_are_test_only(&item.attrs) {
            continue;
        }
        for item in item.items {
            let syn::ImplItem::Fn(method) = item else {
                continue;
            };
            if attributes_are_test_only(&method.attrs) {
                continue;
            }
            let mut visitor = MethodCallLineVisitor::default();
            visitor.visit_block(&method.block);
            methods.push((
                method.sig.ident.to_string(),
                visitor.calls,
                method
                    .block
                    .stmts
                    .first()
                    .is_some_and(statement_is_planning_reset_busy_return_guard),
            ));
        }
    }
    methods
}

fn statement_is_planning_reset_busy_return_guard(statement: &syn::Stmt) -> bool {
    let syn::Stmt::Expr(syn::Expr::If(guard), None) = statement else {
        return false;
    };
    if guard.else_branch.is_some() {
        return false;
    }
    let syn::Expr::MethodCall(condition) = guard.cond.as_ref() else {
        return false;
    };
    if condition.method != "planning_workspace_operation_blocks_direct_mutation"
        || !condition.args.is_empty()
        || !matches!(
            condition.receiver.as_ref(),
            syn::Expr::Path(receiver) if receiver.qself.is_none() && receiver.path.is_ident("self")
        )
    {
        return false;
    }
    matches!(
        guard.then_branch.stmts.as_slice(),
        [syn::Stmt::Expr(syn::Expr::Return(return_expression), Some(_))]
            if return_expression.expr.is_none()
    )
}

#[derive(Default)]
struct MethodCallLineVisitor {
    calls: Vec<(String, usize)>,
}

impl<'ast> Visit<'ast> for MethodCallLineVisitor {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.calls
            .push((call.method.to_string(), call.method.span().start().line));
        visit::visit_expr_method_call(self, call);
    }
}

fn test_only_item_line_ranges(source: &str) -> Vec<(usize, usize)> {
    let syntax = syn::parse_file(source)
        .unwrap_or_else(|error| panic!("architecture source must parse as Rust: {error}"));
    let mut visitor = TestOnlyItemRangeVisitor::default();
    visitor.visit_file(&syntax);
    visitor.ranges
}

#[derive(Default)]
struct TestOnlyItemRangeVisitor {
    ranges: Vec<(usize, usize)>,
}

impl<'ast> Visit<'ast> for TestOnlyItemRangeVisitor {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if let Some(range) = test_only_line_range(item_attributes(item), item) {
            self.ranges.push(range);
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if let Some(range) = test_only_line_range(impl_item_attributes(item), item) {
            self.ranges.push(range);
            return;
        }
        visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if let Some(range) = test_only_line_range(trait_item_attributes(item), item) {
            self.ranges.push(range);
            return;
        }
        visit::visit_trait_item(self, item);
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        if let Some(range) = test_only_line_range(foreign_item_attributes(item), item) {
            self.ranges.push(range);
            return;
        }
        visit::visit_foreign_item(self, item);
    }
}

fn test_only_line_range<T: Spanned>(
    attributes: Option<&[syn::Attribute]>,
    item: &T,
) -> Option<(usize, usize)> {
    let attributes = attributes.filter(|attributes| attributes_are_test_only(attributes))?;
    let item_span = item.span();
    let start = attributes.first().map_or_else(
        || item_span.start().line,
        |attribute| attribute.span().start().line,
    );
    Some((start, item_span.end().line))
}

fn rust_code_without_comments_and_literals(source: &str) -> String {
    let chars = source.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(source.len());
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '/' && chars.get(index + 1) == Some(&'/') {
            while index < chars.len() && chars[index] != '\n' {
                output.push(' ');
                index += 1;
            }
            continue;
        }

        if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
            let mut depth = 1usize;
            output.push(' ');
            output.push(' ');
            index += 2;
            while index < chars.len() && depth > 0 {
                if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
                    output.push(' ');
                    output.push(' ');
                    index += 2;
                    depth += 1;
                } else if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
                    output.push(' ');
                    output.push(' ');
                    index += 2;
                    depth -= 1;
                } else {
                    push_masked_character(&mut output, chars[index]);
                    index += 1;
                }
            }
            continue;
        }

        if let Some((quote_index, hash_count)) = raw_string_opening(&chars, index) {
            while index <= quote_index {
                output.push(' ');
                index += 1;
            }
            while index < chars.len() {
                if chars[index] == '"'
                    && (0..hash_count).all(|offset| chars.get(index + 1 + offset) == Some(&'#'))
                {
                    output.push(' ');
                    index += 1;
                    for _ in 0..hash_count {
                        output.push(' ');
                        index += 1;
                    }
                    break;
                }
                push_masked_character(&mut output, chars[index]);
                index += 1;
            }
            continue;
        }

        if chars[index] == '"' {
            output.push(' ');
            index += 1;
            let mut escaped = false;
            while index < chars.len() {
                let character = chars[index];
                push_masked_character(&mut output, character);
                index += 1;
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    break;
                }
            }
            continue;
        }

        if let Some(end) = char_literal_end(&chars, index) {
            while index <= end {
                push_masked_character(&mut output, chars[index]);
                index += 1;
            }
            continue;
        }

        output.push(chars[index]);
        index += 1;
    }

    output
}

fn raw_string_opening(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let mut index = start;
    if chars.get(index) == Some(&'b') {
        index += 1;
    }
    if chars.get(index) != Some(&'r') {
        return None;
    }
    index += 1;
    let hash_start = index;
    while chars.get(index) == Some(&'#') {
        index += 1;
    }
    (chars.get(index) == Some(&'"')).then_some((index, index - hash_start))
}

fn char_literal_end(chars: &[char], start: usize) -> Option<usize> {
    if chars.get(start) != Some(&'\'') {
        return None;
    }
    let first = *chars.get(start + 1)?;
    if first == '\\' {
        let mut index = start + 2;
        while let Some(character) = chars.get(index) {
            if *character == '\'' {
                return Some(index);
            }
            if *character == '\n' {
                return None;
            }
            index += 1;
        }
        return None;
    }
    (chars.get(start + 2) == Some(&'\'')).then_some(start + 2)
}

fn push_masked_character(output: &mut String, character: char) {
    output.push(if character == '\n' { '\n' } else { ' ' });
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

fn rust_module_paths_from_declarations(
    root_file: &Path,
    root_module_path: &[String],
) -> Result<HashMap<PathBuf, Vec<String>>, String> {
    fn path_attribute(module: &syn::ItemMod) -> Option<PathBuf> {
        module.attrs.iter().find_map(|attribute| {
            if !attribute.path().is_ident("path") {
                return None;
            }
            let syn::Meta::NameValue(value) = &attribute.meta else {
                return None;
            };
            let syn::Expr::Lit(expression) = &value.value else {
                return None;
            };
            let syn::Lit::Str(path) = &expression.lit else {
                return None;
            };
            Some(PathBuf::from(path.value()))
        })
    }

    fn default_child_directory(source_file: &Path) -> PathBuf {
        let parent = source_file.parent().unwrap_or_else(|| Path::new(""));
        let stem = source_file
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if matches!(stem, "lib" | "main" | "mod") {
            parent.to_path_buf()
        } else {
            parent.join(stem)
        }
    }

    fn declared_child_file(base: &Path, module: &syn::ItemMod) -> Result<PathBuf, String> {
        let direct = base.join(format!("{}.rs", module.ident));
        let nested = base.join(module.ident.to_string()).join("mod.rs");
        match (direct.is_file(), nested.is_file()) {
            (true, false) => Ok(direct),
            (false, true) => Ok(nested),
            (true, true) => Err(format!(
                "module `{}` is ambiguous between {} and {}",
                module.ident,
                direct.display(),
                nested.display()
            )),
            (false, false) => Err(format!(
                "module `{}` has no source at {} or {}",
                module.ident,
                direct.display(),
                nested.display()
            )),
        }
    }

    fn walk_items(
        items: &[syn::Item],
        logical_path: &[String],
        module_directory: &Path,
        path_attribute_directory: &Path,
        paths: &mut HashMap<PathBuf, Vec<String>>,
        visited: &mut HashSet<PathBuf>,
    ) -> Result<(), String> {
        for item in items {
            let syn::Item::Mod(module) = item else {
                continue;
            };
            if attributes_are_test_only(&module.attrs) {
                continue;
            }
            let mut child_logical_path = logical_path.to_vec();
            child_logical_path.push(module.ident.to_string());
            if let Some((_, nested_items)) = &module.content {
                let inline_directory = module_directory.join(module.ident.to_string());
                walk_items(
                    nested_items,
                    &child_logical_path,
                    &inline_directory,
                    &inline_directory,
                    paths,
                    visited,
                )?;
                continue;
            }
            let child_file = if let Some(path) = path_attribute(module) {
                path_attribute_directory.join(path)
            } else {
                declared_child_file(module_directory, module)?
            };
            walk_file(&child_file, &child_logical_path, paths, visited)?;
        }
        Ok(())
    }

    fn walk_file(
        source_file: &Path,
        logical_path: &[String],
        paths: &mut HashMap<PathBuf, Vec<String>>,
        visited: &mut HashSet<PathBuf>,
    ) -> Result<(), String> {
        if !source_file.is_file() {
            return Err(format!(
                "declared module source does not exist: {}",
                source_file.display()
            ));
        }
        let source_file = source_file.to_path_buf();
        if let Some(previous) = paths.insert(source_file.clone(), logical_path.to_vec())
            && previous != logical_path
        {
            return Err(format!(
                "module source {} is declared as both {} and {}",
                source_file.display(),
                previous.join("::"),
                logical_path.join("::")
            ));
        }
        if !visited.insert(source_file.clone()) {
            return Ok(());
        }
        let source = fs::read_to_string(&source_file)
            .map_err(|error| format!("failed to read {}: {error}", source_file.display()))?;
        let syntax = syn::parse_file(&source)
            .map_err(|error| format!("failed to parse {}: {error}", source_file.display()))?;
        let module_directory = default_child_directory(&source_file);
        let path_attribute_directory = source_file.parent().unwrap_or_else(|| Path::new(""));
        walk_items(
            &syntax.items,
            logical_path,
            &module_directory,
            path_attribute_directory,
            paths,
            visited,
        )
    }

    let mut paths = HashMap::new();
    walk_file(root_file, root_module_path, &mut paths, &mut HashSet::new())?;
    Ok(paths)
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
