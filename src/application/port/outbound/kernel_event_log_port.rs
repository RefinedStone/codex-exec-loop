use anyhow::Result;

use crate::domain::kernel_event::{KernelEvent, KernelEventPage, RecordedKernelEvent};

/*
 * 커널의 판단 사건을 저장소별 추가 전용 기록에 쓰고 읽는다 (plan 19 Q3-1). 작업 공간으로 그 저장소의 기록을 찾는다.
 * 읽을 때는 기록 순서를 지킨다. 이 binary 가 모르는 사건(더 새 binary 가 쓴 것)은 건너뛴다.
 */
pub trait KernelEventLogPort: Send + Sync {
    /// Read the durable usage projection without starting work or creating a store.
    fn load_task_usage(
        &self,
        _workspace: &str,
        _task_id: &str,
    ) -> Result<Option<crate::domain::task_usage::UsageProjection>> {
        Ok(None)
    }

    /// 증거 outbox의 ID와 공개 payload를 함께 확인한다. 실제 저장 adapter는 history retention과
    /// 독립된 영속 receipt를 사용한다. 같은 ID의 다른 payload는 존재하는 것으로 처리하지 않는다.
    fn has_task_evidence_event(&self, workspace_dir: &str, expected: &KernelEvent) -> Result<bool> {
        let event_id = expected
            .task_evidence_event_id()
            .ok_or_else(|| anyhow::anyhow!("expected a task evidence event"))?;
        for event in self.load_kernel_events(
            workspace_dir,
            &[
                "criterion_evidence_recorded",
                "criterion_waived",
                "task_verification_finished",
            ],
        )? {
            if event.task_evidence_event_id() == Some(event_id) {
                anyhow::ensure!(
                    event == *expected,
                    "a task evidence event identity cannot change"
                );
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 바인딩 실행 상태의 완료 사실은 별도 ID 공간과 전체 payload로 중복을 확인한다.
    fn has_issue_binding_pause_event(
        &self,
        workspace_dir: &str,
        expected: &KernelEvent,
    ) -> Result<bool> {
        let id = expected
            .issue_binding_pause_event_id()
            .ok_or_else(|| anyhow::anyhow!("expected a binding pause event"))?;
        for event in self.load_kernel_events(workspace_dir, &["issue_binding_pause_completed"])? {
            if event.issue_binding_pause_event_id() == Some(id) {
                anyhow::ensure!(
                    event == *expected,
                    "a binding pause event identity cannot change"
                );
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// outbox ack 전 crash를 복구할 때 쓰는 영속 dedupe 조회다.
    fn has_issue_work_event(&self, workspace_dir: &str, event_id: &str) -> Result<bool> {
        Ok(self
            .load_kernel_events(
                workspace_dir,
                &[
                    "issue_work_submitted",
                    "issue_work_permit_granted",
                    "issue_execution_progressed",
                    "issue_integrated",
                    "issue_closed",
                    "issue_feedback_posted",
                    "issue_decision_reminded",
                ],
            )?
            .iter()
            .any(|event| event.issue_work_event_id() == Some(event_id)))
    }

    /// 사건을 커널이 정한 sequence 와 시각 그대로 쓴다 (Q3-3b). 남김 종류가 아닌 오래된 사건은 최근
    /// `RETAINED_KERNEL_EVENTS` 개만 남긴다 (Q3-3). 마지막 `repository_changed` 는 한도와 관계없이 남긴다.
    fn append_kernel_events(
        &self,
        workspace_dir: &str,
        records: &[RecordedKernelEvent],
    ) -> Result<()>;

    /// 기록의 가장 큰 sequence 이다. 비었으면 0 이다 (Q3-3b). 커널은 그 다음 수부터 정한다. 기본값은 0 이다 (시험용 기록).
    fn last_kernel_event_sequence(&self, _workspace_dir: &str) -> Result<i64> {
        Ok(0)
    }

    /// 이 종류(`KernelEvent::kind`)의 사건만 기록 순서대로 읽는다.
    fn load_kernel_events(&self, workspace_dir: &str, kinds: &[&str]) -> Result<Vec<KernelEvent>>;

    /// 전체 이슈 좌표가 같은 최근 기록을 최대 limit개 읽는다. 반환 기록은 sequence 오름차순이다.
    /// 미지원이면 None이며, 조회는 저장소 생성·migration·원격 수집을 시작하지 않는다.
    fn load_recent_issue_kernel_events(
        &self,
        _workspace_dir: &str,
        _issue: &crate::domain::issue_work::IssueReference,
        _limit: usize,
    ) -> Result<Option<Vec<RecordedKernelEvent>>> {
        Ok(None)
    }

    /// `after` 보다 뒤의 사건을 최대 `limit` 개 기록 순서대로 읽는다 (plan 19 Q3-3). `limit` 은
    /// `MAX_KERNEL_EVENT_PAGE` 를 넘지 않는다. 기본값은 빈 쪽이다 (조회를 쓰지 않는 시험용 기록).
    fn load_kernel_events_after(
        &self,
        _workspace_dir: &str,
        after: i64,
        _limit: usize,
    ) -> Result<KernelEventPage> {
        Ok(KernelEventPage {
            records: Vec::new(),
            next_after: after,
            recorder: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issue_automation::execution::{
        IssueExecutionIntegration, IssueExecutionProgress,
    };
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

    struct StoredEvents(Vec<KernelEvent>);
    impl KernelEventLogPort for StoredEvents {
        fn append_kernel_events(&self, _: &str, _: &[RecordedKernelEvent]) -> Result<()> {
            unreachable!("the recovery lookup never writes")
        }

        fn load_kernel_events(&self, _: &str, kinds: &[&str]) -> Result<Vec<KernelEvent>> {
            Ok(self
                .0
                .iter()
                .filter(|event| kinds.contains(&event.kind()))
                .cloned()
                .collect())
        }
    }

    #[test]
    fn default_outbox_lookup_finds_execution_and_integration_receipts() {
        let issue = IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_stable".into(),
            issue_id: "I_stable".into(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".into(),
            },
        };
        let log = StoredEvents(vec![
            KernelEvent::IssueExecutionProgressed {
                event_id: "execution-1".into(),
                issue: (&issue).into(),
                work_order_id: "work-1".into(),
                progress: IssueExecutionProgress::default(),
                revision: 1,
            },
            KernelEvent::IssueIntegrated {
                event_id: "integrated-1".into(),
                issue: (&issue).into(),
                work_order_id: "work-1".into(),
                integration: IssueExecutionIntegration {
                    record_id: "integration-1".into(),
                    source_sha: "a".repeat(40),
                    integration_sha: "b".repeat(40),
                    pull_request_url: None,
                },
                revision: 2,
            },
            KernelEvent::IssueClosed {
                event_id: "closed-1".into(),
                issue: (&issue).into(),
                origin: crate::domain::kernel_event::IssueClosureRecordOrigin::ConfirmedFeedback,
                intent_id: Some("close-intent-1".into()),
                actor_id: Some("actor-1".into()),
                closure_event_id: Some("closure-provider-1".into()),
                observation_revision: None,
                revision: 3,
            },
        ]);
        assert!(
            log.has_issue_work_event("/work/project", "execution-1")
                .unwrap()
        );
        assert!(
            log.has_issue_work_event("/work/project", "integrated-1")
                .unwrap()
        );
        assert!(
            log.has_issue_work_event("/work/project", "closed-1")
                .unwrap()
        );
        assert!(
            !log.has_issue_work_event("/work/project", "missing")
                .unwrap()
        );
    }
}
