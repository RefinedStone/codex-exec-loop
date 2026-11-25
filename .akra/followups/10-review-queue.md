대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

직전 결과와 현재 프롬프트의 `[accepted-db-direction-authority]`,
`[accepted-db-task-authority]`, `[db-queue-projection]`을 함께 검토하세요.
별도의 markdown queue 파일을 만들거나 파일을 planning authority로 취급하지 마세요.

실행 가능한 accepted task가 있으면 가장 우선순위가 높은 리뷰 가능 단위 1개만 진행하세요.
새 작업이 꼭 필요하면 현재 세션에 제공된 공식 Akra planning mutation 인터페이스만 사용하고,
ready 작업은 최대 1개로 제한하며 나머지는 proposed 상태로 남기세요.
저장소의 AGENTS.md와 worktree/배포 규칙을 지키고, 검증되지 않은 원격에는 쓰지 마세요.
더 이어갈 작업이 없다면 마지막 줄에 {stop_keyword} 만 출력하세요.

직전 답변:
{last_message}
