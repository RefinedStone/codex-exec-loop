use super::akra_dashboard::{GameStaticPose, GameVisualState};
use crate::application::port::inbound::admin_debug_port::AdminDebugStage;

pub(super) const DEBUG_AGENT_COUNT: usize = 3;

pub(super) fn actor_progress(stage: AdminDebugStage, index: usize) -> u8 {
    let base: u8 = match stage {
        AdminDebugStage::Dispatching => 8,
        AdminDebugStage::Working => 44,
        AdminDebugStage::Reviewing => 72,
        AdminDebugStage::Blocked => 68,
        AdminDebugStage::Recovering => 36,
        AdminDebugStage::Delivering => 88,
        AdminDebugStage::Cleanup => 96,
        AdminDebugStage::Complete => 100,
        _ => 0,
    };
    base.saturating_sub((index as u8) * 7)
}

pub(super) fn stage_progress(index: usize, count: usize) -> usize {
    if count <= 1 {
        100
    } else {
        index * 100 / (count - 1)
    }
}

pub(super) fn visual_state_key(state: GameVisualState) -> &'static str {
    match state {
        GameVisualState::Idle => "idle",
        GameVisualState::Starting => "starting",
        GameVisualState::Working => "working",
        GameVisualState::AwaitingReview => "awaiting_review",
        GameVisualState::Blocked => "blocked",
        GameVisualState::Delivering => "delivering",
        GameVisualState::Cleanup => "cleanup",
    }
}

pub(super) fn static_pose(state: GameVisualState) -> GameStaticPose {
    match state {
        GameVisualState::Working => GameStaticPose::Laptop,
        GameVisualState::AwaitingReview | GameVisualState::Delivering => GameStaticPose::Callout,
        GameVisualState::Blocked => GameStaticPose::Alert,
        GameVisualState::Cleanup => GameStaticPose::Sit,
        GameVisualState::Idle | GameVisualState::Starting => GameStaticPose::Neutral,
    }
}

pub(super) fn visual_severity(state: GameVisualState) -> &'static str {
    match state {
        GameVisualState::Blocked => "danger",
        GameVisualState::Cleanup => "warning",
        GameVisualState::Starting
        | GameVisualState::AwaitingReview
        | GameVisualState::Delivering => "info",
        GameVisualState::Working => "success",
        GameVisualState::Idle => "muted",
    }
}

pub(super) fn visual_label(state: GameVisualState) -> &'static str {
    match state {
        GameVisualState::Idle => "대기",
        GameVisualState::Starting => "시작 준비",
        GameVisualState::Working => "작업 중",
        GameVisualState::AwaitingReview => "검토 대기",
        GameVisualState::Blocked => "차단됨",
        GameVisualState::Delivering => "배포 중",
        GameVisualState::Cleanup => "정리 중",
    }
}

pub(super) fn actor_summary(state: GameVisualState) -> &'static str {
    match state {
        GameVisualState::Idle => "다음 디버그 작업 대기",
        GameVisualState::Starting => "임대와 세션 계약 확인",
        GameVisualState::Working => "할당된 작업을 구현 중",
        GameVisualState::AwaitingReview => "완료 보고와 리뷰 결과 대기",
        GameVisualState::Blocked => "주입된 테스트 실패로 중단",
        GameVisualState::Delivering => "PR과 rebase 통합 진행",
        GameVisualState::Cleanup => "worktree 및 임대 정리",
    }
}

pub(super) fn actor_bubble(state: GameVisualState) -> &'static str {
    match state {
        GameVisualState::Idle => "준비 완료",
        GameVisualState::Starting => "출발!",
        GameVisualState::Working => "구현 중",
        GameVisualState::AwaitingReview => "검토 부탁!",
        GameVisualState::Blocked => "도움 필요!",
        GameVisualState::Delivering => "배포 중",
        GameVisualState::Cleanup => "정리할게요",
    }
}

pub(super) fn event_icon(stage: AdminDebugStage) -> &'static str {
    match stage {
        AdminDebugStage::Ready => "R",
        AdminDebugStage::Intake => "Q",
        AdminDebugStage::QueuePressure => "!",
        AdminDebugStage::Dispatching => "D",
        AdminDebugStage::Working => "W",
        AdminDebugStage::Reviewing => "V",
        AdminDebugStage::Blocked => "X",
        AdminDebugStage::Recovering => "H",
        AdminDebugStage::Delivering => "P",
        AdminDebugStage::Cleanup => "C",
        AdminDebugStage::Complete => "✓",
    }
}

pub(super) fn stage_severity(stage: AdminDebugStage) -> &'static str {
    match stage {
        AdminDebugStage::Blocked => "danger",
        AdminDebugStage::QueuePressure | AdminDebugStage::Cleanup => "warning",
        AdminDebugStage::Working | AdminDebugStage::Complete => "success",
        AdminDebugStage::Ready => "muted",
        _ => "info",
    }
}

#[derive(Clone, Copy)]
pub(super) struct DebugProfile {
    pub(super) agent_id: &'static str,
    pub(super) display_name: &'static str,
    pub(super) archetype: &'static str,
    pub(super) role: &'static str,
    pub(super) task_id: &'static str,
    pub(super) task_title: &'static str,
}

pub(super) fn debug_profile(index: usize) -> DebugProfile {
    [
        DebugProfile {
            agent_id: "fake-ui-worker",
            display_name: "민아",
            archetype: "Artificer",
            role: "UI 엔지니어",
            task_id: "fake-task-ui",
            task_title: "동적 워커 UX 구현",
        },
        DebugProfile {
            agent_id: "fake-runtime-worker",
            display_name: "준호",
            archetype: "Guardian",
            role: "루프 엔지니어",
            task_id: "fake-task-runtime",
            task_title: "하네스 상태 전이 검증",
        },
        DebugProfile {
            agent_id: "fake-review-worker",
            display_name: "소라",
            archetype: "Ranger",
            role: "리뷰 엔지니어",
            task_id: "fake-task-review",
            task_title: "배포 게이트 검사",
        },
    ][index % DEBUG_AGENT_COUNT]
}
