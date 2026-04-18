use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning_bootstrap_service::{
    PlanningBootstrapArtifacts, PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning_contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, DIRECTIONS_FILE_PATH, PLAN_OFF_FILE_PATH,
    PLANNING_DIRECTION_DOCS_DIRECTORY, PLANNING_PROMPTS_DIRECTORY, QUEUE_SNAPSHOT_FILE_PATH,
    RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
};
use crate::domain::planning::{TaskLedgerDocument, TaskStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningResetTarget {
    Queue,
    Directions,
    All,
}

impl PlanningResetTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Directions => "directions",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceResetResult {
    pub target: PlanningResetTarget,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
}

#[derive(Clone)]
pub struct PlanningResetService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_bootstrap_service: PlanningBootstrapService,
}

impl PlanningResetService {
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_bootstrap_service: PlanningBootstrapService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_bootstrap_service,
        }
    }

    pub fn reset_workspace(
        &self,
        workspace_dir: &str,
        target: PlanningResetTarget,
    ) -> Result<PlanningWorkspaceResetResult> {
        let workspace = self.load_existing_workspace(workspace_dir)?;
        let bootstrap = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(PlanningBootstrapMode::Simple);

        match target {
            PlanningResetTarget::Queue => self.reset_queue(workspace_dir, &bootstrap),
            PlanningResetTarget::Directions => {
                self.ensure_directions_reset_is_safe(&workspace)?;
                self.reset_directions(workspace_dir, &bootstrap)
            }
            PlanningResetTarget::All => self.reset_all(workspace_dir, &bootstrap),
        }
    }

    fn load_existing_workspace(&self, workspace_dir: &str) -> Result<PlanningWorkspaceLoadRecord> {
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        if workspace.has_any_files() {
            Ok(workspace)
        } else {
            Err(anyhow!(
                "planning workspace is unavailable; initialize planning first"
            ))
        }
    }

    fn reset_queue(
        &self,
        workspace_dir: &str,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                TASK_LEDGER_FILE_PATH,
                Some(&bootstrap.task_ledger_json),
            )?;
        self.planning_workspace_port
            .replace_planning_workspace_file(workspace_dir, QUEUE_SNAPSHOT_FILE_PATH, None)?;

        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::Queue,
            rewritten_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
            removed_paths: vec![QUEUE_SNAPSHOT_FILE_PATH.to_string()],
        })
    }

    fn ensure_directions_reset_is_safe(
        &self,
        workspace: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        let task_ledger_json = workspace.task_ledger_json.as_deref().ok_or_else(|| {
            anyhow!(
                "planning directions reset requires task-ledger.json; use reset all to replace the full workspace"
            )
        })?;
        let task_ledger: TaskLedgerDocument = serde_json::from_str(task_ledger_json).map_err(
            |error| {
                anyhow!(
                    "planning directions reset requires a valid task-ledger.json; use reset all to replace the full workspace ({error})"
                )
            },
        )?;
        let live_tasks = task_ledger
            .tasks
            .iter()
            .filter(|task| !matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled))
            .map(|task| format!("{}({})", task.id.trim(), task.status.label()))
            .collect::<Vec<_>>();
        if live_tasks.is_empty() {
            return Ok(());
        }

        let live_task_summary = live_tasks
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let extra_count = live_tasks.len().saturating_sub(3);
        let suffix = if extra_count == 0 {
            String::new()
        } else {
            format!(" (+{extra_count} more)")
        };
        Err(anyhow!(
            "planning directions reset is blocked by live tasks: {live_task_summary}{suffix}; use reset all to replace the full workspace instead"
        ))
    }

    fn reset_directions(
        &self,
        workspace_dir: &str,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.reset_directions_side_artifacts(workspace_dir, bootstrap)?;

        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::Directions,
            rewritten_paths: vec![
                DIRECTIONS_FILE_PATH.to_string(),
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            ],
            removed_paths: vec![
                PLANNING_DIRECTION_DOCS_DIRECTORY.to_string(),
                PLANNING_PROMPTS_DIRECTORY.to_string(),
                QUEUE_SNAPSHOT_FILE_PATH.to_string(),
            ],
        })
    }

    fn reset_all(
        &self,
        workspace_dir: &str,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.reset_directions_side_artifacts(workspace_dir, bootstrap)?;
        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                TASK_LEDGER_FILE_PATH,
                Some(&bootstrap.task_ledger_json),
            )?;
        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                TASK_LEDGER_SCHEMA_FILE_PATH,
                Some(&bootstrap.task_ledger_schema_json),
            )?;
        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
                Some(&bootstrap.result_output_markdown),
            )?;
        self.planning_workspace_port
            .replace_planning_workspace_file(workspace_dir, PLAN_OFF_FILE_PATH, None)?;

        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::All,
            rewritten_paths: vec![
                DIRECTIONS_FILE_PATH.to_string(),
                TASK_LEDGER_FILE_PATH.to_string(),
                TASK_LEDGER_SCHEMA_FILE_PATH.to_string(),
                RESULT_OUTPUT_FILE_PATH.to_string(),
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            ],
            removed_paths: vec![
                PLANNING_DIRECTION_DOCS_DIRECTORY.to_string(),
                PLANNING_PROMPTS_DIRECTORY.to_string(),
                QUEUE_SNAPSHOT_FILE_PATH.to_string(),
                PLAN_OFF_FILE_PATH.to_string(),
            ],
        })
    }

    fn reset_directions_side_artifacts(
        &self,
        workspace_dir: &str,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<()> {
        self.planning_workspace_port
            .remove_planning_workspace_entry(workspace_dir, PLANNING_DIRECTION_DOCS_DIRECTORY)?;
        self.planning_workspace_port
            .remove_planning_workspace_entry(workspace_dir, PLANNING_PROMPTS_DIRECTORY)?;
        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                DIRECTIONS_FILE_PATH,
                Some(&bootstrap.directions_toml),
            )?;
        for supplemental_file in &bootstrap.supplemental_files {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    &supplemental_file.active_path,
                    Some(&supplemental_file.body),
                )?;
        }
        self.planning_workspace_port
            .replace_planning_workspace_file(workspace_dir, QUEUE_SNAPSHOT_FILE_PATH, None)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use anyhow::Result;

    use super::{PlanningResetService, PlanningResetTarget};
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning_bootstrap_service::PlanningBootstrapService;
    use crate::application::service::planning_contract::{
        DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, DIRECTIONS_FILE_PATH, PLAN_OFF_FILE_PATH,
        PLANNING_DIRECTION_DOCS_DIRECTORY, PLANNING_PROMPTS_DIRECTORY, QUEUE_SNAPSHOT_FILE_PATH,
        RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
    };

    #[derive(Default)]
    struct FakePlanningWorkspacePort {
        active_file_bodies: Mutex<HashMap<String, String>>,
    }

    impl FakePlanningWorkspacePort {
        fn active_file(&self, relative_path: &str) -> Option<String> {
            self.active_file_bodies
                .lock()
                .expect("active file mutex should not be poisoned")
                .get(relative_path)
                .cloned()
        }
    }

    impl PlanningWorkspacePort for FakePlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            unreachable!("draft staging is not used in planning reset tests")
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            unreachable!("draft loading is not used in planning reset tests")
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            unreachable!("draft writes are not used in planning reset tests")
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            let active_file_bodies = self
                .active_file_bodies
                .lock()
                .expect("active file mutex should not be poisoned");
            Ok(PlanningWorkspaceLoadRecord {
                directions_toml: active_file_bodies.get(DIRECTIONS_FILE_PATH).cloned(),
                task_ledger_json: active_file_bodies.get(TASK_LEDGER_FILE_PATH).cloned(),
                task_ledger_schema_json: active_file_bodies
                    .get(TASK_LEDGER_SCHEMA_FILE_PATH)
                    .cloned(),
                queue_snapshot_json: active_file_bodies.get(QUEUE_SNAPSHOT_FILE_PATH).cloned(),
                result_output_markdown: active_file_bodies.get(RESULT_OUTPUT_FILE_PATH).cloned(),
            })
        }

        fn load_planning_workspace_candidate_files(
            &self,
            workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            self.load_planning_workspace_files(workspace_dir)
        }

        fn commit_planning_workspace_files(
            &self,
            _workspace_dir: &str,
            record: &PlanningWorkspaceLoadRecord,
        ) -> Result<()> {
            let mut active_file_bodies = self
                .active_file_bodies
                .lock()
                .expect("active file mutex should not be poisoned");
            active_file_bodies.clear();
            if let Some(body) = record.directions_toml.as_ref() {
                active_file_bodies.insert(DIRECTIONS_FILE_PATH.to_string(), body.clone());
            }
            if let Some(body) = record.task_ledger_json.as_ref() {
                active_file_bodies.insert(TASK_LEDGER_FILE_PATH.to_string(), body.clone());
            }
            if let Some(body) = record.task_ledger_schema_json.as_ref() {
                active_file_bodies.insert(TASK_LEDGER_SCHEMA_FILE_PATH.to_string(), body.clone());
            }
            if let Some(body) = record.queue_snapshot_json.as_ref() {
                active_file_bodies.insert(QUEUE_SNAPSHOT_FILE_PATH.to_string(), body.clone());
            }
            if let Some(body) = record.result_output_markdown.as_ref() {
                active_file_bodies.insert(RESULT_OUTPUT_FILE_PATH.to_string(), body.clone());
            }
            Ok(())
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            Ok(self.active_file(relative_path))
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
            body: Option<&str>,
        ) -> Result<()> {
            let mut active_file_bodies = self
                .active_file_bodies
                .lock()
                .expect("active file mutex should not be poisoned");
            match body {
                Some(body) => {
                    active_file_bodies.insert(relative_path.to_string(), body.to_string());
                }
                None => {
                    active_file_bodies.remove(relative_path);
                }
            }
            Ok(())
        }

        fn remove_planning_workspace_entry(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<()> {
            let mut active_file_bodies = self
                .active_file_bodies
                .lock()
                .expect("active file mutex should not be poisoned");
            active_file_bodies.retain(|path, _| {
                path != relative_path
                    && !path
                        .strip_prefix(relative_path)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            });
            Ok(())
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            unreachable!("archive writes are not used in planning reset tests")
        }
    }

    fn reset_service() -> (PlanningResetService, Arc<FakePlanningWorkspacePort>) {
        let workspace_port = Arc::new(FakePlanningWorkspacePort::default());
        let service =
            PlanningResetService::new(workspace_port.clone(), PlanningBootstrapService::new());
        (service, workspace_port)
    }

    fn seed_active_workspace(
        workspace_port: &Arc<FakePlanningWorkspacePort>,
        task_ledger_json: &str,
    ) {
        let mut active_file_bodies = workspace_port
            .active_file_bodies
            .lock()
            .expect("active file mutex should not be poisoned");
        active_file_bodies.insert(
            DIRECTIONS_FILE_PATH.to_string(),
            r#"version = 1

[queue_idle]
policy = "stop"
prompt_path = ".codex-exec-loop/planning/prompts/custom.md"

[[directions]]
id = "ship-reset"
title = "Ship reset"
summary = "Recover the workspace safely."
success_criteria = ["reset is predictable"]
scope_hints = ["keep the operator in control"]
detail_doc_path = ".codex-exec-loop/planning/directions/ship-reset.md"
state = "active"
"#
            .to_string(),
        );
        active_file_bodies.insert(
            TASK_LEDGER_FILE_PATH.to_string(),
            task_ledger_json.to_string(),
        );
        active_file_bodies.insert(
            TASK_LEDGER_SCHEMA_FILE_PATH.to_string(),
            "{\"type\":\"object\"}".to_string(),
        );
        active_file_bodies.insert(
            RESULT_OUTPUT_FILE_PATH.to_string(),
            "# Result Output Prompt".to_string(),
        );
        active_file_bodies.insert(
            QUEUE_SNAPSHOT_FILE_PATH.to_string(),
            "{\"next_task\":null}".to_string(),
        );
        active_file_bodies.insert(
            format!("{PLANNING_DIRECTION_DOCS_DIRECTORY}/ship-reset.md"),
            "# Ship reset".to_string(),
        );
        active_file_bodies.insert(
            format!("{PLANNING_PROMPTS_DIRECTORY}/custom.md"),
            "# Custom queue idle prompt".to_string(),
        );
        active_file_bodies.insert(PLAN_OFF_FILE_PATH.to_string(), "plan off\n".to_string());
    }

    #[test]
    fn reset_queue_rewrites_task_ledger_and_removes_queue_snapshot() {
        let (service, workspace_port) = reset_service();
        seed_active_workspace(
            &workspace_port,
            r#"{"version":1,"tasks":[{"id":"task-1","direction_id":"ship-reset","direction_relation_note":"keep moving","title":"Do work","description":"Ship the reset flow","status":"ready","base_priority":10,"created_by":"user","last_updated_by":"user","updated_at":"2026-04-16T00:00:00Z"}]}"#,
        );

        let result = service
            .reset_workspace("/tmp/workspace", PlanningResetTarget::Queue)
            .expect("queue reset should succeed");

        assert_eq!(result.target, PlanningResetTarget::Queue);
        assert_eq!(
            result.rewritten_paths,
            vec![TASK_LEDGER_FILE_PATH.to_string()]
        );
        assert_eq!(
            result.removed_paths,
            vec![QUEUE_SNAPSHOT_FILE_PATH.to_string()]
        );
        assert_eq!(
            workspace_port.active_file(TASK_LEDGER_FILE_PATH).as_deref(),
            Some("{\n  \"version\": 1,\n  \"tasks\": []\n}")
        );
        assert!(
            workspace_port
                .active_file(QUEUE_SNAPSHOT_FILE_PATH)
                .is_none()
        );
        assert!(
            workspace_port.active_file(DIRECTIONS_FILE_PATH).is_some(),
            "directions should remain intact during queue reset"
        );
    }

    #[test]
    fn reset_directions_refuses_when_live_tasks_exist() {
        let (service, workspace_port) = reset_service();
        seed_active_workspace(
            &workspace_port,
            r#"{"version":1,"tasks":[{"id":"task-1","direction_id":"ship-reset","direction_relation_note":"keep moving","title":"Do work","description":"Ship the reset flow","status":"in_progress","base_priority":10,"created_by":"user","last_updated_by":"user","updated_at":"2026-04-16T00:00:00Z"}]}"#,
        );

        let error = service
            .reset_workspace("/tmp/workspace", PlanningResetTarget::Directions)
            .expect_err("directions reset should block while live tasks remain");

        assert!(
            error.to_string().contains(
                "planning directions reset is blocked by live tasks: task-1(in_progress)"
            )
        );
        assert!(
            workspace_port
                .active_file(&format!(
                    "{PLANNING_DIRECTION_DOCS_DIRECTORY}/ship-reset.md"
                ))
                .is_some()
        );
    }

    #[test]
    fn reset_directions_rewrites_default_scaffold_and_clears_supporting_artifacts() {
        let (service, workspace_port) = reset_service();
        seed_active_workspace(
            &workspace_port,
            r#"{"version":1,"tasks":[{"id":"task-1","direction_id":"ship-reset","direction_relation_note":"already done","title":"Finished work","description":"The prior work is complete","status":"done","base_priority":10,"created_by":"user","last_updated_by":"user","updated_at":"2026-04-16T00:00:00Z"}]}"#,
        );

        let result = service
            .reset_workspace("/tmp/workspace", PlanningResetTarget::Directions)
            .expect("directions reset should succeed when only done tasks remain");

        assert_eq!(result.target, PlanningResetTarget::Directions);
        assert_eq!(
            result.rewritten_paths,
            vec![
                DIRECTIONS_FILE_PATH.to_string(),
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            ]
        );
        assert_eq!(
            result.removed_paths,
            vec![
                PLANNING_DIRECTION_DOCS_DIRECTORY.to_string(),
                PLANNING_PROMPTS_DIRECTORY.to_string(),
                QUEUE_SNAPSHOT_FILE_PATH.to_string(),
            ]
        );
        let directions = workspace_port
            .active_file(DIRECTIONS_FILE_PATH)
            .expect("directions should be rewritten");
        assert!(directions.contains("general-workstream"));
        assert!(directions.contains(r#"policy = "review_and_enqueue""#));
        assert!(
            workspace_port
                .active_file(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH)
                .is_some()
        );
        assert!(
            workspace_port
                .active_file(&format!(
                    "{PLANNING_DIRECTION_DOCS_DIRECTORY}/ship-reset.md"
                ))
                .is_none()
        );
        assert!(
            workspace_port
                .active_file(&format!("{PLANNING_PROMPTS_DIRECTORY}/custom.md"))
                .is_none()
        );
        assert!(
            workspace_port
                .active_file(QUEUE_SNAPSHOT_FILE_PATH)
                .is_none()
        );
        assert!(
            workspace_port.active_file(TASK_LEDGER_FILE_PATH).is_some(),
            "task ledger should stay intact during directions reset"
        );
    }

    #[test]
    fn reset_all_rewrites_full_scaffold_and_turns_plan_back_on() {
        let (service, workspace_port) = reset_service();
        seed_active_workspace(
            &workspace_port,
            r#"{"version":1,"tasks":[{"id":"task-1","direction_id":"ship-reset","direction_relation_note":"blocked work","title":"Blocked work","description":"replace everything","status":"blocked","base_priority":10,"created_by":"user","last_updated_by":"user","updated_at":"2026-04-16T00:00:00Z"}]}"#,
        );

        let result = service
            .reset_workspace("/tmp/workspace", PlanningResetTarget::All)
            .expect("full reset should succeed");

        assert_eq!(result.target, PlanningResetTarget::All);
        assert_eq!(
            result.rewritten_paths,
            vec![
                DIRECTIONS_FILE_PATH.to_string(),
                TASK_LEDGER_FILE_PATH.to_string(),
                TASK_LEDGER_SCHEMA_FILE_PATH.to_string(),
                RESULT_OUTPUT_FILE_PATH.to_string(),
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            ]
        );
        assert_eq!(
            result.removed_paths,
            vec![
                PLANNING_DIRECTION_DOCS_DIRECTORY.to_string(),
                PLANNING_PROMPTS_DIRECTORY.to_string(),
                QUEUE_SNAPSHOT_FILE_PATH.to_string(),
                PLAN_OFF_FILE_PATH.to_string(),
            ]
        );
        assert!(workspace_port.active_file(PLAN_OFF_FILE_PATH).is_none());
        assert!(
            workspace_port
                .active_file(QUEUE_SNAPSHOT_FILE_PATH)
                .is_none()
        );
        assert!(
            workspace_port
                .active_file(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH)
                .is_some()
        );
        assert_eq!(
            workspace_port.active_file(TASK_LEDGER_FILE_PATH).as_deref(),
            Some("{\n  \"version\": 1,\n  \"tasks\": []\n}")
        );
    }
}
