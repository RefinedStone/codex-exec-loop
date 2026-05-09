/*
 * `validation.rs`는 planning 문서를 "실행 전에 사람이 고칠 수 있는 문제 목록"으로 바꾸는
 * semantic validation 계층이다. 이 파일은 오류를 바로 반환하지 않고 `PlanningValidationReport`에
 * 누적한다. operator가 한 번에 하나의 오류만 보는 대신 directions/task authority 전체에서 고쳐야
 * 할 항목을 한 화면, admin API, repair prompt에서 함께 볼 수 있게 하기 위한 설계다.
 *
 * `queue.rs`와의 차이는 의도적이다. validation은 넓게 검사하고 report에 여러 issue를 쌓는다.
 * queue builder는 실제 queue projection을 만들기 위해 반드시 필요한 전제를 빠르게 확인하고 `Result`로
 * 실패한다. 그래서 이 파일은 operator feedback과 repair loop의 입력이고, queue는 runtime 실행 방어선이다.
 */
use std::collections::{HashMap, HashSet};

use chrono::DateTime;

use super::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningFileKind, PlanningValidationReport,
    TaskActor, TaskAuthorityDocument, TaskDefinition, TaskStatus,
};

#[derive(Default, Clone)]
// stateless service라 clone이 싸고, runtime/workspace/task mutation service가 같은 규칙을 주입받아 쓸 수 있다.
pub struct PlanningSemanticValidationService;

impl PlanningSemanticValidationService {
    // 내부 상태가 없으므로 생성자는 service graph에서 의도를 드러내는 역할만 한다.
    pub fn new() -> Self {
        Self
    }

    /*
     * validation orchestration은 세 단계로 나뉜다. direction 문서만 있어도 direction 자체 검사는 가능하고,
     * task authority만 있어도 task 자체 검사는 가능하다. 두 문서가 모두 있을 때만 cross-reference와
     * graph semantics를 검사한다. 이 분리는 초기화/repair 중 일부 파일만 존재하는 상태에서도 가능한 문제를
     * 최대한 보여 주기 위한 계약이다.
     */
    pub fn validate(
        &self,
        direction_catalog: Option<&DirectionCatalogDocument>,
        task_authority: Option<&TaskAuthorityDocument>,
        report: &mut PlanningValidationReport,
    ) {
        if let Some(direction_catalog) = direction_catalog {
            self.validate_direction_catalog(direction_catalog, report);
        }
        if let Some(task_authority) = task_authority {
            self.validate_task_authority(task_authority, report);
        }
        if let (Some(direction_catalog), Some(task_authority)) = (direction_catalog, task_authority)
        {
            self.validate_cross_references(direction_catalog, task_authority, report);
        }
    }

    fn validate_direction_catalog(
        &self,
        direction_catalog: &DirectionCatalogDocument,
        report: &mut PlanningValidationReport,
    ) {
        /*
         * direction catalog 검사는 상위 목표 문서의 최소 품질을 보장한다. queue는 `direction_id`만으로
         * task를 묶을 수 있지만, operator와 worker가 방향을 이해하려면 title, summary,
         * success criteria가 비어 있으면 안 된다. 실행 알고리즘에는 직접 필요 없어 보여도 여기서
         * 문서 품질을 검사하는 이유다.
         */
        if direction_catalog.version != PLANNING_FORMAT_VERSION {
            report.push_error(
                PlanningFileKind::Directions,
                "unsupported_directions_version",
                format!(
                    "direction authority version {} does not match supported version {}",
                    direction_catalog.version, PLANNING_FORMAT_VERSION
                ),
            );
        }

        if direction_catalog.directions.is_empty() {
            report.push_error(
                PlanningFileKind::Directions,
                "missing_directions",
                "direction authority must contain at least one direction",
            );
            return;
        }

        let mut seen_ids = HashSet::new();
        for direction in &direction_catalog.directions {
            let direction_id = direction.id.trim();
            // id는 cross-reference의 key가 되므로 blank와 duplicate는 error로 보고한다.
            if direction_id.is_empty() {
                report.push_error(
                    PlanningFileKind::Directions,
                    "blank_direction_id",
                    "direction ids must not be blank",
                );
            } else if !seen_ids.insert(direction_id.to_string()) {
                report.push_error(
                    PlanningFileKind::Directions,
                    "duplicate_direction_id",
                    format!("duplicate direction id: {direction_id}"),
                );
            }

            if direction.title.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::Directions,
                    "blank_direction_title",
                    format!("direction {direction_id} must have a non-empty title"),
                );
            }
            if direction.summary.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::Directions,
                    "blank_direction_summary",
                    format!("direction {direction_id} must have a non-empty summary"),
                );
            }
            if direction.success_criteria.is_empty()
                || direction
                    .success_criteria
                    .iter()
                    .any(|criterion| criterion.trim().is_empty())
            {
                report.push_error(
                    PlanningFileKind::Directions,
                    "invalid_success_criteria",
                    format!(
                        "direction {direction_id} must include at least one non-empty success criterion"
                    ),
                );
            }
        }
    }

    fn validate_task_authority(
        &self,
        task_authority: &TaskAuthorityDocument,
        report: &mut PlanningValidationReport,
    ) {
        /*
         * task authority 검사는 실행 단위의 기본 형식과 감사 가능성을 확인한다. 특히 worker-authored
         * 작업에는 relation note를 요구해 자동 생성된 일이 어떤 direction을 만족시키려는지 나중에
         * 추적할 수 있게 한다.
         */
        if task_authority.version != PLANNING_FORMAT_VERSION {
            report.push_error(
                PlanningFileKind::TaskAuthority,
                "unsupported_task_authority_version",
                format!(
                    "task authority version {} does not match supported version {}",
                    task_authority.version, PLANNING_FORMAT_VERSION
                ),
            );
        }

        let mut seen_ids = HashSet::new();
        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            // task id는 dependency graph의 node id라 direction id와 같은 수준으로 strict하게 본다.
            if task_id.is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_task_id",
                    "task ids must not be blank",
                );
            } else if !seen_ids.insert(task_id.to_string()) {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "duplicate_task_id",
                    format!("duplicate task id: {task_id}"),
                );
            }

            if task.direction_id.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_direction_reference",
                    format!("task {task_id} must reference a direction_id"),
                );
            }
            if task.title.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_task_title",
                    format!("task {task_id} must have a non-empty title"),
                );
            }
            if task.description.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_task_description",
                    format!("task {task_id} must have a non-empty description"),
                );
            }
            if task.requires_relation_note() && task.direction_relation_note.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "missing_direction_relation_note",
                    format!("worker-authored task {task_id} must include direction_relation_note"),
                );
            }
            if task.dynamic_priority_delta != 0 && task.priority_reason.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "missing_priority_reason",
                    format!(
                        "task {task_id} must include priority_reason when dynamic_priority_delta is non-zero"
                    ),
                );
            }
            self.validate_task_priority(task, report);
            if DateTime::parse_from_rfc3339(task.updated_at.as_str()).is_err() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "invalid_updated_at",
                    format!("task {task_id} must use RFC3339 updated_at"),
                );
            }

            // 내부 link shape는 task 하나만으로 판정할 수 있어 cross-reference pass를 기다릴 필요가 없다.
            self.validate_task_links(task, report);
        }
    }

    fn validate_task_priority(&self, task: &TaskDefinition, report: &mut PlanningValidationReport) {
        /*
         * 우선순위 범위는 mutation 서비스의 세부 사항이 아니라 task authority의 불변 조건이다.
         * 큐 랭킹과 모든 inbound projection이 같은 combined priority를 읽기 때문에,
         * queue projection을 다시 만들기 전에 semantic validation이 허용 범위를 소유한다.
         */
        let task_id = task.id.trim();
        if !(0..=100).contains(&task.base_priority) {
            report.push_error(
                PlanningFileKind::TaskAuthority,
                "invalid_base_priority",
                format!("task {task_id} base_priority must be within 0..100"),
            );
        }
        if !(-100..=100).contains(&task.dynamic_priority_delta) {
            report.push_error(
                PlanningFileKind::TaskAuthority,
                "invalid_dynamic_priority_delta",
                format!("task {task_id} dynamic_priority_delta must be within -100..100"),
            );
        }
        let combined_priority = task.base_priority.checked_add(task.dynamic_priority_delta);
        if !matches!(combined_priority, Some(priority) if (0..=100).contains(&priority)) {
            report.push_error(
                PlanningFileKind::TaskAuthority,
                "invalid_combined_priority",
                format!("task {task_id} combined priority must stay within 0..100"),
            );
        }
    }

    fn validate_task_links(&self, task: &TaskDefinition, report: &mut PlanningValidationReport) {
        /*
         * `validate_task_links`는 한 task 내부의 `depends_on`/`blocked_by` 배열만 본다. 여기서는
         * 참조 대상이 실제 존재하는지까지는 판단하지 않고, 빈 문자열, 자기 자신, 중복, 동일 id가
         * dependency와 blocker에 동시에 들어간 모순처럼 "한 task만 봐도 알 수 있는 문제"를 잡는다.
         */
        let task_id = task.id.trim();
        let mut dependency_ids = HashSet::new();
        for dependency_id in &task.depends_on {
            let normalized_dependency_id = dependency_id.trim();
            if normalized_dependency_id.is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_dependency_id",
                    format!("task {task_id} contains a blank depends_on entry"),
                );
                continue;
            }
            if normalized_dependency_id == task_id {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "self_dependency",
                    format!("task {task_id} cannot depend on itself"),
                );
            }
            if !dependency_ids.insert(normalized_dependency_id.to_string()) {
                report.push_warning(
                    PlanningFileKind::TaskAuthority,
                    "duplicate_dependency_id",
                    format!("task {task_id} repeats dependency id {normalized_dependency_id}"),
                );
            }
        }

        let mut blocker_ids = HashSet::new();
        for blocker_id in &task.blocked_by {
            let normalized_blocker_id = blocker_id.trim();
            if normalized_blocker_id.is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_blocker_id",
                    format!("task {task_id} contains a blank blocked_by entry"),
                );
                continue;
            }
            if !blocker_ids.insert(normalized_blocker_id.to_string()) {
                report.push_warning(
                    PlanningFileKind::TaskAuthority,
                    "duplicate_blocker_id",
                    format!("task {task_id} repeats blocker id {normalized_blocker_id}"),
                );
            }
            if normalized_blocker_id == task_id {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "self_blocker",
                    format!("task {task_id} cannot block itself"),
                );
            }
        }

        let mut conflict_ids = HashSet::new();
        for dependency_id in &task.depends_on {
            let normalized_dependency_id = dependency_id.trim();
            if normalized_dependency_id.is_empty() {
                continue;
            }
            if blocker_ids.contains(normalized_dependency_id)
                && conflict_ids.insert(normalized_dependency_id.to_string())
            {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "dependency_blocker_conflict",
                    format!(
                        "task {task_id} cannot list {normalized_dependency_id} in both depends_on and blocked_by"
                    ),
                );
            }
        }

        // Proposed worker task는 실행 queue에 바로 들어가지 않는 상태라 warning으로 드러내 operator 승격을 유도한다.
        if matches!(task.status, TaskStatus::Proposed) && task.created_by == TaskActor::Worker {
            report.push_warning(
                PlanningFileKind::TaskAuthority,
                "worker_proposed_task",
                format!("task {task_id} is proposed by a worker and will stay out of normal execution until promoted"),
            );
        }
    }

    fn validate_cross_references(
        &self,
        direction_catalog: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
        report: &mut PlanningValidationReport,
    ) {
        /*
         * cross-reference pass는 두 authority 문서를 함께 놓고 참조 그래프를 확인한다. task의
         * `direction_id`가 실제 direction에 있는지, dependency/blocker id가 실제 task에 있는지 확인한 뒤
         * 더 깊은 의미 규칙과 dependency cycle 검사를 이어서 실행한다.
         */
        let direction_ids = direction_catalog
            .directions
            .iter()
            .map(|direction| direction.id.trim().to_string())
            .collect::<HashSet<_>>();
        let task_map = task_authority
            .tasks
            .iter()
            .map(|task| (task.id.trim().to_string(), task))
            .collect::<HashMap<_, _>>();

        // 아래 reference check는 missing target을 자세히 보고하고, 뒤의 semantic pass가 같은 map을 재사용한다.
        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            if !direction_ids.contains(task.direction_id.trim()) {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "missing_direction_reference",
                    format!(
                        "task {task_id} references unknown direction_id {}",
                        task.direction_id.trim()
                    ),
                );
            }
            for dependency_id in &task.depends_on {
                let normalized_dependency_id = dependency_id.trim();
                if !task_map.contains_key(normalized_dependency_id) {
                    report.push_error(
                        PlanningFileKind::TaskAuthority,
                        "missing_dependency_reference",
                        format!(
                            "task {task_id} references unknown dependency {normalized_dependency_id}"
                        ),
                    );
                }
            }
            for blocker_id in &task.blocked_by {
                let normalized_blocker_id = blocker_id.trim();
                if !task_map.contains_key(normalized_blocker_id) {
                    report.push_error(
                        PlanningFileKind::TaskAuthority,
                        "missing_blocker_reference",
                        format!(
                            "task {task_id} references unknown blocker {normalized_blocker_id}"
                        ),
                    );
                }
            }
        }

        self.validate_task_semantics(task_authority, &task_map, report);

        // 존재하지 않는 dependency는 위에서 이미 error가 되며, cycle detection은 존재하는 node 사이의 순환을 본다.
        if self.contains_dependency_cycle(task_authority) {
            report.push_error(
                PlanningFileKind::TaskAuthority,
                "dependency_cycle_detected",
                "task authority contains a dependency cycle",
            );
        }
    }

    fn contains_dependency_cycle(&self, task_authority: &TaskAuthorityDocument) -> bool {
        /*
         * dependency cycle은 "A가 B를 기다리고 B가 다시 A를 기다리는" 형태라 queue가 영원히 풀리지
         * 않는다. 이 함수는 task authority를 adjacency map으로 바꾼 뒤 DFS용 `detect_cycle`에 넘겨
         * 순환 참조 여부만 bool로 돌려준다.
         */
        let adjacency_map = task_authority
            .tasks
            .iter()
            .map(|task| {
                (
                    task.id.trim().to_string(),
                    task.depends_on
                        .iter()
                        .map(|dependency_id| dependency_id.trim().to_string())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();

        let mut temporary_marks = HashSet::new();
        let mut permanent_marks = HashSet::new();

        // 하나라도 cycle을 발견하면 report에는 단일 summary error만 추가한다.
        adjacency_map.keys().any(|task_id| {
            self.detect_cycle(
                task_id,
                &adjacency_map,
                &mut temporary_marks,
                &mut permanent_marks,
            )
        })
    }

    fn detect_cycle(
        &self,
        task_id: &str,
        adjacency_map: &HashMap<String, Vec<String>>,
        temporary_marks: &mut HashSet<String>,
        permanent_marks: &mut HashSet<String>,
    ) -> bool {
        /*
         * DFS의 temporary/permanent mark 알고리즘이다. temporary mark에 이미 있는 node를 다시 만나면
         * 현재 재귀 경로 안에서 되돌아온 것이므로 cycle이다. permanent mark에 있는 node는 이전 탐색에서
         * 안전하다고 확인된 node라 다시 검사하지 않는다.
         */
        if permanent_marks.contains(task_id) {
            return false;
        }
        if !temporary_marks.insert(task_id.to_string()) {
            return true;
        }

        if let Some(dependencies) = adjacency_map.get(task_id) {
            for dependency_id in dependencies {
                // missing dependency는 reference pass에서 이미 보고되므로 cycle 탐색은 graph 안에 있는 node만 따라간다.
                if adjacency_map.contains_key(dependency_id)
                    && self.detect_cycle(
                        dependency_id,
                        adjacency_map,
                        temporary_marks,
                        permanent_marks,
                    )
                {
                    return true;
                }
            }
        }

        temporary_marks.remove(task_id);
        permanent_marks.insert(task_id.to_string());
        false
    }

    fn validate_task_semantics(
        &self,
        task_authority: &TaskAuthorityDocument,
        task_map: &HashMap<String, &TaskDefinition>,
        report: &mut PlanningValidationReport,
    ) {
        /*
         * `validate_task_semantics`는 단순 형식 검사를 넘어 상태 조합의 의미를 검사한다. Done task가
         * 아직 완료되지 않은 dependency를 가지고 있으면 history가 모순된다. 또한 InProgress task가 둘
         * 이상이면 queue priority model과 agent handoff가 "현재 진행 중인 일"을 하나로 좁힐 수 없다.
         */
        let mut in_progress_task_ids = Vec::new();

        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            if task.status == TaskStatus::InProgress {
                in_progress_task_ids.push(task_id);
            }
            if task.status != TaskStatus::Done {
                continue;
            }

            for dependency_id in &task.depends_on {
                let normalized_dependency_id = dependency_id.trim();
                if let Some(dependency) = task_map.get(normalized_dependency_id)
                    && !dependency.status.is_dependency_complete()
                {
                    report.push_error(
                        PlanningFileKind::TaskAuthority,
                        "done_task_unresolved_dependency",
                        format!(
                            "done task {task_id} cannot depend on incomplete task {normalized_dependency_id} ({})",
                            dependency.status.label()
                        ),
                    );
                }
            }

            for blocker_id in &task.blocked_by {
                let normalized_blocker_id = blocker_id.trim();
                if let Some(blocker) = task_map.get(normalized_blocker_id)
                    && !blocker.status.clears_blocker()
                {
                    report.push_error(
                        PlanningFileKind::TaskAuthority,
                        "done_task_unresolved_blocker",
                        format!(
                            "done task {task_id} cannot remain blocked by task {normalized_blocker_id} ({})",
                            blocker.status.label()
                        ),
                    );
                }
            }
        }

        if in_progress_task_ids.len() > 1 {
            report.push_error(
                PlanningFileKind::TaskAuthority,
                "multiple_in_progress_tasks",
                format!(
                    "task authority may contain at most one in_progress task; found {}: {}",
                    in_progress_task_ids.len(),
                    in_progress_task_ids.join(", ")
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PlanningSemanticValidationService;
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, PlanningFileKind,
        PlanningValidationReport, QueueIdleConfig, TaskActor, TaskAuthorityDocument,
        TaskDefinition, TaskStatus,
    };

    fn direction(id: &str) -> DirectionDefinition {
        DirectionDefinition {
            id: id.to_string(),
            title: format!("{id} title"),
            summary: format!("{id} summary"),
            success_criteria: vec![format!("{id} done")],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }
    }

    fn task(id: &str, status: TaskStatus) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: "direction-a".to_string(),
            direction_relation_note: "supports direction".to_string(),
            title: format!("{id} title"),
            description: format!("{id} description"),
            status,
            base_priority: 10,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: Default::default(),
            updated_at: "2026-04-09T09:00:00Z".to_string(),
        }
    }

    fn validate(
        directions: &DirectionCatalogDocument,
        ledger: &TaskAuthorityDocument,
    ) -> PlanningValidationReport {
        // 테스트는 public validation orchestration을 통과해 report 누적 계약을 검증한다.
        let mut report = PlanningValidationReport::new();
        PlanningSemanticValidationService::new().validate(
            Some(directions),
            Some(ledger),
            &mut report,
        );
        report
    }

    #[test]
    fn validates_cross_references_and_dependency_cycles() {
        // dependency cycle과 missing reference는 같은 graph pass에서 잡히지만 서로 다른 issue code로 남아야 한다.
        let directions = DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![direction("direction-a")],
        };
        let mut first = task("task-a", TaskStatus::Ready);
        first.depends_on = vec!["task-b".to_string()];
        let mut second = task("task-b", TaskStatus::Ready);
        second.depends_on = vec!["task-a".to_string(), "missing-task".to_string()];
        let ledger = TaskAuthorityDocument {
            version: 1,
            tasks: vec![first, second],
        };

        let report = validate(&directions, &ledger);
        let codes = report
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<Vec<_>>();

        assert!(codes.contains(&"missing_dependency_reference"));
        assert!(codes.contains(&"dependency_cycle_detected"));
    }

    #[test]
    fn validates_done_task_dependency_and_single_in_progress_rules() {
        // 완료된 task의 unresolved dependency와 다중 in_progress는 queue projection 전에 막아야 하는 semantic error다.
        let directions = DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![direction("direction-a")],
        };
        let mut done = task("done-task", TaskStatus::Done);
        done.depends_on = vec!["open-task".to_string()];
        let ledger = TaskAuthorityDocument {
            version: 1,
            tasks: vec![
                done,
                task("open-task", TaskStatus::Ready),
                task("active-a", TaskStatus::InProgress),
                task("active-b", TaskStatus::InProgress),
            ],
        };

        let report = validate(&directions, &ledger);

        assert!(report.issues.iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskAuthority
                && issue.code == "done_task_unresolved_dependency"
        }));
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "multiple_in_progress_tasks")
        );
    }

    #[test]
    fn validates_task_priority_bounds() {
        // priority 범위는 mutation helper가 아니라 task authority semantic validation에서 보고돼야 한다.
        let directions = DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![direction("direction-a")],
        };
        let mut invalid_base = task("invalid-base", TaskStatus::Ready);
        invalid_base.base_priority = -1;
        let mut invalid_delta = task("invalid-delta", TaskStatus::Ready);
        invalid_delta.dynamic_priority_delta = 101;
        invalid_delta.priority_reason = "temporary boost".to_string();
        let mut invalid_combined = task("invalid-combined", TaskStatus::Ready);
        invalid_combined.base_priority = 90;
        invalid_combined.dynamic_priority_delta = 20;
        invalid_combined.priority_reason = "temporary boost".to_string();
        let mut overflow_combined = task("overflow-combined", TaskStatus::Ready);
        overflow_combined.base_priority = i32::MAX;
        overflow_combined.dynamic_priority_delta = 1;
        overflow_combined.priority_reason = "temporary boost".to_string();
        let ledger = TaskAuthorityDocument {
            version: 1,
            tasks: vec![
                invalid_base,
                invalid_delta,
                invalid_combined,
                overflow_combined,
            ],
        };

        let report = validate(&directions, &ledger);
        let codes = report
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<Vec<_>>();

        assert!(codes.contains(&"invalid_base_priority"));
        assert!(codes.contains(&"invalid_dynamic_priority_delta"));
        assert!(codes.contains(&"invalid_combined_priority"));
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "invalid_combined_priority"
                    && issue.message.contains("overflow-combined"))
        );
    }
}
