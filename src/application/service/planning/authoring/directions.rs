use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::Utc;
use toml_edit::{DocumentMut, Item, value};

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, DIRECTIONS_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    TASK_LEDGER_SCHEMA_FILE_PATH, default_direction_detail_doc_path,
};
use crate::application::service::planning::authoring::init::{
    PlanningDraftEditorFile, PlanningDraftEditorSession,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningValidationReport, QueueIdlePolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionsSupportingFileStatus {
    MissingMapping,
    Ready,
    BrokenMapping,
}

impl DirectionsSupportingFileStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::MissingMapping => "unset",
            Self::Ready => "ready",
            Self::BrokenMapping => "broken",
        }
    }

    pub fn needs_attention(self) -> bool {
        self != Self::Ready
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceDirectionSummary {
    pub id: String,
    pub title: String,
    pub detail_doc_path: Option<String>,
    pub detail_doc_status: DirectionsSupportingFileStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceSummary {
    pub directions: Vec<DirectionsMaintenanceDirectionSummary>,
    pub missing_detail_doc_count: usize,
    pub broken_detail_doc_count: usize,
    pub queue_idle_policy: QueueIdlePolicy,
    pub queue_idle_prompt_path: Option<String>,
    pub queue_idle_prompt_status: DirectionsSupportingFileStatus,
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueIdleReviewContext {
    pub policy: QueueIdlePolicy,
    pub prompt_path: Option<String>,
    pub prompt_markdown: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDoctorOutcome {
    pub repaired_detail_doc_mappings: usize,
    pub created_detail_doc_files: usize,
    pub repaired_queue_idle_prompt_mapping: bool,
    pub created_queue_idle_prompt_file: bool,
    pub validation_report: PlanningValidationReport,
}

impl PlanningDoctorOutcome {
    pub fn applied_fix_count(&self) -> usize {
        self.repaired_detail_doc_mappings
            + self.created_detail_doc_files
            + usize::from(self.repaired_queue_idle_prompt_mapping)
            + usize::from(self.created_queue_idle_prompt_file)
    }
}

#[derive(Clone)]
pub struct PlanningDirectionsService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
}

impl PlanningDirectionsService {
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_validation_service,
        }
    }

    pub fn load_summary(&self, workspace_dir: &str) -> Result<DirectionsMaintenanceSummary> {
        let directions_toml = self
            .planning_workspace_port
            .load_optional_planning_file(workspace_dir, DIRECTIONS_FILE_PATH)?
            .ok_or_else(|| {
                anyhow!("planning directions are unavailable; initialize planning first")
            })?;
        let parsed = match toml::from_str::<DirectionCatalogDocument>(&directions_toml) {
            Ok(document) => Some(document),
            Err(error) => {
                return Ok(DirectionsMaintenanceSummary {
                    directions: Vec::new(),
                    missing_detail_doc_count: 0,
                    broken_detail_doc_count: 0,
                    queue_idle_policy: QueueIdlePolicy::Stop,
                    queue_idle_prompt_path: None,
                    queue_idle_prompt_status: DirectionsSupportingFileStatus::MissingMapping,
                    parse_error: Some(format!("failed to parse directions.toml: {error}")),
                });
            }
        };
        let catalog = parsed.expect("parsed directions should exist");
        let queue_idle_prompt_path =
            trimmed_non_empty(catalog.queue_idle.prompt_path.as_str()).map(str::to_string);
        let queue_idle_prompt_status = self.supporting_file_status(
            workspace_dir,
            queue_idle_prompt_path.as_deref(),
            PLANNING_PROMPTS_DIRECTORY,
        );

        let directions = catalog
            .directions
            .into_iter()
            .map(|direction| {
                let detail_doc_path =
                    trimmed_non_empty(direction.detail_doc_path.as_str()).map(str::to_string);
                let detail_doc_status = self.supporting_file_status(
                    workspace_dir,
                    detail_doc_path.as_deref(),
                    PLANNING_DIRECTION_DOCS_DIRECTORY,
                );

                Ok(DirectionsMaintenanceDirectionSummary {
                    id: direction.id.trim().to_string(),
                    title: direction.title.trim().to_string(),
                    detail_doc_path,
                    detail_doc_status,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let missing_detail_doc_count = directions
            .iter()
            .filter(|direction| {
                direction.detail_doc_status == DirectionsSupportingFileStatus::MissingMapping
            })
            .count();
        let broken_detail_doc_count = directions
            .iter()
            .filter(|direction| {
                direction.detail_doc_status == DirectionsSupportingFileStatus::BrokenMapping
            })
            .count();

        Ok(DirectionsMaintenanceSummary {
            directions,
            missing_detail_doc_count,
            broken_detail_doc_count,
            queue_idle_policy: catalog.queue_idle.policy,
            queue_idle_prompt_path,
            queue_idle_prompt_status,
            parse_error: None,
        })
    }

    pub fn load_queue_idle_review_context(
        &self,
        workspace_dir: &str,
    ) -> Result<QueueIdleReviewContext> {
        let directions_toml = self
            .planning_workspace_port
            .load_optional_planning_file(workspace_dir, DIRECTIONS_FILE_PATH)?
            .ok_or_else(|| {
                anyhow!("planning directions are unavailable; initialize planning first")
            })?;
        let directions: DirectionCatalogDocument = toml::from_str(&directions_toml)
            .map_err(|error| anyhow!("failed to parse directions.toml: {error}"))?;
        let prompt_path =
            trimmed_non_empty(directions.queue_idle.prompt_path.as_str()).map(str::to_string);
        let prompt_markdown = prompt_path
            .as_deref()
            .and_then(|path| self.load_supporting_file_best_effort(workspace_dir, path));

        Ok(QueueIdleReviewContext {
            policy: directions.queue_idle.policy,
            prompt_path,
            prompt_markdown,
        })
    }

    pub fn stage_editor_session(&self, workspace_dir: &str) -> Result<PlanningDraftEditorSession> {
        let workspace = self.load_complete_workspace(workspace_dir)?;
        let editable_paths =
            match toml::from_str::<DirectionCatalogDocument>(&workspace.directions_toml) {
                Ok(directions) => {
                    if let Some(prompt_path) =
                        trimmed_non_empty(directions.queue_idle.prompt_path.as_str())
                    {
                        if workspace
                            .extra_files
                            .iter()
                            .any(|file| file.active_path == prompt_path)
                        {
                            vec![DIRECTIONS_FILE_PATH.to_string(), prompt_path.to_string()]
                        } else {
                            vec![DIRECTIONS_FILE_PATH.to_string()]
                        }
                    } else {
                        vec![DIRECTIONS_FILE_PATH.to_string()]
                    }
                }
                Err(_) => vec![DIRECTIONS_FILE_PATH.to_string()],
            };

        self.stage_session_from_source(workspace_dir, workspace, &editable_paths)
    }

    pub fn stage_detail_doc_editor_session(
        &self,
        workspace_dir: &str,
        direction_id: &str,
    ) -> Result<PlanningDraftEditorSession> {
        let mut workspace = self.load_complete_workspace(workspace_dir)?;
        let directions: DirectionCatalogDocument = toml::from_str(&workspace.directions_toml)
            .map_err(|error| anyhow!("failed to parse directions.toml: {error}"))?;
        let selected_direction = directions
            .directions
            .iter()
            .find(|direction| direction.id.trim() == direction_id.trim())
            .ok_or_else(|| anyhow!("unknown direction id: {}", direction_id.trim()))?;
        let (detail_doc_path, detail_doc_body) = self.resolve_detail_doc_editor_target(
            workspace_dir,
            direction_id,
            trimmed_non_empty(selected_direction.detail_doc_path.as_str()),
        )?;
        let next_directions_toml = set_direction_detail_doc_path(
            &workspace.directions_toml,
            direction_id,
            &detail_doc_path,
        )?;
        workspace.directions_toml = next_directions_toml;
        workspace
            .extra_files
            .retain(|file| file.active_path != detail_doc_path);
        workspace.extra_files.push(PlanningDraftFileRecord {
            active_path: detail_doc_path.clone(),
            body: detail_doc_body,
        });

        self.stage_session_from_source(
            workspace_dir,
            workspace,
            &[DIRECTIONS_FILE_PATH.to_string(), detail_doc_path],
        )
    }

    pub fn stage_queue_idle_prompt_editor_session(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningDraftEditorSession> {
        let mut workspace = self.load_complete_workspace(workspace_dir)?;
        let directions: DirectionCatalogDocument = toml::from_str(&workspace.directions_toml)
            .map_err(|error| anyhow!("failed to parse directions.toml: {error}"))?;
        let (prompt_path, prompt_body) = self.resolve_queue_idle_prompt_editor_target(
            workspace_dir,
            trimmed_non_empty(directions.queue_idle.prompt_path.as_str()),
        )?;
        let next_directions_toml =
            set_queue_idle_prompt_path(&workspace.directions_toml, &prompt_path)?;
        workspace.directions_toml = next_directions_toml;
        workspace
            .extra_files
            .retain(|file| file.active_path != prompt_path);
        workspace.extra_files.push(PlanningDraftFileRecord {
            active_path: prompt_path.clone(),
            body: prompt_body,
        });

        self.stage_session_from_source(
            workspace_dir,
            workspace,
            &[DIRECTIONS_FILE_PATH.to_string(), prompt_path],
        )
    }

    #[cfg(test)]
    pub fn doctor_workspace(&self, workspace_dir: &str) -> Result<PlanningDoctorOutcome> {
        let workspace = self.load_complete_workspace(workspace_dir)?;
        let directions: DirectionCatalogDocument = toml::from_str(&workspace.directions_toml)
            .map_err(|error| anyhow!("failed to parse directions.toml: {error}"))?;
        let mut next_directions_toml = workspace.directions_toml.clone();
        let mut repaired_detail_doc_mappings = 0;
        let mut created_detail_doc_files = 0;
        let mut repaired_queue_idle_prompt_mapping = false;
        let mut created_queue_idle_prompt_file = false;
        let mut pending_supporting_files = HashMap::<String, String>::new();

        for direction in &directions.directions {
            let configured_path = trimmed_non_empty(direction.detail_doc_path.as_str());
            let target_path = if configured_path.is_some_and(|path| {
                is_valid_planning_markdown_path(path, PLANNING_DIRECTION_DOCS_DIRECTORY)
            }) {
                configured_path.expect("checked above").to_string()
            } else {
                default_validated_direction_detail_doc_path(&direction.id)?
            };

            if configured_path != Some(target_path.as_str()) {
                next_directions_toml = set_direction_detail_doc_path(
                    &next_directions_toml,
                    &direction.id,
                    &target_path,
                )?;
                repaired_detail_doc_mappings += 1;
            }

            if self
                .load_supporting_file_best_effort(workspace_dir, &target_path)
                .is_none()
                && pending_supporting_files
                    .insert(
                        target_path.clone(),
                        build_default_detail_doc_markdown(direction),
                    )
                    .is_none()
            {
                created_detail_doc_files += 1;
            }
        }

        let configured_prompt_path = trimmed_non_empty(directions.queue_idle.prompt_path.as_str());
        let should_repair_queue_idle_prompt = directions.queue_idle.policy
            == QueueIdlePolicy::ReviewAndEnqueue
            || configured_prompt_path.is_some();
        if should_repair_queue_idle_prompt {
            let target_prompt_path = if configured_prompt_path.is_some_and(|path| {
                is_valid_planning_markdown_path(path, PLANNING_PROMPTS_DIRECTORY)
            }) {
                configured_prompt_path.expect("checked above").to_string()
            } else {
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()
            };

            if configured_prompt_path != Some(target_prompt_path.as_str()) {
                next_directions_toml =
                    set_queue_idle_prompt_path(&next_directions_toml, &target_prompt_path)?;
                repaired_queue_idle_prompt_mapping = true;
            }

            if self
                .load_supporting_file_best_effort(workspace_dir, &target_prompt_path)
                .is_none()
                && pending_supporting_files
                    .insert(
                        target_prompt_path,
                        DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string(),
                    )
                    .is_none()
            {
                created_queue_idle_prompt_file = true;
            }
        }

        if next_directions_toml != workspace.directions_toml {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    DIRECTIONS_FILE_PATH,
                    Some(&next_directions_toml),
                )?;
        }
        for (relative_path, body) in pending_supporting_files {
            self.planning_workspace_port
                .replace_planning_workspace_file(workspace_dir, &relative_path, Some(&body))?;
        }

        let validation_report = self.validate_active_workspace(workspace_dir)?;

        Ok(PlanningDoctorOutcome {
            repaired_detail_doc_mappings,
            created_detail_doc_files,
            repaired_queue_idle_prompt_mapping,
            created_queue_idle_prompt_file,
            validation_report,
        })
    }

    fn stage_session_from_source(
        &self,
        workspace_dir: &str,
        source: ActiveDirectionsWorkspace,
        editable_paths: &[String],
    ) -> Result<PlanningDraftEditorSession> {
        let draft_name = build_maintenance_draft_name();
        let mut files = vec![
            PlanningDraftFileRecord {
                active_path: DIRECTIONS_FILE_PATH.to_string(),
                body: source.directions_toml,
            },
            PlanningDraftFileRecord {
                active_path: TASK_LEDGER_FILE_PATH.to_string(),
                body: source.task_ledger_json,
            },
            PlanningDraftFileRecord {
                active_path: TASK_LEDGER_SCHEMA_FILE_PATH.to_string(),
                body: source.task_ledger_schema_json,
            },
            PlanningDraftFileRecord {
                active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                body: source.result_output_markdown,
            },
        ];
        files.extend(source.extra_files);
        self.planning_workspace_port.stage_planning_draft_files(
            workspace_dir,
            &draft_name,
            &files,
        )?;
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(workspace_dir, &draft_name)?;
        let validation_report = validate_loaded_draft(&self.planning_validation_service, &loaded);

        let editable_path_set = editable_paths.iter().cloned().collect::<HashSet<_>>();
        Ok(PlanningDraftEditorSession {
            draft_name: loaded.draft_name.clone(),
            draft_directory: loaded.draft_directory.clone(),
            editable_files: loaded
                .staged_files
                .into_iter()
                .filter(|file| editable_path_set.contains(file.active_path.as_str()))
                .map(|file| PlanningDraftEditorFile {
                    active_path: file.active_path,
                    staged_path: file.staged_path,
                    body: file.body,
                })
                .collect(),
            validation_report,
        })
    }

    fn load_complete_workspace(&self, workspace_dir: &str) -> Result<ActiveDirectionsWorkspace> {
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let mut active_workspace = ActiveDirectionsWorkspace {
            directions_toml: workspace.directions_toml.ok_or_else(|| {
                anyhow!("planning directions are unavailable; initialize planning first")
            })?,
            task_ledger_json: workspace
                .task_ledger_json
                .ok_or_else(|| anyhow!("planning workspace is missing task-ledger.json"))?,
            task_ledger_schema_json: workspace
                .task_ledger_schema_json
                .ok_or_else(|| anyhow!("planning workspace is missing task-ledger.schema.json"))?,
            result_output_markdown: workspace
                .result_output_markdown
                .ok_or_else(|| anyhow!("planning workspace is missing result-output.md"))?,
            extra_files: Vec::new(),
        };
        if let Ok(directions) =
            toml::from_str::<DirectionCatalogDocument>(&active_workspace.directions_toml)
        {
            let mut supporting_paths = HashSet::new();
            if let Some(prompt_path) = trimmed_non_empty(directions.queue_idle.prompt_path.as_str())
            {
                supporting_paths.insert(prompt_path.to_string());
            }
            supporting_paths.extend(
                directions
                    .directions
                    .iter()
                    .filter_map(|direction| trimmed_non_empty(direction.detail_doc_path.as_str()))
                    .map(str::to_string),
            );
            for supporting_path in supporting_paths {
                if let Some(body) =
                    self.load_supporting_file_best_effort(workspace_dir, &supporting_path)
                {
                    active_workspace.extra_files.push(PlanningDraftFileRecord {
                        active_path: supporting_path,
                        body,
                    });
                }
            }
        }
        Ok(active_workspace)
    }

    #[cfg(test)]
    fn validate_active_workspace(&self, workspace_dir: &str) -> Result<PlanningValidationReport> {
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let mut result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions_toml: workspace.directions_toml.as_deref().ok_or_else(|| {
                    anyhow!("planning directions are unavailable; initialize planning first")
                })?,
                task_ledger_json: workspace
                    .task_ledger_json
                    .as_deref()
                    .ok_or_else(|| anyhow!("planning workspace is missing task-ledger.json"))?,
                task_ledger_schema_json: workspace.task_ledger_schema_json.as_deref().ok_or_else(
                    || anyhow!("planning workspace is missing task-ledger.schema.json"),
                )?,
                result_output_markdown: workspace
                    .result_output_markdown
                    .as_deref()
                    .ok_or_else(|| anyhow!("planning workspace is missing result-output.md"))?,
            },
        );
        if let Some(directions) = result.directions.as_ref() {
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| {
                        self.load_supporting_file_best_effort(workspace_dir, path)
                            .is_some()
                    },
                    &mut result.report,
                );
        }

        Ok(result.report)
    }

    fn load_supporting_file_best_effort(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Option<String> {
        self.planning_workspace_port
            .load_optional_planning_file(workspace_dir, relative_path)
            .ok()
            .flatten()
    }

    fn supporting_file_status(
        &self,
        workspace_dir: &str,
        configured_path: Option<&str>,
        required_prefix: &str,
    ) -> DirectionsSupportingFileStatus {
        let Some(path) = configured_path else {
            return DirectionsSupportingFileStatus::MissingMapping;
        };
        if !is_valid_planning_markdown_path(path, required_prefix) {
            return DirectionsSupportingFileStatus::BrokenMapping;
        }
        if self
            .load_supporting_file_best_effort(workspace_dir, path)
            .is_some()
        {
            DirectionsSupportingFileStatus::Ready
        } else {
            DirectionsSupportingFileStatus::BrokenMapping
        }
    }

    fn resolve_detail_doc_editor_target(
        &self,
        workspace_dir: &str,
        direction_id: &str,
        configured_path: Option<&str>,
    ) -> Result<(String, String)> {
        if let Some(path) = configured_path
            .filter(|path| is_valid_planning_markdown_path(path, PLANNING_DIRECTION_DOCS_DIRECTORY))
        {
            match self
                .planning_workspace_port
                .load_optional_planning_file(workspace_dir, path)
            {
                Ok(Some(body)) => return Ok((path.to_string(), body)),
                Ok(None) => return Ok((path.to_string(), String::new())),
                Err(_) => {}
            }
        }

        let fallback_path = default_direction_detail_doc_path(direction_id);
        let fallback_body = self
            .load_supporting_file_best_effort(workspace_dir, &fallback_path)
            .unwrap_or_default();
        Ok((fallback_path, fallback_body))
    }

    fn resolve_queue_idle_prompt_editor_target(
        &self,
        workspace_dir: &str,
        configured_path: Option<&str>,
    ) -> Result<(String, String)> {
        if let Some(path) = configured_path
            .filter(|path| is_valid_planning_markdown_path(path, PLANNING_PROMPTS_DIRECTORY))
        {
            match self
                .planning_workspace_port
                .load_optional_planning_file(workspace_dir, path)
            {
                Ok(Some(body)) => return Ok((path.to_string(), body)),
                Ok(None) => {
                    return Ok((
                        path.to_string(),
                        DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string(),
                    ));
                }
                Err(_) => {}
            }
        }

        let fallback_path = DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string();
        let fallback_body = self
            .load_supporting_file_best_effort(workspace_dir, &fallback_path)
            .unwrap_or_else(|| DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string());
        Ok((fallback_path, fallback_body))
    }
}

struct ActiveDirectionsWorkspace {
    directions_toml: String,
    task_ledger_json: String,
    task_ledger_schema_json: String,
    result_output_markdown: String,
    extra_files: Vec<PlanningDraftFileRecord>,
}

fn trimmed_non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn build_maintenance_draft_name() -> String {
    let now = Utc::now();
    format!(
        "directions-{}Z-{:09}",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    )
}

#[cfg(test)]
fn build_default_detail_doc_markdown(
    direction: &crate::domain::planning::DirectionDefinition,
) -> String {
    let mut lines = vec![
        format!("# {}", direction.title.trim()),
        String::new(),
        format!("- Direction id: `{}`", direction.id.trim()),
        String::new(),
        "## Goal".to_string(),
        String::new(),
        direction.summary.trim().to_string(),
    ];
    if !direction.success_criteria.is_empty() {
        lines.push(String::new());
        lines.push("## Success criteria".to_string());
        lines.push(String::new());
        lines.extend(
            direction
                .success_criteria
                .iter()
                .map(|criterion| format!("- {}", criterion.trim())),
        );
    }
    if !direction.scope_hints.is_empty() {
        lines.push(String::new());
        lines.push("## Scope hints".to_string());
        lines.push(String::new());
        lines.extend(
            direction
                .scope_hints
                .iter()
                .map(|hint| format!("- {}", hint.trim())),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
fn default_validated_direction_detail_doc_path(direction_id: &str) -> Result<String> {
    let fallback_path = default_direction_detail_doc_path(direction_id);
    if is_valid_planning_markdown_path(&fallback_path, PLANNING_DIRECTION_DOCS_DIRECTORY) {
        Ok(fallback_path)
    } else {
        Err(anyhow!(
            "direction {} does not produce a safe default detail_doc_path",
            direction_id.trim()
        ))
    }
}

fn is_valid_planning_markdown_path(path: &str, required_prefix: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains("../")
        || normalized.contains("/..")
        || Path::new(&normalized)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }

    let Some(suffix) = normalized.strip_prefix(required_prefix) else {
        return false;
    };

    suffix.starts_with('/') && suffix.len() > 1 && normalized.ends_with(".md")
}

fn validate_loaded_draft(
    validation_service: &PlanningValidationService,
    loaded: &PlanningDraftLoadRecord,
) -> crate::domain::planning::PlanningValidationReport {
    let staged_file_map = loaded
        .staged_files
        .iter()
        .map(|file| (file.active_path.as_str(), file.body.as_str()))
        .collect::<HashMap<_, _>>();
    let mut result = validation_service.validate_workspace_files(
        crate::domain::planning::PlanningWorkspaceFiles {
            directions_toml: staged_file_map
                .get(DIRECTIONS_FILE_PATH)
                .copied()
                .unwrap_or_default(),
            task_ledger_json: staged_file_map
                .get(TASK_LEDGER_FILE_PATH)
                .copied()
                .unwrap_or_default(),
            task_ledger_schema_json: staged_file_map
                .get(TASK_LEDGER_SCHEMA_FILE_PATH)
                .copied()
                .unwrap_or_default(),
            result_output_markdown: staged_file_map
                .get(RESULT_OUTPUT_FILE_PATH)
                .copied()
                .unwrap_or_default(),
        },
    );
    if let Some(directions) = result.directions.as_ref() {
        validation_service.validate_direction_supporting_files(
            directions,
            |path| staged_file_map.contains_key(path),
            &mut result.report,
        );
    }

    result.report
}

fn set_direction_detail_doc_path(
    directions_toml: &str,
    direction_id: &str,
    detail_doc_path: &str,
) -> Result<String> {
    let mut document = directions_toml
        .parse::<DocumentMut>()
        .map_err(|error| anyhow!("failed to parse directions.toml: {error}"))?;
    let tables = document["directions"]
        .as_array_of_tables_mut()
        .ok_or_else(|| anyhow!("directions.toml does not contain [[directions]] tables"))?;
    let mut updated = false;
    for table in tables.iter_mut() {
        let Some(id) = table.get("id").and_then(|item| item.as_str()) else {
            continue;
        };
        if id.trim() == direction_id.trim() {
            table["detail_doc_path"] = value(detail_doc_path);
            updated = true;
            break;
        }
    }

    if updated {
        Ok(document.to_string())
    } else {
        Err(anyhow!("unknown direction id: {}", direction_id.trim()))
    }
}

fn set_queue_idle_prompt_path(directions_toml: &str, prompt_path: &str) -> Result<String> {
    let mut document = directions_toml
        .parse::<DocumentMut>()
        .map_err(|error| anyhow!("failed to parse directions.toml: {error}"))?;
    if !document.as_table().contains_key("queue_idle") {
        document["queue_idle"] = Item::Table(Default::default());
        document["queue_idle"]["policy"] = value("stop");
    }
    document["queue_idle"]["prompt_path"] = value(prompt_path);
    Ok(document.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN,
        DIRECTIONS_FILE_PATH, DirectionsSupportingFileStatus, PlanningDirectionsService,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::service::planning::authoring::bootstrap::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning::shared::contract::default_direction_detail_doc_path;
    use crate::application::service::planning::runtime::validation::PlanningValidationService;
    use crate::domain::planning::QueueIdlePolicy;

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
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        let planning_dir = Path::new(workspace_dir).join(".codex-exec-loop/planning");
        fs::create_dir_all(&planning_dir).expect("planning directory should be created");
        fs::write(
            planning_dir.join("directions.toml"),
            artifacts.directions_toml,
        )
        .expect("directions should write");
        fs::write(
            planning_dir.join("task-ledger.json"),
            artifacts.task_ledger_json,
        )
        .expect("task ledger should write");
        fs::write(
            planning_dir.join("task-ledger.schema.json"),
            artifacts.task_ledger_schema_json,
        )
        .expect("task-ledger schema should write");
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

    fn rewrite_directions_toml(workspace_dir: &str, f: impl FnOnce(String) -> String) {
        let directions_path = Path::new(workspace_dir).join(DIRECTIONS_FILE_PATH);
        let directions =
            fs::read_to_string(&directions_path).expect("directions.toml should be readable");
        fs::write(&directions_path, f(directions)).expect("updated directions.toml should write");
    }

    fn sample_service() -> PlanningDirectionsService {
        PlanningDirectionsService::new(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            PlanningValidationService::new(),
        )
    }

    #[test]
    fn load_summary_reflects_simple_mode_queue_review_defaults() {
        let workspace_dir = create_temp_workspace("planning-directions-summary");
        write_bootstrap_workspace(&workspace_dir);

        let summary = sample_service()
            .load_summary(&workspace_dir)
            .expect("directions summary should load");

        assert_eq!(summary.queue_idle_policy, QueueIdlePolicy::ReviewAndEnqueue);
        assert_eq!(
            summary.queue_idle_prompt_path,
            Some(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string())
        );
        assert_eq!(
            summary.queue_idle_prompt_status,
            DirectionsSupportingFileStatus::Ready
        );
        assert_eq!(summary.directions.len(), 1);
        assert_eq!(summary.directions[0].detail_doc_path, None);
        assert_eq!(
            summary.directions[0].detail_doc_status,
            DirectionsSupportingFileStatus::MissingMapping
        );
        assert_eq!(summary.missing_detail_doc_count, 1);
        assert_eq!(summary.broken_detail_doc_count, 0);

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn stage_detail_doc_editor_session_stages_generated_detail_doc_mapping() {
        let workspace_dir = create_temp_workspace("planning-directions-detail-doc");
        write_bootstrap_workspace(&workspace_dir);

        let session = sample_service()
            .stage_detail_doc_editor_session(&workspace_dir, "general-workstream")
            .expect("detail doc editor session should stage");

        let directions = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DIRECTIONS_FILE_PATH)
            .expect("directions.toml should be editable");
        let detail_doc_path = default_direction_detail_doc_path("general-workstream");
        let detail_doc = session
            .editable_files
            .iter()
            .find(|file| file.active_path == detail_doc_path)
            .expect("generated detail doc should be editable");

        assert!(
            directions
                .body
                .contains(&format!(r#"detail_doc_path = "{detail_doc_path}""#))
        );
        assert_eq!(detail_doc.body, "");
        assert!(session.validation_report.is_valid());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn stage_editor_session_tolerates_invalid_supporting_paths() {
        let workspace_dir = create_temp_workspace("planning-directions-invalid-summary");
        write_bootstrap_workspace(&workspace_dir);
        rewrite_directions_toml(&workspace_dir, |directions| {
            directions
                .replace(
                    r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
                    r#"prompt_path = "../escape.md""#,
                )
                .replace(
                    r#"detail_doc_path = """#,
                    r#"detail_doc_path = "../detail.md""#,
                )
        });

        let summary = sample_service()
            .load_summary(&workspace_dir)
            .expect("directions summary should still load");
        assert_eq!(
            summary.queue_idle_prompt_path,
            Some("../escape.md".to_string())
        );
        assert_eq!(
            summary.queue_idle_prompt_status,
            DirectionsSupportingFileStatus::BrokenMapping
        );
        assert_eq!(
            summary.directions[0].detail_doc_path,
            Some("../detail.md".to_string())
        );
        assert_eq!(
            summary.directions[0].detail_doc_status,
            DirectionsSupportingFileStatus::BrokenMapping
        );
        assert_eq!(summary.missing_detail_doc_count, 0);
        assert_eq!(summary.broken_detail_doc_count, 1);

        let session = sample_service()
            .stage_editor_session(&workspace_dir)
            .expect("directions editor should still stage");
        assert!(
            session
                .editable_files
                .iter()
                .any(|file| file.active_path == DIRECTIONS_FILE_PATH)
        );
        assert!(!session.validation_report.is_valid());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn doctor_workspace_repairs_invalid_existing_supporting_paths() {
        let workspace_dir = create_temp_workspace("planning-directions-doctor");
        write_bootstrap_workspace(&workspace_dir);
        fs::write(
            Path::new(&workspace_dir).join("README.md"),
            "# not a planning supporting path\n",
        )
        .expect("workspace readme should write");
        rewrite_directions_toml(&workspace_dir, |directions| {
            directions
                .replace(
                    r#"detail_doc_path = """#,
                    r#"detail_doc_path = "README.md""#,
                )
                .replace(
                    r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
                    r#"prompt_path = "README.md""#,
                )
        });

        let outcome = sample_service()
            .doctor_workspace(&workspace_dir)
            .expect("planning doctor should repair safe path issues");
        let directions = fs::read_to_string(Path::new(&workspace_dir).join(DIRECTIONS_FILE_PATH))
            .expect("directions.toml should be readable after doctor");
        let detail_doc_path = default_direction_detail_doc_path("general-workstream");

        assert_eq!(outcome.repaired_detail_doc_mappings, 1);
        assert_eq!(outcome.created_detail_doc_files, 1);
        assert!(outcome.repaired_queue_idle_prompt_mapping);
        assert!(!outcome.created_queue_idle_prompt_file);
        assert!(outcome.validation_report.is_valid());
        assert!(directions.contains(&format!(r#"detail_doc_path = "{detail_doc_path}""#)));
        assert!(directions.contains(&format!(
            r#"prompt_path = "{DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH}""#
        )));
        assert!(Path::new(&workspace_dir).join(&detail_doc_path).is_file());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn doctor_workspace_preserves_valid_custom_paths_and_creates_missing_files() {
        let workspace_dir = create_temp_workspace("planning-directions-doctor-custom-paths");
        write_bootstrap_workspace(&workspace_dir);
        let custom_detail_doc_path = ".codex-exec-loop/planning/directions/custom-detail.md";
        let custom_prompt_path = ".codex-exec-loop/planning/prompts/custom-review.md";
        rewrite_directions_toml(&workspace_dir, |directions| {
            directions
                .replace(
                    r#"detail_doc_path = """#,
                    &format!(r#"detail_doc_path = "{custom_detail_doc_path}""#),
                )
                .replace(
                    r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
                    &format!(r#"prompt_path = "{custom_prompt_path}""#),
                )
        });
        fs::remove_file(Path::new(&workspace_dir).join(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH))
            .expect("default queue-idle prompt should be removable");

        let outcome = sample_service()
            .doctor_workspace(&workspace_dir)
            .expect("planning doctor should preserve valid custom paths");
        let directions = fs::read_to_string(Path::new(&workspace_dir).join(DIRECTIONS_FILE_PATH))
            .expect("directions.toml should be readable after doctor");

        assert_eq!(outcome.repaired_detail_doc_mappings, 0);
        assert_eq!(outcome.created_detail_doc_files, 1);
        assert!(!outcome.repaired_queue_idle_prompt_mapping);
        assert!(outcome.created_queue_idle_prompt_file);
        assert!(outcome.validation_report.is_valid());
        assert!(directions.contains(&format!(r#"detail_doc_path = "{custom_detail_doc_path}""#)));
        assert!(directions.contains(&format!(r#"prompt_path = "{custom_prompt_path}""#)));
        assert!(
            Path::new(&workspace_dir)
                .join(custom_detail_doc_path)
                .is_file()
        );
        assert!(Path::new(&workspace_dir).join(custom_prompt_path).is_file());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn doctor_workspace_rejects_unsafe_direction_id_without_partial_updates() {
        let workspace_dir = create_temp_workspace("planning-directions-doctor-unsafe-id");
        write_bootstrap_workspace(&workspace_dir);
        rewrite_directions_toml(&workspace_dir, |directions| {
            directions
                .replace(r#"id = "general-workstream""#, r#"id = "../escape""#)
                .replace(r#"title = "General workstream""#, r#"title = "Escape""#)
        });
        let directions_path = Path::new(&workspace_dir).join(DIRECTIONS_FILE_PATH);
        let original_directions =
            fs::read_to_string(&directions_path).expect("directions.toml should be readable");

        let error = sample_service()
            .doctor_workspace(&workspace_dir)
            .expect_err("planning doctor should reject unsafe fallback paths");

        assert!(
            error
                .to_string()
                .contains("does not produce a safe default detail_doc_path")
        );
        let directions_after =
            fs::read_to_string(&directions_path).expect("directions.toml should stay readable");
        assert_eq!(directions_after, original_directions);

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn stage_queue_idle_prompt_editor_session_stages_default_prompt_and_mapping() {
        let workspace_dir = create_temp_workspace("planning-directions-queue-idle");
        write_bootstrap_workspace(&workspace_dir);

        let session = sample_service()
            .stage_queue_idle_prompt_editor_session(&workspace_dir)
            .expect("queue-idle prompt editor session should stage");

        let directions = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DIRECTIONS_FILE_PATH)
            .expect("directions.toml should be editable");
        let prompt = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH)
            .expect("queue-idle prompt should be editable");

        assert!(directions.body.contains(&format!(
            r#"prompt_path = "{DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH}""#
        )));
        assert_eq!(prompt.body, DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN);
        assert!(session.validation_report.is_valid());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn stage_detail_doc_editor_session_recovers_from_invalid_detail_doc_path() {
        let workspace_dir = create_temp_workspace("planning-directions-invalid-detail-doc");
        write_bootstrap_workspace(&workspace_dir);
        rewrite_directions_toml(&workspace_dir, |directions| {
            directions.replace(
                r#"detail_doc_path = """#,
                r#"detail_doc_path = "../detail.md""#,
            )
        });

        let session = sample_service()
            .stage_detail_doc_editor_session(&workspace_dir, "general-workstream")
            .expect("detail doc editor should recover from invalid path");
        let detail_doc_path = default_direction_detail_doc_path("general-workstream");
        let directions = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DIRECTIONS_FILE_PATH)
            .expect("directions.toml should be editable");
        let detail_doc = session
            .editable_files
            .iter()
            .find(|file| file.active_path == detail_doc_path)
            .expect("default detail doc should be editable");

        assert!(
            directions
                .body
                .contains(&format!(r#"detail_doc_path = "{detail_doc_path}""#))
        );
        assert_eq!(detail_doc.body, "");
        assert!(session.validation_report.is_valid());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn stage_detail_doc_editor_session_ignores_non_planning_detail_doc_path() {
        let workspace_dir = create_temp_workspace("planning-directions-non-planning-detail-doc");
        write_bootstrap_workspace(&workspace_dir);
        fs::write(
            format!("{workspace_dir}/README.md"),
            "# not a direction doc",
        )
        .expect("readme should write");
        rewrite_directions_toml(&workspace_dir, |directions| {
            directions.replace(
                r#"detail_doc_path = """#,
                r#"detail_doc_path = "README.md""#,
            )
        });

        let session = sample_service()
            .stage_detail_doc_editor_session(&workspace_dir, "general-workstream")
            .expect("detail doc editor should recover from non-planning path");
        let detail_doc_path = default_direction_detail_doc_path("general-workstream");
        let directions = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DIRECTIONS_FILE_PATH)
            .expect("directions.toml should be editable");
        let detail_doc = session
            .editable_files
            .iter()
            .find(|file| file.active_path == detail_doc_path)
            .expect("default detail doc should be editable");

        assert!(
            directions
                .body
                .contains(&format!(r#"detail_doc_path = "{detail_doc_path}""#))
        );
        assert_eq!(detail_doc.body, "");
        assert!(
            session
                .editable_files
                .iter()
                .all(|file| file.active_path != "README.md")
        );
        assert!(session.validation_report.is_valid());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn stage_detail_doc_editor_session_recovers_from_parent_dir_component_path() {
        let workspace_dir = create_temp_workspace("planning-directions-parent-dir-detail-doc");
        write_bootstrap_workspace(&workspace_dir);
        rewrite_directions_toml(&workspace_dir, |directions| {
            directions.replace(
                r#"detail_doc_path = """#,
                r#"detail_doc_path = ".codex-exec-loop/planning/directions/../escape.md""#,
            )
        });

        let session = sample_service()
            .stage_detail_doc_editor_session(&workspace_dir, "general-workstream")
            .expect("detail doc editor should recover from parent-dir component path");
        let detail_doc_path = default_direction_detail_doc_path("general-workstream");
        let directions = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DIRECTIONS_FILE_PATH)
            .expect("directions.toml should be editable");

        assert!(
            directions
                .body
                .contains(&format!(r#"detail_doc_path = "{detail_doc_path}""#))
        );
        assert!(
            session
                .editable_files
                .iter()
                .all(|file| file.active_path != ".codex-exec-loop/planning/directions/../escape.md")
        );
        assert!(session.validation_report.is_valid());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn stage_queue_idle_prompt_editor_session_recovers_from_invalid_prompt_path() {
        let workspace_dir = create_temp_workspace("planning-directions-invalid-queue-idle");
        write_bootstrap_workspace(&workspace_dir);
        rewrite_directions_toml(&workspace_dir, |directions| {
            directions.replace(
                r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
                r#"prompt_path = "../escape.md""#,
            )
        });

        let session = sample_service()
            .stage_queue_idle_prompt_editor_session(&workspace_dir)
            .expect("queue-idle prompt editor should recover from invalid path");
        let directions = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DIRECTIONS_FILE_PATH)
            .expect("directions.toml should be editable");
        let prompt = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH)
            .expect("default queue-idle prompt should be editable");

        assert!(directions.body.contains(&format!(
            r#"prompt_path = "{DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH}""#
        )));
        assert_eq!(prompt.body, DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN);
        assert!(session.validation_report.is_valid());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn stage_queue_idle_prompt_editor_session_ignores_non_planning_prompt_path() {
        let workspace_dir = create_temp_workspace("planning-directions-non-planning-queue-idle");
        write_bootstrap_workspace(&workspace_dir);
        fs::write(
            format!("{workspace_dir}/README.md"),
            "# not a queue idle prompt",
        )
        .expect("readme should write");
        rewrite_directions_toml(&workspace_dir, |directions| {
            directions.replace(
                r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
                r#"prompt_path = "README.md""#,
            )
        });

        let session = sample_service()
            .stage_queue_idle_prompt_editor_session(&workspace_dir)
            .expect("queue-idle prompt editor should recover from non-planning path");
        let directions = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DIRECTIONS_FILE_PATH)
            .expect("directions.toml should be editable");
        let prompt = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH)
            .expect("default queue-idle prompt should be editable");

        assert!(directions.body.contains(&format!(
            r#"prompt_path = "{DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH}""#
        )));
        assert_eq!(prompt.body, DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN);
        assert!(
            session
                .editable_files
                .iter()
                .all(|file| file.active_path != "README.md")
        );
        assert!(session.validation_report.is_valid());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn stage_queue_idle_prompt_editor_session_recovers_from_parent_dir_component_path() {
        let workspace_dir = create_temp_workspace("planning-directions-parent-dir-queue-idle");
        write_bootstrap_workspace(&workspace_dir);
        rewrite_directions_toml(&workspace_dir, |directions| {
            directions.replace(
                r#"prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md""#,
                r#"prompt_path = ".codex-exec-loop/planning/prompts/../escape.md""#,
            )
        });

        let session = sample_service()
            .stage_queue_idle_prompt_editor_session(&workspace_dir)
            .expect("queue-idle prompt editor should recover from parent-dir component path");
        let directions = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DIRECTIONS_FILE_PATH)
            .expect("directions.toml should be editable");

        assert!(directions.body.contains(&format!(
            r#"prompt_path = "{DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH}""#
        )));
        assert!(
            session
                .editable_files
                .iter()
                .all(|file| file.active_path != ".codex-exec-loop/planning/prompts/../escape.md")
        );
        assert!(session.validation_report.is_valid());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }
}
