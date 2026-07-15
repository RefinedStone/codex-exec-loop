use crate::application::service::planning::PlanningResetTarget;
use crate::domain::conversation::ConversationReasoningEffort;
use crate::domain::recent_sessions::SessionCatalogTier;

use super::inline_shell_commands::is_turn_option_clear_argument;
use super::parallel_mode_shell_command::{
    ParsedParallelModeShellCommand, parse_parallel_mode_shell_argument,
};
use super::planning_overlay_shell_command::parse_planning_overlay_shell_argument;
use super::planning_reset_shell_command::parse_planning_reset_shell_argument;
use super::planning_shell_command::{ParsedPlanningShellCommand, parse_planning_shell_argument};
use super::progressive_activity_overlay_ui::parse_progressive_activity_detail_kind;
use super::view_selection_overlay_ui::ConversationViewMode;
use super::{InlineShellCommand, ShellActionAvailability};

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

    pub(super) const fn inline_shell_command_detail(
        self,
        command: InlineShellCommand,
    ) -> &'static str {
        match (self, command) {
            (Self::English, InlineShellCommand::Diagnostics) => "diagnostics",
            (Self::English, InlineShellCommand::Parallel) => "parallel mode",
            (Self::English, InlineShellCommand::Peek) => "parallel agent peek",
            (Self::English, InlineShellCommand::Activity) => "turn activity detail",
            (Self::English, InlineShellCommand::Sessions) => "recent sessions",
            (Self::English, InlineShellCommand::Reviews) => "review center",
            (Self::English, InlineShellCommand::Queue) => "planning queue",
            (Self::English, InlineShellCommand::Directions) => "directions maintenance",
            (Self::English, InlineShellCommand::Turns) => "auto-follow opt-in; off or 0 disables",
            (Self::English, InlineShellCommand::Stop) => "stop active sessions",
            (Self::English, InlineShellCommand::Model) => "model and think",
            (Self::English, InlineShellCommand::View) => "conversation view",
            (Self::English, InlineShellCommand::Language) => "TUI language",
            (Self::English, InlineShellCommand::Think) => "reasoning effort",
            (Self::English, InlineShellCommand::Doctor) => "planning health",
            (Self::English, InlineShellCommand::PlanningInit) => "planning control center",
            (Self::English, InlineShellCommand::Reset) => "planning reset",
            (Self::English, InlineShellCommand::NewDraft) => "new draft",
            (Self::English, InlineShellCommand::Help) => "command help",
            (Self::Korean, InlineShellCommand::Diagnostics) => "진단",
            (Self::Korean, InlineShellCommand::Parallel) => "병렬 모드",
            (Self::Korean, InlineShellCommand::Peek) => "병렬 에이전트 보기",
            (Self::Korean, InlineShellCommand::Activity) => "턴 활동 상세",
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

    pub(super) const fn inline_command_palette_key_lines(self) -> [&'static str; 2] {
        match self {
            Self::English => [
                "Up/Shift+Tab previous | Down/Tab next",
                "Enter choose | Esc close",
            ],
            Self::Korean => ["Up/Shift+Tab 이전 | Down/Tab 다음", "Enter 선택 | Esc 닫기"],
        }
    }

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
                Some(argument) => match parse_progressive_activity_detail_kind(argument) {
                    Some(_) => format!(
                        "Enter로 보관된 `{}` 활동 상세를 확인합니다.",
                        argument.trim().to_ascii_lowercase()
                    ),
                    None => format!(
                        "`:activity {}`은 지원하지 않습니다. 사용 가능: diff, output.",
                        argument.trim()
                    ),
                },
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
            | InlineShellCommand::Peek
            | InlineShellCommand::Sessions
            | InlineShellCommand::Stop
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

    pub(super) const fn inline_command_palette_argument_suffix(self) -> &'static str {
        match self {
            Self::English => " / add value",
            Self::Korean => " / 값 입력",
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
            Self::English => " / inline inspection",
            Self::Korean => " / 인라인 보기",
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

    pub(super) const fn startup_ready_action_line(self) -> &'static str {
        match self {
            Self::English => "ready: send a task or reopen a session",
            Self::Korean => "준비됨: 작업을 보내거나 세션을 다시 여세요",
        }
    }

    pub(super) const fn startup_examples_line(self) -> &'static str {
        match self {
            Self::English => "examples: fix src/...  |  review PR #123  |  explain tests/...",
            Self::Korean => "예시: src/... 수정  |  PR #123 검토  |  tests/... 설명",
        }
    }

    pub(super) const fn startup_shortcuts_line(self) -> &'static str {
        match self {
            Self::English => "shortcuts: Ctrl+o sessions  |  Ctrl+d diagnostics  |  :help",
            Self::Korean => "단축키: Ctrl+o 세션  |  Ctrl+d 진단  |  :help",
        }
    }

    pub(super) const fn startup_buffered_prompt_line(self) -> &'static str {
        match self {
            Self::English => "draft: opening prompt buffered below",
            Self::Korean => "초안: 아래 입력 프롬프트가 대기 중",
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
    use crate::domain::recent_sessions::SessionCatalogTier;

    use super::{
        InlineShellCommand, LanguageSelectionOverlayUiState, ShellActionAvailability,
        TUI_LOCALIZED_IMPORTANT_MARKERS, TuiLanguage, language_option_index,
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
    fn command_palette_and_help_copy_are_localized_without_translating_keys() {
        assert_eq!(
            TuiLanguage::English.inline_shell_command_detail(InlineShellCommand::Queue),
            "planning queue"
        );
        assert_eq!(
            TuiLanguage::Korean.inline_shell_command_detail(InlineShellCommand::Queue),
            "계획 큐"
        );
        let english = TuiLanguage::English.inline_command_palette_header(3, 19);
        let korean = TuiLanguage::Korean.inline_command_palette_header(3, 19);
        assert!(english.contains("palette 3/19"));
        assert!(korean.contains("팔레트 3/19"));
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
        assert_eq!(
            TuiLanguage::English.startup_warning_line("check config"),
            "warning: check config"
        );
        assert_eq!(
            TuiLanguage::Korean.startup_warning_line("설정 확인"),
            "경고: 설정 확인"
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
