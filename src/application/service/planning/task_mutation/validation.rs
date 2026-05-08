// Context는 queue projection rebuild 실패에 application-level 설명을 붙이고, Result는 validation 실패를
// mutation caller에게 중단 가능한 오류로 돌려준다.
use anyhow::{Context, Result};

// 이 파일은 domain document를 직접 수정하지 않고, mutation 이후 문서가 여전히 semantic rule과 queue rule을 만족하는지
// 확인한 뒤 PriorityQueueProjection을 다시 계산한다.
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningSemanticValidationService, PlanningValidationReport,
    PriorityQueueProjection, TaskAuthorityDocument,
};

// validation은 PlanningTaskMutationService의 private step이다. public API는 apply/preview 흐름을 쓰고, 이 module은
// commit 직전 방어선으로만 호출된다.
use super::PlanningTaskMutationService;
// mutation validation은 domain report 해석 뒤 queue projection rebuild로 끝난다.
use super::helpers::reject_task_validation_errors;

// 이 impl은 task mutation service의 validation phase다. mutation command가 문서를 바꾼 뒤 여기서
// semantic/report/link/priority/queue projection을 모두 통과해야 repository commit으로 넘어간다.
impl PlanningTaskMutationService {
    // validate_and_project는 "검증된 task_authority"와 "다시 계산한 queue projection"을 묶는 관문이다.
    // caller는 성공한 projection만 commit result에 포함하므로, stale queue head를 사용자에게 보여 주지 않는다.
    pub(super) fn validate_and_project(
        &self,
        // directions는 task.direction_id가 실제 active/workstream catalog와 맞는지 확인하는 기준 문서다.
        directions: &DirectionCatalogDocument,
        // task_authority는 mutation command들이 적용된 후보 문서다. 아직 영구 저장 전 상태다.
        task_authority: &TaskAuthorityDocument,
    ) -> Result<PriorityQueueProjection> {
        // domain semantic validator는 여러 issue를 report에 누적한다. 즉시 bail하지 않고 모아 두면 caller가
        // 한 번에 더 많은 구조적 문제를 볼 수 있다.
        let mut report = PlanningValidationReport::new();
        // directions와 task_authority를 함께 넣어 task가 존재하지 않는 direction을 가리키는 문제 같은
        // cross-document invariant를 검사한다.
        PlanningSemanticValidationService::new().validate(
            Some(directions),
            Some(task_authority),
            &mut report,
        );
        // task authority에 해당하는 error가 하나라도 있으면 mutation은 실패한다. warning이나 다른 file kind issue는
        // 이 helper의 정책에 따라 걸러진다.
        reject_task_validation_errors(&report)?;
        self.priority_queue_service
            // 검증된 directions/task 문서로 queue head와 ordering projection을 새로 계산한다.
            .build_projection(directions, task_authority)
            // projection 자체가 실패하면 mutation 적용은 중단되고 이 context가 운영 로그/응답에 남는다.
            .context("failed to rebuild planning queue projection")
    }
}
