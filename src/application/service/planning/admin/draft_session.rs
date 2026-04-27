use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use chrono::Utc;

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord,
};
use crate::application::service::planning::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, DIRECTIONS_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    TASK_LEDGER_SCHEMA_FILE_PATH,
};
use crate::domain::planning::{DirectionCatalogDocument, PlanningFileKind, PlanningWorkspaceFiles};

use super::projection::{map_queue_preview, map_validation_report};
use super::{
    PlanningAdminDraftFileView, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminFacadeService, PlanningAdminFileKey,
    PlanningAdminQueuePreview, PlanningAdminSessionView, PlanningAdminValidationView,
};
use crate::application::service::planning::PlanningDraftEditorFile;

impl PlanningAdminFacadeService {
    pub(super) fn resolve_mutated_visible_files(
        &self,
        request: &PlanningAdminDraftMutationRequest,
    ) -> Result<Vec<PlanningDraftEditorFile>> {
        let session = self.load_draft_session(PlanningAdminDraftLoadRequest {
            draft_name: request.draft_name.clone(),
            kind: request.kind,
            direction_id: request.direction_id.clone(),
        })?;
        let update_map = request
            .files
            .iter()
            .map(|update| (update.key, update.body.clone()))
            .collect::<BTreeMap<_, _>>();

        let mut files = Vec::with_capacity(session.files.len());
        for file in session.files {
            files.push(PlanningDraftEditorFile {
                active_path: file.active_path,
                staged_path: format!("{}#{}", request.draft_name, file.key.label()),
                body: update_map.get(&file.key).cloned().unwrap_or(file.body),
            });
        }
        Ok(files)
    }

    pub(super) fn build_session_view(
        &self,
        kind: PlanningAdminDraftKind,
        direction_id: Option<String>,
        loaded: PlanningDraftLoadRecord,
    ) -> Result<PlanningAdminSessionView> {
        let validation = self.validate_loaded_draft(&loaded)?;
        let files = loaded
            .staged_files
            .into_iter()
            .filter_map(|file| {
                file_key_for_kind(kind, &file.active_path).map(|key| PlanningAdminDraftFileView {
                    key,
                    label: key.label().to_string(),
                    active_path: file.active_path,
                    editor_language: key.editor_language().to_string(),
                    body: file.body,
                })
            })
            .collect::<Vec<_>>();

        Ok(PlanningAdminSessionView {
            kind,
            direction_id,
            draft_name: loaded.draft_name,
            draft_directory: loaded.draft_directory,
            editor_heading: kind.editor_heading().to_string(),
            return_path: kind.return_path().to_string(),
            files,
            validation: validation.validation,
            queue_preview: validation.queue_preview,
        })
    }

    fn validate_loaded_draft(
        &self,
        loaded: &PlanningDraftLoadRecord,
    ) -> Result<PlanningAdminDraftValidationSnapshot> {
        let staged_files = loaded
            .staged_files
            .iter()
            .map(|file| (file.active_path.as_str(), file.body.as_str()))
            .collect::<BTreeMap<_, _>>();
        let directions_toml = self.load_effective_draft_body(
            &staged_files,
            DIRECTIONS_FILE_PATH,
            PlanningFileKind::Directions,
        )?;
        let task_ledger_json = self.load_effective_draft_body(
            &staged_files,
            TASK_LEDGER_FILE_PATH,
            PlanningFileKind::TaskLedger,
        )?;
        let task_ledger_schema_json = self.load_effective_draft_body(
            &staged_files,
            TASK_LEDGER_SCHEMA_FILE_PATH,
            PlanningFileKind::TaskLedgerSchema,
        )?;
        let result_output_markdown = self.load_effective_draft_body(
            &staged_files,
            RESULT_OUTPUT_FILE_PATH,
            PlanningFileKind::ResultOutput,
        )?;

        let mut result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml: &directions_toml,
                    task_ledger_json: &task_ledger_json,
                    task_ledger_schema_json: &task_ledger_schema_json,
                    result_output_markdown: &result_output_markdown,
                });

        if let Some(directions) = result.directions.as_ref() {
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| {
                        staged_files.contains_key(path)
                            || self
                                .planning_workspace_port
                                .load_optional_planning_file(self.workspace_dir.as_str(), path)
                                .ok()
                                .flatten()
                                .is_some()
                    },
                    &mut result.report,
                );
        }

        let queue_preview = if result.report.is_valid() {
            match (result.directions.as_ref(), result.task_ledger.as_ref()) {
                (Some(directions), Some(task_ledger)) => self
                    .priority_queue_service
                    .build_projection(directions, task_ledger)
                    .ok()
                    .map(map_queue_preview),
                _ => None,
            }
        } else {
            None
        };

        Ok(PlanningAdminDraftValidationSnapshot {
            validation: map_validation_report(&result.report),
            queue_preview,
        })
    }

    fn load_effective_draft_body(
        &self,
        staged_files: &BTreeMap<&str, &str>,
        path: &'static str,
        file_kind: PlanningFileKind,
    ) -> Result<String> {
        if let Some(body) = staged_files.get(path) {
            return Ok((*body).to_string());
        }
        self.planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), path)?
            .ok_or_else(|| missing_core_draft_file_error(path, file_kind))
    }

    pub(super) fn stage_active_manual_editor_draft(&self) -> Result<String> {
        let directions_toml = self
            .planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), DIRECTIONS_FILE_PATH)?
            .ok_or_else(|| {
                anyhow!("planning directions are unavailable; initialize planning first")
            })?;
        let task_ledger_json = self
            .planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), TASK_LEDGER_FILE_PATH)?
            .ok_or_else(|| anyhow!("task-ledger.json is unavailable; initialize planning first"))?;
        let result_output_markdown = self
            .planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), RESULT_OUTPUT_FILE_PATH)?
            .ok_or_else(|| anyhow!("result-output.md is unavailable; initialize planning first"))?;
        let task_ledger_schema_json = self
            .planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), TASK_LEDGER_SCHEMA_FILE_PATH)?
            .ok_or_else(|| {
                anyhow!("task-ledger.schema.json is unavailable; initialize planning first")
            })?;
        let mut files = vec![
            PlanningDraftFileRecord {
                active_path: DIRECTIONS_FILE_PATH.to_string(),
                body: directions_toml.clone(),
            },
            PlanningDraftFileRecord {
                active_path: TASK_LEDGER_FILE_PATH.to_string(),
                body: task_ledger_json,
            },
            PlanningDraftFileRecord {
                active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                body: result_output_markdown,
            },
            PlanningDraftFileRecord {
                active_path: TASK_LEDGER_SCHEMA_FILE_PATH.to_string(),
                body: task_ledger_schema_json,
            },
        ];

        let supporting_paths = collect_direction_supporting_paths(&directions_toml);
        for path in supporting_paths {
            if let Some(body) = self
                .planning_workspace_port
                .load_optional_planning_file(self.workspace_dir.as_str(), &path)?
            {
                files.push(PlanningDraftFileRecord {
                    active_path: path,
                    body,
                });
            }
        }

        let now = Utc::now();
        let draft_name = format!(
            "admin-{}Z-{:09}",
            now.format("%Y%m%dT%H%M%S"),
            now.timestamp_subsec_nanos()
        );
        self.planning_workspace_port.stage_planning_draft_files(
            self.workspace_dir.as_str(),
            &draft_name,
            &files,
        )?;
        Ok(draft_name)
    }
}

#[derive(Debug, Clone)]
pub(super) struct PlanningAdminDraftValidationSnapshot {
    validation: PlanningAdminValidationView,
    queue_preview: Option<PlanningAdminQueuePreview>,
}

fn collect_direction_supporting_paths(directions_toml: &str) -> Vec<String> {
    let Ok(directions) = toml::from_str::<DirectionCatalogDocument>(directions_toml) else {
        return Vec::new();
    };

    let mut paths = BTreeSet::new();
    let prompt_path = directions.queue_idle.prompt_path.trim();
    if !prompt_path.is_empty() {
        paths.insert(prompt_path.to_string());
    }

    for direction in directions.directions {
        let detail_doc_path = direction.detail_doc_path.trim();
        if !detail_doc_path.is_empty() {
            paths.insert(detail_doc_path.to_string());
        }
    }

    paths.into_iter().collect()
}

fn missing_core_draft_file_error(path: &'static str, file_kind: PlanningFileKind) -> anyhow::Error {
    anyhow!(
        "draft is missing required {} content at {}",
        match file_kind {
            PlanningFileKind::Directions => "directions",
            PlanningFileKind::TaskLedger => "task catalog compatibility file",
            PlanningFileKind::TaskLedgerSchema => "task catalog compatibility schema",
            PlanningFileKind::ResultOutput => "result output",
        },
        path
    )
}

fn file_key_for_kind(
    kind: PlanningAdminDraftKind,
    active_path: &str,
) -> Option<PlanningAdminFileKey> {
    let key = if active_path == DIRECTIONS_FILE_PATH {
        PlanningAdminFileKey::Directions
    } else if active_path == TASK_LEDGER_FILE_PATH {
        PlanningAdminFileKey::TaskLedger
    } else if active_path == RESULT_OUTPUT_FILE_PATH {
        PlanningAdminFileKey::ResultOutput
    } else if active_path == DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
        || active_path.starts_with(&format!("{PLANNING_PROMPTS_DIRECTORY}/"))
    {
        PlanningAdminFileKey::QueueIdlePrompt
    } else if active_path.starts_with(&format!("{PLANNING_DIRECTION_DOCS_DIRECTORY}/")) {
        PlanningAdminFileKey::DirectionDetail
    } else {
        return None;
    };

    match kind {
        PlanningAdminDraftKind::FullPlanning => matches!(
            key,
            PlanningAdminFileKey::Directions
                | PlanningAdminFileKey::TaskLedger
                | PlanningAdminFileKey::ResultOutput
        )
        .then_some(key),
        PlanningAdminDraftKind::Directions => matches!(
            key,
            PlanningAdminFileKey::Directions | PlanningAdminFileKey::QueueIdlePrompt
        )
        .then_some(key),
        PlanningAdminDraftKind::TaskLedger => matches!(
            key,
            PlanningAdminFileKey::TaskLedger | PlanningAdminFileKey::ResultOutput
        )
        .then_some(key),
        PlanningAdminDraftKind::QueueIdlePrompt => matches!(
            key,
            PlanningAdminFileKey::Directions | PlanningAdminFileKey::QueueIdlePrompt
        )
        .then_some(key),
        PlanningAdminDraftKind::DirectionDetail => matches!(
            key,
            PlanningAdminFileKey::Directions | PlanningAdminFileKey::DirectionDetail
        )
        .then_some(key),
    }
}
