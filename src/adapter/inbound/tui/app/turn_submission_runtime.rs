/* Turn submission is the execution layer for ConversationRuntimeEffect. The
 * reducer decides what should happen; this module gates the prompt against shell
 * readiness, sends stream startup and post-turn planning evaluation through the
 * core runtime, and re-enters the reducer for auto-follow prompts.
 */
#[path = "turn_submission_runtime/post_turn_execution.rs"]
mod post_turn_execution;

use crate::application::service::manual_prompt_preparation::{
    ManualPlanningBootstrapFailureKind, ManualPromptPreparationRequest,
    ManualPromptPreparationResult,
};
use crate::application::service::parallel_mode::turn::ParallelTurnSlotLeaseHandoff;
use crate::application::service::planning::{
    ManualPromptIntakeOutcome, QUEUED_TASK_TRANSCRIPT_TEXT,
};
use crate::core::app::{AppCommand, CorePromptOrigin, TurnSubmissionRequest};
use post_turn_execution::PostTurnEvaluationRequest;

use super::planning_worker_debug_preview::build_debug_preview_lines;
use super::{
    AutoFollowSubmitContext, ConversationInputEvent, ConversationRuntimeEffect,
    ConversationRuntimeEvent, ConversationState, InlineShellCommandInput,
    ManualIntakeSubmitContext, NativeTuiApp, PromptOrigin, ShellActionAvailability,
    ShellChromeEvent,
};

const AUTO_FOLLOW_TRANSCRIPT_DEBUG_MAX_BLOCK_LINES: usize = 32;

impl NativeTuiApp {
    pub(super) fn start_turn_submission(&mut self) {
        // Enter first belongs to inline shell commands. Only non-command prompt
        // text becomes a conversation turn, and only when the current conversation
        // can accept a manual prompt.
        let inline_command = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                InlineShellCommandInput::parse(&conversation.input_buffer)
            }
            _ => None,
        };
        if let Some(command) = inline_command {
            self.execute_inline_shell_command_input(command);
            return;
        }
        let operator_prompt = match &self.conversation_state {
            ConversationState::Ready(conversation) if conversation.can_accept_manual_prompt() => {
                conversation.input_buffer.clone()
            }
            _ => return,
        };
        if operator_prompt.trim().is_empty() {
            return;
        }

        self.submit_manual_prompt_from_text(operator_prompt);
    }

    pub(super) fn execute_conversation_runtime_effect(
        &mut self,
        effect: ConversationRuntimeEffect,
    ) {
        // This switchboard is intentionally thin: stream work and post-turn planning
        // live in submodules, while auto-follow reuses the same submit path as a
        // manual prompt with a different origin.
        match effect {
            ConversationRuntimeEffect::StartStream {
                workspace_directory,
                thread_id,
                prompt,
                prompt_origin,
            } => self.dispatch_core_command(AppCommand::SubmitTurn(
                self.build_turn_submission_request(
                    workspace_directory,
                    thread_id,
                    prompt,
                    &prompt_origin,
                ),
            )),
            ConversationRuntimeEffect::EvaluatePostTurn {
                workspace_directory,
                completed_turn_id,
                changed_planning_file_paths,
                execution_snapshot_capture,
            } => self.execute_post_turn_evaluation(PostTurnEvaluationRequest {
                workspace_directory,
                completed_turn_id,
                changed_planning_file_paths,
                execution_snapshot_capture,
            }),
            ConversationRuntimeEffect::QueueAutoPrompt {
                prompt,
                completed_turn_id,
                mode_label,
                transcript_text,
                handoff_task,
            } => {
                let debug_detail = self.build_auto_follow_transcript_debug_detail(&transcript_text);
                let _ = self.submit_prompt(
                    prompt,
                    PromptOrigin::AutoFollow(Box::new(AutoFollowSubmitContext {
                        completed_turn_id,
                        mode_label,
                        transcript_text,
                        debug_detail,
                        handoff_task,
                    })),
                );
            }
            ConversationRuntimeEffect::DispatchOperatorAlert { alert } => {
                let _ = self.tx.send(super::BackgroundMessage::OperatorAlert(alert));
            }
        }
    }

    fn build_turn_submission_request(
        &self,
        workspace_directory: String,
        thread_id: Option<String>,
        prompt: String,
        prompt_origin: &PromptOrigin,
    ) -> TurnSubmissionRequest {
        TurnSubmissionRequest {
            workspace_directory,
            thread_id,
            prompt,
            prompt_origin: core_prompt_origin(prompt_origin),
            turn_options: self.turn_options.clone(),
            slot_lease_handoff: self.build_parallel_mode_slot_lease_handoff(),
        }
    }

    fn build_parallel_mode_slot_lease_handoff(&self) -> Option<ParallelTurnSlotLeaseHandoff> {
        // A slot lease needs a concrete planning handoff so the parallel pool can
        // bind cleanup ownership. Application/domain code owns the lease request and
        // slug policy; the TUI only forwards task identity.
        if !self.parallel_mode_enabled() {
            return None;
        }
        let ConversationState::Ready(conversation) = &self.conversation_state else {
            return None;
        };
        let handoff_task = conversation.last_planning_task_handoff()?;

        Some(ParallelTurnSlotLeaseHandoff::new(
            handoff_task.task_id.clone(),
            handoff_task.task_title.clone(),
        ))
    }

    pub(super) fn sync_active_turn_workspace_directory(&mut self, workspace_directory: &str) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };

        conversation.replace_active_turn_workspace_directory(workspace_directory.to_string());
        self.conversation_state = ConversationState::ready(conversation);
    }

    pub(super) fn resolve_startup_submit_queue(&mut self) {
        let (startup_submit_armed, operator_prompt) = match &self.conversation_state {
            ConversationState::Ready(conversation) => (
                conversation.startup_submit_armed,
                conversation.input_buffer.clone(),
            ),
            ConversationState::Loading | ConversationState::Failed(_) => return,
        };
        if !startup_submit_armed {
            return;
        }

        // Prompts typed during startup checks are replayed only after the shell is
        // action-ready; blocked startup keeps the text in the buffer for the operator.
        match self.shell_action_availability() {
            ShellActionAvailability::Ready if operator_prompt.trim().is_empty() => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: None,
                });
            }
            ShellActionAvailability::Ready => {
                self.submit_manual_prompt_from_text(operator_prompt);
            }
            ShellActionAvailability::Pending => {}
            ShellActionAvailability::Blocked => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: Some(format!(
                        "{}; queued prompt kept in buffer",
                        self.submission_blocked_status(PromptOrigin::Manual)
                    )),
                });
            }
        }
    }

    pub(super) fn submit_manual_prompt_from_text(&mut self, operator_prompt: String) {
        let transcript_text = operator_prompt.trim().to_string();
        if transcript_text.is_empty() {
            return;
        }

        let workspace_directory = self.planning_workspace_directory();
        let (parent_thread_id, parent_turn_id) = match &self.conversation_state {
            ConversationState::Ready(conversation) => (
                Some(conversation.thread_id.clone())
                    .filter(|thread_id| !thread_id.trim().is_empty()),
                conversation.active_turn_id.clone(),
            ),
            ConversationState::Loading | ConversationState::Failed(_) => (None, None),
        };
        self.dispatch_core_command(AppCommand::PrepareManualPrompt(Box::new(
            ManualPromptPreparationRequest {
                workspace_directory,
                raw_prompt: transcript_text,
                parent_thread_id,
                parent_turn_id,
            },
        )));
    }

    pub(super) fn apply_manual_prompt_preparation(
        &mut self,
        result: ManualPromptPreparationResult,
    ) {
        self.sync_ready_conversation_planning_runtime_projection(
            result.runtime_projection().clone(),
        );
        match result {
            ManualPromptPreparationResult::PromptReady {
                transcript_text,
                intake,
                ..
            } => {
                if !self.manual_prompt_preparation_still_matches_input(&transcript_text) {
                    return;
                }
                self.apply_manual_prompt_intake_outcome(*intake, transcript_text);
            }
            ManualPromptPreparationResult::BootstrapReviewRequired {
                transcript_text,
                review,
                ..
            } => {
                if !self.manual_prompt_preparation_still_matches_input(&transcript_text) {
                    return;
                }
                let draft_name = review.draft_name.clone();
                self.planning_init_overlay_ui_state
                    .open_simple_review_summary(
                        review.draft_name,
                        review.staged_file_count,
                        review.validation_report,
                    );
                self.planning_draft_editor_ui_state.reset();
                self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "planning bootstrap promote blocked / draft: {draft_name} / validation needs attention"
                    ),
                });
            }
            ManualPromptPreparationResult::BootstrapFailed {
                transcript_text,
                kind,
                reason,
                ..
            } => {
                if !self.manual_prompt_preparation_still_matches_input(&transcript_text) {
                    return;
                }
                let status_text = match kind {
                    ManualPlanningBootstrapFailureKind::Stage => {
                        format!("planning bootstrap failed: {reason}")
                    }
                    ManualPlanningBootstrapFailureKind::Promote => {
                        format!("planning bootstrap promote failed: {reason}")
                    }
                };
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            ManualPromptPreparationResult::Rejected {
                transcript_text,
                reason,
                ..
            } => {
                if !self.manual_prompt_preparation_still_matches_input(&transcript_text) {
                    return;
                }
                self.dispatch_conversation_input(
                    ConversationInputEvent::ManualPromptPreparationFailed {
                        transcript_text,
                        status_text: format!("turn preparation failed / {reason}"),
                    },
                );
            }
        }
    }

    fn apply_manual_prompt_intake_outcome(
        &mut self,
        outcome: ManualPromptIntakeOutcome,
        transcript_text: String,
    ) {
        match outcome {
            ManualPromptIntakeOutcome::NoTaskNeeded(handoff) => {
                if !self.manual_prompt_preparation_still_matches_input(&handoff.transcript_text) {
                    return;
                }
                let _ = self.submit_prompt_with_transcript(
                    handoff.prompt,
                    handoff.transcript_text,
                    PromptOrigin::Manual,
                );
            }
            ManualPromptIntakeOutcome::TaskCommitted { handoff, .. }
            | ManualPromptIntakeOutcome::TaskUpdated { handoff, .. } => {
                if !self.manual_prompt_preparation_still_matches_input(&handoff.transcript_text) {
                    return;
                }
                let _ = self.submit_prompt_with_transcript(
                    handoff.prompt,
                    handoff.transcript_text.clone(),
                    PromptOrigin::ManualIntake(Box::new(ManualIntakeSubmitContext {
                        transcript_text: handoff.transcript_text,
                        handoff_task: handoff.task,
                    })),
                );
            }
            ManualPromptIntakeOutcome::Rejected { reason }
            | ManualPromptIntakeOutcome::Failed { reason } => {
                self.dispatch_conversation_input(
                    ConversationInputEvent::ManualPromptPreparationFailed {
                        transcript_text,
                        status_text: format!("turn preparation failed / {reason}"),
                    },
                );
            }
        }
    }

    fn manual_prompt_preparation_still_matches_input(&self, transcript_text: &str) -> bool {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.input_buffer.trim() == transcript_text
            }
            ConversationState::Loading | ConversationState::Failed(_) => false,
        }
    }

    pub(super) fn submit_prompt(&mut self, prompt: String, prompt_origin: PromptOrigin) -> bool {
        let transcript_text = match &prompt_origin {
            PromptOrigin::Manual => prompt.trim().to_string(),
            PromptOrigin::ManualIntake(context) => context.transcript_text.clone(),
            PromptOrigin::AutoFollow(context) => context.transcript_text.clone(),
        };
        self.submit_prompt_with_transcript(prompt, transcript_text, prompt_origin)
    }

    pub(super) fn submit_prompt_with_transcript(
        &mut self,
        prompt: String,
        transcript_text: String,
        prompt_origin: PromptOrigin,
    ) -> bool {
        if matches!(
            prompt_origin,
            PromptOrigin::Manual | PromptOrigin::ManualIntake(_)
        ) && matches!(
            self.shell_action_availability(),
            ShellActionAvailability::Pending
        ) {
            self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitArmed {
                status_text: "prompt queued until startup checks finish".to_string(),
            });
            return false;
        }

        if !self.shell_action_availability().allows_actions() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self.submission_blocked_status(prompt_origin),
            });
            return false;
        }

        crate::akra_event!(
            tracing::Level::DEBUG,
            "user_prompt_submit_inspected",
            origin = prompt_origin_label(&prompt_origin),
            transcript_text = transcript_text,
            transcript_text_len = transcript_text.len(),
            prompt = prompt,
            prompt_len = prompt.len(),
            parallel_mode_enabled = self.parallel_mode_enabled(),
        );

        self.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            transcript_text,
            origin: prompt_origin,
        })
    }

    fn build_auto_follow_transcript_debug_detail(&self, transcript_text: &str) -> Option<String> {
        if !self.planning_worker_shows_debug_details()
            || transcript_text != QUEUED_TASK_TRANSCRIPT_TEXT
        {
            return None;
        }
        let planning_worker = &self.planning_worker_panel_state;
        let operation_label = planning_worker
            .last_operation_label
            .as_deref()
            .unwrap_or("unknown");
        let prompt = planning_worker.last_prompt.as_deref();
        let response = planning_worker.last_response.as_deref();
        let summary = planning_worker.last_summary.as_deref();
        if prompt.is_none() && response.is_none() && summary.is_none() {
            return None;
        }
        let mut lines = vec![format!(
            "planning worker temporary session: {operation_label} / {}",
            planning_worker.status.label()
        )];
        if let Some(summary) = summary.filter(|summary: &&str| !summary.trim().is_empty()) {
            lines.push(format!("planning worker summary: {summary}"));
        }
        append_debug_detail_preview_block(&mut lines, "planning worker prompt:", prompt);
        append_debug_detail_preview_block(&mut lines, "planning worker response:", response);

        Some(lines.join("\n"))
    }
}

#[cfg(test)]
fn user_prompt_submit_detail(
    prompt: &str,
    transcript_text: &str,
    prompt_origin: &PromptOrigin,
    parallel_mode_enabled: bool,
) -> serde_json::Value {
    serde_json::json!({
        "origin": prompt_origin_label(prompt_origin),
        "transcript_text": transcript_text,
        "transcript_text_len": transcript_text.len(),
        "prompt": prompt,
        "prompt_len": prompt.len(),
        "parallel_mode_enabled": parallel_mode_enabled,
    })
}

fn prompt_origin_label(prompt_origin: &PromptOrigin) -> &'static str {
    match prompt_origin {
        PromptOrigin::Manual => "Manual",
        PromptOrigin::ManualIntake(_) => "ManualIntake",
        PromptOrigin::AutoFollow(_) => "AutoFollow",
    }
}

fn core_prompt_origin(prompt_origin: &PromptOrigin) -> CorePromptOrigin {
    match prompt_origin {
        PromptOrigin::Manual => CorePromptOrigin::Manual,
        PromptOrigin::ManualIntake(_) => CorePromptOrigin::ManualIntake,
        PromptOrigin::AutoFollow(_) => CorePromptOrigin::AutoFollow,
    }
}

fn append_debug_detail_preview_block(lines: &mut Vec<String>, label: &str, block: Option<&str>) {
    let Some(block) = block.filter(|block| !block.trim().is_empty()) else {
        return;
    };

    lines.push(label.to_string());
    for line in build_debug_preview_lines(block, AUTO_FOLLOW_TRANSCRIPT_DEBUG_MAX_BLOCK_LINES) {
        lines.push(format!("  {line}"));
    }
}

#[cfg(test)]
mod prompt_submit_diagnostics_tests {
    use super::{PromptOrigin, core_prompt_origin, user_prompt_submit_detail};
    use crate::core::app::CorePromptOrigin;

    #[test]
    fn user_prompt_submit_detail_keeps_operator_text_and_final_prompt() {
        let detail = user_prompt_submit_detail(
            "final wrapper\noperator text",
            "operator text",
            &PromptOrigin::Manual,
            true,
        );

        assert_eq!(detail["origin"], "Manual");
        assert_eq!(detail["transcript_text"], "operator text");
        assert_eq!(detail["transcript_text_len"], 13);
        assert_eq!(detail["prompt"], "final wrapper\noperator text");
        assert_eq!(detail["prompt_len"], 27);
        assert_eq!(detail["parallel_mode_enabled"], true);
    }

    #[test]
    fn prompt_origin_maps_to_core_origin_without_tui_context() {
        assert_eq!(
            core_prompt_origin(&PromptOrigin::Manual),
            CorePromptOrigin::Manual
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::test_helpers;
    use crate::adapter::inbound::tui::app::{
        AutoFollowSubmitContext, BackgroundMessage, ConversationInputEvent, ConversationState,
        NativeTuiApp, NativeTuiParallelModeBinding, PlanningInitOverlayStep, PlanningWorkerStatus,
        PlanningWorkerVisibility, ShellOverlay, StartupState,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
    use crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort;
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    use crate::application::port::outbound::startup_probe_port::{
        AppServerStartupContext, StartupProbePort,
    };
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::manual_prompt_preparation::{
        ManualPlanningBootstrapReview, ManualPromptPreparationResult,
    };
    use crate::application::service::parallel_mode::turn::ParallelTurnSlotLeaseHandoff;
    use crate::application::service::planning::{
        ManualPromptIntakeOutcome, ManualPromptMainSessionHandoff, PlanningRuntimeProjection,
        PlanningTaskHandoff,
    };
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::core::app::{CorePromptOrigin, StartupReadySnapshot};
    use crate::domain::conversation::{
        ConversationReasoningEffort, ConversationRuntimeControlTruth, ConversationSnapshot,
    };
    use crate::domain::operator_alert::OperatorAlert;
    use crate::domain::planning::PlanningValidationReport;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};
    use crate::domain::startup_diagnostics::StartupDiagnostics;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    use anyhow::Result;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Default)]
    struct FakeAppServerPort;

    impl StartupProbePort for FakeAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
                initialize_detail: "ok".to_string(),
                account_detail: "ok".to_string(),
                account_ok: true,
                warnings: Vec::new(),
            })
        }
    }

    impl SessionCatalogPort for FakeAppServerPort {
        fn load_session_catalog(&self, _request: SessionCatalogRequest) -> Result<SessionCatalog> {
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into())
        }
    }

    impl InteractiveTurnRuntimePort for FakeAppServerPort {
        fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
            ConversationRuntimeControlTruth::codex_app_server()
        }

        fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
            Ok(ConversationSnapshot {
                thread_id: thread_id.to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/root".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            })
        }

        fn request_stop_all_sessions(&self) -> Result<()> {
            Ok(())
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            Ok(())
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            Ok(())
        }
    }

    struct TempWorkspace {
        path: PathBuf,
        path_text: String,
    }

    impl TempWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::create_dir_all(&path).expect("temp workspace should be created");
            let path_text = path.display().to_string();
            Self { path, path_text }
        }

        fn path_str(&self) -> &str {
            &self.path_text
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn make_test_app(workspace: &TempWorkspace) -> NativeTuiApp {
        let codex_port = Arc::new(FakeAppServerPort);
        let planning = test_helpers::test_planning_services(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        let parallel_mode_control_plane_composition =
            test_helpers::test_parallel_mode_control_plane_composition_with_worker(
                test_helpers::test_parallel_mode_service(),
                planning,
                Arc::new(NoopParallelAgentWorkerPort),
            );
        let parallel_mode_binding =
            NativeTuiParallelModeBinding::from_composition(parallel_mode_control_plane_composition);
        let mut app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            parallel_mode_binding,
        );
        app.startup_state = StartupState::Ready(startup_ready_snapshot(workspace.path_str(), true));
        app.sync_draft_shell_workspace(workspace.path_str());
        app
    }

    fn startup_ready_snapshot(
        workspace_path: &str,
        can_continue: bool,
    ) -> Box<StartupReadySnapshot> {
        Box::new(StartupReadySnapshot::from_diagnostics(StartupDiagnostics {
            cwd: workspace_path.to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "ok".to_string(),
            workspace_ok: true,
            workspace_path: workspace_path.to_string(),
            workspace_detail: "ok".to_string(),
            attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_ok: true,
            initialize_detail: "ok".to_string(),
            account_ok: can_continue,
            account_detail: if can_continue {
                "ok"
            } else {
                "missing account"
            }
            .to_string(),
            warnings: Vec::new(),
            schema_snapshot: "schema".to_string(),
        }))
    }

    fn ready_conversation(app: &NativeTuiApp) -> &super::super::ConversationViewModel {
        match &app.conversation_state {
            ConversationState::Ready(conversation) => conversation,
            other => panic!("conversation should be ready, got {other:?}"),
        }
    }

    fn ready_conversation_mut(app: &mut NativeTuiApp) -> &mut super::super::ConversationViewModel {
        match &mut app.conversation_state {
            ConversationState::Ready(conversation) => conversation,
            other => panic!("conversation should be ready, got {other:?}"),
        }
    }

    fn set_input(app: &mut NativeTuiApp, input: &str) {
        ready_conversation_mut(app).input_buffer = input.to_string();
    }

    fn runtime_projection() -> Box<PlanningRuntimeProjection> {
        Box::new(PlanningRuntimeProjection::ready_with_details(
            "Planning Context".to_string(),
            "queue idle".to_string(),
            None,
            None,
        ))
    }

    fn sample_handoff_task() -> PlanningTaskHandoff {
        PlanningTaskHandoff {
            task_id: "task-1".to_string(),
            task_title: "Implement turn submission coverage".to_string(),
            direction_id: "general-workstream".to_string(),
            combined_priority: 10,
            updated_at: "2026-05-12T00:00:00Z".to_string(),
            status_label: "Ready".to_string(),
        }
    }

    fn handoff(
        prompt: &str,
        transcript_text: &str,
        task: Option<PlanningTaskHandoff>,
    ) -> ManualPromptMainSessionHandoff {
        ManualPromptMainSessionHandoff {
            prompt: prompt.to_string(),
            transcript_text: transcript_text.to_string(),
            task,
        }
    }

    fn auto_follow_origin() -> PromptOrigin {
        PromptOrigin::AutoFollow(Box::new(AutoFollowSubmitContext {
            completed_turn_id: "turn-1".to_string(),
            mode_label: "planning queue".to_string(),
            transcript_text: QUEUED_TASK_TRANSCRIPT_TEXT.to_string(),
            debug_detail: None,
            handoff_task: None,
        }))
    }

    #[test]
    fn submit_prompt_respects_startup_readiness_gates() {
        let workspace = TempWorkspace::new("turn-submit-startup-gates");
        let mut pending_app = make_test_app(&workspace);
        pending_app.startup_state = StartupState::Loading;
        set_input(&mut pending_app, "ship it");

        assert!(!pending_app.submit_prompt_with_transcript(
            "ship it".to_string(),
            "ship it".to_string(),
            PromptOrigin::Manual,
        ));

        let pending_conversation = ready_conversation(&pending_app);
        assert!(pending_conversation.startup_submit_armed);
        assert_eq!(
            pending_conversation.status_text,
            "prompt queued until startup checks finish"
        );

        let mut blocked_app = make_test_app(&workspace);
        blocked_app.startup_state =
            StartupState::Ready(startup_ready_snapshot(workspace.path_str(), false));

        assert!(!blocked_app.submit_prompt_with_transcript(
            "continue queue".to_string(),
            QUEUED_TASK_TRANSCRIPT_TEXT.to_string(),
            auto_follow_origin(),
        ));

        assert_eq!(
            ready_conversation(&blocked_app).status_text,
            "auto-follow paused because startup diagnostics need attention"
        );
    }

    #[test]
    fn resolve_startup_submit_queue_keeps_or_disarms_buffered_prompt() {
        let workspace = TempWorkspace::new("turn-submit-startup-queue");
        let mut pending_app = make_test_app(&workspace);
        pending_app.startup_state = StartupState::Loading;
        set_input(&mut pending_app, "queued prompt");
        pending_app.dispatch_conversation_input(ConversationInputEvent::StartupSubmitArmed {
            status_text: "queued".to_string(),
        });

        pending_app.resolve_startup_submit_queue();

        assert!(ready_conversation(&pending_app).startup_submit_armed);
        assert_eq!(
            ready_conversation(&pending_app).input_buffer,
            "queued prompt"
        );

        let mut blocked_app = make_test_app(&workspace);
        blocked_app.startup_state =
            StartupState::Ready(startup_ready_snapshot(workspace.path_str(), false));
        set_input(&mut blocked_app, "queued prompt");
        blocked_app.dispatch_conversation_input(ConversationInputEvent::StartupSubmitArmed {
            status_text: "queued".to_string(),
        });

        blocked_app.resolve_startup_submit_queue();

        let blocked_conversation = ready_conversation(&blocked_app);
        assert!(!blocked_conversation.startup_submit_armed);
        assert_eq!(blocked_conversation.input_buffer, "queued prompt");
        assert_eq!(
            blocked_conversation.status_text,
            "startup diagnostics need attention; open diagnostics with Ctrl+d; queued prompt kept in buffer"
        );

        let mut ready_empty_app = make_test_app(&workspace);
        set_input(&mut ready_empty_app, "   ");
        ready_empty_app.dispatch_conversation_input(ConversationInputEvent::StartupSubmitArmed {
            status_text: "queued".to_string(),
        });

        ready_empty_app.resolve_startup_submit_queue();

        assert!(!ready_conversation(&ready_empty_app).startup_submit_armed);
    }

    #[test]
    fn apply_manual_prompt_preparation_routes_bootstrap_review_and_failures() {
        let workspace = TempWorkspace::new("turn-submit-bootstrap-review");
        let mut review_app = make_test_app(&workspace);
        set_input(&mut review_app, "create the planning workspace");

        review_app.apply_manual_prompt_preparation(
            ManualPromptPreparationResult::BootstrapReviewRequired {
                transcript_text: "create the planning workspace".to_string(),
                runtime_projection: runtime_projection(),
                review: ManualPlanningBootstrapReview {
                    draft_name: "simple-draft".to_string(),
                    staged_file_count: 2,
                    validation_report: PlanningValidationReport::default(),
                },
            },
        );

        assert_eq!(review_app.shell_overlay, ShellOverlay::PlanningInit);
        assert_eq!(
            review_app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
        let review = review_app
            .planning_init_overlay_ui_state
            .simple_review()
            .expect("bootstrap review should be retained for the overlay");
        assert_eq!(review.draft_name(), "simple-draft");
        assert_eq!(review.staged_file_count(), 2);
        assert_eq!(
            ready_conversation(&review_app).status_text,
            "planning bootstrap promote blocked / draft: simple-draft / validation needs attention"
        );

        for (kind, expected_status) in [
            (
                ManualPlanningBootstrapFailureKind::Stage,
                "planning bootstrap failed: disk full",
            ),
            (
                ManualPlanningBootstrapFailureKind::Promote,
                "planning bootstrap promote failed: disk full",
            ),
        ] {
            let mut failure_app = make_test_app(&workspace);
            set_input(&mut failure_app, "create the planning workspace");

            failure_app.apply_manual_prompt_preparation(
                ManualPromptPreparationResult::BootstrapFailed {
                    transcript_text: "create the planning workspace".to_string(),
                    runtime_projection: runtime_projection(),
                    kind,
                    reason: "disk full".to_string(),
                },
            );

            assert_eq!(
                ready_conversation(&failure_app).status_text,
                expected_status
            );
        }
    }

    #[test]
    fn apply_manual_prompt_preparation_rejects_and_ignores_stale_results() {
        let workspace = TempWorkspace::new("turn-submit-prep-rejected");
        let mut app = make_test_app(&workspace);
        set_input(&mut app, "ship it");

        app.apply_manual_prompt_preparation(ManualPromptPreparationResult::Rejected {
            transcript_text: "ship it".to_string(),
            runtime_projection: runtime_projection(),
            reason: "not actionable".to_string(),
        });

        let conversation = ready_conversation(&app);
        assert_eq!(
            conversation.status_text,
            "turn preparation failed / not actionable"
        );
        assert_eq!(conversation.input_buffer, "");
        assert_eq!(conversation.messages.last().unwrap().text, "ship it");

        let mut stale_app = make_test_app(&workspace);
        set_input(&mut stale_app, "newer text");
        let previous_status = ready_conversation(&stale_app).status_text.clone();

        stale_app.apply_manual_prompt_preparation(ManualPromptPreparationResult::Rejected {
            transcript_text: "older text".to_string(),
            runtime_projection: runtime_projection(),
            reason: "should not surface".to_string(),
        });

        assert_eq!(ready_conversation(&stale_app).status_text, previous_status);
        assert!(ready_conversation(&stale_app).messages.is_empty());
    }

    #[test]
    fn apply_manual_prompt_preparation_routes_manual_intake_outcomes() {
        let workspace = TempWorkspace::new("turn-submit-manual-intake");
        let mut no_task_app = make_test_app(&workspace);
        set_input(&mut no_task_app, "answer directly");

        no_task_app.apply_manual_prompt_preparation(ManualPromptPreparationResult::PromptReady {
            transcript_text: "answer directly".to_string(),
            runtime_projection: runtime_projection(),
            intake: Box::new(ManualPromptIntakeOutcome::NoTaskNeeded(handoff(
                "wrapped answer",
                "answer directly",
                None,
            ))),
        });

        let no_task_conversation = ready_conversation(&no_task_app);
        assert_eq!(no_task_conversation.status_text, "starting turn");
        assert_eq!(no_task_conversation.input_buffer, "");
        assert_eq!(
            no_task_conversation.messages.last().unwrap().text,
            "answer directly"
        );
        assert!(no_task_conversation.last_planning_task_handoff().is_none());

        let task = sample_handoff_task();
        let mut committed_app = make_test_app(&workspace);
        set_input(&mut committed_app, "turn this into a task");

        committed_app.apply_manual_prompt_preparation(ManualPromptPreparationResult::PromptReady {
            transcript_text: "turn this into a task".to_string(),
            runtime_projection: runtime_projection(),
            intake: Box::new(ManualPromptIntakeOutcome::TaskCommitted {
                committed_task_id: task.task_id.clone(),
                committed_planning_revision: 7,
                handoff: handoff(
                    "wrapped task prompt",
                    "turn this into a task",
                    Some(task.clone()),
                ),
            }),
        });

        assert_eq!(
            ready_conversation(&committed_app).last_planning_task_handoff(),
            Some(&task)
        );
        assert_eq!(
            ready_conversation(&committed_app).status_text,
            "starting turn"
        );

        for outcome in [
            ManualPromptIntakeOutcome::Rejected {
                reason: "too small".to_string(),
            },
            ManualPromptIntakeOutcome::Failed {
                reason: "intake crashed".to_string(),
            },
        ] {
            let mut failure_app = make_test_app(&workspace);
            set_input(&mut failure_app, "make task");
            let expected_reason = match &outcome {
                ManualPromptIntakeOutcome::Rejected { reason }
                | ManualPromptIntakeOutcome::Failed { reason } => reason.clone(),
                _ => unreachable!("test only uses failure outcomes"),
            };

            failure_app.apply_manual_prompt_preparation(
                ManualPromptPreparationResult::PromptReady {
                    transcript_text: "make task".to_string(),
                    runtime_projection: runtime_projection(),
                    intake: Box::new(outcome),
                },
            );

            assert_eq!(
                ready_conversation(&failure_app).status_text,
                format!("turn preparation failed / {expected_reason}")
            );
            assert_eq!(
                ready_conversation(&failure_app)
                    .messages
                    .last()
                    .unwrap()
                    .text,
                "make task"
            );
        }
    }

    #[test]
    fn queue_auto_prompt_records_debug_detail_and_handoff() {
        let workspace = TempWorkspace::new("turn-submit-auto-debug");
        let mut app = make_test_app(&workspace);
        app.planning_worker_visibility = PlanningWorkerVisibility::Debug;
        app.planning_worker_panel_state.status = PlanningWorkerStatus::RefreshSucceeded;
        app.planning_worker_panel_state.last_operation_label = Some("refresh queue".to_string());
        app.planning_worker_panel_state.last_summary = Some("accepted task".to_string());
        app.planning_worker_panel_state.last_prompt = Some("worker prompt".to_string());
        app.planning_worker_panel_state.last_response = Some("worker response".to_string());
        let handoff_task = sample_handoff_task();

        app.execute_conversation_runtime_effect(ConversationRuntimeEffect::QueueAutoPrompt {
            prompt: "continue task".to_string(),
            completed_turn_id: "turn-completed".to_string(),
            mode_label: "planning queue".to_string(),
            transcript_text: QUEUED_TASK_TRANSCRIPT_TEXT.to_string(),
            handoff_task: Some(handoff_task.clone()),
        });

        let conversation = ready_conversation(&app);
        assert!(
            conversation
                .status_text
                .starts_with("auto-follow submitted / turn")
        );
        assert_eq!(
            conversation.last_planning_task_handoff(),
            Some(&handoff_task)
        );
        let transcript_message = conversation.messages.last().unwrap();
        assert_eq!(
            transcript_message.display_label.as_deref(),
            Some("Auto Follow-up")
        );
        let debug_detail = transcript_message
            .debug_detail
            .as_deref()
            .expect("debug visibility should attach worker detail");
        assert!(
            debug_detail.contains("planning worker temporary session: refresh queue / refresh ok")
        );
        assert!(debug_detail.contains("planning worker summary: accepted task"));
        assert!(debug_detail.contains("worker prompt"));
        assert!(debug_detail.contains("worker response"));
    }

    #[test]
    fn build_turn_submission_request_maps_origin_and_parallel_slot_handoff() {
        let workspace = TempWorkspace::new("turn-submit-request");
        let mut app = make_test_app(&workspace);
        let task = sample_handoff_task();
        ready_conversation_mut(&mut app).record_manual_intake_handoff(Some(&task));
        app.set_parallel_mode_enabled_for_test(true);
        app.turn_options.model = Some("gpt-5.4".to_string());
        app.turn_options.reasoning_effort = Some(ConversationReasoningEffort::High);

        let request = app.build_turn_submission_request(
            workspace.path_str().to_string(),
            Some("thread-1".to_string()),
            "wrapped task prompt".to_string(),
            &PromptOrigin::ManualIntake(Box::new(super::super::ManualIntakeSubmitContext {
                transcript_text: "operator text".to_string(),
                handoff_task: Some(task.clone()),
            })),
        );

        assert_eq!(request.workspace_directory, workspace.path_str());
        assert_eq!(request.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(request.prompt, "wrapped task prompt");
        assert_eq!(request.prompt_origin, CorePromptOrigin::ManualIntake);
        assert_eq!(request.turn_options, app.turn_options);
        assert_eq!(
            request.slot_lease_handoff,
            Some(ParallelTurnSlotLeaseHandoff::new(
                task.task_id.clone(),
                task.task_title.clone(),
            ))
        );

        app.set_parallel_mode_enabled_for_test(false);
        let manual_request = app.build_turn_submission_request(
            workspace.path_str().to_string(),
            None,
            "manual prompt".to_string(),
            &PromptOrigin::Manual,
        );

        assert_eq!(manual_request.prompt_origin, CorePromptOrigin::Manual);
        assert_eq!(manual_request.slot_lease_handoff, None);
    }

    #[test]
    fn inline_model_and_think_commands_update_turn_options() {
        let workspace = TempWorkspace::new("turn-options-command");
        let mut app = make_test_app(&workspace);

        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":model gpt-5.4").expect("model command should parse"),
        );
        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":think high").expect("think command should parse"),
        );

        assert_eq!(app.turn_options.model.as_deref(), Some("gpt-5.4"));
        assert_eq!(
            app.turn_options.reasoning_effort,
            Some(ConversationReasoningEffort::High)
        );

        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":model default")
                .expect("model clear command should parse"),
        );
        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":think default")
                .expect("think clear command should parse"),
        );

        assert!(app.turn_options.is_default());
    }

    #[test]
    fn sync_active_turn_workspace_directory_updates_ready_conversation_only() {
        let workspace = TempWorkspace::new("turn-submit-active-workspace");
        let mut app = make_test_app(&workspace);

        app.sync_active_turn_workspace_directory("/tmp/active-turn");

        assert_eq!(
            ready_conversation(&app)
                .active_turn_workspace_directory
                .as_deref(),
            Some("/tmp/active-turn")
        );

        app.conversation_state = ConversationState::Loading;
        app.sync_active_turn_workspace_directory("/tmp/ignored");
        assert!(matches!(app.conversation_state, ConversationState::Loading));
    }

    #[test]
    fn dispatch_operator_alert_effect_sends_background_message() {
        let workspace = TempWorkspace::new("turn-submit-operator-alert");
        let mut app = make_test_app(&workspace);
        let alert = OperatorAlert::planning_queue_drained();

        app.execute_conversation_runtime_effect(ConversationRuntimeEffect::DispatchOperatorAlert {
            alert: alert.clone(),
        });

        let message = app
            .rx
            .try_recv()
            .expect("operator alert should be queued for the runtime");
        assert!(matches!(
            message,
            BackgroundMessage::OperatorAlert(queued_alert) if queued_alert == alert
        ));
    }
}
