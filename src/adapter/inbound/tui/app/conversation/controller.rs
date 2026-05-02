// 학습 주석: resumed session copy는 planning snapshot의 상태를 사람이 읽을 수 있는 status text로 바꿉니다.
// controller는 snapshot을 새로 싣고, 이 helper로 copy를 만든 뒤 conversation status에 붙입니다.
use super::super::planning::status_projection::build_resumed_session_status_text;
// 학습 주석: 이 controller extension은 `NativeTuiApp`의 conversation/startup state를 조정합니다. follow-up
// controls event까지 함께 가져오는 이유는 workspace 변경이 prompt/follow-up UI의 기준 디렉터리도 바꾸기 때문입니다.
use super::super::{ConversationState, FollowupControlEvent, NativeTuiApp, StartupState};
// 학습 주석: `PlanningRuntimeSnapshot`은 application planning runtime에서 읽은 queue/ledger/diagnostic 요약입니다.
// TUI conversation model은 이 snapshot을 들고 rendering과 resumed-session status를 결정합니다.
use crate::application::service::planning::PlanningRuntimeSnapshot;

// 학습 주석: 이 impl 블록은 conversation lifecycle 중 "어느 workspace의 planning runtime을 보여 줄 것인가"를
// 관리합니다. startup, draft conversation, resumed thread가 각각 workspace 기준을 가질 수 있어 한곳에서 동기화합니다.
impl NativeTuiApp {
    // 학습 주석: draft shell workspace sync는 아직 active thread가 없는 Ready conversation만 갱신합니다. 이미
    // thread가 붙은 대화는 session 자체의 workspace/planning snapshot을 보존해야 하므로 여기서 건드리지 않습니다.
    pub(crate) fn sync_draft_shell_workspace(&mut self, workspace_directory: &str) {
        // 학습 주석: should_refresh_draft는 "Ready + draft + workspace가 달라짐" 세 조건을 압축한 guard입니다.
        // startup shell이 cwd를 새로 확인했을 때 빈 draft만 현재 workspace에 맞춰 따라오게 합니다.
        let should_refresh_draft = matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                // 학습 주석: active thread가 없다는 것은 아직 app-server session에 attach되지 않은 local draft라는 뜻입니다.
                // 이때만 draft workspace를 shell workspace로 갱신해 새 턴이 올바른 directory에서 시작합니다.
                if !conversation.has_active_thread()
                    && conversation.draft_workspace_directory() != workspace_directory
        );
        // 학습 주석: 갱신할 draft가 없으면 follow-up controls와 planning snapshot을 그대로 둡니다. 불필요한
        // dispatch를 피하면 사용자가 보고 있는 resumed session status가 빈 draft 기준으로 덮이지 않습니다.
        if !should_refresh_draft {
            return;
        }

        // 학습 주석: follow-up controls는 ConversationViewModel 안의 draft workspace를 owning state로 다룹니다.
        // event를 통해 갱신해야 auto-follow prompt와 prompt composer가 같은 workspace label을 보게 됩니다.
        self.dispatch_followup_controls(FollowupControlEvent::DraftWorkspaceSynced {
            workspace_directory: workspace_directory.to_string(),
        });
        // 학습 주석: workspace가 바뀌면 planning queue/ledger snapshot도 달라질 수 있습니다. draft state를
        // 갱신한 직후 같은 workspace 기준으로 runtime snapshot을 다시 로드합니다.
        self.refresh_ready_conversation_planning_runtime_snapshot();
    }

    // 학습 주석: current_workspace_directory는 shell 자체가 인식하는 작업 디렉터리입니다. startup diagnostics가
    // 준비되면 그 값을 신뢰하고, 아직 startup 전이면 process cwd로 fallback해 early rendering도 경로를 가집니다.
    pub(crate) fn current_workspace_directory(&self) -> String {
        // 학습 주석: startup_state는 app launch diagnostics의 소유자입니다. Ready가 아니면 diagnostics가 없으므로
        // filesystem cwd를 읽어 최소한의 planning workspace fallback을 제공합니다.
        match &self.startup_state {
            StartupState::Ready(diagnostics) => diagnostics.workspace_path.clone(),
            _ => std::env::current_dir()
                // 학습 주석: current_dir의 PathBuf를 UI와 service request에서 쓰는 display string으로 변환합니다.
                // fallback 경로라서 canonicalization 같은 무거운 해석은 하지 않습니다.
                .map(|path| path.display().to_string())
                // 학습 주석: cwd 조회 실패는 startup 초기나 테스트 환경에서 치명적으로 취급하지 않습니다. "."는
                // planning snapshot loader가 invalid snapshot으로 다룰 수 있는 보수적 placeholder입니다.
                .unwrap_or_else(|_| ".".to_string()),
        }
    }

    // 학습 주석: planning_workspace_directory는 conversation이 실제로 바라보는 planning workspace를 고릅니다.
    // Ready conversation이면 session/draft model의 workspace를 우선하고, 그 외에는 shell workspace로 대체합니다.
    pub(crate) fn planning_workspace_directory(&self) -> String {
        // 학습 주석: resumed session은 startup cwd와 다른 workspace를 가질 수 있습니다. Ready branch에서
        // conversation 값을 우선해야 session list에서 연 대화의 planning context가 정확히 유지됩니다.
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.planning_workspace_directory().to_string()
            }
            // 학습 주석: Loading/Failed 상태에는 conversation model이 없으므로 startup/current workspace를 사용합니다.
            // 이 fallback은 planning overlay가 conversation 준비 전에도 empty/invalid state를 그릴 수 있게 합니다.
            _ => self.current_workspace_directory(),
        }
    }

    // 학습 주석: snapshot loader는 TUI controller에서 application service로 넘어가는 읽기 경계입니다. 실패를
    // exception처럼 올리지 않고 invalid snapshot으로 접어야 shell rendering이 계속 유지됩니다.
    pub(crate) fn load_planning_runtime_snapshot(
        &self,
        // 학습 주석: workspace_directory는 shell cwd가 아니라 caller가 선택한 planning context입니다. draft
        // refresh와 resumed session refresh가 각각 자신에게 맞는 directory를 넘깁니다.
        workspace_directory: &str,
    ) -> PlanningRuntimeSnapshot {
        self.planning
            .runtime
            // 학습 주석: `load_runtime_snapshot_or_invalid`는 파일 누락이나 parse 실패를 snapshot 내부의 invalid
            // 상태로 바꿉니다. 덕분에 TUI는 panic 대신 planning status warning을 표시할 수 있습니다.
            .load_runtime_snapshot_or_invalid(workspace_directory)
    }

    // 학습 주석: 현재 conversation 기준 workspace를 계산한 뒤 snapshot을 갱신하는 convenience wrapper입니다.
    // 대부분의 caller는 workspace 선택 규칙을 직접 알 필요 없이 이 메서드를 호출합니다.
    pub(crate) fn refresh_ready_conversation_planning_runtime_snapshot(&mut self) {
        // 학습 주석: 먼저 String으로 소유권을 확보해 두면 아래 mutable refresh 호출 중 `self` borrow가 겹치지 않습니다.
        // Rust borrow 제약을 피하는 동시에 workspace 선택을 한 번만 수행합니다.
        let workspace_directory = self.planning_workspace_directory();
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
    }

    // 학습 주석: 명시된 workspace로 Ready conversation의 planning snapshot을 갈아끼웁니다. caller가 이미
    // workspace를 알고 있는 startup/resume 경로에서 중복 계산 없이 정확한 snapshot을 주입할 수 있습니다.
    pub(crate) fn refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
        &mut self,
        // 학습 주석: 이 값은 snapshot load에만 쓰고 conversation의 workspace field를 직접 바꾸지는 않습니다.
        // workspace mutation은 follow-up controls나 lifecycle reducer 쪽 책임으로 남깁니다.
        workspace_directory: &str,
    ) {
        // 학습 주석: take_ready_conversation_state는 enum 안의 boxed conversation을 꺼내는 안전한 mutation pattern입니다.
        // Loading/Failed 상태면 snapshot을 넣을 대상이 없으므로 조용히 종료합니다.
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };
        // 학습 주석: conversation model이 snapshot을 소유해야 presentation layer가 매 render마다 service를 다시
        // 호출하지 않습니다. 여기서 최신 snapshot으로 교체해 다음 render tick이 새 planning status를 보게 합니다.
        conversation.replace_planning_runtime_snapshot(
            self.load_planning_runtime_snapshot(workspace_directory),
        );
        // 학습 주석: take로 비워 둔 conversation_state를 Ready로 되돌립니다. 이 복원 단계가 있어야 이후
        // input/runtime reducer가 같은 conversation model을 계속 사용할 수 있습니다.
        self.conversation_state = ConversationState::ready(conversation);
    }

    // 학습 주석: resumed session을 연 직후 planning context를 status line에 표면화합니다. snapshot 자체는
    // conversation에 이미 있고, 이 함수는 그 snapshot을 warning 포함 status copy로 바꿔 사용자에게 보여 줍니다.
    pub(crate) fn surface_resumed_session_planning_context(&mut self) {
        // 학습 주석: Ready conversation이 아니면 resumed session status를 붙일 대상이 없습니다. lifecycle loading
        // 중에 호출되어도 shell state를 손상시키지 않도록 early return합니다.
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };
        // 학습 주석: status copy는 planning snapshot의 validity, queue, repair 상태를 짧은 message/warning으로
        // 축약합니다. session을 열었을 때 사용자가 planning context mismatch를 즉시 알아차리게 하는 장치입니다.
        conversation.set_status_with_warnings(build_resumed_session_status_text(
            &conversation.planning_runtime_snapshot,
        ));
        // 학습 주석: status를 붙인 conversation을 다시 Ready state로 넣어 prompt composer, tail panel,
        // planning overlays가 모두 같은 resumed-session status를 읽게 합니다.
        self.conversation_state = ConversationState::ready(conversation);
    }
}
