// Centralize operator-facing planning auto-follow copy so future localization
// can swap one seam instead of touching orchestration logic.

pub const BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT: &str = "다음 queued task 1개를 이어서 진행합니다.";
pub const PLANNING_QUEUE_REFRESH_WITH_PROPOSALS_TRANSCRIPT_TEXT: &str = "previous answer와 existing proposal 작업 목록을 priority queue에 넣고, queue head 1개만 수행한 뒤 남은 queued work와 proposal을 정리합니다.";
pub const PLANNING_QUEUE_REFRESH_WITHOUT_PROPOSALS_TRANSCRIPT_TEXT: &str = "previous answer의 실행 가능한 작업 목록을 priority queue에 넣고, queue head 1개만 수행한 뒤 남은 queued work와 proposal을 정리합니다.";
pub const DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN: &str = r#"# Queue Idle Review Prompt

Queue가 비었을 때만 이 prompt를 사용합니다.

- `directions.toml`의 direction 목표, success criteria, detail doc를 기준으로 현재 DB task authority work list를 다시 점검하세요.
- 최신 사용자 요청과 최신 답변이 다음 작업을 암시하면, 그 내용을 근거로 새 follow-up task를 적극적으로 도출하세요.
- 최신 답변에 다음 순서, 이어서 할 일, 보완 항목, numbered checklist가 보이면 가장 확실한 다음 작업은 `ready` 또는 `in_progress`로 두고 나머지는 `proposed`로 정리하세요.
- 이미 done / in_progress / blocked 로 관리 중인 항목과 의미가 겹치면 새 task를 만들지 말고 기존 task를 갱신하세요.
- direction 기준으로 미달, 보완, 추가 제안이 명확할 때만 새 항목을 추가하세요.
- 지금 바로 이어서 실행해야 할 항목만 `ready` 또는 `in_progress`로 두고, 나머지는 `proposed`로 남기세요.
- 더 이어갈 작업이 정말 없다면 queue를 비운 채 유지하고, 그 판단을 짧게 요약하세요.
"#;
pub const PLANNING_AUTO_FOLLOW_REFRESH_QUEUE_BODY: &str = r#"직전 답변을 실행 관점에서 정리해 planning priority queue를 갱신하세요.
- 직전 답변에서 실행 가능한 보완, 수정, 후속 제안 사항을 작업 목록으로 정리하고 DB task authority에 반영하세요. `task-ledger.json`은 read-only export이므로 직접 수정하지 마세요.
- 기존 proposal 또는 task와 의미가 겹치면 중복 생성 대신 기존 항목을 갱신하세요.
- 실행 가능한 작업 목록은 priority가 보이도록 normal queue task로 반영하고, `proposed`는 아직 일반 queue에 올리면 안 되는 후보만 남기세요.
- 실행 가능한 queue head가 없다면, 작업 목록 전체를 queue에 반영하되 이번 턴에서는 가장 높은 우선순위의 executable task 1개만 수행하세요.
- 마지막에는 이번 턴에서 실제로 수행한 일과 남은 queued work 및 proposal 목록을 함께 정리하세요.
더 이어갈 작업이 정말 없다면 마지막 줄에 {stop_keyword} 만 출력하세요.

직전 답변:
{last_message}"#;
