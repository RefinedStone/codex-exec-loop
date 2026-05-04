use super::current_timestamp;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeLiveSessionDetailDefaults,
    ParallelModeSlotLeaseSnapshot,
};
use chrono::Utc;
use std::path::Path;
mod store;
#[cfg(test)]
pub(super) use self::store::{agent_session_detail_record_path, read_agent_session_detail_record};
use self::store::{
    push_session_history, update_agent_session_detail_record, write_agent_session_detail_record,
};
pub(super) fn default_validation_summary() -> &'static str {
    "validation summary is not recorded in runtime yet"
}
pub(super) fn default_authority_refresh_outcome() -> &'static str {
    "no official completion has been reported yet"
}

/*
session key는 slot lease와 session detail record를 이어 주는 안정 키이다.
slot id만으로는 같은 slot을 재사용한 과거 이력과 현재 lease를 구분할 수 없고, branch/path만으로는
복구 중 변경될 수 있다. lease가 제공하는 session_key를 공통 키로 쓰면 supervisor,
distributor, store가 같은 agent 실행을 추적할 수 있다.
*/
pub(super) fn lease_session_key(lease: &ParallelModeSlotLeaseSnapshot) -> String {
    lease.session_key()
}

/*
assigned detail은 lease가 만들어진 직후 supervisor에 표시할 최초 세션 기록이다.
아직 validation summary나 official completion 결과가 없으므로 domain projection의 기본 문구를
함께 넣는다. 이후 starting/running/reported_complete 같은 상태 기록 함수들이 이 record를
업데이트하며 history를 누적한다.
*/
pub(super) fn build_assigned_session_detail(
    lease: &ParallelModeSlotLeaseSnapshot,
) -> ParallelModeAgentSessionDetailSnapshot {
    ParallelModeAgentSessionDetailSnapshot::assigned_for_lease(
        lease,
        ParallelModeLiveSessionDetailDefaults {
            validation_summary: default_validation_summary(),
            authority_refresh_outcome: default_authority_refresh_outcome(),
        },
    )
}
pub(super) fn record_assigned_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    // 최초 lease record와 session detail record를 같은 시점에 맞춘다. 이후 상태 전이는
    // 모두 이 record를 update하면서 history만 추가하므로, missing detail 복구의 기준점이다.
    let detail = build_assigned_session_detail(lease);
    write_agent_session_detail_record(planning_authority, workspace_dir, pool_root, &detail)?;
    Ok(detail)
}

/*
thread prepared 기록은 app-server가 thread id를 알려준 시점의 흔적이다.
사용자에게는 아직 "작업 실행 중"보다 이전 단계지만, 문제가 발생했을 때 어떤 Codex thread가
slot 작업에 연결되었는지 추적할 수 있어야 한다. 그래서 state_label은 starting으로 두고
history에 thread id를 남긴다.
*/
pub(super) fn record_thread_prepared_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    thread_id: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.thread_id = Some(thread_id.to_string());
            detail.state_label = "starting".to_string();
            detail.completion_state_label = "in_progress".to_string();
            let summary = format!("thread prepared for the leased session / thread: {thread_id}");
            detail.latest_summary = summary.clone();
            detail.updated_at = timestamp.clone();
            push_session_history(&mut detail, "starting", timestamp, summary);
            detail
        },
    )
}
pub(super) fn record_running_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    // running은 app-server turn이 실제 agent 실행에 들어간 첫 live 상태다.
    // lease에 running_started_at이 있으면 그 시간을 우선해 roster duration과 history 기준을 맞춘다.
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = lease
                .running_started_at
                .clone()
                .unwrap_or_else(current_timestamp);
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "running".to_string();
            detail.completion_state_label = "in_progress".to_string();
            detail.latest_summary = "agent session entered the running state".to_string();
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "running",
                timestamp,
                "agent session entered the running state".to_string(),
            );
            detail
        },
    )
}

/*
reported_complete 업데이트에는 official completion 계약으로 넘길 정보와 supervisor에
보여 줄 정보가 함께 들어 있다. final response summary, validation summary, failure context는
agent가 작업을 어떤 상태로 마쳤는지 설명하고, 이후 ledger refresh와 distributor 단계가 이 record를
이어받는다.
*/
pub(super) struct ReportedCompleteSessionDetailUpdate<'a> {
    pub(super) completed_at: &'a str,
    pub(super) final_response_summary: &'a str,
    pub(super) validation_summary: &'a str,
    pub(super) failure_context: Option<&'a str>,
}
pub(super) fn record_reported_complete_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    update: ReportedCompleteSessionDetailUpdate<'_>,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "reported_complete".to_string();
            detail.completion_state_label = "reported_complete".to_string();
            detail.latest_summary = update.final_response_summary.to_string();
            detail.validation_summary = update.validation_summary.to_string();
            detail.authority_refresh_outcome =
                "completion reported; official ledger refresh is pending".to_string();
            detail.distributor_outcome = None;
            detail.updated_at = update.completed_at.to_string();
            let history_summary = update.failure_context.map_or_else(
                || update.final_response_summary.to_string(),
                |context| format!("{} / context: {context}", update.final_response_summary),
            );
            push_session_history(
                &mut detail,
                "reported_complete",
                update.completed_at.to_string(),
                history_summary,
            );
            detail
        },
    )
}
pub(super) fn record_ledger_refreshing_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    // ledger_refreshing은 hidden official worker가 contract를 잡았다는 runtime 표시다.
    // lease는 아직 Running이므로 distributor가 이 상태만 보고 queue에 넣으면 안 된다.
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "ledger_refreshing".to_string();
            detail.completion_state_label = "ledger_refreshing".to_string();
            detail.latest_summary =
                "completion reported and hidden planning worker is refreshing the ledger"
                    .to_string();
            detail.authority_refresh_outcome =
                "hidden planning worker is refreshing the official task ledger".to_string();
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "ledger_refreshing",
                timestamp,
                "hidden planning worker is refreshing the official task ledger".to_string(),
            );
            detail
        },
    )
}

/*
commit_ready는 official ledger refresh가 agent 결과를 받아들인 직후의 상태이다.
이 record가 생겨야 distributor가 "이 결과를 queue에 넣어도 된다"는 근거를 얻는다. 그래서
authority refresh outcome을 저장하고, distributor_outcome에는 아직 통합 대기 중이라는 문구를
넣어 supervisor가 다음 단계를 드러내게 한다.
*/
pub(super) fn record_commit_ready_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    authority_refresh_outcome: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "commit_ready".to_string();
            detail.completion_state_label = "commit_ready".to_string();
            detail.latest_summary =
                "official ledger refresh accepted the completion report".to_string();
            detail.authority_refresh_outcome = authority_refresh_outcome.trim().to_string();
            detail.distributor_outcome =
                Some("commit-ready result is waiting for distributor integration".to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "commit_ready",
                timestamp,
                "official ledger refresh accepted the completion report".to_string(),
            );
            detail
        },
    )
}
pub(super) fn record_merge_queued_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    // merge_queued는 commit_ready 결과가 distributor queue에 영속화된 뒤의 표시 상태다.
    // 이 기록이 있어 supervisor는 official refresh와 queue enqueue 사이 실패를 구분한다.
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "merge_queued".to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary =
                "commit-ready result accepted into the distributor queue".to_string();
            detail.distributor_outcome = Some(
                "distributor accepted the result and queued it for GitHub delivery".to_string(),
            );
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "merge_queued",
                timestamp,
                "distributor accepted the result and queued it for GitHub delivery".to_string(),
            );
            detail
        },
    )
}
pub(super) fn record_pushing_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    // pushing/pr/merge/integrating wrapper들은 delivery 단계명을 domain projection에 고정한다.
    // caller는 summary만 넘기고, 공통 history/label 갱신은 아래 helper가 맡는다.
    record_distributor_progress_session_detail(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        "pushing",
        summary,
        false,
    )
}
pub(super) fn record_pr_pending_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    record_distributor_progress_session_detail(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        "pr_pending",
        summary,
        false,
    )
}
pub(super) fn record_merge_pending_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    record_distributor_progress_session_detail(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        "merge_pending",
        summary,
        false,
    )
}
pub(super) fn record_integrating_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    summary: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    record_distributor_progress_session_detail(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        "integrating",
        summary,
        true,
    )
}

/*
distributor progress 기록은 pushing, pr_pending, merge_pending, integrating처럼
delivery queue 내부의 세부 단계를 같은 함수로 처리한다. completion_state_label은
merge_queued로 유지해 큰 흐름에서는 "통합 큐 처리 중"으로 묶고, state_label과 latest_summary로
구체적인 현재 단계를 보여 준다.

integrating 단계는 같은 상태에서 summary가 자주 바뀔 수 있어 마지막 history entry를 교체할 수
있다. 이렇게 하지 않으면 cherry-pick/rebase 진행 중 같은 상태의 noisy history가 과도하게
쌓인다.
*/
fn record_distributor_progress_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    state_label: &'static str,
    summary: &str,
    replace_latest_same_state_history: bool,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let summary = summary.trim().to_string();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = state_label.to_string();
            detail.completion_state_label = "merge_queued".to_string();
            detail.latest_summary = summary.clone();
            detail.distributor_outcome = Some(summary.clone());
            detail.updated_at = timestamp.clone();
            if replace_latest_same_state_history
                && let Some(last_entry) = detail.history.last_mut()
                && last_entry.state_label == state_label
            {
                last_entry.timestamp = timestamp;
                last_entry.summary = summary;
            } else {
                push_session_history(&mut detail, state_label, timestamp, summary);
            }
            detail
        },
    )
}
pub(super) fn record_distributor_failed_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    failure_detail: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    // distributor 실패는 official completion 실패와 다르다. ledger는 이미 통과했지만
    // push, PR, integration 중 멈춘 상태라 distributor_outcome에 실패 사유를 둔다.
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "failed".to_string();
            detail.completion_state_label = "failed".to_string();
            detail.latest_summary = "distributor delivery failed".to_string();
            detail.distributor_outcome = Some(failure_detail.trim().to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "failed",
                timestamp,
                failure_detail.trim().to_string(),
            );
            detail
        },
    )
}
pub(super) fn record_official_completion_failed_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    failure_detail: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    // official completion 실패는 distributor queue로 넘어가기 전의 실패다.
    // authority_refresh_outcome을 실패 원인으로 덮어 이후 자동 통합 대상에서 빠진 이유를 남긴다.
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "failed".to_string();
            detail.completion_state_label = "failed".to_string();
            detail.latest_summary = "official completion refresh failed".to_string();
            detail.authority_refresh_outcome = failure_detail.trim().to_string();
            detail.distributor_outcome = Some(
                "not queued for distributor integration because official refresh failed"
                    .to_string(),
            );
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "failed",
                timestamp,
                failure_detail.trim().to_string(),
            );
            detail
        },
    )
}

pub(super) fn record_official_completion_recovery_needed_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    recovery_detail: &str,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    // stale ledger_refreshing은 오류가 아니라 orphaned intermediate state다.
    // 자동 로딩은 멈추되 실제 작업 실패로 오인하지 않도록 별도 복구 상태로 남긴다.
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "official_refresh_recovery_needed".to_string();
            detail.completion_state_label = "official_refresh_recovery_needed".to_string();
            detail.latest_summary = "official completion refresh needs recovery".to_string();
            detail.authority_refresh_outcome = recovery_detail.trim().to_string();
            detail.distributor_outcome = Some(
                "not queued for distributor integration because official refresh needs recovery"
                    .to_string(),
            );
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "official_refresh_recovery_needed",
                timestamp,
                recovery_detail.trim().to_string(),
            );
            detail
        },
    )
}

/*
cleanup pending 기록은 "변경은 baseline에 들어갔고 slot 반환만 남았다"는 상태를
history에 두 단계로 남긴다. 먼저 merged를 기록해 completion feed의 merged 항목이 채워지게
하고, 이어 cleanup_pending을 기록해 slot이 아직 idle이 아니라는 운영 상태를 보존한다.
*/
pub(super) fn record_cleanup_pending_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "cleanup_pending".to_string();
            detail.completion_state_label = "merged".to_string();
            detail.latest_summary =
                "agent branch is merged into prerelease and awaiting slot cleanup".to_string();
            detail.distributor_outcome = Some(
                "branch is merged into prerelease and the slot is awaiting cleanup".to_string(),
            );
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "merged",
                timestamp.clone(),
                "branch is integrated into prerelease".to_string(),
            );
            push_session_history(
                &mut detail,
                "cleanup_pending",
                timestamp,
                "slot is waiting for cleanup before it can be reused".to_string(),
            );
            detail
        },
    )
}

/*
cleaned 기록은 slot lifecycle의 정상 종료점이다. distributor delivery나 startup
failure cleanup이 slot을 baseline으로 돌려놓으면 이 상태를 남겨 supervisor가 과거 세션을
"정리 완료"로 보여 주고, roster에서는 더 이상 live lease로 취급하지 않게 된다.
*/
pub(super) fn record_cleaned_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "cleaned".to_string();
            detail.completion_state_label = "cleaned".to_string();
            detail.latest_summary =
                "merged session cleaned up and the slot returned to the idle pool".to_string();
            detail.distributor_outcome =
                Some("branch merged into prerelease and the slot returned to idle".to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "cleaned",
                timestamp,
                "slot cleaned and returned to the idle pool".to_string(),
            );
            detail
        },
    )
}

/*
startup failure 기록은 turn이 running 상태에 들어가기 전에 lease가 해제된 경우를
나타낸다. 이 경우 distributor로 보낼 결과가 없으므로 failed와 cleaned history를 함께 남겨
"실행 실패했고 slot은 회수됨"을 분명히 한다.
*/
pub(super) fn record_failed_start_session_detail(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String> {
    update_agent_session_detail_record(
        planning_authority,
        workspace_dir,
        pool_root,
        lease,
        |current| {
            let timestamp = current_timestamp();
            let mut detail = current.unwrap_or_else(|| build_assigned_session_detail(lease));
            detail.state_label = "failed".to_string();
            detail.completion_state_label = "aborted".to_string();
            detail.latest_summary =
                "launch failed before the session reached the running state".to_string();
            detail.distributor_outcome =
                Some("not queued for distributor work; slot returned to idle".to_string());
            detail.updated_at = timestamp.clone();
            push_session_history(
                &mut detail,
                "failed",
                timestamp.clone(),
                "launch failed before the session reached the running state".to_string(),
            );
            push_session_history(
                &mut detail,
                "cleaned",
                timestamp,
                "slot cleaned and returned to the idle pool after launch failure".to_string(),
            );
            detail
        },
    )
}

/*
running elapsed label은 supervisor roster에서 live agent가 얼마나 오래 실행 중인지
보여 주는 표시값이다. 저장된 RFC3339 timestamp를 UTC로 파싱하고 현재 시각과 비교한다.
파싱 실패는 None으로 두어 화면이 잘못된 시간을 억지로 표시하지 않게 한다.
*/
pub(super) fn format_elapsed_label_from_timestamp(timestamp: &str) -> Option<String> {
    let started_at = chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .with_timezone(&Utc);
    let elapsed_seconds = Utc::now()
        .signed_duration_since(started_at)
        .num_seconds()
        .max(0);

    Some(format_elapsed_seconds(elapsed_seconds))
}
fn format_elapsed_seconds(elapsed_seconds: i64) -> String {
    // roster 폭을 아끼기 위해 가장 큰 두 단위까지만 표시한다. 1분 미만은 초 단위를
    // 보존해 방금 시작한 agent와 멈춘 agent를 구분하기 쉽게 한다.
    if elapsed_seconds < 60 {
        return format!("{elapsed_seconds}s");
    }
    if elapsed_seconds < 60 * 60 {
        let minutes = elapsed_seconds / 60;
        let seconds = elapsed_seconds % 60;
        if seconds == 0 {
            return format!("{minutes}m");
        }
        return format!("{minutes}m {seconds}s");
    }
    if elapsed_seconds < 60 * 60 * 24 {
        let hours = elapsed_seconds / (60 * 60);
        let minutes = (elapsed_seconds % (60 * 60)) / 60;
        return format!("{hours}h {minutes}m");
    }
    let days = elapsed_seconds / (60 * 60 * 24);
    let hours = (elapsed_seconds % (60 * 60 * 24)) / (60 * 60);
    format!("{days}d {hours}h")
}
#[cfg(test)]
mod tests {
    use super::format_elapsed_seconds;
    #[test]
    fn elapsed_label_keeps_seconds_for_short_running_agents() {
        assert_eq!(format_elapsed_seconds(65), "1m 5s");
        assert_eq!(format_elapsed_seconds(120), "2m");
    }
}
