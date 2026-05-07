/*
 * 이 파일은 planning authority 문서를 "실행 가능한 순서"로 바꾸는 도메인 알고리즘이다.
 * mod.rs의 TaskDefinition/DirectionDefinition은 원본 문서이고, 여기의 PriorityQueueService는
 * 그 원본을 읽어 PriorityQueueProjection이라는 화면/자동화 친화적인 뷰를 만든다.
 *
 * 중요한 연결점:
 * - validation.rs는 문서가 의미적으로 말이 되는지 폭넓게 검사한다.
 * - queue.rs는 실행 시점에 필요한 더 엄격한 전제, 즉 unknown reference, updated_at 정렬 가능성,
 *   in_progress 단일성을 다시 확인한다.
 * - application/service/planning/runtime 쪽은 projection.next_task를 보고 다음 main/sub session
 *   handoff를 만들지 결정한다.
 */
use std::{collections::HashMap, fmt};

use chrono::DateTime;

use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, PriorityQueueProjection,
    PriorityQueueSkippedTask, PriorityQueueTask, TaskAuthorityDocument, TaskDefinition,
};

// PriorityQueueService는 저장소나 clock을 갖지 않는 순수 도메인 서비스다. 같은 authority
// 문서를 넣으면 항상 같은 queue projection을 돌려줘야 하므로 상태를 들지 않는다.
#[derive(Default, Clone)]
pub struct PriorityQueueService;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorityQueueBuildError {
    // 실행 큐는 현재 진행 중인 작업을 최우선으로 고정한다. 둘 이상이면 자동 handoff가 어느
    // 작업을 이어받아야 하는지 정의할 수 없어서 projection 자체를 실패시킨다.
    MultipleInProgressTasks {
        task_ids: Vec<String>,
    },
    // 아래 reference 오류들은 validation report가 이미 잡을 수 있지만, runtime queue builder는
    // panic 대신 Result로 멈추기 위해 자체 preflight에서도 같은 전제를 확인한다.
    UnknownDirection {
        task_id: String,
        direction_id: String,
    },
    MissingDependency {
        task_id: String,
        dependency_id: String,
    },
    MissingBlocker {
        task_id: String,
        blocker_id: String,
    },
    InvalidUpdatedAt {
        task_id: String,
        updated_at: String,
    },
}

impl fmt::Display for PriorityQueueBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MultipleInProgressTasks { task_ids } => write!(
                formatter,
                "task authority may contain at most one in_progress task; found {}: {}",
                task_ids.len(),
                task_ids.join(", ")
            ),
            Self::UnknownDirection {
                task_id,
                direction_id,
            } => write!(
                formatter,
                "task {task_id} references unknown direction_id {}",
                display_reference(direction_id)
            ),
            Self::MissingDependency {
                task_id,
                dependency_id,
            } => write!(
                formatter,
                "task {task_id} references unknown dependency {}",
                display_reference(dependency_id)
            ),
            Self::MissingBlocker {
                task_id,
                blocker_id,
            } => write!(
                formatter,
                "task {task_id} references unknown blocker {}",
                display_reference(blocker_id)
            ),
            Self::InvalidUpdatedAt {
                task_id,
                updated_at,
            } => write!(
                formatter,
                "task {task_id} must use RFC3339 updated_at for queue ordering, got {}",
                display_reference(updated_at)
            ),
        }
    }
}

impl std::error::Error for PriorityQueueBuildError {}

#[derive(Debug, Clone)]
struct QueueCandidate {
    /*
     * QueueCandidate는 외부에 공개되는 DTO가 아니라 정렬을 위해 잠깐 쓰는 내부 구조체다.
     * PriorityQueueTask만으로도 화면에 필요한 정보는 충분하지만, 정렬에는 readiness_rank와
     * timestamp처럼 화면에 그대로 노출하지 않는 보조 값이 필요해서 별도 구조체로 감싼다.
     */
    // 낮을수록 먼저 온다. InProgress가 Ready보다 앞서야 진행 중인 작업이 끊기지 않는다.
    readiness_rank: u8,
    combined_priority: i32,
    // 문자열 updated_at은 표시용으로 남기고, 정렬은 parse된 epoch millis로만 한다.
    updated_at_epoch_millis: i64,
    task: PriorityQueueTask,
}

impl PriorityQueueService {
    pub fn new() -> Self {
        Self
    }

    pub fn build_projection(
        &self,
        directions: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
    ) -> Result<PriorityQueueProjection, PriorityQueueBuildError> {
        /*
         * build_projection은 이 파일의 핵심 진입점이다. 입력으로 받은 directions/task_authority를
         * 직접 수정하지 않고, 다음 네 가지 결과 뷰를 계산한다.
         * - active_tasks: 지금 실행 가능한 작업들, rank가 붙어 있음
         * - next_task: active_tasks의 첫 번째 항목, 자동 handoff가 실제로 집는 작업
         * - proposed_tasks: worker가 제안했지만 operator promotion 전이라 실행 큐와 분리된 작업
         * - skipped_tasks: 왜 실행 후보가 아닌지 사람이 읽을 수 있는 reason이 붙은 작업
         *
         * 이 함수가 domain 계층에 있는 이유는 정렬/skip 규칙이 UI나 DB 형식이 아니라 Akra
         * planning의 업무 규칙이기 때문이다.
         */
        // trim된 id를 key로 삼아 YAML/JSON authoring 중 생긴 양끝 공백이 reference resolution을
        // 망가뜨리지 않게 한다. 원본 id 문자열은 projection DTO에 그대로 보존한다.
        let direction_map = directions
            .directions
            .iter()
            .map(|direction| (direction.id.trim(), direction))
            .collect::<HashMap<_, _>>();
        let task_map = task_authority
            .tasks
            .iter()
            .map(|task| (task.id.trim(), task))
            .collect::<HashMap<_, _>>();
        let updated_at_epoch_millis_by_task_id =
            self.validate_queue_inputs(task_authority, &direction_map, &task_map)?;

        let mut candidates = Vec::new();
        let mut proposed_candidates = Vec::new();
        let mut skipped_tasks = Vec::new();

        for task in &task_authority.tasks {
            /*
             * 이 반복문은 task 하나를 세 갈래 중 하나로 분류한다.
             * 1. direction이 비활성/완료거나 dependency/blocker가 풀리지 않았으면 skipped_tasks.
             * 2. Proposed 상태는 실행 큐가 아니라 proposed_candidates.
             * 3. Ready/InProgress처럼 queue_readiness_rank가 있는 상태는 active candidates.
             */
            let normalized_direction_id = task.direction_id.trim();
            let direction = direction_map
                .get(normalized_direction_id)
                .expect("queue build preflight should validate direction references");

            // direction이 paused/done이면 그 아래 task 상태가 Ready여도 실행하지 않는다. direction은
            // operator가 큰 흐름을 멈추는 상위 switch다.
            if !direction.state.allows_queue_execution() {
                skipped_tasks.push(self.skipped_task(
                    task,
                    format!("direction {} is {}", direction.id, direction.state_label()),
                ));
                continue;
            }

            if task.status == crate::domain::planning::TaskStatus::Proposed {
                // proposed task도 dependency/blocker가 풀리지 않으면 proposed list가 아니라 skipped로
                // 보낸다. promotion 후보 UI가 "검토할 수 있는 제안"만 보여주도록 하기 위해서다.
                if let Some(reason) = self.unresolved_dependency_reason(task, &task_map) {
                    skipped_tasks.push(self.skipped_task(task, reason));
                    continue;
                }
                if let Some(reason) = self.unresolved_blocker_reason(task, &task_map) {
                    skipped_tasks.push(self.skipped_task(task, reason));
                    continue;
                }

                // Proposed는 자동 실행 순서에 들어가지 않지만, operator 검토 순서를 위해 active
                // 후보와 같은 DTO shape를 재사용한다.
                proposed_candidates.push(QueueCandidate {
                    readiness_rank: 0,
                    combined_priority: task.combined_priority(),
                    updated_at_epoch_millis: *updated_at_epoch_millis_by_task_id
                        .get(task.id.trim())
                        .expect("queue build preflight should validate updated_at"),
                    task: PriorityQueueTask {
                        rank: 0,
                        task_id: task.id.clone(),
                        direction_id: task.direction_id.clone(),
                        direction_title: direction.title.clone(),
                        task_title: task.title.clone(),
                        status: task.status,
                        combined_priority: task.combined_priority(),
                        updated_at: task.updated_at.clone(),
                        rank_reasons: build_rank_reasons(task),
                    },
                });
                continue;
            }

            let Some(readiness_rank) = task.status.queue_readiness_rank() else {
                skipped_tasks.push(self.skipped_task(
                    task,
                    format!("status {} is not executable", task.status.label()),
                ));
                continue;
            };

            // dependency와 blocker는 서로 다른 의미를 가진다. dependency는 완료되어야 하는 선행
            // 작업이고, blocker는 취소/해소되어야 하는 막힘이므로 reason copy도 분리한다.
            if let Some(reason) = self.unresolved_dependency_reason(task, &task_map) {
                skipped_tasks.push(self.skipped_task(task, reason));
                continue;
            }
            if let Some(reason) = self.unresolved_blocker_reason(task, &task_map) {
                skipped_tasks.push(self.skipped_task(task, reason));
                continue;
            }

            candidates.push(QueueCandidate {
                readiness_rank,
                combined_priority: task.combined_priority(),
                updated_at_epoch_millis: *updated_at_epoch_millis_by_task_id
                    .get(task.id.trim())
                    .expect("queue build preflight should validate updated_at"),
                task: PriorityQueueTask {
                    rank: 0,
                    task_id: task.id.clone(),
                    direction_id: task.direction_id.clone(),
                    direction_title: direction.title.clone(),
                    task_title: task.title.clone(),
                    status: task.status,
                    combined_priority: task.combined_priority(),
                    updated_at: task.updated_at.clone(),
                    rank_reasons: build_rank_reasons(task),
                },
            });
        }

        candidates.sort_by(|left, right| {
            /*
             * active queue 정렬 규칙은 stable한 자동화를 위해 순서를 명확히 고정한다.
             * 먼저 InProgress를 Ready보다 우선하고, 그 안에서는 높은 combined_priority를 먼저 둔다.
             * 우선순위까지 같으면 오래된 updated_at을 먼저 처리하고, 마지막으로 task_id를 비교해
             * 완전한 결정성을 확보한다.
             */
            left.readiness_rank
                .cmp(&right.readiness_rank)
                .then_with(|| right.combined_priority.cmp(&left.combined_priority))
                .then_with(|| {
                    left.updated_at_epoch_millis
                        .cmp(&right.updated_at_epoch_millis)
                })
                .then_with(|| left.task.task_id.cmp(&right.task.task_id))
        });

        // rank는 정렬이 끝난 뒤 외부 DTO에 부여한다. 후보 생성 중에는 아직 최종 순서가 아니므로
        // 0으로 두고, projection boundary에서 1-based rank로 바꾼다.
        let active_tasks = candidates
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| PriorityQueueTask {
                rank: index + 1,
                ..candidate.task
            })
            .collect::<Vec<_>>();
        let next_task = active_tasks.first().cloned();

        proposed_candidates.sort_by(|left, right| {
            /*
             * proposed task는 아직 자동 실행하지 않으므로 readiness_rank를 쓰지 않는다.
             * 대신 operator가 검토할 때 가치가 큰 제안이 위로 오도록 priority, 오래된 제안, id
             * 순서로 정렬한다.
             */
            right
                .combined_priority
                .cmp(&left.combined_priority)
                .then_with(|| {
                    left.updated_at_epoch_millis
                        .cmp(&right.updated_at_epoch_millis)
                })
                .then_with(|| left.task.task_id.cmp(&right.task.task_id))
        });

        let proposed_tasks = proposed_candidates
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| PriorityQueueTask {
                rank: index + 1,
                ..candidate.task
            })
            .collect::<Vec<_>>();
        Ok(PriorityQueueProjection {
            next_task,
            active_tasks,
            proposed_tasks,
            skipped_tasks,
        })
    }

    fn validate_queue_inputs<'a>(
        &self,
        task_authority: &'a TaskAuthorityDocument,
        direction_map: &HashMap<&'a str, &'a DirectionDefinition>,
        task_map: &HashMap<&'a str, &'a TaskDefinition>,
    ) -> Result<HashMap<&'a str, i64>, PriorityQueueBuildError> {
        /*
         * validate_queue_inputs는 projection 계산 전에 "이후 expect/unreachable이 안전한가"를
         * 확인하는 방어선이다. validation.rs와 비슷해 보이지만 목적이 다르다. validation.rs는
         * 사용자에게 누적 report를 보여주고, 이 함수는 queue 계산이 중간에 잘못된 참조로 패닉하지
         * 않도록 즉시 Result error로 멈춘다.
         */
        let in_progress_tasks = task_authority
            .tasks
            .iter()
            .filter(|task| task.status == crate::domain::planning::TaskStatus::InProgress)
            .collect::<Vec<_>>();

        if in_progress_tasks.len() > 1 {
            return Err(PriorityQueueBuildError::MultipleInProgressTasks {
                task_ids: in_progress_tasks
                    .into_iter()
                    .map(|task| task.id.trim().to_string())
                    .collect(),
            });
        }

        let mut updated_at_epoch_millis_by_task_id = HashMap::new();
        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            if !direction_map.contains_key(task.direction_id.trim()) {
                return Err(PriorityQueueBuildError::UnknownDirection {
                    task_id: task_id.to_string(),
                    direction_id: task.direction_id.trim().to_string(),
                });
            }
            let updated_at_epoch_millis = parse_updated_at_epoch_millis(task.updated_at.as_str())
                .map_err(|_| {
                PriorityQueueBuildError::InvalidUpdatedAt {
                    task_id: task_id.to_string(),
                    updated_at: task.updated_at.clone(),
                }
            })?;
            updated_at_epoch_millis_by_task_id.insert(task_id, updated_at_epoch_millis);

            // dependency/blocker reference 존재성은 여기서만 확인하고, "해결됐는가"는 후보 분류
            // 단계에서 skip reason으로 만든다.
            for dependency_id in &task.depends_on {
                let normalized_dependency_id = dependency_id.trim();
                if !task_map.contains_key(normalized_dependency_id) {
                    return Err(PriorityQueueBuildError::MissingDependency {
                        task_id: task_id.to_string(),
                        dependency_id: normalized_dependency_id.to_string(),
                    });
                }
            }
            for blocker_id in &task.blocked_by {
                let normalized_blocker_id = blocker_id.trim();
                if !task_map.contains_key(normalized_blocker_id) {
                    return Err(PriorityQueueBuildError::MissingBlocker {
                        task_id: task_id.to_string(),
                        blocker_id: normalized_blocker_id.to_string(),
                    });
                }
            }
        }

        Ok(updated_at_epoch_millis_by_task_id)
    }

    fn skipped_task(&self, task: &TaskDefinition, reason: String) -> PriorityQueueSkippedTask {
        // skipped projection은 queue가 왜 집지 않았는지를 설명하는 최소 DTO다. direction title이나
        // priority는 실행 후보에만 필요하므로 여기서는 원인 설명에 필요한 값만 유지한다.
        PriorityQueueSkippedTask {
            task_id: task.id.clone(),
            task_title: task.title.clone(),
            direction_id: task.direction_id.clone(),
            status: task.status,
            reason,
        }
    }
    fn unresolved_dependency_reason(
        &self,
        task: &TaskDefinition,
        task_map: &HashMap<&str, &TaskDefinition>,
    ) -> Option<String> {
        // dependency는 완료 상태여야 해소된다. 아직 진행 중인 dependency는 그 상태 label과 함께
        // reason에 남겨 operator가 무엇을 기다리는지 바로 알 수 있게 한다.
        let unresolved_dependencies = task
            .depends_on
            .iter()
            .filter_map(|dependency_id| {
                let normalized_dependency_id = dependency_id.trim();
                match task_map.get(normalized_dependency_id) {
                    Some(dependency) if dependency.status.is_dependency_complete() => None,
                    Some(dependency) => Some(format!(
                        "{}({})",
                        normalized_dependency_id,
                        dependency.status.label()
                    )),
                    None => {
                        unreachable!("queue build preflight should validate dependency references")
                    }
                }
            })
            .collect::<Vec<_>>();
        if unresolved_dependencies.is_empty() {
            None
        } else {
            Some(format!(
                "waiting on dependencies: {}",
                unresolved_dependencies.join(", ")
            ))
        }
    }

    fn unresolved_blocker_reason(
        &self,
        task: &TaskDefinition,
        task_map: &HashMap<&str, &TaskDefinition>,
    ) -> Option<String> {
        // blocker는 dependency와 반대로 "clears_blocker" 상태가 되면 해소된다. 완료뿐 아니라
        // not_planned 같은 상태도 blocker 해소로 취급할 수 있어 TaskStatus helper에 위임한다.
        let unresolved_blockers = task
            .blocked_by
            .iter()
            .filter_map(|blocker_id| {
                let normalized_blocker_id = blocker_id.trim();
                match task_map.get(normalized_blocker_id) {
                    Some(blocker) if blocker.status.clears_blocker() => None,
                    Some(blocker) => Some(format!(
                        "{}({})",
                        normalized_blocker_id,
                        blocker.status.label()
                    )),
                    None => {
                        unreachable!("queue build preflight should validate blocker references")
                    }
                }
            })
            .collect::<Vec<_>>();
        if unresolved_blockers.is_empty() {
            None
        } else {
            Some(format!(
                "blocked by tasks: {}",
                unresolved_blockers.join(", ")
            ))
        }
    }
}

fn parse_updated_at_epoch_millis(updated_at: &str) -> Result<i64, chrono::ParseError> {
    // updated_at 문자열은 문서에는 그대로 보존하지만, 정렬 비교는 timezone 차이를 제거한 epoch
    // millis 기준으로 수행한다.
    DateTime::parse_from_rfc3339(updated_at).map(|timestamp| timestamp.timestamp_millis())
}

fn display_reference(value: &str) -> &str {
    if value.is_empty() { "<blank>" } else { value }
}

fn build_rank_reasons(task: &TaskDefinition) -> Vec<String> {
    // rank_reasons는 queue 결과를 보는 사람이 왜 이 작업이 위에 왔는지 추적하기 위한 설명
    // trail이다. 실제 정렬은 QueueCandidate의 필드로 수행하고, 이 값은 UI/보고용으로만 쓴다.
    let mut reasons = vec![
        format!("status={}", task.status.label()),
        format!(
            "combined_priority={} (base {} + delta {})",
            task.combined_priority(),
            task.base_priority,
            task.dynamic_priority_delta
        ),
    ];
    if !task.depends_on.is_empty() {
        reasons.push(format!("dependencies_ready={}", task.depends_on.len()));
    }
    if task.dynamic_priority_delta != 0 && !task.priority_reason.trim().is_empty() {
        reasons.push(format!("priority_reason={}", task.priority_reason.trim()));
    }

    reasons
}

// DirectionDefinition 자체에 UI copy를 얹지 않기 위해 queue 전용 label extension을 둔다.
trait DirectionQueueLabel {
    fn state_label(&self) -> &'static str;
}

impl DirectionQueueLabel for DirectionDefinition {
    fn state_label(&self) -> &'static str {
        match self.state {
            crate::domain::planning::DirectionState::Active => "active",
            crate::domain::planning::DirectionState::Paused => "paused",
            crate::domain::planning::DirectionState::Done => "done",
        }
    }
}

#[cfg(test)]
mod tests;
