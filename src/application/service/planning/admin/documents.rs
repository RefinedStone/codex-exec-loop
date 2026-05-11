use std::collections::BTreeSet;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};

use super::{PlanningAdminDirectionMutationRequest, PlanningAdminFacadeService};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit,
    PlanningTaskAuthorityCommitResult,
};
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::shared::authority_seed::PlanningAuthoritySeedService;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PlanningWorkspaceFiles,
    TaskAuthorityDocument,
};

/*
 * 이 모듈은 admin이 편집한 planning authority를 실제 저장소에 반영하는 write boundary다. admin form과 draft
 * file은 operator가 다루기 쉬운 text 표면이지만, committed authority는 DB-backed direction/task snapshot과
 * workspace result markdown으로 나뉘어 있다. 여기서는 그 세 저장소를 하나의 편집 문서처럼 읽고, commit 때는
 * revision 순서와 validation 순서를 지켜 authority graph가 중간 상태로 남지 않게 한다.
 */
pub(super) const DEFAULT_DIRECTION_ID: &str = "general-workstream";
const GENERATED_DIRECTION_ID_PREFIX: &str = "dir";

// default direction은 bootstrap artifact에서 파생한다. admin 화면은 자주 reload되므로 parsed definition을 캐시해
// 매번 bootstrap bundle을 다시 만들지 않게 한다.
static DEFAULT_DIRECTION_DEFINITION: OnceLock<Result<DirectionDefinition, String>> =
    OnceLock::new();

impl PlanningAdminFacadeService {
    pub(super) fn ensure_default_authority(&self) -> Result<()> {
        // admin page가 workspace에서 처음 열리는 planning entrypoint일 수 있다. 그래서 runtime startup과 같은
        // authority seed 경로를 호출해 directions/task/result_output baseline을 맞춘 뒤 이후 admin 작업을 진행한다.
        PlanningAuthoritySeedService::new(
            self.planning_workspace_port.clone(),
            self.planning_task_repository_port.clone(),
            self.planning_validation_service.clone(),
            self.priority_queue_service.clone(),
        )
        .ensure_default_authority(self.workspace_dir.as_str())
        .map(|_| ())
    }
    pub(super) fn load_operator_planning_documents(
        &self,
    ) -> Result<PlanningOperatorPlanningDocuments> {
        // load는 direction/task repository snapshot을 authority로 삼고, result_output만 workspace file system에서
        // 읽는다. observed revision은 operator가 읽은 DB snapshot의 버전이므로 commit 때 optimistic concurrency
        // guard로 그대로 전달한다.
        self.ensure_default_authority()?;
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(self.workspace_dir.as_str())?;
        let result_output_markdown = workspace.result_output_markdown.ok_or_else(|| {
            anyhow!("default planning authority seed did not provide result output")
        })?;
        let direction_authority_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(self.workspace_dir.as_str())?
            .ok_or_else(|| {
                anyhow!("default planning authority seed did not provide direction authority")
            })?;
        let task_authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(self.workspace_dir.as_str())?
            .ok_or_else(|| {
                anyhow!("default planning authority seed did not provide task authority")
            })?;
        Ok(PlanningOperatorPlanningDocuments {
            directions: direction_authority_snapshot.directions,
            task_authority: task_authority_snapshot.task_authority,
            result_output_markdown,
            observed_planning_revision: Some(task_authority_snapshot.planning_revision),
        })
    }
    pub(super) fn commit_operator_planning_documents(
        &self,
        mut documents: PlanningOperatorPlanningDocuments,
    ) -> Result<()> {
        // admin edit는 direction을 먼저 지우고 child task 정리를 나중에 할 수 있다. commit boundary에서는 default
        // direction 복구와 unresolved direction task 제거를 먼저 수행한 뒤, 실제 persist할 세 문서 조합을 그대로
        // validation에 넣는다.
        ensure_default_direction(&mut documents.directions)?;
        remove_tasks_with_unresolved_directions(&mut documents);

        let task_authority_json = serde_json::to_string_pretty(&documents.task_authority)?;
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions: &documents.directions,
                    task_authority_json: &task_authority_json,
                    result_output_markdown: &documents.result_output_markdown,
                });
        if !validation_result.report.is_valid() {
            bail!(
                "planning mutation failed validation: {}",
                validation_result
                    .report
                    .issues
                    .iter()
                    .map(|issue| issue.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        let queue_projection = self
            .priority_queue_service
            .build_projection(&documents.directions, &documents.task_authority)
            .context("failed to rebuild planning queue")?;
        let task_observed_revision = match self
            .planning_task_repository_port
            .commit_direction_authority_snapshot(
                self.workspace_dir.as_str(),
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: documents.observed_planning_revision,
                    directions: &documents.directions,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { planning_revision } => planning_revision,
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision,
                current_planning_revision,
            } => {
                bail!(
                    "planning db changed while editing directions (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                );
            }
        };
        // direction authority와 task authority는 같은 planning DB revision을 공유한다. direction snapshot commit이
        // 성공하면 task snapshot은 그 새 revision을 observed 값으로 삼아야 하며, 그래야 두 snapshot이 같은
        // logical authority update 안에서 순서대로 갱신된다.
        match self
            .planning_task_repository_port
            .commit_task_authority_snapshot(
                self.workspace_dir.as_str(),
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: Some(task_observed_revision),
                    task_authority: &documents.task_authority,
                    queue_projection: &queue_projection,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { .. } => {}
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision,
                current_planning_revision,
            } => {
                bail!(
                    "planning db changed while editing (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                );
            }
        }
        // result_output은 아직 file-backed authority라 DB conflict detection에 참여하지 않는다. 그래서 DB authority와
        // queue projection이 mutation을 받아들인 뒤 마지막에 파일을 교체해, repository 쪽 권위 상태가 거절된 변경을
        // workspace markdown이 먼저 반영하는 일을 피한다.
        self.planning_workspace_port
            .replace_planning_workspace_file(
                self.workspace_dir.as_str(),
                RESULT_OUTPUT_FILE_PATH,
                Some(&documents.result_output_markdown),
            )?;
        Ok(())
    }
}

/*
 * loaded admin edit session은 planning authority 저장소 전체를 한 값으로 묶은 내부 문서 모델이다. revision은
 * DB snapshot만 추적한다. result_output은 repository conflict detection 대상이 아니므로 commit phase에서 DB
 * snapshot 성공 뒤 따로 쓰인다.
 */
#[derive(Debug, Clone)]
pub(super) struct PlanningOperatorPlanningDocuments {
    pub(super) directions: DirectionCatalogDocument,
    pub(super) task_authority: TaskAuthorityDocument,
    pub(super) result_output_markdown: String,
    observed_planning_revision: Option<i64>,
}

pub(super) fn direction_from_request(
    request: PlanningAdminDirectionMutationRequest,
    directions: &DirectionCatalogDocument,
) -> Result<DirectionDefinition> {
    // direction form은 기존 id를 업데이트하거나 title에서 stable id를 생성할 수 있다. success criteria는
    // queue-idle review가 완료 판정의 authority로 쓰는 필드라서 blank direction을 허용하지 않는다.
    let title = normalized_required_text(&request.title, "direction title")?;
    let id = if request.id.trim().is_empty() {
        generated_unique_id(
            GENERATED_DIRECTION_ID_PREFIX,
            title,
            directions
                .directions
                .iter()
                .map(|direction| direction.id.trim()),
        )
    } else {
        normalized_required_id(&request.id, "direction id")?.to_string()
    };
    let success_criteria = split_lines(&request.success_criteria_text);
    if success_criteria.is_empty() {
        bail!("direction `{id}` requires at least one success criterion");
    }
    Ok(DirectionDefinition {
        id,
        title: title.to_string(),
        summary: non_empty_or(&request.summary, title),
        success_criteria,
        scope_hints: split_lines(&request.scope_hints_text),
        detail_doc_path: request.detail_doc_path.trim().to_string(),
        state: parse_direction_state(&request.state)?,
    })
}

pub(super) fn ensure_default_direction(directions: &mut DirectionCatalogDocument) -> Result<()> {
    // default direction은 blank task form과 오래된 planning data를 위한 compatibility anchor다. operator가 방향을
    // 재구성하다가 이 anchor를 제거해도 commit 직전에 복구해 task creation fallback이 사라지지 않게 한다.
    if directions
        .directions
        .iter()
        .any(|direction| direction.id.trim() == DEFAULT_DIRECTION_ID)
    {
        return Ok(());
    }
    directions.directions.push(default_direction_definition()?);
    Ok(())
}

fn default_direction_definition() -> Result<DirectionDefinition> {
    DEFAULT_DIRECTION_DEFINITION
        .get_or_init(build_default_direction_definition)
        .clone()
        .map_err(|message| anyhow!(message))
}

fn build_default_direction_definition() -> Result<DirectionDefinition, String> {
    // default definition은 새 workspace 생성과 같은 bootstrap path에서 가져온다. admin repair용 기본값과 first-run
    // initialization 기본값이 서로 갈라지면 나중에 validation/queue behavior가 workspace 생성 시점에 따라 달라진다.
    let artifacts =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
    artifacts
        .directions
        .directions
        .into_iter()
        .find(|direction| direction.id.trim() == DEFAULT_DIRECTION_ID)
        .ok_or_else(|| format!("bootstrap default direction `{DEFAULT_DIRECTION_ID}` is missing"))
}

fn remove_tasks_with_unresolved_directions(documents: &mut PlanningOperatorPlanningDocuments) {
    // direction이 사라지면 그 direction의 task는 더 이상 queue에 들어갈 수 없다. commit boundary에서 task를
    // 제거하고, 제거된 task를 가리키던 dependency/blocker edge도 같이 정리해 dangling graph를 남기지 않는다.
    let direction_ids = documents
        .directions
        .directions
        .iter()
        .map(|direction| direction.id.trim())
        .collect::<BTreeSet<_>>();
    let mut removed_task_ids = BTreeSet::new();
    documents.task_authority.tasks.retain(|task| {
        let should_keep = direction_ids.contains(task.direction_id.trim());
        if !should_keep {
            removed_task_ids.insert(task.id.trim().to_string());
        }
        should_keep
    });
    if removed_task_ids.is_empty() {
        return;
    }
    remove_task_references(&mut documents.task_authority, &removed_task_ids);
}

fn parse_direction_state(raw: &str) -> Result<DirectionState> {
    // state blank는 active로 처리한다. 간단한 creation form이 title/criteria만 제출해도 새 direction이 바로 queue
    // 후보가 되도록 하되, 명시 label은 domain enum으로만 변환한다.
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "active" => Ok(DirectionState::Active),
        "paused" => Ok(DirectionState::Paused),
        "done" => Ok(DirectionState::Done),
        other => bail!("unknown direction state `{other}`"),
    }
}

pub(super) fn default_direction_id(directions: &DirectionCatalogDocument) -> Result<&str> {
    // task creation fallback은 compatibility default를 최우선으로 고른다. 없으면 active direction, 그래도 없으면
    // 첫 direction id를 사용해 operator가 direction authority를 재구성하는 중에도 deterministic target을 제공한다.
    if let Some(direction) = directions
        .directions
        .iter()
        .find(|direction| direction.id.trim() == DEFAULT_DIRECTION_ID)
    {
        return Ok(direction.id.trim());
    }
    directions
        .directions
        .iter()
        .find(|direction| direction.state == DirectionState::Active)
        .or_else(|| directions.directions.first())
        .map(|direction| direction.id.trim())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow!("at least one direction is required"))
}

pub(super) fn normalized_required_id<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    // id는 authority graph reference와 route/generated path에 동시에 쓰인다. whitespace나 path separator를
    // 허용하면 graph matching과 URL parameter 해석이 서로 다른 문자열 정규화에 의존하게 된다.
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    if value.contains(char::is_whitespace) || value.contains('/') || value.contains('\\') {
        bail!("{label} `{value}` must not contain whitespace or path separators");
    }
    Ok(value)
}

fn normalized_required_text<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value)
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn generated_unique_id<'a>(
    prefix: &str,
    title: &str,
    existing_ids: impl IntoIterator<Item = &'a str>,
) -> String {
    // generated id는 같은 title에 대해 deterministic해야 operator가 예측할 수 있다. 동시에 현재 authority 문서
    // 안에서는 collision을 피해야 하므로 base slug 뒤에 numeric suffix를 붙이는 단순한 규칙을 쓴다.
    let existing = existing_ids.into_iter().collect::<BTreeSet<_>>();
    let slug = slugify_title(title);
    let base = format!("{prefix}-{slug}");
    if !existing.contains(base.as_str()) {
        return base;
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!("numeric suffix search should eventually find an unused id")
}

fn slugify_title(title: &str) -> String {
    // Unicode alphanumeric을 유지해 비영어 direction title도 generated id 안에서 의미를 보존한다. 모든 non-ASCII를
    // 버리면 한국어 title이 item-2 같은 opaque id로 바뀌어 admin 화면에서 추적하기 어려워진다.
    let mut slug = String::new();
    let mut previous_dash = false;
    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash && !slug.is_empty() {
            slug.push('-');
            previous_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "item".to_string()
    } else {
        slug
    }
}

fn split_lines(raw: &str) -> Vec<String> {
    // admin form은 list field를 textarea로 편집한다. blank line은 사람이 읽기 쉽게 넣은 presentation noise이므로
    // authority entry로 저장하지 않는다.
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn remove_task_references(
    task_authority: &mut TaskAuthorityDocument,
    removed_task_ids: &BTreeSet<String>,
) {
    // reference cleanup은 양쪽을 trim해서 비교한다. legacy authority file에 공백이 섞여 있어도 제거된 task를
    // 가리키는 dependency가 살아남지 않아야 한다.
    for task in &mut task_authority.tasks {
        task.depends_on
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
        task.blocked_by
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
    use crate::application::service::planning::PlanningServices;
    use crate::domain::planning::{
        OriginSessionKind, PLANNING_FORMAT_VERSION, QueueIdleConfig, QueueIdlePolicy, TaskActor,
        TaskDefinition, TaskMutationProvenance, TaskStatus,
    };
    use std::fs;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn slugify_title_preserves_unicode_alphanumerics() {
        // generated id는 비영어 operator title에서도 읽을 수 있는 의미를 유지해야 한다.
        assert_eq!(slugify_title("한글 작업 1"), "한글-작업-1");
    }

    #[test]
    fn generated_unique_id_keeps_unicode_title_identity() {
        // collision suffix는 readable slug를 대체하지 않고 뒤에 붙어야 title identity가 유지된다.
        let existing = ["task-한글-작업", "task-한글-작업-2"];

        assert_eq!(
            generated_unique_id("task", "한글 작업", existing),
            "task-한글-작업-3"
        );
    }

    #[test]
    fn direction_from_request_generates_id_and_normalizes_text_fields() {
        let directions =
            direction_catalog(vec![direction("dir-build-release", DirectionState::Active)]);

        let direction = direction_from_request(
            PlanningAdminDirectionMutationRequest {
                id: " ".to_string(),
                title: " Build Release! ".to_string(),
                summary: " ".to_string(),
                success_criteria_text: "\nship it\n\nverify it\n".to_string(),
                scope_hints_text: " backend \n frontend \n".to_string(),
                detail_doc_path: " docs/release.md ".to_string(),
                state: " ".to_string(),
            },
            &directions,
        )
        .expect("valid direction form should normalize into a definition");

        assert_eq!(direction.id, "dir-build-release-2");
        assert_eq!(direction.title, "Build Release!");
        assert_eq!(direction.summary, "Build Release!");
        assert_eq!(direction.success_criteria, vec!["ship it", "verify it"]);
        assert_eq!(direction.scope_hints, vec!["backend", "frontend"]);
        assert_eq!(direction.detail_doc_path, "docs/release.md");
        assert_eq!(direction.state, DirectionState::Active);
    }

    #[test]
    fn direction_from_request_rejects_invalid_id_state_and_empty_success_criteria() {
        let directions = direction_catalog(Vec::new());

        let bad_id = direction_from_request(
            PlanningAdminDirectionMutationRequest {
                id: "bad id".to_string(),
                title: "Title".to_string(),
                summary: String::new(),
                success_criteria_text: "done".to_string(),
                scope_hints_text: String::new(),
                detail_doc_path: String::new(),
                state: "active".to_string(),
            },
            &directions,
        )
        .expect_err("ids with whitespace should be rejected");
        assert_eq!(
            bad_id.to_string(),
            "direction id `bad id` must not contain whitespace or path separators"
        );

        let empty_success = direction_from_request(
            PlanningAdminDirectionMutationRequest {
                id: String::new(),
                title: "Title".to_string(),
                summary: String::new(),
                success_criteria_text: "\n \n".to_string(),
                scope_hints_text: String::new(),
                detail_doc_path: String::new(),
                state: "active".to_string(),
            },
            &directions,
        )
        .expect_err("success criteria is required");
        assert_eq!(
            empty_success.to_string(),
            "direction `dir-title` requires at least one success criterion"
        );

        let bad_state = direction_from_request(
            PlanningAdminDirectionMutationRequest {
                id: String::new(),
                title: "Title".to_string(),
                summary: String::new(),
                success_criteria_text: "done".to_string(),
                scope_hints_text: String::new(),
                detail_doc_path: String::new(),
                state: "waiting".to_string(),
            },
            &directions,
        )
        .expect_err("unknown state should be rejected");
        assert_eq!(bad_state.to_string(), "unknown direction state `waiting`");
    }

    #[test]
    fn default_direction_selection_prefers_default_then_active_then_first() {
        let catalog = direction_catalog(vec![
            direction("paused-direction", DirectionState::Paused),
            direction("active-direction", DirectionState::Active),
            direction(DEFAULT_DIRECTION_ID, DirectionState::Paused),
        ]);
        assert_eq!(
            default_direction_id(&catalog).unwrap(),
            DEFAULT_DIRECTION_ID
        );

        let catalog = direction_catalog(vec![
            direction("paused-direction", DirectionState::Paused),
            direction("active-direction", DirectionState::Active),
        ]);
        assert_eq!(default_direction_id(&catalog).unwrap(), "active-direction");

        let catalog =
            direction_catalog(vec![direction("paused-direction", DirectionState::Paused)]);
        assert_eq!(default_direction_id(&catalog).unwrap(), "paused-direction");

        let empty = direction_catalog(Vec::new());
        assert_eq!(
            default_direction_id(&empty).unwrap_err().to_string(),
            "at least one direction is required"
        );
    }

    #[test]
    fn ensure_default_direction_restores_bootstrap_anchor_once() {
        let mut catalog = direction_catalog(vec![direction("custom", DirectionState::Active)]);

        ensure_default_direction(&mut catalog).expect("default direction should be restored");
        ensure_default_direction(&mut catalog).expect("second restore should be idempotent");

        let default_count = catalog
            .directions
            .iter()
            .filter(|direction| direction.id == DEFAULT_DIRECTION_ID)
            .count();
        assert_eq!(default_count, 1);
        let restored = catalog
            .directions
            .iter()
            .find(|direction| direction.id == DEFAULT_DIRECTION_ID)
            .expect("default direction should be present");
        assert_eq!(restored.state, DirectionState::Active);
    }

    #[test]
    fn unresolved_direction_cleanup_prunes_tasks_and_references() {
        let mut documents = PlanningOperatorPlanningDocuments {
            directions: direction_catalog(vec![direction("kept", DirectionState::Active)]),
            task_authority: TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![
                    task(
                        "kept-task",
                        "kept",
                        vec!["removed-task"],
                        vec!["removed-task"],
                    ),
                    task("removed-task", "missing", Vec::new(), Vec::new()),
                ],
            },
            result_output_markdown: "# Result Output\n\nKeep reporting.".to_string(),
            observed_planning_revision: Some(1),
        };

        remove_tasks_with_unresolved_directions(&mut documents);

        assert_eq!(documents.task_authority.tasks.len(), 1);
        assert_eq!(documents.task_authority.tasks[0].id, "kept-task");
        assert!(documents.task_authority.tasks[0].depends_on.is_empty());
        assert!(documents.task_authority.tasks[0].blocked_by.is_empty());
    }

    #[test]
    fn load_and_commit_operator_documents_round_trips_seeded_authority() {
        let fixture = TestAdminFixture::new("admin-documents-round-trip");
        let mut documents = fixture
            .facade
            .load_operator_planning_documents()
            .expect("seeded operator documents should load");
        documents.result_output_markdown = "# Result Output\n\nUpdated admin copy.".to_string();
        documents
            .directions
            .directions
            .retain(|direction| direction.id != DEFAULT_DIRECTION_ID);

        fixture
            .facade
            .commit_operator_planning_documents(documents)
            .expect("valid operator documents should commit");

        let reloaded = fixture
            .facade
            .load_operator_planning_documents()
            .expect("committed operator documents should reload");
        assert_eq!(
            reloaded.result_output_markdown,
            "# Result Output\n\nUpdated admin copy."
        );
        assert!(
            reloaded
                .directions
                .directions
                .iter()
                .any(|direction| direction.id == DEFAULT_DIRECTION_ID)
        );
    }

    #[test]
    fn invalid_operator_documents_fail_before_overwriting_result_output() {
        let fixture = TestAdminFixture::new("admin-documents-invalid");
        let mut documents = fixture
            .facade
            .load_operator_planning_documents()
            .expect("seeded operator documents should load");
        let original_result_output = documents.result_output_markdown.clone();
        documents.result_output_markdown = "no heading".to_string();

        let error = fixture
            .facade
            .commit_operator_planning_documents(documents)
            .expect_err("invalid result output should fail validation");

        assert!(
            error
                .to_string()
                .contains("planning mutation failed validation")
        );
        let reloaded = fixture
            .facade
            .load_operator_planning_documents()
            .expect("documents should still load after failed commit");
        assert_eq!(reloaded.result_output_markdown, original_result_output);
    }

    fn direction_catalog(directions: Vec<DirectionDefinition>) -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::Stop,
                prompt_path: String::new(),
            },
            directions,
        }
    }

    fn direction(id: &str, state: DirectionState) -> DirectionDefinition {
        DirectionDefinition {
            id: id.to_string(),
            title: format!("Direction {id}"),
            summary: format!("Summary for {id}"),
            success_criteria: vec!["done".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state,
        }
    }

    fn task(
        id: &str,
        direction_id: &str,
        depends_on: Vec<&str>,
        blocked_by: Vec<&str>,
    ) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: direction_id.to_string(),
            direction_relation_note: "relates to the direction".to_string(),
            title: format!("Task {id}"),
            description: "Do the task".to_string(),
            status: TaskStatus::Ready,
            base_priority: 10,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: depends_on.into_iter().map(str::to_string).collect(),
            blocked_by: blocked_by.into_iter().map(str::to_string).collect(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: TaskMutationProvenance::new(OriginSessionKind::System),
            updated_at: "2026-05-12T00:00:00Z".to_string(),
        }
    }

    struct TestAdminFixture {
        _workspace: TempPlanningWorkspace,
        facade: PlanningAdminFacadeService,
    }

    impl TestAdminFixture {
        fn new(prefix: &str) -> Self {
            let workspace = TempPlanningWorkspace::new(prefix);
            let workspace_port: Arc<dyn PlanningWorkspacePort> =
                Arc::new(FilesystemPlanningWorkspaceAdapter::new());
            let sqlite = Arc::new(SqlitePlanningAuthorityAdapter::new());
            let authority_port: Arc<dyn PlanningAuthorityPort> = sqlite.clone();
            let task_repository_port: Arc<dyn PlanningTaskRepositoryPort> = sqlite.clone();
            let planning = PlanningServices::from_ports(
                workspace_port.clone(),
                authority_port.clone(),
                task_repository_port.clone(),
                Arc::new(NoopPlanningWorkerPort),
            );
            let facade = PlanningAdminFacadeService::from_planning_with_authority(
                workspace.path.clone(),
                planning,
                workspace_port,
                authority_port,
                task_repository_port,
            );
            Self {
                _workspace: workspace,
                facade,
            }
        }
    }

    struct TempPlanningWorkspace {
        path: String,
    }

    impl TempPlanningWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::create_dir_all(&path).expect("temp planning workspace should be created");
            Self {
                path: path.display().to_string(),
            }
        }
    }

    impl Drop for TempPlanningWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
