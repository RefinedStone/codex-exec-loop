use super::super::supervisor::selected_runtime_session_detail;
use super::super::{
    DISTRIBUTOR_INTEGRATION_BRANCH, PoolRuntimeContext, current_branch_name,
    inspect_slot_git_status, short_sha,
};
use super::{ParallelModeDistributorQueueRecord, matching_lease_for_queue_record};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeEventRecord;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeCompletionFeedEntry,
    ParallelModeDistributorSnapshot, ParallelModeOrchestratorStatus, ParallelModeQueueItemState,
    ParallelModeRuntimeEventFeedEntry, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};

/*
distributor snapshot은 durable queue와 session history를 TUI용 읽기 모델로
바꾼다. 활성 queue record가 있으면 그 queue head가 화면의 중심이다. 활성 record가
없으면 최근 session detail을 살펴 commit_ready, ledger_refreshing 같은 완료 파이프라인의
마지막 의미 있는 상태를 보여 준다.

이 함수가 `selected_runtime_session_detail`을 재사용하는 이유는 supervisor detail과
distributor 패널이 같은 "현재 가장 볼 만한 세션" 기준을 공유해야 하기 때문이다.
*/
pub(super) fn build_distributor_snapshot_from_context(
    context: &PoolRuntimeContext,
) -> ParallelModeDistributorSnapshot {
    let history = context.session_details.clone();
    let queue_records = context.distributor_queue_records.clone();
    let runtime_event_feed = build_runtime_event_feed(&context.runtime_events);
    /*
    queue_items는 화면에 직접 나갈 active record 목록이다. done/idle record를
    여기서 제외해야 completion feed와 queue table이 서로 다른 질문에 답한다:
    queue table은 "지금 막힌 head와 그 뒤 대기열", feed는 "최근 완료 파이프라인 흔적"이다.
    */
    let queue_items = queue_records
        .iter()
        .filter(|record| record.queue_state.is_active())
        .map(ParallelModeDistributorQueueRecord::display_item)
        .collect::<Vec<_>>();
    let completion_feed = build_distributor_completion_feed(&history);
    if let Some(queue_head) = active_distributor_queue_head(&queue_records) {
        /*
        active queue head가 있으면 session detail보다 우선한다. distributor는 head 하나만
        전진시킬 수 있고, 뒤 queue item은 head 상태에 종속되므로 snapshot의 headline,
        blocked detail, rebase provenance, orchestrator status를 모두 같은 record에서 뽑는다.
        */
        return ParallelModeDistributorSnapshot::new(
            queue_items,
            completion_feed,
            queue_head.queue_state.label(),
            queue_head.integration_note.clone(),
        )
        .with_head_blocked_detail(blocked_head_detail(queue_head))
        .with_head_rebase_provenance(rebase_provenance_label(queue_head))
        .with_orchestrator_status(build_orchestrator_status(context, queue_head))
        .with_runtime_event_feed(runtime_event_feed);
    }
    let Some(detail) = selected_runtime_session_detail(context, &history, &queue_records) else {
        return build_placeholder_distributor_snapshot(
            ParallelModeQueueItemState::Idle.label(),
            "no distributor queue items are waiting",
        )
        .with_orchestrator_status(build_idle_orchestrator_status(context))
        .with_runtime_event_feed(runtime_event_feed);
    };
    /*
    queue가 비었어도 마지막 session detail은 중요하다. agent가 reported_complete를 냈지만
    ledger refresh가 아직 도는 중이거나, commit_ready가 만들어졌지만 enqueue 전이라면
    operator는 queue table이 비어 있는 이유를 알아야 한다.
    */
    let (head_summary, note) = match detail.state_label.as_str() {
        "reported_complete" => ("reported".to_string(), detail.latest_summary.clone()),
        "ledger_refreshing" => (
            "ledger refreshing".to_string(),
            detail.authority_refresh_outcome.clone(),
        ),
        "commit_ready" => (
            "official".to_string(),
            detail.distributor_outcome.clone().unwrap_or_else(|| {
                "commit-ready result is waiting for distributor enqueue".to_string()
            }),
        ),
        "failed" if detail_has_history_state(&detail, "reported_complete") => (
            "blocked".to_string(),
            detail.authority_refresh_outcome.clone(),
        ),
        "official_refresh_recovery_needed" => (
            "recovery needed".to_string(),
            detail.authority_refresh_outcome.clone(),
        ),
        _ => (
            ParallelModeQueueItemState::Idle.label().to_string(),
            "no distributor queue items are waiting".to_string(),
        ),
    };
    ParallelModeDistributorSnapshot::new(queue_items, completion_feed, head_summary, note)
        .with_head_rebase_provenance(history_rebase_provenance(&detail))
        .with_orchestrator_status(build_idle_orchestrator_status(context))
        .with_runtime_event_feed(runtime_event_feed)
}

fn build_runtime_event_feed(
    events: &[PlanningAuthorityRuntimeEventRecord],
) -> Vec<ParallelModeRuntimeEventFeedEntry> {
    events
        .iter()
        .map(|event| {
            ParallelModeRuntimeEventFeedEntry::new(
                event.sequence,
                event.event_kind.clone(),
                event.projection_kind.clone(),
                event.projection_key.clone(),
                event.observed_planning_revision,
                event.summary.clone(),
                event.recorded_at.clone(),
            )
        })
        .collect()
}

/*
orchestrator status는 queue head 하나가 왜 진행 중이거나 막혀 있는지를
작업자 관점으로 압축한 진단 정보이다. active record 개수로 뒤 queue item이 head에
막혀 있는지 표시하고, matching lease를 찾아 slot return 대기 사유까지 함께 보여 준다.
이 값은 단순 queue item 목록보다 "다음에 무엇을 복구해야 하는가"에 초점을 둔다.
*/
fn build_orchestrator_status(
    context: &PoolRuntimeContext,
    queue_head: &ParallelModeDistributorQueueRecord,
) -> ParallelModeOrchestratorStatus {
    let active_record_count = context
        .distributor_queue_records
        .iter()
        .filter(|record| record.queue_state.is_active())
        .count();
    let matching_lease = matching_lease_for_queue_record(context, queue_head);

    ParallelModeOrchestratorStatus {
        /*
        queue_head label은 agent/task/state만 남긴다. branch, commit, integration note는
        queue item row에 이미 있으므로 barrier line에서는 어떤 work unit이 제어권을
        쥐고 있는지만 빠르게 읽히게 한다.
        */
        queue_head: format!(
            "{} / {} / {}",
            queue_head.agent_id,
            queue_head.task_id,
            queue_head.queue_state.label()
        ),
        barrier_state: orchestrator_barrier_state(queue_head, active_record_count),
        blocked_reason: blocked_head_detail(queue_head).or_else(|| {
            queue_head
                .recovery_note
                .clone()
                .filter(|note| !note.trim().is_empty())
        }),
        conflict_files: queue_head.conflict_files.clone(),
        held_queue_count: active_record_count.saturating_sub(1),
        integration_worktree_readiness: inspect_integration_worktree_readiness(context),
        slot_return_wait_reason: slot_return_wait_reason(queue_head, matching_lease),
    }
}
fn build_idle_orchestrator_status(context: &PoolRuntimeContext) -> ParallelModeOrchestratorStatus {
    let mut status = ParallelModeOrchestratorStatus::idle();
    /*
    idle orchestrator도 integration worktree readiness를 덮어쓴다. queue가 비어 있는
    동안 dirty branch를 먼저 고치면 다음 commit_ready가 들어왔을 때 즉시 처리할 수 있다.
    */
    status.integration_worktree_readiness = inspect_integration_worktree_readiness(context);
    status
}
fn orchestrator_barrier_state(
    queue_head: &ParallelModeDistributorQueueRecord,
    active_record_count: usize,
) -> String {
    /*
    barrier label은 queue state를 그대로 반복하지 않는다. blocked/failed는 recovery가
    필요하다는 뜻이고, cleaning은 slot 반환이 남았다는 뜻이며, 여러 active record가
    있으면 head가 뒤 작업을 의도적으로 잡고 있음을 표시한다.
    */
    match queue_head.queue_state {
        ParallelModeQueueItemState::Blocked | ParallelModeQueueItemState::Failed => {
            "blocked".to_string()
        }
        ParallelModeQueueItemState::Cleaning => "slot return".to_string(),
        _ if active_record_count > 1 => {
            format!(
                "head {} holds later queue items",
                queue_head.queue_state.label()
            )
        }
        _ => format!("head {}", queue_head.queue_state.label()),
    }
}

/*
integration worktree readiness는 queue가 비어 있을 때도 계속 보여 줘야 하는
운영 상태이다. queue item이 없더라도 integration branch가 아닌 곳에 있거나 로컬 변경이
남아 있으면 다음 delivery tick이 막힌다. snapshot에서 미리 드러내면 사용자가 queue가
생기기 전에 작업대를 정리할 수 있다.
*/
fn inspect_integration_worktree_readiness(context: &PoolRuntimeContext) -> String {
    let repo_root = context.canonical_repo_root.as_path();
    let Some(branch_name) = current_branch_name(repo_root) else {
        return "unknown: branch could not be inspected".to_string();
    };
    if branch_name != DISTRIBUTOR_INTEGRATION_BRANCH {
        return format!(
            "blocked: expected `{DISTRIBUTOR_INTEGRATION_BRANCH}` but checked out `{branch_name}`"
        );
    }
    let Some(status) = inspect_slot_git_status(repo_root) else {
        return "unknown: git status could not be inspected".to_string();
    };
    if status.is_ready_for_integration() {
        format!("ready: {DISTRIBUTOR_INTEGRATION_BRANCH} worktree clean")
    } else {
        format!("blocked: {}", status.detail_label())
    }
}

/*
slot return wait reason은 queue head와 lease state를 함께 봐야만 알 수 있는
메시지이다. queue가 Cleaning이면 cleanup 자체가 남은 것이고, queue가 아직 통합 단계이면
Running lease가 유지되는 것이 정상이다. 이 설명이 없으면 사용자는 슬롯이 오래 점유된
상태를 누수로 오해할 수 있다.
*/
fn slot_return_wait_reason(
    queue_head: &ParallelModeDistributorQueueRecord,
    matching_lease: Option<&ParallelModeSlotLeaseSnapshot>,
) -> Option<String> {
    let lease = matching_lease?;
    match (queue_head.queue_state, lease.state) {
        (ParallelModeQueueItemState::Cleaning, ParallelModeSlotLeaseState::CleanupPending) => {
            Some(format!(
                "slot `{}` is waiting for cleanup to return idle",
                lease.slot_id
            ))
        }
        (_, ParallelModeSlotLeaseState::CleanupPending) => Some(format!(
            "slot `{}` is waiting for distributor cleanup",
            lease.slot_id
        )),
        (_, ParallelModeSlotLeaseState::Running)
            if matches!(
                queue_head.queue_state,
                ParallelModeQueueItemState::Queued
                    | ParallelModeQueueItemState::Pushing
                    | ParallelModeQueueItemState::PrPending
                    | ParallelModeQueueItemState::MergePending
                    | ParallelModeQueueItemState::Integrating
            ) =>
        {
            Some(format!(
                "slot `{}` stays running until the queue head is integrated",
                lease.slot_id
            ))
        }
        _ => None,
    }
}

/*
queue head helper들은 snapshot이 delivery 상태를 새로 계산하지 않도록 경계를 세운다.
queue record가 이미 가진 state, note, original commit provenance만 읽어 화면 문구로
투영하고, side effect가 필요한 recovery나 git 조작은 delivery/reconcile 경로에 남겨 둔다.
*/
fn active_distributor_queue_head(
    queue_records: &[ParallelModeDistributorQueueRecord],
) -> Option<&ParallelModeDistributorQueueRecord> {
    queue_records
        .iter()
        .find(|record| record.queue_state.is_active())
}
fn blocked_head_detail(record: &ParallelModeDistributorQueueRecord) -> Option<String> {
    /*
    blocked detail은 Blocked 상태에만 붙인다. 같은 integration_note라도 queued/pending
    상태에서는 진행 설명일 수 있으므로, popup의 "blocked head" copy로 승격하지 않는다.
    */
    (record.queue_state == ParallelModeQueueItemState::Blocked)
        .then(|| record.integration_note.clone())
}

fn rebase_provenance_label(record: &ParallelModeDistributorQueueRecord) -> Option<String> {
    /*
    original_commit_sha가 없으면 현재 commit을 원본으로 간주한다. 이렇게 하면 오래된
    queue record도 provenance 없는 정상 record로 표시되고, 실제 rewrite가 있었던 경우만
    short sha 전후 관계를 드러낸다.
    */
    let original_commit_sha = record
        .original_commit_sha
        .as_deref()
        .filter(|commit| !commit.trim().is_empty())
        .unwrap_or(record.commit_sha.as_str());
    (original_commit_sha != record.commit_sha).then(|| {
        format!(
            "rebased {} -> {} onto `{DISTRIBUTOR_INTEGRATION_BRANCH}`",
            short_sha(original_commit_sha),
            short_sha(&record.commit_sha)
        )
    })
}

/*
queue record가 이미 사라진 idle 화면에서도 마지막 rebase provenance는 session history에
남아 있을 수 있다. snapshot은 이 history fallback을 써서 "방금 통합된 작업이 rebase를
거쳤는가"를 유지하되, 오래된 임의 history가 아니라 가장 최근 integrating summary만 사용한다.
*/
fn history_rebase_provenance(detail: &ParallelModeAgentSessionDetailSnapshot) -> Option<String> {
    detail
        .history
        .iter()
        .rev()
        .find(|entry| entry.state_label == "integrating" && entry.summary.starts_with("rebased "))
        .map(|entry| entry.summary.clone())
}
fn detail_has_history_state(
    detail: &ParallelModeAgentSessionDetailSnapshot,
    state_label: &str,
) -> bool {
    detail
        .history
        .iter()
        .any(|entry| entry.state_label == state_label)
}

/*
completion feed는 병렬 작업의 큰 흐름을 다섯 단계로 요약한다. reported는
agent가 결과를 냈는지, ledger refreshing은 official completion이 돌고 있는지, official은
commit-ready 결과가 생겼는지, merge queued는 distributor가 잡은 일이 있는지, merged는
integration branch에 실제로 들어갔는지를 보여 준다. 각 항목은 session history에서 가장
최근 요약을 골라 화면에 올린다.
*/
fn build_distributor_completion_feed(
    history: &[ParallelModeAgentSessionDetailSnapshot],
) -> Vec<ParallelModeCompletionFeedEntry> {
    vec![
        ParallelModeCompletionFeedEntry::new(
            "reported",
            latest_history_summary_across_records(history, &["reported_complete"])
                .unwrap_or_else(|| "no agent results reported yet".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "ledger refreshing",
            latest_history_summary_across_records(history, &["ledger_refreshing"])
                .unwrap_or_else(|| "no official refresh workers are active".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "official",
            latest_history_summary_across_records(history, &["commit_ready"])
                .unwrap_or_else(|| "nothing is queued for merge".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "merge queued",
            latest_history_summary_across_records(
                history,
                &[
                    "merge_queued",
                    "pushing",
                    "pr_pending",
                    "merge_pending",
                    "integrating",
                ],
            )
            .unwrap_or_else(|| "no distributor queue items are waiting".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "merged",
            latest_history_summary_across_records(history, &["merged", "cleaned"]).unwrap_or_else(
                || format!("nothing has been integrated into {DISTRIBUTOR_INTEGRATION_BRANCH} yet"),
            ),
        ),
    ]
}
fn latest_history_summary_across_records(
    history: &[ParallelModeAgentSessionDetailSnapshot],
    state_labels: &[&str],
) -> Option<String> {
    /*
    session history는 여러 agent에서 섞여 들어오므로 timestamp가 feed의 주 정렬 기준이다.
    timestamp가 같은 테스트 fixture나 빠른 연속 이벤트에서는 summary를 tie-breaker로 써
    snapshot 출력을 결정적으로 만든다.
    */
    history
        .iter()
        .flat_map(|detail| detail.history.iter())
        .filter(|entry| state_labels.contains(&entry.state_label.as_str()))
        .max_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.summary.cmp(&right.summary))
        })
        .map(|entry| entry.summary.clone())
}

/*
placeholder snapshot은 병렬 모드가 꺼졌거나 아직 queue/session evidence가 없을 때 쓰는
빈 읽기 모델이다. adapter가 optional field와 빈 list를 직접 해석하지 않도록 completion
feed의 단계 이름과 empty copy를 여기서 같은 순서로 고정한다.
*/
pub(super) fn build_placeholder_distributor_snapshot(
    head_summary: impl Into<String>,
    note: impl Into<String>,
) -> ParallelModeDistributorSnapshot {
    ParallelModeDistributorSnapshot::new(
        Vec::new(),
        vec![
            ParallelModeCompletionFeedEntry::new("reported", "no agent results reported yet"),
            ParallelModeCompletionFeedEntry::new(
                "ledger refreshing",
                "no official refresh workers are active",
            ),
            ParallelModeCompletionFeedEntry::new("official", "nothing is queued for merge"),
            ParallelModeCompletionFeedEntry::new(
                "merge queued",
                "no distributor queue items are waiting",
            ),
            ParallelModeCompletionFeedEntry::new(
                "merged",
                format!("nothing has been integrated into {DISTRIBUTOR_INTEGRATION_BRANCH} yet"),
            ),
        ],
        head_summary,
        note,
    )
}
