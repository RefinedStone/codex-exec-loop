// `Arc`는 여러 런타임 구성 요소가 같은 conversation runtime 구현을 공유하게 해 주는
// 원자적 참조 카운터이다. TUI app runtime, shell entrypoint, 테스트 fixture는 service를 복제해도
// 실제 app-server adapter 인스턴스는 하나의 port 객체로 유지된다.
use std::sync::Arc;
// `anyhow::Result`는 application service가 adapter 오류를 상위 TUI 흐름에 전달하는 공통 결과 타입이다.
// 여기서는 오류 종류를 새 도메인 enum으로 재포장하지 않고, runtime port의 실패 맥락을 그대로 보존한다.
use anyhow::{Context, Result};

// `InteractiveTurnRuntimePort`는 application 계층이 outbound runtime에 기대하는 최소 계약이다.
// 실제 구현은 Codex app-server adapter이지만, TUI와 service는 trait object만 보므로 테스트 fake나 다른 runtime으로
// 교체해도 호출 코드는 바뀌지 않는다.
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterThreadProjection,
};
// conversation runtime event는 이전 계층에서 정리한 스트림 계약이다.
// service는 이 이벤트 타입을 알고 있지만 이벤트 payload를 직접 만들거나 줄이지 않는다.
use crate::application::service::conversation_runtime_event::ConversationStreamSender;
use crate::application::service::review_center::{
    ReviewCenterReadService, ReviewCenterWriteService,
};
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationRuntimeControlTruth, ConversationSnapshot,
    ConversationTurnOptions,
};
use crate::domain::turn_terminal::ConversationTurnTerminalReceipt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConversationThreadSnapshot {
    pub conversation: ConversationSnapshot,
    pub thread_review: Vec<ReviewCenterThreadProjection>,
}

#[derive(Clone)]
// `ConversationService`는 TUI inbound adapter와 outbound interactive runtime port 사이의
// application facade이다. 현재 메서드는 대부분 얇은 위임이지만, 이 얇은 층이 중요한 이유는
// TUI가 `CodexAppServerAdapter` 같은 구체 adapter를 직접 잡지 않게 하고 application 언어로만 대화 기능을
// 호출하게 만들기 때문이다.
//
// 이 구조는 adapter -> application -> domain 방향을 지키는 경계이다. inbound TUI는 service를 호출하고,
// service는 port trait을 호출하며, outbound adapter는 그 port를 구현한다. 나중에 캐싱, 정책 검증, telemetry가 필요하면
// TUI나 app-server adapter를 흔들지 않고 이 service에 추가할 수 있다.
pub struct ConversationService {
    // trait object를 `Arc`에 담아 소유한다. `dyn InteractiveTurnRuntimePort`는 런타임의 실제 타입을
    // 숨기고, `Arc`는 service clone이 많아져도 같은 runtime 제어면을 공유하게 한다.
    interactive_turn_runtime_port: Arc<dyn InteractiveTurnRuntimePort>,
    review_center_read_service: Option<ReviewCenterReadService>,
    review_center_write_service: Option<ReviewCenterWriteService>,
}

impl ConversationService {
    // service 생성자는 runtime port를 주입받는다. shell entrypoint에서는 실제 app-server adapter를 넘기고,
    // TUI 테스트 fixture에서는 fake port를 넘겨 같은 application API를 검증한다.
    pub fn new(interactive_turn_runtime_port: Arc<dyn InteractiveTurnRuntimePort>) -> Self {
        Self {
            interactive_turn_runtime_port,
            review_center_read_service: None,
            review_center_write_service: None,
        }
    }

    pub fn with_review_center_read_service(
        mut self,
        review_center_read_service: ReviewCenterReadService,
    ) -> Self {
        self.review_center_write_service = Some(review_center_read_service.write_service());
        self.review_center_read_service = Some(review_center_read_service);
        self
    }

    // 저장된 conversation snapshot을 읽는 조회 메서드이다. TUI는 thread id만 알고 있고,
    // snapshot 저장 위치나 app-server 세션 디테일을 알 필요가 없으므로 port로 위임한다.
    pub fn load_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        self.interactive_turn_runtime_port
            // port 메서드 이름에는 `conversation`을 포함해 outbound 경계에서의 책임을 더 분명히 한다.
            // service 메서드는 TUI 쪽 호출 문맥에 맞춰 더 짧은 `load_snapshot`으로 노출한다.
            .load_conversation_snapshot(thread_id)
    }

    pub fn load_thread_snapshot(
        &self,
        thread_id: &str,
        fallback_workspace_directory: &str,
    ) -> Result<LoadedConversationThreadSnapshot> {
        let mut conversation = self.load_snapshot(thread_id)?;
        let thread_review = match self.review_center_read_service.as_ref() {
            Some(review_center_read_service) => {
                let workspace_dir = if conversation.cwd.trim().is_empty() {
                    fallback_workspace_directory
                } else {
                    conversation.cwd.as_str()
                };
                match review_center_read_service
                    .load_thread_reviews_for_workspace(workspace_dir, thread_id)
                {
                    Ok(thread_review) => thread_review,
                    Err(error) => {
                        conversation
                            .runtime_notices
                            .push(format!("review hydration unavailable: {error}"));
                        Vec::new()
                    }
                }
            }
            None => Vec::new(),
        };
        Ok(LoadedConversationThreadSnapshot {
            conversation,
            thread_review,
        })
    }

    pub fn load_review_center_thread_reviews(
        &self,
        thread_id: &str,
    ) -> Result<Vec<ReviewCenterThreadProjection>> {
        self.review_center_read_service
            .as_ref()
            .context("review-center read service is required for review overlay")?
            .load_thread_reviews(thread_id)
    }

    pub fn load_review_center_pending_inbox(&self) -> Result<Vec<ReviewCenterInboxItem>> {
        self.review_center_read_service
            .as_ref()
            .context("review-center read service is required for review overlay")?
            .load_pending_inbox()
    }

    pub fn load_review_center_recent_history(&self) -> Result<Vec<ReviewCenterHistoryEntry>> {
        self.review_center_read_service
            .as_ref()
            .context("review-center read service is required for review overlay")?
            .load_recent_history()
    }

    pub fn load_review_center_thread_reviews_for_workspace(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> Result<Vec<ReviewCenterThreadProjection>> {
        self.review_center_read_service
            .as_ref()
            .context("review-center read service is required for review overlay")?
            .load_thread_reviews_for_workspace(workspace_dir, thread_id)
    }

    pub fn load_review_center_pending_inbox_for_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<ReviewCenterInboxItem>> {
        self.review_center_read_service
            .as_ref()
            .context("review-center read service is required for review overlay")?
            .load_pending_inbox_for_workspace(workspace_dir)
    }

    pub fn load_review_center_recent_history_for_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<ReviewCenterHistoryEntry>> {
        self.review_center_read_service
            .as_ref()
            .context("review-center read service is required for review overlay")?
            .load_recent_history_for_workspace(workspace_dir)
    }

    pub fn persist_review_center_approval_review_for_workspace(
        &self,
        workspace_dir: &str,
        thread_id: &str,
        review: &ConversationApprovalReview,
    ) -> Result<()> {
        if let Some(review_center_write_service) = self.review_center_write_service.as_ref() {
            review_center_write_service.persist_approval_review_for_workspace(
                workspace_dir,
                thread_id,
                review,
            )?;
        }
        Ok(())
    }

    // runtime control truth는 "중단 버튼, 전체 세션 정지, 실행 상태 판단을 어느 runtime이
    // 실제로 담당하는지"를 알려 주는 값이다. AppRuntime 초기화 시 이 값을 읽어 TUI 제어 모델을 맞춘다.
    pub fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
        self.interactive_turn_runtime_port.runtime_control_truth()
    }

    // 사용자가 전체 대화 실행을 멈추려 할 때 호출되는 명령 메서드이다.
    // 실제로 어떤 프로세스/세션을 멈출지는 outbound runtime이 알고 있으므로 service는 명령만 전달한다.
    pub fn request_stop_all_sessions(&self) -> Result<()> {
        self.interactive_turn_runtime_port
            // 실패를 그대로 반환해야 TUI가 "중단 요청 자체가 실패했다"는 상태를 사용자에게 표시할 수 있다.
            .request_stop_all_sessions()
    }

    pub fn resolve_approval_request(
        &self,
        approval_id: &str,
        decision: crate::domain::conversation::ConversationApprovalDecision,
    ) -> Result<()> {
        self.interactive_turn_runtime_port
            .resolve_approval_request(approval_id, decision)
    }

    // 새 thread를 만들며 첫 prompt를 실행하는 스트리밍 진입점이다.
    // TUI의 turn submission runtime은 현재 thread_id가 없을 때 이 메서드를 호출하고, 이후 ThreadPrepared/TurnStarted 같은
    // `ConversationStreamEvent`를 수신해 세션 상태를 채운다.
    pub fn run_new_thread_stream(
        &self,
        // cwd는 app-server가 새 대화를 어느 workspace에서 시작할지 결정하는 실행 문맥이다.
        cwd: &str,
        // prompt는 사용자 입력 원문이다. service는 prompt를 변형하지 않아 adapter가 Codex 프로토콜로 매핑한다.
        prompt: &str,
        // operator가 선택한 model/think override이다. 비어 있으면 app-server 기본값을 유지한다.
        options: ConversationTurnOptions,
        // event_sender는 호출자가 만든 수신 루프와 짝을 이룬다. 소유권을 넘기는 이유는
        // runtime worker가 thread 종료까지 이 sender를 들고 스트림 이벤트를 계속 보낼 수 있어야 하기 때문이다.
        event_sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        self.interactive_turn_runtime_port
            // 새 thread 생성, app-server launch/reattach, protocol notification 해석은 모두 outbound 구현 책임이다.
            .run_new_thread_stream(cwd, prompt, options, event_sender)
    }

    // 이미 준비된 thread에 후속 prompt를 실행하는 스트리밍 진입점이다.
    // 새 thread 흐름과 같은 이벤트 계약을 사용하므로 TUI 수신 루프는 "새 대화"와 "기존 대화"를 거의 같은 방식으로 처리한다.
    pub fn run_turn_stream(
        &self,
        // thread_id는 이전 `ThreadPrepared`나 세션 목록에서 얻은 대화 식별자이다.
        // 이 값으로 outbound runtime은 올바른 app-server conversation에 prompt를 붙인다.
        thread_id: &str,
        // 후속 turn의 사용자 입력이다. service는 validation/prompt rewrite를 하지 않는 얇은 경계이다.
        prompt: &str,
        // operator가 선택한 model/think override이다. 비어 있으면 app-server 기본값을 유지한다.
        options: ConversationTurnOptions,
        // 같은 `ConversationStreamEvent` 채널을 사용해 delta, 도구 활동, 승인 상태, 완료/실패를 돌려받는다.
        event_sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        self.interactive_turn_runtime_port
            // 기존 thread에서의 turn 실행도 service가 직접 구현하지 않는다.
            // port 경계를 통과시켜 app-server adapter가 프로토콜과 세션 저장 책임을 계속 소유하게 한다.
            .run_turn_stream(thread_id, prompt, options, event_sender)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::review_center_repository_port::{
        ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterRepositoryPort,
        ReviewCenterThreadProjection,
    };
    use crate::domain::conversation::{
        ConversationApprovalReview, ConversationApprovalReviewStatus,
        ConversationRuntimeControlTruth, ConversationTurnOptions,
    };
    use anyhow::Result;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeReviewCenterRepository {
        requested_workspaces: Mutex<Vec<String>>,
        thread_reviews: Mutex<Vec<ReviewCenterThreadProjection>>,
        pending_inbox: Mutex<Vec<ReviewCenterInboxItem>>,
        history: Mutex<Vec<ReviewCenterHistoryEntry>>,
    }

    impl ReviewCenterRepositoryPort for FakeReviewCenterRepository {
        fn load_thread_reviews(
            &self,
            workspace_dir: &str,
            _thread_id: &str,
        ) -> Result<Vec<ReviewCenterThreadProjection>> {
            self.requested_workspaces
                .lock()
                .expect("workspace tracker mutex poisoned")
                .push(workspace_dir.to_string());
            Ok(self
                .thread_reviews
                .lock()
                .expect("thread review mutex poisoned")
                .clone())
        }

        fn load_pending_inbox(&self, _workspace_dir: &str) -> Result<Vec<ReviewCenterInboxItem>> {
            Ok(self
                .pending_inbox
                .lock()
                .expect("pending inbox mutex poisoned")
                .clone())
        }

        fn load_recent_history(
            &self,
            _workspace_dir: &str,
        ) -> Result<Vec<ReviewCenterHistoryEntry>> {
            Ok(self.history.lock().expect("history mutex poisoned").clone())
        }

        fn upsert_thread_review(
            &self,
            _workspace_dir: &str,
            review: &ReviewCenterThreadProjection,
        ) -> Result<()> {
            let mut thread_reviews = self
                .thread_reviews
                .lock()
                .expect("thread review mutex poisoned");
            if let Some(existing) = thread_reviews
                .iter_mut()
                .find(|existing| existing.review_id == review.review_id)
            {
                *existing = review.clone();
            } else {
                thread_reviews.push(review.clone());
            }
            Ok(())
        }

        fn replace_pending_inbox(
            &self,
            _workspace_dir: &str,
            inbox: &[ReviewCenterInboxItem],
        ) -> Result<()> {
            *self
                .pending_inbox
                .lock()
                .expect("pending inbox mutex poisoned") = inbox.to_vec();
            Ok(())
        }

        fn append_history_entry(
            &self,
            _workspace_dir: &str,
            entry: &ReviewCenterHistoryEntry,
        ) -> Result<()> {
            self.history
                .lock()
                .expect("history mutex poisoned")
                .push(entry.clone());
            Ok(())
        }
    }

    struct FakeInteractiveTurnRuntimePort {
        snapshot: ConversationSnapshot,
    }

    impl InteractiveTurnRuntimePort for FakeInteractiveTurnRuntimePort {
        fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
            ConversationRuntimeControlTruth::default()
        }

        fn load_conversation_snapshot(&self, _thread_id: &str) -> Result<ConversationSnapshot> {
            Ok(self.snapshot.clone())
        }

        fn request_stop_all_sessions(&self) -> Result<()> {
            Ok(())
        }

        fn run_new_thread_stream(
            &self,
            cwd: &str,
            _prompt: &str,
            _options: ConversationTurnOptions,
            event_sender: ConversationStreamSender,
        ) -> Result<ConversationTurnTerminalReceipt> {
            crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
                &event_sender,
                "test-thread",
                cwd,
            )
        }

        fn run_turn_stream(
            &self,
            thread_id: &str,
            _prompt: &str,
            _options: ConversationTurnOptions,
            event_sender: ConversationStreamSender,
        ) -> Result<ConversationTurnTerminalReceipt> {
            crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
                &event_sender,
                thread_id,
                "/tmp/test-workspace",
            )
        }
    }

    #[test]
    fn resumed_thread_snapshot_uses_snapshot_workspace_for_review_lookup() {
        let runtime_port = Arc::new(FakeInteractiveTurnRuntimePort {
            snapshot: ConversationSnapshot {
                thread_id: "thread-1".to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/loaded-workspace".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            },
        });
        let review_repository = Arc::new(FakeReviewCenterRepository {
            requested_workspaces: Mutex::new(Vec::new()),
            thread_reviews: Mutex::new(vec![ReviewCenterThreadProjection::new(
                "thread-1",
                "review-1",
                "Manual review",
                "pending",
                "Need operator follow-up",
                "2026-07-06T10:00:00Z",
                "2026-07-06T10:01:00Z",
            )]),
            pending_inbox: Mutex::new(Vec::new()),
            history: Mutex::new(Vec::new()),
        });
        let service = ConversationService::new(runtime_port).with_review_center_read_service(
            ReviewCenterReadService::new("/tmp/launch-workspace", review_repository.clone()),
        );

        let snapshot = service
            .load_thread_snapshot("thread-1", "/tmp/launch-workspace")
            .expect("resumed thread snapshot should load");

        assert_eq!(snapshot.conversation.cwd, "/tmp/loaded-workspace");
        assert_eq!(snapshot.thread_review.len(), 1);
        assert_eq!(
            review_repository
                .requested_workspaces
                .lock()
                .expect("workspace tracker mutex poisoned")
                .as_slice(),
            ["/tmp/loaded-workspace"]
        );
    }

    #[test]
    fn resumed_thread_snapshot_uses_bound_workspace_when_snapshot_cwd_missing() {
        let runtime_port = Arc::new(FakeInteractiveTurnRuntimePort {
            snapshot: ConversationSnapshot {
                thread_id: "thread-1".to_string(),
                title: "Loaded thread".to_string(),
                cwd: String::new(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            },
        });
        let review_repository = Arc::new(FakeReviewCenterRepository {
            requested_workspaces: Mutex::new(Vec::new()),
            thread_reviews: Mutex::new(Vec::new()),
            pending_inbox: Mutex::new(Vec::new()),
            history: Mutex::new(Vec::new()),
        });
        let service = ConversationService::new(runtime_port).with_review_center_read_service(
            ReviewCenterReadService::new("/tmp/launch-workspace", review_repository.clone()),
        );

        let snapshot = service
            .load_thread_snapshot("thread-1", "/tmp/shell-workspace")
            .expect("resumed thread snapshot should load");

        assert!(snapshot.thread_review.is_empty());
        assert_eq!(
            review_repository
                .requested_workspaces
                .lock()
                .expect("workspace tracker mutex poisoned")
                .as_slice(),
            ["/tmp/shell-workspace"]
        );
    }

    #[test]
    fn resumed_thread_snapshot_keeps_loading_when_review_lookup_fails() {
        struct FailingReviewCenterRepository;

        impl ReviewCenterRepositoryPort for FailingReviewCenterRepository {
            fn load_thread_reviews(
                &self,
                _workspace_dir: &str,
                _thread_id: &str,
            ) -> Result<Vec<ReviewCenterThreadProjection>> {
                Err(anyhow::anyhow!("review db unavailable"))
            }

            fn load_pending_inbox(
                &self,
                _workspace_dir: &str,
            ) -> Result<Vec<ReviewCenterInboxItem>> {
                Ok(Vec::new())
            }

            fn load_recent_history(
                &self,
                _workspace_dir: &str,
            ) -> Result<Vec<ReviewCenterHistoryEntry>> {
                Ok(Vec::new())
            }

            fn upsert_thread_review(
                &self,
                _workspace_dir: &str,
                _review: &ReviewCenterThreadProjection,
            ) -> Result<()> {
                Ok(())
            }

            fn replace_pending_inbox(
                &self,
                _workspace_dir: &str,
                _inbox: &[ReviewCenterInboxItem],
            ) -> Result<()> {
                Ok(())
            }

            fn append_history_entry(
                &self,
                _workspace_dir: &str,
                _entry: &ReviewCenterHistoryEntry,
            ) -> Result<()> {
                Ok(())
            }
        }

        let runtime_port = Arc::new(FakeInteractiveTurnRuntimePort {
            snapshot: ConversationSnapshot {
                thread_id: "thread-1".to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/loaded-workspace".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            },
        });
        let service = ConversationService::new(runtime_port).with_review_center_read_service(
            ReviewCenterReadService::new(
                "/tmp/launch-workspace",
                Arc::new(FailingReviewCenterRepository),
            ),
        );

        let snapshot = service
            .load_thread_snapshot("thread-1", "/tmp/shell-workspace")
            .expect("review lookup failure should not block loading the conversation");

        assert!(snapshot.thread_review.is_empty());
        assert!(
            snapshot.conversation.runtime_notices.iter().any(
                |notice| notice.contains("review hydration unavailable: review db unavailable")
            )
        );
    }
    #[test]
    fn persist_review_center_approval_review_writes_thread_inbox_and_history() {
        let runtime_port = Arc::new(FakeInteractiveTurnRuntimePort {
            snapshot: ConversationSnapshot {
                thread_id: "thread-1".to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/loaded-workspace".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            },
        });
        let review_repository = Arc::new(FakeReviewCenterRepository::default());
        let service = ConversationService::new(runtime_port).with_review_center_read_service(
            ReviewCenterReadService::new("/tmp/launch-workspace", review_repository.clone()),
        );

        service
            .persist_review_center_approval_review_for_workspace(
                "/tmp/repo",
                "thread-1",
                &ConversationApprovalReview {
                    target_item_id: "tool-7".to_string(),
                    status: ConversationApprovalReviewStatus::Unknown(
                        "human_review_requested".to_string(),
                    ),
                    risk_level: Some("high".to_string()),
                    rationale: Some("Need operator follow-up".to_string()),
                },
            )
            .expect("approval review should persist");

        let stored_reviews = review_repository
            .thread_reviews
            .lock()
            .expect("thread review mutex poisoned")
            .clone();
        assert_eq!(stored_reviews.len(), 1);
        assert_eq!(stored_reviews[0].thread_id, "thread-1");
        assert_eq!(stored_reviews[0].review_id, "tool-7");
        assert_eq!(stored_reviews[0].review_label, "manual handoff");
        assert_eq!(stored_reviews[0].review_state, "waiting");
        assert_eq!(stored_reviews[0].review_summary, "Need operator follow-up");
        assert_eq!(
            stored_reviews[0].handoff_target.as_deref(),
            Some("operator")
        );
        assert_eq!(
            stored_reviews[0].handoff_note.as_deref(),
            Some("open review center inbox")
        );

        let inbox = review_repository
            .pending_inbox
            .lock()
            .expect("pending inbox mutex poisoned")
            .clone();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].review_id, "tool-7");
        assert_eq!(inbox[0].thread_id, "thread-1");
        assert_eq!(inbox[0].inbox_state, "waiting");
        assert_eq!(inbox[0].summary, "Need operator follow-up");
        assert_eq!(inbox[0].handoff_target.as_deref(), Some("operator"));

        let history = review_repository
            .history
            .lock()
            .expect("history mutex poisoned")
            .clone();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].review_id, "tool-7");
        assert_eq!(history[0].thread_id, "thread-1");
        assert_eq!(
            history[0].event_kind,
            "manual_handoff_human_review_requested"
        );
        assert_eq!(history[0].summary, "Need operator follow-up");
    }

    #[test]
    fn persist_review_center_approval_review_removes_terminal_inbox_and_preserves_requested_at() {
        let runtime_port = Arc::new(FakeInteractiveTurnRuntimePort {
            snapshot: ConversationSnapshot {
                thread_id: "thread-1".to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/loaded-workspace".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            },
        });
        let mut existing_review = ReviewCenterThreadProjection::new(
            "thread-1",
            "tool-7",
            "manual handoff",
            "waiting",
            "Need operator follow-up",
            "2026-07-06T10:00:00Z",
            "2026-07-06T10:01:00Z",
        );
        existing_review.handoff_target = Some("operator".to_string());
        existing_review.handoff_note = Some("open review center inbox".to_string());
        let review_repository = Arc::new(FakeReviewCenterRepository {
            requested_workspaces: Mutex::new(Vec::new()),
            thread_reviews: Mutex::new(vec![existing_review]),
            pending_inbox: Mutex::new(vec![ReviewCenterInboxItem::new(
                "tool-7",
                "thread-1",
                "waiting",
                "Need operator follow-up",
                "2026-07-06T10:00:00Z",
                "2026-07-06T10:01:00Z",
            )]),
            history: Mutex::new(Vec::new()),
        });
        let service = ConversationService::new(runtime_port).with_review_center_read_service(
            ReviewCenterReadService::new("/tmp/launch-workspace", review_repository.clone()),
        );

        service
            .persist_review_center_approval_review_for_workspace(
                "/tmp/repo",
                "thread-1",
                &ConversationApprovalReview {
                    target_item_id: "tool-7".to_string(),
                    status: ConversationApprovalReviewStatus::Approved,
                    risk_level: Some("high".to_string()),
                    rationale: Some("Approved by operator".to_string()),
                },
            )
            .expect("approved review should persist");

        let stored_reviews = review_repository
            .thread_reviews
            .lock()
            .expect("thread review mutex poisoned")
            .clone();
        assert_eq!(stored_reviews.len(), 1);
        assert_eq!(stored_reviews[0].review_label, "approval review");
        assert_eq!(stored_reviews[0].review_state, "approved");
        assert_eq!(stored_reviews[0].review_summary, "Approved by operator");
        assert_eq!(stored_reviews[0].requested_at, "2026-07-06T10:00:00Z");
        assert!(stored_reviews[0].handoff_target.is_none());
        assert!(stored_reviews[0].handoff_note.is_none());

        let inbox = review_repository
            .pending_inbox
            .lock()
            .expect("pending inbox mutex poisoned")
            .clone();
        assert!(inbox.is_empty());

        let history = review_repository
            .history
            .lock()
            .expect("history mutex poisoned")
            .clone();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].event_kind, "review_approved");
        assert_eq!(history[0].summary, "Approved by operator");
    }
}
