# OpenCode v1.17.18 심층 분석

[English](../../../competitive/opencode/analysis.md)

이 분석은 OpenCode v1.17.18과 prerelease 커밋
`354f4782a73ffabab6abf342cb0f37285a96c177`의 Akra를 비교한다. 변경 불가능한 소스 링크,
재현 명령, 릴리스 산출물 식별 정보, 감사 한계는 [evidence.md](evidence.md)에 있다. Akra를
기준으로 한 결정과 리뷰 가능한 구현 단위는 [gap-matrix.md](gap-matrix.md)에 있다.

## 요약 결론

OpenCode는 Codex 래퍼가 아니라 강력한 터미널 제품을 갖춘 폭넓은 코딩 에이전트 플랫폼이다.
프로바이더 라우팅, 모델 실행, 도구, 권한, MCP, LSP, 플러그인, 세션, 영속성, 서버 API, 웹 및
Electron 클라이언트, GitHub 자동화, 이미 마운트되었지만 아직 완성되지 않은 실험적 2세대
런타임까지 소유한다. OpenCode를 선택할 이유는 폭넓은 기능과 세련되고 연결 가능한 TUI의
결합이다. 한 번 설치하면 터미널, 브라우저, 데스크톱, SDK, 자동화 표면에서 여러 프로바이더와
프로젝트를 운용할 수 있다.

Akra에 가장 강력한 위협은 다음과 같다.

1. 별도로 시작한 서버가 attach 클라이언트가 분리된 뒤에도 살아 있을 때 가능한 서버 범위
   비동기 턴과 외부 attach, 그리고 해당 서버에 대한 웹/TUI 공유 접근;
2. 세션 탐색, diff/타임라인 워크플로, 자식 탐색, 권한/질문 양식, 명령 탐색, compact 터미널
   모드를 갖춘 프로토콜이 풍부한 TUI;
3. 에이전트를 실행하고 커밋, 브랜치, pull request를 만들 수 있는 GitHub 이벤트 및 일정 진입점;
4. 호스팅 웹, 임베디드 웹, 6개 데스크톱 릴리스 대상에서 재사용되는 폭넓은 공유 API와
   애플리케이션 UI.

이 장점들이 성능, 내구성, 안전성의 승자를 입증하는 것은 아니다. 감사에서는 동일 머신에서
인증한 공정한 대화형 벤치마크를 찾지 못했다. OpenCode의 수동 렌더러 성능 스위트에는 머신과
무관한 예산이 없고 패키징된 Electron 프로세스 트리를 제외한다. 실시간 외부 이벤트 스트림에는
사용 가능한 재생 커서가 없고, 레거시 백그라운드 작업 소유권은 프로세스 로컬이며, GitHub 경로는
Akra의 리뷰된 고정 범위 통합 권한을 제공하지 않는다.

악용 가능성이 있는 가장 중대한 약점은 신뢰 경계다. 재현한 격리 서버에서는 암호가 없으면 인증도
없었고, 인증되지 않은 `GET /config`가 설정을 통해 제공된 해석 완료
`openai.options.apiKey`와 비활성 로컬 MCP `environment` 카나리를 반환했다. 레거시
`GET /provider`는 프로바이더 카나리를 반환했다. `{file:...}`, 프로바이더 인증 파일,
원격 MCP 헤더/OAuth, 실제 MCP 실행, GitHub 토큰, v2 설정 시크릿 출력은 동적으로 테스트하지
않았다. 소스 검사에서는 직접 읽기 `.env` 확인을 합성하지 않는 셸 경로와, 활성 앱 토큰을 저장소
로컬 Git 설정에 기록해 그 헤더를 남길 수 있는 GitHub 자동화 경로도 발견했다. 이는 범위가 제한된
발견이며 모든 기본 TUI 세션이 원격으로 노출된다는 주장이 아니다. 기본 TUI는 인프로세스 전송을
사용하고 독립 실행 서버는 운영자가 네트워크 설정을 바꾸지 않는 한 기본적으로 루프백을 사용한다.

Akra는 OpenCode의 프로바이더 런타임, 플러그인 호스트, 브라우저 IDE, 선택적 인증 모델을
복제하는 방식으로 대응해서는 안 된다. 더 좁은 제품을 날카롭게 다듬어야 한다.

> Akra는 공식 `codex app-server` 세션을 내구성 있고 검사 가능하며 리뷰를 거쳐 전달되는
> 작업으로 전환하는 Codex 우선 운영 및 전달 계층이다.

이를 위해 먼저 Akra의 실시간 실행, 조정/복구, 측정 가능한 성능, 정확한 소스 전달 증거 격차를
해소해야 한다. 승인 선택 충실도는 경계가 명확한 프로토콜 단위로 뒤따르며, fork 계보와 명령 탐색은
이후의 세션/TUI 작업으로 남는다. 기존 전달, 인증된 Admin, 기본으로 정제된 자식 환경 경계는
대체 제품 명제가 아니라 눈에 보이고 카나리로 테스트되는 불변 조건으로 유지해야 한다.

## 제품과 대상 사용자

OpenCode는 모델 프로바이더와 표면 전반에서 하나의 에이전트 하네스를 원하는 개발자를 대상으로
한다. 감사한 릴리스는 다음을 제공한다.

- 대체 화면 TUI와 compact 네이티브 스크롤백 모드;
- 헤드리스 `run`, 장기 실행 `serve`, 브라우저를 여는 `web`, 외부 `attach` 명령;
- 세션 내보내기/가져오기, 통계, 프로바이더 및 에이전트 관리, MCP, 플러그인, ACP, GitHub,
  pull request 명령;
- 호스팅 웹, 임베디드 서버 UI, Electron 데스크톱에서 사용하는 공유 Solid 애플리케이션;
- SDK/OpenAPI 표면과 실험적 v2 프로토콜/런타임;
- 터미널 클라이언트를 시작하고 현재 편집기 컨텍스트를 덧붙이는 VS Code 계열 확장;
- GitHub 이슈, pull request, 리뷰 댓글, 일정, 수동 dispatch 자동화.

이 폭넓음은 실제 제품상의 장점이다. 동시에 Akra와 다른 제품 명제이기도 하다. Akra 운영자는
프로바이더 이식성이나 임의 확장 호스트가 아니라 공식 Codex 동작과 내구성 있는 전달 권한을
선택한다.

## 아키텍처와 런타임 권한

### 현재 OpenCode 경로

레거시/현재 경로에는 세션, 프로바이더, 도구, 설정, 파일, PTY, 권한, 공유, 이벤트를 관할하는
하나의 서버 권한이 있다. 일반 TUI는 worker를 띄우고 인프로세스 fetch 전송과 직접 이벤트
브리지를 사용한다. 외부 attach는 HTTP와 SSE를 사용한다. `serve`, `web`, 호스팅 애플리케이션,
Electron 렌더러, 확장, 생성된 SDK는 서로 다른 방식으로 서버 경계를 재사용한다.

이 토폴로지는 클라이언트 수명과 턴 수명을 분리할 수 있다. 프롬프트가 서버 범위에서 비동기로
실행되는 동안 별도의 `attach` 명령이 독립적으로 실행 중인 `serve` 프로세스에 나중에 연결할 수
있다. 기본 `opencode` TUI는 이 생존 계약을 제공하지 않는다. worker 서버를 소유하고 종료 시
worker를 끈다. 소스가 토폴로지를 검증하지만, 이 감사에서는 종단 간 detach 실험을 실행하지
않았다. 현재 run 레지스트리가 메모리에 있고 인스턴스 폐기가 활성 작업을 취소하므로 crash-safe
실행의 증거도 아니다.

외부 이벤트 경로에도 연속성 구멍이 있다. 서버는 재생 가능한 SSE ID 없이 이벤트를 내보낸다.
TUI는 제한된 backoff로 재시도하지만 연결 해제 뒤 완전한 스냅샷을 다시 bootstrap하지 않는다.
브라우저 애플리케이션은 더 넓은 root/directory bootstrap을 수행하고 재연결 뒤 세션 목록과
상태를 새로 고치지만 캐시된 모든 세션 message/part 창의 재로딩을 강제하지는 않는다. 따라서
어느 외부 클라이언트든 이벤트 공백을 지나 오래된 transcript 창을 유지할 수 있고, 브라우저는
catalog/status 상태를 더 많이 복구한다고 추론하는 것이 합리적이다. 기본 인프로세스 TUI 경로는
같은 방식으로 네트워크 연결 해제의 영향을 받지 않는다.

헤드리스 `opencode run` 역시 stdout과 idle 완료를 실시간 이벤트 스트림에서 도출하고, 누락된
출력을 복원하는 데 성공한 프롬프트 응답 본문을 사용하지 않는다. 레거시 무제한 이벤트 큐 및 재생
ID 부재와 결합하면 연결 해제로 stdout이 불완전해지거나 비정상적으로 완료를 기다릴 수 있다.
이는 소스가 뒷받침하는 추론이지 재현된 장애가 아니다.

### 마운트된 v2 경로와 성숙도

배포된 listener는 레거시 API 옆에 두 번째 프로토콜과 서버 route 계층을 마운트한다. 따라서
그 handler, 내구성 있는 이벤트/projector 저장소, 프로세스 로컬 run coordinator, 세션별
history/replay route는 endpoint 이름이나 소스 주석이 인터페이스를 experimental로 표시해도
소스로 검증된 배포 코드다. 증거 상태와 제품 성숙도는 별개의 축이다.

중요한 소유권, 재시도, 취소, 플러그인 동등성, 오래된 작업, crash 연속성 동작은 여전히 명시적으로
미완성이다. 마운트된 v2 프로바이더 응답은 원시 요청 헤더와 본문도 전달한다. v1 설정 lowering은
그 헤더에 `Authorization`, `x-api-key`, `api-key` 값을 넣을 수 있다. 이 응답 위험은 소스로
검증했지만 시크릿 카나리로 동적 재현하지 않았다. 마운트된 route는 v1.17.18에 내구성 있는
멀티 노드 실행이 있음을 입증하지 않는다.

### Akra 결정

공식 `codex app-server`를 런타임 권한으로 유지한다. OpenCode식 프로바이더/도구 daemon을
만들지 않는다. 대신 다음을 수행한다.

- 자식 또는 전송 손실 뒤 공식 thread/turn API가 현재 권한을 제공하는 필드만 조정하고, 복구할
  수 없는 이벤트 상세는 unknown으로 표시한다;
- TUI, Admin, CLI, Telegram, 자동화 전반에 typed snapshot과 공백 인식 projection을 유지한다;
- transcript 연속성, 클라이언트 detach 연속성, 프로세스 crash 연속성을 구분한다;
- 공식 증거 없이 재시작된 Akra 프로세스가 턴을 계속 살렸다고 절대 주장하지 않는다.

## TUI와 상호작용 설계

### 선택적으로 도입할 강점

OpenCode의 TUI는 제품에서 가장 강력한 부분이다. 명령 팔레트와 leader 기반 which-key 표면은
많은 명령 어휘를 검색하고 학습할 수 있게 한다. 세션 처리는 검색, 고정, 이름 변경/삭제, 빠른
slot, 모델/프로바이더/에이전트 선택, fork, revert, compaction, timeline, 공유, 자식 세션
탐색을 포함한다. 권한 및 질문 흐름은 Akra의 현재 accept/decline modal보다 더 많은 typed
선택을 보존한다. diff와 workspace view는 모든 사실을 상시 chrome으로 만들지 않고 응집력 있는
drilldown을 제공한다.

터미널 수명 주기에도 차용할 만한 패턴이 있다. 표준 모드는 내부 renderer loop와 대체 화면을
대상으로 한다. `--mini`는 제한된 split footer, 네이티브 scrollback, replay, reset 로직을
사용한다. Akra는 자체 inline-scrollback 계약을 유지하되 OpenCode viewport를 그대로 도입하는
대신 resize/replay 교훈을 이용해 복구를 강화해야 한다.

OpenCode는 최신 100개 메시지만 TUI 상태에 hydrate하고 timeline은 그 인메모리 집합을 읽는다.
브라우저 애플리케이션은 이전 메시지를 batch로 page한다. 합리적인 반응성 절충이지만 TUI의 긴
세션 timeline이 완전한 history는 아니다. Akra는 명시적 truncation과 공식 load-more 경로를
유지하면서 적응형 제한 projection을 사용해야 한다.

### Akra 기준선

Akra에는 이미 단일 `:` 명령 레지스트리, 검색 가능한 inline palette, 모델 및 세션 overlay,
네이티브 host scrollback, 공식 세션 browser, planning authority, 병렬 operations board가
있다. 격차는 "명령 팔레트를 추가하라"가 아니다. 기존 레지스트리를 상황에 맞게 만드는 것이다.

- 작업을 현재 사용할 수 있는지와 사용할 수 없다면 그 이유를 노출한다;
- 같은 metadata를 palette, help, 선택적 leader/which-key discovery에 projection한다;
- shortcut 전용 명령을 따로 유지하지 말고 하나의 실행 정의를 보존한다;
- 승인 dialog는 modal이며 우회할 수 없게 유지한다;
- 새로운 범용 widget framework가 아니라 작업 관련성과 터미널 너비에 따라 접는다.

## 세션, 포크, 백그라운드 작업

OpenCode는 메시지를 새 세션으로 복사하여 세션을 fork할 수 있다. 레거시 fork는 새 세션에 명시적
source/fork 계보를 유지하지 않는다. Akra가 고정한 공식 app-server schema에는 이미
`thread/fork`, 선택적 턴 선택, `forkedFromId`가 있지만 Akra 애플리케이션 경로가 없다. 이는
transcript 복사보다 더 좁고 책임 있는 워크플로를 제공할 기회다. 공식 프로토콜을 통해 fork하고,
반환된 계보를 보존하며, 원본 thread가 변경되지 않았음을 입증한다.

OpenCode는 자식 세션과 실험적 background subagent도 지원한다. 보이는 계보와 탐색은 유용한
UX다. background registry는 프로세스 로컬이며 내구성이 없음을 명시한다. 재시작 또는 owner
scope 종료 시 상태를 잃고 진행 중인 작업이 중단된다. 운영자가 별도 workspace를 만들지 않으면
task subagent는 현재 directory를 공유한다. background 완료는 synthetic prompt로 parent에
비동기 주입되고, 레거시 runner는 새 작업을 독립적으로 schedule하지 않고 이미 실행 중인 loop에
join한다. 늦게 주입된 작업이 loop의 마지막 history read 뒤 pending으로 남는 좁은 race를
합리적으로 추론할 수 있으나 감사에서는 재현하지 않았다. Akra는 계보와 완료 projection을
차용하되 같은 checkout이나 프로세스 로컬 background authority는 차용하지 않아야 한다.

## 웹, 데스크톱, IDE 인터페이스

공유 애플리케이션은 폭넓고 응집력이 있다. 서버, 프로젝트, worktree, 세션, 파일, diff, PTY,
프로바이더, 설정을 관리한다. CLI는 정적 build를 embed하고 호스팅 앱은 서버에 연결할 수 있다.
Electron은 같은 renderer를 embedded server와 함께 패키징하고 유용한 로컬/WSL 통합을
제공한다. renderer 보안 설정에는 context isolation, Node integration 비활성화, sandboxing,
protocol path check, permission allowlisting이 포함된다. 데스크톱 패키징은 6개 대상 조합에
hardened runtime/notarization과 플랫폼 signing도 설정한다.

대가는 상당한 표면과 footprint다. 릴리스 API에서 관찰한 v1.17.18 데스크톱 installer는 약
104~158 MiB였고 제품은 여전히 desktop beta로 표시된다. 로컬 build에서는 압축 전 2.8 MB 웹
entry JavaScript chunk와 6.7 MB Electron renderer entry가 생성됐다. 이는 build footprint
사실이지 startup 또는 RAM 측정값이 아니다.

username과 password를 포함한 browser server credential은 local storage에 영속된다. Akra는
이 소유권 모델을 거부해야 한다. Admin은 범용 remote IDE가 아니라 server-side application
authority를 갖춘 인증된 loopback operator projection으로 남아야 한다.

VS Code 확장은 실용적인 터미널 launcher와 context bridge다. split terminal에서 `opencode`를
시작하고, 준비 상태를 잠시 polling하며, 현재 file 또는 selection을 TUI에 보낸다. editor-native
diff, chat, code-action 경험은 아니며 확장 테스트도 발견되지 않았다. Akra는 네이티브 operator
loop가 측정 가능하게 우수해질 때까지 IDE 확장을 우선하면 안 된다.

## 워크트리, GitHub, 전달

OpenCode에는 실제 worktree service가 있다. `opencode/<slug>` branch와 worktree를 만들고,
startup command를 실행하며, reset하고, 제거할 수 있다. reset 경로는 non-primary worktree에서
의도적으로 파괴적이다. hard reset, `clean -ffdx`, submodule cleanup을 수행한다. 이는 유용한
대화형 workspace 관리이지 task-owned delivery authority가 아니다.

GitHub action은 여러 event type을 받아 agent session을 시작하고, `git add .`으로 변경을 stage한
뒤 commit, push하고 pull request를 생성하거나 갱신한다. 사용자 촉발 event에서는 actor의 repository
permission을 확인한다. 생성 workflow는 변경 가능한 `anomalyco/opencode/github@latest`를 참조한다.
agent가 branch를 바꾸면 handler가 agent가 직접 전달을 관리했다고 가정하므로 infrastructure
delivery를 건너뛸 수 있다.

public repository에서는 `share: false`를 설정하지 않으면 GitHub Action이 기본적으로 session을
공유한다. 이는 event별 전달 전에 일어나며 share 경로를 통해 session message, part, diff, model
data를 업로드한다. 호스팅 접근, 보존, 삭제, redaction 정책은 검증하지 않았다. 이는 단순한 협업
편의가 아니라 중대한 신뢰 절충이다.

감사에서는 필수 review, 필수 check, 정확한 source-SHA 검증, 보호된 base로의 직렬화된 integration,
원격 integration 검증, 조건부 cleanup에 대한 구조적 gate를 찾지 못했다. 따라서 OpenCode는 유용한
intake ergonomics를 보여 주지만 Akra의 delivery contract를 대체하지 못한다.

Akra의 현재 review된 default는 실질적으로 더 강하다. lane마다 하나의 leased worktree, 고정된
source 및 target identity, review된 PR head, 직렬화된 integration worktree, remote verification,
조건부 branch/slot cleanup이 있다. 명시적인 parent-level high-risk autonomous mode는 예외로
남으며 review된 전달로 계산하지 말고 review-skipped로 projection해야 한다.

## 안전성과 신뢰 경계

### 재현된 시크릿 공개 경계

`OPENCODE_SERVER_PASSWORD`를 설정한 격리된 `serve` 프로세스는 Basic Auth 없이 `401`,
인증하면 `200`을 반환했다. 이 변수가 없으면 같은 서버가 경고만 하고 요청을 허용했다.
`{env:...}` 치환을 사용한 카나리 설정에서 다음 결과를 검증했다.

- 인증되지 않은 `GET /config`가 HTTP `200`과 해석 완료된 프로바이더 및 MCP 카나리를 반환했다;
- 인증되지 않은 `GET /provider`가 프로바이더 카나리를 반환했다;
- 실험 동안 프로세스는 loopback 전용이었다.

소스가 이 결과를 설명한다. 변수 치환은 parsing 전에 일어나고 config handler는 해석된 설정을
반환하며 provider public mapping은 legacy `key`를 제거하지 않고, 인증 middleware는 암호가
없으면 pass-through다. loopback은 remote reach를 줄이지만 같은 사용자로 실행되는 다른 로컬
프로세스에서 보호하지 않는다. 명시적 host 또는 mDNS 설정으로 경계를 넓힐 수 있다. 서버는 내장
TLS 대신 HTTP를 노출하며 인증은 URL query token도 허용한다. 따라서 non-loopback 배포에는 별도로
운영되는 TLS 경계가 필요하다. Basic Auth만으로는 인증일 뿐 전송 기밀성이 아니며 query credential은
URL history나 proxy log에 들어갈 수 있다.

### 기능 불일치

기본 권한 규칙은 `.env` 패턴 직접 읽기 전에 묻는다. 셸 도구는 별도로 명령을 parse하고 workspace
밖의 external-directory access를 물은 다음 bash pattern을 확인한다. workspace 내부의
`cat .env`에는 direct-read permission을 적용하지 않는다. 기본 wildcard가 bash를 허용하므로 이
셸 경로가 직접 읽기 prompt를 우회한다는 추론을 소스 합성이 뒷받침한다. 인증된 모델 주도 악용은
수행하지 않았다.

Akra는 Codex 앞에 두 번째 tool sandbox를 구현하는 방식으로 반응해서는 안 된다. 심어 둔 canary로
공식 Codex sandbox와 approval contract를 검증하고, credential/content class별 허용 sink와 금지
sink를 정의하며, Akra가 environment, approval, logging, delivery, transport를 소유하는 곳에서는
fail closed해야 한다. 이는 일반적인 data-loss prevention이나 운영자가 Codex에 읽도록 명시적으로
허용한 content의 억제가 아니라 제한된 non-amplification을 입증할 수 있다.

### GitHub 자격 증명 수명

OpenCode는 agent 실행 전 GitHub app token을 HTTP extra header로 repository-local `.git/config`에
기록한다. 이전 header가 없으면 restore function은 새 값을 제거하지 않고 return한다. token은 나중에
revoke되지만 run 동안 worktree에서 활성 값을 읽을 수 있고 잔여물이 남을 수 있다. token exchange,
attachment acquisition, Git credential configuration도 handler의 actor-authorization check보다 먼저
이뤄진다. 감사에서는 token exfiltration을 실행하지 않았다.

Akra의 GitHub write는 credential mint 또는 untrusted download 전에 identity를 계속 authorize한
다음 격리된 ephemeral credential context를 사용해야 한다. 제한된 egress contract는 token이 승인된
outbound authentication sink에 도달하지만 prompt, source Git configuration, log, Admin response,
PR body, durable application state에는 들어가지 않음을 입증해야 한다.

### 플러그인, MCP, 공급망

OpenCode의 확장성은 실제 강점이다. Server plugin은 provider/auth behavior, chat parameter와 header,
tool, permission, shell environment, tool execution, compaction을 변경할 수 있고 TUI plugin은 terminal
동작을 확장할 수 있다. 이 폭넓음 덕분에 고급 사용자는 core release를 기다리는 대신 하나의 harness를
조정할 수 있다.

동시에 신뢰 경계이기도 하다. plugin은 server process 안에서 trusted code로 실행되고 authenticated
client, project/worktree path, hook, `Bun.$`를 받는다. local MCP process는 전체 environment를
상속한다. 일부 LSP installer는 default branch archive나 `@latest` tool version 같은 변경 가능한
upstream artifact를 사용한다. 이는 sandbox가 아니라 operator가 신뢰하는 extension boundary다.

Legacy provider auth와 MCP OAuth state는 mode-`0600` plaintext JSON으로 저장되고 v2 credential
table은 SQLite에 JSON value를 저장한다. file mode는 다른 OS user의 access를 줄이지만 encryption이나
OS keychain은 아니다. Akra는 raw third-party credential을 application persistence에 저장하지 않아야
한다. 향후 persistent secret dependency는 별도로 소유하는 secret-store handle을 사용해야 한다.

Akra core 방향으로 임의 in-process plugin을 거부한다. Port는 실제 outbound boundary를 나타내야
하며 operator가 선택한 MCP behavior는 공식 Codex capability와 approval semantic 아래에 남는다.

## 성능

비교상의 승자는 확립되지 않았다.

OpenCode에는 공유 browser renderer를 위한 유용한 performance instrumentation이 있다. cold/warm tab,
streaming throughput, animation-frame gap, long task, layout mutation, Chrome trace를 다룬다. 이 suite는
하나의 worker로 production build에 대해 실행된다. 자체 문서는 결과가 machine-dependent라고 밝히고
portable budget을 주장하지 않으며 packaged Electron process behavior를 향후 작업으로 남긴다.

감사에서는 interactive latency가 아닌 artifact 및 build footprint를 기록했다.

- Linux x64 CLI 압축 파일: 69,427,073바이트;
- 압축을 푼 Linux x64 바이너리: 188,979,328바이트;
- 웹 진입점 JavaScript: gzip 전 2,815.62 kB, gzip 후 842.34 kB;
- Electron 렌더러 진입점: 압축 전 6,731.22 kB;
- Electron 설치 프로그램: 주요 대상 전반에서 약 103.9~158.3 MiB.

Akra에도 완전하고 versioned된 process-tree benchmark가 없다. 기존
[Native Performance Evidence Contract](../jcode/gap-matrix.md#p0-native-performance-evidence-contract)가
여전히 올바른 dependency다. OpenCode renderer metric은 추가 stress case에 참고할 수 있지만 두 번째
evidence schema나 호환되지 않는 숫자에 기반한 주장을 만들어서는 안 된다.

## 품질과 릴리스 규율

정확한 source가 `bun typecheck`를 통과했다. `packages/opencode` suite는 22 skip, 1 todo, 실패 0으로
3,110개 테스트를 통과했다. app package script는 CI 조건에서 567개 unit test와 27개 browser test를
통과했고, 별도로 실행한 workflow에 연결되지 않은 desktop suite는 61개 테스트를 통과했다. 저장소는
상당한 Linux/Windows unit 및 Chromium E2E job 범위, generated-client check, exact dependency install,
dependency-age policy, SHA-pinned GitHub Actions를 갖추고 있다.

의미 있는 격차도 있다.

- Arabic locale에는 English key 3개가 없고 focused parity test에서 실패를 재현했지만 `CI`가 설정되면
  해당 suite를 skip한다;
- desktop test file이 있지만 desktop package에 `test` script가 없어 monorepo test task가 이를
  실행하지 않는다;
- root lint가 검사한 GitHub workflow에 연결되지 않았다;
- publish DAG는 test 또는 typecheck job에 구조적으로 의존하지 않는다;
- VS Code extension에는 test configuration이 있지만 대응하는 test가 없다;
- Nix desktop expression은 Electron 41을 사용하지만 package manifest는 42.3.3을 사용하고 Nix
  desktop evaluation은 warning-only다;
- gate를 추가할 수 있는 외부 branch protection 또는 release policy는 검증하지 않았다.

이 발견들이 릴리스가 전반적으로 망가졌음을 의미하지는 않는다. 설정된 job과 code 존재가 실행 결과에
결합된 release DAG보다 약한 증거인 이유를 보여 준다.

## Akra 비교

| 차원 | OpenCode v1.17.18 | Akra `354f4782` | 결론 |
| --- | --- | --- | --- |
| 제품 명제 | 프로바이더와 표면을 소유하는 폭넓은 agent platform | 공식 Codex 운영 및 review-delivery layer | 의도적으로 다르다. Akra는 자신의 wedge를 더 명확히 밝혀야 한다 |
| 런타임 권한 | model, tool, provider auth, session, server, client를 소유 | model/tool execution을 공식 app-server에 위임 | Akra의 더 좁은 경계를 보존한다 |
| TUI 폭 | 풍부한 palette, key discovery, session, fork, timeline, diff, question, permission | inline host scrollback, 검색 가능한 `:` registry, planning 및 parallel operation, protocol detail gap | interaction 폭에서는 OpenCode가 앞선다 |
| Detach 연속성 | 별도 `serve`와 `attach`로 client 수명 분리 가능, 기본 TUI는 worker를 소유하고 중지, live detach 미재현 | child connection이 runtime을 소유, official transcript resume 가능 | OpenCode의 attach topology가 더 강하지만 종단 간 결과는 추론으로 남는다 |
| 외부 재연결 | replay cursor나 TUI full resync 없이 재시도 | restart recovery 미완성 | 둘 다 명시적 reconciliation 필요, durability 승자는 없다 |
| Fork 계보 | 명시적 source lineage 없는 transcript-copy fork | 공식 schema가 `thread/fork`와 `forkedFromId` 지원, Akra 경로 없음 | Akra가 차별화할 기회다 |
| Background agent | child session, experimental process-local background job, shared directory | worktree 격리와 durable lease/detail을 갖춘 고정 lane | 대화형 fan-out은 OpenCode가, isolation/delivery는 Akra가 앞선다 |
| Worktree | 대화형 create/reset/remove 및 startup command | leased lane lifecycle과 identity-checked cleanup | delivery에서는 Akra가 앞서며 interactive scope는 다르다 |
| GitHub 자동화 | 폭넓은 event와 prompt-driven commit/push/PR | frozen source, review/check default gate, serialized integration, remote verification, cleanup | delivery authority는 Akra가, intake 폭은 OpenCode가 앞선다 |
| 서버 안전성 | password optional, loopback default, password 없는 raw resolved config/provider response 재현 | loopback-only Admin, mandatory capability/session auth, origin guard | 구조적으로 Akra가 앞서지만 제한된 egress 증거는 여전히 필요하다 |
| 승인 충실도 | typed permission 및 question UI | 제한된 one-shot accept/decline, file approval과 MCP elicitation decline | 선택 충실도는 OpenCode가 앞선다 |
| 웹/데스크톱 | 공유 full application과 6개 desktop target | authenticated operational Admin, desktop 없음 | 제품 폭은 OpenCode가 앞서나 복제는 거부한다 |
| IDE | terminal launcher와 context bridge | 없음 | OpenCode가 앞서지만 Akra 우선순위는 낮다 |
| 성능 증거 | portable budget 없는 manual renderer suite | 기존 terminal validation, versioned full process-tree benchmark 없음 | 승자는 알 수 없고 Akra evidence gap은 P0로 남는다 |
| 품질 | green Turbo core/app task와 별도로 실행된 workflow-ungated desktop test, release-gate gap | 폭넓은 Rust/native gate와 delivery test, 이 docs audit에서는 Akra full CI 미실행 | 전체 품질 승자는 없다 |

## 결정

### 도입

- 기본 TUI의 연결 분리나 비정상 종료 생존을 주장하지 않으면서, 기존 활성 턴 종료 및 재시작 조정
  계약의 증거로 별도 서버와 연결 분리에서 얻은 교훈을 도입한다;
- 타입이 지정된 권한/질문 표현과 정확한 결정 보존;
- 대화 기록 복사가 아닌 공식 포크 계보;
- 기존 실시간 실행 및 세션 출처 작업을 통해 변경 차이, 타임라인, 자식 세션, 장기 세션 투영을
  강화한다;
- Akra 기존 명령 레지스트리의 사용 가능 여부 및 이유 메타데이터와 선택적인 리더 키 방식 탐색;
- 기존 영속 자동화 유입 계획을 통해서만 GitHub 이벤트와 일정의 사용성을 도입한다;
- 기존 성능 근거 계약에 렌더러 부하 차원을 추가한다.

### 거부

- 공식 Codex 옆의 다중 공급자 또는 로컬 도구 런타임;
- 임의의 동일 프로세스 플러그인과 Akra 소유 LSP 설치 프로그램;
- 제어 API 또는 비밀 정보를 담는 API의 선택적 인증;
- Admin을 통한 원시 설정, 공급자, 자격 증명, 파일 시스템, PTY, 셸 변경;
- 브라우저에 저장되는 서버 자격 증명, 공개 저장소의 기본 공유, 자동 공유, 완전한 브라우저 IDE;
- 같은 체크아웃을 쓰는 백그라운드 에이전트와 파괴적인 범용 워크트리 초기화;
- 프롬프트에만 의존하는 전달, `git add .`, 브랜치 관리 우회, 변경 가능한 `@latest` 자동화;
- 네이티브 TUI 성능과 전달 흐름을 측정하기 전의 데스크톱 또는 IDE 확장.

### 차별화

- 타입이 지정된 승인, 추가 정보 요청, 조정, 포크, 변경 차이, 스레드 출처를 갖춘 공식 Codex
  프로토콜의 직접적이고 충실한 구현;
- 인증된 루프백 Admin과 출처부터 도착점까지 추적하는 알려진 카나리 릴리스 매트릭스;
- 명시적인 고위험 예외 출처를 갖춘 정확한 소스, 워크트리 격리, 검토 기반 전달;
- 트리거부터 세션, 브랜치, PR, 통합, 정리까지 이어지는 영속 계획 및 자동화 계보;
- 영속 수명 주기와 활동 전이로만 구동되는 사실에 충실한 게임 운영 투영;
- 반복 가능한 전체 프로세스 트리 근거가 생긴 뒤에만 네이티브 성능을 주장한다.

## 갱신 조건

다음 상황에서 이 분석을 갱신한다.

- OpenCode가 새 주요 런타임을 출시하거나 v2 서버/세션 소유권을 기본값으로 만들 때;
- 서버 인증, 공개 DTO 정제, GitHub 자격 증명, SSE 재생, 백그라운드 지속성, 권한 합성이 변경될 때;
- 릴리스에서 TUI 장기 세션 적재, 포크 계보, 워크트리 전달, 데스크톱 패키징, 성능 모음이 크게
  바뀔 때;
- Akra가 타입이 지정된 승인/추가 정보 요청, 공식 포크 계보, 재연결 조정, 알려진 카나리 기반의
  제한된 외부 전송 매트릭스를 구현할 때;
- 인증된 공정한 OpenCode/Akra 대화형 벤치마크를 사용할 수 있게 될 때.

## 출처

변경 불가능한 링크, 명령, 관찰한 원시 결과, 명시적 한계는 [evidence.md](evidence.md)를 참조한다.
의존성을 고려한 구현 계약은 [gap-matrix.md](gap-matrix.md)를 참조한다.
