use crate::application::service::planning::runtime::facade::{
    PlanningRuntimeFacadeService, PlanningTaskHandoff,
};
use crate::application::service::planning::runtime::intake::{
    PlanningTaskIntakeRequest, PlanningTaskIntakeService,
};
use crate::diagnostics::event_log;
use crate::domain::planning::{
    ManualPromptIntakeOutcome as DomainManualPromptIntakeOutcome,
    ManualPromptIntakeRequest as DomainManualPromptIntakeRequest,
    ManualPromptMainSessionHandoff as DomainManualPromptMainSessionHandoff, OriginSessionKind,
    PriorityQueueTask, TaskDefinition, TaskHandoff as DomainTaskHandoff, TaskMutationProvenance,
};
use serde_json::json;

pub type ManualPromptIntakeRequest = DomainManualPromptIntakeRequest;
pub type ManualPromptMainSessionHandoff = DomainManualPromptMainSessionHandoff;
pub type ManualPromptIntakeOutcome = DomainManualPromptIntakeOutcome;

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
) -> DomainTaskHandoff {
    DomainTaskHandoff {
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
    use std::sync::Mutex;

    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_task_repository_port::{
        PlanningDirectionAuthorityCommit, PlanningDirectionAuthoritySnapshot,
        PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult,
        PlanningTaskAuthoritySnapshot, PlanningTaskRepositoryPort,
    };
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::service::planning::{
        ManualPromptIntakeOutcome, ManualPromptIntakeRequest, PlanningServices,
    };
    use crate::diagnostics::event_log;
    use crate::diagnostics::trace_event_log::AKRA_EVENT_TARGET;
    use anyhow::anyhow;
    use serde_json::json;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::prelude::*;

    #[test]
    fn manual_prompt_intake_commits_greetings_and_questions_as_tasks() {
        let workspace_dir = create_temp_git_repo("manual-intake-no-heuristic");
        let planning = planning_services();
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

    #[test]
    fn manual_prompt_intake_emits_trace_events_for_committed_and_failed_outcomes() {
        let capture = CaptureWriter::default();
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new(format!("{AKRA_EVENT_TARGET}=debug")))
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(capture.clone()),
            );

        tracing::subscriber::with_default(subscriber, || {
            event_log::emit_lazy("manual_intake_test_probe", || json!({ "enabled": true }));
            let planning = planning_services();
            let success_workspace = create_temp_git_repo("manual-intake-event-success");
            bootstrap_planning_workspace(&planning, &success_workspace);
            let committed =
                planning
                    .runtime
                    .prepare_manual_prompt_intake(ManualPromptIntakeRequest {
                        workspace_directory: success_workspace,
                        raw_prompt: "Trace manual intake success".to_string(),
                        legacy_source_turn_id: None,
                        parent_thread_id: Some("parent-thread".to_string()),
                        parent_turn_id: Some("parent-turn".to_string()),
                    });
            assert!(matches!(
                committed,
                ManualPromptIntakeOutcome::TaskCommitted { .. }
            ));

            let failed_workspace = create_temp_git_repo("manual-intake-event-failure");
            write_invalid_result_output(&failed_workspace);
            let failed = planning
                .runtime
                .prepare_manual_prompt_intake(ManualPromptIntakeRequest {
                    workspace_directory: failed_workspace,
                    raw_prompt: "Trace manual intake failure".to_string(),
                    legacy_source_turn_id: None,
                    parent_thread_id: None,
                    parent_turn_id: None,
                });
            let ManualPromptIntakeOutcome::Failed { reason } = failed else {
                panic!("invalid result output should fail manual intake: {failed:?}");
            };
            assert!(reason.contains("manual intake prepare failed"));
        });

        let joined = capture.lines().join("\n");
        assert!(joined.contains("manual_intake_test_probe"));
        assert!(joined.contains("manual_intake_started"));
        assert!(joined.contains("manual_intake_committed"));
        assert!(joined.contains("manual_intake_failed"));
        assert!(joined.contains("manual intake prepare failed"));
        let mut sink = capture.make_writer();
        std::io::Write::flush(&mut sink).expect("capture sink should flush");
    }

    #[test]
    fn manual_prompt_intake_reports_commit_failure_from_task_repository() {
        let workspace_dir = create_temp_git_repo("manual-intake-commit-failure");
        let workspace = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let bootstrap_planning = PlanningServices::from_ports(
            workspace.clone(),
            authority.clone(),
            authority.clone(),
            Arc::new(NoopPlanningWorkerPort),
        );
        bootstrap_planning_workspace(&bootstrap_planning, &workspace_dir);
        let failing_repository = Arc::new(CommitFailingTaskRepositoryPort {
            inner: authority.clone(),
        });
        let direction_snapshot = failing_repository
            .load_direction_authority_snapshot(&workspace_dir)
            .expect("direction authority should load")
            .expect("direction authority should exist");
        let direction_commit = failing_repository
            .commit_direction_authority_snapshot(
                &workspace_dir,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions: &direction_snapshot.directions,
                },
            )
            .expect("direction authority commit should delegate");
        assert!(matches!(
            direction_commit,
            PlanningTaskAuthorityCommitResult::Committed { .. }
        ));
        let task_snapshot = failing_repository
            .load_task_authority_snapshot(&workspace_dir)
            .expect("task authority should load")
            .expect("task authority should exist");
        let task_commit = failing_repository
            .commit_task_authority_snapshot(
                &workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_authority: &task_snapshot.task_authority,
                    queue_projection: &task_snapshot.queue_projection,
                },
            )
            .expect("seed-style task authority commit should delegate");
        assert!(matches!(
            task_commit,
            PlanningTaskAuthorityCommitResult::Committed { .. }
        ));
        let failing_planning = PlanningServices::from_ports(
            workspace,
            authority.clone(),
            failing_repository.clone(),
            Arc::new(NoopPlanningWorkerPort),
        );

        let outcome =
            failing_planning
                .runtime
                .prepare_manual_prompt_intake(ManualPromptIntakeRequest {
                    workspace_directory: workspace_dir.clone(),
                    raw_prompt: "Commit this as a task".to_string(),
                    legacy_source_turn_id: None,
                    parent_thread_id: None,
                    parent_turn_id: None,
                });
        let ManualPromptIntakeOutcome::Failed { reason } = outcome else {
            panic!("commit repository failure should fail manual intake: {outcome:?}");
        };

        assert!(reason.contains("manual intake commit failed"));
        assert!(reason.contains("synthetic task authority commit failure"));
        failing_repository
            .clear_direction_authority_snapshot(&workspace_dir)
            .expect("direction authority clear should delegate");
        failing_repository
            .clear_task_authority_snapshot(&workspace_dir)
            .expect("task authority clear should delegate");
    }

    fn planning_services() -> PlanningServices {
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        PlanningServices::from_ports(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            authority.clone(),
            authority,
            Arc::new(NoopPlanningWorkerPort),
        )
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

    fn write_invalid_result_output(workspace_dir: &str) {
        let path = PathBuf::from(workspace_dir).join(".codex-exec-loop/planning");
        std::fs::create_dir_all(&path).expect("planning directory should be created");
        std::fs::write(path.join("result-output.md"), "not a markdown heading")
            .expect("invalid result output should write");
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

    #[derive(Clone, Default)]
    struct CaptureWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl CaptureWriter {
        fn lines(&self) -> Vec<String> {
            let bytes = self
                .bytes
                .lock()
                .expect("capture lock should not be poisoned");
            String::from_utf8(bytes.clone())
                .expect("captured diagnostics should be UTF-8")
                .lines()
                .map(str::to_string)
                .collect()
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureSink;

        fn make_writer(&'a self) -> Self::Writer {
            CaptureSink {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }

    struct CaptureSink {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl std::io::Write for CaptureSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes
                .lock()
                .expect("capture lock should not be poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct CommitFailingTaskRepositoryPort {
        inner: Arc<SqlitePlanningAuthorityAdapter>,
    }

    impl PlanningTaskRepositoryPort for CommitFailingTaskRepositoryPort {
        fn load_direction_authority_snapshot(
            &self,
            workspace_dir: &str,
        ) -> anyhow::Result<Option<PlanningDirectionAuthoritySnapshot>> {
            self.inner.load_direction_authority_snapshot(workspace_dir)
        }

        fn commit_direction_authority_snapshot(
            &self,
            workspace_dir: &str,
            commit: PlanningDirectionAuthorityCommit<'_>,
        ) -> anyhow::Result<PlanningTaskAuthorityCommitResult> {
            self.inner
                .commit_direction_authority_snapshot(workspace_dir, commit)
        }

        fn clear_direction_authority_snapshot(&self, workspace_dir: &str) -> anyhow::Result<()> {
            self.inner.clear_direction_authority_snapshot(workspace_dir)
        }

        fn load_task_authority_snapshot(
            &self,
            workspace_dir: &str,
        ) -> anyhow::Result<Option<PlanningTaskAuthoritySnapshot>> {
            self.inner.load_task_authority_snapshot(workspace_dir)
        }

        fn commit_task_authority_snapshot(
            &self,
            workspace_dir: &str,
            commit: PlanningTaskAuthorityCommit<'_>,
        ) -> anyhow::Result<PlanningTaskAuthorityCommitResult> {
            if commit.observed_planning_revision.is_some() {
                return Err(anyhow!("synthetic task authority commit failure"));
            }
            self.inner
                .commit_task_authority_snapshot(workspace_dir, commit)
        }

        fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> anyhow::Result<()> {
            self.inner.clear_task_authority_snapshot(workspace_dir)
        }
    }
}
