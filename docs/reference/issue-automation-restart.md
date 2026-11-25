# Gateway의 이슈 자동화 재시작

관련 이슈: [#2561](https://github.com/RefinedStone/codex-exec-loop/issues/2561).

Gateway가 시작되면 명시적으로 자동화 수준을 선택한 Git 저장소의 복구 후보를 읽고,
현재 조건을 충족하는 저장소의 커널을 client 없이 시작한다. Desktop을 열어야 처음
커널이 생기는 기존 수동 접속 경로와 같은 저장소별 커널·OS 잠금을 사용한다.

## 후보와 실행 권한

후보 등록부는 실행 권한을 저장하지 않는다. 각 후보는 기존 Git incarnation과 대표
checkout을 확인한 뒤 저장소 OS 잠금을 얻고, 미완료 개입·수준 변경을 복구한다.
그다음 현재 정책과 명시적 선택을 다시 읽어 아래 조건을 모두 만족할 때만 runtime을 만든다.

- 수준 변경이 pending 상태가 아니다.
- 명시적으로 요청한 수준이 Gated 이상이다.
- 현재 정책을 적용한 유효 수준도 Gated 이상이다.

파일에만 `gated` 또는 `auto`를 적은 상태는 명시적 선택을 대신하지 않는다.
Native 확인을 거친 Gated와 Auto 선택을 모두 복구하며, 요청 Auto가 현재 정책에 의해
유효 Gated로 제한되면 Gated의 사람 승인·전달 규칙을 적용한다. 유효 수준이 Propose
이하이면 후보를 유지하고 자동 시작을 건너뛴다. Auto 전달의 증거·출처·현재 허가 검사는
[Auto 전달 계약](issue-auto-delivery.md)을 따른다.

복구는 collection Off, 개별 작업 Hold, 사용자·저장소 정책, 사람 필수 관문,
작업·일일 시작·시도 예산을 바꾸지 않는다. 병렬 실행도 별도로 켜야 한다.
병렬 모드만 켜져 있던 저장소를 자동 복구 후보로 추가하지 않는다.

## 저장 위치와 내용

등록부는 planning authority와 **동일한 검증된 사용자 홈** 아래에 있다.

```text
{authority_home}/issue-automation/runtime/restart-registry.db
```

`SqlitePlanningAuthorityAdapter::authority_home_root()`가 기존 authority 경로 계산을
그대로 공유한다. 따라서 `AKRA_HOME`이 다르면 등록부도 분리된다. 환경변수를 무시하는
전역 등록부를 사용하거나, authority와 기본 경로·검증 규칙이 다른
`configuration::akra_home_dir()`로 다시 계산하지 않는다.

| 영속 필드 | 의미 |
| --- | --- |
| `repository_key` | 기존 Git incarnation marker의 저장소 신원. 기본키이며 linked worktree를 같은 저장소로 묶는다. |
| `workspace_root` | 확인된 대표 checkout의 절대 경로. |

자율 수준·정책·실행 허가·자격 증명·이슈 제목이나 본문은 행에 저장하지 않는다.
SQLite의 전용 application ID와 schema version 1로 파일 종류를 확인한다.

기존 private SQLite 경계를 재사용하여 디렉터리·DB·journal의 소유권, 권한, 링크와
identity를 검사한다. 조회나 없는 후보 삭제는 디렉터리·DB·schema를 만들지 않는다.
손상된 DB나 모르는 schema를 빈 목록으로 덮어쓰지 않는다. 후보 수와 필드 길이에
상한이 있으며, 등록·삭제는 짧은 SQLite 트랜잭션으로 직렬화한다.
후보 snapshot을 읽은 연결은 저장소별 잠금을 얻기 전에 해제한다.

## 수준 변경과 중단 복구

등록부 변경은 명시적 수준 변경과 그 복구·조정에서 수행한다.
조회, 확인창 준비, observer tick마다 목록을 쓰지 않는다. 이미 같은 후보의 재등록과
없는 후보의 삭제는 멱등이며, 바뀌지 않은 행을 다시 갱신하지 않는다.

자동 수준으로 올릴 때는 native 확인을 검증한 뒤 후보를 먼저 기록하고,
기존 issue DB의 변경 intent를 시작한다. 기존 작업의 fence와 허가 회수를 마친 뒤
후보가 기록됐는지 다시 확인하고 selection·revision·완료 영수증을 원자적으로 완료한다.
이 순서로 pending intent만 남아 Gateway가 저장소를 발견하지 못하는 창을 막는다.

낮은 수준으로 내릴 때는 기존 fence와 낮은 selection을 먼저 완료하고 후보를 삭제한다.
삭제 실패가 완료된 하향을 되돌리지는 않는다. 명령은 오류를 보고하고,
다음 복구나 재전송에서 현재 selection을 기준으로 삭제를 재시도한다.

| 중단 지점 | 다음 시작의 결과 |
| --- | --- |
| 후보 기록 뒤, intent 기록 전 | 현재 낮은 수준 또는 무선택 상태에는 자동 시작 권한이 없다. 후보를 정리한다. |
| intent 기록 뒤, 완료 전 | 저장소 잠금 아래에서 같은 intent와 owned fence를 복구한 뒤 현재 상태를 판단한다. |
| 상향 완료 뒤, 응답 전 | 후보와 완료 영수증을 재사용한다. 명령 재전송이 별도 선택이나 작업 시작을 만들지 않는다. |
| 하향 완료 뒤, 후보 삭제 전 | 낮은 selection이 우선한다. 오래된 후보로 실행하지 않고 삭제를 재시도한다. |

과거 명령의 목표 수준으로 목록을 되돌리지 않는다. 완료된 명령을 다시 받더라도
최신 durable selection을 읽어 조정한다. 요청 Gated가 유지된 채 정책 상한만
낮아진 경우에는 후보를 유지하고 자동 시작만 건너뛴다.

## 커널 시작과 장애 격리

Gateway는 idle 알림을 연결한 뒤 한 번의 복구 작업을 시작한다.
복구 중에도 접속·종료 루프를 처리하며, 각 후보는 기존 repository entry 잠금을
공유하므로 수동 attach와 별도 커널을 만들지 않는다.

기존 신원 조회에서는 Git marker나 runtime을 만들지 않는다. 확인한 authority
location으로 OS 잠금을 얻은 후 신원과 대표 checkout을 다시 검사한다.
잠금을 잡지 못하면 자동 시작을 우회하지 않는다.
복구 서비스에 들어갈 때도 저장된 신원을 다시 확인하며, 기존 marker가 사라지거나 다른
신원으로 바뀐 경로를 새 namespace로 초기화하지 않는다. 일반 Git 작업 공간 자체를
파일 시스템 snapshot으로 고정하는 기능은 아니므로, 커널 소유 중 checkout의 삭제·교체는
해당 프로세스를 종료한 뒤 수행해야 한다.

- 경로 삭제, 다른 clone으로 교체, 잘못된 marker, 대표 checkout 부재는 해당 후보만 건너뛴다.
- TUI가 저장소 잠금을 소유하면 해당 후보를 건너뛴다.
- linked worktree와 대표 checkout은 같은 커널로 모인다. 병렬 pool 안의 slot은 거절한다.
- 하나의 후보가 실패해도 다른 후보를 계속 처리하고 실패 이유를 로그에 남긴다.
- 등록부 전체를 읽을 수 없으면 그 시작의 자동 복구를 보류한다.
- 수동 접속은 미완료 intent와 fence 복구 실패를 계속 거절한다. 이미 완료된 선택의 후보 조정만 실패한 경우에는 경고를 남기고 접속을 허용하여 설정을 낮추거나 복구할 수 있게 한다. 명시적인 수준 변경·재전송은 등록부 오류를 계속 반환한다.
- 종료가 시작되면 새 복구 커널을 게시하지 않고 기존 host 종료 후 OS 잠금을 해제한다.

수동 접속과 무인 복구는 같은 조립 경로에서 하나의 binding admission을 만든다.
영속 Pause와 미완료 Pause 처리를 runtime 시작 전에 복구하고, collection·coordinator,
실행·전달·Host 검증·Native Pause owner가 같은 admission과 issue store를 공유한다.
따라서 커널이 복구되어도 이미 Pause된 바인딩의 효과는 계속 차단되며, Native 확인을
거친 현재 Resume 완료가 있어야 다시 열린다. Pause 자체는 저장소의 Gated/Auto 선택과
복구 후보를 제거하지 않는다. 자세한 영수증·종료 계약은
[바인딩 전체 Pause](issue-binding-pause.md)를 따른다.

Host 검증의 live owner 초기화는 기존 headless runtime 조립에서 한 번 수행한다.
이후 Desktop이나 다른 client가 접속하면 같은 커널을 재사용하며 초기화를 반복하지 않는다.

존재하는 non-Git 폴더는 incarnation 신원이 없으므로 이번 무인 복구 대상이 아니다.
기존 수동 접속과 수준 선택은 유지한다. 같은 경로가 존재한다는 사실만으로 이전
저장소의 권한을 새 폴더에 적용하지 않는다.

## 기존 설치와 지원 범위

이미 Gated 또는 Auto를 선택한 저장소는 업데이트 뒤 한 번 수동으로 접속하거나 수준 선택을
재시도하면 기존 selection 복구 경로에서 후보가 등록된다. 파일 시스템 전체를
검색하여 이전 저장소를 추측하지 않는다.

이 기능은 **Gateway 프로세스가 실제 시작된 뒤**의 저장소 복구이다.
OS 로그인 자동 실행이나 서비스 설치는 추가하지 않는다. 후보 등록부를 복사해
다른 authority 홈의 선택·작업 장부를 이관하는 기능도 제공하지 않는다.

## 검증 경계

시험은 격리된 Git 저장소·private SQLite·가짜 source, policy, execution 포트를 사용한다.
커널 개수뿐 아니라 실제 수집 catalog와 coordinator의 영속 work 기록을 확인하고,
종료 후 새 Registry에서 같은 작업·시도·예산 결과가 유지되는지 비교한다.
실제 모델 턴이나 공급자 쓰기 요청을 시험의 전제로 삼지 않는다.

추가 회귀는 후보 부재 시 무생성, 서로 다른 profile의 독립성, 동시 갱신,
등록·삭제 실패와 명령 재전송, 경로 교체, TUI 잠금, 현재 정책 상한,
Off/Hold와 미완료 변경 복구, attach·shutdown 경합을 다룬다.
