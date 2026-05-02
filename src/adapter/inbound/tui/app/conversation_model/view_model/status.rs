// 학습 주석: status view model은 domain 값을 직접 화면 문구로 만들지 않고, conversation_text의
// copy helper를 거쳐 approval/control 용어를 TUI 전체와 동일하게 유지합니다.
use crate::adapter::inbound::tui::conversation_text::{
    approval_review_status_text, approval_review_summary_text, control_support_label,
};
// 학습 주석: approval review는 conversation domain의 상태이지만, 여기서는 footer/status copy에
// 반영할 projection 입력으로만 사용합니다.
use crate::domain::conversation::ConversationApprovalReview;

// 학습 주석: 이 impl은 ConversationViewModel 안에 쌓인 warnings, runtime notices, approval state를
// 짧은 UI summary로 접는 상태 표현 전용 확장입니다.
use super::ConversationViewModel;

// 학습 주석: ConversationViewModel의 status 계층은 원본 이벤트 목록을 보존하면서, 좁은 TUI 영역에
// 맞는 한 줄 요약만 계산합니다. 그래서 이 impl은 저장소 갱신보다 문자열 선택과 축약 규칙에 집중합니다.
impl ConversationViewModel {
    fn compact_warning_text(warning: &str) -> String {
        // 학습 주석: 원본 warning은 여러 줄이거나 들여쓰기를 포함할 수 있으므로, footer에 넣기 전에
        // whitespace를 하나의 공백 규칙으로 접습니다. capacity는 원문 길이를 잡아 재할당을 줄입니다.
        let mut compact = String::with_capacity(warning.len());
        // 학습 주석: split_whitespace는 빈 segment를 버리기 때문에 runtime notice의 줄바꿈/탭도
        // 모두 사람이 읽기 쉬운 단일 문장처럼 정리됩니다.
        for segment in warning.split_whitespace() {
            // 학습 주석: 첫 segment 앞에는 공백을 붙이지 않고, 이후 segment 사이에만 공백을 넣어
            // leading/trailing space가 status text에 새어 나오지 않게 합니다.
            if !compact.is_empty() {
                compact.push(' ');
            }
            // 학습 주석: segment 자체는 원문 단어를 그대로 보존해 오류 메시지의 파일명이나 명령어를 바꾸지 않습니다.
            compact.push_str(segment);
        }
        compact
    }

    fn truncate_warning_text(warning: &str, max_detail_len: usize) -> String {
        // 학습 주석: status/footer의 축약 suffix는 ASCII 세 글자로 고정해 terminal 폭 계산과 snapshot
        // 테스트가 locale에 흔들리지 않게 합니다.
        const TRUNCATION_SUFFIX: &str = "...";

        // 학습 주석: 먼저 공백을 접은 뒤 길이를 재야, 여러 줄 warning이 화면 폭을 예상 밖으로 소모하지 않습니다.
        let compact = Self::compact_warning_text(warning);
        // 학습 주석: caller가 suffix보다 작은 제한을 넘겨도 최소 suffix 길이는 확보해 underflow 없이
        // 항상 의미 있는 축약 문자열을 만들 수 있게 합니다.
        let max_detail_len = max_detail_len.max(TRUNCATION_SUFFIX.len());
        // 학습 주석: Rust String의 byte 길이가 아니라 char 개수를 기준으로 판단해 한글 warning을
        // 중간 byte에서 자르는 문제를 피합니다.
        if compact.chars().count() <= max_detail_len {
            // 학습 주석: 이미 제한 안에 들어오면 suffix를 붙이지 않아 실제 상태 문구를 불필요하게
            // "축약된 것처럼" 보이게 하지 않습니다.
            return compact;
        }

        // 학습 주석: suffix가 차지할 폭을 먼저 빼고 본문을 잘라, 최종 문자열이 caller의 detail budget을
        // 넘지 않게 합니다.
        let truncated = compact
            // 학습 주석: char iterator를 사용해 UTF-8 경계가 보장된 prefix만 선택합니다.
            .chars()
            // 학습 주석: 남은 예산은 suffix 길이를 제외한 실제 detail 본문 길이입니다.
            .take(max_detail_len - TRUNCATION_SUFFIX.len())
            // 학습 주석: 선택된 char들을 새 String으로 모아 suffix와 결합할 owned summary를 만듭니다.
            .collect::<String>();
        format!("{truncated}{TRUNCATION_SUFFIX}")
    }

    fn selected_warning_for_summary(&self) -> Option<&str> {
        // 학습 주석: warning summary는 가장 최근 warning을 대표값으로 삼습니다. 전체 목록은
        // count label로 보완하고, footer에는 operator가 지금 확인해야 할 최신 원인을 노출합니다.
        self.base_warnings.last().map(String::as_str)
    }

    fn warning_status_label(&self) -> Option<String> {
        // 학습 주석: status_text에 붙일 warning badge는 원문 warning이 아니라 개수만 표현합니다.
        // 상세 내용은 warning_summary가 별도로 제공하므로 기본 상태 줄이 과하게 길어지지 않습니다.
        let runtime_count = self.base_warnings.len();

        // 학습 주석: warning이 없으면 base status를 그대로 유지하고, 하나면 단수, 여러 개면 개수를
        // 포함해 상태 줄에서 위험 신호의 크기를 빠르게 읽게 합니다.
        match runtime_count {
            0 => None,
            1 => Some("warning".to_string()),
            warning_count => Some(format!("warnings ({warning_count})")),
        }
    }

    pub(crate) fn warning_summary(&self, max_detail_len: usize) -> String {
        // 학습 주석: warning summary consumer는 항상 String을 기대하므로, warning이 없어도 명시적인
        // "none" 문구를 돌려 빈 패널이나 누락처럼 보이지 않게 합니다.
        let Some(selected_warning) = self.selected_warning_for_summary() else {
            return "warning: none".to_string();
        };

        // 학습 주석: 대표 warning은 footer 폭에 맞게 축약하지만, count prefix는 유지해 누적 warning
        // 상황과 최신 warning 내용을 한 줄에 함께 전달합니다.
        let summary = Self::truncate_warning_text(selected_warning, max_detail_len);
        // 학습 주석: selected_warning을 이미 얻은 뒤에도 len을 다시 분기해, 단수/복수 copy가
        // warning_status_label과 같은 문법을 유지하게 합니다.
        match self.base_warnings.len() {
            0 => "warning: none".to_string(),
            1 => format!("warning: {summary}"),
            warning_count => format!("warnings ({warning_count}): {summary}"),
        }
    }

    pub(crate) fn runtime_notice_summary(&self, max_detail_len: usize) -> Option<String> {
        // 학습 주석: runtime notice는 없을 수 있는 부가 상태이므로 Option으로 반환합니다. 호출자는
        // None이면 해당 summary row를 숨기고, Some이면 최신 notice를 보여 줍니다.
        let selected_notice = self.runtime_notices.last()?;
        // 학습 주석: warning과 같은 축약 helper를 공유해 runtime notice도 줄바꿈 제거와 UTF-8-safe
        // truncation 규칙을 동일하게 적용받습니다.
        let summary = Self::truncate_warning_text(selected_notice, max_detail_len);
        Some(if self.runtime_notices.len() == 1 {
            // 학습 주석: notice가 하나면 단순한 runtime prefix만 붙여 화면 잡음을 줄입니다.
            format!("runtime: {summary}")
        } else {
            // 학습 주석: notice가 여러 개면 최신 detail만 표시하되 총 개수를 보여 누적 상태임을 드러냅니다.
            format!(
                "runtime notices ({}): {summary}",
                self.runtime_notices.len()
            )
        })
    }

    pub(crate) fn planning_notice_summary(&self, max_detail_len: usize) -> Option<String> {
        // 학습 주석: planning notice는 runtime_notices 안에 섞여 들어오는 planning 전용 메시지만
        // 별도 summary row로 빼내기 위한 필터 projection입니다.
        let planning_notices = self
            .runtime_notices
            // 학습 주석: 원본 notice 목록은 보존하고, borrowed iterator로 planning prefix만 골라냅니다.
            .iter()
            // 학습 주석: prefix 계약은 notice producer 쪽 copy와 맞물려 있습니다. 여기서 새 enum을
            // 만들지 않고 문자열 prefix로 분리하는 대신, 화면 projection을 가볍게 유지합니다.
            .filter(|notice| notice.starts_with("planning "))
            // 학습 주석: count와 last를 모두 써야 하므로 iterator를 한 번 Vec로 모읍니다. 원소는 참조라
            // notice 문자열을 복제하지 않습니다.
            .collect::<Vec<_>>();
        // 학습 주석: planning notice가 없으면 row 자체를 숨길 수 있도록 None을 반환합니다.
        let selected_notice = planning_notices.last()?;
        // 학습 주석: planning summary도 가장 최근 planning notice를 대표 detail로 삼고 폭 제한을 적용합니다.
        let summary = Self::truncate_warning_text(selected_notice, max_detail_len);

        Some(if planning_notices.len() == 1 {
            // 학습 주석: 단일 planning notice는 간단한 prefix를 사용해 runtime summary와 읽는 패턴을 맞춥니다.
            format!("planning: {summary}")
        } else {
            // 학습 주석: 여러 planning notice는 최신 detail과 전체 개수를 함께 보여 planner 상태 누적을 알립니다.
            format!("planning notices ({}): {summary}", planning_notices.len())
        })
    }

    pub(crate) fn approval_summary(&self) -> Option<String> {
        self.approval_review
            // 학습 주석: approval review가 아직 없으면 summary row도 없어야 하므로 Option을 유지합니다.
            .as_ref()
            // 학습 주석: review 내부의 copy 결정은 conversation_text helper에 맡겨 status line과
            // detail summary가 같은 approval 용어를 공유하게 합니다.
            .map(approval_review_summary_text)
    }

    pub(crate) fn update_approval_review(&mut self, review: ConversationApprovalReview) {
        // 학습 주석: approval review가 들어오면 먼저 현재 turn control truth와 결합해 status_text를
        // 갱신합니다. 그래야 approval 가능 여부가 바뀐 즉시 footer 상태에도 반영됩니다.
        self.set_status_with_warnings(approval_review_status_text(
            &review,
            self.turn_control_truth.approval,
        ));
        // 학습 주석: 원본 review는 summary/detail renderer가 다시 조회할 수 있도록 view model에 보관합니다.
        self.approval_review = Some(review);
    }

    pub(crate) fn interrupt_support_label(&self) -> &'static str {
        // 학습 주석: interrupt 가능 여부도 turn control truth에서 오며, label 문구는 helper를 통해
        // 다른 control support copy와 같은 규칙으로 계산합니다.
        control_support_label(self.turn_control_truth.interrupt)
    }

    pub(crate) fn set_status_with_warnings(&mut self, base_status: String) {
        // 학습 주석: base_status는 conversation state의 주 상태이고 warning label은 보조 badge입니다.
        // 둘을 여기서 합쳐 renderer가 별도 warning count 계산을 반복하지 않게 합니다.
        self.status_text = match self.warning_status_label() {
            // 학습 주석: warning이 있으면 slash separator로 붙여 기존 status copy를 보존하면서 위험 신호를 추가합니다.
            Some(warning_label) => format!("{base_status} / {warning_label}"),
            // 학습 주석: warning이 없으면 base status를 그대로 써서 정상 상태의 status line을 짧게 유지합니다.
            None => base_status,
        };
    }
}
