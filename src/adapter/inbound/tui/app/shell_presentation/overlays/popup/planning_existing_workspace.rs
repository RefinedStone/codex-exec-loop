// 학습 주석: existing workspace overlay는 planning runtime snapshot을 읽어 "이미 planning workspace가 있음" 화면을
// 구성합니다. Ready conversation snapshot을 우선하고, conversation이 없으면 PlanningServices runtime loader로 fallback합니다.
use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningServices};

// 학습 주석: ConversationState는 현재 shell이 이미 session/draft model을 갖고 있는지 알려 주는 source입니다.
// NativeTuiApp wrapper는 app-level entrypoint에서 workspace, conversation, planning services를 꺼내기 위해 필요합니다.
use super::super::super::super::{ConversationState, NativeTuiApp};
// 학습 주석: 최종 반환값은 planning init popup renderer가 소비하는 공통 overlay DTO입니다.
use super::super::PlanningInitOverlayView;
// 학습 주석: input builder는 workspace path와 snapshot을 presentation copy DTO로 낮춥니다. 이 파일은
// snapshot 선택만 책임지고 line copy 구성은 input/view 모듈로 넘깁니다.
use super::existing_workspace_inputs::build_existing_workspace_copy;
// 학습 주석: init_copy builder는 copy DTO를 header/summary/options/status/key lines로 변환합니다.
use super::init_copy::build_existing_workspace_overlay_view;

// 학습 주석: build_existing_workspace_overlay_view_for_app은 router가 호출하는 app-level entrypoint입니다.
// app에서 planning workspace path, conversation state, planning services를 꺼내 state-level builder에 넘깁니다.
pub(super) fn build_existing_workspace_overlay_view_for_app(
    // 학습 주석: app은 shell presentation의 root state입니다. 여기서는 immutable read만 해서 rendering 중
    // overlay state나 planning runtime을 mutate하지 않습니다.
    app: &NativeTuiApp,
) -> PlanningInitOverlayView {
    // 학습 주석: planning_workspace_directory는 Ready conversation이 있으면 그 workspace를, 아니면 current
    // shell workspace를 고릅니다. existing workspace 화면은 이 path를 사용자에게 보여 주고 snapshot lookup에도 사용합니다.
    let workspace_directory = app.planning_workspace_directory();
    build_existing_workspace_overlay_view_for_state(
        &app.conversation_state,
        &app.planning,
        &workspace_directory,
    )
}

// 학습 주석: state-level builder는 test하기 쉬운 pure-ish boundary입니다. app 전체 대신 conversation/planning/path만
// 받아 snapshot resolution, copy construction, view construction 순서를 고정합니다.
fn build_existing_workspace_overlay_view_for_state(
    // 학습 주석: conversation_state는 Ready 상태일 때 이미 refresh된 planning_runtime_snapshot을 제공할 수 있습니다.
    conversation_state: &ConversationState,
    // 학습 주석: planning services는 conversation snapshot이 없을 때 filesystem/runtime snapshot을 읽는 fallback capability입니다.
    planning: &PlanningServices,
    // 학습 주석: workspace_directory는 copy에도 그대로 들어가고 runtime loader에도 전달되는 화면의 기준 workspace입니다.
    workspace_directory: &str,
) -> PlanningInitOverlayView {
    // 학습 주석: snapshot 선택을 먼저 끝낸 뒤 copy builder로 넘깁니다. 이렇게 하면 line-copy module은
    // conversation/loading/fallback 정책을 알 필요 없이 snapshot 하나만 다룹니다.
    let snapshot =
        resolve_existing_workspace_snapshot(conversation_state, planning, workspace_directory);
    // 학습 주석: copy DTO는 raw snapshot 값을 popup에 필요한 labels/summaries로 낮추고, view builder는 이를
    // 실제 ratatui Line 목록으로 렌더링 가능한 형태로 바꿉니다.
    build_existing_workspace_overlay_view(build_existing_workspace_copy(
        workspace_directory,
        &snapshot,
    ))
}

// 학습 주석: resolve_existing_workspace_snapshot은 existing workspace 화면에서 가장 중요한 stale-data 정책입니다.
// Ready conversation이면 conversation model의 snapshot이 source of truth이고, Loading/Failed면 service loader를 사용합니다.
fn resolve_existing_workspace_snapshot(
    // 학습 주석: Ready conversation은 post-turn refresh나 planning controller가 이미 갱신한 snapshot을 들고 있습니다.
    conversation_state: &ConversationState,
    // 학습 주석: planning은 fallback loader입니다. conversation이 아직 준비되지 않은 startup/loading surface에서만 씁니다.
    planning: &PlanningServices,
    // 학습 주석: workspace_directory는 fallback loader가 어떤 planning workspace를 읽을지 결정합니다.
    workspace_directory: &str,
) -> PlanningRuntimeSnapshot {
    // 학습 주석: 이 match가 conversation snapshot 우선 정책을 명시합니다. Ready 상태에서 다시 filesystem을
    // 읽으면 방금 runtime reducer가 반영한 in-memory snapshot보다 오래된 값을 보여 줄 수 있습니다.
    match conversation_state {
        // 학습 주석: Ready branch는 clone으로 read-only snapshot을 view pipeline에 넘깁니다. conversation model은
        // 계속 동일한 snapshot을 소유하므로 rendering이 app state를 빼앗지 않습니다.
        ConversationState::Ready(conversation) => conversation.planning_runtime_snapshot.clone(),
        // 학습 주석: Loading/Failed에는 usable ConversationViewModel이 없으므로 runtime service에서 직접 읽습니다.
        // loader는 실패를 invalid snapshot으로 접어 popup이 panic 없이 warning copy를 표시하게 합니다.
        ConversationState::Loading | ConversationState::Failed(_) => planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory),
    }
}

#[cfg(test)]
// 학습 주석: tests는 snapshot source 우선순위를 고정합니다. existing workspace overlay는 같은 workspace라도
// Ready conversation과 Loading fallback에서 서로 다른 snapshot source를 사용해야 합니다.
mod tests {
    // 학습 주석: PlanningServices는 Arc-backed workspace port를 요구하므로 테스트도 production 생성 경로와
    // 같은 shape로 adapter를 주입합니다.
    use std::sync::Arc;

    // 학습 주석: test는 ConversationState::Ready 안에 직접 ConversationViewModel을 넣어 snapshot priority를 검증합니다.
    use super::super::super::super::super::{ConversationState, ConversationViewModel};
    // 학습 주석: private resolver를 직접 호출해 rendering copy와 무관하게 source selection policy만 확인합니다.
    use super::resolve_existing_workspace_snapshot;
    // 학습 주석: sample snapshot helper는 queue summary가 식별 가능한 Ready snapshot을 만들어 줍니다.
    use crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_snapshot;
    // 학습 주석: filesystem adapter는 fallback loader의 실제 invalid-snapshot path를 테스트하기 위해 사용합니다.
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    // 학습 주석: PlanningServices는 resolver fallback branch가 호출하는 runtime use case bundle입니다.
    use crate::application::service::planning::PlanningServices;

    #[test]
    // 학습 주석: Ready conversation이 있으면 workspace_directory 인자가 다른 path여도 conversation snapshot이
    // 우선되어야 합니다. 이 테스트는 existing workspace popup이 session-local planning state를 보존하는지 확인합니다.
    fn ready_state_prefers_conversation_snapshot() {
        // 학습 주석: draft conversation도 Ready 상태면 planning_runtime_snapshot을 들 수 있습니다.
        let mut conversation = ConversationViewModel::new_draft("/tmp/app".to_string());
        // 학습 주석: 구분 가능한 queue summary를 넣어 fallback loader 결과와 혼동되지 않게 합니다.
        let snapshot = sample_planning_runtime_snapshot(
            "Planning Context",
            "queue summary from ready conversation",
        );
        // 학습 주석: resolver가 이 snapshot을 clone해서 반환해야 합니다.
        conversation.replace_planning_runtime_snapshot(snapshot.clone());
        // 학습 주석: planning service는 이 branch에서 쓰이면 안 되지만 resolver signature를 맞추기 위해 제공합니다.
        let planning = PlanningServices::from_workspace_port(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));

        // 학습 주석: workspace_directory를 일부러 다른 path로 넘겨도 Ready snapshot priority가 이겨야 합니다.
        let resolved = resolve_existing_workspace_snapshot(
            &ConversationState::ready(conversation),
            &planning,
            "/tmp/other-workspace",
        );

        assert_eq!(resolved, snapshot);
    }

    #[test]
    // 학습 주석: Loading 상태에서는 conversation snapshot이 없으므로 runtime loader 결과를 사용해야 합니다.
    // nonexistent workspace도 invalid snapshot으로 접혀 반환되는지 비교합니다.
    fn loading_state_uses_runtime_loader() {
        // 학습 주석: filesystem-backed planning services를 만들어 실제 fallback loader와 같은 경로를 사용합니다.
        let planning = PlanningServices::from_workspace_port(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        // 학습 주석: 존재하지 않는 path를 사용해 loader가 invalid snapshot을 만들어내는 branch를 안정적으로 탑니다.
        let workspace_directory = "/tmp/nonexistent-planning-workspace";

        // 학습 주석: Loading state에서는 resolver가 planning.runtime.load_runtime_snapshot_or_invalid를 호출해야 합니다.
        let resolved = resolve_existing_workspace_snapshot(
            &ConversationState::Loading,
            &planning,
            workspace_directory,
        );

        assert_eq!(
            resolved,
            planning
                .runtime
                .load_runtime_snapshot_or_invalid(workspace_directory)
        );
    }
}
