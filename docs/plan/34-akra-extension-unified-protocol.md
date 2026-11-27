# 34. Akra 확장 통합 규약과 AI 양방향 조작

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. 현재 동작은 [커널 확장 호스트](../reference/kernel-extensions.md)와
[저장 공간 분석기](../reference/storage-analysis.md)가 설명한다. · 기준일: 2026-11-28 (Asia/Seoul) ·
기준 커밋: `d2e44576` (`origin/prerelease`) ·
관련: [plan 30 커널 확장 구조](30-kernel-extensions-and-storage-analysis.md), [plan 30 기술 블로그 확장](30-tech-blog-extension.md),
[plan 31 자동 블로그](31-akra-auto-blog.md), [plan 18 커널](18-akra-kernel-architecture.md)

## 한 줄 요약

모든 Desktop 확장은 한 가지 규약으로 등록하고 호출한다. 사용자와 AI 모델은 같은 operation, 같은 권한 검사, 같은
이벤트를 쓴다. 확장은 사용자가 확인한 범위만 대화에 넘긴다. 이 통합을 위해 기술 블로그 확장의 명령 계약을 바꾼다.

## 1. 사용자가 원하는 것 (요청 2026-11-28)

- Desktop 확장(플러그인)의 통신 규약과 포맷을 하나로 정리한다.
- 블로그 확장을 바꾸더라도 통합형으로 만든다.
- 사용자와 AI 모델이 양방향으로, 대화하면서 확장을 관리한다.

이 문서는 "양방향"을 다음 두 방향으로 읽었다.

1. **AI → 확장.** 대화 중인 모델이 확장을 찾고, 상태를 읽고, 명령을 실행한다. 그 결과는 Desktop 패널에 바로 보인다.
2. **확장 → AI.** 확장이 사용자가 확인한 데이터와 질문을 대화에 넘긴다. 모델의 답은 다시 확장의 명령으로 돌아온다.

## 2. 현재 상태 (`d2e44576`)

공통 envelope는 정리되어 있다. 확장별 payload와 AI 경로는 정리되어 있지 않다.

| 영역 | 현재 | 문제 |
| --- | --- | --- |
| envelope | `akra/kernel/extensions/{attach,detach,list,call,changed}`. Rust 명세 `schema/akra-kernel-spec.json`에서 TS 타입을 생성한다. | 없음. |
| 등록 | `akra.example`, `akra.storage-analysis`는 공통 호스트(`ExtensionService`)에 등록한다. | 없음. |
| 블로그 | `akra.tech-blog`는 `RepositoryExtensions`라는 별도 경로로 간다. Gateway가 ID로 분기한다(`src/adapter/inbound/gateway/kernel/extensions.rs`). 카탈로그는 `ExtensionCatalogItem::Legacy`다. 작업 수, 바쁨 검사, 이벤트를 자체로 관리한다. | 공통 호스트의 활성 revision, 작업 receipt, 취소, 보존, 재시작 복구를 받지 못한다. 확장마다 규약이 둘이다. |
| 예약 이름 충돌 | 공통 호스트는 `configure`, `jobs`, `job`, `cancel`을 예약한다. 블로그는 `configure {enabled}`, `cancel {id}`를 자기 뜻으로 쓴다. | 블로그를 그대로 등록할 수 없다. |
| Desktop payload 타입 | `desktop/src/renderer/storage/storageTypes.ts`와 블로그 타입을 손으로 쓴다. | Rust 스키마가 바뀌어도 TS가 모른다. |
| Desktop 패널 목록 | `desktop/src/renderer/extensions/panelDefinitions.ts`가 ID, 버전, 범위를 다시 적는다. | Rust manifest와 값이 두 곳에 있다. |
| AI → 확장 | `akra extensions list/call` CLI가 있다. 모델용 `akra-kernel` 스킬은 확장을 설명하지 않는다. | 모델이 확장의 존재와 입력 스키마를 모른다. AI가 호출해도 actor가 기록되지 않는다. |
| 확장 → AI | 저장 공간만 `share_preview` → `share_create` → 입력창 초안 → `shared_*` 조회 경로가 있다. | 저장 공간 전용이다. 블로그는 대화 대신 자체 모델 작업만 쓴다. |
| 용어 | plan 28은 "동적 plugin은 만들지 않는다", plan 30은 "첫 플러그인"이라고 쓴다. | 같은 것을 두 이름으로 부른다. |

## 3. 원칙

- **경로 하나.** 확장 하나는 manifest 하나와 공통 호스트 경로 하나를 갖는다. Gateway, CLI, Desktop에는 확장 ID 분기가 없다.
- **원천 하나.** 기계 계약의 원천은 Rust 등록(manifest와 JSON Schema)이다. Desktop, CLI, AI 스킬은 생성물이나 실행 중
  카탈로그만 읽는다. 손으로 다시 적지 않는다.
- **같은 문, 다른 actor.** 사용자와 AI는 같은 operation을 부른다. 호스트가 같은 스키마, 활성 상태, 권한 검사를 한다.
  다른 점은 actor 기록과 확인 정책뿐이다.
- **권한은 사용자가 준다.** AI가 쓰는 권한은 operation 정책과 사용자 확인에서만 온다. 대화 글, 스킬 문서, 모델 출력은
  권한을 주지 못한다.
- **넘김은 grant로 한다.** 확장이 대화에 넘기는 데이터는 사용자가 미리 본 범위의 grant로 묶는다. 저장 공간 공유를
  일반화한다.
- **용어.** 문서와 화면은 "확장(extension)"을 쓴다. Codex app-server의 `plugin/*`은 "Codex 플러그인"으로 부른다.

## 4. 통합 규약

### 4.1 manifest에 더하는 값

`ExtensionOperation`에 다음 필드를 더한다. API 버전은 2가 된다.

| 필드 | 값 | 뜻 |
| --- | --- | --- |
| `summary` | 한 문장 | 모델과 CLI 도움말에 보이는 설명. |
| `agent` | `allow` \| `confirm` \| `deny` | AI actor의 호출 정책. 기본값은 `deny`다. |

- `allow`: 읽기와, 되돌릴 수 있는 쓰기에 쓴다. 예: 블로그 `list`, `open`, `save`(판이 남는다).
- `confirm`: 비용이 들거나 밖에 남는 동작에 쓴다. 예: 블로그 `generate`, `export`, 저장 공간 분석 시작.
- `deny`: 공유 발급, 삭제, 활성화처럼 사용자만 하는 동작에 쓴다. 예: `share_create`, `delete`, `configure`.

처리기(`ExtensionHandler`)는 `max_active_jobs()`를 선택해서 구현한다. 블로그는 1을 돌려준다. 이것이 지금의
"모델 호출이 끝나기 전에 두 번째 작업을 시작하지 않는다" 검사를 대신한다.

### 4.2 actor

`ExtensionContext`에 `actor: user | agent | cli`를 더한다. 값은 서버가 정한다.

- Desktop 연결의 사용자 동작은 `user`다.
- `--thread-id`로 서버가 확인한 대화에서 온 호출은 `agent`다. 확인 경로는 저장 공간의 대화 검증과 같다.
- 그 밖의 CLI 호출은 `cli`다. `cli`는 `user`와 같은 정책을 쓴다. CLI를 실행한 OS 사용자가 주체이기 때문이다.

작업(`ExtensionJob`)과 이벤트(`extensions/changed`)는 `actor`와 `thread_id?`를 싣는다. Desktop 패널은 AI가 시작한
작업에 "AI" 표시와 대화 링크를 단다.

### 4.3 확인 (`confirm`)

`agent` actor가 `confirm` operation을 부르면 호스트는 바로 실행하지 않는다.

1. 호스트는 `pending_confirmation`을 만들고 `{confirmation_id, extension, action, input, thread_id, expires_at_ms}`를
   돌려준다. 오류 코드는 `confirmation_required`다.
2. `extensions/changed` 이벤트가 Desktop에 알린다. Desktop은 대화 옆에 확인 카드를 보인다. 카드는 실제 입력을 보인다.
3. 사용자가 받아들이면 Desktop이 `confirm {confirmation_id}`를 보낸다. 호스트는 저장한 입력 그대로 실행한다. actor는
   `agent`, 확인자는 `user`로 기록한다.
4. 거절, 만료(10분), 활성 revision 변경, 대화 변경은 확인을 무효로 만든다.

`confirm`과 `decline`은 호스트 예약 동작이 되고 `agent`는 부를 수 없다.

### 4.4 계약 생성물

- `schema/akra-extensions.json`: 빌드에 포함한 모든 등록의 정적 manifest와 operation 스키마. 실행 상태는 넣지 않는다.
  `akra extensions describe --json`과 같은 내용이다.
- `desktop/src/shared/extension-protocol/generated/`: 위 파일에서 생성한 TS 입력·출력 타입과 패널 목록(ID, 버전, 범위).
- `assets/app-server/hub-skills/akra-kernel/SKILL.md`의 "Extensions" 절: 확장 목록, operation 요약, `agent` 정책, 호출
  예시. 지금의 kernel 명령 표와 같은 생성기(`scripts/generate-kernel-skill.mjs`)가 쓴다.
- 생성물이 Rust와 다르면 CI가 실패한다. 지금의 kernel spec 검사와 같은 방식이다.

`panelDefinitions.ts`는 확장 패널의 ID, 버전, 범위를 생성물에서 읽는다. 라벨과 아이콘 같은 화면 값만 Desktop에 남는다.

## 5. 블로그 버전 3

블로그를 공통 호스트에 `ExtensionRegistration`으로 등록한다. Python 스튜디오, 글의 상태 기계, revision 검사, 판(edition)은
그대로 둔다. Rust 처리기가 공통 호스트와 스튜디오를 잇는다.

| 버전 2 | 버전 3 | 바뀌는 점 |
| --- | --- | --- |
| `catalog` | 없음 | 공통 `extensions/list`가 대신한다. |
| `configure {enabled}` | 호스트 `configure {enabled, expected_revision}` | 활성 상태는 호스트가 가진다. 켜고 끌 때 처리기가 스튜디오 설정을 맞춘다. 끌 때는 호스트의 drain과 cleanup 순서를 따른다. |
| `cancel {id}` | `stop {id}` (Command) | 글 단위 멈춤. 호스트 `cancel {job_id}`도 같은 결과가 된다. |
| `generate`, `resume`, `revise`, `review` (Command, 글을 돌려줌) | 같은 이름의 Job | 바로 receipt를 돌려준다. 진행은 `{article_id, stage}` progress로, 결과는 완성된 글로 온다. 단계 반복은 호스트 작업 스레드가 한다. |
| `targets`, `list`, `open`, `seen`, `save`, `restore`, `figure`, `export`, `delete` | 그대로 | 스키마 변화 없음. |

이전과 호환:

- 버전 2 호출은 "Desktop과 Akra를 함께 업데이트하세요" 오류로 거절한다. CLI는 계속 `--version`을 요구한다.
- 이미 켠 저장소는 처리기가 처음 등록될 때 스튜디오 설정을 읽어 호스트 활성 상태를 한 번 채운다. 사용자가 다시 켤
  필요가 없다.
- 저장된 글(`tech-blog.sqlite3`)은 바꾸지 않는다.

이 단계가 끝나면 `RepositoryExtensions`의 블로그 전용 작업 수, 이벤트 채널, Gateway 분기, `ExtensionCatalogItem::Legacy`,
CLI의 블로그 분기를 지운다.

## 6. AI → 확장

- 모델은 `akra-kernel` 스킬의 "Extensions" 절로 확장을 안다. 실행 중인 상태는 `akra extensions list --json`으로 읽는다.
- 모델은 `akra extensions call <id> <action> --version <n> --thread-id <현재 대화> --input <json> --json`으로 호출한다.
  `--thread-id`가 없으면 `cli` actor가 되므로, 스킬은 대화 안에서 항상 넣으라고 안내한다.
- shortcut: 모델이 `--thread-id`를 빼면 `cli` 정책을 받는다. 같은 OS 사용자의 셸은 원래 같은 권한을 갖기 때문이다.
  Codex 세션에 확장 호출 전용 MCP 도구가 생기면 actor를 연결에서 정하도록 바꾼다.
- 작업은 receipt를 돌려준다. 모델은 `job {job_id}`를 다시 읽거나 `--wait`로 끝까지 기다린다.
- 패널은 이벤트로 바로 갱신된다. 사용자는 AI가 바꾼 글과 시작한 분석을 같은 화면에서 본다.

## 7. 확장 → AI

저장 공간의 공유를 호스트 공통 기능으로 옮긴다.

| 호스트 동작 | 입력 | 뜻 |
| --- | --- | --- |
| `handoff_preview` | `{source_action, source_input, thread_id, intent}` | 처리기가 넘길 범위와 필드를 계산해 미리 보여준다. |
| `handoff_create` | `{preview_id, thread_id}` | 사용자 확인 뒤 grant를 발급하고, 입력창에 넣을 초안을 돌려준다. 자동 전송하지 않는다. |
| `handoff_revoke` | `{grant_id}` | grant를 끊는다. 이미 보낸 대화 글은 회수하지 않는다. |
| `resolve` | `{reference, thread_id}` | `akra-ext:<extension>/<kind>/<id>` 참조를 패널 위치로 바꾼다. |

- `intent`는 `ask`(질문), `edit`(모델이 고쳐 주기를 바람) 중 하나다. `edit`이면 초안이 되돌려 쓸 명령(예: 블로그
  `save {id, revision, draft}`)을 같이 적는다.
- 처리기는 `handoff_scope(action, input)`만 구현한다. grant의 만료, 대화 묶음, cursor 서명, 응답 크기 제한은 호스트가
  저장 공간과 같은 규칙으로 한다.
- 저장 공간의 `share_*`, `shared_*`, `resolve_evidence`는 이 동작 위로 옮긴다. 1단계 동안 옛 이름을 별칭으로 둔다.
- 블로그의 첫 사용처는 "AI와 다듬기"다. 사용자가 글에서 누르면 글 초안 grant와 요청이 입력창에 들어간다. 모델은 grant로
  글을 읽고 `save`로 새 판을 쓴다. 판이 남으므로 사용자는 `restore`로 되돌릴 수 있다.

## 8. 조각과 합격 기준

| 조각 | 범위 | 합격 기준 |
| --- | --- | --- |
| X1 | 블로그 버전 3을 공통 호스트에 등록. Gateway·CLI 분기와 `Legacy` 카탈로그 삭제. Desktop 블로그 패널을 receipt와 progress로 바꿈. | 블로그 작업이 비활성화 drain, 재시작 `interrupted`, receipt 재시도를 통과한다. 모델 없는 왕복 테스트와 Desktop 블로그 e2e가 통과한다. `rg 'akra.tech-blog' src/adapter` 결과가 테스트 밖에 없다. |
| X2 | `schema/akra-extensions.json`, TS 생성물, `panelDefinitions.ts` 전환, CI drift 검사. | `storageTypes.ts`와 블로그 타입이 생성 타입을 쓴다. 스키마를 바꾸고 생성을 빼면 CI가 실패한다. |
| X3 | `summary`, `agent` 정책, `actor` 기록, 확인 카드, 스킬 "Extensions" 절. | 모델 없는 fake thread로 `allow`는 바로 실행, `confirm`은 카드 후 실행, `deny`는 거절된다. 패널에 AI 표시가 보인다. |
| X4 | 호스트 공통 handoff. 저장 공간 이전과 별칭. | 저장 공간 공유 회귀 테스트가 새 경로로 통과한다. |
| X5 | 블로그 "AI와 다듬기". | fake thread가 grant로 글을 읽고 `save`로 새 판을 만든다. `restore`로 되돌린다. 실제 모델 턴은 사용자가 허락한 뒤 확인한다. |

X1은 X2보다 먼저 한다. 생성물을 만들 때 블로그가 이미 공통 등록이어야 하기 때문이다. X3은 X2의 생성기를 쓴다.

## 9. 하지 않는 것

- 외부 확장 패키지 설치와 실행 격리. plan 30의 후속 범위로 남긴다.
- 확장 호출 전용 MCP 서버. 지금은 CLI와 스킬이 모델의 입구다(§6 shortcut).
- 모델의 자동 전송. 확장은 입력창 초안만 만든다. 보내기는 사용자가 한다.
- 블로그 글쓰기 규칙과 화면 개편. plan 31이 다룬다.

## 10. 사용자가 정할 것

- Q1. `confirm` 카드를 대화 옆(승인 요청 자리)에 둘지, 확장 패널 안에 둘지. 기본값: 대화 옆.
- Q2. 블로그 `save`를 `allow`로 둘지 `confirm`으로 둘지. 기본값: `allow`(판이 남아 되돌릴 수 있다).
- Q3. 저장 공간 분석 시작을 AI가 `confirm`으로 부를 수 있게 할지. 기본값: `confirm`.
