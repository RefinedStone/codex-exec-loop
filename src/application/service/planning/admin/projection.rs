use std::collections::BTreeMap;

use super::{
    PlanningAdminDirectionManagementView, PlanningAdminDirectionSummaryView,
    PlanningAdminDirectionTaskView, PlanningAdminDirectionsSummaryView, PlanningAdminDoctorSummary,
    PlanningAdminManagementView, PlanningAdminQueueHeadView, PlanningAdminQueuePreview,
    PlanningAdminQueueTaskView, PlanningAdminRuntimeSummary, PlanningAdminSkippedTaskView,
    PlanningAdminTaskManagementView, PlanningAdminValidationIssueView, PlanningAdminValidationView,
};
use crate::application::service::planning::{
    DirectionsMaintenanceSummary, PlanningApplicationProjection, PlanningApplicationQueueTask,
    PlanningApplicationSkippedTask, PlanningDoctorReport,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionState, PlanningFileKind, PlanningValidationReport,
    PlanningValidationSeverity, PriorityQueueProjection, TaskAuthorityDocument,
};

/*
 * admin projection은 application service 안에 있지만 성격은 adapter에 가깝다. domain 문서와 runtime
 * snapshot은 업무 규칙을 담은 원본이고, 여기서 나가는 값은 route/template/json client가 바로 소비하는
 * 화면 계약이다. enum label, queue preview limit, textarea join 방식 같은 표시 규칙을 이 파일에 모아두면
 * admin handler가 domain 구조를 직접 해석하지 않고 "읽기/쓰기 use case 호출"에만 집중할 수 있다.
 */
pub(super) fn map_management_view(
    directions: &DirectionCatalogDocument,
    task_authority: &TaskAuthorityDocument,
    default_direction_id: &str,
) -> PlanningAdminManagementView {
    // task count는 direction id를 trim한 값으로 합산한다. queue resolution도 공백을 제거한 id로 연결을
    // 판단하므로, admin 화면의 "이 direction에 몇 개 task가 붙었는가"가 runtime 판단과 어긋나지 않는다.
    // 다만 editable row에는 원문 id를 유지해 operator가 실제 문서에 들어 있는 값을 그대로 볼 수 있게 한다.
    let mut tasks_by_direction = BTreeMap::<&str, Vec<PlanningAdminDirectionTaskView>>::new();
    for task in &task_authority.tasks {
        tasks_by_direction
            .entry(task.direction_id.trim())
            .or_default()
            .push(map_direction_task_view(task));
    }
    let task_views = task_authority
        .tasks
        .iter()
        .map(map_task_management_view)
        .collect::<Vec<_>>();

    PlanningAdminManagementView {
        default_direction_id: default_direction_id.to_string(),
        // success criteria와 scope hints는 domain에서는 Vec<String>이지만 admin form에서는 textarea 하나로
        // 편집된다. newline join 정책을 projection에 고정해 두면 form submit parser와 round-trip 표현이 같은
        // 규칙을 공유하고, template 쪽이 domain collection 구조를 알 필요가 없다.
        directions: directions
            .directions
            .iter()
            .map(|direction| {
                let tasks = tasks_by_direction.get(direction.id.trim());
                PlanningAdminDirectionManagementView {
                    tasks: tasks.cloned().unwrap_or_default(),
                    id: direction.id.clone(),
                    title: direction.title.clone(),
                    summary: direction.summary.clone(),
                    success_criteria_text: direction.success_criteria.join("\n"),
                    scope_hints_text: direction.scope_hints.join("\n"),
                    detail_doc_path: direction.detail_doc_path.clone(),
                    state: direction_state_label(direction.state).to_string(),
                    task_count: tasks.map(Vec::len).unwrap_or_default(),
                }
            })
            .collect(),
        // task dependency/blocker 목록도 같은 newline 표현을 쓴다. 순서가 유지되는 text block으로 내보내야
        // operator가 dependency를 재정렬하거나 삭제한 뒤 submit했을 때 mutation parser가 같은 순서를 복원한다.
        tasks: task_views,
    }
}

fn map_direction_task_view(
    task: &crate::domain::planning::TaskDefinition,
) -> PlanningAdminDirectionTaskView {
    PlanningAdminDirectionTaskView {
        id: task.id.clone(),
        title: task.title.clone(),
        status: task.status.label().to_string(),
        base_priority: task.base_priority,
        dynamic_priority_delta: task.dynamic_priority_delta,
        updated_at: task.updated_at.clone(),
    }
}

fn map_task_management_view(
    task: &crate::domain::planning::TaskDefinition,
) -> PlanningAdminTaskManagementView {
    PlanningAdminTaskManagementView {
        id: task.id.clone(),
        direction_id: task.direction_id.clone(),
        title: task.title.clone(),
        description: task.description.clone(),
        status: task.status.label().to_string(),
        base_priority: task.base_priority,
        dynamic_priority_delta: task.dynamic_priority_delta,
        priority_reason: task.priority_reason.clone(),
        depends_on_text: task.depends_on.join("\n"),
        blocked_by_text: task.blocked_by.join("\n"),
        updated_at: task.updated_at.clone(),
    }
}

pub(super) fn map_doctor_report(report: &PlanningDoctorReport) -> PlanningAdminDoctorSummary {
    // doctor report는 workspace 건강 상태를 이미 service 쪽에서 집계한 결과다. projection은 추가 판단을 하지
    // 않고 label/string 형태만 고정해 admin response contract가 domain enum 변경에 직접 흔들리지 않게 한다.
    PlanningAdminDoctorSummary {
        planning_state: report.planning_state().label().to_string(),
        queue_idle_policy: report.queue_idle_policy().map(str::to_string),
        queue_summary: report.queue_summary().map(str::to_string),
        proposal_summary: report.proposal_summary().map(str::to_string),
        health: report.health().map(str::to_string),
        issue: report.issue().map(str::to_string),
        note: report.note().map(str::to_string),
    }
}

pub(super) fn map_application_projection(
    projection: PlanningApplicationProjection,
) -> PlanningAdminRuntimeSummary {
    // projection source가 admin summary와 control surface 사이에서 공유된다. 이 함수는 admin 화면이 필요한
    // 표시 제한과 DTO shape만 책임지고, queue/proposal lane 판단은 이미 application projection에 고정되어 있다.
    PlanningAdminRuntimeSummary {
        workspace_present: projection.workspace_present,
        task_authority_signature: projection.task_authority_signature,
        queue_head_task_signature: projection.queue_head_task_signature,
        preview_status_label: projection.status_label,
        preview_detail: projection.status_detail,
        queue_head: projection.queue_head.map(map_application_queue_head),
        visible_tasks: projection
            .visible_tasks
            .into_iter()
            .take(5)
            .map(map_application_queue_task)
            .collect(),
        proposed_tasks: projection
            .proposed_tasks
            .into_iter()
            .take(5)
            .map(map_application_queue_task)
            .collect(),
        skipped_count: projection.skipped_tasks.len(),
        skipped_tasks: projection
            .skipped_tasks
            .into_iter()
            .take(5)
            .map(map_application_skipped_task)
            .collect(),
    }
}

pub(super) fn map_directions_summary(
    summary: DirectionsMaintenanceSummary,
) -> PlanningAdminDirectionsSummaryView {
    // maintenance summary는 detail doc 존재 여부와 queue-idle prompt 상태 같은 filesystem 검사를 이미 수행한
    // 값이다. admin view는 template branch에 필요한 label과 needs_attention flag만 얹어, UI가 service enum의
    // 내부 의미를 다시 계산하지 않게 한다.
    PlanningAdminDirectionsSummaryView {
        missing_detail_doc_count: summary.missing_detail_doc_count,
        broken_detail_doc_count: summary.broken_detail_doc_count,
        queue_idle_policy: summary.queue_idle_policy.label().to_string(),
        queue_idle_prompt_path: summary.queue_idle_prompt_path,
        queue_idle_prompt_status: summary.queue_idle_prompt_status.label().to_string(),
        parse_error: summary.parse_error,
        directions: summary
            .directions
            .into_iter()
            .map(|direction| PlanningAdminDirectionSummaryView {
                id: direction.id,
                title: direction.title,
                detail_doc_path: direction.detail_doc_path,
                detail_doc_status: direction.detail_doc_status.label().to_string(),
                needs_attention: direction.detail_doc_status.needs_attention(),
            })
            .collect(),
    }
}

pub(super) fn map_validation_report(
    report: &PlanningValidationReport,
) -> PlanningAdminValidationView {
    // validation report 자체가 오류/경고 판정의 authority다. projection은 severity를 재해석하지 않고 count만
    // 중복 계산해 badge 렌더링을 돕는다. 이렇게 하면 admin client가 issue list를 다시 순회하며 error/warning
    // 분류 규칙을 복제하지 않아도 된다.
    let error_count = report
        .issues
        .iter()
        .filter(|issue| issue.severity == PlanningValidationSeverity::Error)
        .count();
    let warning_count = report
        .issues
        .iter()
        .filter(|issue| issue.severity != PlanningValidationSeverity::Error)
        .count();
    PlanningAdminValidationView {
        is_valid: report.is_valid(),
        error_count,
        warning_count,
        issues: report
            .issues
            .iter()
            .map(|issue| PlanningAdminValidationIssueView {
                // severity/file kind는 template class와 JSON client가 의존하는 작은 문자열 계약이다. domain enum
                // 이름을 그대로 노출하지 않고 여기서 lower-case label로 고정해 외부 표시 계약을 안정화한다.
                severity: match issue.severity {
                    PlanningValidationSeverity::Error => "error".to_string(),
                    PlanningValidationSeverity::Warning => "warning".to_string(),
                },
                file_kind: match issue.file_kind {
                    PlanningFileKind::Directions => "directions".to_string(),
                    PlanningFileKind::TaskAuthority => "task_authority".to_string(),
                    PlanningFileKind::ResultOutput => "result_output".to_string(),
                },
                code: issue.code.clone(),
                message: issue.message.clone(),
            })
            .collect(),
    }
}

pub(super) fn map_queue_preview(snapshot: &PriorityQueueProjection) -> PlanningAdminQueuePreview {
    // overview는 전체 queue dump가 아니라 운영자가 지금 볼 compact preview만 필요하다. 다섯 줄 제한은 화면을
    // 스캔 가능한 크기로 유지하고, 실제 handoff 판단에 필요한 rank reason은 queue_head에만 자세히 남긴다.
    PlanningAdminQueuePreview {
        queue_summary: match snapshot.next_task.as_ref() {
            Some(task) => format!("now: {}", task.task_title.trim()),
            None => "queue head: none".to_string(),
        },
        proposal_summary: snapshot
            .proposed_tasks
            .first()
            .map(|task| task.task_title.trim().to_string()),
        queue_head: snapshot
            .next_task
            .as_ref()
            .map(|task| PlanningAdminQueueHeadView {
                task_id: task.task_id.clone(),
                task_title: task.task_title.clone(),
                direction_id: task.direction_id.clone(),
                status: task.status.label().to_string(),
                combined_priority: task.combined_priority,
                updated_at: task.updated_at.clone(),
                rank_reasons: task.rank_reasons.clone(),
            }),
        visible_tasks: snapshot
            .visible_tasks(5)
            .into_iter()
            .map(|task| PlanningAdminQueueTaskView {
                task_id: task.task_id,
                task_title: task.task_title,
                direction_id: task.direction_id,
                status: task.status.label().to_string(),
                combined_priority: task.combined_priority,
                updated_at: task.updated_at,
            })
            .collect(),
        proposed_tasks: snapshot
            .visible_proposed_tasks(5)
            .into_iter()
            .map(|task| PlanningAdminQueueTaskView {
                task_id: task.task_id,
                task_title: task.task_title,
                direction_id: task.direction_id,
                status: task.status.label().to_string(),
                combined_priority: task.combined_priority,
                updated_at: task.updated_at,
            })
            .collect(),
        skipped_count: snapshot.skipped_tasks.len(),
        skipped_tasks: snapshot
            .visible_skipped_tasks(5)
            .into_iter()
            .map(|task| PlanningAdminSkippedTaskView {
                task_id: task.task_id,
                task_title: task.task_title,
                direction_id: task.direction_id,
                status: task.status.label().to_string(),
                reason: task.reason,
            })
            .collect(),
    }
}

fn map_application_queue_head(task: PlanningApplicationQueueTask) -> PlanningAdminQueueHeadView {
    PlanningAdminQueueHeadView {
        task_id: task.task_id,
        task_title: task.task_title,
        direction_id: task.direction_id,
        status: task.status_label,
        combined_priority: task.combined_priority,
        updated_at: task.updated_at,
        rank_reasons: task.rank_reasons,
    }
}

fn map_application_queue_task(task: PlanningApplicationQueueTask) -> PlanningAdminQueueTaskView {
    PlanningAdminQueueTaskView {
        task_id: task.task_id,
        task_title: task.task_title,
        direction_id: task.direction_id,
        status: task.status_label,
        combined_priority: task.combined_priority,
        updated_at: task.updated_at,
    }
}

fn map_application_skipped_task(
    task: PlanningApplicationSkippedTask,
) -> PlanningAdminSkippedTaskView {
    PlanningAdminSkippedTaskView {
        task_id: task.task_id,
        task_title: task.task_title,
        direction_id: task.direction_id,
        status: task.status_label,
        reason: task.reason,
    }
}

fn direction_state_label(state: DirectionState) -> &'static str {
    // The admin form accepts these lower-case labels when mutating directions.
    match state {
        DirectionState::Active => "active",
        DirectionState::Paused => "paused",
        DirectionState::Done => "done",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        map_application_projection, map_directions_summary, map_doctor_report, map_management_view,
        map_queue_preview, map_validation_report,
    };
    use crate::application::service::planning::{
        DirectionsMaintenanceDirectionSummary, DirectionsMaintenanceSummary,
        DirectionsSupportingFileStatus, PlanningApplicationProjection, PlanningDoctorReport,
        PlanningRuntimeProjection,
    };
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
        PLANNING_FORMAT_VERSION, PlanningFileKind, PlanningValidationReport,
        PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, QueueIdleConfig,
        QueueIdlePolicy, TaskActor, TaskAuthorityDocument, TaskDefinition, TaskMutationProvenance,
        TaskStatus,
    };

    #[test]
    fn management_view_maps_direction_task_rows_and_trimmed_task_counts() {
        let directions = DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::Stop,
                prompt_path: String::new(),
            },
            directions: vec![
                direction("dir-active", DirectionState::Active),
                direction("dir-paused", DirectionState::Paused),
                direction("dir-done", DirectionState::Done),
            ],
        };
        let task_authority = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![
                task(
                    "task-1",
                    " dir-active ",
                    vec!["task-a", "task-b"],
                    vec!["task-c"],
                ),
                task("task-2", "dir-paused", Vec::new(), Vec::new()),
            ],
        };

        let view = map_management_view(&directions, &task_authority, "dir-active");

        assert_eq!(view.default_direction_id, "dir-active");
        assert_eq!(
            view.directions
                .iter()
                .map(|direction| (
                    direction.id.as_str(),
                    direction.state.as_str(),
                    direction.task_count
                ))
                .collect::<Vec<_>>(),
            vec![
                ("dir-active", "active", 1),
                ("dir-paused", "paused", 1),
                ("dir-done", "done", 0)
            ]
        );
        assert_eq!(
            view.directions[0]
                .tasks
                .iter()
                .map(|task| (task.id.as_str(), task.status.as_str(), task.base_priority))
                .collect::<Vec<_>>(),
            vec![("task-1", "ready", 10)]
        );
        assert_eq!(
            view.directions[1]
                .tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["task-2"]
        );
        assert_eq!(view.directions[0].success_criteria_text, "done\nverified");
        assert_eq!(view.directions[0].scope_hints_text, "scope-a\nscope-b");
        assert_eq!(view.tasks[0].direction_id, " dir-active ");
        assert_eq!(view.tasks[0].depends_on_text, "task-a\ntask-b");
        assert_eq!(view.tasks[0].blocked_by_text, "task-c");
    }

    #[test]
    fn doctor_report_projection_keeps_path_issue_fields_stable() {
        let report = PlanningDoctorReport::path_issue("workspace path does not exist".to_string());

        let view = map_doctor_report(&report);

        assert_eq!(view.planning_state, "invalid");
        assert_eq!(view.issue.as_deref(), Some("workspace path does not exist"));
        assert_eq!(view.queue_idle_policy, None);
        assert_eq!(view.queue_summary, None);
        assert_eq!(view.proposal_summary, None);
        assert_eq!(view.health, None);
        assert_eq!(view.note, None);
    }

    #[test]
    fn admin_queue_preview_reads_domain_projection_without_reordering() {
        /*
         * admin overview는 queue policy를 다시 계산하지 않는다. domain projection이 준 rank
         * 순서와 proposal lane을 표시용 DTO로 낮추되, 화면 한계 때문에 각 list를 5개로만 자른다.
         */
        let projection = PriorityQueueProjection {
            next_task: Some(queue_task(1, "task-1", "Current task", TaskStatus::Ready)),
            active_tasks: (1..=6)
                .map(|rank| {
                    queue_task(
                        rank,
                        &format!("task-{rank}"),
                        &format!("Active task {rank}"),
                        TaskStatus::Ready,
                    )
                })
                .collect(),
            proposed_tasks: (1..=6)
                .map(|rank| {
                    queue_task(
                        rank,
                        &format!("proposal-{rank}"),
                        &format!("Proposal {rank}"),
                        TaskStatus::Proposed,
                    )
                })
                .collect(),
            skipped_tasks: Vec::new(),
        };

        let preview = map_queue_preview(&projection);

        assert_eq!(preview.queue_summary, "now: Current task");
        assert_eq!(
            preview.queue_head.expect("queue head").rank_reasons,
            vec!["domain-rank=1".to_string()]
        );
        assert_eq!(
            preview
                .visible_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["task-1", "task-2", "task-3", "task-4", "task-5"]
        );
        assert_eq!(
            preview
                .proposed_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "proposal-1",
                "proposal-2",
                "proposal-3",
                "proposal-4",
                "proposal-5"
            ]
        );
    }

    #[test]
    fn admin_queue_preview_reports_idle_when_projection_has_no_head() {
        let projection = PriorityQueueProjection {
            next_task: None,
            active_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
            skipped_tasks: vec![skipped_task("skipped-1", "Skipped", TaskStatus::Blocked)],
        };

        let preview = map_queue_preview(&projection);

        assert_eq!(preview.queue_summary, "queue head: none");
        assert_eq!(preview.proposal_summary, None);
        assert!(preview.queue_head.is_none());
        assert!(preview.visible_tasks.is_empty());
        assert!(preview.proposed_tasks.is_empty());
        assert_eq!(preview.skipped_count, 1);
        assert_eq!(preview.skipped_tasks[0].reason, "blocked by another task");
    }

    #[test]
    fn admin_runtime_summary_uses_application_projection_queue_lanes() {
        let runtime_projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "Planning Context".to_string(),
            "queue ready".to_string(),
            Some("proposal ready".to_string()),
            Some(queue_task(1, "task-1", "Current task", TaskStatus::Ready)),
            PriorityQueueProjection {
                next_task: Some(queue_task(1, "task-1", "Current task", TaskStatus::Ready)),
                active_tasks: vec![
                    queue_task(1, "task-1", "Current task", TaskStatus::Ready),
                    queue_task(2, "task-2", "Next task", TaskStatus::Ready),
                ],
                proposed_tasks: vec![queue_task(
                    1,
                    "proposal-1",
                    "Candidate task",
                    TaskStatus::Proposed,
                )],
                skipped_tasks: vec![skipped_task("skipped-1", "Skipped", TaskStatus::Blocked)],
            },
        )
        .with_test_signatures(Some(42), Some(7));

        let summary = map_application_projection(
            PlanningApplicationProjection::from_runtime_projection(&runtime_projection),
        );

        assert!(summary.workspace_present);
        assert_eq!(summary.task_authority_signature, Some(42));
        assert_eq!(summary.queue_head_task_signature, Some(7));
        assert_eq!(summary.preview_status_label, "ready");
        assert_eq!(summary.preview_detail.as_deref(), Some("queue ready"));
        assert_eq!(
            summary.queue_head.expect("queue head").task_id,
            "task-1".to_string()
        );
        assert_eq!(
            summary
                .visible_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["task-1", "task-2"]
        );
        assert_eq!(summary.proposed_tasks[0].status, "proposed");
        assert_eq!(summary.skipped_count, 1);
        assert_eq!(summary.skipped_tasks[0].reason, "blocked by another task");
    }

    #[test]
    fn admin_runtime_summary_limits_application_projection_lanes() {
        let runtime_projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "Planning Context".to_string(),
            "queue ready".to_string(),
            None,
            Some(queue_task(1, "task-1", "Current task", TaskStatus::Ready)),
            PriorityQueueProjection {
                next_task: Some(queue_task(1, "task-1", "Current task", TaskStatus::Ready)),
                active_tasks: (1..=6)
                    .map(|rank| {
                        queue_task(
                            rank,
                            &format!("task-{rank}"),
                            &format!("Active task {rank}"),
                            TaskStatus::Ready,
                        )
                    })
                    .collect(),
                proposed_tasks: (1..=6)
                    .map(|rank| {
                        queue_task(
                            rank,
                            &format!("proposal-{rank}"),
                            &format!("Proposal {rank}"),
                            TaskStatus::Proposed,
                        )
                    })
                    .collect(),
                skipped_tasks: (1..=6)
                    .map(|rank| {
                        skipped_task(
                            &format!("skipped-{rank}"),
                            &format!("Skipped {rank}"),
                            TaskStatus::Blocked,
                        )
                    })
                    .collect(),
            },
        );

        let summary = map_application_projection(
            PlanningApplicationProjection::from_runtime_projection(&runtime_projection),
        );

        assert_eq!(summary.visible_tasks.len(), 5);
        assert_eq!(summary.proposed_tasks.len(), 5);
        assert_eq!(summary.skipped_count, 6);
        assert_eq!(summary.skipped_tasks.len(), 5);
        assert_eq!(summary.visible_tasks[4].task_id, "task-5");
        assert_eq!(summary.proposed_tasks[4].task_id, "proposal-5");
        assert_eq!(summary.skipped_tasks[4].task_id, "skipped-5");
    }

    #[test]
    fn directions_summary_maps_status_labels_attention_and_parse_error() {
        let summary = DirectionsMaintenanceSummary {
            directions: vec![
                direction_summary(
                    "dir-unset",
                    None,
                    DirectionsSupportingFileStatus::MissingMapping,
                ),
                direction_summary(
                    "dir-ready",
                    Some(".codex-exec-loop/planning/directions/ready.md"),
                    DirectionsSupportingFileStatus::Ready,
                ),
                direction_summary(
                    "dir-broken",
                    Some("../outside.md"),
                    DirectionsSupportingFileStatus::BrokenMapping,
                ),
            ],
            missing_detail_doc_count: 1,
            broken_detail_doc_count: 1,
            queue_idle_policy: QueueIdlePolicy::ReviewAndEnqueue,
            queue_idle_prompt_path: Some(".codex-exec-loop/planning/prompts/queue.md".to_string()),
            queue_idle_prompt_status: DirectionsSupportingFileStatus::BrokenMapping,
            parse_error: Some("directions did not parse".to_string()),
        };

        let view = map_directions_summary(summary);

        assert_eq!(view.missing_detail_doc_count, 1);
        assert_eq!(view.broken_detail_doc_count, 1);
        assert_eq!(view.queue_idle_policy, "review_and_enqueue");
        assert_eq!(
            view.queue_idle_prompt_path.as_deref(),
            Some(".codex-exec-loop/planning/prompts/queue.md")
        );
        assert_eq!(view.queue_idle_prompt_status, "broken");
        assert_eq!(
            view.parse_error.as_deref(),
            Some("directions did not parse")
        );
        assert_eq!(view.directions[0].detail_doc_status, "unset");
        assert!(view.directions[0].needs_attention);
        assert_eq!(view.directions[1].detail_doc_status, "ready");
        assert!(!view.directions[1].needs_attention);
        assert_eq!(view.directions[2].detail_doc_status, "broken");
        assert!(view.directions[2].needs_attention);
    }

    #[test]
    fn validation_report_maps_counts_severity_and_file_kind_labels() {
        let mut report = PlanningValidationReport::new();
        report.push_error(
            PlanningFileKind::Directions,
            "bad_direction",
            "direction is invalid",
        );
        report.push_warning(
            PlanningFileKind::TaskAuthority,
            "task_warning",
            "task is degraded",
        );
        report.push_warning(
            PlanningFileKind::ResultOutput,
            "result_warning",
            "result output is degraded",
        );

        let view = map_validation_report(&report);

        assert!(!view.is_valid);
        assert_eq!(view.error_count, 1);
        assert_eq!(view.warning_count, 2);
        assert_eq!(
            view.issues
                .iter()
                .map(|issue| {
                    (
                        issue.severity.as_str(),
                        issue.file_kind.as_str(),
                        issue.code.as_str(),
                        issue.message.as_str(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    "error",
                    "directions",
                    "bad_direction",
                    "direction is invalid"
                ),
                (
                    "warning",
                    "task_authority",
                    "task_warning",
                    "task is degraded"
                ),
                (
                    "warning",
                    "result_output",
                    "result_warning",
                    "result output is degraded"
                ),
            ]
        );
    }

    fn direction(id: &str, state: DirectionState) -> DirectionDefinition {
        DirectionDefinition {
            id: id.to_string(),
            title: format!("Direction {id}"),
            summary: format!("Summary for {id}"),
            success_criteria: vec!["done".to_string(), "verified".to_string()],
            scope_hints: vec!["scope-a".to_string(), "scope-b".to_string()],
            detail_doc_path: format!(".codex-exec-loop/planning/directions/{id}.md"),
            state,
        }
    }

    fn direction_summary(
        id: &str,
        detail_doc_path: Option<&str>,
        detail_doc_status: DirectionsSupportingFileStatus,
    ) -> DirectionsMaintenanceDirectionSummary {
        DirectionsMaintenanceDirectionSummary {
            id: id.to_string(),
            title: format!("Direction {id}"),
            detail_doc_path: detail_doc_path.map(str::to_string),
            detail_doc_status,
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

    fn queue_task(
        rank: usize,
        task_id: &str,
        task_title: &str,
        status: TaskStatus,
    ) -> PriorityQueueTask {
        PriorityQueueTask {
            rank,
            task_id: task_id.to_string(),
            direction_id: "direction-a".to_string(),
            direction_title: "Direction A".to_string(),
            task_title: task_title.to_string(),
            status,
            combined_priority: 100 - rank as i32,
            updated_at: "2026-05-08T00:00:00Z".to_string(),
            rank_reasons: vec![format!("domain-rank={rank}")],
        }
    }

    fn skipped_task(
        task_id: &str,
        task_title: &str,
        status: TaskStatus,
    ) -> PriorityQueueSkippedTask {
        PriorityQueueSkippedTask {
            task_id: task_id.to_string(),
            task_title: task_title.to_string(),
            direction_id: "direction-a".to_string(),
            status,
            reason: "blocked by another task".to_string(),
        }
    }
}
