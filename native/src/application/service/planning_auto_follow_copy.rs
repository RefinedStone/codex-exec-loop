// Centralize operator-facing planning auto-follow copy so future localization
// can swap one seam instead of touching orchestration logic.

pub const BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT: &str =
    "priority queue의 현재 next task 1개를 이어서 진행합니다.";
pub const PLANNING_QUEUE_REFRESH_WITH_PROPOSALS_TRANSCRIPT_TEXT: &str = "previous answer와 existing proposal 작업 목록을 priority queue에 넣고, queue head 1개만 수행한 뒤 남은 queued work와 proposal을 정리합니다.";
pub const PLANNING_QUEUE_REFRESH_WITHOUT_PROPOSALS_TRANSCRIPT_TEXT: &str = "previous answer의 실행 가능한 작업 목록을 priority queue에 넣고, queue head 1개만 수행한 뒤 남은 queued work와 proposal을 정리합니다.";
pub const PLANNING_AUTO_FOLLOW_REFRESH_QUEUE_BODY: &str = r#"대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

직전 답변을 실행 관점에서 정리해 planning priority queue를 갱신하세요.
- 직전 답변에서 실행 가능한 보완, 수정, 후속 제안 사항을 작업 목록으로 정리하고 `task-ledger.json`에 반영하세요.
- 기존 proposal 또는 task와 의미가 겹치면 중복 생성 대신 기존 항목을 갱신하세요.
- 실행 가능한 작업 목록은 priority가 보이도록 normal queue task로 반영하고, `proposed`는 아직 일반 queue에 올리면 안 되는 후보만 남기세요.
- 실행 가능한 queue head가 없다면, 작업 목록 전체를 queue에 반영하되 이번 턴에서는 가장 높은 우선순위의 executable task 1개만 수행하세요.
- 마지막에는 이번 턴에서 실제로 수행한 일과 남은 queued work 및 proposal 목록을 함께 정리하세요.
더 이어갈 작업이 정말 없다면 마지막 줄에 {stop_keyword} 만 출력하세요.

직전 답변:
{last_message}"#;
