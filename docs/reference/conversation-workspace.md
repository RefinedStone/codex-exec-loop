# 새 사용자 대화의 작업 공간 준비

Gateway의 로컬 연결에서 새 대화용 Git worktree를 먼저 준비하고 같은 요청 ID로 다시 찾을 수 있다.
이 API의 소유자는 **사용자의 준비 요청**이다. `Prepared`는 Git 공간의 준비가 확인됐다는 뜻이며,
대화 생성이나 `thread/start` 성공, thread 소유권 확정을 뜻하지 않는다.

## 준비와 조회

`akra/conversationWorkspace/prepare`의 params는 다음 세 값이다.

```json
{
  "cwd": "/absolute/repository",
  "requestId": "new-conversation-2026-11-26-1",
  "baseCommit": "0123456789abcdef0123456789abcdef01234567"
}
```

`baseCommit`은 그 저장소에서 실제로 존재하는 전체 커밋 SHA여야 한다. `requestId`는 1~128자의
ASCII 영문자·숫자·`-`·`_`만 허용한다. 하나의 사용자 시작 시도는 응답 유실, 재연결, 재시작 후에도
같은 ID를 사용해야 한다. 같은 ID에 다른 기준 커밋을 지정하면 거절한다.

응답에는 `requestId`, `repositoryDir`, `workspaceDir`, `branch`, `baseCommit`, `state`, `reason`이
있다. `akra/conversationWorkspace/get`은 `{ "cwd": "/absolute/repository", "requestId": "..." }`를
받아 마지막으로 저장한 기록 또는 `null`을 반환한다. 조회는 없는 runtime, 저장소 marker, DB를 만들지
않는다. 조회 기록은 마지막 확인 상태이며, 이후 사용자가 파일을 지우거나 Git을 바꿨는지 다시 검사한
결과는 아니다. `prepare` 재시도가 실제 공간의 신원을 다시 확인한다.

## 재시도와 보존

| 상태 | 뜻과 재시도 동작 |
| --- | --- |
| `preparing` | Git 생성 전에 소유 요청·경로·브랜치를 durable commit했다. 재시작 시 동일 경로가 같은 저장소·브랜치·기준 커밋으로 준비됐으면 `prepared`로 복구한다. |
| `prepared` | 등록 경로·Git common directory·worktree root·브랜치를 확인했다. 이후 사용자의 새 커밋이나 dirty 파일은 유지한 채 같은 공간을 돌려준다. |
| `preserved` | 생성이 완결되지 않았거나 경로·브랜치의 신원을 확인하지 못했다. 해당 요청과 공간을 보존하고 새 공간으로 교체하지 않는다. 첫 구현에는 자동 정리나 강제 재생성이 없다. |

SQLite 예약 잠금이 같은 저장소의 준비 작업을 프로세스 사이에서 직렬화한다. 준비 기록은 별도 DB에서
Git 부작용보다 먼저 commit한다. 공간은 저장소의 private runtime 아래 `conversation-workspaces/`에
요청 ID의 SHA-256 이름으로, 브랜치는 `akra-conversation/<같은 이름>`으로 생성한다. 기존 소유 기록 없이
동명 브랜치나 목적지가 있으면 채택하거나 삭제하지 않는다.

기존 Git 쓰기 guard를 사용하므로 실행 가능한 저장소 설정, `core.worktree` 우회, 대상 커밋의 LFS 등
content filter가 필요한 체크아웃은 거절한다. hook과 submodule 자동 실행은 기존 guarded subprocess
규칙을 따른다. 작업 공간은 worker pool의 reset/clean 수명 주기에 편입하지 않는다.

## 이번 범위와 후속 연결

이번 API에는 화면, `threadId` 입력, 임의 bind, 작업 공간 삭제가 없다. Desktop에서 새 대화를 시작할 때
실제 `thread/start`에 이 경로를 사용하고, 신뢰할 수 있는 성공 응답의 thread와 해당 요청을 연결하는 작업은
후속 범위다. 연결이 실패하거나 응답을 잃어도 같은 준비 ID로 기존 공간을 재조회·재사용할 수 있다.
활성·dirty·원격 상태 미확인 공간을 자동 삭제하는 정책은 포함하지 않는다.

## 검증

```text
cargo test --locked --all-features --lib conversation_workspace
node scripts/agent-plan.mjs
```

첫 명령은 실제 임시 Git 저장소와 private SQLite를 사용해 재시도, 동시 중복 요청, 준비 기록 재시작 복구,
dirty 파일·새 커밋 보존, 미소유 목적지, 스키마 오류, 기존 쓰기 guard와 로컬 Gateway 경계를 확인한다.
planner가 출력한 플랫폼별 검증도 함께 실행한다. 모델 턴은 호출하지 않는다.
