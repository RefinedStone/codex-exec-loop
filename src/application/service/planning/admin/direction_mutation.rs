// 학습 주석: direction 삭제는 해당 direction에 매달린 task id를 모은 뒤 다른 task의 dependency/done
// references에서도 제거해야 합니다. BTreeSet을 쓰면 결과가 안정된 순서라 테스트와 audit log가 흔들리지 않습니다.
use std::collections::BTreeSet;

// 학습 주석: admin mutation은 operator 입력 검증 실패와 persistence 실패를 호출자에게 그대로 전달해야 하므로
// anyhow Result를 사용하고, 도메인 규칙 위반은 bail!로 즉시 중단합니다.
use anyhow::{Result, bail};

// 학습 주석: documents helper들은 direction request 정규화, default direction 보장, task cross-reference
// 정리를 담당합니다. mutation service는 high-level orchestration만 맡습니다.
use super::documents::{
    DEFAULT_DIRECTION_ID, direction_from_request, ensure_default_direction, normalized_required_id,
    remove_task_references,
};
// 학습 주석: admin facade service는 operator planning documents를 load/commit하는 boundary이고,
// request 타입은 CLI/admin API 쪽에서 들어온 direction mutation payload입니다.
use super::{
    PlanningAdminDirectionDeleteRequest, PlanningAdminDirectionMutationRequest,
    PlanningAdminFacadeService,
};
// 학습 주석: task authority document는 direction 삭제 시 같이 정리되는 source of truth입니다.
use crate::domain::planning::TaskAuthorityDocument;

// 학습 주석: PlanningAdminDirectionMutationService는 direction catalog와 task authority를 함께 수정하는
// admin use-case service입니다. facade를 통해 파일/DB boundary를 숨기고 문서 단위 mutation만 표현합니다.
pub(super) struct PlanningAdminDirectionMutationService<'a> {
    // 학습 주석: load/commit, validation context, workspace path를 가진 상위 admin facade입니다.
    facade: &'a PlanningAdminFacadeService,
}

#[derive(Debug, Clone)]
// 학습 주석: direction mutation은 upsert와 delete 두 명령만 허용합니다. caller는 request shape를
// command로 감싸고, service는 apply에서 공통 결과 타입으로 돌려줍니다.
pub(super) enum PlanningAdminDirectionMutationCommand {
    Upsert(PlanningAdminDirectionMutationRequest),
    Delete(PlanningAdminDirectionDeleteRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: mutation outcome은 admin API/CLI가 사용자에게 "무엇이 바뀌었는지"를 보고하는 DTO입니다.
// direction 자체의 생성/갱신/삭제뿐 아니라 cascade로 제거된 task 수까지 담습니다.
pub(super) struct PlanningAdminDirectionMutationOutcome {
    // 학습 주석: mutation 대상 direction id입니다.
    pub(super) direction_id: String,
    // 학습 주석: upsert가 기존 direction을 교체했으면 true, 새로 추가했으면 false입니다.
    pub(super) updated: bool,
    // 학습 주석: delete가 실제 direction을 제거했는지 여부입니다. default direction 보호 경로는 false입니다.
    pub(super) deleted: bool,
    // 학습 주석: direction 삭제 때문에 task authority에서 제거된 task 개수입니다.
    pub(super) removed_task_count: usize,
}

impl<'a> PlanningAdminDirectionMutationService<'a> {
    pub(super) fn new(facade: &'a PlanningAdminFacadeService) -> Self {
        // 학습 주석: service는 facade를 소유하지 않고 borrow만 합니다. 한 admin request 처리 흐름 안에서
        // 다른 admin services와 같은 facade boundary를 공유하기 위해서입니다.
        Self { facade }
    }

    pub(super) fn apply(
        &self,
        // 학습 주석: caller가 upsert/delete 중 하나로 정규화한 admin mutation command입니다.
        command: PlanningAdminDirectionMutationCommand,
    ) -> Result<PlanningAdminDirectionMutationOutcome> {
        // 학습 주석: apply는 command router입니다. 실제 문서 load/commit과 cascade cleanup은
        // 각 private method에 나눠 두어 upsert와 delete의 정책 차이를 드러냅니다.
        match command {
            PlanningAdminDirectionMutationCommand::Upsert(request) => self.upsert(request),
            PlanningAdminDirectionMutationCommand::Delete(request) => self.delete(request),
        }
    }

    fn upsert(
        &self,
        // 학습 주석: direction 생성 또는 갱신 request입니다. documents helper가 id/title/detail 정규화를 수행합니다.
        request: PlanningAdminDirectionMutationRequest,
    ) -> Result<PlanningAdminDirectionMutationOutcome> {
        // 학습 주석: direction catalog와 task authority를 같은 document snapshot으로 읽습니다.
        // upsert는 direction catalog만 바꾸지만 commit 단위는 operator planning documents 전체입니다.
        let mut documents = self.facade.load_operator_planning_documents()?;
        // 학습 주석: request를 domain direction으로 변환하면서 기존 catalog와의 id normalization/validation을 적용합니다.
        let direction = direction_from_request(request, &documents.directions)?;
        // 학습 주석: outcome과 기존 direction lookup에 같은 canonical id를 사용해야 합니다.
        let direction_id = direction.id.clone();
        // 학습 주석: 같은 id가 이미 있으면 그 entry를 교체하고, 없으면 catalog 끝에 추가합니다.
        // trim 비교는 문서에 남은 주변 공백 때문에 같은 direction이 중복되는 일을 막습니다.
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
        // 학습 주석: mutation은 in-memory document를 모두 갱신한 뒤 한 번만 commit합니다.
        self.facade.commit_operator_planning_documents(documents)?;

        // 학습 주석: upsert는 direction 삭제나 task cascade를 수행하지 않으므로 deleted와 removed_task_count는 고정값입니다.
        Ok(PlanningAdminDirectionMutationOutcome {
            direction_id,
            updated,
            deleted: false,
            removed_task_count: 0,
        })
    }

    fn delete(
        &self,
        // 학습 주석: 삭제할 direction id request입니다.
        request: PlanningAdminDirectionDeleteRequest,
    ) -> Result<PlanningAdminDirectionMutationOutcome> {
        // 학습 주석: blank id는 admin command contract 위반이므로 문서를 읽기 전에 실패시킵니다.
        let direction_id = normalized_required_id(&request.id, "direction id")?.to_string();
        // 학습 주석: 삭제는 direction catalog와 task authority를 함께 갱신해야 하므로 전체 planning documents를 로드합니다.
        let mut documents = self.facade.load_operator_planning_documents()?;
        // 학습 주석: default direction은 workspace bootstrap과 queue validation의 안전망이라 삭제하지 않습니다.
        // 대신 누락되어 있다면 다시 보장하고 no-op outcome을 반환합니다.
        if direction_id == DEFAULT_DIRECTION_ID {
            ensure_default_direction(&mut documents.directions)?;
            self.facade.commit_operator_planning_documents(documents)?;
            return Ok(PlanningAdminDirectionMutationOutcome {
                direction_id,
                updated: false,
                deleted: false,
                removed_task_count: 0,
            });
        }

        // 학습 주석: retain 전후 개수를 비교해 실제로 삭제된 direction이 있었는지 확인합니다.
        let original_count = documents.directions.directions.len();
        documents
            .directions
            .directions
            .retain(|direction| direction.id.trim() != direction_id);
        // 학습 주석: 없는 direction 삭제를 성공으로 처리하면 operator가 오타를 놓치므로 명시적으로 실패합니다.
        if documents.directions.directions.len() == original_count {
            bail!("direction `{direction_id}` was not found");
        }

        // 학습 주석: direction을 삭제하면 그 direction에 속한 task도 authority에서 제거합니다.
        // 제거된 task id는 아래에서 다른 task들의 references를 정리하는 입력이 됩니다.
        let removed_task_ids =
            remove_tasks_for_direction(&mut documents.task_authority, &direction_id);
        // 학습 주석: task 삭제 뒤 dangling dependency/done references를 남기지 않도록 task graph를 한 번 더 정리합니다.
        remove_task_references(&mut documents.task_authority, &removed_task_ids);

        // 학습 주석: outcome에 cascade 규모를 보고해 admin caller가 삭제 영향 범위를 알 수 있게 합니다.
        let removed_task_count = removed_task_ids.len();
        // 학습 주석: 삭제 후에도 direction catalog에는 항상 default direction이 남아야 합니다.
        ensure_default_direction(&mut documents.directions)?;
        self.facade.commit_operator_planning_documents(documents)?;

        // 학습 주석: 이 outcome은 direction 자체가 삭제되었고 task cascade가 몇 건 있었는지 보고합니다.
        Ok(PlanningAdminDirectionMutationOutcome {
            direction_id,
            updated: false,
            deleted: true,
            removed_task_count,
        })
    }
}

fn remove_tasks_for_direction(
    // 학습 주석: direction 삭제와 함께 수정될 task authority document입니다.
    task_authority: &mut TaskAuthorityDocument,
    // 학습 주석: 삭제된 direction id입니다.
    direction_id: &str,
) -> BTreeSet<String> {
    // 학습 주석: 제거된 task id를 모아야 이후 cross-reference cleanup이 정확한 대상만 지울 수 있습니다.
    let mut removed_task_ids = BTreeSet::new();
    // 학습 주석: retain은 task authority 목록을 in-place로 줄입니다. 삭제 대상 task를 발견하면
    // id를 기록하고 false를 반환해 목록에서 제거합니다.
    task_authority.tasks.retain(|task| {
        // 학습 주석: 문서에 공백이 남아 있어도 같은 direction으로 취급하기 위해 trim 비교를 사용합니다.
        if task.direction_id.trim() == direction_id {
            removed_task_ids.insert(task.id.trim().to_string());
            return false;
        }
        true
    });
    removed_task_ids
}
