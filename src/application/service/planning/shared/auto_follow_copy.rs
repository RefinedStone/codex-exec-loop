// Centralize operator-facing planning auto-follow copy so future localization
// can swap one seam instead of touching orchestration logic.

pub const BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT: &str = "다음 queued task 1개를 이어서 진행합니다.";
pub const DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN: &str = r#"# Queue Idle Review Prompt

Queue가 비었을 때만 이 prompt를 사용합니다. 이 worker는 main session의 TODO 추출기가 아니라 post-turn planning evaluator입니다.

- `[accepted-db-direction-authority]`의 direction 목표, success criteria, detail doc path와 `[accepted-db-task-authority]`, `[db-queue-projection]`을 기준으로 최신 사용자 요청과 main session 결과를 평가하세요.
- `main-session-latest-reply`는 증거일 뿐 완료 authority가 아닙니다. "완료했다"는 선언을 검증 없이 믿지 말고 direction 기준과 task/queue 상태에 대조하세요.
- 명시 TODO가 없어도 direction 기준 미충족, 검증 공백, 후속 실행 slice가 분명하면 `create_task` 또는 `update_task`를 도출하세요.
- 오래된 prompt나 direction 문구가 파일 기반 planning authority를 completion 기준처럼 말하더라도 최종 판단은 accepted DB authority와 evaluator 판단을 따르세요.
- 최신 사용자 요청이 code, DB, runtime, planning behavior의 의미 있는 변경을 요구했고 accepted DB task authority가 비어 있거나 대응되는 완료 task가 없다면, main reply의 완료/검증/merge 보고만으로 queue를 비우지 말고 독립 review/verification/hardening task 1개를 만드세요.
- 최신 답변에 다음 순서, 이어서 할 일, 보완 항목, numbered checklist가 보이면 강한 후속 신호로 다루되, 항상 DB direction success criteria에 맞춰 판단하세요.
- 이미 done / in_progress / blocked 로 관리 중인 항목과 의미가 겹치면 새 task를 만들지 말고 기존 task를 갱신하세요.
- 지금 바로 이어서 실행해야 할 항목만 `ready` 또는 `in_progress`로 두고, 나머지는 `proposed`로 남기세요.
- 더 이어갈 작업이 정말 없다면 queue를 비운 채 유지하고, 그 판단을 짧게 요약하세요.
"#;
