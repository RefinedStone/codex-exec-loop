// Direction 삭제는 해당 direction에 매달린 task id를 모은 뒤 다른 task의 dependency/done references에서도 제거해야 한다.
// BTreeSet을 쓰면 결과가 안정된 순서라 테스트와 audit log가 흔들리지 않는다.
use std::collections::BTreeSet;

// Admin mutation은 operator 입력 검증 실패와 persistence 실패를 호출자에게 그대로 전달해야 하므로 anyhow Result를
// 사용하고, 도메인 규칙 위반은 bail!로 즉시 중단한다.
use anyhow::{Result, bail};

// Documents helper들은 direction request 정규화, default direction 보장, task cross-reference 정리를 담당한다.
// mutation service는 high-level orchestration만 맡는다.
use super::documents::{
    DEFAULT_DIRECTION_ID, direction_from_request, ensure_default_direction, normalized_required_id,
    remove_task_references,
};
// Admin facade service는 operator planning documents를 load/commit하는 boundary이고, request 타입은 CLI/admin API 쪽에서
// 들어온 direction mutation payload다.
use super::{
    PlanningAdminDirectionDeleteRequest, PlanningAdminDirectionMutationRequest,
    PlanningAdminFacadeService,
};
// Task authority document는 direction 삭제 시 같이 정리되는 source of truth다.
use crate::domain::planning::TaskAuthorityDocument;

// PlanningAdminDirectionMutationService는 direction catalog와 task authority를 함께 수정하는 admin use-case service다.
// facade를 통해 파일/DB boundary를 숨기고 문서 단위 mutation만 표현한다.
pub(super) struct PlanningAdminDirectionMutationService<'a> {
    // load/commit, validation context, workspace path를 가진 상위 admin facade다.
    facade: &'a PlanningAdminFacadeService,
}

#[derive(Debug, Clone)]
// Direction mutation은 upsert와 delete 두 명령만 허용한다. caller는 request shape를 command로 감싸고,
// service는 apply에서 공통 결과 타입으로 돌려준다.
pub(super) enum PlanningAdminDirectionMutationCommand {
    Upsert(PlanningAdminDirectionMutationRequest),
    Delete(PlanningAdminDirectionDeleteRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Mutation outcome은 admin API/CLI가 사용자에게 "무엇이 바뀌었는지"를 보고하는 DTO다. direction 자체의
// 생성/갱신/삭제뿐 아니라 cascade로 제거된 task 수까지 담는다.
pub(super) struct PlanningAdminDirectionMutationOutcome {
    // mutation 대상 direction id다.
    pub(super) direction_id: String,
    // upsert가 기존 direction을 교체했으면 true, 새로 추가했으면 false다.
    pub(super) updated: bool,
    // delete가 실제 direction을 제거했는지 여부다. default direction 보호 경로는 false다.
    pub(super) deleted: bool,
    // direction 삭제 때문에 task authority에서 제거된 task 개수다.
    pub(super) removed_task_count: usize,
    // direction 삭제 때문에 task authority에서 제거된 task id 목록이다.
    pub(super) removed_task_ids: BTreeSet<String>,
}

impl<'a> PlanningAdminDirectionMutationService<'a> {
    pub(super) fn new(facade: &'a PlanningAdminFacadeService) -> Self {
        // service는 facade를 소유하지 않고 borrow만 한다. 한 admin request 처리 흐름 안에서 다른 admin services와
        // 같은 facade boundary를 공유하기 위해서다.
        Self { facade }
    }

    pub(super) fn apply(
        &self,
        // caller가 upsert/delete 중 하나로 정규화한 admin mutation command다.
        command: PlanningAdminDirectionMutationCommand,
    ) -> Result<PlanningAdminDirectionMutationOutcome> {
        // apply는 command router다. 실제 문서 load/commit과 cascade cleanup은 각 private method에 나눠 두어
        // upsert와 delete의 정책 차이를 드러낸다.
        match command {
            PlanningAdminDirectionMutationCommand::Upsert(request) => self.upsert(request),
            PlanningAdminDirectionMutationCommand::Delete(request) => self.delete(request),
        }
    }

    fn upsert(
        &self,
        // direction 생성 또는 갱신 request다. documents helper가 id/title/detail 정규화를 수행한다.
        request: PlanningAdminDirectionMutationRequest,
    ) -> Result<PlanningAdminDirectionMutationOutcome> {
        // direction catalog와 task authority를 같은 document snapshot으로 읽는다. upsert는 direction catalog만
        // 바꾸지만 commit 단위는 operator planning documents 전체다.
        let mut documents = self.facade.load_operator_planning_documents()?;
        // request를 domain direction으로 변환하면서 기존 catalog와의 id normalization/validation을 적용한다.
        let direction = direction_from_request(request, &documents.directions)?;
        // outcome과 기존 direction lookup에 같은 canonical id를 사용해야 한다.
        let direction_id = direction.id.clone();
        // 같은 id가 이미 있으면 그 entry를 교체하고, 없으면 catalog 끝에 추가한다. trim 비교는 문서에 남은
        // 주변 공백 때문에 같은 direction이 중복되는 일을 막는다.
        let updated = if let Some(existing) = documents
            .directions
            .directions
            .iter_mut()
            .find(|existing| existing.id.trim() == direction_id)
        {
            *existing = direction;
            true
        } else {
            documents.directions.directions.push(direction);
            false
        };
        // mutation은 in-memory document를 모두 갱신한 뒤 한 번만 commit한다.
        self.facade.commit_operator_planning_documents(documents)?;

        // upsert는 direction 삭제나 task cascade를 수행하지 않으므로 deleted와 removed_task_count는 고정값이다.
        Ok(PlanningAdminDirectionMutationOutcome {
            direction_id,
            updated,
            deleted: false,
            removed_task_count: 0,
            removed_task_ids: BTreeSet::new(),
        })
    }

    fn delete(
        &self,
        // 삭제할 direction id request다.
        request: PlanningAdminDirectionDeleteRequest,
    ) -> Result<PlanningAdminDirectionMutationOutcome> {
        // blank id는 admin command contract 위반이므로 문서를 읽기 전에 실패시킨다.
        let direction_id = normalized_required_id(&request.id, "direction id")?.to_string();
        // 삭제는 direction catalog와 task authority를 함께 갱신해야 하므로 전체 planning documents를 로드한다.
        let mut documents = self.facade.load_operator_planning_documents()?;
        // default direction은 workspace bootstrap과 queue validation의 안전망이라 삭제하지 않는다. 대신 누락되어
        // 있다면 다시 보장하고 no-op outcome을 반환한다.
        if direction_id == DEFAULT_DIRECTION_ID {
            ensure_default_direction(&mut documents.directions)?;
            self.facade.commit_operator_planning_documents(documents)?;
            return Ok(PlanningAdminDirectionMutationOutcome {
                direction_id,
                updated: false,
                deleted: false,
                removed_task_count: 0,
                removed_task_ids: BTreeSet::new(),
            });
        }

        // retain 전후 개수를 비교해 실제로 삭제된 direction이 있었는지 확인한다.
        let original_count = documents.directions.directions.len();
        documents
            .directions
            .directions
            .retain(|direction| direction.id.trim() != direction_id);
        // 없는 direction 삭제를 성공으로 처리하면 operator가 오타를 놓치므로 명시적으로 실패한다.
        if documents.directions.directions.len() == original_count {
            bail!("direction `{direction_id}` was not found");
        }

        // direction을 삭제하면 그 direction에 속한 task도 authority에서 제거한다. 제거된 task id는 아래에서 다른
        // task들의 references를 정리하는 입력이 된다.
        let removed_task_ids =
            remove_tasks_for_direction(&mut documents.task_authority, &direction_id);
        // task 삭제 뒤 dangling dependency/done references를 남기지 않도록 task graph를 한 번 더 정리한다.
        remove_task_references(&mut documents.task_authority, &removed_task_ids);

        // outcome에 cascade 규모를 보고해 admin caller가 삭제 영향 범위를 알 수 있게 한다.
        let removed_task_count = removed_task_ids.len();
        // 삭제 후에도 direction catalog에는 항상 default direction이 남아야 한다.
        ensure_default_direction(&mut documents.directions)?;
        self.facade.commit_operator_planning_documents(documents)?;

        // 이 outcome은 direction 자체가 삭제되었고 task cascade가 몇 건 있었는지 보고한다.
        Ok(PlanningAdminDirectionMutationOutcome {
            direction_id,
            updated: false,
            deleted: true,
            removed_task_count,
            removed_task_ids,
        })
    }
}

fn remove_tasks_for_direction(
    // direction 삭제와 함께 수정될 task authority document다.
    task_authority: &mut TaskAuthorityDocument,
    // 삭제된 direction id다.
    direction_id: &str,
) -> BTreeSet<String> {
    // 제거된 task id를 모아야 이후 cross-reference cleanup이 정확한 대상만 지울 수 있다.
    let mut removed_task_ids = BTreeSet::new();
    // retain은 task authority 목록을 in-place로 줄인다. 삭제 대상 task를 발견하면 id를 기록하고 false를 반환해
    // 목록에서 제거한다.
    task_authority.tasks.retain(|task| {
        // 문서에 공백이 남아 있어도 같은 direction으로 취급하기 위해 trim 비교를 사용한다.
        if task.direction_id.trim() == direction_id {
            removed_task_ids.insert(task.id.trim().to_string());
            return false;
        }
        true
    });
    removed_task_ids
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
    use crate::application::service::planning::PlanningServices;
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
        PLANNING_FORMAT_VERSION, QueueIdleConfig, QueueIdlePolicy, TaskActor, TaskDefinition,
        TaskMutationProvenance, TaskStatus,
    };

    #[test]
    fn upsert_command_inserts_then_updates_direction_documents() {
        let fixture = TestAdminFixture::new("direction-mutation-upsert");
        let service = PlanningAdminDirectionMutationService::new(&fixture.facade);

        let inserted = service
            .apply(PlanningAdminDirectionMutationCommand::Upsert(
                direction_request("", "Release Planning", "active"),
            ))
            .expect("new direction should be inserted");
        let updated = service
            .apply(PlanningAdminDirectionMutationCommand::Upsert(
                direction_request("dir-release-planning", "Release Execution", "paused"),
            ))
            .expect("existing direction should be updated");
        let documents = fixture
            .facade
            .load_operator_planning_documents()
            .expect("documents should reload after upsert");
        let direction = documents
            .directions
            .directions
            .iter()
            .find(|direction| direction.id == "dir-release-planning")
            .expect("upserted direction should exist");

        assert_eq!(inserted.direction_id, "dir-release-planning");
        assert!(!inserted.updated);
        assert!(!inserted.deleted);
        assert_eq!(inserted.removed_task_count, 0);
        assert_eq!(updated.direction_id, "dir-release-planning");
        assert!(updated.updated);
        assert_eq!(direction.title, "Release Execution");
        assert_eq!(direction.state, DirectionState::Paused);
    }

    #[test]
    fn deleting_default_direction_is_a_noop_that_restores_anchor() {
        let fixture = TestAdminFixture::new("direction-mutation-default-delete");
        let mut documents = fixture
            .facade
            .load_operator_planning_documents()
            .expect("seeded documents should load");
        documents
            .directions
            .directions
            .retain(|direction| direction.id != DEFAULT_DIRECTION_ID);
        fixture
            .facade
            .commit_operator_planning_documents(documents)
            .expect("commit should restore default direction");

        let outcome = PlanningAdminDirectionMutationService::new(&fixture.facade)
            .apply(PlanningAdminDirectionMutationCommand::Delete(
                PlanningAdminDirectionDeleteRequest {
                    id: DEFAULT_DIRECTION_ID.to_string(),
                },
            ))
            .expect("default delete should be accepted as protected noop");
        let reloaded = fixture
            .facade
            .load_operator_planning_documents()
            .expect("documents should reload after protected delete");

        assert_eq!(outcome.direction_id, DEFAULT_DIRECTION_ID);
        assert!(!outcome.updated);
        assert!(!outcome.deleted);
        assert_eq!(outcome.removed_task_count, 0);
        assert!(outcome.removed_task_ids.is_empty());
        assert!(
            reloaded
                .directions
                .directions
                .iter()
                .any(|direction| direction.id == DEFAULT_DIRECTION_ID)
        );
    }

    #[test]
    fn delete_command_removes_direction_tasks_and_dangling_references() {
        let fixture = TestAdminFixture::new("direction-mutation-delete-cascade");
        let mut documents = fixture
            .facade
            .load_operator_planning_documents()
            .expect("seeded documents should load");
        documents.directions = direction_catalog(vec![
            direction(DEFAULT_DIRECTION_ID),
            direction("dir-keep"),
            direction("dir-remove"),
        ]);
        documents.task_authority = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![
                task(
                    "kept-task",
                    "dir-keep",
                    vec!["removed-a", "external-task"],
                    vec!["removed-b"],
                ),
                task("external-task", "dir-keep", Vec::new(), Vec::new()),
                task("removed-a", " dir-remove ", Vec::new(), Vec::new()),
                task("removed-b", "dir-remove", vec!["kept-task"], Vec::new()),
            ],
        };
        fixture
            .facade
            .commit_operator_planning_documents(documents)
            .expect("fixture documents should commit");

        let outcome = PlanningAdminDirectionMutationService::new(&fixture.facade)
            .apply(PlanningAdminDirectionMutationCommand::Delete(
                PlanningAdminDirectionDeleteRequest {
                    id: "dir-remove".to_string(),
                },
            ))
            .expect("non-default direction should be deleted");
        let reloaded = fixture
            .facade
            .load_operator_planning_documents()
            .expect("documents should reload after delete");
        let kept_task = reloaded
            .task_authority
            .tasks
            .iter()
            .find(|task| task.id == "kept-task")
            .expect("kept task should remain");

        assert!(outcome.deleted);
        assert_eq!(outcome.removed_task_count, 2);
        assert_eq!(
            outcome.removed_task_ids,
            BTreeSet::from(["removed-a".to_string(), "removed-b".to_string()])
        );
        assert!(
            !reloaded
                .directions
                .directions
                .iter()
                .any(|direction| direction.id == "dir-remove")
        );
        assert_eq!(reloaded.task_authority.tasks.len(), 2);
        assert_eq!(kept_task.depends_on, vec!["external-task".to_string()]);
        assert!(kept_task.blocked_by.is_empty());
    }

    #[test]
    fn delete_command_rejects_blank_and_missing_direction_ids() {
        let fixture = TestAdminFixture::new("direction-mutation-delete-errors");
        let service = PlanningAdminDirectionMutationService::new(&fixture.facade);

        let blank = service
            .apply(PlanningAdminDirectionMutationCommand::Delete(
                PlanningAdminDirectionDeleteRequest {
                    id: "  ".to_string(),
                },
            ))
            .expect_err("blank id should fail before document loading");
        let missing = service
            .apply(PlanningAdminDirectionMutationCommand::Delete(
                PlanningAdminDirectionDeleteRequest {
                    id: "missing-direction".to_string(),
                },
            ))
            .expect_err("missing id should fail explicitly");

        assert_eq!(blank.to_string(), "direction id is required");
        assert_eq!(
            missing.to_string(),
            "direction `missing-direction` was not found"
        );
    }

    fn direction_request(
        id: &str,
        title: &str,
        state: &str,
    ) -> PlanningAdminDirectionMutationRequest {
        PlanningAdminDirectionMutationRequest {
            id: id.to_string(),
            title: title.to_string(),
            summary: format!("Summary for {title}"),
            success_criteria_text: "done".to_string(),
            scope_hints_text: "scope".to_string(),
            detail_doc_path: String::new(),
            state: state.to_string(),
        }
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

    fn direction(id: &str) -> DirectionDefinition {
        DirectionDefinition {
            id: id.to_string(),
            title: format!("Direction {id}"),
            summary: format!("Summary for {id}"),
            success_criteria: vec!["done".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state: DirectionState::Active,
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
