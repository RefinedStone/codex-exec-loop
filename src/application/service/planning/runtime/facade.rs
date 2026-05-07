/*
 * runtime facade는 TUI와 app-server turn execution이 사용하는 application-layer boundary다. planning rule을
 * 다시 구현하지 않고, snapshot loading, auto-follow policy decision, prompt assembly, post-turn reconciliation을
 * inbound adapter가 바로 소비할 return shape로 순서화한다. adapter는 내부 service graph를 모르고 이 facade의
 * 작은 DTO와 method만 알면 된다.
 */
use crate::application::service::parallel_agent_persona::ParallelAgentPersona;
use crate::application::service::planning::repair::reconciliation::{
    PlanningExecutionSnapshot, PlanningReconciliationResult, PlanningReconciliationService,
};
use crate::application::service::planning::runtime::manual_intake::{
    manual_intake_handoff_from_queue_head, manual_intake_handoff_from_task,
    manual_intake_task_prompt,
};
use crate::application::service::planning::runtime::policy::{
    PlanningAutoFollowBlockReason, PlanningAutoFollowPolicyDecision, PlanningAutoFollowPromptMode,
    PlanningRuntimePolicyService,
};
use crate::application::service::planning::runtime::prompt::{
    PlanningPromptService, PlanningRuntimeSnapshot,
};
use crate::application::service::planning::shared::auto_follow_copy::QUEUED_TASK_TRANSCRIPT_TEXT;
use crate::application::service::prompt_component::PromptDocument;
use crate::application::service::turn_prompt_assembly_service::{
    MainSessionPromptAssemblyRequest, ManualPromptAssemblyRequest, SubSessionPromptAssemblyRequest,
    TurnPromptAssemblyService,
};
use crate::domain::planning::{PriorityQueueTask, TaskDefinition};
use anyhow::Result;

// policy view model을 facade에서 re-export해 caller가 runtime import surface 하나만 바라보게 한다.
pub use crate::application::service::planning::runtime::policy::{
    PlanningRuntimeRepairAttempt, PlanningRuntimeStatusProjection,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    PlanningRuntimeSummaryRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
// auto-follow decision에는 policy가 참고할 last message/stop keyword와, 중복 load를 피하기 위한 preloaded snapshot이 들어간다.
pub struct PlanningRuntimeAutoFollowRequest<'a> {
    pub stop_keyword: &'a str,
    pub last_message: &'a str,
    pub snapshot: &'a PlanningRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// auto-follow preview rendering은 read-only이고, UI가 planning status만 필요로 할 때는 last message 없이도 실행된다.
pub struct PlanningRuntimeAutoFollowPreviewRequest<'a> {
    pub stop_keyword: &'a str,
    pub last_message: Option<&'a str>,
    pub snapshot: &'a PlanningRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// facade가 필요한 queued-task prompt를 materialize한 뒤 adapter에게 돌려주는 policy output이다.
pub enum PlanningRuntimeAutoFollowDecision {
    QueuePrompt(PlanningRuntimeQueuedAutoFollowPrompt),
    Blocked(PlanningAutoFollowBlockReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
// queued auto-follow는 실제 실행 prompt와 visible session에 남길 transcript copy를 함께 담는다.
pub struct PlanningRuntimeQueuedAutoFollowPrompt {
    pub prompt: String,
    pub transcript_text: String,
    pub handoff_task: Option<PlanningTaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// auto-follow preview bundle은 실제 prompt preview와 policy-derived status copy를 한 응답으로 묶는다.
pub struct PlanningRuntimeAutoFollowPreview {
    pub rendered_prompt: String,
    pub planning_status_line: String,
    pub planning_detail_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// main-session handoff는 operator-visible conversation에 append되므로 transcript copy를 함께 가진다.
pub struct PlanningMainSessionHandoff {
    pub prompt: String,
    pub transcript_text: String,
    pub task: PlanningTaskHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// sub-session handoff는 hidden work를 시작한다. prompt와 task identity만 필요하고 visible transcript marker는 없다.
pub struct PlanningSubSessionHandoff {
    pub prompt: String,
    pub developer_instructions: String,
    pub service_name: String,
    pub task: PlanningTaskHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// auto-follow, UI status, repair handoff code가 공유하는 compact task identity다.
pub struct PlanningTaskHandoff {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub combined_priority: i32,
    pub updated_at: String,
    pub status_label: String,
}

#[derive(Clone)]
/*
 * facade는 runtime service 사이의 순서를 소유한다. prompt service는 immutable snapshot을 만들고, policy service는
 * snapshot을 status/follow decision으로 바꾸고, turn prompt assembly는 task instruction을 main/sub-session prompt로
 * 감싸며, reconciliation은 turn 이후 protected planning state를 복구한다.
 */
pub struct PlanningRuntimeFacadeService {
    planning_prompt_service: PlanningPromptService,
    planning_reconciliation_service: PlanningReconciliationService,
    planning_runtime_policy_service: PlanningRuntimePolicyService,
    turn_prompt_assembly_service: TurnPromptAssemblyService,
}
impl PlanningRuntimeFacadeService {
    // composition은 concrete service를 한 번 주입한다. adapter는 runtime wiring detail 대신 facade method만 호출한다.
    pub fn new(
        planning_prompt_service: PlanningPromptService,
        planning_reconciliation_service: PlanningReconciliationService,
        planning_runtime_policy_service: PlanningRuntimePolicyService,
        turn_prompt_assembly_service: TurnPromptAssemblyService,
    ) -> Self {
        Self {
            planning_prompt_service,
            planning_reconciliation_service,
            planning_runtime_policy_service,
            turn_prompt_assembly_service,
        }
    }

    /*
     * TUI rendering은 planning file load가 실패해도 panic하면 안 된다. loader error를 invalid snapshot으로 낮추면
     * 같은 policy/status path가 blocked planning state와 failure reason을 표시할 수 있다.
     */
    pub fn load_runtime_snapshot_or_invalid(
        &self,
        workspace_directory: &str,
    ) -> PlanningRuntimeSnapshot {
        self.planning_prompt_service
            .load_runtime_snapshot(workspace_directory)
            .unwrap_or_else(|error| {
                PlanningRuntimeSnapshot::invalid(format!(
                    "failed to load planning workspace: {error}"
                ))
            })
    }

    // manual prompt는 user-authored prompt만 main-session으로 넘긴다. planning context와 task mutation 규칙은
    // hidden intake/planning worker 경로의 입력으로만 쓰고, main prompt에는 주입하지 않는다.
    pub fn build_manual_prompt(&self, operator_prompt: &str) -> Option<String> {
        self.turn_prompt_assembly_service
            .build_manual_prompt(ManualPromptAssemblyRequest { operator_prompt })
    }

    /*
     * current queue head를 main-session prompt로 변환한다. queue head가 없으면 work를 조작해 만들지 않고 None을
     * 돌려준다. policy는 auto-follow가 허용된 듯 보이더라도 이 상태를 actionable queue-required block으로 낮춘다.
     */
    pub fn build_queued_task_handoff(
        &self,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<PlanningMainSessionHandoff> {
        let queue_head = snapshot.queue_head()?;
        Some(self.build_main_session_task_handoff(queue_head))
    }

    // 이미 task를 가진 caller가 planning fragment 없이 main-session handoff만 만들 때 쓰는 public helper다.
    pub fn build_main_session_task_handoff(
        &self,
        task: &PriorityQueueTask,
    ) -> PlanningMainSessionHandoff {
        self.build_compact_task_handoff(task)
    }

    pub fn build_manual_intake_task_handoff(
        &self,
        task: &TaskDefinition,
        direction_title: &str,
        queue_head: Option<&PriorityQueueTask>,
        original_prompt: &str,
    ) -> PlanningMainSessionHandoff {
        let task_prompt = manual_intake_task_prompt(task, direction_title, original_prompt);
        let prompt = self
            .turn_prompt_assembly_service
            .build_main_session_prompt(MainSessionPromptAssemblyRequest {
                user_prompt: &task_prompt,
            })
            .expect("manual intake task handoff prompt should not be empty");
        let handoff_task = queue_head
            .filter(|queue_task| queue_task.task_id.trim() == task.id.trim())
            .map(manual_intake_handoff_from_queue_head)
            .unwrap_or_else(|| manual_intake_handoff_from_task(task, direction_title));

        PlanningMainSessionHandoff {
            prompt,
            transcript_text: original_prompt.trim().to_string(),
            task: handoff_task,
        }
    }

    /*
     * hidden sub-session은 queued-task handoff prompt만 받는다. planning prompt fragment를 의도적으로 생략하는 이유는
     * orchestration-specific worker가 worker prompt builder를 통해 자기 authority context를 별도로 렌더링하기 때문이다.
     */
    pub fn build_sub_session_task_handoff(
        &self,
        task: &PriorityQueueTask,
    ) -> PlanningSubSessionHandoff {
        self.build_sub_session_task_handoff_with_persona(task, ParallelAgentPersona::None)
    }

    pub fn build_sub_session_task_handoff_with_persona(
        &self,
        task: &PriorityQueueTask,
        persona: ParallelAgentPersona,
    ) -> PlanningSubSessionHandoff {
        let task_prompt = render_queued_task_handoff_prompt(task);
        let prompt = self
            .turn_prompt_assembly_service
            .build_sub_session_prompt(SubSessionPromptAssemblyRequest {
                handoff_prompt: &task_prompt,
                persona,
            })
            .expect("queued sub-session handoff prompt should not be empty");

        PlanningSubSessionHandoff {
            prompt: prompt.turn_prompt,
            developer_instructions: prompt.developer_instructions,
            service_name: prompt.service_name,
            task: planning_task_handoff_from_priority_queue_task(task),
        }
    }

    /*
     * main-session handoff는 operator에게 보이는 conversation에 들어가지만 planning fragment를 포함하지 않는다.
     * transcript marker는 runtime이 queued-task continuation을 큐에서 넘겼다는 사실만 기록하고, 내부 queue prompt 전체를
     * chat history에 노출하지 않는다.
     */
    fn build_compact_task_handoff(&self, task: &PriorityQueueTask) -> PlanningMainSessionHandoff {
        let task_prompt = render_queued_task_handoff_prompt(task);
        let prompt = self
            .turn_prompt_assembly_service
            .build_main_session_prompt(MainSessionPromptAssemblyRequest {
                user_prompt: &task_prompt,
            })
            .expect("queued-task handoff prompt should not be empty");

        PlanningMainSessionHandoff {
            prompt,
            transcript_text: QUEUED_TASK_TRANSCRIPT_TEXT.to_string(),
            task: planning_task_handoff_from_priority_queue_task(task),
        }
    }

    // preview는 execution과 같은 prompt builder를 사용한다. task가 없을 때만 queue-idle explanatory copy로 대체해,
    // 실제 실행될 prompt와 preview가 서로 다른 규칙을 타지 않게 한다.
    pub fn queued_task_preview_prompt(&self, snapshot: &PlanningRuntimeSnapshot) -> String {
        self.build_queued_task_handoff(snapshot)
            .map(|handoff| handoff.prompt)
            .unwrap_or_else(|| {
                match snapshot.queue_idle_policy() {
                    crate::domain::planning::QueueIdlePolicy::Stop => {
                        "The current planning queue has no actionable head and queue-idle policy is stop, so internal continuation will end after the current turn.".to_string()
                    }
                    crate::domain::planning::QueueIdlePolicy::ReviewAndEnqueue => {
                        "A planning worker reviews the direction goals after the current turn and re-enqueues follow-up work only when a justified actionable task exists.".to_string()
                    }
                }
            })
    }

    /*
     * auto-follow를 결정하고, 허용된 queued-task mode에서만 executable prompt를 materialize한다. 추가 queue-head check는
     * stale snapshot이나 미래 policy 변경이 concrete task 없이 actionable work를 주장하는 상황을 방어한다.
     */
    pub fn decide_auto_follow(
        &self,
        request: PlanningRuntimeAutoFollowRequest<'_>,
    ) -> PlanningRuntimeAutoFollowDecision {
        match self
            .planning_runtime_policy_service
            .decide_auto_follow(request.snapshot)
        {
            PlanningAutoFollowPolicyDecision::Blocked(block_reason) => {
                PlanningRuntimeAutoFollowDecision::Blocked(block_reason)
            }
            PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::ContinueQueuedTask,
            ) => match self.build_queued_task_handoff(request.snapshot) {
                Some(handoff) => PlanningRuntimeAutoFollowDecision::QueuePrompt(
                    PlanningRuntimeQueuedAutoFollowPrompt {
                        prompt: handoff.prompt,
                        transcript_text: handoff.transcript_text,
                        handoff_task: Some(handoff.task),
                    },
                ),
                None => PlanningRuntimeAutoFollowDecision::Blocked(
                    PlanningAutoFollowBlockReason::ActionableQueueRequired,
                ),
            },
        }
    }

    // auto-follow submit 전에 보여 주는 read-only prompt/status preview를 만든다.
    pub fn build_auto_follow_preview(
        &self,
        request: PlanningRuntimeAutoFollowPreviewRequest<'_>,
    ) -> PlanningRuntimeAutoFollowPreview {
        let policy_decision = self
            .planning_runtime_policy_service
            .decide_auto_follow(request.snapshot);
        let planning_view = self
            .planning_runtime_policy_service
            .build_preview_view_for_decision(policy_decision, request.snapshot);
        let rendered_prompt = self.queued_task_preview_prompt(request.snapshot);
        PlanningRuntimeAutoFollowPreview {
            rendered_prompt,
            planning_status_line: format!("planning: {}", planning_view.status_label),
            planning_detail_line: planning_view
                .detail
                .map(|detail| format!("planning detail: {detail}")),
        }
    }

    // summary/status helper는 policy service에 위임한다. facade caller가 policy service를 직접 import하지 않게 하기 위해서다.
    pub fn build_summary_line(
        &self,
        request: PlanningRuntimeSummaryLineRequest<'_>,
    ) -> Option<String> {
        self.planning_runtime_policy_service
            .build_summary_line(request)
    }

    pub fn build_auto_follow_status_projection(
        &self,
        request: PlanningRuntimeStatusProjectionRequest<'_>,
    ) -> PlanningRuntimeStatusProjection {
        self.planning_runtime_policy_service
            .build_status_projection(request)
    }

    // turn 전에 protected planning file snapshot을 잡는다. 이후 reconciliation이 필요하면 이 snapshot을 기준으로 복구한다.
    pub fn load_execution_snapshot(
        &self,
        workspace_directory: &str,
    ) -> Result<PlanningExecutionSnapshot> {
        self.planning_reconciliation_service
            .load_execution_snapshot(workspace_directory)
    }

    // reconciliation은 facade 뒤에 남긴다. adapter는 changed path와 pre-turn snapshot만 알고, 복구 세부 규칙은 service가 담당한다.
    pub fn reconcile_after_turn(
        &self,
        workspace_directory: &str,
        turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        self.planning_reconciliation_service.reconcile_after_turn(
            workspace_directory,
            turn_id,
            changed_planning_file_paths,
            execution_snapshot,
        )
    }
}

// PriorityQueueTask field를 한 번 normalize한 뒤 UI/reconciliation code와 handoff identity를 공유한다.
fn planning_task_handoff_from_priority_queue_task(task: &PriorityQueueTask) -> PlanningTaskHandoff {
    PlanningTaskHandoff {
        task_id: task.task_id.trim().to_string(),
        task_title: task.task_title.trim().to_string(),
        direction_id: task.direction_id.trim().to_string(),
        combined_priority: task.combined_priority,
        updated_at: task.updated_at.trim().to_string(),
        status_label: task.status.label().to_string(),
    }
}

/*
 * domain PriorityQueueTask를 Codex에게 보낼 instruction document로 렌더링한다. task section은 무엇을 이어갈지와 왜 queue
 * 첫 항목인지 설명하고, rules section은 사용자가 명시적으로 planning maintenance를 요청하지 않은 한 worker가 repository
 * work에 집중하게 한다.
 */
fn render_queued_task_handoff_prompt(queue_head: &PriorityQueueTask) -> String {
    let rank_reason = queue_head
        .rank_reasons
        .iter()
        .find(|reason| !reason.trim().is_empty())
        .map(String::as_str)
        .unwrap_or("this is the highest-priority actionable task");
    PromptDocument::builder("queued-task-handoff")
        .lines(
            "task",
            vec![
                "intent=Continue the next highest-priority task.".to_string(),
                format!("title={}", queue_head.task_title.trim()),
                format!("direction={}", queue_head.direction_title.trim()),
                format!("rank={}", queue_head.rank),
                format!("combined_priority={}", queue_head.combined_priority),
                format!("why_now={}", rank_reason.trim()),
            ],
        )
        .bullets(
            "rules",
            vec![
                "Work from the current repository state and focus only on this task.".to_string(),
                "Treat `.codex-exec-loop/planning` and planning control files as internal runtime state unless the user explicitly requested planning maintenance or the task strictly requires it."
                    .to_string(),
                "Do not describe planning queue refresh logic in commentary or final answer."
                    .to_string(),
                "When finished, summarize what changed and what remains.".to_string(),
            ],
        )
        .build()
        .render()
}
