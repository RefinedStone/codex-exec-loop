# Agent Canvas v1.2.1 심층 분석

[English](../../../competitive/agent-canvas/analysis.md)

이 분석은 OpenHands Agent Canvas v1.2.1과 prerelease 커밋
`0229ed71a541d91abc04ad13b188d28b9eafcaf8`의 Akra를 비교한다. 재현 명령, 변경 불가능한 소스 링크,
제한 사항은 [evidence.md](evidence.md)에 있다. Akra의 결정 사항과 검토 가능한 구현 단위는
[gap-matrix.md](gap-matrix.md)에 있다.

## 총괄 결론

Agent Canvas는 로컬, 원격, 클라우드 백엔드에 걸쳐 코딩 에이전트 세션과 자동화를 시작하고,
전환하고, 살펴볼 수 있는 폭넓은 브라우저 제어 센터다. 검증된 병렬 전달 오케스트레이터는 아니다.
Akra에 가장 큰 위협이 되는 부분은 선택한 대화 검사기의 응집성이다. 채팅, 파일과 diff, 관찰된
터미널 출력, 브라우저 스크린샷, 태스크, 일부 플래너 상태가 하나의 반응형 작업 공간을 공유한다.
두 번째 위협은 백엔드 전환과 영속적인 Automation 서비스의 결합이다. 이 서비스는 일정, 서명된
이벤트 트리거, 실행 이력, 대화 및 로그로 돌아가는 링크를 지원한다.

구조적 약점은 Canvas가 다른 런타임 위에 놓인 표현 및 라우팅 계층이라는 점이다. 출시된 Codex
경로는 Agent Server와 ACP를 거치면서 중요한 네이티브 의미를 잃거나 변경하며,
커밋에서 검토된 통합까지의 상태를 소유하지 않는다. 다중 세션 탐색은 유용하지만 플릿 토폴로지,
worktree 권한, 충돌 소유권, 정확한 소스 검증, 리뷰 처리 또는 정리는 아니다. 출시된 스택에는
구체적인 안전 및 계약상의 약점도 있다. Codex ACP 권한이 자동 선택되고, 환경 로그인을 사용하는
동일 공급자 세션이 공급자 구성과 잠금 상태를 공유할 수 있으며, 백엔드 키가 브라우저 로컬
스토리지에 저장되고, Automation의 두 종결 상태가 Canvas 타입과 배지 구현에 빠져 있다.

Akra는 현재 활동 가시성, 영속적인 트리거/실행 출처, 그리고 장기적으로 읽기 전용 원격 Akra 노드
상태를 채택해야 한다. 핵심 런타임으로서의 ACP, 다중 공급자 범위, 브라우저 IDE 복제, 프롬프트만을
사용하는 전달 제어, 브라우저에 저장되는 노드 자격 증명, 범용 워크플로 마켓플레이스는 거부해야
한다. 다음과 같이 차별화해야 한다.

> 수락된 의도를 격리되고, 정확한 소스가 검증되고, 리뷰되고, 원격으로 확인된 통합과 정리로
> 전환하는 Codex 우선 운영 및 전달 계층.

## 제품과 사용자층

Agent Canvas는 자신을 코딩 에이전트와 자동화를 위한 자체 호스팅 개발자 제어 센터라고 설명한다.
v1.2.1 README는 OpenHands, Claude Code, Codex, Gemini와 기타 ACP 에이전트를 명시하며 로컬,
원격, 클라우드 백엔드를 지원한다. 저장소는 MIT 라이선스이고 제품에 Beta 표시가 있다. 이는
`documented` 제품 주장이고, 조사한 소스는 백엔드 레지스트리, 에이전트 프리셋, Automation UI를
검증한다.

주요 운영자는 한 공급자의 네이티브 프로토콜이나 규범적인 전달 수명주기보다 공급자 및 실행
백엔드 선택권, 브라우저 접근, 풍부한 검사기를 중시하는 개발자 또는 자체 호스팅 팀일 가능성이
크다. 이 사용자층 추론은 출시된 레지스트리, 배포 모드, UI에서 나온 것이며 재현된 사용자 연구에
근거하지 않는다.

애플리케이션은 npm CLI와 Docker 이미지로 배포된다. 풀스택 모드는 정적 프런트엔드/프록시,
Agent Server, Automation 서비스를 시작한다. Docker를 쓰지 않는 기본 로컬 모드는 명시적으로
샌드박스가 없으며 에이전트에 호스트 파일 시스템 접근 권한을 준다. Docker 샌드박싱은 선택 가능한
더 안전한 모드다. 공개 자체 호스팅에는 공유 API 키와 함께 Canvas 외부의 인그레스, TLS, 네트워크
강화가 필요하다.

조사한 릴리스에는 네이티브 TUI도, 별도로 패키징된 데스크톱/모바일 클라이언트도 없다. 반응형
브라우저 애플리케이션이 운영자 화면이다.

## 아키텍처

### Canvas 경계

Canvas는 Vite로 빌드된 React 19 및 React Router 7 프런트엔드다. REST와 WebSocket으로 Agent
Server와 통신하고 HTTP로 별도의 Automation 서비스와 통신한다. 브라우저는 여러 백엔드를
기억하고 상태를 확인하며 대화를 그 대화의 소유 백엔드와 연결할 수 있다. 대화 런타임, 샌드박스,
작업 공간, 이벤트 이력, ACP 하위 프로세스는 Canvas가 아니라 Agent Server가 소유한다.
Automation은 트리거 정의, 스케줄링, 실행 디스패치, 복구, 자체 데이터베이스를 소유한다.

이 분리는 프런트엔드의 적응성을 유지하지만 종단 간 보장이 여러 계약과 프로세스에 의존하게
만든다. Canvas는 REST를 통해 이력을 미리 불러오고 대화 WebSocket을 다시 연결한 뒤 마지막으로
관찰한 시퀀스 이후의 이벤트를 요청한다. 메시지 제출에는 HTTP 큐 폴백이 있다. 대화 목록은 10초마다
폴링한다. 선택된 대화 상태는 전환 중에는 더 자주, 안정된 상태에서는 더 느리게 폴링한다. WebSocket
재연결은 기본적으로 지수 백오프나 지터 없이 고정된 3초 지연과 무제한 재시도를 사용한다.

Canvas v1.2.1은 다음과 같은 출시 구성 요소를 고정한다.

```text
Agent Canvas 1.2.1
  -> OpenHands Agent Server / software-agent-sdk 1.33.0
    -> @zed-industries/codex-acp 0.16.0
      -> embedded Codex Rust crates from rust-v0.137.0
```

출시된 Codex ACP 프로세스는 Codex 코어, 인증, 스레드, 스토리지 crate를 내장한다.
`codex app-server`를 실행하지 않는다. 이후 소스 스냅샷의 토폴로지는 다르므로 이 점이 중요하다.

```text
software-agent-sdk 1.35.0 (not shipped by Canvas 1.2.1)
  -> @agentclientprotocol/codex-acp 1.1.2
    -> official codex app-server child from @openai/codex ^0.144.0
```

두 번째 경로는 유용한 미래 증거이지 v1.2.1 기능은 아니다. 프로세스와 JSON-RPC 경계가 하나 더
생기지만, 이 조사에서는 그로 인해 사용자가 체감하는 지연 시간이나 메모리 불이익이 생기는지
측정하지 않았다.

### 작업 공간과 연속성

연결된 로컬 작업 공간 흐름은 선택기를 `local_repo`로 초기화하고 사용자가 `new_worktree`로 바꿀
수 있다. 별도의 처음부터 시작하기 흐름은 작업 공간과 모드를 모두 생략하며, 서비스는
`worktree: true`인 생성된 작업 디렉터리로 폴백한다. 요청이 있으면 Agent Server는
`/tmp/conversation-worktrees/<conversation-id>/` 아래에 브랜치와 worktree를 만든다. 따라서 격리는
하나의 전역 기본값이 아니라 경로에 따라 달라진다. 대화를 삭제해도 작업 공간은 의도적으로
보존되며, 이 조사에서는 그에 대응하는 백엔드 worktree/브랜치 정리 경로를 찾지 못했다.

선택된 저장소, 브랜치, 작업 공간, 작업 공간 모드는 Agent Server의 소스 출처 개념이 아니다.
Canvas는 이를 브라우저 로컬 대화 메타데이터로 저장하고 해당 클라이언트에서 뷰를 복원한다. 따라서
다른 브라우저를 사용하거나 스토리지를 지우면 대화가 남아 있어도 해당 레이블을 잃을 수 있다. 이는
Akra의 영속적인 수락 태스크, lease, 브랜치, 고정 범위 출처보다 실질적으로 약하다.

ACP 세션 ID와 작업 디렉터리는 불투명한 Agent Server 상태에 저장된다. 재개 시 SDK는
`load_session`을 시도하고, 프로토콜 오류가 발생하면 새 세션으로 폴백할 수 있다. 이는 복구이지만
연속성 의미를 조용히 바꿀 수도 있다. 출시된 클라이언트는 대화별 범용 ACP 데이터 디렉터리 격리를
요청할 수 없으므로 환경 로그인을 사용하는 대화가 공급자의 HOME/구성 상태를 재사용할 수 있다.
`CODEX_AUTH_JSON` 같은 파일 기반 자격 증명은 별도로 대화별 디렉터리에 구체화된다. 비교용 충돌 및
재개 실험은 수행하지 않았다.

## 하네스와 컨텍스트

Canvas는 에이전트 프리셋, 모델 선택, MCP 구성, 시크릿, 스킬, 런타임 서비스 URL, 확인 모드,
공급자별 설정을 노출한다. 내장 ACP 공급자에서는 빈 명령을 임의 실행 파일로 흘려보내지 않고
고정된 레지스트리를 통해 해석한다. 이는 건전한 패키징 패턴이다.

컨텍스트 조립, 압축, 모델 메모리, 도구 실행은 여전히 Agent Server 또는 공급자 에이전트의
책임이다. Canvas는 이러한 시스템을 구성하고 렌더링하지만 별도의 권위 있는 메모리 또는 출처
계층을 제공하지 않는다.

출시된 Codex 경로는 종단 간으로 Codex 코어나 ACP 래퍼보다 실질적으로 좁다.

- 조사한 두 ACP 래퍼 모두에 계획 업데이트가 있지만 Python `ACPAgent` 이벤트 브리지는 이를
  소비하지 않는다.
- 이와 별개로 Canvas는 ACP 세션에서 Code/Plan 모드 전환 컨트롤을 숨긴다.
- reasoning delta는 스트리밍 시점의 교차 순서를 유지하지 않고 최종 reasoning 내용으로 버퍼링된다.
- 네이티브 thread 및 turn ID는 Agent Server 이벤트로 변환되며 기본 Canvas 상관관계 키로
  보존되지 않는다.
- 도구 진행 상황은 축약되고, 네이티브 diff, patch-delta, 일부 오류 구분은 사라지거나 일반 도구
  데이터로만 렌더링된다.
- `request_permission`은 운영자에게 묻지 않고 항상 첫 번째 ACP 옵션을 선택한다.
- SDK의 `ask_agent()` 경로는 ACP `fork_session`을 호출하지만 조사한 두 Codex ACP 래퍼 어느 쪽도
  이를 광고하거나 구현하지 않는다.

Canvas 바깥쪽 OpenHands 확인 정책, ACP 세션 모드, ACP 권한 콜백은 별도의 컨트롤이다. Canvas는
바깥쪽 정책의 기본값을 확인 안 함으로 두고, 출시된 Codex 프리셋은 full-access 모드를 사용하며,
Python ACP 콜백은 독립적으로 운영자 왕복 없이 옵션 인덱스 0을 선택한다. 일반 파일 시스템 또는
네트워크 권한 요청에서 조사한 두 Codex 래퍼 모두 세션 범위 승인을 첫 번째에 둔다. 반면 Akra는
공식 app-server 알림 스키마를 직접 분류하며 메서드에 handled, deferred, diagnostic, ignored 중 어느
처분도 없으면 계약 테스트를 실패시킨다. Akra 역시 일부 네이티브 이벤트를 지나치게 거칠게
축약하므로 직접 토폴로지는 충실도를 높일 기회이지 모든 필드가 이미 잘 투영된다는 증거는 아니다.

## 브라우저 UX와 세션 감독

Agent Canvas에는 네이티브 터미널 UI가 없다. 이 절은 브라우저 상호작용 모델을 평가한다.

### 강점

Canvas의 선택된 대화 경험은 성숙하다. 크기 조절 가능한 채팅과 검사기에서 다음을 볼 수 있다.

- 작업 공간 파일 트리, 읽기 전용 파일 내용, diff
- xterm을 통해 관찰된 셸 명령과 출력
- 최신 브라우저 스크린샷과 URL
- 태스크 목록과 지원되는 클라우드 컨텍스트의 플래너 상태
- OpenHands 및 ACP 도구, reasoning, 스킬, MCP, 확인을 위한 구조화된 카드
- 페이지 단위 이력, 스트리밍 축약, 대기 메시지 피드백, 첨부 파일, 재연결 상태

대화 레일은 고정, 그룹화, 필터링, 정렬, 활성 항목 전용 보기를 지원한다. 이는 효과적인 다중 세션
탐색이다. 백엔드 상태 저하와 세션-백엔드 기억도 실행 환경 사이를 오가는 운영자의 마찰을 줄인다.

`/goal` 상호작용은 별도의 장점으로 인정할 만하다. Canvas는 명령을 가로채 Agent Server 판정
루프의 목표, 라운드, 점수, 누락 증거, 중지/재개 상태를 렌더링한다. 목표 런타임은 Canvas 소유가
아니지만 UI는 장시간 실행되는 제어 루프를 이해하기 쉽게 만든다.

### 한계

브라우저 작업 공간은 검사기이지 완전한 의미의 브라우저 IDE가 아니다. xterm 인스턴스는 입력이
비활성화되어 있고 기록된 명령 로그만 렌더링한다. 브라우저 상태는 스크린샷, URL, 외부에서 열기
액션뿐이다. 파일과 diff는 읽기 전용이고, 파일 목록은 2,000개로 제한되며, 변경 사항 뷰는 처음
100개 파일까지만 요청한다.

Akra의 제품 방향에서 더 중요한 점은 Canvas가 그래프 캔버스나 병렬 감독자가 아니라는 것이다.
독립된 대화 목록과 선택한 하나에 대한 깊은 뷰를 제공한다. 출시된 하위 대화 투영은 하나의 계획
에이전트를 전제로 한다. 태스크 의존성, worktree lease, 파일/핫스폿 소유권, 충돌 위험, 검증 소스
SHA, PR 리뷰 상태, 통합 순서 또는 정리를 보여 주지 않는다.

목록 상태 점은 `IDLE`과 `WAITING_FOR_CONFIRMATION`도 같은 녹색 작업 상태로 매핑한다. 선택한
세션에서는 확인 세부 정보를 노출할 수 있지만 플릿 전체에서 승인 병목을 식별하기 어렵다.

## Admin, 원격, 자동화

### 원격 백엔드

Canvas는 출시된 원격 라우팅에서 Akra보다 앞선다. 브라우저는 로컬, 원격, 클라우드 Agent Server
엔드포인트를 등록하고 상태를 관찰하며 그 사이를 전환할 수 있다. 그 대가로 광범위한 신뢰 모델을
갖는다. 각 백엔드 레코드는 호스트와 평문 API 키를 브라우저 `localStorage`에 저장한다. 로컬 정적
서버도 키를 HTML/로컬 스토리지에 주입할 수 있다. 공개 모드는 이 주입을 피하지만 Canvas 자체에는
다중 사용자 RBAC나 영속적인 운영자 감사 계층이 없다.

현재 Akra Admin 서버는 loopback 전용이다. 향후 해답은 서버 측 또는 OS 비공개 자격 증명,
상호 TLS ID, 기능, 버전, 작업 공간 범위, 오래된 상태 처리를 갖춘 읽기 전용 Akra 노드 레지스트리여야
한다. 범용 공급자 레지스트리가 되어서는 안 된다.

### 자동화

OpenHands Automation 1.1.4는 선별적으로 차용할 가장 명확한 기능이다. 다음을 갖는다.

- IANA 시간대를 사용하는 cron 일정
- 소스/이벤트 매칭과 JMESPath 필터를 갖춘 이벤트 트리거
- 내장 및 사용자 정의 webhook 소스용 HMAC 검증
- 활성화/비활성화 정의, 소프트 삭제, 즉시 실행
- 타임스탬프, 오류, 이벤트 페이로드, 대화, 샌드박스, bash 명령을 연결하는 영속 실행 행
- 데이터베이스 잠금이 있는 스케줄러 폴링, 디스패처 시간 제한, 취소, watchdog 복구
- 자동화별 암호화된 런타임 키/값 상태

Canvas는 정의 목록/상세, 활성화/비활성화, Run Now, 부분 편집, 활동 이력, 대화 링크,
stdout/stderr 로그를 노출한다. 이는 유용한 운영 추적이지만 전체 백엔드 계약을 노출하지는 않는다.
자동화 생성은 결정론적 양식을 제출하는 대신 새 에이전트 대화에서 번역된 프롬프트를 실행한다.
편집 모달은 일정, 모델, 프롬프트 필드를 바꿀 수 있지만 이벤트 트리거의 소스/이벤트/필터 필드는
읽기 전용이다. 백엔드에는 실행 취소 기능이 있지만 Canvas 서비스와 UI는 이를 호출하지 않는다.

구현은 다음과 같은 설계 경고도 제공한다.

- webhook 재전송 또는 delivery-ID 멱등성이 구현되지 않았다.
- 속도 제한과 요청 본문 제한은 인프라에 위임한다.
- 일반 실패 실행에는 범용 재시도/시도 모델이 없다.
- 디스패처 배치 크기는 가져오기 한도이지 전역 실행 세마포어가 아니다.
- 동시성 고갈은 `SKIPPED` 종결 상태를 만들 수 있다.
- 백엔드에는 6개 실행 상태가 있지만 Canvas의 TypeScript enum과 배지는 4개만 다룬다.
- Canvas는 백엔드 `timeout_at`, `sandbox_id`, `created_at` 필드를 생략하고 재시도 횟수, 큐 대기,
  watchdog 복구 이유 또는 감사 행위자를 투영하지 않는다.

API가 백엔드 상태를 변경 없이 반환하고 배지가 없는 구성을 역참조하므로 `CANCELLED` 또는
`SKIPPED` 실행을 렌더링하는 소스상 확실한 `TypeError` 경로가 있다. 백엔드는 두 상태를 모두 만들
수 있으므로 이는 검증된 계약 불일치이자 추론된 런타임 실패다. 전체 실패를 실제 배포에서 재현하지는
않았다.

Akra는 범용 자동화 플랫폼을 만들면 안 된다. 타입이 있고 인증되며 멱등적인 코드 변경 이벤트를
운영자 소유 태스크 템플릿으로 받아들이고, 트리거에서 수락 태스크, lease, 세션, 검증, PR, 전달
결과, 정리까지 하나의 추적을 보존해야 한다.

## 병렬 작업과 전달

Canvas는 여러 대화를 만들고 대화별 Agent Server worktree를 요청할 수 있다. Git pull, push,
create-PR 버튼은 자연어 지시를 에이전트 대화에 삽입한다. 편리하지만 애플리케이션은 전달 상태
머신을 소유하지 않으며 요청한 작업이 의도한 소스를 대상으로 수행되었음을 증명하지 않는다.

이 차원에서 Akra의 출시된 병렬 모드 계약이 더 강하다. 수락된 태스크 의도, lease, worktree,
세션 상세, 고정 소스 범위, GitHub 리뷰/검사 게이트, 직렬화된 통합, 원격 검증, 정리를 소유한다.
명시적인 상위 정책은 고위험 리뷰 생략 경로를 선택할 수 있지만, 해당 예외에는 검토된 전달을
조용히 흉내 내는 대신 출처가 있다. 정확히 고정된 SHA에 묶인 호스트 실행 또는 신뢰할 수 있는 CI
검증 증거는 계획된 P0 계약으로 남아 있다. 현재의 자유 형식 검증 요약은 그 증명이 아니다.

Akra의 즉각적인 약점은 활동 가시성이다. 병렬 reducer는 도구 활동 및 delta/completion 알림에서
감독자 상태를 의도적으로 무효화하지 않는다. 운영자는 수명주기와 선택된 세부 정보는 볼 수 있지만
각 에이전트가 지금 무엇을 하는지, 얼마나 오래 조용했는지를 안정적으로 훑어볼 수 없다. Canvas는
풍부한 선택 세션 이벤트의 가치를 보여 주고, 이전 jcode 조사도 동일한 플릿 수준 격차를 이미
확인했다. 따라서 이 비교는 경쟁 하위 시스템을 만들지 말고 기존 병렬 활동 구현 단위를 수정하고
우선순위를 높여야 한다.

## 성능

공정한 제품 성능 비교는 완료하지 못했다. Canvas 저장소는 Akra와 비교할 수 있는 최신의 재현
가능한 시작, 상호작용, 안정 상태 메모리 또는 동시 세션 벤치마크를 공개하지 않는다. 이 조사에서는
인증된 Codex turn이나 세 세션 Canvas 배포를 실행하지 않았다.

내보낸 소스 트리에서 로컬 릴리스 게이트를 실행하여 다음과 같은 개발 프로세스 결과를 얻었다.

| 명령 | 결과 | 실행 시간 | 최대 RSS |
| --- | --- | ---: | ---: |
| `npm run lint` | 통과 | 46.62 s | 3,362,648 KiB |
| `npm test` | 통과: 480개 파일, 1개 건너뜀; 3,666개 테스트 통과 | 52.82 s | 763,632 KiB |
| `npm run build` | 통과 | 8.76 s | 1,759,796 KiB |
| `npm run build:lib` | 통과 | 12.83 s | 1,581,092 KiB |

이 값들은 한 머신에서의 빌드 및 테스트 프로세스를 설명한다. Canvas 런타임 지연 시간이나 리소스
측정값이 아니며 Akra의 우위를 입증하지 않는다. 빌드는 청크 크기 경고를 냈고 압축 전 가장 큰
클라이언트 청크는 526.28 kB였다. `npm pack --dry-run`은 14,215,651바이트 tarball,
57,069,667바이트 압축 해제 크기, 9,439개 항목을 명시했다. 이는 패키징 사실이지 사용자 경험
점수가 아니다.

기존 jcode P0 성능 증거 계약이 여전히 올바른 Akra 작업 항목이다. 그 스키마를 사용할 수 있게 되면
Canvas 토폴로지 프로필에 cold usable UI, 재개, submit-to-first-event, 첫 assistant delta, 세 세션
전파, 전체 프로세스 트리 메모리를 추가할 수 있다. 그때까지 성능 승자는 `unverified`다.

## 품질과 위험

Agent Canvas에는 상당한 자동화 커버리지가 있다. 릴리스 checkout에는 경로명 기준 TypeScript
test/spec 파일이 500개, E2E spec 파일이 19개 있다. Vitest는 이 환경에서 파일 481개와 테스트 사례
3,680개를 실행했다. 3,666개가 통과했고 5개는 건너뛰었으며 9개는 todo였다. 메인 CI는 Ubuntu와
Windows에서 설치 및 애플리케이션 빌드를 실행한다. lint, unit test, library build, package
verification은 Ubuntu full-check 작업에서 실행된다. 실시간 E2E는 수동 트리거나 적격 레이블이
붙은 pull request에서 조건부로 실행되며, 강제되는 커버리지 임계값은 찾지 못했다.

아키텍처의 폭은 저장소 간 계약 위험을 만든다. Automation 상태 불일치가 한 가지 구체적인
예다. ACP 체인도 마찬가지다. Canvas는 알려진 인자 순서 깨짐 때문에 client/protocol 범위를
고정하지만 Agent Server, Python ACP, 공급자 래퍼, Codex는 각각 별도 릴리스 축에서 발전한다.
다음 유지보수 Codex 브리지는 아키텍처상 Akra에 더 가깝지만 Canvas v1.2.1에는 포함되지 않는다.

### 출시된 v1.2.1

보안 및 개인정보와 관련된 발견 사항은 다음과 같다.

- 브라우저 로컬 스토리지의 평문 백엔드 API 키
- 전체 호스트 파일 시스템 접근으로 문서화된 샌드박스 없는 로컬 기본값
- 기본적으로 비활성화된 바깥쪽 OpenHands 확인과 full-access Codex ACP 프리셋
- Python ACP 브리지의 자동 첫 옵션 권한 선택
- 상위 환경에서 시작하여 일부 변수만 제거하고 대화 레지스트리의 모든 비파일 시크릿을 추가하는
  ACP 하위 환경
- 출시된 Canvas 경로에서 환경 로그인을 사용하는 동일 공급자 동시 대화가 공급자 HOME/구성/잠금
  상태를 공유할 가능성
- 애플리케이션 수준 webhook 재전송 중복 제거 없음
- DNT나 환경 정책으로 비활성화하지 않는 한 브라우저 플랫폼, 사용자 에이전트, referrer, origin,
  embedded 상태를 포함한 하나의 익명 설치 이벤트가 일반 추적 동의 전에 전송될 수 있음

### 이후 경로, 출시되지 않음

유지보수되는 Codex ACP 브리지에는 눈에 보이는 redaction 없이 원시 app-server 프레임과 프롬프트를
기록할 수 있는 opt-in 로그 모드가 있다. 이는 이후 스택에 대한 추론된 위험이지 Canvas v1.2.1
발견 사항은 아니다.

이 발견들이 적절히 격리된 배포에서 원격으로 악용할 수 있는 취약점을 입증하지는 않는다. Akra가
더 좁은 런타임 경계, 명시적인 하위 환경 정책, 서버 측 자격 증명 소유권을 유지해야 한다는 점은
입증한다.

## Akra 비교

| 차원 | Agent Canvas v1.2.1 | Akra 기준선 | 결론 | 증거 |
| --- | --- | --- | --- | --- |
| 제품 중심 | 다중 에이전트 브라우저 제어 센터 | Codex 네이티브 운영자 및 전달 계층 | 서로 다름 | [Canvas](evidence.md#product-packaging-and-boundary), [Akra](evidence.md#akra-baseline-evidence) |
| 선택 세션 | 응집력 있는 채팅/파일/diff/로그/브라우저/태스크 검사기 | 집중형 overlay와 Admin 상세가 있는 네이티브 inline TUI | 검사기 응집성에서는 Canvas가 앞섬 | [Canvas](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence) |
| 플릿 감독 | 필터 가능한 대화 레일, 거친 상태 | 태스크/에이전트/worktree/전달 토폴로지, 약한 현재 활동 | 나뉨 | [Canvas UX](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence) |
| Codex 런타임 | Agent Server와 출시된 내장 코어 ACP 브리지 | 직접 공식 `codex app-server` 어댑터 | 구조상 Akra가 앞서며 종단 간 충실도는 미검증 | [ACP](evidence.md#released-codex-acp-path), [Akra](evidence.md#akra-baseline-evidence) |
| 권한 | 바깥쪽 정책 꺼짐, ACP 옵션 0 자동 선택 | 직접 app-server 경계이지만 accept/decline만 지원; file-change approval과 MCP elicitation은 양식 없이 fail closed | 구조상 Akra가 앞서지만 선택 충실도는 불완전 | [ACP](evidence.md#released-codex-acp-path), [Akra](evidence.md#akra-baseline-evidence) |
| 격리 | 연결된 작업 공간 기본값은 로컬, scratch는 worktree 요청으로 폴백, 정리 경로 없음 | 병렬 lane별 leased worktree | 전달 권한에서는 Akra가 앞섬 | [Canvas](evidence.md#conversation-creation-reconnect-and-workspaces), [Akra](evidence.md#akra-baseline-evidence) |
| 전달 | Git/PR 액션은 프롬프트 | 고정 범위, 리뷰/검사 정책, 통합 검증, 정리가 출시됨; 정확한 SHA 검증은 계획됨 | Akra가 앞서지만 증명 게이트는 불완전 | [Canvas](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence), [계획된 검증](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) |
| 원격 운영 | 로컬/원격/클라우드 백엔드 레지스트리와 상태 | loopback Admin 및 Telegram 제어 영역 | Canvas가 앞섬 | [Canvas](evidence.md#backend-registry-and-telemetry), [Akra](evidence.md#akra-baseline-evidence) |
| 자동화 | 일정, webhook, 영속 실행, 일부 Canvas UI | planning tool과 parallel tick, trigger/run 원장 없음 | Canvas가 앞섬 | [Canvas](evidence.md#automation-114), [Akra](evidence.md#akra-baseline-evidence) |
| 시크릿 | 로컬 스토리지의 백엔드 키, 광범위한 ACP 하위 환경 | 필터링된 app-server 하위 환경, 원격 레지스트리 없음 | 구조상 Akra가 앞서며 원격 설계는 미정 | [Canvas](evidence.md#backend-registry-and-telemetry), [Akra](evidence.md#akra-baseline-evidence) |
| 성능 | 비교 가능한 제품 벤치마크를 찾지 못함 | 완전한 네이티브 성능 계약이 아직 없음 | 알 수 없음 | [제한](evidence.md#audit-limits), [계획된 계약](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) |
| 릴리스 검증 | 전체 Ubuntu JS 게이트, Windows 앱 빌드, 조건부 E2E | Rust format/test/clippy 및 네이티브 플랫폼 대상 작업 | 서로 다르며 단일 승자 없음 | [Canvas](evidence.md#quality-and-validation), [Akra](evidence.md#akra-baseline-evidence) |

## 결정 사항

### 채택

- 기존 TUI/Admin 병렬 투영에 에이전트별 제한된 현재 활동과 마지막 활동 경과 시간 추가
- 트리거, 태스크, 에이전트 세션, app-server thread, 검증, PR, 통합, 정리를 잇는 선택 실행 딥 링크
- 검토되는 Codex 코드 변경 수신을 위한 일정, 서명된 이벤트 트리거, 활성화/비활성화, 즉시 실행,
  취소, 영속 실행 이력
- 향후 읽기 전용 원격 Akra 노드 상태, 기능, 버전, 오래된 상태 가시성

### 거부

- Akra의 핵심인 ACP 또는 다중 공급자 런타임
- 경쟁 브라우저 IDE로서 Monaco, xterm-log 또는 브라우저 스크린샷 복제
- 범용 Slack, Notion, 임의 워크플로 마켓플레이스 범위
- 권위 있는 Git 또는 전달 제어로서의 프롬프트 삽입
- 병렬 전달을 위한 선택적 worktree 격리
- 원격 노드 자격 증명을 위한 브라우저 로컬 스토리지
- 자동 권한 선택

### 차별화

- `event -> accepted task -> leased worktree -> exact-SHA validation -> reviewed PR -> verified
  integration -> cleanup`을 하나의 영속적인 애플리케이션 추적으로 만든다.
- TUI, Admin, CLI, Telegram, 이후 원격 읽기 모델을 통해 같은 진실을 투영한다.
- Admin 게임의 이동, 패킷, 차단 요소, 리뷰 대기, 진행을 장식적인 연속 활동이 아니라 영속 상태
  전환의 결과로 만든다.
- 직접 공식 app-server 스키마 책임을 유지하고 프로토콜 relay를 추가하지 말고 필드 충실도를
  측정한다.

## 갱신 조건

다음 경우 이 분석을 갱신한다.

- Agent Canvas가 고정된 Agent Server 1.33.0을 교체하거나 app-server 기반의 유지보수 Codex ACP
  경로를 출시하는 경우
- Canvas 또는 Automation이 6개 상태 실행 계약을 수정하거나, 재전송 중복 제거를 추가하거나,
  자격 증명 모델을 변경하는 경우
- Canvas가 출시된 플릿 토폴로지, 전달 권한, worktree 정리 또는 대화형 작업 공간 도구를 추가하는
  경우
- Akra가 활동 envelope, 진실한 diorama, 자동화 수신 또는 원격 노드 레지스트리를 출시하는 경우
- 동일 머신 인증 벤치마크 또는 fault-injection 행렬을 사용할 수 있게 되는 경우

## 출처

고정된 저장소, 줄 단위 소스 링크, 명령, 관찰된 릴리스 게이트 출력, 검증되지 않은 실험은
[evidence.md](evidence.md)를 참조한다.
