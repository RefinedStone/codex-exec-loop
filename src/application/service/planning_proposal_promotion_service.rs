use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::{SecondsFormat, Utc};

use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning_contract::TASK_LEDGER_FILE_PATH;
use crate::application::service::planning_prompt_service::{
    PlanningPromptService, PlanningRuntimeSnapshot,
};
use crate::application::service::planning_reconciliation_service::PlanningReconciliationService;
use crate::application::service::planning_validation_service::PlanningValidationService;
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningWorkspaceFiles, TaskActor,
    TaskLedgerDocument, TaskStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningProposalPromotionRequest<'a> {
    pub workspace_directory: &'a str,
    pub root_turn_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningProposalPromotionOutcome {
    pub runtime_snapshot: PlanningRuntimeSnapshot,
    pub notices: Vec<String>,
    pub promoted_task_title: Option<String>,
    pub promoted: bool,
}

#[derive(Clone)]
pub struct PlanningProposalPromotionService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_prompt_service: PlanningPromptService,
    planning_reconciliation_service: PlanningReconciliationService,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}

impl PlanningProposalPromotionService {
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_prompt_service: PlanningPromptService,
        planning_reconciliation_service: PlanningReconciliationService,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_prompt_service,
            planning_reconciliation_service,
            planning_validation_service,
            priority_queue_service,
        }
    }

    pub fn promote_top_proposal_to_ready_if_needed(
        &self,
        request: PlanningProposalPromotionRequest<'_>,
    ) -> Result<PlanningProposalPromotionOutcome> {
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(request.workspace_directory)?;
        let (directions, mut task_ledger) =
            self.load_valid_workspace_documents(&workspace_record)?;
        let queue_snapshot = self
            .priority_queue_service
            .build_snapshot(&directions, &task_ledger)?;
        if queue_snapshot.next_task.is_some() || queue_snapshot.proposed_tasks.is_empty() {
            return Ok(PlanningProposalPromotionOutcome {
                runtime_snapshot: self
                    .planning_prompt_service
                    .load_runtime_snapshot(request.workspace_directory)?,
                notices: Vec::new(),
                promoted_task_title: None,
                promoted: false,
            });
        }

        let execution_snapshot = self
            .planning_reconciliation_service
            .load_execution_snapshot(request.workspace_directory)?;
        let top_proposal = queue_snapshot
            .proposed_tasks
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("proposal promotion requested without a promotable proposal"))?;
        let promoted_task = task_ledger
            .tasks
            .iter_mut()
            .find(|task| task.id.trim() == top_proposal.task_id.trim())
            .ok_or_else(|| {
                anyhow!(
                    "top promotable proposal {} was not found in task-ledger.json",
                    top_proposal.task_id
                )
            })?;

        promoted_task.status = TaskStatus::Ready;
        promoted_task.last_updated_by = TaskActor::System;
        promoted_task.updated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

        let next_task_ledger = serde_json::to_string_pretty(&task_ledger)?;
        self.planning_workspace_port
            .replace_planning_workspace_file(
                request.workspace_directory,
                TASK_LEDGER_FILE_PATH,
                Some(next_task_ledger.as_str()),
            )?;

        let reconciliation_result = self.planning_reconciliation_service.reconcile_after_turn(
            request.workspace_directory,
            &format!("proposal-promotion-{}", request.root_turn_id),
            &[TASK_LEDGER_FILE_PATH.to_string()],
            &execution_snapshot,
        )?;

        let runtime_snapshot =
            if let Some(block_reason) = reconciliation_result.auto_followup_block_reason.clone() {
                PlanningRuntimeSnapshot::invalid(block_reason)
            } else {
                self.planning_prompt_service
                    .load_runtime_snapshot(request.workspace_directory)?
            };

        let promoted_task_title = top_proposal.task_title.trim().to_string();
        let mut notices = reconciliation_result.notices;
        notices.push(format!(
            "host promoted top follow-up proposal into the executable queue: {}",
            promoted_task_title
        ));

        Ok(PlanningProposalPromotionOutcome {
            runtime_snapshot,
            notices,
            promoted_task_title: Some(promoted_task_title),
            promoted: true,
        })
    }

    fn load_valid_workspace_documents(
        &self,
        workspace_record: &PlanningWorkspaceLoadRecord,
    ) -> Result<(DirectionCatalogDocument, TaskLedgerDocument)> {
        let validation_result = self
            .planning_validation_service
            .validate_workspace_files(workspace_record_to_files(workspace_record)?);
        if !validation_result.is_valid() {
            let first_error = validation_result
                .report
                .errors()
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "planning validation failed".to_string());
            return Err(anyhow!(
                "cannot promote proposal from an invalid planning workspace: {first_error}"
            ));
        }

        let directions = validation_result
            .directions
            .ok_or_else(|| anyhow!("validated planning workspace did not include directions"))?;
        let task_ledger = validation_result
            .task_ledger
            .ok_or_else(|| anyhow!("validated planning workspace did not include task-ledger"))?;
        if task_ledger.version != PLANNING_FORMAT_VERSION {
            return Err(anyhow!(
                "unsupported task-ledger version {}; expected {}",
                task_ledger.version,
                PLANNING_FORMAT_VERSION
            ));
        }

        Ok((directions, task_ledger))
    }
}

fn workspace_record_to_files(
    workspace_record: &PlanningWorkspaceLoadRecord,
) -> Result<PlanningWorkspaceFiles<'_>> {
    Ok(PlanningWorkspaceFiles {
        directions_toml: workspace_record
            .directions_toml
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing directions.toml"))?,
        task_ledger_json: workspace_record
            .task_ledger_json
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing task-ledger.json"))?,
        task_ledger_schema_json: workspace_record
            .task_ledger_schema_json
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing task-ledger.schema.json"))?,
        result_output_markdown: workspace_record
            .result_output_markdown
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing result-output.md"))?,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{PlanningProposalPromotionRequest, PlanningProposalPromotionService};
    use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
    use crate::application::service::planning_bootstrap_service::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning_contract::TASK_LEDGER_FILE_PATH;
    use crate::application::service::planning_prompt_service::PlanningPromptService;
    use crate::application::service::planning_reconciliation_service::PlanningReconciliationService;
    use crate::application::service::planning_validation_service::PlanningValidationService;
    use crate::application::service::priority_queue_service::PriorityQueueService;

    fn create_temp_workspace(prefix: &str) -> String {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&path).expect("temp workspace should be created");
        path.display().to_string()
    }

    fn write_bootstrap_workspace(workspace_dir: &str) {
        let planning_dir = Path::new(workspace_dir).join(".codex-exec-loop/planning");
        fs::create_dir_all(&planning_dir).expect("planning directory should be created");
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        fs::write(
            planning_dir.join("directions.toml"),
            artifacts.directions_toml,
        )
        .expect("directions should write");
        fs::write(
            planning_dir.join("task-ledger.schema.json"),
            artifacts.task_ledger_schema_json,
        )
        .expect("schema should write");
        fs::write(
            planning_dir.join("result-output.md"),
            artifacts.result_output_markdown,
        )
        .expect("result output should write");
        for file in artifacts.supplemental_files {
            let file_path = Path::new(workspace_dir).join(&file.active_path);
            fs::create_dir_all(
                file_path
                    .parent()
                    .expect("supplemental planning file should have a parent"),
            )
            .expect("supplemental planning directory should be created");
            fs::write(file_path, file.body).expect("supplemental planning file should write");
        }
    }

    fn service() -> PlanningProposalPromotionService {
        let workspace_port: Arc<dyn PlanningWorkspacePort> =
            Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let validation_service = PlanningValidationService::new();
        let priority_queue_service = PriorityQueueService::new();
        let planning_prompt_service = PlanningPromptService::new(
            workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );
        let planning_reconciliation_service = PlanningReconciliationService::new(
            workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );

        PlanningProposalPromotionService::new(
            workspace_port,
            planning_prompt_service,
            planning_reconciliation_service,
            validation_service,
            priority_queue_service,
        )
    }

    #[test]
    fn promote_top_proposal_to_ready_rebuilds_queue_snapshot() {
        let workspace_dir = create_temp_workspace("planning-proposal-promotion");
        write_bootstrap_workspace(&workspace_dir);
        let task_ledger = r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-proposal-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Follow-up option offered in the latest answer.",
      "title": "Draft a Korea-specific Chinese-chef job entry guide",
      "description": "Expand the answer into a Korea-specific hiring guide.",
      "status": "proposed",
      "base_priority": 70,
      "dynamic_priority_delta": 0,
      "priority_reason": "First follow-up branch from the latest answer.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": null,
      "updated_at": "2026-04-13T00:00:00Z"
    },
    {
      "id": "task-proposal-2",
      "direction_id": "general-workstream",
      "direction_relation_note": "Alternate follow-up option offered in the latest answer.",
      "title": "Create a beginner 3-month Chinese-cooking training plan",
      "description": "Turn the answer into a 3-month training plan.",
      "status": "proposed",
      "base_priority": 65,
      "dynamic_priority_delta": 0,
      "priority_reason": "Second follow-up branch from the latest answer.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": null,
      "updated_at": "2026-04-13T00:00:00Z"
    }
  ]
}"#;
        fs::write(
            Path::new(&workspace_dir).join(".codex-exec-loop/planning/task-ledger.json"),
            task_ledger,
        )
        .expect("task ledger should write");

        let outcome = service()
            .promote_top_proposal_to_ready_if_needed(PlanningProposalPromotionRequest {
                workspace_directory: &workspace_dir,
                root_turn_id: "turn-1",
            })
            .expect("proposal promotion should succeed");

        assert!(outcome.promoted);
        assert_eq!(
            outcome.promoted_task_title.as_deref(),
            Some("Draft a Korea-specific Chinese-chef job entry guide")
        );
        assert!(outcome.runtime_snapshot.has_actionable_queue_head());
        assert_eq!(
            outcome
                .runtime_snapshot
                .queue_head()
                .map(|task| task.task_id.as_str()),
            Some("task-proposal-1")
        );
        assert!(outcome.notices.iter().any(|notice| {
            notice.contains("host promoted top follow-up proposal into the executable queue")
        }));

        let persisted_task_ledger =
            fs::read_to_string(Path::new(&workspace_dir).join(TASK_LEDGER_FILE_PATH))
                .expect("promoted task ledger should read");
        assert!(persisted_task_ledger.contains("\"status\": \"ready\""));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn promote_top_proposal_returns_error_when_workspace_is_incomplete() {
        let workspace_dir = create_temp_workspace("planning-proposal-promotion-missing");
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::create_dir_all(&planning_dir).expect("planning directory should be created");
        fs::write(
            planning_dir.join("task-ledger.json"),
            "{\"version\":1,\"tasks\":[]}",
        )
        .expect("task ledger should write");

        let error = service()
            .promote_top_proposal_to_ready_if_needed(PlanningProposalPromotionRequest {
                workspace_directory: &workspace_dir,
                root_turn_id: "turn-missing",
            })
            .expect_err("incomplete workspace should return an error");

        assert!(error.to_string().contains("missing directions.toml"));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }
}
