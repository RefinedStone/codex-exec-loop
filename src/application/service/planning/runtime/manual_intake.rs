use crate::application::service::planning::runtime::facade::{
    PlanningRuntimeFacadeService, PlanningTaskHandoff,
};
use crate::application::service::planning::runtime::intake::{
    PlanningTaskIntakeRequest, PlanningTaskIntakeService,
};
use crate::diagnostics::event_log;
use crate::domain::planning::{
    OriginSessionKind, PriorityQueueTask, TaskDefinition, TaskMutationProvenance,
};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq)]
// manual prompt hidden intake가 소비하는 원본 입력이다. main-session prompt를 만들기 전에 task authority 여부를 결정한다.
pub struct ManualPromptIntakeRequest {
    pub workspace_directory: String,
    pub raw_prompt: String,
    // Legacy task lookup key. Provider-neutral audit identity lives in `provenance`.
    pub legacy_source_turn_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub parent_turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// main-session에 넘길 최종 실행 입력이다. visible transcript에는 `transcript_text`만 남기고, prompt는 app-server로만 간다.
pub struct ManualPromptMainSessionHandoff {
    pub prompt: String,
    pub transcript_text: String,
    pub task: Option<PlanningTaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// manual intake outcome은 task authority 변경 여부를 명시한다. Failed도 값으로 내려 TUI가 main turn 시작을 막을 수 있다.
pub enum ManualPromptIntakeOutcome {
    TaskCommitted {
        committed_task_id: String,
        committed_planning_revision: i64,
        handoff: ManualPromptMainSessionHandoff,
    },
    TaskUpdated {
        updated_task_id: String,
        committed_planning_revision: i64,
        handoff: ManualPromptMainSessionHandoff,
    },
    Rejected {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone)]
/*
 * ManualPromptIntakeService는 수동 prompt와 main-session 사이의 hidden preflight다.
 * 비어 있지 않은 operator prompt는 의미를 임의 분류하지 않고 task authority로 보낸 뒤
 * main-session에는 작은 handoff만 전달한다.
 */
pub struct ManualPromptIntakeService {
    task_intake: PlanningTaskIntakeService,
    runtime_facade: PlanningRuntimeFacadeService,
}

impl ManualPromptIntakeService {
    pub fn new(
        task_intake: PlanningTaskIntakeService,
        runtime_facade: PlanningRuntimeFacadeService,
    ) -> Self {
        Self {
            task_intake,
            runtime_facade,
        }
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub fn prepare_manual_turn(
        &self,
        request: ManualPromptIntakeRequest,
    ) -> ManualPromptIntakeOutcome {
        let transcript_text = request.raw_prompt.trim().to_string();
        if transcript_text.is_empty() {
            return ManualPromptIntakeOutcome::Rejected {
                reason: "manual prompt is empty".to_string(),
            };
        }

        event_log::emit_lazy("manual_intake_started", || {
            json!({
                "workspace_directory": &request.workspace_directory,
                "prompt_chars": transcript_text.chars().count(),
            })
        });

        let outcome = self.commit_prompt_as_task(&request, &transcript_text);
        match &outcome {
            ManualPromptIntakeOutcome::TaskCommitted {
                committed_task_id,
                committed_planning_revision,
                handoff,
            } => event_log::emit_lazy("manual_intake_committed", || {
                json!({
                    "workspace_directory": &request.workspace_directory,
                    "task_id": committed_task_id,
                    "planning_revision": committed_planning_revision,
                    "handoff_task_id": handoff.task.as_ref().map(|task| task.task_id.as_str()),
                })
            }),
            ManualPromptIntakeOutcome::Failed { reason } => {
                event_log::emit_lazy("manual_intake_failed", || {
                    json!({
                        "workspace_directory": &request.workspace_directory,
                        "reason": reason,
                    })
                });
            }
            _ => {}
        }
        outcome
    }

    fn commit_prompt_as_task(
        &self,
        request: &ManualPromptIntakeRequest,
        transcript_text: &str,
    ) -> ManualPromptIntakeOutcome {
        let proposal = match self
            .task_intake
            .prepare_task_intake(PlanningTaskIntakeRequest {
                workspace_directory: request.workspace_directory.clone(),
                raw_prompt: transcript_text.to_string(),
                legacy_source_turn_id: request.legacy_source_turn_id.clone(),
                provenance: TaskMutationProvenance::new(OriginSessionKind::ManualIntake)
                    .with_parent(
                        request.parent_thread_id.clone(),
                        request.parent_turn_id.clone(),
                    ),
                requested_direction_id: None,
                observed_planning_revision: None,
            }) {
            Ok(proposal) => proposal,
            Err(error) => {
                return ManualPromptIntakeOutcome::Failed {
                    reason: format!("manual intake prepare failed: {error}"),
                };
            }
        };
        let commit = match self.task_intake.commit_task_intake(&proposal) {
            Ok(commit) => commit,
            Err(error) => {
                return ManualPromptIntakeOutcome::Failed {
                    reason: format!("manual intake commit failed: {error}"),
                };
            }
        };
        let handoff = self.runtime_facade.build_manual_intake_task_handoff(
            &proposal.draft.task,
            &proposal.draft.direction_title,
            commit.queue_head.as_ref(),
            transcript_text,
        );
        ManualPromptIntakeOutcome::TaskCommitted {
            committed_task_id: commit.committed_task_id,
            committed_planning_revision: commit.committed_planning_revision,
            handoff: ManualPromptMainSessionHandoff {
                prompt: handoff.prompt,
                transcript_text: transcript_text.to_string(),
                task: Some(handoff.task),
            },
        }
    }
}

pub(super) fn manual_intake_handoff_from_task(
    task: &TaskDefinition,
    _direction_title: &str,
) -> PlanningTaskHandoff {
    PlanningTaskHandoff {
        task_id: task.id.trim().to_string(),
        task_title: task.title.trim().to_string(),
        direction_id: task.direction_id.trim().to_string(),
        combined_priority: task.combined_priority(),
        updated_at: task.updated_at.trim().to_string(),
        status_label: task.status.label().to_string(),
    }
}

pub(super) fn manual_intake_handoff_from_queue_head(
    queue_head: &PriorityQueueTask,
) -> PlanningTaskHandoff {
    PlanningTaskHandoff {
        task_id: queue_head.task_id.trim().to_string(),
        task_title: queue_head.task_title.trim().to_string(),
        direction_id: queue_head.direction_id.trim().to_string(),
        combined_priority: queue_head.combined_priority,
        updated_at: queue_head.updated_at.trim().to_string(),
        status_label: queue_head.status.label().to_string(),
    }
}

pub(super) fn manual_intake_task_prompt(
    task: &TaskDefinition,
    direction_title: &str,
    original_prompt: &str,
) -> String {
    crate::application::service::prompt_component::PromptDocument::builder(
        "manual-intake-task-handoff",
    )
    .lines(
        "task",
        vec![
            "intent=Execute the task prepared by hidden ManualPromptIntake.".to_string(),
            format!("task_id={}", task.id.trim()),
            format!("title={}", task.title.trim()),
            format!("direction={}", direction_title.trim()),
            format!("direction_id={}", task.direction_id.trim()),
            format!("status={}", task.status.label()),
            format!("combined_priority={}", task.combined_priority()),
        ],
    )
    .text("description", &task.description)
    .text("original-user-prompt", original_prompt)
    .bullets(
        "rules",
        vec![
            "Perform only this handoff and the user's concrete feedback.".to_string(),
            "Do not mutate task authority or planning queue state directly.".to_string(),
            "When finished, summarize what changed and any follow-up suggestion.".to_string(),
        ],
    )
    .build()
    .render()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Arc;

    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::service::planning::{
        ManualPromptIntakeOutcome, ManualPromptIntakeRequest, PlanningServices,
    };

    #[test]
    fn manual_prompt_intake_commits_greetings_and_questions_as_tasks() {
        let workspace_dir = create_temp_git_repo("manual-intake-no-heuristic");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = PlanningServices::from_ports(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            authority.clone(),
            authority,
            Arc::new(NoopPlanningWorkerPort),
        );
        bootstrap_planning_workspace(&planning, &workspace_dir);

        for (prompt, expected_title) in [
            ("안녕하세요 ?", "안녕하세요"),
            ("How does the queue work?", "How does the queue work"),
        ] {
            let outcome =
                planning
                    .runtime
                    .prepare_manual_prompt_intake(ManualPromptIntakeRequest {
                        workspace_directory: workspace_dir.clone(),
                        raw_prompt: prompt.to_string(),
                        legacy_source_turn_id: None,
                        parent_thread_id: None,
                        parent_turn_id: None,
                    });

            let ManualPromptIntakeOutcome::TaskCommitted { handoff, .. } = outcome else {
                panic!("manual prompt should be committed as a task: {outcome:?}");
            };
            let task = handoff
                .task
                .expect("committed manual intake should carry a task handoff");
            assert_eq!(task.task_title, expected_title);
            assert_eq!(handoff.transcript_text, prompt);
        }
    }

    fn bootstrap_planning_workspace(planning: &PlanningServices, workspace_dir: &str) {
        let stage_result = planning
            .workspace
            .stage_simple_mode_draft(workspace_dir)
            .expect("planning workspace should stage");
        let promote_result = planning
            .workspace
            .promote_staged_draft(workspace_dir, &stage_result.draft_name)
            .expect("planning workspace should promote");
        assert!(
            promote_result.promoted_file_count > 0,
            "bootstrap planning workspace should become ready"
        );
    }

    fn create_temp_git_repo(prefix: &str) -> String {
        let root = PathBuf::from(create_temp_workspace(prefix)).join("repo");
        std::fs::create_dir_all(&root).expect("temp git repo should be created");

        run_git(&root, &["init", "-q"]);
        run_git(&root, &["config", "user.name", "RefinedStone"]);
        run_git(&root, &["config", "user.email", "chem.en.9273@gmail.com"]);
        std::fs::write(root.join("README.md"), "seed\n").expect("seed file should write");
        run_git(&root, &["add", "README.md"]);
        run_git(&root, &["commit", "-qm", "init"]);
        run_git(&root, &["branch", "prerelease"]);
        run_git(
            &root,
            &["update-ref", "refs/remotes/origin/prerelease", "prerelease"],
        );

        std::fs::canonicalize(&root)
            .expect("temp git repo should canonicalize")
            .display()
            .to_string()
    }

    fn create_temp_workspace(prefix: &str) -> String {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        std::fs::create_dir_all(&path).expect("temp workspace should be created");
        path.display().to_string()
    }

    fn run_git(repo_root: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repo_root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git command should spawn");
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}
