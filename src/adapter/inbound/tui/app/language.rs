use crate::core::app::QueueAuthorityLoadError;
use crate::domain::conversation::ConversationReasoningEffort;
use crate::domain::planning::PlanningResetTarget;
use crate::domain::recent_sessions::SessionCatalogTier;

use super::inline_shell_commands::is_turn_option_clear_argument;
use super::parallel_mode_shell_command::{
    ParsedParallelModeShellCommand, parse_parallel_mode_shell_argument,
};
use super::planning_overlay_shell_command::parse_planning_overlay_shell_argument;
use super::planning_reset_shell_command::parse_planning_reset_shell_argument;
use super::planning_shell_command::{ParsedPlanningShellCommand, parse_planning_shell_argument};
use super::progressive_activity_overlay_ui::{
    parse_progressive_activity_card_filter, parse_progressive_activity_detail_kind,
};
use super::queue_overlay_ui::{QueueActionBlockReason, QueueMutationKind};
use super::view_selection_overlay_ui::ConversationViewMode;
use super::{
    InlineShellCommand, InlineShellCommandAvailability, InlineShellCommandAvailabilityReason,
    ShellActionAvailability,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum TuiLanguage {
    #[default]
    English,
    Korean,
}

pub(super) struct WorkCenterLocalizedCopy {
    pub(super) subtitle: &'static str,
    pub(super) no_task: &'static str,
    pub(super) no_active_turn: &'static str,
    pub(super) no_terminal: &'static str,
    pub(super) no_approval: &'static str,
    pub(super) unknown: &'static str,
    pub(super) selected_prefix: &'static str,
    pub(super) keys_navigation: &'static str,
    pub(super) keys_direct: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LanguageSelectionOption {
    pub(super) language: TuiLanguage,
    pub(super) label: &'static str,
    pub(super) detail: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct LanguageSelectionOverlayUiState {
    selected_language_index: usize,
}

pub(super) const LANGUAGE_SELECTION_OPTIONS: &[LanguageSelectionOption] = &[
    LanguageSelectionOption {
        language: TuiLanguage::English,
        label: "English",
        detail: "Use English for TUI system messages.",
    },
    LanguageSelectionOption {
        language: TuiLanguage::Korean,
        label: "한국어",
        detail: "TUI 시스템 메시지를 한국어로 표시합니다.",
    },
];
pub(super) const TUI_LOCALIZED_IMPORTANT_MARKERS: &[&str] =
    &["차단", "실패", "오류", "완료", "병합", "보류"];

impl TuiLanguage {
    pub(super) const SUPPORTED_LABELS: &'static str = "english, korean";

    pub(super) const fn work_center_copy(self) -> WorkCenterLocalizedCopy {
        match self {
            Self::English => WorkCenterLocalizedCopy {
                subtitle: "Read-only authority summary · Enter opens the selected detail surface",
                no_task: "No ready conversation projection",
                no_active_turn: "no active turn",
                no_terminal: "No terminal activity recorded",
                no_approval: "No runtime approval is waiting",
                unknown: "unknown",
                selected_prefix: "Selected",
                keys_navigation: "Keys · ↑↓ / jk select · Enter drill in · Esc close",
                keys_direct: "A activity · V agents · T terminal · R reviews · D delivery",
            },
            Self::Korean => WorkCenterLocalizedCopy {
                subtitle: "읽기 전용 권한 요약 · Enter로 선택한 상세 화면 열기",
                no_task: "준비된 대화 투영이 없습니다",
                no_active_turn: "활성 턴 없음",
                no_terminal: "기록된 터미널 활동이 없습니다",
                no_approval: "대기 중인 런타임 승인이 없습니다",
                unknown: "알 수 없음",
                selected_prefix: "선택",
                keys_navigation: "키 · ↑↓ / jk 선택 · Enter 상세 · Esc 닫기",
                keys_direct: "A 활동 · V 에이전트 · T 터미널 · R 리뷰 · D 전달",
            },
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "english" | "en" | "eng" => Some(Self::English),
            "korean" | "ko" | "kor" | "kr" | "한국어" | "한글" => Some(Self::Korean),
            _ => None,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Korean => "한국어",
        }
    }

    pub(super) const fn status_label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Korean => "Korean",
        }
    }

    pub(super) fn terminal_copy_requested(self, selection: bool, character_count: usize) -> String {
        match (self, selection) {
            (Self::English, true) => {
                format!("copied selection to terminal clipboard ({character_count} characters)")
            }
            (Self::English, false) => {
                format!("copied last answer to terminal clipboard ({character_count} characters)")
            }
            (Self::Korean, true) => {
                format!("선택 영역을 터미널 클립보드로 복사했습니다 ({character_count}자)")
            }
            (Self::Korean, false) => {
                format!("마지막 답변을 터미널 클립보드로 복사했습니다 ({character_count}자)")
            }
        }
    }

    pub(super) fn clipboard_image_attached(self, display_name: &str) -> String {
        match self {
            Self::English => format!("attached clipboard image ({display_name})"),
            Self::Korean => format!("클립보드 이미지를 첨부했습니다 ({display_name})"),
        }
    }

    pub(super) fn terminal_copy_unavailable(self, selection: bool) -> String {
        match (self, selection) {
            (Self::English, true) => {
                "nothing is selected; drag across transcript text first".to_string()
            }
            (Self::Korean, true) => {
                "선택 영역이 없습니다. 먼저 대화문 텍스트를 드래그하세요".to_string()
            }
            (Self::English, false) => "no assistant answer is available to copy".to_string(),
            (Self::Korean, false) => "복사할 assistant 답변이 없습니다".to_string(),
        }
    }

    pub(super) fn terminal_copy_usage(self) -> String {
        match self {
            Self::English => "supported forms: :copy, :copy selection, :copy last".to_string(),
            Self::Korean => "지원 명령: :copy, :copy selection, :copy last".to_string(),
        }
    }

    pub(super) const fn language_set_status(self) -> &'static str {
        match self {
            Self::English => "language set to English",
            Self::Korean => "언어가 한국어로 설정되었습니다.",
        }
    }

    pub(super) const fn turn_starting_prompt_hint(self, buffered: bool) -> &'static str {
        match (self, buffered) {
            (Self::English, false) => "prompt: wait for turn start  |  type now",
            (Self::English, true) => {
                "buffered prompt  |  wait for turn start  |  Enter queues after start  |  Ctrl+j nl"
            }
            (Self::Korean, false) => "프롬프트: 턴 시작 대기 중  |  지금 입력 가능",
            (Self::Korean, true) => {
                "입력된 프롬프트  |  턴 시작 대기  |  시작 후 Enter 큐 등록  |  Ctrl+j 줄바꿈"
            }
        }
    }

    pub(super) const fn composer_placeholder(self) -> &'static str {
        match self {
            Self::English => "Describe a task or type : for commands",
            Self::Korean => "작업을 입력하거나 : 명령을 사용하세요",
        }
    }

    pub(super) const fn composer_empty_action(self) -> &'static str {
        match self {
            Self::English => "Type a task  |  : commands",
            Self::Korean => "작업 입력  |  : 명령",
        }
    }

    pub(super) const fn composer_send_action(self) -> &'static str {
        match self {
            Self::English => "Enter send  |  Ctrl+J newline",
            Self::Korean => "Enter 전송  |  Ctrl+J 줄바꿈",
        }
    }

    pub(super) const fn composer_streaming_empty_action(self) -> &'static str {
        match self {
            Self::English => "Type a follow-up  |  Tab steers after typing",
            Self::Korean => "후속 작업 입력  |  입력 후 Tab으로 전달",
        }
    }

    pub(super) const fn composer_streaming_buffered_action(self) -> &'static str {
        match self {
            Self::English => "Enter queue  |  Tab steer  |  Ctrl+J newline",
            Self::Korean => "Enter 큐  |  Tab 전달  |  Ctrl+J 줄바꿈",
        }
    }

    pub(super) const fn composer_palette_action(
        self,
        has_matches: bool,
        selected_is_ready: bool,
        selected_requires_argument: bool,
    ) -> &'static str {
        match (
            self,
            has_matches,
            selected_is_ready,
            selected_requires_argument,
        ) {
            (Self::English, false, _, _) => "Esc close",
            (Self::English, true, false, _) => {
                "↑/↓ or Tab select  |  Enter unavailable  |  Esc close"
            }
            (Self::English, true, true, true) => "↑/↓ or Tab select  |  Enter insert  |  Esc close",
            (Self::English, true, true, false) => "↑/↓ or Tab select  |  Enter run  |  Esc close",
            (Self::Korean, false, _, _) => "Esc 닫기",
            (Self::Korean, true, false, _) => "↑/↓ 또는 Tab 선택  |  Enter 사용 불가  |  Esc 닫기",
            (Self::Korean, true, true, true) => "↑/↓ 또는 Tab 선택  |  Enter 입력  |  Esc 닫기",
            (Self::Korean, true, true, false) => "↑/↓ 또는 Tab 선택  |  Enter 실행  |  Esc 닫기",
        }
    }

    pub(super) const fn composer_startup_blocked_action(self) -> &'static str {
        match self {
            Self::English => "Ctrl+D diagnostics  |  draft preserved",
            Self::Korean => "Ctrl+D 진단  |  초안 유지",
        }
    }

    pub(super) const fn composer_parallel_loading_action(self) -> &'static str {
        match self {
            Self::English => "Parallel board loading  |  input paused",
            Self::Korean => "병렬 보드 로딩 중  |  입력 일시 정지",
        }
    }

    pub(super) const fn composer_approval_action(self) -> &'static str {
        match self {
            Self::English => "Input paused  •  Y approve once  •  N / Esc decline",
            Self::Korean => "입력 일시 정지  •  Y 한 번 승인  •  N / Esc 거절",
        }
    }

    pub(super) const fn composer_loading_status(self) -> &'static str {
        match self {
            Self::English => "Preparing the prompt…",
            Self::Korean => "입력 화면을 준비하는 중…",
        }
    }

    pub(super) const fn composer_loading_action(self) -> &'static str {
        match self {
            Self::English => "Wait for shell readiness",
            Self::Korean => "셸 준비가 끝날 때까지 기다리세요",
        }
    }

    pub(super) const fn composer_unavailable_status(self) -> &'static str {
        match self {
            Self::English => "Prompt unavailable",
            Self::Korean => "입력을 사용할 수 없습니다",
        }
    }

    pub(super) const fn composer_startup_pending_action(self) -> &'static str {
        match self {
            Self::English => "Type now  |  submission waits for startup",
            Self::Korean => "지금 입력 가능  |  시작 준비 후 전송",
        }
    }

    pub(super) fn manual_prompt_queued_status(
        self,
        task_id: &str,
        revision: i64,
        undo_available: bool,
    ) -> String {
        match (self, undo_available) {
            (Self::English, true) => {
                format!("queued task {task_id} / revision {revision} / undo available")
            }
            (Self::English, false) => format!("queued task {task_id} / revision {revision}"),
            (Self::Korean, true) => {
                format!("작업 {task_id} 큐 등록 완료 / 리비전 {revision} / 되돌리기 가능")
            }
            (Self::Korean, false) => {
                format!("작업 {task_id} 큐 등록 완료 / 리비전 {revision}")
            }
        }
    }

    pub(super) const fn manual_prompt_queue_pending_status(self) -> &'static str {
        match self {
            Self::English => "queue registration is already being prepared; steer was not opened",
            Self::Korean => "큐 등록을 준비 중이므로 현재 턴 전달을 열지 않았습니다.",
        }
    }

    pub(super) fn queue_mutation_pending_feedback(self, operation_id: u64) -> String {
        match self {
            Self::English => {
                format!("Queue change op-{operation_id} is waiting for authority acknowledgement.")
            }
            Self::Korean => format!("큐 변경 op-{operation_id}의 권한 확인을 기다리는 중입니다."),
        }
    }

    pub(super) const fn queue_mutation_refresh_required_feedback(self) -> &'static str {
        match self {
            Self::English => {
                "Queue authority needs refresh; close and reopen the queue before changing it."
            }
            Self::Korean => "큐 권한을 새로 확인해야 합니다. 큐를 닫았다가 다시 연 뒤 변경하세요.",
        }
    }

    pub(super) fn queue_overlay_authority_loading_summary(self, request_id: u64) -> String {
        match self {
            Self::English => {
                format!("queue authority load-{request_id} in progress; rows remain read-only")
            }
            Self::Korean => {
                format!("큐 권한 load-{request_id} 확인 중; 행은 읽기 전용으로 유지됩니다")
            }
        }
    }

    pub(super) const fn queue_overlay_authority_pending_summary(self) -> &'static str {
        match self {
            Self::English => "queue authority load is preparing; rows remain read-only",
            Self::Korean => "큐 권한 확인 준비 중; 행은 읽기 전용으로 유지됩니다",
        }
    }

    pub(super) fn queue_overlay_authority_failed_summary(
        self,
        request_id: u64,
        error: &str,
    ) -> String {
        match self {
            Self::English => {
                format!("queue authority load-{request_id} failed: {error}")
            }
            Self::Korean => format!("큐 권한 load-{request_id} 실패: {error}"),
        }
    }

    pub(super) const fn queue_overlay_authority_loading_feedback(self) -> &'static str {
        match self {
            Self::English => "Queue authority is still loading; remove and undo remain disabled.",
            Self::Korean => "큐 권한을 확인 중입니다. 제거와 되돌리기는 계속 비활성화됩니다.",
        }
    }

    pub(super) const fn queue_overlay_authority_loading_disabled_key_line(self) -> &'static str {
        match self {
            Self::English => "authority loading: remove/undo disabled",
            Self::Korean => "권한 확인 중: 제거/되돌리기 비활성화",
        }
    }

    pub(super) const fn queue_overlay_authority_failed_disabled_key_line(self) -> &'static str {
        match self {
            Self::English => "authority unavailable: close and reopen to retry",
            Self::Korean => "권한 확인 실패: 닫았다가 다시 열어 재시도",
        }
    }

    pub(super) fn queue_overlay_authority_load_error(
        self,
        error: &QueueAuthorityLoadError,
    ) -> String {
        self.queue_mutation_authority_refresh_error(error)
    }

    pub(super) fn queue_mutation_authority_refresh_error(
        self,
        error: &QueueAuthorityLoadError,
    ) -> String {
        match (self, error) {
            (Self::English, QueueAuthorityLoadError::AuthorityUnavailable(detail)) => {
                format!("Queue authority is unavailable: {detail}")
            }
            (Self::Korean, QueueAuthorityLoadError::AuthorityUnavailable(detail)) => {
                format!("큐 권한을 사용할 수 없습니다: {detail}")
            }
            (
                Self::English,
                QueueAuthorityLoadError::RevisionsKeptChanging {
                    projection_revision,
                    authority_revision,
                },
            ) => format!(
                "Queue authority kept changing while refreshing (projection revision {projection_revision}, authority revision {authority_revision}); reopen the queue."
            ),
            (
                Self::Korean,
                QueueAuthorityLoadError::RevisionsKeptChanging {
                    projection_revision,
                    authority_revision,
                },
            ) => format!(
                "새로고침 중 큐 권한이 계속 변경되었습니다 (projection 리비전 {projection_revision}, 권한 리비전 {authority_revision}). 큐를 다시 여세요."
            ),
            (Self::English, QueueAuthorityLoadError::RuntimeProjectionUnavailable) => {
                "Queue runtime projection is unavailable after the authority change; reopen the queue."
                    .to_string()
            }
            (Self::Korean, QueueAuthorityLoadError::RuntimeProjectionUnavailable) => {
                "권한 변경 후 큐 runtime projection을 사용할 수 없습니다. 큐를 다시 여세요."
                    .to_string()
            }
        }
    }

    pub(super) const fn queue_mutation_selected_item_changed_feedback(self) -> &'static str {
        match self {
            Self::English => "The selected queue item changed; reopen the queue to refresh it.",
            Self::Korean => "선택한 큐 항목이 변경되었습니다. 큐를 다시 열어 새로 확인하세요.",
        }
    }

    pub(super) fn queue_mutation_committed_refresh_failed(
        self,
        operation_id: u64,
        refresh_error: &str,
    ) -> String {
        match self {
            Self::English => format!(
                "Queue change op-{operation_id} committed, but authority refresh failed: {refresh_error}"
            ),
            Self::Korean => {
                format!("큐 변경 op-{operation_id} 커밋 완료 / 권한 새로고침 실패: {refresh_error}")
            }
        }
    }

    pub(super) fn queue_mutation_unresolved_refresh_failed(
        self,
        operation_id: u64,
        mutation_error: &str,
        refresh_error: &str,
    ) -> String {
        match self {
            Self::English => format!(
                "Queue change op-{operation_id} is unresolved: {mutation_error}; authority refresh failed: {refresh_error}"
            ),
            Self::Korean => format!(
                "큐 변경 op-{operation_id} 미확정: {mutation_error}; 권한 새로고침 실패: {refresh_error}"
            ),
        }
    }

    pub(super) fn queue_mutation_reconcile_failed(self, operation_id: u64, error: &str) -> String {
        match self {
            Self::English => format!(
                "Queue change op-{operation_id} could not reconcile its authority acknowledgement: {error}"
            ),
            Self::Korean => {
                format!("큐 변경 op-{operation_id}의 권한 확인 결과를 반영하지 못했습니다: {error}")
            }
        }
    }

    pub(super) const fn queue_mutation_projection_revision_missing(self) -> &'static str {
        match self {
            Self::English => "the refreshed projection has no planning revision",
            Self::Korean => "새로고침한 projection에 계획 리비전이 없습니다",
        }
    }

    pub(super) fn queue_mutation_completion_older_than_planning(
        self,
        planning_revision: i64,
    ) -> String {
        match self {
            Self::English => format!(
                "completion revision {planning_revision} is older than the visible planning revision"
            ),
            Self::Korean => format!(
                "완료 리비전 {planning_revision}이 현재 표시된 계획 리비전보다 오래되었습니다"
            ),
        }
    }

    pub(super) fn queue_mutation_completion_older_than_receipt(
        self,
        planning_revision: i64,
    ) -> String {
        match self {
            Self::English => format!(
                "completion revision {planning_revision} is older than the visible queue receipt"
            ),
            Self::Korean => format!(
                "완료 리비전 {planning_revision}이 현재 표시된 큐 receipt보다 오래되었습니다"
            ),
        }
    }

    pub(super) fn queue_mutation_projection_authority_revision_mismatch(
        self,
        projection_revision: i64,
        authority_revision: i64,
    ) -> String {
        match self {
            Self::English => format!(
                "projection revision {projection_revision} does not match authority revision {authority_revision}"
            ),
            Self::Korean => format!(
                "projection 리비전 {projection_revision}과 권한 리비전 {authority_revision}이 일치하지 않습니다"
            ),
        }
    }

    pub(super) const fn queue_mutation_rows_mismatch_authority(self) -> &'static str {
        match self {
            Self::English => "the refreshed queue rows do not match task authority",
            Self::Korean => "새로고침한 큐 행이 작업 권한과 일치하지 않습니다",
        }
    }

    pub(super) const fn queue_mutation_success_label(
        self,
        kind: QueueMutationKind,
    ) -> &'static str {
        match (self, kind) {
            (Self::English, QueueMutationKind::RemoveSelected) => "Removed selected queue item",
            (Self::English, QueueMutationKind::UndoLatestRegistration) => {
                "Undid latest queue registration"
            }
            (Self::Korean, QueueMutationKind::RemoveSelected) => "선택한 큐 항목 제거",
            (Self::Korean, QueueMutationKind::UndoLatestRegistration) => "최근 큐 등록 되돌리기",
        }
    }

    pub(super) fn queue_mutation_acknowledged(
        self,
        operation_id: u64,
        success_label: &str,
        task_count: usize,
        planning_revision: i64,
    ) -> String {
        match self {
            Self::English => format!(
                "op-{operation_id} acknowledged / {success_label}: {task_count} task(s) marked Cancelled / revision {planning_revision}"
            ),
            Self::Korean => format!(
                "op-{operation_id} 확인 완료 / {success_label}: 작업 {task_count}개를 취소로 변경 / 리비전 {planning_revision}"
            ),
        }
    }

    pub(super) fn queue_mutation_acknowledged_without_confirmation(
        self,
        operation_id: u64,
    ) -> String {
        match self {
            Self::English => format!(
                "Queue change op-{operation_id} was acknowledged, but the refreshed authority did not confirm every cancellation; review the queue."
            ),
            Self::Korean => format!(
                "큐 변경 op-{operation_id} 확인 완료 / 새 권한에서 모든 취소를 확인할 수 없습니다. 큐를 검토하세요."
            ),
        }
    }

    pub(super) fn queue_mutation_authority_confirmed_after_error(
        self,
        operation_id: u64,
        error: &str,
    ) -> String {
        match self {
            Self::English => format!(
                "op-{operation_id} authority confirmed cancellation after the worker reported an error: {error}"
            ),
            Self::Korean => format!("op-{operation_id} 작업 오류 후 권한에서 취소 확인: {error}"),
        }
    }

    pub(super) fn queue_mutation_rejected(self, operation_id: u64, error: &str) -> String {
        match self {
            Self::English => {
                format!("op-{operation_id} rejected / Queue change rejected: {error}")
            }
            Self::Korean => format!("op-{operation_id} 거부 / 큐 변경 거부: {error}"),
        }
    }

    pub(super) fn queue_mutation_pending_summary(self, operation_id: u64) -> String {
        match self {
            Self::English => {
                format!("op-{operation_id} | authority acknowledgement pending")
            }
            Self::Korean => format!("op-{operation_id} | 권한 확인 대기 중"),
        }
    }

    pub(super) const fn queue_mutation_refresh_required_summary(self) -> &'static str {
        match self {
            Self::English => "queue authority refresh required",
            Self::Korean => "큐 권한 새로고침 필요",
        }
    }

    pub(super) fn queue_mutation_pending_disabled_key_line(self, operation_id: u64) -> String {
        match self {
            Self::English => format!("op-{operation_id} pending: remove/undo disabled"),
            Self::Korean => format!("op-{operation_id} 대기 중: 제거/되돌리기 비활성화"),
        }
    }

    pub(super) const fn queue_mutation_refresh_disabled_key_line(self) -> &'static str {
        match self {
            Self::English => "remove/undo disabled: close and reopen to refresh",
            Self::Korean => "제거/되돌리기 비활성화: 닫았다가 다시 열어 새로고침",
        }
    }

    pub(super) const fn queue_overlay_select_key_line(self) -> &'static str {
        match self {
            Self::English => "Up/Down, j/k: select",
            Self::Korean => "Up/Down, j/k: 선택",
        }
    }

    pub(super) const fn queue_overlay_remove_key_line(self, undo_available: bool) -> &'static str {
        match (self, undo_available) {
            (Self::English, false) => "x/Delete: remove",
            (Self::English, true) => "x/Delete: remove | u: undo added",
            (Self::Korean, false) => "x/Delete: 제거",
            (Self::Korean, true) => "x/Delete: 제거 | u: 등록 되돌리기",
        }
    }

    pub(super) fn queue_overlay_remove_confirmation_line(self, task_id: &str) -> String {
        match self {
            Self::English => format!("remove {task_id}?"),
            Self::Korean => format!("{task_id} 제거할까요?"),
        }
    }

    pub(super) const fn queue_overlay_remove_confirmation_key_line(self) -> &'static str {
        match self {
            Self::English => "Enter/x/Delete: confirm remove",
            Self::Korean => "Enter/x/Delete: 제거 확인",
        }
    }

    pub(super) const fn queue_overlay_undo_only_key_line(self) -> &'static str {
        match self {
            Self::English => "u: undo added",
            Self::Korean => "u: 등록 되돌리기",
        }
    }

    pub(super) const fn queue_action_block_reason(
        self,
        reason: QueueActionBlockReason,
    ) -> &'static str {
        match (self, reason) {
            (Self::English, QueueActionBlockReason::ParallelModeOwnsTaskLeases) => {
                "queue changes are disabled while parallel mode owns task leases"
            }
            (Self::English, QueueActionBlockReason::PostTurnPlanningInFlight) => {
                "wait for post-turn planning to finish"
            }
            (Self::English, QueueActionBlockReason::ActiveTurnInFlight) => {
                "wait for the active turn to finish"
            }
            (Self::English, QueueActionBlockReason::ConversationNotReady) => {
                "queue changes require a ready conversation"
            }
            (Self::English, QueueActionBlockReason::AuthoritySnapshotChanged) => {
                "queue authority changed; wait for refresh"
            }
            (Self::English, QueueActionBlockReason::SelectedItemUnavailable) => {
                "select an actionable queue item"
            }
            (Self::Korean, QueueActionBlockReason::ParallelModeOwnsTaskLeases) => {
                "병렬 모드가 작업 임대를 소유하는 동안 큐를 변경할 수 없습니다"
            }
            (Self::Korean, QueueActionBlockReason::PostTurnPlanningInFlight) => {
                "턴 이후 계획 처리가 끝날 때까지 기다리세요"
            }
            (Self::Korean, QueueActionBlockReason::ActiveTurnInFlight) => {
                "진행 중인 턴이 끝날 때까지 기다리세요"
            }
            (Self::Korean, QueueActionBlockReason::ConversationNotReady) => {
                "준비된 대화에서만 큐를 변경할 수 있습니다"
            }
            (Self::Korean, QueueActionBlockReason::AuthoritySnapshotChanged) => {
                "큐 권한이 변경되었습니다. 새로고침을 기다리세요"
            }
            (Self::Korean, QueueActionBlockReason::SelectedItemUnavailable) => {
                "변경할 수 있는 큐 항목을 선택하세요"
            }
        }
    }

    pub(super) fn queue_overlay_remove_blocked_key_line(
        self,
        reason: QueueActionBlockReason,
    ) -> String {
        let reason = self.queue_action_block_reason(reason);
        match self {
            Self::English => format!("remove disabled: {reason}"),
            Self::Korean => format!("제거 비활성화: {reason}"),
        }
    }

    pub(super) fn queue_overlay_actions_blocked_key_line(
        self,
        reason: QueueActionBlockReason,
    ) -> String {
        let reason = self.queue_action_block_reason(reason);
        match self {
            Self::English => format!("remove/undo disabled: {reason}"),
            Self::Korean => format!("제거/되돌리기 비활성화: {reason}"),
        }
    }

    pub(super) fn queue_overlay_undo_blocked_key_line(
        self,
        reason: QueueActionBlockReason,
    ) -> String {
        let reason = self.queue_action_block_reason(reason);
        match self {
            Self::English => format!("undo disabled: {reason}"),
            Self::Korean => format!("되돌리기 비활성화: {reason}"),
        }
    }

    pub(super) const fn queue_overlay_close_key_line(self) -> &'static str {
        match self {
            Self::English => "Esc/Ctrl+C: close",
            Self::Korean => "Esc/Ctrl+C: 닫기",
        }
    }

    pub(super) fn queue_mutation_tail_pending_label(self, operation_id: u64) -> String {
        match self {
            Self::English => format!("queue: op-{operation_id}"),
            Self::Korean => format!("큐: op-{operation_id}"),
        }
    }

    pub(super) const fn queue_mutation_tail_pending_detail(self) -> &'static str {
        match self {
            Self::English => "  |  authority acknowledgement pending",
            Self::Korean => "  |  권한 확인 대기 중",
        }
    }

    pub(super) const fn queue_mutation_tail_refresh_label(self) -> &'static str {
        match self {
            Self::English => "queue: refresh required",
            Self::Korean => "큐: 새로고침 필요",
        }
    }

    pub(super) const fn queue_mutation_tail_refresh_detail(self) -> &'static str {
        match self {
            Self::English => "  |  open :queue before retrying",
            Self::Korean => "  |  다시 시도하기 전에 :queue 열기",
        }
    }

    pub(super) const fn turn_steer_confirmation_title(self) -> &'static str {
        match self {
            Self::English => "Steer Active Turn",
            Self::Korean => "현재 턴에 전달",
        }
    }

    pub(super) const fn turn_steer_confirmation_question(self) -> &'static str {
        match self {
            Self::English => "Send this exact draft into the active turn?",
            Self::Korean => "이 초안을 현재 실행 중인 턴에 정확히 전달할까요?",
        }
    }

    pub(super) const fn turn_steer_confirmation_keys(self) -> &'static str {
        match self {
            Self::English => "Enter/Tab: steer    Esc: keep draft",
            Self::Korean => "Enter/Tab: 전달    Esc: 초안 유지",
        }
    }

    pub(super) const fn turn_steer_preview_truncated(self) -> &'static str {
        match self {
            Self::English => "[preview truncated; exact draft will be sent]",
            Self::Korean => "[미리보기 생략됨 / 원본 초안이 전달됩니다]",
        }
    }

    pub(super) const fn session_rename_pending_feedback(self) -> &'static str {
        match self {
            Self::English => "Rename is pending; wait for app-server confirmation.",
            Self::Korean => "이름 변경 확인 중입니다. app-server 응답을 기다리세요.",
        }
    }

    pub(super) const fn session_rename_already_pending_feedback(self) -> &'static str {
        match self {
            Self::English => "Rename is already pending; wait for app-server confirmation.",
            Self::Korean => "이미 이름 변경을 확인 중입니다. app-server 응답을 기다리세요.",
        }
    }

    pub(super) const fn session_rename_empty_feedback(self) -> &'static str {
        match self {
            Self::English => "Name cannot be empty. Enter a title or press Esc to cancel.",
            Self::Korean => "이름은 비워둘 수 없습니다. 제목을 입력하거나 Esc로 취소하세요.",
        }
    }

    pub(super) const fn session_rename_working_feedback(self) -> &'static str {
        match self {
            Self::English => "Renaming session...",
            Self::Korean => "세션 이름 변경 중...",
        }
    }

    pub(super) fn session_rename_failed_feedback(self, reason: &str) -> String {
        match self {
            Self::English => {
                format!("Rename failed; draft kept. Enter retries, Esc cancels. {reason}")
            }
            Self::Korean => {
                format!("이름 변경 실패 / 초안 유지. Enter 재시도, Esc 취소. {reason}")
            }
        }
    }

    pub(super) const fn session_rename_select_status(self) -> &'static str {
        match self {
            Self::English => "select a session before renaming it",
            Self::Korean => "이름을 변경할 세션을 먼저 선택하세요.",
        }
    }

    pub(super) fn session_rename_started_status(self, name: &str) -> String {
        match self {
            Self::English => format!("renaming session: {name}"),
            Self::Korean => format!("세션 이름 변경 중: {name}"),
        }
    }

    pub(super) fn session_renamed_status(self, name: &str) -> String {
        match self {
            Self::English => format!("session renamed: {name}"),
            Self::Korean => format!("세션 이름 변경 완료: {name}"),
        }
    }

    pub(super) fn session_rename_failed_status(self, reason: &str) -> String {
        match self {
            Self::English => format!("session rename failed; draft kept / {reason}"),
            Self::Korean => format!("세션 이름 변경 실패 / 초안 유지 / {reason}"),
        }
    }

    pub(super) const fn session_rename_label(self) -> &'static str {
        match self {
            Self::English => "rename",
            Self::Korean => "새 이름",
        }
    }

    pub(super) const fn session_rename_key_lines(self, pending: bool) -> [&'static str; 2] {
        match (self, pending) {
            (Self::English, true) => [
                "Rename pending; the editor is locked until app-server responds.",
                "Wait for confirmation; duplicate submit and cancel are disabled.",
            ],
            (Self::English, false) => [
                "Type the session title directly. The selected thread id stays fixed.",
                "Enter: rename    Esc/Ctrl+C: cancel    Backspace: delete",
            ],
            (Self::Korean, true) => [
                "이름 변경 확인 중 / app-server 응답 전까지 편집이 잠깁니다.",
                "중복 제출과 취소는 확인이 끝날 때까지 비활성화됩니다.",
            ],
            (Self::Korean, false) => [
                "세션 제목을 입력하세요. 선택한 thread id는 바뀌지 않습니다.",
                "Enter: 변경    Esc/Ctrl+C: 취소    Backspace: 삭제",
            ],
        }
    }

    pub(super) const fn turn_steer_needs_prompt_status(self) -> &'static str {
        match self {
            Self::English => "type a prompt before steering the active turn",
            Self::Korean => "현재 턴에 전달할 프롬프트를 먼저 입력하세요.",
        }
    }

    pub(super) const fn turn_steer_unavailable_status(self) -> &'static str {
        match self {
            Self::English => "the active turn changed; draft kept",
            Self::Korean => "실행 중인 턴이 변경되어 초안을 유지했습니다.",
        }
    }

    pub(super) const fn turn_steer_pending_status(self) -> &'static str {
        match self {
            Self::English => "steer request pending; draft kept until app-server confirms it",
            Self::Korean => "현재 턴 전달 요청 확인 중 / app-server 확인 전까지 초안을 유지합니다.",
        }
    }

    pub(super) const fn turn_steer_cancelled_status(self) -> &'static str {
        match self {
            Self::English => "steer cancelled; draft kept for queue submission",
            Self::Korean => "현재 턴 전달을 취소했습니다. 초안은 큐 등록용으로 유지됩니다.",
        }
    }

    pub(super) fn turn_steer_succeeded_status(self, turn_id: &str) -> String {
        match self {
            Self::English => format!("steered into active turn {turn_id}"),
            Self::Korean => format!("현재 턴 {turn_id}에 전달했습니다."),
        }
    }

    pub(super) fn turn_steer_failed_status(self, reason: &str) -> String {
        match self {
            Self::English => format!("steer rejected; draft kept / {reason}"),
            Self::Korean => format!("현재 턴 전달이 거부되어 초안을 유지했습니다. / {reason}"),
        }
    }

    pub(super) const fn inline_shell_command_detail(
        self,
        command: InlineShellCommand,
    ) -> &'static str {
        match (self, command) {
            (Self::English, InlineShellCommand::Copy) => "copy transcript text",
            (Self::Korean, InlineShellCommand::Copy) => "대화문 텍스트 복사",
            (Self::English, InlineShellCommand::Diagnostics) => "diagnostics",
            (Self::English, InlineShellCommand::Work) => "unified work center",
            (Self::English, InlineShellCommand::Parallel) => "parallel mode",
            (Self::English, InlineShellCommand::Peek) => "parallel agent peek",
            (Self::English, InlineShellCommand::Activity) => "progressive activity cards",
            (Self::English, InlineShellCommand::Sessions) => "recent sessions",
            (Self::English, InlineShellCommand::Reviews) => "review center",
            (Self::English, InlineShellCommand::Queue) => "planning queue",
            (Self::English, InlineShellCommand::Directions) => "directions maintenance",
            (Self::English, InlineShellCommand::Turns) => "auto-follow opt-in; off or 0 disables",
            (Self::English, InlineShellCommand::Stop) => "stop active sessions",
            (Self::English, InlineShellCommand::Model) => "model and reasoning",
            (Self::English, InlineShellCommand::View) => "conversation view",
            (Self::English, InlineShellCommand::Language) => "TUI language",
            (Self::English, InlineShellCommand::Think) => "reasoning effort",
            (Self::English, InlineShellCommand::Doctor) => "planning health",
            (Self::English, InlineShellCommand::PlanningInit) => "planning control center",
            (Self::English, InlineShellCommand::Reset) => "planning reset",
            (Self::English, InlineShellCommand::NewDraft) => "new draft",
            (Self::English, InlineShellCommand::Help) => "command help",
            (Self::Korean, InlineShellCommand::Diagnostics) => "진단",
            (Self::Korean, InlineShellCommand::Work) => "통합 작업 센터",
            (Self::Korean, InlineShellCommand::Parallel) => "병렬 모드",
            (Self::Korean, InlineShellCommand::Peek) => "병렬 에이전트 보기",
            (Self::Korean, InlineShellCommand::Activity) => "활동 카드 목록",
            (Self::Korean, InlineShellCommand::Sessions) => "최근 세션",
            (Self::Korean, InlineShellCommand::Reviews) => "리뷰 센터",
            (Self::Korean, InlineShellCommand::Queue) => "계획 큐",
            (Self::Korean, InlineShellCommand::Directions) => "계획 지침 관리",
            (Self::Korean, InlineShellCommand::Turns) => "자동 후속 실행 설정; off 또는 0으로 끔",
            (Self::Korean, InlineShellCommand::Stop) => "실행 중인 세션 중지",
            (Self::Korean, InlineShellCommand::Model) => "모델 및 추론 수준",
            (Self::Korean, InlineShellCommand::View) => "대화 표시 방식",
            (Self::Korean, InlineShellCommand::Language) => "TUI 언어",
            (Self::Korean, InlineShellCommand::Think) => "추론 수준",
            (Self::Korean, InlineShellCommand::Doctor) => "계획 상태 점검",
            (Self::Korean, InlineShellCommand::PlanningInit) => "계획 제어 센터",
            (Self::Korean, InlineShellCommand::Reset) => "계획 상태 초기화",
            (Self::Korean, InlineShellCommand::NewDraft) => "새 초안",
            (Self::Korean, InlineShellCommand::Help) => "명령 도움말",
        }
    }

    pub(super) fn inline_command_palette_header(self, selected: usize, total: usize) -> String {
        match self {
            Self::English => format!("palette {selected}/{total}"),
            Self::Korean => format!("팔레트 {selected}/{total}"),
        }
    }

    pub(super) const fn inline_command_palette_detail_label(self) -> &'static str {
        match self {
            Self::English => "detail",
            Self::Korean => "상세",
        }
    }

    pub(super) const fn inline_command_palette_args_label(self) -> &'static str {
        match self {
            Self::English => "args",
            Self::Korean => "인수",
        }
    }

    pub(super) const fn startup_diagnostics_scroll_key_line(self) -> &'static str {
        match self {
            Self::English => "↑↓/jk: warnings    PgUp/PgDn: page    Home/End: bounds",
            Self::Korean => "↑↓/jk: 경고    PgUp/PgDn: 페이지    Home/End: 처음/끝",
        }
    }

    pub(super) const fn inline_command_availability_label(
        self,
        availability: InlineShellCommandAvailability,
    ) -> &'static str {
        match availability {
            InlineShellCommandAvailability::Ready => "READY",
            InlineShellCommandAvailability::Pending(_) => "PENDING",
            InlineShellCommandAvailability::Locked(_) => "LOCKED",
        }
    }

    pub(super) const fn inline_command_availability_reason(
        self,
        reason: InlineShellCommandAvailabilityReason,
    ) -> &'static str {
        match (self, reason) {
            (Self::English, InlineShellCommandAvailabilityReason::StartupChecksRunning) => {
                "startup checks are still running"
            }
            (
                Self::English,
                InlineShellCommandAvailabilityReason::StartupDiagnosticsNeedAttention,
            ) => "resolve startup diagnostics first",
            (Self::English, InlineShellCommandAvailabilityReason::ParallelTransitionInFlight) => {
                "parallel control transition is in progress"
            }
            (Self::English, InlineShellCommandAvailabilityReason::ParallelModeDisabled) => {
                "start parallel mode first"
            }
            (Self::English, InlineShellCommandAvailabilityReason::NoActiveParallelAgents) => {
                "available after an agent starts"
            }
            (Self::Korean, InlineShellCommandAvailabilityReason::StartupChecksRunning) => {
                "시작 검사가 진행 중입니다"
            }
            (
                Self::Korean,
                InlineShellCommandAvailabilityReason::StartupDiagnosticsNeedAttention,
            ) => "먼저 시작 진단을 해결하세요",
            (Self::Korean, InlineShellCommandAvailabilityReason::ParallelTransitionInFlight) => {
                "병렬 제어 전환이 진행 중입니다"
            }
            (Self::Korean, InlineShellCommandAvailabilityReason::ParallelModeDisabled) => {
                "먼저 병렬 모드를 시작하세요"
            }
            (Self::Korean, InlineShellCommandAvailabilityReason::NoActiveParallelAgents) => {
                "에이전트가 시작된 뒤 사용할 수 있습니다"
            }
        }
    }

    pub(super) const fn inline_command_argument_preview(
        self,
        command: InlineShellCommand,
    ) -> &'static str {
        match (self, command) {
            (Self::English, InlineShellCommand::Parallel) => "[off]",
            (Self::English, InlineShellCommand::Activity) => "[all|kind]",
            (Self::English, InlineShellCommand::Turns) => "<positive|infinite|off>",
            (Self::English, InlineShellCommand::Model) => "[default]",
            (Self::English, InlineShellCommand::View) => "[simple|medium|detail]",
            (Self::English, InlineShellCommand::Language) => "[english|korean]",
            (Self::English, InlineShellCommand::Think) => "<level|default>",
            (Self::English, InlineShellCommand::Copy) => "[selection|last]",
            (Self::English, InlineShellCommand::PlanningInit) => "[doctor]",
            (Self::English, InlineShellCommand::Reset) => "<queue|directions|all>",
            (Self::English, _) => "none",
            (Self::Korean, InlineShellCommand::Parallel) => "[off]",
            (Self::Korean, InlineShellCommand::Activity) => "[all|종류]",
            (Self::Korean, InlineShellCommand::Turns) => "<양수|infinite|off>",
            (Self::Korean, InlineShellCommand::Model) => "[default]",
            (Self::Korean, InlineShellCommand::View) => "[simple|medium|detail]",
            (Self::Korean, InlineShellCommand::Language) => "[english|korean]",
            (Self::Korean, InlineShellCommand::Think) => "<수준|default>",
            (Self::Korean, InlineShellCommand::Copy) => "[selection|last]",
            (Self::Korean, InlineShellCommand::PlanningInit) => "[doctor]",
            (Self::Korean, InlineShellCommand::Reset) => "<queue|directions|all>",
            (Self::Korean, _) => "없음",
        }
    }

    pub(super) const fn inline_command_expected_result(
        self,
        command: InlineShellCommand,
        parallel_mode_enabled: bool,
    ) -> &'static str {
        match (self, command) {
            (Self::English, InlineShellCommand::Copy) => "copy selection or latest answer",
            (Self::Korean, InlineShellCommand::Copy) => "선택 영역이나 마지막 답변 복사",
            (Self::English, InlineShellCommand::Diagnostics) => "open startup diagnostics",
            (Self::English, InlineShellCommand::Work) => {
                "open task, agent, terminal, approval, and delivery summary"
            }
            (Self::English, InlineShellCommand::Parallel) => "enable mode and open the board",
            (Self::English, InlineShellCommand::Peek) => "inspect active agent work",
            (Self::English, InlineShellCommand::Activity) => "inspect retained activity",
            (Self::English, InlineShellCommand::Sessions) if parallel_mode_enabled => {
                "open the operations board"
            }
            (Self::English, InlineShellCommand::Sessions) => "open recent sessions",
            (Self::English, InlineShellCommand::Reviews) => "load the review center",
            (Self::English, InlineShellCommand::Queue) => "open the accepted queue",
            (Self::English, InlineShellCommand::Directions) => "edit planning directions",
            (Self::English, InlineShellCommand::Turns) => "change auto-follow budget",
            (Self::English, InlineShellCommand::Stop) => "pause automation and stop sessions",
            (Self::English, InlineShellCommand::Model) => "choose model and reasoning level",
            (Self::English, InlineShellCommand::View) => "change transcript density",
            (Self::English, InlineShellCommand::Language) => "change TUI language",
            (Self::English, InlineShellCommand::Think) => "override reasoning effort",
            (Self::English, InlineShellCommand::Doctor) => "inspect planning health",
            (Self::English, InlineShellCommand::PlanningInit) => "open planning control center",
            (Self::English, InlineShellCommand::Reset) => "reset selected planning state",
            (Self::English, InlineShellCommand::NewDraft) => "open a clean draft",
            (Self::English, InlineShellCommand::Help) => "open command help",
            (Self::Korean, InlineShellCommand::Diagnostics) => "시작 진단 열기",
            (Self::Korean, InlineShellCommand::Work) => "작업·에이전트·터미널·승인·전달 요약 열기",
            (Self::Korean, InlineShellCommand::Parallel) => "병렬 모드와 운영 보드 시작",
            (Self::Korean, InlineShellCommand::Peek) => "활성 에이전트 작업 보기",
            (Self::Korean, InlineShellCommand::Activity) => "보존된 활동 보기",
            (Self::Korean, InlineShellCommand::Sessions) if parallel_mode_enabled => {
                "운영 보드 열기"
            }
            (Self::Korean, InlineShellCommand::Sessions) => "최근 세션 열기",
            (Self::Korean, InlineShellCommand::Reviews) => "리뷰 센터 불러오기",
            (Self::Korean, InlineShellCommand::Queue) => "수락된 큐 열기",
            (Self::Korean, InlineShellCommand::Directions) => "계획 지침 편집",
            (Self::Korean, InlineShellCommand::Turns) => "자동 진행 예산 변경",
            (Self::Korean, InlineShellCommand::Stop) => "자동화를 멈추고 세션 중지",
            (Self::Korean, InlineShellCommand::Model) => "모델과 추론 수준 선택",
            (Self::Korean, InlineShellCommand::View) => "대화 표시 밀도 변경",
            (Self::Korean, InlineShellCommand::Language) => "TUI 언어 변경",
            (Self::Korean, InlineShellCommand::Think) => "추론 수준 재정의",
            (Self::Korean, InlineShellCommand::Doctor) => "계획 상태 점검",
            (Self::Korean, InlineShellCommand::PlanningInit) => "계획 제어 센터 열기",
            (Self::Korean, InlineShellCommand::Reset) => "선택한 계획 상태 초기화",
            (Self::Korean, InlineShellCommand::NewDraft) => "새 초안 열기",
            (Self::Korean, InlineShellCommand::Help) => "명령 도움말 열기",
        }
    }

    pub(super) fn inline_command_palette_unavailable_status(
        self,
        command: InlineShellCommand,
        availability: InlineShellCommandAvailability,
    ) -> String {
        let label = self.inline_command_availability_label(availability);
        let reason = availability
            .reason()
            .map(|reason| self.inline_command_availability_reason(reason))
            .unwrap_or_default();
        match self {
            Self::English => format!("{} {label}; {reason}", command.command_name()),
            Self::Korean => format!("{} {label}; {reason}", command.command_name()),
        }
    }

    #[cfg(test)]
    pub(super) const fn inline_command_palette_key_lines(self) -> [&'static str; 2] {
        match self {
            Self::English => [
                "Up/Shift+Tab previous | Down/Tab next",
                "Enter choose | Esc close",
            ],
            Self::Korean => ["Up/Shift+Tab 이전 | Down/Tab 다음", "Enter 선택 | Esc 닫기"],
        }
    }

    #[cfg(test)]
    pub(super) const fn inline_command_palette_empty_key_line(self) -> &'static str {
        match self {
            Self::English => "Esc close",
            Self::Korean => "Esc 닫기",
        }
    }

    const fn inline_shell_command_base_buffered_hint(
        self,
        command: InlineShellCommand,
        english: &'static str,
    ) -> &'static str {
        if matches!(self, Self::English) {
            return english;
        }
        match command {
            InlineShellCommand::Diagnostics => "Enter로 진단 화면을 엽니다.",
            InlineShellCommand::Work => "Enter로 통합 작업 센터를 엽니다.",
            InlineShellCommand::Parallel => "Enter로 병렬 모드를 시작합니다.",
            InlineShellCommand::Peek => "Enter로 실행 중인 병렬 에이전트를 봅니다.",
            InlineShellCommand::Activity => {
                "`:activity [diff|output]`으로 보관된 턴 활동을 확인합니다."
            }
            InlineShellCommand::Sessions => "Enter로 최근 세션을 엽니다.",
            InlineShellCommand::Reviews => "Enter로 리뷰 센터를 엽니다.",
            InlineShellCommand::Queue => "Enter로 계획 큐를 엽니다.",
            InlineShellCommand::Directions => "Enter로 계획 지침을 검토하거나 편집합니다.",
            InlineShellCommand::Turns => {
                "`:turns <positive|infinite>`로 자동 후속 실행을 켜거나 `:turns off`로 끕니다."
            }
            InlineShellCommand::Stop => "Enter로 실행 중인 app-server 세션을 중지합니다.",
            InlineShellCommand::Model => "Enter로 모델과 추론 수준을 선택합니다.",
            InlineShellCommand::View => "Enter로 대화의 도구·상태 행 표시 방식을 선택합니다.",
            InlineShellCommand::Language => "Enter로 TUI 언어를 선택합니다.",
            InlineShellCommand::Think => "`:think <level>`로 추론 수준을 선택합니다.",
            InlineShellCommand::Copy => "`:copy`로 선택 영역 또는 마지막 답변을 복사합니다.",
            InlineShellCommand::Doctor => "Enter로 계획 상태를 점검합니다.",
            InlineShellCommand::PlanningInit => "Enter로 계획 제어 센터를 엽니다.",
            InlineShellCommand::Reset => {
                "`:reset <queue|directions|all>`로 초기화할 계획 상태를 지정합니다."
            }
            InlineShellCommand::NewDraft => "Enter로 새 초안을 엽니다.",
            InlineShellCommand::Help => "Enter로 셸 명령 도움말을 엽니다.",
        }
    }

    pub(super) fn inline_shell_command_buffered_hint(
        self,
        command: InlineShellCommand,
        argument: Option<&str>,
        english: &'static str,
    ) -> String {
        if self == Self::English {
            return english.to_string();
        }

        let base_hint = || {
            self.inline_shell_command_base_buffered_hint(command, english)
                .to_string()
        };
        match command {
            InlineShellCommand::Parallel => match parse_parallel_mode_shell_argument(argument) {
                Ok(ParsedParallelModeShellCommand::Enable) => base_hint(),
                Ok(ParsedParallelModeShellCommand::Disable) => {
                    "Enter로 병렬 모드를 끕니다.".to_string()
                }
                Err(error) => format!(
                    "`:parallel {}`은 지원하지 않습니다. 사용 가능: :parallel, :pa, :parallel off, :pa off.",
                    error.argument()
                ),
            },
            InlineShellCommand::Activity => match argument {
                None => base_hint(),
                Some(argument) => {
                    if parse_progressive_activity_detail_kind(argument).is_some()
                        || parse_progressive_activity_card_filter(argument).is_some()
                    {
                        format!(
                            "Enter로 보관된 `{}` activity 카드를 확인합니다.",
                            argument.trim().to_ascii_lowercase()
                        )
                    } else {
                        format!(
                            "`:activity {}`은 지원하지 않습니다. 사용 가능: all, diff, output, command, patch, mcp, plan, reason, agent, terminal, token, guardian, moderation, unknown.",
                            argument.trim()
                        )
                    }
                }
            },
            InlineShellCommand::PlanningInit => match parse_planning_shell_argument(argument) {
                Ok(ParsedPlanningShellCommand::OpenControlCenter) => base_hint(),
                Ok(ParsedPlanningShellCommand::Doctor) => {
                    "Enter로 계획 상태를 점검합니다.".to_string()
                }
                Err(error) => format!(
                    "`:planning {}`은 지원하지 않습니다. 사용 가능한 인자: doctor.",
                    error.argument()
                ),
            },
            InlineShellCommand::Directions => {
                match parse_planning_overlay_shell_argument(argument) {
                    Ok(()) => base_hint(),
                    Err(error) => format!(
                        "`:directions`는 인자를 받지 않습니다 (`{}`). Enter로 계획 지침을 엽니다.",
                        error.argument()
                    ),
                }
            }
            InlineShellCommand::Turns => match argument {
                Some(value) if value.eq_ignore_ascii_case("off") || value == "0" => {
                    "Enter로 자동 후속 실행을 끕니다.".to_string()
                }
                Some(value) => {
                    format!("Enter로 자동 후속 실행을 켜고 턴 한도를 `{value}`로 설정합니다.")
                }
                None => base_hint(),
            },
            InlineShellCommand::Model => match argument {
                None => base_hint(),
                Some(value) if is_turn_option_clear_argument(value) => {
                    "Enter로 app-server 기본 모델을 사용합니다.".to_string()
                }
                Some(_) => {
                    "`:model`은 입력한 모델명을 무시합니다. Enter로 모델 선택을 엽니다.".to_string()
                }
            },
            InlineShellCommand::View => match argument {
                None => base_hint(),
                Some(argument) => match ConversationViewMode::parse(argument) {
                    Some(mode) => {
                        format!(
                            "Enter로 대화 표시 방식을 `{}`(으)로 설정합니다.",
                            mode.label()
                        )
                    }
                    None => format!(
                        "`:view {}`은 지원하지 않습니다. 사용 가능: {}.",
                        argument.trim(),
                        ConversationViewMode::SUPPORTED_LABELS
                    ),
                },
            },
            InlineShellCommand::Language => match argument {
                None => base_hint(),
                Some(argument) => match Self::parse(argument) {
                    Some(selected_language) => format!(
                        "Enter로 TUI 언어를 {}(으)로 설정합니다.",
                        selected_language.label()
                    ),
                    None => format!(
                        "`:language {}`은 지원하지 않습니다. 사용 가능: {}.",
                        argument.trim(),
                        Self::SUPPORTED_LABELS
                    ),
                },
            },
            InlineShellCommand::Think => match argument {
                None => base_hint(),
                Some(argument) if is_turn_option_clear_argument(argument) => {
                    "Enter로 app-server 기본 추론 수준을 사용합니다.".to_string()
                }
                Some(argument) => match ConversationReasoningEffort::parse(argument) {
                    Some(effort) => {
                        format!("Enter로 추론 수준을 `{}`(으)로 설정합니다.", effort.label())
                    }
                    None => format!(
                        "`:think {}`은 지원하지 않습니다. 사용 가능: {}.",
                        argument.trim(),
                        ConversationReasoningEffort::SUPPORTED_LABELS
                    ),
                },
            },
            InlineShellCommand::Queue => match parse_planning_overlay_shell_argument(argument) {
                Ok(()) => base_hint(),
                Err(error) => format!(
                    "`:queue`는 인자를 받지 않습니다 (`{}`). Enter로 계획 큐를 엽니다.",
                    error.argument()
                ),
            },
            InlineShellCommand::Reviews => match parse_planning_overlay_shell_argument(argument) {
                Ok(()) => base_hint(),
                Err(error) => format!(
                    "`:reviews`는 인자를 받지 않습니다 (`{}`). Enter로 리뷰 센터를 엽니다.",
                    error.argument()
                ),
            },
            InlineShellCommand::Reset => match parse_planning_reset_shell_argument(argument).ok() {
                Some(parsed) => match (parsed.target, parsed.confirmed) {
                    (PlanningResetTarget::Queue, _) => {
                        "Enter로 큐 측 계획 상태를 초기화합니다.".to_string()
                    }
                    (PlanningResetTarget::Directions, true) => {
                        "Enter로 계획 지침 초기화를 확정합니다.".to_string()
                    }
                    (PlanningResetTarget::Directions, false) => {
                        "계획 지침 파일을 다시 쓰기 전에 `:reset directions confirm`을 확인하세요."
                            .to_string()
                    }
                    (PlanningResetTarget::All, true) => {
                        "Enter로 전체 계획 초기화를 확정합니다.".to_string()
                    }
                    (PlanningResetTarget::All, false) => {
                        "전체 계획 구조를 바꾸기 전에 `:reset all confirm`을 확인하세요."
                            .to_string()
                    }
                },
                None => match argument.map(str::trim).filter(|value| !value.is_empty()) {
                    None => base_hint(),
                    Some(argument) => format!(
                        "`:reset {argument}`은 지원하지 않습니다. 사용 가능: queue, directions, all."
                    ),
                },
            },
            InlineShellCommand::Diagnostics
            | InlineShellCommand::Work
            | InlineShellCommand::Peek
            | InlineShellCommand::Sessions
            | InlineShellCommand::Stop
            | InlineShellCommand::Copy
            | InlineShellCommand::Doctor
            | InlineShellCommand::NewDraft
            | InlineShellCommand::Help => base_hint(),
        }
    }

    pub(super) fn inline_shell_command_execution_status(
        self,
        command: InlineShellCommand,
        english: &str,
    ) -> String {
        match (self, command) {
            (Self::Korean, InlineShellCommand::Diagnostics) => {
                "진단 화면을 열었습니다.".to_string()
            }
            (Self::Korean, InlineShellCommand::Work) => "통합 작업 센터를 열었습니다.".to_string(),
            (Self::Korean, InlineShellCommand::Sessions) => "최근 세션을 열었습니다.".to_string(),
            (Self::Korean, InlineShellCommand::Reviews) => "리뷰 센터를 열었습니다.".to_string(),
            (Self::Korean, InlineShellCommand::Queue) => "계획 큐를 열었습니다.".to_string(),
            (Self::Korean, InlineShellCommand::Help) => "셸 명령 도움말을 열었습니다.".to_string(),
            _ => english.to_string(),
        }
    }

    pub(super) const fn parallel_control_tower_opened_status(self) -> &'static str {
        match self {
            Self::English => "opened supersession control tower",
            Self::Korean => "병렬 제어 센터를 열었습니다.",
        }
    }

    pub(super) fn inline_command_palette_no_matches(self, prefix: &str) -> String {
        match self {
            Self::English => format!("no shell commands match `{prefix}`"),
            Self::Korean => format!("`{prefix}`와 일치하는 셸 명령이 없습니다"),
        }
    }

    pub(super) const fn shell_command_help_title(self) -> &'static str {
        match self {
            Self::English => "Shell Command Help",
            Self::Korean => "셸 명령 도움말",
        }
    }

    pub(super) const fn shell_command_help_context(self) -> &'static str {
        match self {
            Self::English => " / focused view",
            Self::Korean => " / 집중 보기",
        }
    }

    pub(super) const fn shell_command_help_intro(self) -> &'static str {
        match self {
            Self::English => "Type commands in the prompt; type `:` to open the palette.",
            Self::Korean => "프롬프트에 명령을 입력하세요. `:`를 입력하면 팔레트가 열립니다.",
        }
    }

    pub(super) const fn shell_command_help_keys(self) -> &'static str {
        match self {
            Self::English => {
                "Up/Down or j/k: scroll  |  PgUp/PgDn: page  |  Home/End  |  Esc/Ctrl+C: close"
            }
            Self::Korean => {
                "Up/Down 또는 j/k: 스크롤  |  PgUp/PgDn: 페이지  |  Home/End  |  Esc/Ctrl+C: 닫기"
            }
        }
    }

    pub(super) const fn shell_commands_panel_title(self) -> &'static str {
        match self {
            Self::English => "Shell Commands",
            Self::Korean => "셸 명령",
        }
    }

    pub(super) const fn commands_section_title(self) -> &'static str {
        match self {
            Self::English => "Commands",
            Self::Korean => "명령",
        }
    }

    pub(super) const fn keys_section_title(self) -> &'static str {
        match self {
            Self::English => "Keys",
            Self::Korean => "키",
        }
    }

    #[cfg(test)]
    pub(super) fn startup_axis_row(
        self,
        workflow_status: &str,
        session_status: &str,
        review_status: &str,
    ) -> String {
        match self {
            Self::English => {
                format!(
                    "  |  Workflows: {workflow_status}  |  Sessions: {session_status}  |  Reviews: {review_status}"
                )
            }
            Self::Korean => {
                format!(
                    "  |  워크플로: {workflow_status}  |  세션: {session_status}  |  리뷰: {review_status}"
                )
            }
        }
    }

    pub(super) const fn startup_axis_status(
        self,
        shell_action_availability: ShellActionAvailability,
    ) -> &'static str {
        match (self, shell_action_availability) {
            (Self::English, ShellActionAvailability::Ready) => "ready",
            (Self::English, ShellActionAvailability::Pending) => "pending",
            (Self::English, ShellActionAvailability::Blocked) => "blocked",
            (Self::Korean, ShellActionAvailability::Ready) => "준비됨",
            (Self::Korean, ShellActionAvailability::Pending) => "대기 중",
            (Self::Korean, ShellActionAvailability::Blocked) => "차단됨",
        }
    }

    pub(super) const fn operator_attention_label(
        self,
        shell_action_availability: ShellActionAvailability,
        warning_count: usize,
    ) -> &'static str {
        match (self, shell_action_availability, warning_count) {
            (Self::English, ShellActionAvailability::Pending, _) => "CHECKING",
            (Self::English, ShellActionAvailability::Blocked, _) => "BLOCKED",
            (Self::English, ShellActionAvailability::Ready, 1..) => "DEGRADED",
            (Self::English, ShellActionAvailability::Ready, 0) => "NOTICE",
            (Self::Korean, ShellActionAvailability::Pending, _) => "확인 중",
            (Self::Korean, ShellActionAvailability::Blocked, _) => "차단",
            (Self::Korean, ShellActionAvailability::Ready, 1..) => "주의",
            (Self::Korean, ShellActionAvailability::Ready, 0) => "알림",
        }
    }

    pub(super) const fn operator_attention_impact(
        self,
        shell_action_availability: ShellActionAvailability,
        warning_count: usize,
    ) -> &'static str {
        match (self, shell_action_availability, warning_count) {
            (Self::English, ShellActionAvailability::Pending, _) => "input pending",
            (Self::English, ShellActionAvailability::Blocked, _) => "input locked",
            (Self::English, ShellActionAvailability::Ready, 1..) => "runtime degraded",
            (Self::English, ShellActionAvailability::Ready, 0) => "runtime update",
            (Self::Korean, ShellActionAvailability::Pending, _) => "입력 대기",
            (Self::Korean, ShellActionAvailability::Blocked, _) => "입력 잠김",
            (Self::Korean, ShellActionAvailability::Ready, 1..) => "런타임 저하",
            (Self::Korean, ShellActionAvailability::Ready, 0) => "런타임 갱신",
        }
    }

    pub(super) const fn operator_attention_action(
        self,
        shell_action_availability: ShellActionAvailability,
    ) -> &'static str {
        match (self, shell_action_availability) {
            (Self::English, ShellActionAvailability::Blocked) => "Ctrl+D resolve",
            (Self::English, _) => "Ctrl+D details",
            (Self::Korean, ShellActionAvailability::Blocked) => "Ctrl+D 해결",
            (Self::Korean, _) => "Ctrl+D 상세",
        }
    }

    pub(super) const fn operator_queue_metric_label(self, compact: bool) -> &'static str {
        match (self, compact) {
            (_, true) => "q",
            (Self::English, false) => "queue",
            (Self::Korean, false) => "큐",
        }
    }

    pub(super) const fn operator_agent_metric_label(self, compact: bool) -> &'static str {
        match (self, compact) {
            (_, true) => "a",
            (Self::English, false) => "agents",
            (Self::Korean, false) => "요원",
        }
    }

    pub(super) const fn operator_terminal_metric_label(self, compact: bool) -> &'static str {
        match (self, compact) {
            (_, true) => "t",
            (Self::English, false) => "terminals",
            (Self::Korean, false) => "터미널",
        }
    }

    pub(super) fn operator_state_label(self, state: &str) -> String {
        match (self, state) {
            (Self::Korean, "off") => "꺼짐".to_string(),
            (Self::Korean, "blocked") => "차단".to_string(),
            (Self::Korean, "pending" | "loading") => "대기".to_string(),
            (Self::Korean, "ready") => "준비".to_string(),
            (Self::Korean, "idle") => "유휴".to_string(),
            _ => state.to_string(),
        }
    }

    pub(super) const fn operator_warning_label(self, count: usize) -> &'static str {
        match (self, count) {
            (Self::English, 1) => "warning",
            (Self::English, _) => "warnings",
            (Self::Korean, _) => "경고",
        }
    }

    pub(super) const fn operator_runtime_notice_label(self, count: usize) -> &'static str {
        match (self, count) {
            (Self::English, 1) => "runtime notice",
            (Self::English, _) => "runtime notices",
            (Self::Korean, _) => "런타임 알림",
        }
    }

    #[cfg(test)]
    pub(super) fn github_review_polling_status(self, status: &str) -> String {
        match (self, status) {
            (Self::Korean, "off") => "꺼짐".to_string(),
            _ => status.to_string(),
        }
    }

    #[cfg(test)]
    pub(super) fn startup_workspace_line(self, workspace_path: &str) -> String {
        match self {
            Self::English => format!("workspace: {workspace_path}"),
            Self::Korean => format!("작업공간: {workspace_path}"),
        }
    }

    pub(super) fn startup_status_line(self, status: &str) -> String {
        match self {
            Self::English => format!("status: {status}"),
            Self::Korean => format!("상태: {status}"),
        }
    }

    #[cfg(test)]
    pub(super) const fn startup_ready_action_line(self) -> &'static str {
        match self {
            Self::English => "ready: send a task or reopen a session",
            Self::Korean => "준비됨: 작업을 보내거나 세션을 다시 여세요",
        }
    }

    #[cfg(test)]
    pub(super) const fn startup_examples_line(self) -> &'static str {
        match self {
            Self::English => "examples: fix src/...  |  review PR #123  |  explain tests/...",
            Self::Korean => "예시: src/... 수정  |  PR #123 검토  |  tests/... 설명",
        }
    }

    #[cfg(test)]
    pub(super) const fn startup_shortcuts_line(self) -> &'static str {
        match self {
            Self::English => "shortcuts: Ctrl+o sessions  |  Ctrl+d diagnostics  |  :help",
            Self::Korean => "단축키: Ctrl+o 세션  |  Ctrl+d 진단  |  :help",
        }
    }

    #[cfg(test)]
    pub(super) const fn startup_buffered_prompt_line(self) -> &'static str {
        match self {
            Self::English => "draft: opening prompt buffered below",
            Self::Korean => "초안: 아래 입력 프롬프트가 대기 중",
        }
    }

    #[cfg(test)]
    pub(super) fn startup_diagnostics_summary_line(
        self,
        codex_status: &str,
        app_server_status: &str,
        account_status: &str,
    ) -> String {
        match self {
            Self::English => {
                format!(
                    "diagnostics: codex {codex_status}  |  app-server {app_server_status}  |  account {account_status}"
                )
            }
            Self::Korean => {
                format!(
                    "진단: codex {codex_status}  |  app-server {app_server_status}  |  계정 {account_status}"
                )
            }
        }
    }

    #[cfg(test)]
    pub(super) fn inline_diagnostic_status(
        self,
        ok: bool,
        failed_status: &'static str,
    ) -> &'static str {
        match (self, ok, failed_status) {
            (Self::English, true, _) => "ok",
            (Self::English, false, "attention") => "attention",
            (Self::English, false, _) => "check",
            (Self::Korean, true, _) => "정상",
            (Self::Korean, false, "attention") => "확인 필요",
            (Self::Korean, false, _) => "점검 필요",
        }
    }

    pub(super) fn startup_attachment_summary_line(
        self,
        mode_label: &str,
        recovery_anchor_label: &str,
    ) -> String {
        match self {
            Self::English => {
                format!("attachment: {mode_label}  |  recovery: {recovery_anchor_label}")
            }
            Self::Korean => {
                format!("연결: {mode_label}  |  복구: {recovery_anchor_label}")
            }
        }
    }

    pub(super) const fn recent_session_status_waiting_for_startup(self) -> &'static str {
        match self {
            Self::English => "waiting startup",
            Self::Korean => "startup 대기",
        }
    }

    pub(super) const fn recent_session_status_blocked_by_startup(self) -> &'static str {
        match self {
            Self::English => "blocked",
            Self::Korean => "차단됨",
        }
    }

    pub(super) const fn recent_session_status_not_requested(self) -> &'static str {
        match self {
            Self::English => "idle",
            Self::Korean => "대기",
        }
    }

    pub(super) const fn recent_session_status_ready_to_load(self) -> &'static str {
        match self {
            Self::English => "idle",
            Self::Korean => "대기",
        }
    }

    pub(super) const fn recent_session_status_loading(self) -> &'static str {
        match self {
            Self::English => "loading",
            Self::Korean => "로드 중",
        }
    }

    pub(super) const fn recent_session_status_load_failed(self) -> &'static str {
        match self {
            Self::English => "error",
            Self::Korean => "오류",
        }
    }

    pub(super) fn recent_session_status_unsupported(self, _tier: SessionCatalogTier) -> String {
        match self {
            Self::English => "no catalog".to_string(),
            Self::Korean => "카탈로그 없음".to_string(),
        }
    }

    pub(super) fn recent_session_status_partial(self, _tier: SessionCatalogTier) -> String {
        match self {
            Self::English => "partial".to_string(),
            Self::Korean => "부분".to_string(),
        }
    }

    pub(super) fn recent_session_status_loaded(
        self,
        _tier: SessionCatalogTier,
        count: usize,
    ) -> String {
        match self {
            Self::English => format!("{count} loaded"),
            Self::Korean => format!("{count}개 로드"),
        }
    }

    pub(super) fn parallel_board_refreshed(self, notice: &str) -> String {
        match self {
            Self::English => format!("parallel board refreshed. {notice}"),
            Self::Korean => format!("parallel board 상태를 갱신했습니다. {notice}"),
        }
    }

    pub(super) fn pool_slot_state(
        self,
        slot_id: &str,
        state_label: &str,
        owner_label: &str,
    ) -> String {
        match self {
            Self::English => format!("{slot_id} is {state_label}; owner is {owner_label}."),
            Self::Korean => {
                format!("{slot_id} 상태는 {state_label}이며 owner는 {owner_label}입니다.")
            }
        }
    }

    pub(super) fn agent_roster_state(
        self,
        task_title: &str,
        slot_id: &str,
        state_label: &str,
        summary: &str,
    ) -> String {
        match self {
            Self::English => format!("{task_title} is {state_label} in {slot_id}. {summary}"),
            Self::Korean => {
                format!("{task_title} 작업이 {slot_id}에서 {state_label} 상태입니다. {summary}")
            }
        }
    }

    pub(super) fn distributor_queue_item(
        self,
        task_title: &str,
        queue_state: &str,
        branch_name: &str,
        integration_note: &str,
    ) -> String {
        match self {
            Self::English => {
                format!(
                    "{task_title} result is {queue_state}. branch {branch_name} / {integration_note}"
                )
            }
            Self::Korean => {
                format!(
                    "{task_title} 결과가 {queue_state} 상태로 대기 중입니다. branch {branch_name} / {integration_note}"
                )
            }
        }
    }

    pub(super) fn ledger_stage_record(self, stage_label: &str, summary: &str) -> String {
        match self {
            Self::English => format!("{stage_label} stage record: {summary}"),
            Self::Korean => format!("{stage_label} 단계 기록: {summary}"),
        }
    }

    pub(super) fn integration_blocked(self, reason: &str) -> String {
        match self {
            Self::English => format!("integration is blocked. {reason}"),
            Self::Korean => format!("integration이 차단되었습니다. {reason}"),
        }
    }

    pub(super) fn slot_return_withheld(self, reason: &str) -> String {
        match self {
            Self::English => format!("slot return withheld. {reason}"),
            Self::Korean => format!("slot 반환을 보류했습니다. {reason}"),
        }
    }

    pub(super) const fn no_parallel_events(self) -> &'static str {
        match self {
            Self::English => "[--:--:--] Supervisor: no parallel events yet.",
            Self::Korean => "[--:--:--] Supervisor: 아직 parallel 이벤트가 없습니다.",
        }
    }

    pub(super) fn parallel_history_summary(
        self,
        state_label: &str,
        task_title: &str,
        slot_id: &str,
        agent_id: &str,
        fallback_summary: &str,
    ) -> String {
        match state_label {
            "assigned" | "starting" => self.slot_leased(slot_id, agent_id),
            "running" => self.task_started(task_title),
            "reported_complete" => self.task_reported_complete(task_title),
            "ledger_refreshing" => self.ledger_checking_official_completion(task_title),
            "commit_ready" => self.ledger_accepted_official_completion(task_title),
            "merge_queued" => self.distributor_queue_registered(task_title),
            "pushing" | "pr_pending" | "merge_pending" | "integrating" => {
                self.delivery_stage(task_title, state_label)
            }
            "merged" | "cleanup_pending" | "cleaned" => {
                self.integrated_into_integration_branch(task_title)
            }
            "failed" => self.task_failed(task_title),
            "official_refresh_recovery_needed" => {
                self.official_completion_recovery_needed(task_title)
            }
            _ => fallback_summary.to_string(),
        }
    }

    fn slot_leased(self, slot_id: &str, agent_id: &str) -> String {
        match self {
            Self::English => format!("{slot_id} leased to {agent_id}."),
            Self::Korean => format!("{slot_id}이 {agent_id}에게 대여되었습니다."),
        }
    }

    fn task_started(self, task_title: &str) -> String {
        match self {
            Self::English => format!("started {task_title}."),
            Self::Korean => format!("{task_title} 작업을 시작했습니다."),
        }
    }

    fn task_reported_complete(self, task_title: &str) -> String {
        match self {
            Self::English => format!("{task_title} reported completion."),
            Self::Korean => format!("{task_title} 완료를 보고했습니다."),
        }
    }

    fn ledger_checking_official_completion(self, task_title: &str) -> String {
        match self {
            Self::English => format!("checking official completion for {task_title}."),
            Self::Korean => format!("{task_title} official completion을 확인하고 있습니다."),
        }
    }

    fn ledger_accepted_official_completion(self, task_title: &str) -> String {
        match self {
            Self::English => format!("accepted {task_title} as official completion."),
            Self::Korean => format!("{task_title} 결과를 official completion으로 승인했습니다."),
        }
    }

    fn distributor_queue_registered(self, task_title: &str) -> String {
        match self {
            Self::English => format!("{task_title} result added to distributor queue."),
            Self::Korean => format!("{task_title} 결과가 distributor queue에 등록되었습니다."),
        }
    }

    fn delivery_stage(self, task_title: &str, state_label: &str) -> String {
        let stage_label = state_label.replace('_', " ");
        match self {
            Self::English => format!("{task_title} delivery stage is {stage_label}."),
            Self::Korean => format!("{task_title} delivery 단계가 {stage_label}입니다."),
        }
    }

    fn integrated_into_integration_branch(self, task_title: &str) -> String {
        match self {
            Self::English => format!("{task_title} result integrated into the integration branch."),
            Self::Korean => format!("{task_title} 결과가 통합 브랜치에 반영되었습니다."),
        }
    }

    fn task_failed(self, task_title: &str) -> String {
        match self {
            Self::English => format!("{task_title} failed."),
            Self::Korean => format!("{task_title} 작업이 실패했습니다."),
        }
    }

    fn official_completion_recovery_needed(self, task_title: &str) -> String {
        match self {
            Self::English => format!("{task_title} needs official completion recovery."),
            Self::Korean => format!("{task_title} official completion 복구가 필요합니다."),
        }
    }
}

impl LanguageSelectionOverlayUiState {
    pub(super) fn reset_from_language(&mut self, language: TuiLanguage) {
        self.selected_language_index = language_option_index(language).unwrap_or(0);
    }

    pub(super) fn selected_language_index(&self) -> usize {
        self.selected_language_index
    }

    pub(super) fn selected_language(&self) -> TuiLanguage {
        LANGUAGE_SELECTION_OPTIONS[self.selected_language_index].language
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        let len = LANGUAGE_SELECTION_OPTIONS.len();
        if len == 0 {
            return;
        }
        let next =
            (self.selected_language_index as isize + delta).rem_euclid(len as isize) as usize;
        self.selected_language_index = next;
    }

    pub(super) fn select_index(&mut self, index: usize) -> bool {
        if index >= LANGUAGE_SELECTION_OPTIONS.len() {
            return false;
        }
        self.selected_language_index = index;
        true
    }
}

fn language_option_index(language: TuiLanguage) -> Option<usize> {
    LANGUAGE_SELECTION_OPTIONS
        .iter()
        .position(|option| option.language == language)
}

#[cfg(test)]
mod tests {
    use crate::core::app::QueueAuthorityLoadError;
    use crate::domain::recent_sessions::SessionCatalogTier;

    use super::{
        InlineShellCommand, LanguageSelectionOverlayUiState, QueueMutationKind,
        ShellActionAvailability, TUI_LOCALIZED_IMPORTANT_MARKERS, TuiLanguage,
        language_option_index,
    };

    #[test]
    fn parser_accepts_english_and_korean_aliases() {
        assert_eq!(TuiLanguage::parse("english"), Some(TuiLanguage::English));
        assert_eq!(TuiLanguage::parse("en"), Some(TuiLanguage::English));
        assert_eq!(TuiLanguage::parse("ENG"), Some(TuiLanguage::English));
        assert_eq!(TuiLanguage::parse("korean"), Some(TuiLanguage::Korean));
        assert_eq!(TuiLanguage::parse("ko"), Some(TuiLanguage::Korean));
        assert_eq!(TuiLanguage::parse("kor"), Some(TuiLanguage::Korean));
        assert_eq!(TuiLanguage::parse("kr"), Some(TuiLanguage::Korean));
        assert_eq!(TuiLanguage::parse("한국어"), Some(TuiLanguage::Korean));
        assert_eq!(TuiLanguage::parse("한글"), Some(TuiLanguage::Korean));
        assert_eq!(TuiLanguage::parse("spanish"), None);
    }

    #[test]
    fn default_language_is_english() {
        assert_eq!(TuiLanguage::default(), TuiLanguage::English);
        assert_eq!(
            LanguageSelectionOverlayUiState::default().selected_language(),
            TuiLanguage::English
        );
    }

    #[test]
    fn queue_mutation_copy_is_localized_for_feedback_popup_and_tail() {
        assert_eq!(
            TuiLanguage::English.queue_mutation_pending_feedback(7),
            "Queue change op-7 is waiting for authority acknowledgement."
        );
        assert_eq!(
            TuiLanguage::Korean.queue_mutation_pending_feedback(7),
            "큐 변경 op-7의 권한 확인을 기다리는 중입니다."
        );
        assert!(
            TuiLanguage::Korean
                .queue_mutation_unresolved_refresh_failed(7, "변이 실패", "조회 실패")
                .contains("미확정")
        );
        assert!(
            TuiLanguage::Korean
                .queue_mutation_reconcile_failed(7, "리비전 불일치")
                .contains("반영하지 못했습니다")
        );
        assert_eq!(
            TuiLanguage::Korean.queue_mutation_success_label(QueueMutationKind::RemoveSelected),
            "선택한 큐 항목 제거"
        );
        assert_eq!(
            TuiLanguage::English
                .queue_mutation_success_label(QueueMutationKind::UndoLatestRegistration),
            "Undid latest queue registration"
        );
        assert_eq!(
            TuiLanguage::Korean.queue_mutation_acknowledged(7, "최근 큐 등록 되돌리기", 2, 11,),
            "op-7 확인 완료 / 최근 큐 등록 되돌리기: 작업 2개를 취소로 변경 / 리비전 11"
        );
        assert_eq!(
            TuiLanguage::Korean.queue_mutation_pending_summary(7),
            "op-7 | 권한 확인 대기 중"
        );
        assert_eq!(
            TuiLanguage::Korean.queue_mutation_refresh_required_summary(),
            "큐 권한 새로고침 필요"
        );
        assert_eq!(
            TuiLanguage::Korean.queue_mutation_tail_refresh_detail(),
            "  |  다시 시도하기 전에 :queue 열기"
        );
        assert_eq!(
            TuiLanguage::Korean.queue_mutation_authority_refresh_error(
                &QueueAuthorityLoadError::RevisionsKeptChanging {
                    projection_revision: 8,
                    authority_revision: 9,
                }
            ),
            "새로고침 중 큐 권한이 계속 변경되었습니다 (projection 리비전 8, 권한 리비전 9). 큐를 다시 여세요."
        );
    }

    #[test]
    fn selection_state_resets_to_current_language() {
        let mut state = LanguageSelectionOverlayUiState::default();

        state.reset_from_language(TuiLanguage::Korean);
        assert_eq!(state.selected_language(), TuiLanguage::Korean);
        state.move_selection(1);
        assert_eq!(state.selected_language(), TuiLanguage::English);
        state.reset_from_language(TuiLanguage::English);
        assert_eq!(state.selected_language(), TuiLanguage::English);
    }

    #[test]
    fn command_palette_and_help_copy_are_localized_without_translating_keys() {
        assert_eq!(
            TuiLanguage::English.inline_shell_command_detail(InlineShellCommand::Queue),
            "planning queue"
        );
        assert_eq!(
            TuiLanguage::Korean.inline_shell_command_detail(InlineShellCommand::Queue),
            "계획 큐"
        );
        assert_eq!(
            TuiLanguage::English
                .inline_command_expected_result(InlineShellCommand::Sessions, false),
            "open recent sessions"
        );
        assert_eq!(
            TuiLanguage::English.inline_command_expected_result(InlineShellCommand::Sessions, true),
            "open the operations board"
        );
        let english = TuiLanguage::English.inline_command_palette_header(3, 20);
        let korean = TuiLanguage::Korean.inline_command_palette_header(3, 20);
        assert!(english.contains("palette 3/20"));
        assert!(korean.contains("팔레트 3/20"));
        let english_keys = TuiLanguage::English
            .inline_command_palette_key_lines()
            .join("\n");
        let korean_keys = TuiLanguage::Korean
            .inline_command_palette_key_lines()
            .join("\n");
        for literal_key in ["Up", "Shift+Tab", "Down", "Tab", "Enter", "Esc"] {
            assert!(english_keys.contains(literal_key));
            assert!(korean_keys.contains(literal_key));
        }
        assert_eq!(
            TuiLanguage::Korean.inline_command_palette_empty_key_line(),
            "Esc 닫기"
        );
        assert_eq!(
            TuiLanguage::Korean.inline_command_palette_no_matches(":zzzz"),
            "`:zzzz`와 일치하는 셸 명령이 없습니다"
        );
        assert_eq!(
            TuiLanguage::Korean.inline_shell_command_execution_status(
                InlineShellCommand::Queue,
                "opened planning queue inspection"
            ),
            "계획 큐를 열었습니다."
        );
        assert!(
            TuiLanguage::English
                .shell_command_help_intro()
                .contains(":")
        );
        assert!(TuiLanguage::Korean.shell_command_help_intro().contains(":"));
    }

    #[test]
    fn startup_and_diagnostic_copy_are_localized() {
        assert_eq!(TuiLanguage::English.label(), "English");
        assert_eq!(TuiLanguage::Korean.label(), "한국어");
        assert_eq!(TuiLanguage::English.status_label(), "English");
        assert_eq!(TuiLanguage::Korean.status_label(), "Korean");
        assert_eq!(
            TuiLanguage::English.language_set_status(),
            "language set to English"
        );
        assert_eq!(
            TuiLanguage::Korean.language_set_status(),
            "언어가 한국어로 설정되었습니다."
        );
        assert_eq!(
            TuiLanguage::English.github_review_polling_status("off"),
            "off"
        );
        assert_eq!(
            TuiLanguage::Korean.github_review_polling_status("off"),
            "꺼짐"
        );
        assert_eq!(
            TuiLanguage::Korean.github_review_polling_status("watching acme/repo#1"),
            "watching acme/repo#1"
        );

        for availability in [
            ShellActionAvailability::Ready,
            ShellActionAvailability::Pending,
            ShellActionAvailability::Blocked,
        ] {
            assert!(
                !TuiLanguage::English
                    .startup_axis_status(availability)
                    .is_empty()
            );
            assert!(
                !TuiLanguage::Korean
                    .startup_axis_status(availability)
                    .is_empty()
            );
        }
        assert!(
            TuiLanguage::English
                .startup_axis_row("ready", "idle", "ok")
                .contains("Sessions")
        );
        assert!(
            TuiLanguage::Korean
                .startup_axis_row("준비", "대기", "정상")
                .contains("세션")
        );
        assert_eq!(
            TuiLanguage::English.startup_workspace_line("/repo"),
            "workspace: /repo"
        );
        assert_eq!(
            TuiLanguage::Korean.startup_workspace_line("/repo"),
            "작업공간: /repo"
        );
        assert_eq!(
            TuiLanguage::English.startup_status_line("ready"),
            "status: ready"
        );
        assert_eq!(
            TuiLanguage::Korean.startup_status_line("준비"),
            "상태: 준비"
        );
        assert!(
            TuiLanguage::English
                .startup_ready_action_line()
                .contains("send a task")
        );
        assert!(
            TuiLanguage::Korean
                .startup_ready_action_line()
                .contains("작업을 보내")
        );
        assert!(
            TuiLanguage::English
                .startup_examples_line()
                .contains("review PR")
        );
        assert!(TuiLanguage::Korean.startup_examples_line().contains("검토"));
        assert!(
            TuiLanguage::English
                .startup_shortcuts_line()
                .contains("Ctrl+o sessions")
        );
        assert!(
            TuiLanguage::Korean
                .startup_shortcuts_line()
                .contains("Ctrl+o 세션")
        );
        assert!(
            TuiLanguage::English
                .startup_buffered_prompt_line()
                .contains("buffered")
        );
        assert!(
            TuiLanguage::Korean
                .startup_buffered_prompt_line()
                .contains("대기 중")
        );
        assert!(
            TuiLanguage::English
                .startup_diagnostics_summary_line("ok", "ok", "attention")
                .contains("diagnostics")
        );
        assert!(
            TuiLanguage::Korean
                .startup_diagnostics_summary_line("정상", "정상", "확인")
                .contains("진단")
        );
        assert_eq!(
            TuiLanguage::English.inline_diagnostic_status(true, "check"),
            "ok"
        );
        assert_eq!(
            TuiLanguage::English.inline_diagnostic_status(false, "attention"),
            "attention"
        );
        assert_eq!(
            TuiLanguage::English.inline_diagnostic_status(false, "check"),
            "check"
        );
        assert_eq!(
            TuiLanguage::Korean.inline_diagnostic_status(true, "check"),
            "정상"
        );
        assert_eq!(
            TuiLanguage::Korean.inline_diagnostic_status(false, "attention"),
            "확인 필요"
        );
        assert_eq!(
            TuiLanguage::Korean.inline_diagnostic_status(false, "check"),
            "점검 필요"
        );
        assert!(
            TuiLanguage::English
                .startup_attachment_summary_line("files", "anchor")
                .contains("attachment")
        );
        assert!(
            TuiLanguage::Korean
                .startup_attachment_summary_line("파일", "앵커")
                .contains("연결")
        );
    }

    #[test]
    fn recent_session_copy_covers_states_tiers_and_counts() {
        assert_eq!(
            TuiLanguage::English.recent_session_status_waiting_for_startup(),
            "waiting startup"
        );
        assert_eq!(
            TuiLanguage::Korean.recent_session_status_waiting_for_startup(),
            "startup 대기"
        );
        assert!(
            TuiLanguage::English
                .recent_session_status_blocked_by_startup()
                .contains("blocked")
        );
        assert!(
            TuiLanguage::Korean
                .recent_session_status_blocked_by_startup()
                .contains("차단")
        );
        assert_eq!(
            TuiLanguage::English.recent_session_status_not_requested(),
            "idle"
        );
        assert_eq!(
            TuiLanguage::Korean.recent_session_status_not_requested(),
            "대기"
        );
        assert_eq!(
            TuiLanguage::English.recent_session_status_ready_to_load(),
            "idle"
        );
        assert_eq!(
            TuiLanguage::Korean.recent_session_status_ready_to_load(),
            "대기"
        );
        assert_eq!(
            TuiLanguage::English.recent_session_status_loading(),
            "loading"
        );
        assert_eq!(
            TuiLanguage::Korean.recent_session_status_loading(),
            "로드 중"
        );
        assert_eq!(
            TuiLanguage::English.recent_session_status_load_failed(),
            "error"
        );
        assert_eq!(
            TuiLanguage::Korean.recent_session_status_load_failed(),
            "오류"
        );

        for tier in [
            SessionCatalogTier::AttachOnly,
            SessionCatalogTier::HandleBasedReattach,
            SessionCatalogTier::ProviderBackedCatalog,
        ] {
            assert!(
                TuiLanguage::English
                    .recent_session_status_unsupported(tier)
                    .contains("no catalog")
            );
            assert!(
                TuiLanguage::Korean
                    .recent_session_status_unsupported(tier)
                    .contains("카탈로그 없음")
            );
            assert!(
                TuiLanguage::English
                    .recent_session_status_partial(tier)
                    .contains("partial")
            );
            assert!(
                TuiLanguage::Korean
                    .recent_session_status_partial(tier)
                    .contains("부분")
            );
            assert!(
                TuiLanguage::English
                    .recent_session_status_loaded(tier, 3)
                    .contains("3 loaded")
            );
            assert!(
                TuiLanguage::Korean
                    .recent_session_status_loaded(tier, 3)
                    .contains("3개 로드")
            );
        }
    }

    #[test]
    fn parallel_supervisor_copy_helpers_cover_event_summaries() {
        assert!(TUI_LOCALIZED_IMPORTANT_MARKERS.contains(&"차단"));
        assert_eq!(
            TuiLanguage::English.parallel_board_refreshed("ready"),
            "parallel board refreshed. ready"
        );
        assert_eq!(
            TuiLanguage::Korean.parallel_board_refreshed("준비"),
            "parallel board 상태를 갱신했습니다. 준비"
        );
        assert!(
            TuiLanguage::English
                .pool_slot_state("slot-1", "idle", "none")
                .contains("slot-1 is idle")
        );
        assert!(
            TuiLanguage::Korean
                .pool_slot_state("slot-1", "대기", "없음")
                .contains("slot-1 상태는 대기")
        );
        assert!(
            TuiLanguage::English
                .agent_roster_state("Task", "slot-1", "running", "50%")
                .contains("Task is running")
        );
        assert!(
            TuiLanguage::Korean
                .agent_roster_state("작업", "slot-1", "실행", "50%")
                .contains("작업 작업이 slot-1")
        );
        assert!(
            TuiLanguage::English
                .distributor_queue_item("Task", "queued", "feature/task", "waiting")
                .contains("branch feature/task")
        );
        assert!(
            TuiLanguage::Korean
                .distributor_queue_item("작업", "대기", "feature/task", "대기 중")
                .contains("결과가 대기 상태")
        );
        assert_eq!(
            TuiLanguage::English.ledger_stage_record("refresh", "ok"),
            "refresh stage record: ok"
        );
        assert_eq!(
            TuiLanguage::Korean.ledger_stage_record("refresh", "정상"),
            "refresh 단계 기록: 정상"
        );
        assert_eq!(
            TuiLanguage::English.integration_blocked("conflict"),
            "integration is blocked. conflict"
        );
        assert_eq!(
            TuiLanguage::Korean.integration_blocked("충돌"),
            "integration이 차단되었습니다. 충돌"
        );
        assert_eq!(
            TuiLanguage::English.slot_return_withheld("dirty"),
            "slot return withheld. dirty"
        );
        assert_eq!(
            TuiLanguage::Korean.slot_return_withheld("변경 있음"),
            "slot 반환을 보류했습니다. 변경 있음"
        );
        assert!(
            TuiLanguage::English
                .no_parallel_events()
                .contains("no parallel events")
        );
        assert!(
            TuiLanguage::Korean
                .no_parallel_events()
                .contains("아직 parallel 이벤트")
        );
    }

    #[test]
    fn parallel_history_summary_maps_known_states_and_fallback() {
        let cases = [
            ("assigned", "leased to"),
            ("starting", "leased to"),
            ("running", "started Task"),
            ("reported_complete", "reported completion"),
            ("ledger_refreshing", "checking official completion"),
            ("commit_ready", "accepted Task"),
            ("merge_queued", "distributor queue"),
            ("pushing", "delivery stage is pushing"),
            ("pr_pending", "delivery stage is pr pending"),
            ("merge_pending", "delivery stage is merge pending"),
            ("integrating", "delivery stage is integrating"),
            ("merged", "integrated into the integration branch"),
            ("cleanup_pending", "integrated into the integration branch"),
            ("cleaned", "integrated into the integration branch"),
            ("failed", "failed"),
            (
                "official_refresh_recovery_needed",
                "needs official completion recovery",
            ),
        ];
        for (state, expected) in cases {
            assert!(
                TuiLanguage::English
                    .parallel_history_summary(state, "Task", "slot-1", "agent-a", "fallback")
                    .contains(expected),
                "state {state} should contain {expected}"
            );
        }
        assert_eq!(
            TuiLanguage::English
                .parallel_history_summary("unknown", "Task", "slot-1", "agent-a", "fallback"),
            "fallback"
        );
        assert!(
            TuiLanguage::Korean
                .parallel_history_summary("assigned", "작업", "slot-1", "agent-a", "fallback")
                .contains("대여되었습니다")
        );
        assert!(
            TuiLanguage::Korean
                .parallel_history_summary("running", "작업", "slot-1", "agent-a", "fallback")
                .contains("시작했습니다")
        );
        assert!(
            TuiLanguage::Korean
                .parallel_history_summary(
                    "official_refresh_recovery_needed",
                    "작업",
                    "slot-1",
                    "agent-a",
                    "fallback"
                )
                .contains("복구가 필요합니다")
        );
    }

    #[test]
    fn selection_state_wraps_selects_and_rejects_invalid_indices() {
        let mut state = LanguageSelectionOverlayUiState::default();
        assert_eq!(state.selected_language_index(), 0);
        assert_eq!(language_option_index(TuiLanguage::English), Some(0));
        assert_eq!(language_option_index(TuiLanguage::Korean), Some(1));
        assert!(state.select_index(1));
        assert_eq!(state.selected_language(), TuiLanguage::Korean);
        assert!(!state.select_index(99));
        assert_eq!(state.selected_language(), TuiLanguage::Korean);
        state.move_selection(-1);
        assert_eq!(state.selected_language(), TuiLanguage::English);
        state.move_selection(-1);
        assert_eq!(state.selected_language(), TuiLanguage::Korean);
    }
}
