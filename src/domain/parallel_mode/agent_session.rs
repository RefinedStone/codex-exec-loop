use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};

/*
 * agent_session.rs는 parallel-mode runtime의 lease 중심 상태를 operator-facing session/roster snapshot으로
 * 변환하는 도메인 projection이다. lease는 slot ownership과 worktree/branch 사실을 담고, session detail은
 * worker progress, validation, distributor 결과처럼 시간이 지나며 누적되는 설명을 담는다. 이 파일은 두
 * 출처를 합쳐 supervisor popup과 roster list가 같은 vocabulary를 쓰게 한다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeAgentRosterEntry {
    // roster row에서 agent를 식별하는 display id다.
    pub agent_id: String,
    // agent가 맡은 planning task title이다.
    pub task_title: String,
    // pool slot id다. 같은 agent/task라도 slot handoff를 구분한다.
    pub slot_id: String,
    // worker가 push/PR을 만드는 branch name이다.
    pub branch_name: String,
    // lease state와 session detail을 합친 operator-facing state label이다.
    pub state_label: String,
    // elapsed time 또는 delivery phase를 담는 compact label이다.
    pub duration_label: String,
    // roster에서 마지막 의미 있는 진행 상태를 보여 주는 한 줄 요약이다.
    pub latest_summary: String,
}

impl ParallelModeAgentRosterEntry {
    // roster entry는 UI DTO라 여러 display field를 명시적으로 받는다.
    pub fn new(
        agent_id: impl Into<String>,
        task_title: impl Into<String>,
        slot_id: impl Into<String>,
        branch_name: impl Into<String>,
        state_label: impl Into<String>,
        duration_label: impl Into<String>,
        latest_summary: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            task_title: task_title.into(),
            slot_id: slot_id.into(),
            branch_name: branch_name.into(),
            state_label: state_label.into(),
            duration_label: duration_label.into(),
            latest_summary: latest_summary.into(),
        }
    }

    pub fn counts_as_active(&self) -> bool {
        self.state_label != "failed"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeAgentSessionHistoryEntry {
    // 이 history point의 lifecycle label이다.
    pub state_label: String,
    // event timestamp string이다. store/adapter가 그대로 persistence한다.
    pub timestamp: String,
    // event에 대한 operator-facing 설명이다.
    pub summary: String,
}

impl ParallelModeAgentSessionHistoryEntry {
    // store/update code가 history append를 같은 shape로 만들게 하는 생성자다.
    pub fn new(
        state_label: impl Into<String>,
        timestamp: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            state_label: state_label.into(),
            timestamp: timestamp.into(),
            summary: summary.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModeAgentSessionDetailSnapshot {
    // lease와 persisted detail을 join하는 stable key다.
    pub session_key: String,
    pub agent_id: String,
    // planning task identity와 title은 completion/distributor 단계에서도 계속 보여 준다.
    pub task_id: String,
    pub task_title: String,
    pub slot_id: String,
    // app-server thread id가 생기면 assigned가 starting/running detail로 진전되었음을 알 수 있다.
    pub thread_id: Option<String>,
    pub worktree_path: String,
    pub branch_name: String,
    // lease가 처음 잡힌 시간이다. running timestamp가 없을 때 recency fallback으로 쓴다.
    pub lease_started_at: String,
    // session detail의 현재 lifecycle label이다.
    pub state_label: String,
    // official completion pipeline 관점의 completion state다.
    pub completion_state_label: String,
    pub latest_summary: String,
    // worker가 보고한 검증/테스트 요약이다.
    pub validation_summary: String,
    // planning authority refresh 결과다.
    pub authority_refresh_outcome: String,
    // distributor가 push/PR/merge/integration 단계에서 남긴 outcome이다.
    pub distributor_outcome: Option<String>,
    pub history: Vec<ParallelModeAgentSessionHistoryEntry>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelModeLiveSessionDetailDefaults<'a> {
    // live lease만 있고 persisted detail이 비어 있을 때 채울 validation fallback이다.
    pub validation_summary: &'a str,
    // live lease만 있고 authority refresh 결과가 없을 때 채울 fallback이다.
    pub authority_refresh_outcome: &'a str,
}

impl ParallelModeAgentSessionDetailSnapshot {
    #[allow(clippy::too_many_arguments)]
    // persisted/session detail schema와 거의 1:1이라 explicit constructor가 field mapping을 숨기지 않는다.
    pub fn new(
        session_key: impl Into<String>,
        agent_id: impl Into<String>,
        task_id: impl Into<String>,
        task_title: impl Into<String>,
        slot_id: impl Into<String>,
        thread_id: Option<String>,
        worktree_path: impl Into<String>,
        branch_name: impl Into<String>,
        lease_started_at: impl Into<String>,
        state_label: impl Into<String>,
        completion_state_label: impl Into<String>,
        latest_summary: impl Into<String>,
        validation_summary: impl Into<String>,
        authority_refresh_outcome: impl Into<String>,
        distributor_outcome: Option<String>,
        history: Vec<ParallelModeAgentSessionHistoryEntry>,
        updated_at: impl Into<String>,
    ) -> Self {
        Self {
            session_key: session_key.into(),
            agent_id: agent_id.into(),
            task_id: task_id.into(),
            task_title: task_title.into(),
            slot_id: slot_id.into(),
            thread_id,
            worktree_path: worktree_path.into(),
            branch_name: branch_name.into(),
            lease_started_at: lease_started_at.into(),
            state_label: state_label.into(),
            completion_state_label: completion_state_label.into(),
            latest_summary: latest_summary.into(),
            validation_summary: validation_summary.into(),
            authority_refresh_outcome: authority_refresh_outcome.into(),
            distributor_outcome,
            history,
            updated_at: updated_at.into(),
        }
    }

    // slot lease가 막 생성되었고 아직 worker thread가 attach되지 않은 초기 detail을 만든다.
    pub fn assigned_for_lease(
        lease: &ParallelModeSlotLeaseSnapshot,
        defaults: ParallelModeLiveSessionDetailDefaults<'_>,
    ) -> Self {
        Self::new(
            lease.session_key(),
            lease.agent_id.clone(),
            lease.task_id.clone(),
            lease.task_title.clone(),
            lease.slot_id.clone(),
            None,
            lease.worktree_path.clone(),
            lease.branch_name.clone(),
            lease.leased_at.clone(),
            "assigned",
            "in_progress",
            "slot lease acquired and branch reserved for launch",
            defaults.validation_summary,
            defaults.authority_refresh_outcome,
            None,
            vec![ParallelModeAgentSessionHistoryEntry::new(
                "assigned",
                lease.leased_at.clone(),
                "slot lease acquired and branch reserved for launch",
            )],
            lease.leased_at.clone(),
        )
    }

    /*
     * live_for_lease는 persisted detail을 현재 lease 사실로 재수화한다. session store가 오래된 branch,
     * slot, state label을 가지고 있어도 lease snapshot이 source-of-truth인 필드는 항상 lease 값으로 덮는다.
     * 반대로 validation/distributor/history 같은 누적 설명은 기존 detail을 최대한 보존한다.
     */
    pub fn live_for_lease(
        lease: &ParallelModeSlotLeaseSnapshot,
        detail: Option<Self>,
        defaults: ParallelModeLiveSessionDetailDefaults<'_>,
    ) -> Self {
        let mut detail = detail.unwrap_or_else(|| Self::assigned_for_lease(lease, defaults));
        detail.session_key = lease.session_key();
        detail.agent_id = lease.agent_id.clone();
        detail.task_id = lease.task_id.clone();
        detail.task_title = lease.task_title.clone();
        detail.slot_id = lease.slot_id.clone();
        detail.worktree_path = lease.worktree_path.clone();
        detail.branch_name = lease.branch_name.clone();
        detail.lease_started_at = lease.leased_at.clone();
        detail.state_label = live_detail_state_label(lease, &detail);
        detail.completion_state_label = live_completion_state_label(lease, &detail);
        // 비어 있는 text fields는 live supervisor가 공백으로 보이지 않게 fallback을 채운다.
        if detail.latest_summary.trim().is_empty() {
            detail.latest_summary = roster_latest_summary(lease, Some(&detail));
        }
        if detail.validation_summary.trim().is_empty() {
            detail.validation_summary = defaults.validation_summary.to_string();
        }
        if detail.authority_refresh_outcome.trim().is_empty() {
            detail.authority_refresh_outcome = defaults.authority_refresh_outcome.to_string();
        }
        if detail.distributor_outcome.is_none() {
            detail.distributor_outcome = live_distributor_outcome(lease);
        }
        if detail.updated_at.trim().is_empty() {
            detail.updated_at = live_detail_updated_at(lease).to_string();
        }
        detail
    }

    /*
     * supervisor detail 선택은 active queue session을 최우선으로 한다. 그 다음에는 현재 lease 중 selection
     * priority가 가장 높은 session을 보여 주고, live lease가 없으면 persisted history 첫 항목을 fallback으로
     * 사용한다. 이 우선순위를 domain에 두면 UI가 lease ordering을 재구현하지 않는다.
     */
    pub fn select_runtime_detail(
        leases: &[ParallelModeSlotLeaseSnapshot],
        history: &[ParallelModeAgentSessionDetailSnapshot],
        active_queue_session_key: Option<&str>,
        defaults: ParallelModeLiveSessionDetailDefaults<'_>,
    ) -> Option<Self> {
        if let Some(session_key) = active_queue_session_key
            && let Some(detail) =
                Self::detail_for_runtime_session(leases, history, session_key, defaults)
        {
            return Some(detail);
        }

        if let Some(lease) = leases
            .iter()
            .max_by(|left, right| compare_lease_selection(left, right))
        {
            return Some(Self::live_for_lease(
                lease,
                detail_for_lease(history, lease),
                defaults,
            ));
        }

        history.first().cloned()
    }

    // 특정 runtime session key에 대한 persisted detail과 live lease를 결합한다.
    fn detail_for_runtime_session(
        leases: &[ParallelModeSlotLeaseSnapshot],
        history: &[ParallelModeAgentSessionDetailSnapshot],
        session_key: &str,
        defaults: ParallelModeLiveSessionDetailDefaults<'_>,
    ) -> Option<Self> {
        let detail = history
            .iter()
            .find(|detail| detail.session_key == session_key)
            .cloned();
        if let Some(lease) = leases
            .iter()
            .find(|lease| lease.session_key() == session_key)
        {
            return Some(Self::live_for_lease(lease, detail, defaults));
        }

        detail
    }
}

// lease state와 detail override를 합쳐 detail panel의 현재 state label을 만든다.
fn live_detail_state_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> String {
    // completion/distributor pipeline이 더 구체적인 label을 가지고 있으면 lease state보다 우선한다.
    if let Some(label) = lease.runtime_state_override(detail) {
        return label.to_string();
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased => {
            if detail.thread_id.is_some() || detail.state_label == "starting" {
                "starting".to_string()
            } else {
                "assigned".to_string()
            }
        }
        ParallelModeSlotLeaseState::Running => "running".to_string(),
        ParallelModeSlotLeaseState::CleanupPending => "cleanup_pending".to_string(),
    }
}

// completion state는 live lease가 아직 worker 진행 중인지, cleanup pending으로 merge가 끝났는지를 요약한다.
fn live_completion_state_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> String {
    if lease.runtime_state_override(detail).is_some() {
        return detail.completion_state_label.clone();
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running => {
            "in_progress".to_string()
        }
        ParallelModeSlotLeaseState::CleanupPending => "merged".to_string(),
    }
}

// cleanup pending lease는 distributor가 이미 merge를 끝낸 상태라 detail에 outcome을 보강할 수 있다.
fn live_distributor_outcome(lease: &ParallelModeSlotLeaseSnapshot) -> Option<String> {
    match lease.state {
        ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running => None,
        ParallelModeSlotLeaseState::CleanupPending => {
            Some("branch is merged into prerelease and the slot is awaiting cleanup".to_string())
        }
    }
}

// running timestamp가 있으면 그것이 최신 live update이고, 없으면 lease 시작 시간을 사용한다.
fn live_detail_updated_at(lease: &ParallelModeSlotLeaseSnapshot) -> &str {
    lease
        .running_started_at
        .as_deref()
        .unwrap_or(lease.leased_at.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeSupervisorDetailSnapshot {
    // 현재 supervisor detail panel에 표시할 session이다.
    pub session: Option<ParallelModeAgentSessionDetailSnapshot>,
    // session이 없을 때 표시할 상태 문구다.
    pub empty_state: String,
}

impl ParallelModeSupervisorDetailSnapshot {
    // supervisor builder가 session optional과 empty state copy를 함께 고정한다.
    pub fn new(
        session: Option<ParallelModeAgentSessionDetailSnapshot>,
        empty_state: impl Into<String>,
    ) -> Self {
        Self {
            session,
            empty_state: empty_state.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeAgentRosterSnapshot {
    // active lease들을 roster rows로 투영한 결과다.
    pub entries: Vec<ParallelModeAgentRosterEntry>,
    // entries가 비어 있을 때 표시할 문구다.
    pub empty_state: String,
}

impl ParallelModeAgentRosterSnapshot {
    // presentation layer가 empty-state rule을 직접 만들지 않도록 snapshot에 포함한다.
    pub fn new(entries: Vec<ParallelModeAgentRosterEntry>, empty_state: impl Into<String>) -> Self {
        Self {
            entries,
            empty_state: empty_state.into(),
        }
    }

    // compact status copy에서 active agent 수만 빠르게 읽는다.
    pub fn active_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.counts_as_active())
            .count()
    }

    // supervisor header에 들어가는 짧은 roster summary다.
    pub fn compact_summary(&self) -> String {
        format!("{} active", self.active_count())
    }

    /*
     * lease list와 persisted details를 join해 roster snapshot을 만든다. lease는 "지금 slot이 살아 있는가"의
     * source-of-truth이고, detail은 progress copy와 pipeline state를 보강한다. duration labels는 caller가
     * clock에 의존해 계산하므로 domain projection에는 이미 계산된 map만 들어온다.
     */
    pub fn project_from_leases(
        leases: Vec<ParallelModeSlotLeaseSnapshot>,
        details: &[ParallelModeAgentSessionDetailSnapshot],
        mode_enabled: bool,
        running_duration_labels: &BTreeMap<String, String>,
    ) -> Self {
        let active_leases = sorted_active_leases(leases);

        let entries = active_leases
            .iter()
            .map(|lease| {
                let detail = details
                    .iter()
                    .find(|detail| detail.session_key == lease.session_key());
                project_agent_roster_entry(lease, detail, running_duration_labels)
            })
            .collect::<Vec<_>>();
        let empty_state = if mode_enabled {
            "no agent sessions launched in this slice"
        } else {
            "parallel mode is off / agent roster is read-only"
        };

        Self::new(entries, empty_state)
    }
}

// roster는 running > leased > cleanup_pending 우선순위와 최신 session key 순서로 안정 정렬한다.
fn sorted_active_leases(
    mut active_leases: Vec<ParallelModeSlotLeaseSnapshot>,
) -> Vec<ParallelModeSlotLeaseSnapshot> {
    active_leases.sort_by(|left, right| compare_lease_selection(right, left));
    active_leases
}

// selection_priority가 높은 lease를 먼저 고르고, tie는 slot id 역순으로 고정해 snapshot jitter를 줄인다.
fn compare_lease_selection(
    left: &ParallelModeSlotLeaseSnapshot,
    right: &ParallelModeSlotLeaseSnapshot,
) -> std::cmp::Ordering {
    left.selection_priority()
        .cmp(&right.selection_priority())
        .then_with(|| right.slot_id.cmp(&left.slot_id))
}

// persisted history에서 live lease와 같은 session key를 가진 detail을 찾는다.
fn detail_for_lease(
    history: &[ParallelModeAgentSessionDetailSnapshot],
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    history
        .iter()
        .find(|detail| detail.session_key == lease.session_key())
        .cloned()
}

// lease와 optional detail을 roster row 하나로 투영한다.
fn project_agent_roster_entry(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
    running_duration_labels: &BTreeMap<String, String>,
) -> ParallelModeAgentRosterEntry {
    ParallelModeAgentRosterEntry::new(
        lease.agent_id.clone(),
        lease.task_title.clone(),
        lease.slot_id.clone(),
        lease.branch_name.clone(),
        roster_state_label(lease, detail),
        roster_duration_label(lease, detail, running_duration_labels),
        roster_latest_summary(lease, detail),
    )
}

// state priority는 roster sorting과 default selection이 공유하는 lease lifecycle ordering이다.
pub(super) fn roster_state_priority(state: ParallelModeSlotLeaseState) -> u8 {
    match state {
        ParallelModeSlotLeaseState::Running => 3,
        ParallelModeSlotLeaseState::Leased => 2,
        ParallelModeSlotLeaseState::CleanupPending => 1,
    }
}

// running_started_at이 있으면 recency key로 쓰고, 없으면 leased_at을 사용한다.
pub(super) fn roster_recency_key(lease: &ParallelModeSlotLeaseSnapshot) -> &str {
    lease
        .running_started_at
        .as_deref()
        .unwrap_or(lease.leased_at.as_str())
}

// roster state는 detail override가 있으면 pipeline label을 우선하고, 아니면 lease state를 보여 준다.
pub fn roster_state_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
) -> String {
    if let Some(detail) = detail
        && let Some(label) = lease.runtime_state_override(detail)
    {
        return label.to_string();
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased => "starting".to_string(),
        ParallelModeSlotLeaseState::Running => "running".to_string(),
        ParallelModeSlotLeaseState::CleanupPending => "cleanup_pending".to_string(),
    }
}

// duration column은 단순 시간뿐 아니라 official completion/distributor phase를 압축해 보여 주는 자리다.
fn roster_duration_label(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
    running_duration_labels: &BTreeMap<String, String>,
) -> String {
    // detail state는 delivery pipeline phase를 더 구체적으로 표현하므로 elapsed label보다 우선한다.
    if let Some(detail) = detail {
        match detail.state_label.as_str() {
            "reported_complete" => return "reported".to_string(),
            "ledger_refreshing" => return "refreshing".to_string(),
            "commit_ready" => return "official".to_string(),
            "merge_queued" => return "queued".to_string(),
            "pushing" => return "pushing".to_string(),
            "pr_pending" => return "pr pending".to_string(),
            "merge_pending" => return "merge pending".to_string(),
            "integrating" => return "integrating".to_string(),
            "failed" => return "blocked".to_string(),
            _ => {}
        }
    }

    match lease.state {
        ParallelModeSlotLeaseState::Leased => "launch pending".to_string(),
        ParallelModeSlotLeaseState::Running => running_duration_labels
            .get(&lease.session_key())
            .cloned()
            .unwrap_or_else(|| "active".to_string()),
        ParallelModeSlotLeaseState::CleanupPending => "complete".to_string(),
    }
}

// detail summary가 있으면 그것을 쓰고, 없으면 lease state별 안전한 fallback 문구를 제공한다.
pub fn roster_latest_summary(
    lease: &ParallelModeSlotLeaseSnapshot,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
) -> String {
    detail
        .map(|detail| detail.latest_summary.trim())
        .filter(|summary| !summary.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match lease.state {
            ParallelModeSlotLeaseState::Leased => {
                "branch reserved and agent bootstrap in progress".to_string()
            }
            ParallelModeSlotLeaseState::Running => {
                "agent session is active in the leased slot".to_string()
            }
            ParallelModeSlotLeaseState::CleanupPending => {
                "agent session reported completion and slot cleanup is pending".to_string()
            }
        })
}
