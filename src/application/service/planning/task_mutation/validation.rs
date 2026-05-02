// 학습 주석: task link 검증은 "존재하는 task id 집합"을 빠르게 조회해야 하므로 HashSet을 사용합니다.
// depends_on/blocked_by 각각을 검사할 때 매번 Vec를 선형 탐색하지 않게 해 줍니다.
use std::collections::HashSet;

// 학습 주석: Context는 queue projection rebuild 실패에 application-level 설명을 붙이고,
// Result는 validation 실패를 mutation caller에게 중단 가능한 오류로 돌려줍니다.
use anyhow::{Context, Result};

// 학습 주석: 이 파일은 domain document를 직접 수정하지 않고, mutation 이후 문서가 여전히 semantic rule과
// queue rule을 만족하는지 확인한 뒤 PriorityQueueProjection을 다시 계산합니다.
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningSemanticValidationService, PlanningValidationReport,
    PriorityQueueProjection, TaskAuthorityDocument,
};

// 학습 주석: validation은 PlanningTaskMutationService의 private step입니다. public API는 apply/preview 흐름을 쓰고,
// 이 module은 commit 직전 방어선으로만 호출됩니다.
use super::PlanningTaskMutationService;
// 학습 주석: helpers에는 domain validation report 해석, priority 범위 검사, task reference 오류 메시지 포맷이
// 모여 있어 이 파일은 "검증 순서"를 읽기 쉽게 유지합니다.
use super::helpers::{reject_task_validation_errors, validate_priorities, validate_task_reference};

// 학습 주석: 이 impl은 task mutation service의 validation phase입니다. mutation command가 문서를 바꾼 뒤
// 여기서 semantic/report/link/priority/queue projection을 모두 통과해야 repository commit으로 넘어갑니다.
impl PlanningTaskMutationService {
    // 학습 주석: validate_and_project는 "검증된 task_authority"와 "다시 계산한 queue projection"을 묶는 관문입니다.
    // caller는 성공한 projection만 commit result에 포함하므로, stale queue head를 사용자에게 보여 주지 않습니다.
    pub(super) fn validate_and_project(
        &self,
        // 학습 주석: directions는 task.direction_id가 실제 active/workstream catalog와 맞는지 확인하는 기준 문서입니다.
        directions: &DirectionCatalogDocument,
        // 학습 주석: task_authority는 mutation command들이 적용된 후보 문서입니다. 아직 영구 저장 전 상태입니다.
        task_authority: &TaskAuthorityDocument,
    ) -> Result<PriorityQueueProjection> {
        // 학습 주석: domain semantic validator는 여러 issue를 report에 누적합니다. 즉시 bail하지 않고 모아 두면
        // caller가 한 번에 더 많은 구조적 문제를 볼 수 있습니다.
        let mut report = PlanningValidationReport::new();
        // 학습 주석: directions와 task_authority를 함께 넣어 task가 존재하지 않는 direction을 가리키는 문제 같은
        // cross-document invariant를 검사합니다.
        PlanningSemanticValidationService::new().validate(
            Some(directions),
            Some(task_authority),
            &mut report,
        );
        // 학습 주석: task authority에 해당하는 error가 하나라도 있으면 mutation은 실패합니다.
        // warning이나 다른 file kind issue는 이 helper의 정책에 따라 걸러집니다.
        reject_task_validation_errors(&report)?;
        // 학습 주석: semantic validator가 포괄적 문서 규칙을 맡고, 이 추가 검사는 task 간 link graph의
        // blank/self/unknown reference를 mutation-specific error message로 막습니다.
        self.validate_task_links(task_authority)?;
        // 학습 주석: priority 값은 queue ordering의 입력이므로 projection을 만들기 전에 범위를 보장합니다.
        validate_priorities(task_authority)?;
        self.priority_queue_service
            // 학습 주석: 검증된 directions/task 문서로 queue head와 ordering projection을 새로 계산합니다.
            .build_projection(directions, task_authority)
            // 학습 주석: projection 자체가 실패하면 mutation 적용은 중단되고 이 context가 운영 로그/응답에 남습니다.
            .context("failed to rebuild planning queue projection")
    }

    // 학습 주석: validate_task_links는 depends_on과 blocked_by가 모두 기존 task id만 가리키도록 보장합니다.
    // 이 검사가 없으면 queue/worker가 존재하지 않는 선행 작업 때문에 진행 가능성을 잘못 판단할 수 있습니다.
    fn validate_task_links(&self, task_authority: &TaskAuthorityDocument) -> Result<()> {
        // 학습 주석: 먼저 모든 task id를 trim한 형태로 모아 reference 검증의 기준 집합을 만듭니다.
        // 입력 쪽에서도 trim해 비교하므로 파일의 주변 공백 때문에 false negative가 나지 않습니다.
        let task_ids = task_authority
            // 학습 주석: authoritative task 목록 전체를 기준으로 삼습니다.
            .tasks
            // 학습 주석: 각 task definition을 순회해 id만 뽑습니다.
            .iter()
            // 학습 주석: reference 비교 기준은 whitespace가 제거된 canonical id 문자열입니다.
            .map(|task| task.id.trim().to_string())
            // 학습 주석: HashSet으로 모아 contains lookup을 O(1)에 가깝게 만듭니다.
            .collect::<HashSet<_>>();
        // 학습 주석: 각 task마다 outgoing relation 두 종류를 모두 검사합니다.
        for task in &task_authority.tasks {
            // 학습 주석: 오류 메시지와 self-reference 비교에는 trim된 현재 task id를 사용합니다.
            let task_id = task.id.trim();
            // 학습 주석: dependency는 "이 task가 시작되기 전에 완료되어야 하는 작업" 관계입니다.
            for dependency_id in &task.depends_on {
                validate_task_reference("dependency", task_id, dependency_id, &task_ids)?;
            }
            // 학습 주석: blocker는 현재 task를 막는 외부/내부 작업으로, 같은 reference integrity rule을 공유합니다.
            for blocker_id in &task.blocked_by {
                validate_task_reference("blocker", task_id, blocker_id, &task_ids)?;
            }
        }
        // 학습 주석: 모든 relation이 blank/self/unknown을 피했으면 link graph는 commit 가능한 상태입니다.
        Ok(())
    }
}
