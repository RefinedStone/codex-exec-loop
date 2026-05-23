use crate::domain::recent_sessions::SessionCatalogTier;

use super::ShellActionAvailability;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum TuiLanguage {
    #[default]
    English,
    Korean,
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

    pub(super) const fn language_set_status(self) -> &'static str {
        match self {
            Self::English => "language set to English",
            Self::Korean => "언어가 한국어로 설정되었습니다.",
        }
    }

    pub(super) fn startup_axis_row(
        self,
        workflow_status: &str,
        queue_status: &str,
        observability_status: &str,
    ) -> String {
        match self {
            Self::English => {
                format!(
                    "  |  Workflows: {workflow_status}  |  Queues: {queue_status}  |  Observability: {observability_status}"
                )
            }
            Self::Korean => {
                format!(
                    "  |  워크플로: {workflow_status}  |  큐: {queue_status}  |  관찰: {observability_status}"
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

    pub(super) fn github_review_polling_status(self, status: &str) -> String {
        match (self, status) {
            (Self::Korean, "off") => "꺼짐".to_string(),
            _ => status.to_string(),
        }
    }

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

    pub(super) fn startup_warning_line(self, warning: &str) -> String {
        match self {
            Self::English => format!("warning: {warning}"),
            Self::Korean => format!("경고: {warning}"),
        }
    }

    pub(super) const fn startup_conversation_label(self) -> &'static str {
        match self {
            Self::English => "conversation",
            Self::Korean => "대화",
        }
    }

    pub(super) const fn startup_first_reply_hint(self) -> &'static str {
        match self {
            Self::English => "first reply appears here after you send the opening prompt",
            Self::Korean => "첫 응답은 프롬프트 전송 후 표시됩니다",
        }
    }

    pub(super) fn startup_starter_line(self, starter_copy: &str) -> String {
        match self {
            Self::English => format!("starter: {starter_copy}"),
            Self::Korean => format!("시작: {starter_copy}"),
        }
    }

    pub(super) const fn startup_empty_starter_copy(self) -> &'static str {
        match self {
            Self::English => "start with a task, file path, or bug summary",
            Self::Korean => "작업, 파일, 버그 요약으로 시작",
        }
    }

    pub(super) const fn startup_buffered_starter_copy(self) -> &'static str {
        match self {
            Self::English => "opening prompt buffered below",
            Self::Korean => "아래에 시작 프롬프트 입력됨",
        }
    }

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
            Self::English => "waiting for startup checks",
            Self::Korean => "startup 검사 대기 중",
        }
    }

    pub(super) const fn recent_session_status_blocked_by_startup(self) -> &'static str {
        match self {
            Self::English => "blocked by startup diagnostics",
            Self::Korean => "startup 진단으로 차단됨",
        }
    }

    pub(super) const fn recent_session_status_not_requested(self) -> &'static str {
        match self {
            Self::English => "not requested yet",
            Self::Korean => "아직 요청 안 함",
        }
    }

    pub(super) const fn recent_session_status_ready_to_load(self) -> &'static str {
        match self {
            Self::English => "ready to load",
            Self::Korean => "로드 준비됨",
        }
    }

    pub(super) const fn recent_session_status_loading(self) -> &'static str {
        match self {
            Self::English => "loading from codex app-server",
            Self::Korean => "codex app-server에서 로드 중",
        }
    }

    pub(super) const fn recent_session_status_load_failed(self) -> &'static str {
        match self {
            Self::English => "load failed",
            Self::Korean => "로드 실패",
        }
    }

    pub(super) fn recent_session_status_unsupported(self, tier: SessionCatalogTier) -> String {
        match self {
            Self::English => format!("{}: catalog unsupported", tier.label()),
            Self::Korean => format!("{}: 카탈로그 미지원", self.session_catalog_tier_label(tier)),
        }
    }

    pub(super) fn recent_session_status_partial(self, tier: SessionCatalogTier) -> String {
        match self {
            Self::English => format!("{}: partial catalog", tier.label()),
            Self::Korean => format!("{}: 부분 카탈로그", self.session_catalog_tier_label(tier)),
        }
    }

    pub(super) fn recent_session_status_loaded(
        self,
        tier: SessionCatalogTier,
        count: usize,
    ) -> String {
        match self {
            Self::English => format!("{}: {count} loaded", tier.label()),
            Self::Korean => format!(
                "{}: {count}개 로드됨",
                self.session_catalog_tier_label(tier)
            ),
        }
    }

    fn session_catalog_tier_label(self, tier: SessionCatalogTier) -> &'static str {
        match (self, tier) {
            (Self::English, _) => tier.label(),
            (Self::Korean, SessionCatalogTier::AttachOnly) => "attach-only 카탈로그",
            (Self::Korean, SessionCatalogTier::HandleBasedReattach) => "handle 기반 reattach",
            (Self::Korean, SessionCatalogTier::ProviderBackedCatalog) => "provider-backed 카탈로그",
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
            "merged" | "cleanup_pending" | "cleaned" => self.integrated_into_prerelease(task_title),
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

    fn integrated_into_prerelease(self, task_title: &str) -> String {
        match self {
            Self::English => format!("{task_title} result integrated into prerelease."),
            Self::Korean => format!("{task_title} 결과가 prerelease에 반영되었습니다."),
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
    use crate::domain::recent_sessions::SessionCatalogTier;

    use super::{
        LanguageSelectionOverlayUiState, ShellActionAvailability, TUI_LOCALIZED_IMPORTANT_MARKERS,
        TuiLanguage, language_option_index,
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
                .contains("Workflows")
        );
        assert!(
            TuiLanguage::Korean
                .startup_axis_row("준비", "대기", "정상")
                .contains("워크플로")
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
        assert_eq!(
            TuiLanguage::English.startup_warning_line("check config"),
            "warning: check config"
        );
        assert_eq!(
            TuiLanguage::Korean.startup_warning_line("설정 확인"),
            "경고: 설정 확인"
        );
        assert_eq!(
            TuiLanguage::English.startup_conversation_label(),
            "conversation"
        );
        assert_eq!(TuiLanguage::Korean.startup_conversation_label(), "대화");
        assert!(
            TuiLanguage::English
                .startup_first_reply_hint()
                .contains("first reply")
        );
        assert!(
            TuiLanguage::Korean
                .startup_first_reply_hint()
                .contains("첫 응답")
        );
        assert_eq!(
            TuiLanguage::English.startup_starter_line("fix bug"),
            "starter: fix bug"
        );
        assert_eq!(
            TuiLanguage::Korean.startup_starter_line("버그 수정"),
            "시작: 버그 수정"
        );
        assert!(
            TuiLanguage::English
                .startup_empty_starter_copy()
                .contains("task")
        );
        assert!(
            TuiLanguage::Korean
                .startup_empty_starter_copy()
                .contains("작업")
        );
        assert!(
            TuiLanguage::English
                .startup_buffered_starter_copy()
                .contains("buffered")
        );
        assert!(
            TuiLanguage::Korean
                .startup_buffered_starter_copy()
                .contains("입력됨")
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
            "waiting for startup checks"
        );
        assert_eq!(
            TuiLanguage::Korean.recent_session_status_waiting_for_startup(),
            "startup 검사 대기 중"
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
            "not requested yet"
        );
        assert_eq!(
            TuiLanguage::Korean.recent_session_status_not_requested(),
            "아직 요청 안 함"
        );
        assert_eq!(
            TuiLanguage::English.recent_session_status_ready_to_load(),
            "ready to load"
        );
        assert_eq!(
            TuiLanguage::Korean.recent_session_status_ready_to_load(),
            "로드 준비됨"
        );
        assert_eq!(
            TuiLanguage::English.recent_session_status_loading(),
            "loading from codex app-server"
        );
        assert_eq!(
            TuiLanguage::Korean.recent_session_status_loading(),
            "codex app-server에서 로드 중"
        );
        assert_eq!(
            TuiLanguage::English.recent_session_status_load_failed(),
            "load failed"
        );
        assert_eq!(
            TuiLanguage::Korean.recent_session_status_load_failed(),
            "로드 실패"
        );

        for tier in [
            SessionCatalogTier::AttachOnly,
            SessionCatalogTier::HandleBasedReattach,
            SessionCatalogTier::ProviderBackedCatalog,
        ] {
            assert!(
                TuiLanguage::English
                    .recent_session_status_unsupported(tier)
                    .contains("catalog unsupported")
            );
            assert!(
                TuiLanguage::Korean
                    .recent_session_status_unsupported(tier)
                    .contains("카탈로그 미지원")
            );
            assert!(
                TuiLanguage::English
                    .recent_session_status_partial(tier)
                    .contains("partial catalog")
            );
            assert!(
                TuiLanguage::Korean
                    .recent_session_status_partial(tier)
                    .contains("부분 카탈로그")
            );
            assert!(
                TuiLanguage::English
                    .recent_session_status_loaded(tier, 3)
                    .contains("3 loaded")
            );
            assert!(
                TuiLanguage::Korean
                    .recent_session_status_loaded(tier, 3)
                    .contains("3개 로드됨")
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
            ("merged", "integrated into prerelease"),
            ("cleanup_pending", "integrated into prerelease"),
            ("cleaned", "integrated into prerelease"),
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
