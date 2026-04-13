use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::Utc;
use toml_edit::{DocumentMut, Item, value};

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning_auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;
use crate::application::service::planning_init_service::{
    PlanningDraftEditorFile, PlanningDraftEditorSession,
};
use crate::application::service::planning_validation_service::PlanningValidationService;
use crate::domain::planning::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, DIRECTIONS_FILE_PATH, DirectionCatalogDocument,
    QueueIdlePolicy, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
    default_direction_detail_doc_path,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceDirectionSummary {
    pub id: String,
    pub title: String,
    pub detail_doc_path: Option<String>,
    pub detail_doc_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceSummary {
    pub directions: Vec<DirectionsMaintenanceDirectionSummary>,
    pub queue_idle_policy: QueueIdlePolicy,
    pub queue_idle_prompt_path: Option<String>,
    pub queue_idle_prompt_exists: bool,
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueIdleReviewContext {
    pub policy: QueueIdlePolicy,
    pub prompt_path: Option<String>,
    pub prompt_markdown: Option<String>,
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
                    queue_idle_policy: QueueIdlePolicy::Stop,
                    queue_idle_prompt_path: None,
                    queue_idle_prompt_exists: false,
                    parse_error: Some(format!("failed to parse directions.toml: {error}")),
                });
            }
        };
        let catalog = parsed.expect("parsed directions should exist");
        let queue_idle_prompt_path =
            trimmed_non_empty(catalog.queue_idle.prompt_path.as_str()).map(str::to_string);
        let queue_idle_prompt_exists = queue_idle_prompt_path
            .as_deref()
            .map(|path| {
                self.planning_workspace_port
                    .load_optional_planning_file(workspace_dir, path)
                    .map(|body| body.is_some())
            })
            .transpose()?
            .unwrap_or(false);

        let directions = catalog
            .directions
            .into_iter()
            .map(|direction| {
                let detail_doc_path =
                    trimmed_non_empty(direction.detail_doc_path.as_str()).map(str::to_string);
                let detail_doc_exists = detail_doc_path
                    .as_deref()
                    .map(|path| {
                        self.planning_workspace_port
                            .load_optional_planning_file(workspace_dir, path)
                            .map(|body| body.is_some())
                    })
                    .transpose()?
                    .unwrap_or(false);

                Ok(DirectionsMaintenanceDirectionSummary {
                    id: direction.id.trim().to_string(),
                    title: direction.title.trim().to_string(),
                    detail_doc_path,
                    detail_doc_exists,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(DirectionsMaintenanceSummary {
            directions,
            queue_idle_policy: catalog.queue_idle.policy,
            queue_idle_prompt_path,
            queue_idle_prompt_exists,
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
            .map(|path| {
                self.planning_workspace_port
                    .load_optional_planning_file(workspace_dir, path)
            })
            .transpose()?
            .flatten();

        Ok(QueueIdleReviewContext {
            policy: directions.queue_idle.policy,
            prompt_path,
            prompt_markdown,
        })
    }

    pub fn stage_editor_session(&self, workspace_dir: &str) -> Result<PlanningDraftEditorSession> {
        let mut workspace = self.load_complete_workspace(workspace_dir)?;
        let editable_paths =
            match toml::from_str::<DirectionCatalogDocument>(&workspace.directions_toml) {
                Ok(directions) => {
                    if let Some(prompt_path) =
                        trimmed_non_empty(directions.queue_idle.prompt_path.as_str())
                    {
                        if let Some(prompt_body) = self
                            .planning_workspace_port
                            .load_optional_planning_file(workspace_dir, prompt_path)?
                        {
                            workspace.extra_files.push(PlanningDraftFileRecord {
                                active_path: prompt_path.to_string(),
                                body: prompt_body,
                            });
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
        let detail_doc_path = trimmed_non_empty(selected_direction.detail_doc_path.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| default_direction_detail_doc_path(direction_id));
        let next_directions_toml = set_direction_detail_doc_path(
            &workspace.directions_toml,
            direction_id,
            &detail_doc_path,
        )?;
        workspace.directions_toml = next_directions_toml;
        let detail_doc_body = self
            .planning_workspace_port
            .load_optional_planning_file(workspace_dir, &detail_doc_path)?
            .unwrap_or_else(String::new);
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
        let prompt_path = trimmed_non_empty(directions.queue_idle.prompt_path.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string());
        let next_directions_toml =
            set_queue_idle_prompt_path(&workspace.directions_toml, &prompt_path)?;
        workspace.directions_toml = next_directions_toml;
        let prompt_body = self
            .planning_workspace_port
            .load_optional_planning_file(workspace_dir, &prompt_path)?
            .unwrap_or_else(|| DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string());
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
        Ok(ActiveDirectionsWorkspace {
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
        })
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
        DIRECTIONS_FILE_PATH, PlanningDirectionsService,
    };
    use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
    use crate::application::service::planning_bootstrap_service::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning_validation_service::PlanningValidationService;
    use crate::domain::planning::{QueueIdlePolicy, default_direction_detail_doc_path};

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
    }

    fn sample_service() -> PlanningDirectionsService {
        PlanningDirectionsService::new(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            PlanningValidationService::new(),
        )
    }

    #[test]
    fn load_summary_defaults_to_stop_policy_and_missing_detail_docs() {
        let workspace_dir = create_temp_workspace("planning-directions-summary");
        write_bootstrap_workspace(&workspace_dir);

        let summary = sample_service()
            .load_summary(&workspace_dir)
            .expect("directions summary should load");

        assert_eq!(summary.queue_idle_policy, QueueIdlePolicy::Stop);
        assert_eq!(summary.queue_idle_prompt_path, None);
        assert!(!summary.queue_idle_prompt_exists);
        assert_eq!(summary.directions.len(), 1);
        assert_eq!(summary.directions[0].detail_doc_path, None);
        assert!(!summary.directions[0].detail_doc_exists);

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
}
