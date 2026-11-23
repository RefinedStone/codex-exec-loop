//! 이슈 자동화 장부의 영속 경계다 (#2536).
//! 원문이나 임의 JSON을 받지 않는다. 공유본, 원본의 필드 해시, 도메인 상태만 저장한다.
//! workspace_dir는 composition이 정한 대표 checkout의 절대 경로이며 기록의 project scope와 같아야 한다.
use anyhow::Result;

use super::issue_source_port::{IssueDiscoveryPage, IssueDiscoveryRequest};
use crate::domain::issue_automation::collection::{
    IssueCollectionFailure, IssueCollectionFence, IssueCollectionItem, IssueCollectionMode,
    IssueCollectionPage, IssueCollectionProjection, IssueCollectionQuery, IssueResyncBatch,
    IssueTriageProjection, StoredIssueCollection,
};
use crate::domain::issue_automation::decision::{
    IssueDecisionDeadline, IssueDecisionId, IssueDecisionReminder, IssueDecisionRequest,
    StoredIssueDecision,
};
use crate::domain::issue_automation::policy::IssueAutomationTimeouts;
use crate::domain::issue_automation::source::IssueSourceRead;
use crate::domain::issue_automation::triage::IssueTriageAssessment;
use crate::domain::issue_automation::{
    IssueAutomationBinding, IssueAutomationBindingId, TrackedIssue,
};
use crate::domain::issue_work::IssueReference;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIssueAutomationBinding {
    pub binding: IssueAutomationBinding,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutomationBindingInsertResult {
    pub binding: StoredIssueAutomationBinding,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutomationIssueInsertResult {
    pub issue: TrackedIssue,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueAutomationBindingSaveResult {
    Saved {
        record: Box<StoredIssueAutomationBinding>,
        changed: bool,
    },
    Conflict {
        actual_revision: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueAutomationIssueSaveResult {
    Saved {
        record: Box<TrackedIssue>,
        changed: bool,
    },
    Conflict {
        actual_revision: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueCollectionModeSaveResult {
    Saved {
        state: Box<StoredIssueCollection>,
        /// 이 저장 transaction이 확정한 state/binding/catalog다. 반환 뒤 최신 조회로 대체하지 않는다.
        projection: Box<IssueCollectionProjection>,
        changed: bool,
    },
    Conflict {
        current: Option<Box<StoredIssueCollection>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionCommit {
    pub state: StoredIssueCollection,
    /// 페이지/실패/커서 변경과 같은 transaction에서 읽은 확정 snapshot이다.
    pub projection: IssueCollectionProjection,
    /// observation + legacy tracked 합집합에 전체 안정 ID가 처음 생긴 경우만 포함한다.
    pub newly_tracked: Vec<IssueReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueCollectionApplyResult {
    Applied(Box<IssueCollectionCommit>),
    Discarded {
        current: Box<StoredIssueCollection>,
        /// fence를 거절한 transaction에서 관측한 snapshot이다.
        projection: Box<IssueCollectionProjection>,
    },
}

/// 파일 정책 판과 독립적인 저장 CAS다. collection epoch와 Observe도 같은 transaction에서 검사한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueTriageFence {
    pub collection_epoch: u64,
    pub observation_revision: Option<u64>,
    pub tracked_revision: Option<u64>,
    pub assessment_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTriageBatch {
    pub collection: Option<StoredIssueCollection>,
    pub revision: u64,
    pub items: Vec<IssueCollectionItem>,
    pub next_after: Option<IssueReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueTriageSaveResult {
    Saved {
        record: Box<IssueTriageProjection>,
        binding_id: IssueAutomationBindingId,
        catalog_revision: u64,
        /// 의미가 달라진 새 판정만 true다. 같은 의미의 관찰 CAS 재연결은 false다.
        changed: bool,
    },
    Conflict,
}

/// 결정 요청 행과 현재 관찰 판을 함께 검사한다. 모드는 답변 허가가 아니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueDecisionFence {
    pub triage: IssueTriageFence,
    pub decision_revision: u64,
}

/// 저장 transaction에서 확정된 결과다. 최신 조회로 바꾸면 안 된다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDecisionCommit {
    pub record: StoredIssueDecision,
    pub catalog_revision: u64,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueDecisionSaveResult {
    Saved(Box<IssueDecisionCommit>),
    Conflict,
}

/// 저장소 runtime의 별도 장부다. planning DB와 schema를 공유하지 않는다.
/// 같은 안정 ID의 재삽입은 기존 내용을 보존한다. 갱신은 revision CAS로만 한다.
pub trait IssueAutomationStorePort: Send + Sync {
    fn issue_autonomy_selection(
        &self,
        _workspace: &str,
    ) -> Result<crate::domain::issue_automation::autonomy::IssueAutonomySelection> {
        Ok(Default::default())
    }
    fn get_issue_autonomy_command(
        &self,
        _workspace: &str,
        _command_id: &str,
    ) -> Result<Option<crate::domain::issue_automation::autonomy::StoredIssueAutonomyCommand>> {
        Ok(None)
    }
    fn begin_issue_autonomy(
        &self,
        _workspace: &str,
        _intent: &crate::domain::issue_automation::autonomy::IssueAutonomyIntent,
    ) -> Result<crate::domain::issue_automation::autonomy::StoredIssueAutonomyCommand> {
        anyhow::bail!("issue autonomy persistence is unavailable")
    }
    /// Reserve a new Retriage intent and its autonomy ownership in one transaction.
    fn claim_issue_autonomy_fence(
        &self,
        _workspace: &str,
        _intent: &crate::domain::issue_automation::autonomy::IssueAutonomyIntent,
        _work: &crate::domain::issue_automation::coordinator::IssueWorkCheckpoint,
    ) -> Result<crate::domain::issue_automation::intervention::StoredIssueIntervention> {
        anyhow::bail!("owned issue autonomy fences are unavailable")
    }
    fn finish_issue_autonomy(
        &self,
        _workspace: &str,
        _intent: &crate::domain::issue_automation::autonomy::IssueAutonomyIntent,
    ) -> Result<crate::domain::issue_automation::autonomy::IssueAutonomyCommit> {
        anyhow::bail!("issue autonomy persistence is unavailable")
    }
    fn list_issue_work(
        &self,
        _workspace: &str,
    ) -> Result<Vec<crate::domain::issue_automation::coordinator::IssueWorkCheckpoint>> {
        Ok(Vec::new())
    }
    fn closed_issue_decisions(
        &self,
        _workspace: &str,
        _issue: &IssueReference,
        _limit: u16,
    ) -> Result<Vec<StoredIssueDecision>> {
        Ok(Vec::new())
    }

    fn issue_authorization_epoch(&self, _workspace: &str, _issue: &IssueReference) -> Result<u64> {
        Ok(0)
    }

    fn pending_issue_work_events(
        &self,
        workspace: &str,
    ) -> Result<Vec<crate::domain::issue_automation::coordinator::IssueWorkEvent>>;
    fn acknowledge_issue_work_event(&self, workspace: &str, event_id: &str) -> Result<()>;
    fn get_issue_intervention(
        &self,
        _workspace: &str,
        _command_id: &str,
    ) -> Result<Option<crate::domain::issue_automation::intervention::StoredIssueIntervention>>
    {
        anyhow::bail!("issue interventions are unavailable")
    }
    /// Atomically record the steering intent and Unknown receipt. Only the first claimant may act.
    fn claim_issue_steer_delivery(
        &self,
        _workspace: &str,
        _request: &crate::domain::issue_automation::intervention::IssueInterventionRequest,
        _receipt: &crate::domain::issue_automation::intervention::IssueInterventionReceipt,
    ) -> Result<bool> {
        anyhow::bail!("atomic issue steering claims are unavailable")
    }
    fn begin_issue_intervention(
        &self,
        _workspace: &str,
        _request: &crate::domain::issue_automation::intervention::IssueInterventionRequest,
    ) -> Result<crate::domain::issue_automation::intervention::StoredIssueIntervention> {
        anyhow::bail!("issue interventions are unavailable")
    }
    fn finish_issue_intervention(
        &self,
        _workspace: &str,
        _receipt: &crate::domain::issue_automation::intervention::IssueInterventionReceipt,
    ) -> Result<()> {
        anyhow::bail!("issue interventions are unavailable")
    }
    fn pending_issue_interventions(
        &self,
        _workspace: &str,
    ) -> Result<Vec<crate::domain::issue_automation::intervention::StoredIssueIntervention>> {
        Ok(Vec::new())
    }

    /// Unknown을 먼저 저장한 최초 claimant만 실행 포트에 전달할 수 있다.
    fn claim_issue_execution_answer(
        &self,
        _workspace: &str,
        _intent: &crate::domain::issue_automation::execution_answer::IssueExecutionAnswerIntent,
    ) -> Result<crate::domain::issue_automation::execution_answer::IssueExecutionAnswerClaim> {
        anyhow::bail!("durable issue answer claims are unavailable")
    }

    /// 재시작/응답 유실 때에는 이 목록을 읽어 영수증만 조회하고 다시 보내지 않는다.
    fn pending_issue_execution_answers(
        &self,
        _workspace: &str,
        _work_order_id: &str,
    ) -> Result<Vec<crate::domain::issue_automation::execution_answer::IssueExecutionAnswerRecord>>
    {
        Ok(Vec::new())
    }

    /// 동일 ACK는 멱등이며, 서로 다른 ACK/최종 거절은 덮어쓰지 않는다. None은 revision 충돌이다.
    fn save_issue_execution_answer(
        &self,
        _workspace: &str,
        _command_id: &str,
        _expected_revision: u64,
        _status: &crate::domain::issue_automation::execution_answer::IssueExecutionAnswerStatus,
    ) -> Result<Option<crate::domain::issue_automation::execution_answer::IssueExecutionAnswerRecord>>
    {
        anyhow::bail!("durable issue answer receipts are unavailable")
    }

    /// 실행 관측은 수집 모드와 독립적으로 읽는다. 외부 이슈나 새 작업을 만들지 않는다.
    fn get_issue_execution(
        &self,
        _workspace: &str,
        _issue: &IssueReference,
    ) -> Result<Option<crate::domain::issue_automation::execution::IssueExecutionRecord>> {
        Ok(None)
    }

    /// work/progress CAS, 실행·검증 영수증, lineage, 수명과 outbox를 한 transaction으로 확정한다.
    fn apply_issue_execution(
        &self,
        _workspace: &str,
        _work: &crate::domain::issue_automation::coordinator::IssueWorkCheckpoint,
        _expected_progress_revision: u64,
        _observation: &crate::domain::issue_automation::execution::IssueWorkExecutionObservation,
        _max_attempts: Option<u32>,
    ) -> Result<IssueExecutionSaveResult> {
        anyhow::bail!("issue execution observations are unavailable")
    }

    fn get_issue_work(
        &self,
        workspace: &str,
        issue: &IssueReference,
    ) -> Result<Option<crate::domain::issue_automation::coordinator::IssueWorkCheckpoint>>;
    /// Reserves the first work ID before any planning registration. The caller supplies the
    /// current effective budget whose policy version is bound by the assessment.
    /// Replays never consume another daily start, even after reconnect or a UTC day change.
    fn admit_issue_work(
        &self,
        _workspace: &str,
        _expected: IssueTriageFence,
        _assessment: &IssueTriageAssessment,
        _candidate: &crate::domain::issue_automation::coordinator::IssueWorkCheckpoint,
        _budget: &crate::domain::issue_automation::policy::IssueAutomationBudget,
        _now_unix_ms: u64,
    ) -> Result<IssueWorkAdmissionResult> {
        anyhow::bail!("issue work admission is unavailable")
    }
    /// Updates an admitted checkpoint. Every new checkpoint must use admission.
    fn save_issue_work(
        &self,
        workspace: &str,
        expected: IssueTriageFence,
        assessment: &IssueTriageAssessment,
        candidate: &crate::domain::issue_automation::coordinator::IssueWorkCheckpoint,
    ) -> Result<IssueWorkSaveResult>;
    /// 현재 policy/Observe 상태와 무관하게 전달 권한만 줄인다. Start와 revision floor는 보존한다.
    fn clear_issue_delivery(
        &self,
        _workspace: &str,
        _issue: &IssueReference,
        _expected_work_revision: u64,
        _reason: crate::domain::issue_automation::coordinator::IssueWorkBlockReason,
    ) -> Result<IssueWorkSaveResult> {
        anyhow::bail!("issue delivery invalidation is unavailable")
    }
    /// 한 건의 강제 재조회다. 정기 배치의 커서는 이동하지 않는다.
    fn apply_issue_resync(
        &self,
        workspace: &str,
        expected: IssueCollectionFence,
        issue: &IssueReference,
        read: &IssueSourceRead,
    ) -> Result<IssueCollectionApplyResult>;

    /// 기존 ID의 좌표는 바꾸지 않는다. 같은 좌표이면 기존 자율 수준과 revision을 그대로 반환한다.
    fn insert_binding(
        &self,
        workspace_dir: &str,
        binding: &IssueAutomationBinding,
    ) -> Result<IssueAutomationBindingInsertResult>;

    fn get_binding(
        &self,
        workspace_dir: &str,
        binding_id: &IssueAutomationBindingId,
    ) -> Result<Option<StoredIssueAutomationBinding>>;

    fn list_bindings(&self, workspace_dir: &str) -> Result<Vec<StoredIssueAutomationBinding>>;

    /// 바인딩 ID와 원격/프로젝트 좌표는 불변이다. 자율 수준을 바꾸는 허가는 상위 명령 경계가 검사한다.
    fn save_binding(
        &self,
        workspace_dir: &str,
        expected_revision: u64,
        binding: &IssueAutomationBinding,
    ) -> Result<IssueAutomationBindingSaveResult>;

    /// 신규 발견만 받는다(revision 0). 같은 전체 IssueReference가 있으면 그 행을 그대로 반환한다.
    /// 반복 수집이 상태, 표시, 공유본 또는 기존 바인딩을 초기화하지 않는다.
    fn insert_issue(
        &self,
        workspace_dir: &str,
        issue: &TrackedIssue,
    ) -> Result<IssueAutomationIssueInsertResult>;

    fn get_issue(
        &self,
        workspace_dir: &str,
        reference: &IssueReference,
    ) -> Result<Option<TrackedIssue>>;

    fn list_issues(
        &self,
        workspace_dir: &str,
        binding_id: Option<&IssueAutomationBindingId>,
    ) -> Result<Vec<TrackedIssue>>;

    /// candidate.revision은 expected_revision과 같아야 한다. 상태 변경은 도메인 전이 규칙을 다시 검사한다.
    /// 성공은 revision을 1 올리고, 내용이 같은 no-op은 유지한다.
    /// 응답 유실 재시도는 현재 revision이 expected+1이고 내용이 정확히 같을 때만 인정한다.
    /// 그보다 뒤의 동일 내용은 ABA일 수 있으므로 conflict다. 실패를 미존재나 빈 장부로 숨기지 않는다.
    fn save_issue(
        &self,
        workspace_dir: &str,
        expected_revision: u64,
        issue: &TrackedIssue,
    ) -> Result<IssueAutomationIssueSaveResult>;

    fn get_collection(&self, workspace_dir: &str) -> Result<Option<StoredIssueCollection>>;

    /// control/binding/catalog는 한 read transaction의 snapshot이다.
    fn get_collection_projection(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<IssueCollectionProjection>>;

    /// 없을 때 Observe를 먼저 commit한다. 기존 Off 설정은 그대로 보존한다.
    fn ensure_collection(&self, workspace_dir: &str) -> Result<StoredIssueCollection>;

    /// None은 최초 설정 전용이다. 모드 변화는 epoch와 revision을 함께 올린다.
    /// 성공 반환은 동기 commit 뒤이며, 재시도는 바로 다음 epoch의 같은 모드까지만 인정한다.
    fn set_collection_mode(
        &self,
        workspace_dir: &str,
        expected_epoch: Option<u64>,
        mode: IssueCollectionMode,
    ) -> Result<IssueCollectionModeSaveResult>;

    /// 사람 확인이 묶인 이전 mode/epoch/revision을 한 transaction에서 정확히 검사한다.
    /// conflict는 기존 증거로 새 판을 자동 승인하지 않는다.
    fn set_collection_mode_exact(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        from: IssueCollectionMode,
        mode: IssueCollectionMode,
    ) -> Result<IssueCollectionModeSaveResult>;

    /// Observe + fence + 요청 cursor가 모두 일치할 때만 페이지 전체와 다음 cursor를 commit한다.
    /// 최초 binding과 Withheld 관찰도 같은 트랜잭션에서 저장한다.
    fn apply_discovery_page(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        request: &IssueDiscoveryRequest,
        page: &IssueDiscoveryPage,
    ) -> Result<IssueCollectionApplyResult>;

    /// 같은 안정 저장소의 공개 정책/선택/페이지 크기 변경 시 호출한다.
    /// cursor가 있으면 epoch와 revision을 함께 올려 이전 응답을 폐기한다.
    /// cursor가 없으면 no-op이며 재동기화 cursor는 보존한다.
    fn reset_discovery_cursor(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
    ) -> Result<IssueCollectionApplyResult>;

    /// None은 Off 또는 오래된 fence다. 유효한 빈 대상 목록은 Some(empty batch)다.
    /// 완료 작업을 제외하고 observation-only와 legacy tracked를 안정 ID 순서로 함께 순회한다.
    fn read_resync_batch(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        limit: u16,
    ) -> Result<Option<IssueResyncBatch>>;

    /// 대상별 읽기(실패는 Unknown) 전체와 재동기화 cursor를 원자적으로 적용한다.
    fn apply_resync_batch(
        &self,
        workspace_dir: &str,
        batch: &IssueResyncBatch,
        reads: &[IssueSourceRead],
    ) -> Result<IssueCollectionApplyResult>;

    /// 관찰/cursor를 움직이지 않는 시도 결과다. 실패의 외부 원문은 받지 않는다.
    fn record_collection_attempt(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        at_unix_ms: u64,
        failure: Option<IssueCollectionFailure>,
    ) -> Result<IssueCollectionApplyResult>;

    /// 현재 관찰만 투영한다. Withheld/Unknown에 과거 snapshot을 대신 끼우지 않는다.
    fn query_collection_issues(
        &self,
        workspace_dir: &str,
        query: &IssueCollectionQuery,
    ) -> Result<IssueCollectionPage>;

    /// 같은 read transaction에서 전체 합집합을 안정 ID 순서로 유한 페이지씩 순회한다.
    /// visitor는 순수 계산만 하고 파일/네트워크 I/O나 이 Store에 재진입하지 않는다.
    /// 실패 시 부분 집계는 호출자가 폐기한다. 반환값은 같은 snapshot의 catalog revision이다.
    fn scan_collection_issues(
        &self,
        workspace_dir: &str,
        binding_id: Option<&IssueAutomationBindingId>,
        visitor: &mut dyn FnMut(IssueCollectionItem) -> Result<()>,
    ) -> Result<u64>;

    fn read_triage_batch(
        &self,
        workspace_dir: &str,
        after: Option<&IssueReference>,
        limit: u16,
    ) -> Result<IssueTriageBatch>;

    /// 별도 판정만 저장한다. workflow 상태/flags/snapshot을 갱신하는 우회 경로가 아니다.
    /// source 증거와 checked gate successor를 검증하고 모든 fence가 맞을 때만 commit한다.
    fn save_triage(
        &self,
        workspace_dir: &str,
        expected: IssueTriageFence,
        assessment: &IssueTriageAssessment,
    ) -> Result<IssueTriageSaveResult>;

    fn get_issue_decision(
        &self,
        workspace_dir: &str,
        id: &IssueDecisionId,
    ) -> Result<Option<StoredIssueDecision>>;

    /// 답이 이미 확정된 경우 정책/원문을 다시 읽지 않고 같은 snapshot 결과를 반환한다.
    fn get_issue_decision_commit(
        &self,
        workspace_dir: &str,
        id: &IssueDecisionId,
    ) -> Result<Option<IssueDecisionCommit>>;

    /// 같은 불변 정의와 판의 반복 생성은 기존 요청을 반환한다. 서로 다른 질문은 별개다.
    /// 기존 닫힌 요청도 재개방하지 않는다.
    fn ensure_issue_decision(
        &self,
        workspace_dir: &str,
        expected: IssueTriageFence,
        current: &IssueTriageAssessment,
        request: &IssueDecisionRequest,
    ) -> Result<IssueDecisionSaveResult>;

    /// due_at이 없는 생성 template에 최초 저장 시각부터의 기한을 한 번만 부여한다.
    /// 기존 요청의 기한과 legacy None은 바꾸지 않는다. 명시적 기한은 strict ensure를 쓴다.
    #[allow(clippy::too_many_arguments)]
    fn ensure_issue_decision_timed(
        &self,
        _workspace: &str,
        _expected: IssueTriageFence,
        _current: &IssueTriageAssessment,
        _request: &IssueDecisionRequest,
        _now_unix_ms: u64,
        _timeouts: &IssueAutomationTimeouts,
    ) -> Result<IssueDecisionSaveResult> {
        anyhow::bail!("timed issue decisions are unavailable")
    }

    /// 열린 기한 요청의 안정 ID 오름차순 keyset 페이지다. adapter는 limit을 256 이하로 제한한다.
    fn issue_decision_deadlines(
        &self,
        _workspace: &str,
        _after_id: Option<&IssueDecisionId>,
        _limit: usize,
    ) -> Result<Vec<IssueDecisionDeadline>> {
        Ok(Vec::new())
    }

    /// 현재 기준과 서버 시각으로 timeout 전이를 구성한다. 일반 답변 저장과 출처를 분리한다.
    fn timeout_issue_decision(
        &self,
        _workspace: &str,
        _expected: IssueDecisionFence,
        _current: &IssueTriageAssessment,
        _id: &IssueDecisionId,
        _now_unix_ms: u64,
    ) -> Result<IssueDecisionSaveResult> {
        anyhow::bail!("issue decision timeouts are unavailable")
    }

    /// 최초 알림 사실을 확정한다. 이미 처리됐거나 기한 밖이면 None이며 결정 판은 바꾸지 않는다.
    fn remind_issue_decision(
        &self,
        _workspace: &str,
        _expected: IssueDecisionFence,
        _current: &IssueTriageAssessment,
        _id: &IssueDecisionId,
        _now_unix_ms: u64,
    ) -> Result<Option<IssueDecisionReminder>> {
        Ok(None)
    }

    /// ACK 전까지 같은 event_id로 재전달한다. event_id 오름차순이며 최대 256건이다.
    fn pending_issue_decision_reminders(
        &self,
        _workspace: &str,
        _limit: usize,
    ) -> Result<Vec<IssueDecisionReminder>> {
        Ok(Vec::new())
    }

    fn acknowledge_issue_decision_reminder(&self, _workspace: &str, _event_id: &str) -> Result<()> {
        anyhow::bail!("issue decision reminder receipts are unavailable")
    }

    /// 관찰/판정/결정 행 CAS와 첫 답변을 한 transaction으로 확정한다.
    fn save_issue_decision(
        &self,
        workspace_dir: &str,
        expected: IssueDecisionFence,
        current: &IssueTriageAssessment,
        request: &IssueDecisionRequest,
    ) -> Result<IssueDecisionSaveResult>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueWorkAdmissionResult {
    Saved {
        record: Box<crate::domain::issue_automation::coordinator::IssueWorkCheckpoint>,
        catalog_revision: u64,
        changed: bool,
    },
    BudgetExhausted {
        wait: crate::domain::issue_automation::coordinator::IssueStartBudgetWait,
        catalog_revision: u64,
        changed: bool,
    },
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueWorkSaveResult {
    Saved {
        record: Box<crate::domain::issue_automation::coordinator::IssueWorkCheckpoint>,
        catalog_revision: u64,
        changed: bool,
    },
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueExecutionSaveResult {
    Saved {
        record: Box<crate::domain::issue_automation::execution::IssueExecutionRecord>,
        catalog_revision: u64,
        changed: bool,
    },
    Conflict,
}
