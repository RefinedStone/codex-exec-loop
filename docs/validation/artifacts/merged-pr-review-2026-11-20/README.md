# 최근 72시간 머지 PR 리뷰 대응 기록

## 조사 범위

- 저장소: [RefinedStone/codex-exec-loop](https://github.com/RefinedStone/codex-exec-loop)
- 기준: PR 작성·게시 시각이 아닌 `mergedAt`.
- 구간: **2026-11-17 17:21:21 ~ 2026-11-20 17:21:21 (KST)**. 요청을 시작한 시점에 고정한 72시간이다.
- 조사 기준 코드: [d1ee3ad0253e](https://github.com/RefinedStone/codex-exec-loop/commit/d1ee3ad0253e9aa1de102342a476dd9c4d35483e) (`prerelease`).
- 대상 106개 PR의 리뷰 스레드·리뷰 본문·일반 댓글을 대조했다. 수집된 모든 connection은 추가 페이지가 없음을 확인했다.
- 원본 인라인 스레드 55개, 수집 시 미해결 42개.
- 구조화된 전체 판단/근거/검증/대응 결과: [decisions.json](decisions.json).

## 결과

| 구분 | 원본 PR 수 |
| --- | ---: |
| 조치 대상 지적 없음 | 36 |
| 수용·후속 PR 제출 | 22 |
| 반려·판단 문서 | 27 |
| 기대응 | 21 |

수용한 변경은 **22개 후속 PR**에 나누어 제출했다. 원본 #2468과 #2480의 첫 키 생성 경쟁은 #2486으로, #2464와 #2467의 필수 폼 검증 조건은 #2503으로 함께 연결했다. 새로운 PR은 모두 열어 두며 이 작업에서 머지하지 않았다.

판단 기록 113개는 같은 지적을 인라인·일반 리뷰가 각각 확인한 중복 출처를 포함한다. 결함 개수로 사용하지 않는다.

## 수용 후속 PR

CI 확인 시각: 2026-11-20T09:10:16.353Z. 검증 상태는 표의 커밋에 대한 관측 결과다.

| 원본 | 후속 PR | 현재 커밋 | 기반 | 검증 상태 |
| --- | --- | --- | --- | --- |
| [#2460](https://github.com/RefinedStone/codex-exec-loop/pull/2460) | [#2483](https://github.com/RefinedStone/codex-exec-loop/pull/2483) | `397a38be77ce` | `prerelease` | CI Gate 성공 |
| [#2473](https://github.com/RefinedStone/codex-exec-loop/pull/2473) | [#2484](https://github.com/RefinedStone/codex-exec-loop/pull/2484) | `c9ab9c0e8bc8` | `prerelease` | CI Gate 성공 |
| [#2366](https://github.com/RefinedStone/codex-exec-loop/pull/2366) | [#2485](https://github.com/RefinedStone/codex-exec-loop/pull/2485) | `003ff37aa783` | `prerelease` | CI Gate 성공 |
| [#2468](https://github.com/RefinedStone/codex-exec-loop/pull/2468), [#2480](https://github.com/RefinedStone/codex-exec-loop/pull/2480) | [#2486](https://github.com/RefinedStone/codex-exec-loop/pull/2486) | `ad9a9a3858d8` | `prerelease` | CI Gate 성공 |
| [#2458](https://github.com/RefinedStone/codex-exec-loop/pull/2458) | [#2487](https://github.com/RefinedStone/codex-exec-loop/pull/2487) | `16929eb77b6b` | `prerelease` | CI Gate 성공 |
| [#2464](https://github.com/RefinedStone/codex-exec-loop/pull/2464) | [#2488](https://github.com/RefinedStone/codex-exec-loop/pull/2488) | `2af55ad9d8ba` | `prerelease` | CI Gate 성공 |
| [#2468](https://github.com/RefinedStone/codex-exec-loop/pull/2468) | [#2489](https://github.com/RefinedStone/codex-exec-loop/pull/2489) | `d931d83763e1` | `codex/review-vault-first-key` | 현재 head CI Gate 성공 |
| [#2370](https://github.com/RefinedStone/codex-exec-loop/pull/2370) | [#2490](https://github.com/RefinedStone/codex-exec-loop/pull/2490) | `74592b0eec1d` | `prerelease` | CI Gate 성공 |
| [#2373](https://github.com/RefinedStone/codex-exec-loop/pull/2373) | [#2491](https://github.com/RefinedStone/codex-exec-loop/pull/2491) | `47d3a66c1eba` | `prerelease` | CI 진행 중 |
| [#2413](https://github.com/RefinedStone/codex-exec-loop/pull/2413) | [#2492](https://github.com/RefinedStone/codex-exec-loop/pull/2492) | `013452bbd348` | `prerelease` | CI Gate 성공 |
| [#2475](https://github.com/RefinedStone/codex-exec-loop/pull/2475) | [#2493](https://github.com/RefinedStone/codex-exec-loop/pull/2493) | `76fb9a08005b` | `codex/review-vault-access-recheck` | 현재 head CI Gate 성공 |
| [#2476](https://github.com/RefinedStone/codex-exec-loop/pull/2476) | [#2494](https://github.com/RefinedStone/codex-exec-loop/pull/2494) | `dc926bd79947` | `prerelease` | CI Gate 성공 |
| [#2477](https://github.com/RefinedStone/codex-exec-loop/pull/2477) | [#2495](https://github.com/RefinedStone/codex-exec-loop/pull/2495) | `a766a9cdc32d` | `prerelease` | CI Gate 성공 |
| [#2478](https://github.com/RefinedStone/codex-exec-loop/pull/2478) | [#2496](https://github.com/RefinedStone/codex-exec-loop/pull/2496) | `8e02225c9e65` | `prerelease` | CI Gate 성공 |
| [#2469](https://github.com/RefinedStone/codex-exec-loop/pull/2469) | [#2497](https://github.com/RefinedStone/codex-exec-loop/pull/2497) | `c5703fb2ac37` | `prerelease` | CI Gate 성공 |
| [#2472](https://github.com/RefinedStone/codex-exec-loop/pull/2472) | [#2498](https://github.com/RefinedStone/codex-exec-loop/pull/2498) | `78fe4a554b58` | `prerelease` | CI Gate 성공 |
| [#2371](https://github.com/RefinedStone/codex-exec-loop/pull/2371) | [#2499](https://github.com/RefinedStone/codex-exec-loop/pull/2499) | `3eccf720d89c` | `prerelease` | CI Gate 성공 |
| [#2463](https://github.com/RefinedStone/codex-exec-loop/pull/2463) | [#2500](https://github.com/RefinedStone/codex-exec-loop/pull/2500) | `f29979bca9aa` | `prerelease` | CI Gate 성공 |
| [#2471](https://github.com/RefinedStone/codex-exec-loop/pull/2471) | [#2501](https://github.com/RefinedStone/codex-exec-loop/pull/2501) | `c4774d2a662b` | `prerelease` | CI Gate 성공 |
| [#2474](https://github.com/RefinedStone/codex-exec-loop/pull/2474) | [#2502](https://github.com/RefinedStone/codex-exec-loop/pull/2502) | `026c2e1212c2` | `codex/review-vault-restore-session` | 현재 head CI 진행 중 |
| [#2464](https://github.com/RefinedStone/codex-exec-loop/pull/2464), [#2467](https://github.com/RefinedStone/codex-exec-loop/pull/2467) | [#2503](https://github.com/RefinedStone/codex-exec-loop/pull/2503) | `35a7166e7577` | `codex/review-2464-markdown-validation` | 현재 head CI 진행 중 |
| [#2479](https://github.com/RefinedStone/codex-exec-loop/pull/2479) | [#2504](https://github.com/RefinedStone/codex-exec-loop/pull/2504) | `c99c162d447b` | `prerelease` | CI 진행 중 |

## 적용 순서와 제한

- Markdown: **#2488 → #2503**. 공통 해석기를 먼저 적용한다.
- Vault: **#2486 → #2489 → #2493 → #2502**. 첫 키 생성, 세션 재검사, 복원 세션 경계, 게시 의도별 복원 판 순서다.
- 위 의존 PR들은 차이가 원본 리뷰 범위에 맞게 보이도록 부모 브랜치를 기반으로 한다. 부모 PR 적용 뒤 기반 브랜치를 바꾸고 최종 head를 다시 확인해야 한다.
- 저장소 pull_request 트리거는 main/prerelease를 대상으로 하므로 다른 기반의 PR은 현재 head SHA에 대한 scope=rust 수동 CI로 검증한다.
- #2503: Windows에서 템플릿 경로가 설정되어 있으면 기존 secure_fs의 미지원 결과를 Unreadable로 처리하고 게시를 막는다. 검증 없이 게시를 진행하지 않는다. 필수 복수 선택/체크박스도 검증 지원 전까지 미지원으로 표시한다.
- #2501: 검증기 출력 수집은 macOS 전용 구현이다. Windows gate와 CI의 macOS 컴파일은 별개 증거다. 추가 macOS 실행 회귀 시험은 이 Windows 호스트에서 실행하지 않았고 현재 CI 필터도 해당 시험을 실행하지 않는다.
- #2502: 복수 legacy 게시 기록에 순서 정보가 없으면 자동 복원 링크 확정을 보수적으로 차단한다.

## 반려 판단

반려 항목은 아래 근거에 따라 이번 코드 변경을 하지 않는다. 재검토 조건은 미구현 상태이며, 완료된 수정으로 계산하지 않는다.

| 원본 | 제안 | 반려 근거 | 재검토 조건 |
| --- | --- | --- | --- |
| [#2466](https://github.com/RefinedStone/codex-exec-loop/pull/2466#issuecomment-7029574850) | 보드 동작 권한을 개별 변경 승인 영수증으로 확대 | 현재 IssueWriteApproval은 이슈·동작 권한이고 보드 쓰기의 제품 UI/서비스 호출자는 연결되지 않았다. 리뷰도 현 계약의 확정 결함으로 분류하지 않았다. 이 타입을 개별 선택 승인으로 간주하는 동작을 새로 만들지 않는다. | 보드 편집 UI를 연결하기 전에 issue/project/item/field/option/정책 판과 필요 시 기존 값을 묶은 Kernel 승인 객체, 유효한 다른 조합 교체 시 mutation=0 회귀를 구현한다. |
| [#2466](https://github.com/RefinedStone/codex-exec-loop/pull/2466#issuecomment-7029574850) | 보드 mutation 결과 불명 상태와 자동 재전송 정책 | 현재 연결된 제품 호출자나 자동 재전송 경로가 없다. adapter 오류를 미수정의 증거라고 문서화하지 않는다. 결과 불명 상태·재조회 UI는 보드 쓰기 호출자를 추가할 때 함께 정할 통합 계약이다. | 보드 쓰기 UI/Kernel 연결 시 전송 후 오류를 결과 불명으로 표현하고 새 승인 없는 자동 재시도를 차단한다. |
| [#2465](https://github.com/RefinedStone/codex-exec-loop/pull/2465#issuecomment-7029574322) | 마스킹 함수에 탐지 완료 프로토콜 추가 | mask_issue_text는 명시적으로 전달된 구간의 치환만 담당한다. 현재 게시 UI의 사용자 구간과 AI 인계의 패턴 탐지는 각 호출자가 제공한다. 이 함수의 성공을 임의의 비밀값 탐지 완료 증명으로 재정의하지 않는다. 실제 AI 탐지 누락은 #2469 후속 PR에서 별도 수정한다. | 자동 탐지기의 부분 완료/실패 상태를 연결하는 기능이 생기면 그 경계에 별도 완료 상태와 보류 계약을 추가한다. |
| [#2465](https://github.com/RefinedStone/codex-exec-loop/pull/2465#issuecomment-7029574322) | 모든 secret의 복원 가능 정책 강제 금지 | 현재 정책은 secret의 Remove/RestorableMask/UserConfirm을 명시적으로 구분한다. Remove는 사용자 결정으로 약화할 수 없다. 모든 정책을 강제 Remove로 바꾸는 것은 기존 설정 의미를 바꾸는 제품 정책 변경이므로 이번 버그 수정에 포함하지 않는다. | 제품 규칙을 모든 자격 증명 영구 폐기로 확정하면 정책 파서와 처리기의 불변식을 함께 바꾼다. |
| [#2465](https://github.com/RefinedStone/codex-exec-loop/pull/2465#issuecomment-7029574322) | 추가 입력 크기/구간 수 상한 및 순열 시험 | 추가 한도는 현재 API에 없는 입력 허용 정책이다. 이번 검토에서 좌표 오류가 확인되지 않았고 실제 누락은 별도 회귀로 고쳤다. 근거 없는 수치 제한이나 동일 구현을 반복하는 순열 시험을 추가하지 않는다. | 대표적인 대형 로그와 구간 수의 측정으로 허용 크기·지연 목표를 정하면 경계에서 한도를 검증한다. |
| [#2412](https://github.com/RefinedStone/codex-exec-loop/pull/2412#issuecomment-7029544279) | client 상한·Node/Python 입력 표현 통일 추가 | 현재 완료 계약은 연결 전 잘못된 0/음수/비정수 수를 거절하는 것이다. 별도 제품 상한과 표현식 허용 정책은 정해지지 않았으며 기존 유효 입력을 축소하는 변경이므로 이번 버그 대응에서 추가하지 않는다. | 운영 가능한 동시 client 상한 또는 플랫폼 간 표현 동등성 요구가 정해질 때 별도 계약·시험으로 진행. |
| [#2411](https://github.com/RefinedStone/codex-exec-loop/pull/2411#issuecomment-7029543789) | A→B→A·같은 key 재열기 추가 coverage | 현재 generation/mode/recordKey 검사는 모든 async 결과 적용 전에 존재한다. 리뷰도 추가 차단 결함으로 판단하지 않았고 필수 통합 실패는 이미 고쳐졌으므로 별도 순열 시험 확대는 이번 범위에서 하지 않는다. | generation 검사가 없는 새 응답 경로나 재현 가능한 역순 적용이 추가되면 해당 시나리오를 회귀로 고정. |
| [#2408](https://github.com/RefinedStone/codex-exec-loop/pull/2408#issuecomment-7029542054) | 추가 retry/coalesce/Disabled 순열 시험 | 현재 active_setup correlation을 확인한 완료만 처리하고 실패한 동일 setup_request만 지운다. Disabled 유지와 자동 네트워크 backoff는 다른 계약이다. 새 실패 재현 없이 비차단 순열 coverage를 확대하지 않는다. | 늦은 완료가 새 성공/Disabled를 덮는 재현 또는 자동 재시도 요구가 생길 때. |
| [#2407](https://github.com/RefinedStone/codex-exec-loop/pull/2407#issuecomment-7029541440) | Unicode 정규화·비ASCII case-fold 추가 정책 | query와 field 비교는 모두 명시된 ASCII 대소문자 정책이다. NFC/NFD 통합이나 비ASCII case-fold는 검색 결과·점수를 바꾸는 기능 확장이며 현재 동일 원문 누락 수정에 필요하지 않다. | 전체 name/preview/path/branch 비교와 점수에 공통 Unicode 정책을 적용하기로 결정할 때. |
| [#2406](https://github.com/RefinedStone/codex-exec-loop/pull/2406#issuecomment-7029526987) | 지연 show·관계없는 WebContents 추가 주입 시험 | 현재 E2E는 main이 반환한 profileId와 별도 제목/cookie 완료를 기다린다. #2405 통합 browser E2E도 이번 전체 실행에서 통과했다. 제안은 추가 fault permutation이며 현재 실패가 없어 추가 구현을 하지 않는다. | 실제 목표 profile 전에 탐색되는 재현이 생기면 제어된 show 지연 회귀로 고정. |
| [#2405](https://github.com/RefinedStone/codex-exec-loop/pull/2405#issuecomment-7029526509) | 다중 창 reload·자식 장기 프로세스 종료 정책 확대 | ready/render-process-gone/destroyed는 같은 ownerId 정리 경로를 사용한다. 다른 창 보존·반복 reload 추가 coverage 및 shell 자손 종료 보장은 현재 수정과 다른 확대 계약이며 검증되지 않은 보장으로 쓰지 않는다. | 두 창 소유권 누수 재현 또는 터미널 자손 종료 정책 확정 시 해당 통합 시험 추가. |
| [#2403](https://github.com/RefinedStone/codex-exec-loop/pull/2403#issuecomment-7029525408) | 다중 Git 명령 실패의 자동 복구·부분 실행 상태 확대 | rename discard의 rm/restore를 원자 트랜잭션으로 만들거나 자동 롤백하는 계약은 기존 Git 동작 범위를 바꾼다. #2377의 작성자 판단과 plan 27 §9.3은 자동 복구 미제공 및 git status 확인을 이미 명시한다. 부분 실행 가능성을 없다고 주장하지 않으며 이번에 자동 복구를 추가하지 않는다. | 제어된 두 번째 명령 실패 시나리오와 사용자 변경을 보존하는 복구 소유권/저널 계약이 정해질 때. |
| [#2402](https://github.com/RefinedStone/codex-exec-loop/pull/2402#issuecomment-7029537956) | 정상 SIGINT 마지막 로그·모든 producer 종료 시간 추가 시험 | 이번 수정은 main guard의 drop 실행이다. 실제 장기 blocking 회귀가 확인되지 않은 상태에서 모든 producer의 종료 정책까지 확대하지 않는다. flush와 프로세스 전체 종료 기한을 동일 보장으로 주장하지 않는다. | 정상 종료 마지막 로그 누락이나 deadline 초과 재현이 생기면 해당 producer를 대상으로 시험. |
| [#2400](https://github.com/RefinedStone/codex-exec-loop/pull/2400#issuecomment-7029540097) | guard helper 공통화 및 복제 경로 추가 panic 시험 | 현재 task/admin CRUD/file-sync 모두 catch→원래 owner release→resume_unwind 순서로 정리한다. helper 통합과 추가 경로 시험은 비차단 유지보수 권고이며 새 결함 재현이 없다. abort/강제 종료의 lease 복구까지 이번 unwind 수정에 포함하지 않는다. | 경로 간 의미 차이가 생기거나 owner 해제 누락 재현이 있을 때 공통 계약으로 묶기. |
| [#2399](https://github.com/RefinedStone/codex-exec-loop/pull/2399#issuecomment-7029539557) | 같은 target의 두 연속 delivery 전체 통합 시험 | 현재 orchestrator/distributor는 공통 integration_worktree_head 분류를 사용하고 Git 조회 실패를 정상 detached로 보지 않는다. 리뷰의 전체 admission 시나리오는 유용하지만 분류 버그 재현은 아니므로 이번 코드 후속 범위에서는 추가하지 않는다. 기존 분류 시험만으로 전체 delivery 검증 완료라고 주장하지 않는다. | queue delivery 통합 하네스에서 clean→dirty/attached/조회 실패 전환을 제어할 계약을 정할 때. |
| [#2398](https://github.com/RefinedStone/codex-exec-loop/pull/2398#issuecomment-7029537411) | 정상 close·producer 종료 교차 순열 추가 | close enqueue와 writer 종료는 동일 deadline이고 만료 뒤 abort를 await한다. 제안은 정상 flush·종료 경쟁·별도 producer 통합 coverage 확대이며 기존 문제를 재현하는 blocked queue/drop 시험이 있다. 실제 새 실패 없이 종료 정책을 변경하지 않는다. | producer가 session 종료 뒤 남거나 정상 close 전달이 누락되는 재현이 나올 때. |
| [#2397](https://github.com/RefinedStone/codex-exec-loop/pull/2397#issuecomment-7029536894) | SSE TCP backpressure 전체 연결 종료 기한 확대 | watch/take_until은 SSE body가 poll될 때 종료를 관찰한다. 기존 시험은 실제 꽉 찬 TCP 송신 버퍼까지 보장하지 않는다. 별도 소켓 최종 deadline 추가는 비재현 계약 확대라 이번 패치에 넣지 않으며 모든 비읽기 클라이언트 즉시 종료를 검증했다고 주장하지 않는다. 테스트의 무효한 CRLF replace 제거 제안은 동작에 영향 없는 정리이므로 이번 수정 범위에서 제외한다. | 송신 버퍼 포화와 shutdown을 제어한 재현에서 정해진 최종 종료 기한을 넘으면 서버/연결 deadline 검토. |
| [#2396](https://github.com/RefinedStone/codex-exec-loop/pull/2396#issuecomment-7029536364) | stale clipboard staged 파일 정리 및 submit 경계 확대 | 현재 stale 결과는 generation/transcript/Ready 검증에서 버리지만 staged 파일 소유권 토큰을 전달하지 않아 파일 정리는 미구현 상태다. 경로만으로 삭제하는 방식은 사용자 원본을 지울 수 있다. staging 생성·참조·폐기 소유권과 동일 스레드 submit 뒤 probe 허용 정책을 함께 정하는 별도 계약으로 두고 이번 변경에 추가하지 않는다. | owned staging handle과 공유 첨부 수명 규약을 도입한 뒤 stale/completion/drop 모두에서 자신이 만든 파일만 정리하는 회귀로 구현. |
| [#2394](https://github.com/RefinedStone/codex-exec-loop/pull/2394#issuecomment-7029535155) | Ready 전 첨부 제거·실패 후 재제출 추가 순열 | 현재 텍스트와 이미지가 모두 비어야 예약을 취소하고 준비 전에 예약을 소비하므로 반복 Ready 중복 제출은 막는다. 제안된 상태 순열은 비차단 추가 시험이며 현재 재현되지 않은 동작 변경을 넣지 않는다. | 입력 보존/중복 submit에 대한 실제 반례가 나오면 fake port의 제출 횟수로 검증. |
| [#2393](https://github.com/RefinedStone/codex-exec-loop/pull/2393#issuecomment-7029530271) | 추가 child argv 표현·비UTF-8 순열 시험 | 첫 --부터 OsString을 그대로 넘기고 shell 재분해를 하지 않는 현재 계약이 보존된다. 빈 실행 파일 오류·빈 문자열·두 번째 delimiter·Unix 비UTF-8 추가 coverage는 별도 확장으로 남긴다. | 문자열 변환 계층이 추가되거나 실제 인자 손실 사례가 발생할 때. |
| [#2392](https://github.com/RefinedStone/codex-exec-loop/pull/2392#issuecomment-7029529736) | rename 후 fsync 실패 의미·프로세스 잠금 대기 정책 | 현재 rename 이후 directory sync 실패는 새 문서 설치 가능성을 가진 Err다. 단계별 fault injection이나 불확실한 결과 상태를 추가하는 것은 별도 장애 계약이며 실제 재현이 없는 권고로 분류한다. 저장되지 않았다고 단정하지 말고 재조회해야 하며 이번 코드에서 원자 롤백을 추가하지 않는다. write_document의 fresh-home 호출자 주석 정리도 비차단 문구 개선으로 이번 수정 범위에서 제외한다. | fsync 단계별 실패 재현과 catalog invalidation/재조회 결과 표현을 함께 설계할 때. |
| [#2391](https://github.com/RefinedStone/codex-exec-loop/pull/2391#issuecomment-7029529139) | resident mutex를 포함한 사용자 요청 전체 deadline | 현재 한도는 세션 RPC 하나이며 resident mutex 대기나 여러 RPC의 총시간이 아니다. 전체 사용자 요청 120초 보장과 admission 상한은 현재 계약에 없으므로 이번 수정에서 확대하지 않는다. | UI/서비스가 전체 요청 기한을 약속하게 될 때 admission deadline·동시 요청 상한과 함께 구현. |
| [#2388](https://github.com/RefinedStone/codex-exec-loop/pull/2388#issuecomment-7029527441) | FIFO writer 있음·descriptor 누수 추가 조합 시험 | O_NONBLOCK 뒤 동일 fd 정규파일/identity 검증을 유지하므로 writer 없는 FIFO의 선행 open 정체 원인은 해결된다. 추가 객체 조합과 실패 진단은 비차단 coverage이며 현재 새 반례 없이 범위를 넓히지 않는다. | read API의 다른 객체 처리 변경 또는 fd 누수 재현이 생기면 추가. |
| [#2387](https://github.com/RefinedStone/codex-exec-loop/pull/2387#issuecomment-7029539034) | 구형 baseline 재-export의 편집본 보존 UX | workspace_root+relative_path 키로 worktree가 분리되며 구형 baseline은 자동 채택하지 않는다. 재-export는 candidate를 덮어쓸 수 있으므로 편집본 보관이 필요하다. dirty candidate 보호와 자동 migration은 별도 전환/복구 정책으로 남기고 이번에 자동 덮어쓰기나 DB 키 회귀 수정에 섞지 않는다. | 재-export 전 백업/복사 안내와 dirty candidate 처리 UX를 별도 완료 기준으로 정할 때. 현재 사용자 편집본은 먼저 보관해야 한다. |
| [#2386](https://github.com/RefinedStone/codex-exec-loop/pull/2386#issuecomment-7029538449) | 문서 부재·실제 queue 내용 추가 회귀 | 현재 개별 authority snapshot은 한 읽기 트랜잭션이며 호출자 혼합 문제도 별도 수정되었다. 추가 데이터 조합 coverage는 비차단 권고라 이번에 확대하지 않는다. | snapshot schema/port가 바뀌거나 본문·revision 불일치 반례가 발견되면 추가. |
| [#2385](https://github.com/RefinedStone/codex-exec-loop/pull/2385#issuecomment-7029524796) | ..notes와 symlink/junction의 추가 조합 시험 | lexical 판정은 실제 .. 세그먼트·절대 경로만 거절하며 실제 대상 검증은 gitStatus의 descriptor/realpath 경계다. #2404의 FIFO 보완까지 이미 반영되었다. 새 경로 우회 재현이 없는 추가 조합은 이번 수정에서 확대하지 않는다. | Windows 드라이브/UNC 또는 junction 우회 사례가 확인되면 해당 OS에서 회귀 추가. |
| [#2384](https://github.com/RefinedStone/codex-exec-loop/pull/2384#issuecomment-7029524310) | 다중 프로세스 잠금 및 browser 전체 설정 대체 정책 변경 | pendingSaves는 고정 userData 경로를 사용하는 단일 main의 요청을 직렬화하며 마지막 제출 전체 설정 우선 정책이 명시되어 있다. 다중 인스턴스/별칭 경로 잠금과 필드별 병합은 별도 제품 계약이므로 현재 저장 경쟁 수정에 추가하지 않는다. | 다중 앱 인스턴스 또는 창별 browser 설정 병합 지원이 결정될 때. |
| [#2382](https://github.com/RefinedStone/codex-exec-loop/pull/2382#issuecomment-7029523827) | 반복 close·후속 onState·정상 backoff 추가 coverage | close handler는 closed를 daemon 시작과 scheduleReconnect 전에 확인한다. 실제 취소된 handshake 재현을 막는 기존 시험이 있다. 나머지 순열은 비차단 추가 시험이며 새 동작을 넣지 않는다. | 취소 뒤 callback/reconnect가 남는 실제 재현이 생길 때. |
| [#2381](https://github.com/RefinedStone/codex-exec-loop/pull/2381#issuecomment-7029523272) | unborn index 비교 UI 표기 및 첫 commit 전환 추가 | HEAD 없음에서는 --cached로 index 대 빈 트리를 읽고 HEAD 있는 경로는 그대로다. staged 내용만 보여주는 동작은 기존 완료 계약에 맞는다. 비교 대상 UI 확대와 commit 직후 추가 시나리오는 비차단 권고로 이번 범위에서 제외한다. | 사용자에게 worktree 전체 diff로 잘못 표기되는 구체 경로나 HEAD 전환 오류가 확인되면 진행. |
| [#2379](https://github.com/RefinedStone/codex-exec-loop/pull/2379#issuecomment-7029522142) | 여러 unread·재연결 오프라인 완료 복원 추가 범위 | 현재 active→inactive 실제 전환에서만 unread를 추가하고 명시적 열기에서 읽음 처리한다. snapshot 재연결로 오프라인 완료까지 복원하는 기능은 현재 계약에 없으며 비차단 조합 시험을 확대하지 않는다. | 오프라인 완료 복원 요구가 정해지거나 현재 읽음 항목 외 항목을 지우는 재현이 생기면 진행. |

## 전체 대상 목록

| 원본 PR | 머지 시각(KST) | 판단 | 후속 PR |
| --- | --- | --- | --- |
| [#2482](https://github.com/RefinedStone/codex-exec-loop/pull/2482) fix(runtime): release kernel locks and retain completion observations | 2026-11-20 16:00:14 | 조치 대상 지적 없음 | — |
| [#2480](https://github.com/RefinedStone/codex-exec-loop/pull/2480) WIP | 2026-11-20 14:34:30 | 수용·후속 PR 제출 | [#2486](https://github.com/RefinedStone/codex-exec-loop/pull/2486) |
| [#2479](https://github.com/RefinedStone/codex-exec-loop/pull/2479) WIP | 2026-11-20 14:31:32 | 수용·후속 PR 제출 | [#2504](https://github.com/RefinedStone/codex-exec-loop/pull/2504) |
| [#2478](https://github.com/RefinedStone/codex-exec-loop/pull/2478) WIP | 2026-11-20 14:17:49 | 수용·후속 PR 제출 | [#2496](https://github.com/RefinedStone/codex-exec-loop/pull/2496) |
| [#2477](https://github.com/RefinedStone/codex-exec-loop/pull/2477) WIP | 2026-11-20 14:26:26 | 수용·후속 PR 제출 | [#2495](https://github.com/RefinedStone/codex-exec-loop/pull/2495) |
| [#2476](https://github.com/RefinedStone/codex-exec-loop/pull/2476) WIP | 2026-11-20 14:10:18 | 수용·후속 PR 제출 | [#2494](https://github.com/RefinedStone/codex-exec-loop/pull/2494) |
| [#2475](https://github.com/RefinedStone/codex-exec-loop/pull/2475) WIP | 2026-11-20 14:04:03 | 수용·후속 PR 제출 | [#2493](https://github.com/RefinedStone/codex-exec-loop/pull/2493) |
| [#2474](https://github.com/RefinedStone/codex-exec-loop/pull/2474) WIP | 2026-11-20 13:50:46 | 수용·후속 PR 제출 | [#2502](https://github.com/RefinedStone/codex-exec-loop/pull/2502) |
| [#2473](https://github.com/RefinedStone/codex-exec-loop/pull/2473) WIP | 2026-11-20 13:31:59 | 수용·후속 PR 제출 | [#2484](https://github.com/RefinedStone/codex-exec-loop/pull/2484) |
| [#2472](https://github.com/RefinedStone/codex-exec-loop/pull/2472) WIP | 2026-11-20 13:31:18 | 수용·후속 PR 제출 | [#2498](https://github.com/RefinedStone/codex-exec-loop/pull/2498) |
| [#2471](https://github.com/RefinedStone/codex-exec-loop/pull/2471) WIP | 2026-11-20 13:31:28 | 수용·후속 PR 제출 | [#2501](https://github.com/RefinedStone/codex-exec-loop/pull/2501) |
| [#2470](https://github.com/RefinedStone/codex-exec-loop/pull/2470) WIP | 2026-11-20 13:26:05 | 조치 대상 지적 없음 | — |
| [#2469](https://github.com/RefinedStone/codex-exec-loop/pull/2469) WIP | 2026-11-20 13:14:34 | 수용·후속 PR 제출 | [#2497](https://github.com/RefinedStone/codex-exec-loop/pull/2497) |
| [#2468](https://github.com/RefinedStone/codex-exec-loop/pull/2468) WIP | 2026-11-20 12:10:03 | 수용·후속 PR 제출 | [#2486](https://github.com/RefinedStone/codex-exec-loop/pull/2486), [#2489](https://github.com/RefinedStone/codex-exec-loop/pull/2489) |
| [#2467](https://github.com/RefinedStone/codex-exec-loop/pull/2467) WIP | 2026-11-20 13:14:03 | 수용·후속 PR 제출 | [#2503](https://github.com/RefinedStone/codex-exec-loop/pull/2503) |
| [#2466](https://github.com/RefinedStone/codex-exec-loop/pull/2466) WIP | 2026-11-20 12:25:04 | 반려·판단 문서 | — |
| [#2465](https://github.com/RefinedStone/codex-exec-loop/pull/2465) WIP | 2026-11-20 12:46:25 | 반려·판단 문서 | — |
| [#2464](https://github.com/RefinedStone/codex-exec-loop/pull/2464) WIP | 2026-11-20 12:33:12 | 수용·후속 PR 제출 | [#2488](https://github.com/RefinedStone/codex-exec-loop/pull/2488), [#2503](https://github.com/RefinedStone/codex-exec-loop/pull/2503) |
| [#2463](https://github.com/RefinedStone/codex-exec-loop/pull/2463) WIP | 2026-11-20 11:57:55 | 수용·후속 PR 제출 | [#2500](https://github.com/RefinedStone/codex-exec-loop/pull/2500) |
| [#2462](https://github.com/RefinedStone/codex-exec-loop/pull/2462) WIP | 2026-11-20 11:45:50 | 조치 대상 지적 없음 | — |
| [#2461](https://github.com/RefinedStone/codex-exec-loop/pull/2461) WIP | 2026-11-20 11:33:58 | 조치 대상 지적 없음 | — |
| [#2460](https://github.com/RefinedStone/codex-exec-loop/pull/2460) WIP | 2026-11-20 11:33:52 | 수용·후속 PR 제출 | [#2483](https://github.com/RefinedStone/codex-exec-loop/pull/2483) |
| [#2459](https://github.com/RefinedStone/codex-exec-loop/pull/2459) build: 개발 캐시를 유지하며 Cargo 산출물 용량 축소 | 2026-11-20 11:54:07 | 조치 대상 지적 없음 | — |
| [#2458](https://github.com/RefinedStone/codex-exec-loop/pull/2458) WIP | 2026-11-20 11:18:31 | 수용·후속 PR 제출 | [#2487](https://github.com/RefinedStone/codex-exec-loop/pull/2487) |
| [#2456](https://github.com/RefinedStone/codex-exec-loop/pull/2456) WIP | 2026-11-20 11:01:39 | 조치 대상 지적 없음 | — |
| [#2455](https://github.com/RefinedStone/codex-exec-loop/pull/2455) refactor(tui): remove simple review view pass-through layers (#2321) | 2026-11-20 08:09:03 | 조치 대상 지적 없음 | — |
| [#2454](https://github.com/RefinedStone/codex-exec-loop/pull/2454) fix(desktop): apply thread lifecycle notifications to the sidebar (#2320) | 2026-11-20 08:26:44 | 기대응 | — |
| [#2453](https://github.com/RefinedStone/codex-exec-loop/pull/2453) fix(admin): refresh boss-seat detail readiness from the dashboard snapshot (#2311) | 2026-11-20 08:03:27 | 조치 대상 지적 없음 | — |
| [#2452](https://github.com/RefinedStone/codex-exec-loop/pull/2452) fix(admin): give the STANDBY selection a real detail (#2312) | 2026-11-20 08:32:38 | 조치 대상 지적 없음 | — |
| [#2451](https://github.com/RefinedStone/codex-exec-loop/pull/2451) fix(release): package the binary from the configured cargo target dir (#2317) | 2026-11-20 08:16:32 | 기대응 | — |
| [#2450](https://github.com/RefinedStone/codex-exec-loop/pull/2450) fix(cleanup): guard remote delete and prune vanished worktrees | 2026-11-20 08:16:27 | 조치 대상 지적 없음 | — |
| [#2449](https://github.com/RefinedStone/codex-exec-loop/pull/2449) fix(capture): drop inherited GIT_* from fixture git (#2318) | 2026-11-20 08:03:22 | 조치 대상 지적 없음 | — |
| [#2448](https://github.com/RefinedStone/codex-exec-loop/pull/2448) fix(validation): require a Post-Merge Gate for every rollout sample (#2319) | 2026-11-20 08:03:18 | 조치 대상 지적 없음 | — |
| [#2447](https://github.com/RefinedStone/codex-exec-loop/pull/2447) test(desktop): run every e2e spec from the default command (#2313) | 2026-11-20 08:16:22 | 조치 대상 지적 없음 | — |
| [#2446](https://github.com/RefinedStone/codex-exec-loop/pull/2446) fix(admin): show focused scene fallback buttons after Pixi is ready (#2310) | 2026-11-20 08:16:18 | 조치 대상 지적 없음 | — |
| [#2445](https://github.com/RefinedStone/codex-exec-loop/pull/2445) test(parallel-mode): wait for worker launch by signal, not polling (#2295) | 2026-11-20 08:03:13 | 조치 대상 지적 없음 | — |
| [#2444](https://github.com/RefinedStone/codex-exec-loop/pull/2444) fix(admin): hold signal packets still under reduced motion (#2308) | 2026-11-20 08:03:08 | 조치 대상 지적 없음 | — |
| [#2443](https://github.com/RefinedStone/codex-exec-loop/pull/2443) fix(text): keep short detail markers within max_len (#2302) | 2026-11-20 08:03:03 | 조치 대상 지적 없음 | — |
| [#2442](https://github.com/RefinedStone/codex-exec-loop/pull/2442) fix(kernel): refuse a thread prompt for a thread of another repository (#2303) | 2026-11-20 08:42:19 | 기대응 | — |
| [#2441](https://github.com/RefinedStone/codex-exec-loop/pull/2441) fix(admin): refetch the snapshot after an SSE invalidation lands mid-fetch (#2307) | 2026-11-20 08:26:39 | 조치 대상 지적 없음 | — |
| [#2440](https://github.com/RefinedStone/codex-exec-loop/pull/2440) fix(pr-validation): fence poll worker changes on lease ownership (#2288) | 2026-11-20 08:02:58 | 조치 대상 지적 없음 | — |
| [#2439](https://github.com/RefinedStone/codex-exec-loop/pull/2439) fix(kernel): bound the host inbox slice before each core cycle (#2298) | 2026-11-20 08:02:54 | 조치 대상 지적 없음 | — |
| [#2438](https://github.com/RefinedStone/codex-exec-loop/pull/2438) fix(kernel): keep a full stop for background turns that start later (#2304) | 2026-11-20 09:01:12 | 조치 대상 지적 없음 | — |
| [#2437](https://github.com/RefinedStone/codex-exec-loop/pull/2437) fix(kernel): bound the recorder backlog and expose its health (#2300, #2301) | 2026-11-20 08:46:52 | 조치 대상 지적 없음 | — |
| [#2413](https://github.com/RefinedStone/codex-exec-loop/pull/2413) fix(ci): compile Rust-embedded Admin Askama templates in PR gate (#2315) | 2026-11-20 15:21:15 | 수용·후속 PR 제출 | [#2492](https://github.com/RefinedStone/codex-exec-loop/pull/2492) |
| [#2412](https://github.com/RefinedStone/codex-exec-loop/pull/2412) fix(validation): reject invalid proxy client counts before any connections (#2309) | 2026-11-20 15:37:13 | 반려·판단 문서 | — |
| [#2411](https://github.com/RefinedStone/codex-exec-loop/pull/2411) fix(admin): reject stale async detail responses across drawer transitions (#2305) | 2026-11-20 15:37:05 | 반려·판단 문서 | — |
| [#2410](https://github.com/RefinedStone/codex-exec-loop/pull/2410) fix(admin): preserve evidence history drawer across refreshes (#2306) | 2026-11-20 15:36:57 | 기대응 | — |
| [#2409](https://github.com/RefinedStone/codex-exec-loop/pull/2409) test(observer): use controlled read barriers instead of polling snapshots (#2299) | 2026-11-20 15:40:16 | 기대응 | — |
| [#2408](https://github.com/RefinedStone/codex-exec-loop/pull/2408) fix(polling): retry identical GitHub review setup after failure (#2297) | 2026-11-20 15:21:11 | 반려·판단 문서 | — |
| [#2407](https://github.com/RefinedStone/codex-exec-loop/pull/2407) fix(search): preserve non-ASCII characters in session search tokens (#2296) | 2026-11-20 15:21:06 | 반려·판단 문서 | — |
| [#2406](https://github.com/RefinedStone/codex-exec-loop/pull/2406) test(desktop): wait for applied browser profile before navigating (#2294) | 2026-11-20 15:21:00 | 반려·판단 문서 | — |
| [#2405](https://github.com/RefinedStone/codex-exec-loop/pull/2405) fix(desktop): release native PTYs and browser views on renderer reload (#2293) | 2026-11-20 15:20:55 | 반려·판단 문서 | — |
| [#2404](https://github.com/RefinedStone/codex-exec-loop/pull/2404) fix(desktop): block external symlink targets in untracked diff (#2292) | 2026-11-20 15:58:06 | 기대응 | — |
| [#2403](https://github.com/RefinedStone/codex-exec-loop/pull/2403) fix(desktop): revalidate discard plans after confirmation (#2291) | 2026-11-20 15:35:49 | 반려·판단 문서 | — |
| [#2402](https://github.com/RefinedStone/codex-exec-loop/pull/2402) fix(diagnostics): flush standalone binary trace logs before exit (#2290) | 2026-11-20 15:47:36 | 반려·판단 문서 | — |
| [#2401](https://github.com/RefinedStone/codex-exec-loop/pull/2401) fix(planning): preserve newer accepted authority during post-turn reconciliation (#2287) | 2026-11-20 15:54:08 | 기대응 | — |
| [#2400](https://github.com/RefinedStone/codex-exec-loop/pull/2400) fix(planning): release task/admin/file-sync mutation guards on panic (#2289) | 2026-11-20 15:20:49 | 반려·판단 문서 | — |
| [#2399](https://github.com/RefinedStone/codex-exec-loop/pull/2399) fix(parallel): recognize detached integration HEAD as ready (#2286) | 2026-11-20 15:20:43 | 반려·판단 문서 | — |
| [#2398](https://github.com/RefinedStone/codex-exec-loop/pull/2398) fix(gateway): bound close enqueue and abort stalled client writer (#2282) | 2026-11-20 15:20:38 | 반려·판단 문서 | — |
| [#2397](https://github.com/RefinedStone/codex-exec-loop/pull/2397) fix(admin): terminate live SSE streams during graceful shutdown (#2283) | 2026-11-20 15:20:32 | 반려·판단 문서 | — |
| [#2396](https://github.com/RefinedStone/codex-exec-loop/pull/2396) fix(tui): discard stale clipboard probes when composer identity changes (#2280) | 2026-11-20 15:20:27 | 반려·판단 문서 | — |
| [#2395](https://github.com/RefinedStone/codex-exec-loop/pull/2395) fix(gateway): probe stale upstream with absolute trusted ps and deadline (#2279) | 2026-11-20 15:52:10 | 기대응 | — |
| [#2394](https://github.com/RefinedStone/codex-exec-loop/pull/2394) fix(tui): replay queued image-only startup submissions once (#2284) | 2026-11-20 15:20:22 | 반려·판단 문서 | — |
| [#2393](https://github.com/RefinedStone/codex-exec-loop/pull/2393) fix(cli): preserve MCP child flags after -- (#2281) | 2026-11-20 15:20:17 | 반려·판단 문서 | — |
| [#2392](https://github.com/RefinedStone/codex-exec-loop/pull/2392) fix(mcp): serialize and atomically persist hub registry updates (#2278) | 2026-11-20 15:20:11 | 반려·판단 문서 | — |
| [#2391](https://github.com/RefinedStone/codex-exec-loop/pull/2391) fix(mcp): apply one deadline to hub writes and response loops (#2277) | 2026-11-20 16:15:20 | 반려·판단 문서 | — |
| [#2390](https://github.com/RefinedStone/codex-exec-loop/pull/2390) fix(mcp): bound hub app-server output lines and pending queue (#2276) | 2026-11-20 16:38:31 | 기대응 | — |
| [#2389](https://github.com/RefinedStone/codex-exec-loop/pull/2389) fix(mcp): scrub inherited credentials for hub app-server processes (#2275) | 2026-11-20 15:54:16 | 기대응 | — |
| [#2388](https://github.com/RefinedStone/codex-exec-loop/pull/2388) fix(filesystem): reject writerless FIFOs without hanging on open (#2274) | 2026-11-20 15:20:06 | 반려·판단 문서 | — |
| [#2387](https://github.com/RefinedStone/codex-exec-loop/pull/2387) fix(planning): isolate export baselines for linked worktrees (#2273) | 2026-11-20 15:20:00 | 반려·판단 문서 | — |
| [#2386](https://github.com/RefinedStone/codex-exec-loop/pull/2386) fix(planning): keep task/direction authority bodies and revisions atomic (#2272) | 2026-11-20 15:19:52 | 반려·판단 문서 | — |
| [#2385](https://github.com/RefinedStone/codex-exec-loop/pull/2385) fix(desktop): allow legitimate ..-prefixed filenames without traversal (#2271) | 2026-11-20 15:19:47 | 반려·판단 문서 | — |
| [#2384](https://github.com/RefinedStone/codex-exec-loop/pull/2384) fix(desktop): serialize concurrent browser settings writes (#2270) | 2026-11-20 15:19:41 | 반려·판단 문서 | — |
| [#2382](https://github.com/RefinedStone/codex-exec-loop/pull/2382) fix(desktop): stop cancelled handshakes from starting gateway daemons (#2269) | 2026-11-20 15:19:36 | 반려·판단 문서 | — |
| [#2381](https://github.com/RefinedStone/codex-exec-loop/pull/2381) fix(desktop): support staged diff before initial commit (#2268) | 2026-11-20 15:19:31 | 반려·판단 문서 | — |
| [#2380](https://github.com/RefinedStone/codex-exec-loop/pull/2380) fix(desktop): discard stale A → B → A resume results (#2267) | 2026-11-20 15:44:03 | 기대응 | — |
| [#2379](https://github.com/RefinedStone/codex-exec-loop/pull/2379) fix(desktop): preserve unread status on duplicate inactive events (#2266) | 2026-11-20 15:18:40 | 반려·판단 문서 | — |
| [#2377](https://github.com/RefinedStone/codex-exec-loop/pull/2377) feat(git): add the operation engine of plan 27 G3 | 2026-11-19 06:47:49 | 기대응 | — |
| [#2376](https://github.com/RefinedStone/codex-exec-loop/pull/2376) fix(desktop): show an interim dot message while the dot still works | 2026-11-20 15:40:30 | 기대응 | — |
| [#2375](https://github.com/RefinedStone/codex-exec-loop/pull/2375) fix(desktop): show the query and the page of a web search step | 2026-11-19 04:35:35 | 조치 대상 지적 없음 | — |
| [#2374](https://github.com/RefinedStone/codex-exec-loop/pull/2374) feat(git): add commit detail, refs watching, find, and badges of plan 27 G2 | 2026-11-18 20:22:43 | 기대응 | — |
| [#2373](https://github.com/RefinedStone/codex-exec-loop/pull/2373) WIP | 2026-11-18 18:02:45 | 수용·후속 PR 제출 | [#2491](https://github.com/RefinedStone/codex-exec-loop/pull/2491) |
| [#2372](https://github.com/RefinedStone/codex-exec-loop/pull/2372) WIP | 2026-11-18 17:05:15 | 조치 대상 지적 없음 | — |
| [#2371](https://github.com/RefinedStone/codex-exec-loop/pull/2371) WIP | 2026-11-18 16:08:53 | 수용·후속 PR 제출 | [#2499](https://github.com/RefinedStone/codex-exec-loop/pull/2499) |
| [#2370](https://github.com/RefinedStone/codex-exec-loop/pull/2370) WIP | 2026-11-18 16:03:22 | 수용·후속 PR 제출 | [#2490](https://github.com/RefinedStone/codex-exec-loop/pull/2490) |
| [#2369](https://github.com/RefinedStone/codex-exec-loop/pull/2369) docs(plan): add plan 26, Akra Desktop usability gaps and order | 2026-11-18 15:10:52 | 조치 대상 지적 없음 | — |
| [#2368](https://github.com/RefinedStone/codex-exec-loop/pull/2368) WIP | 2026-11-18 14:54:48 | 조치 대상 지적 없음 | — |
| [#2367](https://github.com/RefinedStone/codex-exec-loop/pull/2367) WIP | 2026-11-18 13:40:30 | 조치 대상 지적 없음 | — |
| [#2366](https://github.com/RefinedStone/codex-exec-loop/pull/2366) WIP | 2026-11-18 12:16:25 | 수용·후속 PR 제출 | [#2485](https://github.com/RefinedStone/codex-exec-loop/pull/2485) |
| [#2365](https://github.com/RefinedStone/codex-exec-loop/pull/2365) WIP | 2026-11-18 12:27:11 | 조치 대상 지적 없음 | — |
| [#2364](https://github.com/RefinedStone/codex-exec-loop/pull/2364) WIP | 2026-11-18 12:27:05 | 조치 대상 지적 없음 | — |
| [#2363](https://github.com/RefinedStone/codex-exec-loop/pull/2363) feat(desktop): add read-only Linear work panel | 2026-11-19 05:47:35 | 기대응 | — |
| [#2362](https://github.com/RefinedStone/codex-exec-loop/pull/2362) feat(desktop): add thread-owned browser automation | 2026-11-19 05:23:15 | 기대응 | — |
| [#2361](https://github.com/RefinedStone/codex-exec-loop/pull/2361) feat(mcp): cache search tool catalogs across CLI sessions | 2026-11-18 08:25:48 | 기대응 | — |
| [#2360](https://github.com/RefinedStone/codex-exec-loop/pull/2360) fix(desktop): make three implicit UI states explicit | 2026-11-20 15:42:37 | 기대응 | — |
| [#2359](https://github.com/RefinedStone/codex-exec-loop/pull/2359) feat(context): detect Notion page changes through the MCP hub | 2026-11-18 07:36:30 | 기대응 | — |
| [#2358](https://github.com/RefinedStone/codex-exec-loop/pull/2358) fix(kernel): hold the answer of a command refused for a stale revision | 2026-11-18 07:23:24 | 조치 대상 지적 없음 | — |
| [#2357](https://github.com/RefinedStone/codex-exec-loop/pull/2357) feat(context): detect Jira document changes with Desktop login | 2026-11-19 05:00:55 | 조치 대상 지적 없음 | — |
| [#2356](https://github.com/RefinedStone/codex-exec-loop/pull/2356) refactor(kernel): reduce every thread rule at its session, and remove the swap | 2026-11-18 06:43:55 | 조치 대상 지적 없음 | — |
| [#2355](https://github.com/RefinedStone/codex-exec-loop/pull/2355) fix(kernel): hold the answer of a stored command without stopping the kernel | 2026-11-18 06:15:55 | 기대응 | — |
| [#2354](https://github.com/RefinedStone/codex-exec-loop/pull/2354) fix(kernel): address the code review of the stored commands | 2026-11-18 00:17:35 | 조치 대상 지적 없음 | — |
| [#2353](https://github.com/RefinedStone/codex-exec-loop/pull/2353) refactor(kernel): reduce six thread rules at the session slot, not through a swap (Q5-2a) | 2026-11-17 23:24:55 | 조치 대상 지적 없음 | — |
| [#2352](https://github.com/RefinedStone/codex-exec-loop/pull/2352) feat(kernel): make the number of concurrent conversation turns a policy value (Q5-1) | 2026-11-17 23:02:39 | 조치 대상 지적 없음 | — |
| [#2351](https://github.com/RefinedStone/codex-exec-loop/pull/2351) docs(plan): propose the K6 and Q5 slices (one entrance, turn concurrency policy) | 2026-11-17 22:44:58 | 조치 대상 지적 없음 | — |
| [#2331](https://github.com/RefinedStone/codex-exec-loop/pull/2331) feat(kernel): run stored task commands through the kernel (K4-9d-1) | 2026-11-17 20:37:02 | 기대응 | — |

## 원본 리뷰별 근거

### PR #2480

#### Gateway 동시 publish에서 처음 생성하는 OS 키가 경쟁

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638835801](https://github.com/RefinedStone/codex-exec-loop/pull/2480#discussion_r4638835801).
- 근거: 프로세스 공통 mutex로 최초 키 생성·저장을 직렬화한다. #2468 첫 지적과 공통 수정한다.
- 코드: [src/application/service/evidence_vault_unlock.rs](https://github.com/RefinedStone/codex-exec-loop/blob/ad9a9a3858d814493946af4ad10828a68c4ca532/src/application/service/evidence_vault_unlock.rs).
- 후속 PR: [#2486](https://github.com/RefinedStone/codex-exec-loop/pull/2486).
- 검증: concurrent_sealing_requests_share_one_initial_key.

### PR #2479

#### ADF 글 노드 분할에 따른 의미/해시 차이

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638810315](https://github.com/RefinedStone/codex-exec-loop/pull/2479#discussion_r4638810315).
- 근거: 같은 canonical marks의 연속 text 노드를 합친 뒤 escape해 공급자의 노드 분할에 따라 결과가 달라지지 않게 한다.
- 코드: [src/adapter/outbound/jira/adf.rs](https://github.com/RefinedStone/codex-exec-loop/blob/c99c162d447b7bb829a08875d8ed2ef37e0aa5cd/src/adapter/outbound/jira/adf.rs).
- 후속 PR: [#2504](https://github.com/RefinedStone/codex-exec-loop/pull/2504).
- 검증: Jira 모듈 전체 회귀 시험 통과; Windows Rust gate 통과; git diff --check 통과.

#### literal backtick을 포함한 코드의 무음 손실

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638810318](https://github.com/RefinedStone/codex-exec-loop/pull/2479#discussion_r4638810318).
- 근거: 현재 변환기가 보존하지 못하는 inline/codeBlock delimiter는 unsupported로 표시해 PUT 전에 거절한다.
- 코드: [src/adapter/outbound/jira/adf.rs](https://github.com/RefinedStone/codex-exec-loop/blob/c99c162d447b7bb829a08875d8ed2ef37e0aa5cd/src/adapter/outbound/jira/adf.rs), [src/adapter/outbound/jira/issue_update.rs](https://github.com/RefinedStone/codex-exec-loop/blob/c99c162d447b7bb829a08875d8ed2ef37e0aa5cd/src/adapter/outbound/jira/issue_update.rs).
- 후속 PR: [#2504](https://github.com/RefinedStone/codex-exec-loop/pull/2504).
- 검증: Jira 모듈 전체 회귀 시험 통과; Windows Rust gate 통과; git diff --check 통과.

### PR #2478

#### 키를 잃고 파일만 남은 기존 Vault를 정상 백업으로 복구할 수 없음

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638717913](https://github.com/RefinedStone/codex-exec-loop/pull/2478#discussion_r4638717913).
- 근거: 읽을 수 있는 연결은 보존하고 읽지 못하는 연결은 기본 모드에서 거절한다. 명시적 RecoverUnreadable 승인으로 필드를 복원하고 CurrentVersion을 마지막에 설치한다.
- 코드: [src/application/port/outbound/evidence_vault_backup_port.rs](https://github.com/RefinedStone/codex-exec-loop/blob/8e02225c9e655a1f5d8efb7d57ae765e265cd3a6/src/application/port/outbound/evidence_vault_backup_port.rs), [src/application/service/evidence_vault_backup.rs](https://github.com/RefinedStone/codex-exec-loop/blob/8e02225c9e655a1f5d8efb7d57ae765e265cd3a6/src/application/service/evidence_vault_backup.rs), [src/adapter/outbound/filesystem/evidence_vault_backup.rs](https://github.com/RefinedStone/codex-exec-loop/blob/8e02225c9e655a1f5d8efb7d57ae765e265cd3a6/src/adapter/outbound/filesystem/evidence_vault_backup.rs).
- 후속 PR: [#2496](https://github.com/RefinedStone/codex-exec-loop/pull/2496).
- 검증: lost_device_key_requires_explicit_recovery_and_then_restores_the_backup.

#### 선택한 필드 판과 실제 백업한 CurrentVersion이 달라질 수 있음

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638717920](https://github.com/RefinedStone/codex-exec-loop/pull/2478#discussion_r4638717920).
- 근거: 백업할 CurrentVersion이 선택한 두 필드 범위와 같은 판인지 검증해 변경 시 BindingChanged를 반환한다. 그 한 번 읽은 연결 레코드를 그대로 내보낸다.
- 코드: [src/adapter/outbound/filesystem/evidence_vault_backup.rs](https://github.com/RefinedStone/codex-exec-loop/blob/8e02225c9e655a1f5d8efb7d57ae765e265cd3a6/src/adapter/outbound/filesystem/evidence_vault_backup.rs).
- 후속 PR: [#2496](https://github.com/RefinedStone/codex-exec-loop/pull/2496).
- 검증: export_rejects_a_binding_changed_after_its_field_scopes_were_selected.

### PR #2477

#### 복원/일반 창 API 타입을 shared/bridge.ts로 모으고 복원 preload도 window.akra.vaultRestore 사용; 일반 send/files/browser 권한 미노출

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638696112](https://github.com/RefinedStone/codex-exec-loop/pull/2477#discussion_r4638696112).
- 근거: 복원/일반 창 API 타입을 shared/bridge.ts로 모으고 복원 preload도 window.akra.vaultRestore 사용; 일반 send/files/browser 권한 미노출
- 후속 PR: [#2495](https://github.com/RefinedStone/codex-exec-loop/pull/2495).
- 검증: 전용/일반 창 권한 분리·자동화 열람 보류·프로젝트 변경 잠금·콘텐츠 보호 E2E.

#### onOpenRestore 제공 시에만 제품 버튼 렌더링; 실제 앱의 null source 미연결 상태에서는 콜백 미제공

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638696116](https://github.com/RefinedStone/codex-exec-loop/pull/2477#discussion_r4638696116).
- 근거: onOpenRestore 제공 시에만 제품 버튼 렌더링; 실제 앱의 null source 미연결 상태에서는 콜백 미제공
- 후속 PR: [#2495](https://github.com/RefinedStone/codex-exec-loop/pull/2495).
- 검증: GitHub 프로젝트 E2E에서 미연결 복원 진입점 숨김; Admin 635 passed/7 skipped; 전체 E2E 19 specs/241 PASS/0 FAIL.

### PR #2476

#### IssueDraftPanel asks가 최신 preview.spanIndexes 및 기존 explicit decision을 반영해 선택기 표시

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638663265](https://github.com/RefinedStone/codex-exec-loop/pull/2476#discussion_r4638663265).
- 근거: IssueDraftPanel asks가 최신 preview.spanIndexes 및 기존 explicit decision을 반영해 선택기 표시
- 후속 PR: [#2494](https://github.com/RefinedStone/codex-exec-loop/pull/2494).
- 검증: 정책 변경 후 기존 log 구간 선택기 노출·선택 유지 E2E.

#### busy 및 needs_confirmation 동안 제목·본문·민감 구간 편집을 잠가 게시/재확인 응답의 open(false)에 의한 미게시 편집 유실 방지

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638663269](https://github.com/RefinedStone/codex-exec-loop/pull/2476#discussion_r4638663269).
- 근거: busy 및 needs_confirmation 동안 제목·본문·민감 구간 편집을 잠가 게시/재확인 응답의 open(false)에 의한 미게시 편집 유실 방지
- 후속 PR: [#2494](https://github.com/RefinedStone/codex-exec-loop/pull/2494).
- 검증: 미확정 상태 입력 잠금·확인 완료 편집 재개 E2E; Admin 635 passed/7 skipped; 전체 E2E 19 specs/246 PASS/0 FAIL.

### PR #2475

#### 복원 포트가 종료한 세션의 보관 요청으로도 평문 반환

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638638811](https://github.com/RefinedStone/codex-exec-loop/pull/2475#discussion_r4638638811).
- 근거: 확인 토큰에 원래 세션 ID를 묶고 복원 진입 전·반환 직전에 세션을 검증한다.
- 코드: [src/application/port/outbound/evidence_vault_port.rs](https://github.com/RefinedStone/codex-exec-loop/blob/76fb9a08005b896f4282effa5bbd82cbbb53e69b/src/application/port/outbound/evidence_vault_port.rs), [src/application/service/evidence_vault_unlock.rs](https://github.com/RefinedStone/codex-exec-loop/blob/76fb9a08005b896f4282effa5bbd82cbbb53e69b/src/application/service/evidence_vault_unlock.rs), [src/application/service/evidence_vault_restore.rs](https://github.com/RefinedStone/codex-exec-loop/blob/76fb9a08005b896f4282effa5bbd82cbbb53e69b/src/application/service/evidence_vault_restore.rs).
- 후속 PR: [#2493](https://github.com/RefinedStone/codex-exec-loop/pull/2493).
- 검증: the_restore_port_rejects_saved_requests_after_session_end.

#### 동시 reconnect가 제목·본문과 CurrentVersion을 서로 교차해 저장

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638638814](https://github.com/RefinedStone/codex-exec-loop/pull/2475#discussion_r4638638814).
- 근거: 현재 판 조회부터 모든 필드와 포인터 저장까지 프로세스 공통 reconnect 잠금으로 직렬화한다.
- 코드: [src/application/service/evidence_vault_restore.rs](https://github.com/RefinedStone/codex-exec-loop/blob/76fb9a08005b896f4282effa5bbd82cbbb53e69b/src/application/service/evidence_vault_restore.rs).
- 후속 PR: [#2493](https://github.com/RefinedStone/codex-exec-loop/pull/2493).
- 검증: concurrent_reconnects_do_not_interleave_their_record_transactions.

### PR #2474

#### 모순된 게시 응답의 증거 유실

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638557725](https://github.com/RefinedStone/codex-exec-loop/pull/2474#discussion_r4638557725).
- 근거: write_response_mismatch를 일지에 보존해 재시작·재조회만으로 Confirmed로 전이하지 않게 한다.
- 코드: [src/domain/issue_publish_intent.rs](https://github.com/RefinedStone/codex-exec-loop/blob/026c2e1212c26c7f7c41b2afd7af5e441a7dab10/src/domain/issue_publish_intent.rs), [src/application/service/issue_publish_journal.rs](https://github.com/RefinedStone/codex-exec-loop/blob/026c2e1212c26c7f7c41b2afd7af5e441a7dab10/src/application/service/issue_publish_journal.rs).
- 후속 PR: [#2502](https://github.com/RefinedStone/codex-exec-loop/pull/2502).
- 검증: journal 15 + domain 4 + restore 13 passed; combined Windows Rust gate passed; git diff --check passed.

#### 동일 공유본 해시의 서로 다른 복원 매핑 충돌

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638557730](https://github.com/RefinedStone/codex-exec-loop/pull/2474#discussion_r4638557730).
- 근거: intent별 복원 판과 게시 순서를 사용한다. 더 최신 전송/확인 포기 기록 또는 순서를 모르는 복수 legacy 기록이 있으면 과거 finalize를 거절한다.
- 코드: [src/domain/issue_publish_intent.rs](https://github.com/RefinedStone/codex-exec-loop/blob/026c2e1212c26c7f7c41b2afd7af5e441a7dab10/src/domain/issue_publish_intent.rs), [src/application/service/issue_publish_journal.rs](https://github.com/RefinedStone/codex-exec-loop/blob/026c2e1212c26c7f7c41b2afd7af5e441a7dab10/src/application/service/issue_publish_journal.rs), [src/application/service/evidence_vault_restore.rs](https://github.com/RefinedStone/codex-exec-loop/blob/026c2e1212c26c7f7c41b2afd7af5e441a7dab10/src/application/service/evidence_vault_restore.rs).
- 후속 PR: [#2502](https://github.com/RefinedStone/codex-exec-loop/pull/2502).
- 검증: journal 15 + domain 4 + restore 13 passed; combined Windows Rust gate passed; git diff --check passed.

### PR #2473

#### 겹치는 원문 일치의 잘못된 단일 판정

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638439739](https://github.com/RefinedStone/codex-exec-loop/pull/2473#discussion_r4638439739).
- 근거: 첫 일치의 다음 UTF-8 문자 경계부터 재검색해 겹치는 일치도 Ambiguous로 거절한다.
- 코드: [src/domain/issue_review.rs](https://github.com/RefinedStone/codex-exec-loop/blob/c9ab9c0e8bc82b0b60da6cc3fa00cebe87d53d3a/src/domain/issue_review.rs).
- 후속 PR: [#2484](https://github.com/RefinedStone/codex-exec-loop/pull/2484).
- 검증: Windows Rust gate 및 회귀 3건 성공.

### PR #2472

#### Jira 프로젝트 이동 후 잘못된 필수 필드 조회

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638429986](https://github.com/RefinedStone/codex-exec-loop/pull/2472#discussion_r4638429986).
- 근거: editmeta 조회 전에 현재 이슈·프로젝트 ID를 재검사해 대상 이동 시 metadata 요청을 보내지 않는다.
- 코드: [src/adapter/outbound/jira/issue_snapshot.rs](https://github.com/RefinedStone/codex-exec-loop/blob/78fe4a554b5835257cbad154347f16d46f297f3f/src/adapter/outbound/jira/issue_snapshot.rs).
- 후속 PR: [#2498](https://github.com/RefinedStone/codex-exec-loop/pull/2498).
- 검증: jira::issue_snapshot tests passed; Windows Rust gate passed; git diff --check passed.

### PR #2471

#### 검증기 stdout 경로 교체

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638427745](https://github.com/RefinedStone/codex-exec-loop/pull/2471#discussion_r4638427745).
- 근거: 검증기 제어 파일을 다시 열지 않고 프로세스 생성 시 확보한 pipe handle을 계속 사용한다.
- 코드: [src/adapter/outbound/issue_code_validator.rs](https://github.com/RefinedStone/codex-exec-loop/blob/c4774d2a662bc1576dcf4db5c78721efc5536d18/src/adapter/outbound/issue_code_validator.rs).
- 후속 PR: [#2501](https://github.com/RefinedStone/codex-exec-loop/pull/2501).
- 검증: Windows Rust gate passed; portable validator tests passed; macOS runtime regressions added; not executed on Windows.

#### 검증기 무제한 출력 저장

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638427746](https://github.com/RefinedStone/codex-exec-loop/pull/2471#discussion_r4638427746).
- 근거: 최초 4096바이트만 보관하고 초과 출력은 제한된 주기로 소비하면서 종료 기한을 검사한다.
- 코드: [src/adapter/outbound/issue_code_validator.rs](https://github.com/RefinedStone/codex-exec-loop/blob/c4774d2a662bc1576dcf4db5c78721efc5536d18/src/adapter/outbound/issue_code_validator.rs).
- 후속 PR: [#2501](https://github.com/RefinedStone/codex-exec-loop/pull/2501).
- 검증: Windows Rust gate passed; portable validator tests passed; macOS runtime regressions added; not executed on Windows.

### PR #2469

#### 인용된 비밀값·URL 인증정보의 뒤쪽 유출

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638254571](https://github.com/RefinedStone/codex-exec-loop/pull/2469#discussion_r4638254571).
- 근거: 인용 종료와 이스케이프를 추적하고 쉼표/세미콜론을 URL userinfo 일부로 처리한다.
- 코드: [src/application/service/issue_handoff.rs](https://github.com/RefinedStone/codex-exec-loop/blob/c5703fb2ac377041418f1fd95c2246e32728959f/src/application/service/issue_handoff.rs).
- 후속 PR: [#2497](https://github.com/RefinedStone/codex-exec-loop/pull/2497).
- 검증: issue_handoff tests passed; Windows Rust gate passed; git diff --check passed.

#### 숫자 없는 명시적 비밀값 탐지

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4638254576](https://github.com/RefinedStone/codex-exec-loop/pull/2469#discussion_r4638254576).
- 근거: 명시적 secret 키 값의 숫자 필수 조건을 제거하고 기존 최소 문자 수 조건을 유지한다.
- 코드: [src/application/service/issue_handoff.rs](https://github.com/RefinedStone/codex-exec-loop/blob/c5703fb2ac377041418f1fd95c2246e32728959f/src/application/service/issue_handoff.rs).
- 후속 PR: [#2497](https://github.com/RefinedStone/codex-exec-loop/pull/2497).
- 검증: issue_handoff tests passed; Windows Rust gate passed; git diff --check passed.

### PR #2468

#### 동시 첫 키 요청이 서로 다른 키를 생성하고 마지막 저장이 앞선 레코드를 고립시킴

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637966301](https://github.com/RefinedStone/codex-exec-loop/pull/2468#discussion_r4637966301).
- 근거: 프로세스 공통 mutex로 키 조회·최초 생성·저장을 직렬화한다. #2480의 동일 원인을 함께 수정한다.
- 코드: [src/application/service/evidence_vault_unlock.rs](https://github.com/RefinedStone/codex-exec-loop/blob/ad9a9a3858d814493946af4ad10828a68c4ca532/src/application/service/evidence_vault_unlock.rs).
- 후속 PR: [#2486](https://github.com/RefinedStone/codex-exec-loop/pull/2486).
- 검증: concurrent_sealing_requests_share_one_initial_key.

#### 키 저장소 조회 대기 중 세션 종료 후에도 권한이 반환됨

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637966308](https://github.com/RefinedStone/codex-exec-loop/pull/2468#discussion_r4637966308).
- 근거: 기존 키 조회 완료 뒤 원래 세션·이슈를 다시 검증해 SessionExpired로 거절한다.
- 코드: [src/application/service/evidence_vault_unlock.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d931d83763e1f026a8270e072b88a9f773fc32f4/src/application/service/evidence_vault_unlock.rs).
- 후속 PR: [#2489](https://github.com/RefinedStone/codex-exec-loop/pull/2489).
- 검증: access_rejects_a_session_ended_during_key_lookup.

### PR #2467

#### 템플릿 중간 경로 링크 차단

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637928450](https://github.com/RefinedStone/codex-exec-loop/pull/2467#discussion_r4637928450).
- 근거: 루트에 고정된 secure_fs 읽기와 1 MiB 상한을 사용한다. 이를 지원하지 않는 Windows는 Unreadable로 명시적 차단한다.
- 코드: [src/adapter/outbound/filesystem/issue_template.rs](https://github.com/RefinedStone/codex-exec-loop/blob/35a7166e7577961ae4a8f2affea769db1b7cfa60/src/adapter/outbound/filesystem/issue_template.rs).
- 후속 PR: [#2503](https://github.com/RefinedStone/codex-exec-loop/pull/2503).
- 검증: 템플릿 서비스 8개·게시 서비스 11개·Windows 파일 어댑터 2개 통과; Windows Rust gate 통과; git diff --check 통과.

#### 템플릿 Markdown 펜스 경계

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637928457](https://github.com/RefinedStone/codex-exec-loop/pull/2467#discussion_r4637928457).
- 근거: 게시 검증과 같은 IssueMarkdownHeadings를 사용해 다른 종류/짧은 펜스와 코드 예제의 제목을 무시한다.
- 코드: [src/application/service/issue_template.rs](https://github.com/RefinedStone/codex-exec-loop/blob/35a7166e7577961ae4a8f2affea769db1b7cfa60/src/application/service/issue_template.rs), [src/domain/issue_markdown.rs](https://github.com/RefinedStone/codex-exec-loop/blob/35a7166e7577961ae4a8f2affea769db1b7cfa60/src/domain/issue_markdown.rs).
- 후속 PR: [#2503](https://github.com/RefinedStone/codex-exec-loop/pull/2503).
- 검증: 템플릿 서비스 8개·게시 서비스 11개·Windows 파일 어댑터 2개 통과; Windows Rust gate 통과; git diff --check 통과.

#### 필수 multiple dropdown의 단일 선택 오해석

- 판단: **수용** (후속 PR 제출).
- 리뷰: [7029575364](https://github.com/RefinedStone/codex-exec-loop/pull/2467#issuecomment-7029575364).
- 근거: attributes.multiple을 읽고 required 복수 선택은 unsupported_required_inputs로 반환한다. optional 복수 선택과 required 단일 선택의 기존 의미는 유지한다.
- 코드: [src/application/service/issue_template.rs](https://github.com/RefinedStone/codex-exec-loop/blob/35a7166e7577961ae4a8f2affea769db1b7cfa60/src/application/service/issue_template.rs).
- 후속 PR: [#2503](https://github.com/RefinedStone/codex-exec-loop/pull/2503).
- 검증: 템플릿 서비스 8개·게시 서비스 11개·Windows 파일 어댑터 2개 통과; Windows Rust gate 통과; git diff --check 통과.

#### 일반 리뷰의 중간 링크/펜스 재확인

- 판단: **수용** (후속 PR 제출).
- 리뷰: [7029575364](https://github.com/RefinedStone/codex-exec-loop/pull/2467#issuecomment-7029575364).
- 근거: 인라인 두 지적과 같은 원인으로 동일 후속 PR에서 수정한다.
- 코드: [src/adapter/outbound/filesystem/issue_template.rs](https://github.com/RefinedStone/codex-exec-loop/blob/35a7166e7577961ae4a8f2affea769db1b7cfa60/src/adapter/outbound/filesystem/issue_template.rs), [src/application/service/issue_template.rs](https://github.com/RefinedStone/codex-exec-loop/blob/35a7166e7577961ae4a8f2affea769db1b7cfa60/src/application/service/issue_template.rs).
- 후속 PR: [#2503](https://github.com/RefinedStone/codex-exec-loop/pull/2503).
- 검증: 템플릿 서비스 8개·게시 서비스 11개·Windows 파일 어댑터 2개 통과; Windows Rust gate 통과; git diff --check 통과.

#### Invalid/미지원 필수 입력을 게시 가능으로 오인

- 판단: **수용** (후속 PR 제출).
- 리뷰: [7029575364](https://github.com/RefinedStone/codex-exec-loop/pull/2467#issuecomment-7029575364).
- 근거: 내용 해시의 같음만으로 게시 가능을 판단하지 않는다. Invalid/Missing/Unreadable 및 미지원 필수 입력이 있는 템플릿은 requirements=None으로 게시를 차단하고 원격 쓰기 0회를 검증한다.
- 코드: [src/application/service/issue_draft.rs](https://github.com/RefinedStone/codex-exec-loop/blob/35a7166e7577961ae4a8f2affea769db1b7cfa60/src/application/service/issue_draft.rs).
- 후속 PR: [#2503](https://github.com/RefinedStone/codex-exec-loop/pull/2503).
- 검증: 템플릿 서비스 8개·게시 서비스 11개·Windows 파일 어댑터 2개 통과; Windows Rust gate 통과; git diff --check 통과.

### PR #2466

#### 보드 동작 권한을 개별 변경 승인 영수증으로 확대

- 판단: **반려** (반려).
- 리뷰: [7029574850](https://github.com/RefinedStone/codex-exec-loop/pull/2466#issuecomment-7029574850).
- 근거: 현재 IssueWriteApproval은 이슈·동작 권한이고 보드 쓰기의 제품 UI/서비스 호출자는 연결되지 않았다. 리뷰도 현 계약의 확정 결함으로 분류하지 않았다. 이 타입을 개별 선택 승인으로 간주하는 동작을 새로 만들지 않는다.
- 코드: [src/domain/issue_work.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/domain/issue_work.rs), [src/application/port/outbound/issue_board_field_port.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/port/outbound/issue_board_field_port.rs), [src/adapter/outbound/github/issue_board_field.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/github/issue_board_field.rs).
- 재검토 조건: 보드 편집 UI를 연결하기 전에 issue/project/item/field/option/정책 판과 필요 시 기존 값을 묶은 Kernel 승인 객체, 유효한 다른 조합 교체 시 mutation=0 회귀를 구현한다.
- 검증: 현재 코드 및 기존 회귀 계약 대조.

#### 보드 mutation 결과 불명 상태와 자동 재전송 정책

- 판단: **반려** (반려).
- 리뷰: [7029574850](https://github.com/RefinedStone/codex-exec-loop/pull/2466#issuecomment-7029574850).
- 근거: 현재 연결된 제품 호출자나 자동 재전송 경로가 없다. adapter 오류를 미수정의 증거라고 문서화하지 않는다. 결과 불명 상태·재조회 UI는 보드 쓰기 호출자를 추가할 때 함께 정할 통합 계약이다.
- 코드: [src/application/port/outbound/issue_board_field_port.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/port/outbound/issue_board_field_port.rs), [src/adapter/outbound/github/issue_board_field.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/github/issue_board_field.rs).
- 재검토 조건: 보드 쓰기 UI/Kernel 연결 시 전송 후 오류를 결과 불명으로 표현하고 새 승인 없는 자동 재시도를 차단한다.
- 검증: 현재 코드 및 기존 회귀 계약 대조.

### PR #2465

#### 마스킹 함수에 탐지 완료 프로토콜 추가

- 판단: **반려** (반려).
- 리뷰: [7029574322](https://github.com/RefinedStone/codex-exec-loop/pull/2465#issuecomment-7029574322).
- 근거: mask_issue_text는 명시적으로 전달된 구간의 치환만 담당한다. 현재 게시 UI의 사용자 구간과 AI 인계의 패턴 탐지는 각 호출자가 제공한다. 이 함수의 성공을 임의의 비밀값 탐지 완료 증명으로 재정의하지 않는다. 실제 AI 탐지 누락은 #2469 후속 PR에서 별도 수정한다.
- 코드: [src/application/service/issue_masking.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_masking.rs), [src/application/service/issue_draft.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_draft.rs), [src/application/service/issue_handoff.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_handoff.rs).
- 관련 실제 수정: [#2497](https://github.com/RefinedStone/codex-exec-loop/pull/2497).
- 재검토 조건: 자동 탐지기의 부분 완료/실패 상태를 연결하는 기능이 생기면 그 경계에 별도 완료 상태와 보류 계약을 추가한다.
- 검증: 현재 코드 및 기존 회귀 계약 대조.

#### 모든 secret의 복원 가능 정책 강제 금지

- 판단: **반려** (반려).
- 리뷰: [7029574322](https://github.com/RefinedStone/codex-exec-loop/pull/2465#issuecomment-7029574322).
- 근거: 현재 정책은 secret의 Remove/RestorableMask/UserConfirm을 명시적으로 구분한다. Remove는 사용자 결정으로 약화할 수 없다. 모든 정책을 강제 Remove로 바꾸는 것은 기존 설정 의미를 바꾸는 제품 정책 변경이므로 이번 버그 수정에 포함하지 않는다.
- 코드: [src/domain/issue_policy.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/domain/issue_policy.rs), [src/application/service/issue_policy.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_policy.rs), [src/application/service/issue_masking.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_masking.rs).
- 재검토 조건: 제품 규칙을 모든 자격 증명 영구 폐기로 확정하면 정책 파서와 처리기의 불변식을 함께 바꾼다.
- 검증: 현재 코드 및 기존 회귀 계약 대조.

#### Desktop UTF-16와 마스킹 UTF-8 좌표 변환

- 판단: **기대응** (기대응).
- 리뷰: [7029574322](https://github.com/RefinedStone/codex-exec-loop/pull/2465#issuecomment-7029574322).
- 근거: IssueDraftService::sensitive_span/byte_offset이 UTF-16 위치를 UTF-8 경계로 변환하고 문자 중간을 거절한다. span_positions_are_utf16_units_of_the_client_text 회귀가 있으며 마스킹의 position은 공유본의 UTF-8 위치로 명시되어 있다.
- 코드: [src/application/service/issue_draft.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_draft.rs), [src/application/service/issue_masking.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_masking.rs).
- 검증: 현재 코드 및 기존 회귀 계약 대조.

#### 추가 입력 크기/구간 수 상한 및 순열 시험

- 판단: **반려** (반려).
- 리뷰: [7029574322](https://github.com/RefinedStone/codex-exec-loop/pull/2465#issuecomment-7029574322).
- 근거: 추가 한도는 현재 API에 없는 입력 허용 정책이다. 이번 검토에서 좌표 오류가 확인되지 않았고 실제 누락은 별도 회귀로 고쳤다. 근거 없는 수치 제한이나 동일 구현을 반복하는 순열 시험을 추가하지 않는다.
- 코드: [src/application/service/issue_masking.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_masking.rs), [src/application/service/issue_draft.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_draft.rs).
- 재검토 조건: 대표적인 대형 로그와 구간 수의 측정으로 허용 크기·지연 목표를 정하면 경계에서 한도를 검증한다.
- 검증: 현재 코드 및 기존 회귀 계약 대조.

#### 게시와 AI 인계 정책의 분리

- 판단: **기대응** (기대응).
- 리뷰: [7029574322](https://github.com/RefinedStone/codex-exec-loop/pull/2465#issuecomment-7029574322).
- 근거: IssueDraftService는 Publish, IssueHandoffService는 AiHandoff 정책으로 각각 새 마스킹 결과를 생성한다. 한 결과를 대상 구분 없이 재사용하는 현재 호출 경로는 없다.
- 코드: [src/application/service/issue_draft.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_draft.rs), [src/application/service/issue_handoff.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_handoff.rs).
- 검증: 현재 코드 및 기존 회귀 계약 대조.

### PR #2464

#### 코드 펜스 문자·길이 추적

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637889234](https://github.com/RefinedStone/codex-exec-loop/pull/2464#discussion_r4637889234).
- 근거: Boolean 토글을 문자·길이 기반 공용 해석기로 바꾸어 코드 예제 제목이 필수 섹션을 충족하지 못하게 한다.
- 코드: [src/domain/issue_markdown.rs](https://github.com/RefinedStone/codex-exec-loop/blob/2af55ad9d8bab6cc1a3dc36a2eb648226722a69b/src/domain/issue_markdown.rs), [src/domain/issue_validation.rs](https://github.com/RefinedStone/codex-exec-loop/blob/2af55ad9d8bab6cc1a3dc36a2eb648226722a69b/src/domain/issue_validation.rs).
- 후속 PR: [#2488](https://github.com/RefinedStone/codex-exec-loop/pull/2488).
- 검증: domain::issue_ 20 passed; Windows Rust gate passed; git diff --check passed.

#### C# 제목의 실제 # 보존

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637889237](https://github.com/RefinedStone/codex-exec-loop/pull/2464#discussion_r4637889237).
- 근거: 공백으로 분리된 ATX 닫는 표식만 제거해 C#와 C# ###를 같은 제목으로 인식한다.
- 코드: [src/domain/issue_markdown.rs](https://github.com/RefinedStone/codex-exec-loop/blob/2af55ad9d8bab6cc1a3dc36a2eb648226722a69b/src/domain/issue_markdown.rs), [src/domain/issue_validation.rs](https://github.com/RefinedStone/codex-exec-loop/blob/2af55ad9d8bab6cc1a3dc36a2eb648226722a69b/src/domain/issue_validation.rs).
- 후속 PR: [#2488](https://github.com/RefinedStone/codex-exec-loop/pull/2488).
- 검증: domain::issue_ 20 passed; Windows Rust gate passed; git diff --check passed.

#### 일반 리뷰의 펜스/C# 재확인

- 판단: **수용** (후속 PR 제출).
- 리뷰: [7029573748](https://github.com/RefinedStone/codex-exec-loop/pull/2464#issuecomment-7029573748).
- 근거: 두 인라인 지적과 같은 원인이므로 동일 후속 PR에서 수정한다.
- 코드: [src/domain/issue_validation.rs](https://github.com/RefinedStone/codex-exec-loop/blob/2af55ad9d8bab6cc1a3dc36a2eb648226722a69b/src/domain/issue_validation.rs).
- 후속 PR: [#2488](https://github.com/RefinedStone/codex-exec-loop/pull/2488).
- 검증: domain::issue_ 20 passed; Windows Rust gate passed; git diff --check passed.

#### 게시 직전 요구 사항·정책·템플릿 재검증

- 판단: **기대응** (기대응).
- 리뷰: [7029573748](https://github.com/RefinedStone/codex-exec-loop/pull/2464#issuecomment-7029573748).
- 근거: IssueDraftService::publish가 conventions와 evaluate를 다시 실행하고 IssuePublishApproval의 글 해시·정책·템플릿 판을 현재 값과 대조한다.
- 코드: [src/application/service/issue_draft.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_draft.rs), [src/application/service/issue_publish.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/issue_publish.rs).
- 검증: 현재 코드 및 기존 회귀 계약 대조.

#### 필수 체크박스/복수 선택의 검증 계약

- 판단: **수용** (후속 PR 제출).
- 리뷰: [7029573748](https://github.com/RefinedStone/codex-exec-loop/pull/2464#issuecomment-7029573748).
- 근거: 현재 검증기가 검사하지 못하는 required checkbox와 multiple dropdown을 미지원으로 표시하고 게시를 차단한다. 미지원 항목을 단순 비어 있지 않은 문자열로 통과시키지 않는다.
- 코드: [src/application/service/issue_template.rs](https://github.com/RefinedStone/codex-exec-loop/blob/35a7166e7577961ae4a8f2affea769db1b7cfa60/src/application/service/issue_template.rs), [src/application/service/issue_draft.rs](https://github.com/RefinedStone/codex-exec-loop/blob/35a7166e7577961ae4a8f2affea769db1b7cfa60/src/application/service/issue_draft.rs).
- 후속 PR: [#2503](https://github.com/RefinedStone/codex-exec-loop/pull/2503).
- 검증: 템플릿 서비스 8개·게시 서비스 11개·Windows 파일 어댑터 2개 통과; Windows Rust gate 통과; git diff --check 통과.

### PR #2463

#### 원자적 파일 교체 이후 오류에도 이전 레코드가 남는다고 약속한 저장 계약

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637740159](https://github.com/RefinedStone/codex-exec-loop/pull/2463#discussion_r4637740159).
- 근거: rename 이후 실패를 WriteOutcomeUnknown으로 구분하고 재조회를 요구한다. 원래 오류 문자열도 보존한다.
- 코드: [src/adapter/outbound/filesystem/secure_fs.rs](https://github.com/RefinedStone/codex-exec-loop/blob/f29979bca9aabe1ff59caee137161f7e7c390ced/src/adapter/outbound/filesystem/secure_fs.rs), [src/adapter/outbound/filesystem/evidence_vault_store.rs](https://github.com/RefinedStone/codex-exec-loop/blob/f29979bca9aabe1ff59caee137161f7e7c390ced/src/adapter/outbound/filesystem/evidence_vault_store.rs), [src/application/port/outbound/evidence_vault_port.rs](https://github.com/RefinedStone/codex-exec-loop/blob/f29979bca9aabe1ff59caee137161f7e7c390ced/src/application/port/outbound/evidence_vault_port.rs).
- 후속 PR: [#2500](https://github.com/RefinedStone/codex-exec-loop/pull/2500).
- 검증: failure_after_replacement_reports_an_unknown_outcome.

### PR #2460

#### desktop/src/renderer/conversation/turnBlocks.ts stepStatus: completed + success=false를 failed로 처리

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637740403](https://github.com/RefinedStone/codex-exec-loop/pull/2460#discussion_r4637740403).
- 근거: desktop/src/renderer/conversation/turnBlocks.ts stepStatus: completed + success=false를 failed로 처리
- 후속 PR: [#2483](https://github.com/RefinedStone/codex-exec-loop/pull/2483).
- 검증: 기존 코드에서 회귀 테스트 실패 확인; Admin gate 637 passed, 7 skipped; 전체 E2E 240 PASS/0 FAIL.

#### stepDetail(webSearch)가 원래 검색어/URL/페이지내 검색 대상을 보존

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637740413](https://github.com/RefinedStone/codex-exec-loop/pull/2460#discussion_r4637740413).
- 근거: stepDetail(webSearch)가 원래 검색어/URL/페이지내 검색 대상을 보존
- 후속 PR: [#2483](https://github.com/RefinedStone/codex-exec-loop/pull/2483).
- 검증: 긴 검색/열기/찾기 세 경우 회귀 검증; 전체 E2E 240 PASS/0 FAIL.

### PR #2458

#### classifyInstall이 .claude/.codex/.agents 및 모든 부모 경로의 lstat을 확인하여 symlink/junction을 읽기 전용으로 분류

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637661004](https://github.com/RefinedStone/codex-exec-loop/pull/2458#discussion_r4637661004).
- 근거: classifyInstall이 .claude/.codex/.agents 및 모든 부모 경로의 lstat을 확인하여 symlink/junction을 읽기 전용으로 분류
- 후속 PR: [#2487](https://github.com/RefinedStone/codex-exec-loop/pull/2487).
- 검증: Windows 부모 junction 3종 회귀; 수정 전 5개 회귀 모두 실패→수정 후 73 passed/3 skipped.

#### writeEntry가 프로세스 간 ledger 잠금 안에서 최신 장부 읽기/전체 파일 교체; unique wx 임시 파일, 초기화 중 잠금 fail-closed

- 판단: **수용** (후속 PR 제출).
- 리뷰: [4637661009](https://github.com/RefinedStone/codex-exec-loop/pull/2458#discussion_r4637661009).
- 근거: writeEntry가 프로세스 간 ledger 잠금 안에서 최신 장부 읽기/전체 파일 교체; unique wx 임시 파일, 초기화 중 잠금 fail-closed
- 후속 PR: [#2487](https://github.com/RefinedStone/codex-exec-loop/pull/2487).
- 검증: 실제 별도 Node 프로세스 lock 보유 시 다른 키 쓰기 거절·기존 데이터 보존·해제 후 두 키 유지; Admin gate 640 passed/7 skipped; 스킬 적용·롤백 E2E 19 PASS.

### PR #2454

#### 진행 중인 목록 조회에도 스레드 삭제 알림을 보존하세요

- 판단: **기대응** (기대응).
- 리뷰: [4636091445](https://github.com/RefinedStone/codex-exec-loop/pull/2454#discussion_r4636091445), [4636223156](https://github.com/RefinedStone/codex-exec-loop/pull/2454#discussion_r4636223156).
- 근거: 목록 수신 뒤 withLifecycleChanges가 removedThreadIds/renamedThreads/추가 스레드를 재적용한다. 초기 목록과 rollout 병합 모두 최신 생명주기 변경을 보존한다.
- 코드: [desktop/src/renderer/session/desktopSession.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/session/desktopSession.ts).
- 기존 수정: [de7d738b8cdf](https://github.com/RefinedStone/codex-exec-loop/commit/de7d738b8cdf9f4543b31d2d31745fe8c8079fc7) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2451

#### 캡처 실행 전에 바이너리 경로를 절대 경로로 변환하세요

- 판단: **기대응** (기대응).
- 리뷰: [4636051439](https://github.com/RefinedStone/codex-exec-loop/pull/2451#discussion_r4636051439), [4636224314](https://github.com/RefinedStone/codex-exec-loop/pull/2451#discussion_r4636224314).
- 근거: capture와 release packaging이 상대 CARGO_TARGET_DIR을 빌드 기준 절대 경로로 바꾸고 임시 workspace에서도 동일 바이너리를 참조한다.
- 코드: [scripts/capture_typed_activity_rail_e2e.sh](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/scripts/capture_typed_activity_rail_e2e.sh), [scripts/package_native_release.sh](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/scripts/package_native_release.sh).
- 기존 수정: [2117a4c453aa](https://github.com/RefinedStone/codex-exec-loop/commit/2117a4c453aa531afae8e4132534abb0324470ea) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2442

#### Windows UNC 경로의 확장 접두사를 올바르게 정규화하세요

- 판단: **기대응** (기대응).
- 리뷰: [4628041323](https://github.com/RefinedStone/codex-exec-loop/pull/2442#discussion_r4628041323), [4636287323](https://github.com/RefinedStone/codex-exec-loop/pull/2442#discussion_r4636287323).
- 근거: comparable_path는 Windows 확장 UNC 접두사를 일반 UNC로 먼저 바꾸고 구분자를 정규화한다. 동일 공유 경로 회귀 시험이 있다.
- 코드: [src/core/app/session.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/core/app/session.rs).
- 기존 수정: [0c2e3737742a](https://github.com/RefinedStone/codex-exec-loop/commit/0c2e3737742aa33fed93b4eebcce3a73623d3667) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2413

#### 내장 정적 자산 삭제·이동 누락 검사

- 판단: **수용** (후속 PR 생성).
- 리뷰: [7029573263](https://github.com/RefinedStone/codex-exec-loop/pull/2413#issuecomment-7029573263).
- 근거: 실제 Rust bundled_asset! 선언을 읽고 모든 참조가 정규 파일로 존재하는지 검사하는 Node 시험을 추가했다. 모든 정적 내용 변경에 전체 Rust 빌드를 강제하는 대신 목적에 맞는 존재성 검사를 수행한다.
- 코드: [assets/admin/scripts/embedded-assets.test.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/013452bbd3487e42b20498cdd8976c2c8457f741/assets/admin/scripts/embedded-assets.test.mjs), [src/adapter/inbound/admin_api/static_assets.rs](https://github.com/RefinedStone/codex-exec-loop/blob/013452bbd3487e42b20498cdd8976c2c8457f741/src/adapter/inbound/admin_api/static_assets.rs), [scripts/check_native_pr.ps1](https://github.com/RefinedStone/codex-exec-loop/blob/013452bbd3487e42b20498cdd8976c2c8457f741/scripts/check_native_pr.ps1), [scripts/check_node_surfaces.sh](https://github.com/RefinedStone/codex-exec-loop/blob/013452bbd3487e42b20498cdd8976c2c8457f741/scripts/check_node_surfaces.sh).
- 후속 PR: [#2492](https://github.com/RefinedStone/codex-exec-loop/pull/2492).
- 검증: node --test assets/admin/scripts/embedded-assets.test.mjs: 3개 성공; planner Admin; Windows Admin gate 성공; git diff --check 성공.

### PR #2412

#### 실행 가능한 docs 도구의 CI 범위 및 시험 등록

- 판단: **기대응** (기대응).
- 리뷰: [7029544279](https://github.com/RefinedStone/codex-exec-loop/pull/2412#issuecomment-7029544279).
- 근거: DOC_VALIDATION_PATHS를 docs-only 판정 앞에 두고 client_count_validation.test.mjs를 Windows/POSIX Node gate에 포함한다.
- 코드: [scripts/ci-scope.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/scripts/ci-scope.mjs), [scripts/check_native_pr.ps1](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/scripts/check_native_pr.ps1), [scripts/check_node_surfaces.sh](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/scripts/check_node_surfaces.sh).
- 기존 수정: [2534f8be212b](https://github.com/RefinedStone/codex-exec-loop/commit/2534f8be212b13874b82d332b54532ec8ccf054a) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### client 상한·Node/Python 입력 표현 통일 추가

- 판단: **반려** (반려).
- 리뷰: [7029544279](https://github.com/RefinedStone/codex-exec-loop/pull/2412#issuecomment-7029544279).
- 근거: 현재 완료 계약은 연결 전 잘못된 0/음수/비정수 수를 거절하는 것이다. 별도 제품 상한과 표현식 허용 정책은 정해지지 않았으며 기존 유효 입력을 축소하는 변경이므로 이번 버그 대응에서 추가하지 않는다.
- 코드: [docs/plan/17-akra-desktop/scripts/proxy_ws_check.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/docs/plan/17-akra-desktop/scripts/proxy_ws_check.mjs), [docs/plan/17-akra-desktop/scripts/client_count_validation.test.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/docs/plan/17-akra-desktop/scripts/client_count_validation.test.mjs).
- 재검토 조건: 운영 가능한 동시 client 상한 또는 플랫폼 간 표현 동등성 요구가 정해질 때 별도 계약·시험으로 진행.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2411

#### #2410 통합 fixture와 Node gate 등록

- 판단: **기대응** (기대응).
- 리뷰: [7029543789](https://github.com/RefinedStone/codex-exec-loop/pull/2411#issuecomment-7029543789).
- 근거: evidence fixture가 detailDrawerGeneration: 0을 제공하고 Windows/POSIX에서 admin scripts의 *.test.mjs 전체를 실행한다. 현재 통합 drawer 5개 시험을 Full/Admin gate에서 통과했다.
- 코드: [assets/admin/scripts/akra-dashboard-evidence.test.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/assets/admin/scripts/akra-dashboard-evidence.test.mjs), [scripts/check_native_pr.ps1](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/scripts/check_native_pr.ps1), [scripts/check_node_surfaces.sh](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/scripts/check_node_surfaces.sh).
- 기존 수정: [62f350586354](https://github.com/RefinedStone/codex-exec-loop/commit/62f3505863545a41a6258ecb5d76db438e666460) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### A→B→A·같은 key 재열기 추가 coverage

- 판단: **반려** (반려).
- 리뷰: [7029543789](https://github.com/RefinedStone/codex-exec-loop/pull/2411#issuecomment-7029543789).
- 근거: 현재 generation/mode/recordKey 검사는 모든 async 결과 적용 전에 존재한다. 리뷰도 추가 차단 결함으로 판단하지 않았고 필수 통합 실패는 이미 고쳐졌으므로 별도 순열 시험 확대는 이번 범위에서 하지 않는다.
- 코드: [assets/admin/scripts/akra-dashboard.js](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/assets/admin/scripts/akra-dashboard.js), [assets/admin/scripts/akra-dashboard-detail-generation.test.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/assets/admin/scripts/akra-dashboard-detail-generation.test.mjs).
- 재검토 조건: generation 검사가 없는 새 응답 경로나 재현 가능한 역순 적용이 추가되면 해당 시나리오를 회귀로 고정.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2410

#### #2411 통합 fixture와 Node gate 등록

- 판단: **기대응** (기대응).
- 리뷰: [7029543207](https://github.com/RefinedStone/codex-exec-loop/pull/2410#issuecomment-7029543207).
- 근거: evidence VM에 detailDrawerGeneration 초기값이 있으며 Windows/POSIX admin glob 실행으로 양쪽 suite가 함께 검증된다. 통합 5개 시험이 현재 gate에서 통과했다.
- 코드: [assets/admin/scripts/akra-dashboard-evidence.test.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/assets/admin/scripts/akra-dashboard-evidence.test.mjs), [scripts/check_native_pr.ps1](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/scripts/check_native_pr.ps1), [scripts/check_node_surfaces.sh](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/scripts/check_node_surfaces.sh).
- 기존 수정: [62f350586354](https://github.com/RefinedStone/codex-exec-loop/commit/62f3505863545a41a6258ecb5d76db438e666460) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2409

#### 종료 확인 후 채널 연결 해제까지 기다리세요

- 판단: **기대응** (기대응).
- 리뷰: [4624740702](https://github.com/RefinedStone/codex-exec-loop/pull/2409#discussion_r4624740702).
- 근거: 마지막 종료 검사가 try_recv 대신 recv_timeout(5초)에서 Disconnected를 기다리므로 drop 신호와 송신자 필드 해제 사이 선점을 허용한다.
- 코드: [src/composition/repository_observer.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/composition/repository_observer.rs).
- 기존 수정: [5e7991565233](https://github.com/RefinedStone/codex-exec-loop/commit/5e7991565233d5954b70249cc9eefd9ad7e2a7d3) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 종료 채널 경쟁 및 기존 CI 잠금 실패

- 판단: **기대응** (기대응).
- 리뷰: [7029542592](https://github.com/RefinedStone/codex-exec-loop/pull/2409#issuecomment-7029542592).
- 근거: observer 종료 경쟁은 5e799156에서 recv_timeout으로 해결되었다. 리뷰가 별도로 지목한 kernel ownership lock 경합은 #2482의 후속 머지 범위이며 observer 자체에 중복 수정하지 않는다. 관찰자 drop은 진행 중 read 완료 뒤 다음 주기를 막는 의미다.
- 코드: [src/composition/repository_observer.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/composition/repository_observer.rs), [src/composition/kernel_command_execution.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/composition/kernel_command_execution.rs).
- 기존 수정: [5e7991565233](https://github.com/RefinedStone/codex-exec-loop/commit/5e7991565233d5954b70249cc9eefd9ad7e2a7d3) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2408

#### 추가 retry/coalesce/Disabled 순열 시험

- 판단: **반려** (반려).
- 리뷰: [7029542054](https://github.com/RefinedStone/codex-exec-loop/pull/2408#issuecomment-7029542054).
- 근거: 현재 active_setup correlation을 확인한 완료만 처리하고 실패한 동일 setup_request만 지운다. Disabled 유지와 자동 네트워크 backoff는 다른 계약이다. 새 실패 재현 없이 비차단 순열 coverage를 확대하지 않는다.
- 코드: [src/core/app/github_review_reducer.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/core/app/github_review_reducer.rs).
- 재검토 조건: 늦은 완료가 새 성공/Disabled를 덮는 재현 또는 자동 재시도 요구가 생길 때.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2407

#### Unicode 정규화·비ASCII case-fold 추가 정책

- 판단: **반려** (반려).
- 리뷰: [7029541440](https://github.com/RefinedStone/codex-exec-loop/pull/2407#issuecomment-7029541440).
- 근거: query와 field 비교는 모두 명시된 ASCII 대소문자 정책이다. NFC/NFD 통합이나 비ASCII case-fold는 검색 결과·점수를 바꾸는 기능 확장이며 현재 동일 원문 누락 수정에 필요하지 않다.
- 코드: [src/domain/session_browser/search.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/domain/session_browser/search.rs).
- 재검토 조건: 전체 name/preview/path/branch 비교와 점수에 공통 Unicode 정책을 적용하기로 결정할 때.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2406

#### 지연 show·관계없는 WebContents 추가 주입 시험

- 판단: **반려** (반려).
- 리뷰: [7029526987](https://github.com/RefinedStone/codex-exec-loop/pull/2406#issuecomment-7029526987).
- 근거: 현재 E2E는 main이 반환한 profileId와 별도 제목/cookie 완료를 기다린다. #2405 통합 browser E2E도 이번 전체 실행에서 통과했다. 제안은 추가 fault permutation이며 현재 실패가 없어 추가 구현을 하지 않는다.
- 코드: [desktop/test/e2e/browser.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/test/e2e/browser.mjs), [desktop/src/renderer/browser/BrowserPanel.tsx](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/browser/BrowserPanel.tsx).
- 재검토 조건: 실제 목표 profile 전에 탐색되는 재현이 생기면 제어된 show 지연 회귀로 고정.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2405

#### 다중 창 reload·자식 장기 프로세스 종료 정책 확대

- 판단: **반려** (반려).
- 리뷰: [7029526509](https://github.com/RefinedStone/codex-exec-loop/pull/2405#issuecomment-7029526509).
- 근거: ready/render-process-gone/destroyed는 같은 ownerId 정리 경로를 사용한다. 다른 창 보존·반복 reload 추가 coverage 및 shell 자손 종료 보장은 현재 수정과 다른 확대 계약이며 검증되지 않은 보장으로 쓰지 않는다.
- 코드: [desktop/src/main/index.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/index.ts), [desktop/src/main/terminalService.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/terminalService.ts), [desktop/test/e2e/terminal.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/test/e2e/terminal.mjs), [desktop/test/e2e/browser.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/test/e2e/browser.mjs).
- 재검토 조건: 두 창 소유권 누수 재현 또는 터미널 자손 종료 정책 확정 시 해당 통합 시험 추가.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2404

#### 검증 open 전에 FIFO로 교체되어 멈추는 경로

- 판단: **기대응** (기대응).
- 리뷰: [7029525972](https://github.com/RefinedStone/codex-exec-loop/pull/2404#issuecomment-7029525972).
- 근거: Unix open에 O_NONBLOCK이 포함되며 동일 fd의 정규파일/identity 재검증을 유지한다. 실제 FIFO 교체 회귀 파일이 있다.
- 코드: [desktop/src/main/gitStatus.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/gitStatus.ts), [desktop/test/gitStatusFifo.test.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/test/gitStatusFifo.test.ts).
- 기존 수정: [a7883c03b697](https://github.com/RefinedStone/codex-exec-loop/commit/a7883c03b6977fbee2624a74ea2684f75f1f1358) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2403

#### 이전 경로 존재 검사를 이름 변경 discard에만 적용하십시오

- 판단: **기대응** (기대응).
- 리뷰: [4624645414](https://github.com/RefinedStone/codex-exec-loop/pull/2403#discussion_r4624645414).
- 근거: originalPath 부재 검사는 discard이면서 status가 R로 시작할 때만 적용한다. 복사 C는 원본을 남기고 복사본만 제거하는 경로를 유지한다.
- 코드: [desktop/src/main/gitStatus.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/gitStatus.ts), [desktop/test/gitStatus.test.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/test/gitStatus.test.ts).
- 기존 수정: [f0a20774f31a](https://github.com/RefinedStone/codex-exec-loop/commit/f0a20774f31a33c6b0f3d44dfa2fa55b20133d6c) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 복사 C 상태의 originalPath 부재 검사

- 판단: **기대응** (기대응).
- 리뷰: [7029525408](https://github.com/RefinedStone/codex-exec-loop/pull/2403#issuecomment-7029525408).
- 근거: 추가 부재 검사를 R discard에만 제한하고 C 원본을 보존한다. 원본 inline 지적과 같은 원인이다.
- 코드: [desktop/src/main/gitStatus.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/gitStatus.ts), [desktop/test/gitStatus.test.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/test/gitStatus.test.ts).
- 기존 수정: [f0a20774f31a](https://github.com/RefinedStone/codex-exec-loop/commit/f0a20774f31a33c6b0f3d44dfa2fa55b20133d6c) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 다중 Git 명령 실패의 자동 복구·부분 실행 상태 확대

- 판단: **반려** (반려).
- 리뷰: [7029525408](https://github.com/RefinedStone/codex-exec-loop/pull/2403#issuecomment-7029525408).
- 근거: rename discard의 rm/restore를 원자 트랜잭션으로 만들거나 자동 롤백하는 계약은 기존 Git 동작 범위를 바꾼다. #2377의 작성자 판단과 plan 27 §9.3은 자동 복구 미제공 및 git status 확인을 이미 명시한다. 부분 실행 가능성을 없다고 주장하지 않으며 이번에 자동 복구를 추가하지 않는다.
- 코드: [desktop/src/main/gitStatus.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/gitStatus.ts), [src/application/service/git_operation.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/git_operation.rs).
- 재검토 조건: 제어된 두 번째 명령 실패 시나리오와 사용자 변경을 보존하는 복구 소유권/저널 계약이 정해질 때.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2402

#### ExitCode 변경 후 부정확한 runtime 소멸 주석

- 판단: **기대응** (기대응).
- 리뷰: [7029537956](https://github.com/RefinedStone/codex-exec-loop/pull/2402#issuecomment-7029537956).
- 근거: gateway 종료 주석이 ExitCode를 통한 diagnostics flush와 남은 blocking 작업의 제한된 대기로 정정되었다.
- 코드: [src/adapter/inbound/gateway/mod.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/inbound/gateway/mod.rs).
- 기존 수정: [09b80d03d4de](https://github.com/RefinedStone/codex-exec-loop/commit/09b80d03d4de8179a9cac43bb0ec97e126ff7546) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 정상 SIGINT 마지막 로그·모든 producer 종료 시간 추가 시험

- 판단: **반려** (반려).
- 리뷰: [7029537956](https://github.com/RefinedStone/codex-exec-loop/pull/2402#issuecomment-7029537956).
- 근거: 이번 수정은 main guard의 drop 실행이다. 실제 장기 blocking 회귀가 확인되지 않은 상태에서 모든 producer의 종료 정책까지 확대하지 않는다. flush와 프로세스 전체 종료 기한을 동일 보장으로 주장하지 않는다.
- 코드: [src/adapter/inbound/gateway/mod.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/inbound/gateway/mod.rs).
- 재검토 조건: 정상 종료 마지막 로그 누락이나 deadline 초과 재현이 생기면 해당 producer를 대상으로 시험.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2401

#### worker 자체 commit 오인과 혼합 execution snapshot

- 판단: **기대응** (기대응).
- 리뷰: [7029540760](https://github.com/RefinedStone/codex-exec-loop/pull/2401#issuecomment-7029540760).
- 근거: worker mutation 결과의 정확한 committed_planning_revision을 reconciliation snapshot에 전달한다. execution snapshot은 원자 port를 우선하며 repo authority에서 요구하고 legacy는 앞/뒤 revision 비교를 사용한다. 복구 CAS도 revision을 함께 검사한다.
- 코드: [src/application/service/planning/worker/orchestration.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/planning/worker/orchestration.rs), [src/application/service/planning/repair/reconciliation.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/planning/repair/reconciliation.rs).
- 기존 수정: [1b757f9c3476](https://github.com/RefinedStone/codex-exec-loop/commit/1b757f9c347677971f08e3c7942d884e2541f046) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2400

#### guard helper 공통화 및 복제 경로 추가 panic 시험

- 판단: **반려** (반려).
- 리뷰: [7029540097](https://github.com/RefinedStone/codex-exec-loop/pull/2400#issuecomment-7029540097).
- 근거: 현재 task/admin CRUD/file-sync 모두 catch→원래 owner release→resume_unwind 순서로 정리한다. helper 통합과 추가 경로 시험은 비차단 유지보수 권고이며 새 결함 재현이 없다. abort/강제 종료의 lease 복구까지 이번 unwind 수정에 포함하지 않는다.
- 코드: [src/application/service/planning/shared/authority_mutation_guard.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/planning/shared/authority_mutation_guard.rs), [src/application/service/planning/admin/crud.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/planning/admin/crud.rs), [src/application/service/planning/admin/file_sync.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/planning/admin/file_sync.rs).
- 재검토 조건: 경로 간 의미 차이가 생기거나 owner 해제 누락 재현이 있을 때 공통 계약으로 묶기.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2399

#### 같은 target의 두 연속 delivery 전체 통합 시험

- 판단: **반려** (반려).
- 리뷰: [7029539557](https://github.com/RefinedStone/codex-exec-loop/pull/2399#issuecomment-7029539557).
- 근거: 현재 orchestrator/distributor는 공통 integration_worktree_head 분류를 사용하고 Git 조회 실패를 정상 detached로 보지 않는다. 리뷰의 전체 admission 시나리오는 유용하지만 분류 버그 재현은 아니므로 이번 코드 후속 범위에서는 추가하지 않는다. 기존 분류 시험만으로 전체 delivery 검증 완료라고 주장하지 않는다.
- 코드: [src/application/service/parallel_mode/support.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/parallel_mode/support.rs).
- 재검토 조건: queue delivery 통합 하네스에서 clean→dirty/attached/조회 실패 전환을 제어할 계약을 정할 때.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2398

#### 정상 close·producer 종료 교차 순열 추가

- 판단: **반려** (반려).
- 리뷰: [7029537411](https://github.com/RefinedStone/codex-exec-loop/pull/2398#issuecomment-7029537411).
- 근거: close enqueue와 writer 종료는 동일 deadline이고 만료 뒤 abort를 await한다. 제안은 정상 flush·종료 경쟁·별도 producer 통합 coverage 확대이며 기존 문제를 재현하는 blocked queue/drop 시험이 있다. 실제 새 실패 없이 종료 정책을 변경하지 않는다.
- 코드: [src/adapter/inbound/gateway/session.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/inbound/gateway/session.rs).
- 재검토 조건: producer가 session 종료 뒤 남거나 정상 close 전달이 누락되는 재현이 나올 때.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2397

#### SSE TCP backpressure 전체 연결 종료 기한 확대

- 판단: **반려** (반려).
- 리뷰: [7029536894](https://github.com/RefinedStone/codex-exec-loop/pull/2397#issuecomment-7029536894).
- 근거: watch/take_until은 SSE body가 poll될 때 종료를 관찰한다. 기존 시험은 실제 꽉 찬 TCP 송신 버퍼까지 보장하지 않는다. 별도 소켓 최종 deadline 추가는 비재현 계약 확대라 이번 패치에 넣지 않으며 모든 비읽기 클라이언트 즉시 종료를 검증했다고 주장하지 않는다. 테스트의 무효한 CRLF replace 제거 제안은 동작에 영향 없는 정리이므로 이번 수정 범위에서 제외한다.
- 코드: [src/adapter/inbound/admin_api/api.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/inbound/admin_api/api.rs), [src/adapter/inbound/admin_api/tests.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/inbound/admin_api/tests.rs).
- 재검토 조건: 송신 버퍼 포화와 shutdown을 제어한 재현에서 정해진 최종 종료 기한을 넘으면 서버/연결 deadline 검토.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2396

#### stale clipboard staged 파일 정리 및 submit 경계 확대

- 판단: **반려** (반려).
- 리뷰: [7029536364](https://github.com/RefinedStone/codex-exec-loop/pull/2396#issuecomment-7029536364).
- 근거: 현재 stale 결과는 generation/transcript/Ready 검증에서 버리지만 staged 파일 소유권 토큰을 전달하지 않아 파일 정리는 미구현 상태다. 경로만으로 삭제하는 방식은 사용자 원본을 지울 수 있다. staging 생성·참조·폐기 소유권과 동일 스레드 submit 뒤 probe 허용 정책을 함께 정하는 별도 계약으로 두고 이번 변경에 추가하지 않는다.
- 코드: [src/adapter/inbound/tui/app/shell_controller.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/inbound/tui/app/shell_controller.rs), [src/adapter/inbound/tui/app/conversation_lifecycle.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/inbound/tui/app/conversation_lifecycle.rs).
- 재검토 조건: owned staging handle과 공유 첨부 수명 규약을 도입한 뒤 stale/completion/drop 모두에서 자신이 만든 파일만 정리하는 회귀로 구현.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2395

#### 비ASCII 소켓 경로를 보존하는 로캘로 PID를 검사하세요

- 판단: **기대응** (기대응).
- 리뷰: [4624487663](https://github.com/RefinedStone/codex-exec-loop/pull/2395#discussion_r4624487663).
- 근거: Linux는 /proc/<pid>/cmdline, macOS는 커널 argv를 사용하고 인자 경계로 app-server와 --listen socket을 대조한다. 비ASCII 경로를 훼손하는 ps 로캘에 의존하지 않는다.
- 코드: [src/adapter/outbound/app_server/gateway_upstream.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/app_server/gateway_upstream.rs).
- 기존 수정: [f6e64892af2b](https://github.com/RefinedStone/codex-exec-loop/commit/f6e64892af2b9f79d6a8bee5e5ef66add3f2a145) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### C 로캘의 비ASCII socket 훼손 및 실제 조회 경로 검증

- 판단: **기대응** (기대응).
- 리뷰: [7029535662](https://github.com/RefinedStone/codex-exec-loop/pull/2395#issuecomment-7029535662).
- 근거: 운영체제별 원본 argv 조회와 정확한 인자 비교로 수정되어 locale에 의한 훼손 경로가 사라졌다. 원본 inline 지적과 동일 원인이다.
- 코드: [src/adapter/outbound/app_server/gateway_upstream.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/app_server/gateway_upstream.rs).
- 기존 수정: [f6e64892af2b](https://github.com/RefinedStone/codex-exec-loop/commit/f6e64892af2b9f79d6a8bee5e5ef66add3f2a145) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2394

#### Ready 전 첨부 제거·실패 후 재제출 추가 순열

- 판단: **반려** (반려).
- 리뷰: [7029535155](https://github.com/RefinedStone/codex-exec-loop/pull/2394#issuecomment-7029535155).
- 근거: 현재 텍스트와 이미지가 모두 비어야 예약을 취소하고 준비 전에 예약을 소비하므로 반복 Ready 중복 제출은 막는다. 제안된 상태 순열은 비차단 추가 시험이며 현재 재현되지 않은 동작 변경을 넣지 않는다.
- 코드: [src/adapter/inbound/tui/app/turn_submission_runtime.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/inbound/tui/app/turn_submission_runtime.rs).
- 재검토 조건: 입력 보존/중복 submit에 대한 실제 반례가 나오면 fake port의 제출 횟수로 검증.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2393

#### 추가 child argv 표현·비UTF-8 순열 시험

- 판단: **반려** (반려).
- 리뷰: [7029530271](https://github.com/RefinedStone/codex-exec-loop/pull/2393#issuecomment-7029530271).
- 근거: 첫 --부터 OsString을 그대로 넘기고 shell 재분해를 하지 않는 현재 계약이 보존된다. 빈 실행 파일 오류·빈 문자열·두 번째 delimiter·Unix 비UTF-8 추가 coverage는 별도 확장으로 남긴다.
- 코드: [src/configuration.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/configuration.rs).
- 재검토 조건: 문자열 변환 계층이 추가되거나 실제 인자 손실 사례가 발생할 때.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2392

#### 직접 저장하는 테스트 호출부에서도 디렉터리를 생성하세요

- 판단: **기대응** (기대응).
- 리뷰: [4624410204](https://github.com/RefinedStone/codex-exec-loop/pull/2392#discussion_r4624410204).
- 근거: write_document 진입 시 ensure_private_directory를 호출하여 직접 저장하는 fresh-home 테스트 경로도 임시 파일을 만들 디렉터리를 준비한다.
- 코드: [src/adapter/outbound/mcp_hub/mod.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/mod.rs).
- 기존 수정: [64c9abd4dba2](https://github.com/RefinedStone/codex-exec-loop/commit/64c9abd4dba2bc50836eacd065e986a27bae7fe3) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### fresh-home 저장 디렉터리 생성

- 판단: **기대응** (기대응).
- 리뷰: [7029529736](https://github.com/RefinedStone/codex-exec-loop/pull/2392#issuecomment-7029529736).
- 근거: write_document에서 private directory를 먼저 준비한다. 추가 일반 리뷰에서도 이미 반영되었다고 확인한 항목이다.
- 코드: [src/adapter/outbound/mcp_hub/mod.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/mod.rs).
- 기존 수정: [64c9abd4dba2](https://github.com/RefinedStone/codex-exec-loop/commit/64c9abd4dba2bc50836eacd065e986a27bae7fe3) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### rename 후 fsync 실패 의미·프로세스 잠금 대기 정책

- 판단: **반려** (반려).
- 리뷰: [7029529736](https://github.com/RefinedStone/codex-exec-loop/pull/2392#issuecomment-7029529736).
- 근거: 현재 rename 이후 directory sync 실패는 새 문서 설치 가능성을 가진 Err다. 단계별 fault injection이나 불확실한 결과 상태를 추가하는 것은 별도 장애 계약이며 실제 재현이 없는 권고로 분류한다. 저장되지 않았다고 단정하지 말고 재조회해야 하며 이번 코드에서 원자 롤백을 추가하지 않는다. write_document의 fresh-home 호출자 주석 정리도 비차단 문구 개선으로 이번 수정 범위에서 제외한다.
- 코드: [src/adapter/outbound/mcp_hub/mod.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/mod.rs).
- 재검토 조건: fsync 단계별 실패 재현과 catalog invalidation/재조회 결과 표현을 함께 설계할 때.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2391

#### #2390 통합의 stdin 정체·stdout burst·재연결 회귀

- 판단: **기대응** (기대응).
- 리뷰: [7029529139](https://github.com/RefinedStone/codex-exec-loop/pull/2391#issuecomment-7029529139).
- 근거: 공통 deadline과 bounded stdout 처리의 통합 시험이 추가되어 있다. 후속 duplex 수정은 쓰기 완료 대기 중 stdout도 drain한다.
- 코드: [src/adapter/outbound/mcp_hub/session.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/session.rs).
- 기존 수정: [027df0f0686c](https://github.com/RefinedStone/codex-exec-loop/commit/027df0f0686c35e14db35fef7d61817762441f40) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### resident mutex를 포함한 사용자 요청 전체 deadline

- 판단: **반려** (반려).
- 리뷰: [7029529139](https://github.com/RefinedStone/codex-exec-loop/pull/2391#issuecomment-7029529139).
- 근거: 현재 한도는 세션 RPC 하나이며 resident mutex 대기나 여러 RPC의 총시간이 아니다. 전체 사용자 요청 120초 보장과 admission 상한은 현재 계약에 없으므로 이번 수정에서 확대하지 않는다.
- 코드: [src/adapter/outbound/mcp_hub/mod.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/mod.rs), [src/adapter/outbound/mcp_hub/session.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/session.rs).
- 재검토 조건: UI/서비스가 전체 요청 기한을 약속하게 될 때 admission deadline·동시 요청 상한과 함께 구현.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2390

#### stdout burst·긴 stderr·stdout EOF의 수명 분리

- 판단: **기대응** (기대응).
- 리뷰: [7029528453](https://github.com/RefinedStone/codex-exec-loop/pull/2390#issuecomment-7029528453).
- 근거: stdout은 bounded blocking send로 backpressure를 가하고 stderr는 제한된 tail만 보관하며 계속 drain한다. stderr는 stdout sender를 소유하지 않아 stdout EOF를 즉시 관찰한다. 후속 duplex 처리도 반영되어 있다.
- 코드: [src/adapter/outbound/mcp_hub/session.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/session.rs).
- 기존 수정: [f6b91a60528c](https://github.com/RefinedStone/codex-exec-loop/commit/f6b91a60528c8a05a0cd5787d940a57429517f49) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2389

#### MCP 설정에서 명시적으로 참조하는 환경 변수는 보존하세요

- 판단: **기대응** (기대응).
- 리뷰: [4624308169](https://github.com/RefinedStone/codex-exec-loop/pull/2389#discussion_r4624308169).
- 근거: hub 설정의 bearer_token_env_var, env_vars, env_http_headers에서 명시적으로 참조한 변수만 공통 scrub 뒤 복원한다. 설정에 없는 자격 증명은 제거하며 실제 fake session 경로 시험이 추가되어 있다.
- 코드: [src/adapter/outbound/mcp_hub/session.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/session.rs), [src/adapter/outbound/mcp_hub/mod.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/mod.rs).
- 기존 수정: [45ce725f9265](https://github.com/RefinedStone/codex-exec-loop/commit/45ce725f9265f414a5ad88e237bb75d39c8b43de) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 설정에서 명시적으로 참조한 MCP 환경 변수 보존

- 판단: **기대응** (기대응).
- 리뷰: [7029527968](https://github.com/RefinedStone/codex-exec-loop/pull/2389#issuecomment-7029527968).
- 근거: 신뢰된 hub 설정의 세 종류 변수 참조만 복원하고 나머지 credential scrub과 private CODEX_HOME을 유지하는 실제 session 경로 검증이 있다.
- 코드: [src/adapter/outbound/mcp_hub/session.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/session.rs), [src/adapter/outbound/mcp_hub/mod.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/mod.rs).
- 기존 수정: [45ce725f9265](https://github.com/RefinedStone/codex-exec-loop/commit/45ce725f9265f414a5ad88e237bb75d39c8b43de) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2388

#### FIFO writer 있음·descriptor 누수 추가 조합 시험

- 판단: **반려** (반려).
- 리뷰: [7029527441](https://github.com/RefinedStone/codex-exec-loop/pull/2388#issuecomment-7029527441).
- 근거: O_NONBLOCK 뒤 동일 fd 정규파일/identity 검증을 유지하므로 writer 없는 FIFO의 선행 open 정체 원인은 해결된다. 추가 객체 조합과 실패 진단은 비차단 coverage이며 현재 새 반례 없이 범위를 넓히지 않는다.
- 코드: [src/adapter/outbound/filesystem/secure_fs.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/filesystem/secure_fs.rs).
- 재검토 조건: read API의 다른 객체 처리 변경 또는 fd 누수 재현이 생기면 추가.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2387

#### 구형 baseline 재-export의 편집본 보존 UX

- 판단: **반려** (반려).
- 리뷰: [7029539034](https://github.com/RefinedStone/codex-exec-loop/pull/2387#issuecomment-7029539034).
- 근거: workspace_root+relative_path 키로 worktree가 분리되며 구형 baseline은 자동 채택하지 않는다. 재-export는 candidate를 덮어쓸 수 있으므로 편집본 보관이 필요하다. dirty candidate 보호와 자동 migration은 별도 전환/복구 정책으로 남기고 이번에 자동 덮어쓰기나 DB 키 회귀 수정에 섞지 않는다.
- 코드: [src/adapter/outbound/db/sqlite_planning_authority_adapter/draft_files.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/db/sqlite_planning_authority_adapter/draft_files.rs), [src/application/service/planning/admin/file_sync.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/planning/admin/file_sync.rs).
- 재검토 조건: 재-export 전 백업/복사 안내와 dirty candidate 처리 UX를 별도 완료 기준으로 정할 때. 현재 사용자 편집본은 먼저 보관해야 한다.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2386

#### 호출자 조합 execution snapshot 일관성

- 판단: **기대응** (기대응).
- 리뷰: [7029538449](https://github.com/RefinedStone/codex-exec-loop/pull/2386#issuecomment-7029538449).
- 근거: 원본 task/direction 읽기 트랜잭션은 유지되며 실제 함께 소비하는 workspace·revision 원자 계약은 #2401 후속 수정에서 load_planning_execution_snapshot으로 연결되었다.
- 코드: [src/adapter/outbound/db/sqlite_planning_authority_adapter.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/db/sqlite_planning_authority_adapter.rs), [src/application/service/planning/repair/reconciliation.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/planning/repair/reconciliation.rs).
- 기존 수정: [1b757f9c3476](https://github.com/RefinedStone/codex-exec-loop/commit/1b757f9c347677971f08e3c7942d884e2541f046) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 문서 부재·실제 queue 내용 추가 회귀

- 판단: **반려** (반려).
- 리뷰: [7029538449](https://github.com/RefinedStone/codex-exec-loop/pull/2386#issuecomment-7029538449).
- 근거: 현재 개별 authority snapshot은 한 읽기 트랜잭션이며 호출자 혼합 문제도 별도 수정되었다. 추가 데이터 조합 coverage는 비차단 권고라 이번에 확대하지 않는다.
- 코드: [src/adapter/outbound/db/sqlite_planning_authority_adapter.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/db/sqlite_planning_authority_adapter.rs).
- 재검토 조건: snapshot schema/port가 바뀌거나 본문·revision 불일치 반례가 발견되면 추가.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2385

#### ..notes와 symlink/junction의 추가 조합 시험

- 판단: **반려** (반려).
- 리뷰: [7029524796](https://github.com/RefinedStone/codex-exec-loop/pull/2385#issuecomment-7029524796).
- 근거: lexical 판정은 실제 .. 세그먼트·절대 경로만 거절하며 실제 대상 검증은 gitStatus의 descriptor/realpath 경계다. #2404의 FIFO 보완까지 이미 반영되었다. 새 경로 우회 재현이 없는 추가 조합은 이번 수정에서 확대하지 않는다.
- 코드: [desktop/src/main/gitStatus.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/gitStatus.ts), [desktop/src/main/gitHunks.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/gitHunks.ts).
- 재검토 조건: Windows 드라이브/UNC 또는 junction 우회 사례가 확인되면 해당 OS에서 회귀 추가.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2384

#### 다중 프로세스 잠금 및 browser 전체 설정 대체 정책 변경

- 판단: **반려** (반려).
- 리뷰: [7029524310](https://github.com/RefinedStone/codex-exec-loop/pull/2384#issuecomment-7029524310).
- 근거: pendingSaves는 고정 userData 경로를 사용하는 단일 main의 요청을 직렬화하며 마지막 제출 전체 설정 우선 정책이 명시되어 있다. 다중 인스턴스/별칭 경로 잠금과 필드별 병합은 별도 제품 계약이므로 현재 저장 경쟁 수정에 추가하지 않는다.
- 코드: [desktop/src/main/browserSettings.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/browserSettings.ts).
- 재검토 조건: 다중 앱 인스턴스 또는 창별 browser 설정 병합 지원이 결정될 때.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2382

#### 반복 close·후속 onState·정상 backoff 추가 coverage

- 판단: **반려** (반려).
- 리뷰: [7029523827](https://github.com/RefinedStone/codex-exec-loop/pull/2382#issuecomment-7029523827).
- 근거: close handler는 closed를 daemon 시작과 scheduleReconnect 전에 확인한다. 실제 취소된 handshake 재현을 막는 기존 시험이 있다. 나머지 순열은 비차단 추가 시험이며 새 동작을 넣지 않는다.
- 코드: [desktop/src/main/gatewayConnection.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/gatewayConnection.ts).
- 재검토 조건: 취소 뒤 callback/reconnect가 남는 실제 재현이 생길 때.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2381

#### unborn index 비교 UI 표기 및 첫 commit 전환 추가

- 판단: **반려** (반려).
- 리뷰: [7029523272](https://github.com/RefinedStone/codex-exec-loop/pull/2381#issuecomment-7029523272).
- 근거: HEAD 없음에서는 --cached로 index 대 빈 트리를 읽고 HEAD 있는 경로는 그대로다. staged 내용만 보여주는 동작은 기존 완료 계약에 맞는다. 비교 대상 UI 확대와 commit 직후 추가 시나리오는 비차단 권고로 이번 범위에서 제외한다.
- 코드: [desktop/src/main/gitStatus.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/gitStatus.ts).
- 재검토 조건: 사용자에게 worktree 전체 diff로 잘못 표기되는 구체 경로나 HEAD 전환 오류가 확인되면 진행.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2380

#### 이전 턴 응답의 유효성을 transcript 객체 대신 탐색 세대로 판단하세요

- 판단: **기대응** (기대응).
- 리뷰: [4623795175](https://github.com/RefinedStone/codex-exec-loop/pull/2380#discussion_r4623795175).
- 근거: loadOlderTurns가 conversationGeneration, threadId, cursor를 캡처하고 같은 세대와 커서이면 최신 transcript에 병합한다. streaming으로 객체가 바뀌는 것은 더 이상 폐기 조건이 아니다.
- 코드: [desktop/src/renderer/session/desktopSession.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/session/desktopSession.ts).
- 기존 수정: [1355a2d92553](https://github.com/RefinedStone/codex-exec-loop/commit/1355a2d92553f4a2123694a5ecd6cd427c60a4fc) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### streaming 중 이전 턴 응답 폐기 및 ABA 탐색

- 판단: **기대응** (기대응).
- 리뷰: [7029522694](https://github.com/RefinedStone/codex-exec-loop/pull/2380#issuecomment-7029522694).
- 근거: conversationGeneration/threadId/cursor로 요청의 소유권을 확인하고 최신 transcript에 merge하므로 streaming delta를 보존하면서 구세대 ABA 응답을 버린다.
- 코드: [desktop/src/renderer/session/desktopSession.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/session/desktopSession.ts).
- 기존 수정: [1355a2d92553](https://github.com/RefinedStone/codex-exec-loop/commit/1355a2d92553f4a2123694a5ecd6cd427c60a4fc) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2379

#### 여러 unread·재연결 오프라인 완료 복원 추가 범위

- 판단: **반려** (반려).
- 리뷰: [7029522142](https://github.com/RefinedStone/codex-exec-loop/pull/2379#issuecomment-7029522142).
- 근거: 현재 active→inactive 실제 전환에서만 unread를 추가하고 명시적 열기에서 읽음 처리한다. snapshot 재연결로 오프라인 완료까지 복원하는 기능은 현재 계약에 없으며 비차단 조합 시험을 확대하지 않는다.
- 코드: [desktop/src/renderer/session/desktopStore.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/session/desktopStore.ts), [desktop/src/renderer/App.tsx](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/App.tsx).
- 재검토 조건: 오프라인 완료 복원 요구가 정해지거나 현재 읽음 항목 외 항목을 지우는 재현이 생기면 진행.
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2377

#### 체크아웃 대상 브랜치의 커밋도 기대 상태에 포함하십시오

- 판단: **기대응** (기대응).
- 리뷰: [4621007175](https://github.com/RefinedStone/codex-exec-loop/pull/2377#discussion_r4621007175), [4621187886](https://github.com/RefinedStone/codex-exec-loop/pull/2377#discussion_r4621187886).
- 근거: checkout 대상 branch의 커밋을 기대 상태에 포함한다. 계획 이후 branch가 움직이면 stale로 새 계획을 반환하는 회귀 시험이 있다. 원본 작성자 대응·resolve 완료.
- 코드: [src/application/service/git_operation.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/git_operation.rs).
- 기존 수정: [0ab288e1938b](https://github.com/RefinedStone/codex-exec-loop/commit/0ab288e1938b520fcc0e8ff87fcc86e2dc3684cc) (원래 작성자 대응 식별자이며, 현재 코드 근거와 구분한다).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 작업 트리를 바꾸는 되돌리기는 기록된 작업 트리에 한정하십시오

- 판단: **기대응** (기대응).
- 리뷰: [4621007184](https://github.com/RefinedStone/codex-exec-loop/pull/2377#discussion_r4621007184), [4621188127](https://github.com/RefinedStone/codex-exec-loop/pull/2377#discussion_r4621188127).
- 근거: reset/checkout undo는 기록 worktree와 요청 worktree가 같을 때만 허용한다. ref만 바꾸는 branch 이동과 구분한다. 원본 작성자 대응·resolve 완료.
- 코드: [src/application/service/git_operation.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/git_operation.rs).
- 기존 수정: [0ab288e1938b](https://github.com/RefinedStone/codex-exec-loop/commit/0ab288e1938b520fcc0e8ff87fcc86e2dc3684cc) (원래 작성자 대응 식별자이며, 현재 코드 근거와 구분한다).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 사용자 전역 설정의 줄 끝 값도 안전하게 복원하십시오

- 판단: **기대응** (기대응).
- 리뷰: [4621007192](https://github.com/RefinedStone/codex-exec-loop/pull/2377#discussion_r4621007192), [4621188442](https://github.com/RefinedStone/codex-exec-loop/pull/2377#discussion_r4621188442).
- 근거: 허용된 줄 끝 설정만 repository→global→system 우선순위로 읽고 명령에 적용한다. 전역 환경 전체를 작업 명령에 다시 노출하지 않는다. 원본 작성자 대응·resolve 완료.
- 코드: [src/adapter/outbound/git/workspace.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/git/workspace.rs).
- 기존 수정: [0ab288e1938b](https://github.com/RefinedStone/codex-exec-loop/commit/0ab288e1938b520fcc0e8ff87fcc86e2dc3684cc) (원래 작성자 대응 식별자이며, 현재 코드 근거와 구분한다).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 자체 10건 리뷰와 3개 bot 대응

- 판단: **기대응** (기대응).
- 리뷰: [6992341076](https://github.com/RefinedStone/codex-exec-loop/pull/2377#issuecomment-6992341076).
- 근거: 원본 일반댓글 6992341076에 항목별 수용·반려 이유가 이미 남아 있고 bot 3건도 resolve되었다. 자동 Git 부분 실패 복구·Gateway turn 소유권·측정 전 성능 축소는 미구현 범위를 plan 27 §9.3에 명시한 기존 판단으로, 무응답으로 분류하지 않는다.
- 코드: [src/application/service/git_operation.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/git_operation.rs), [src/adapter/outbound/git/workspace.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/git/workspace.rs).
- 기존 수정: [c7ff0c60f119](https://github.com/RefinedStone/codex-exec-loop/commit/c7ff0c60f119be97da2596e750b89b7e9be82b20) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2376

#### 중간 dot 응답 뒤 live disconnect에서 최종 답 조회 중단

- 판단: **기대응** (기대응).
- 리뷰: [7029521639](https://github.com/RefinedStone/codex-exec-loop/pull/2376#issuecomment-7029521639).
- 근거: nextDotReplyWait가 중간 메시지 수신과 턴 완료 관찰을 분리하고 disconnect 뒤 bounded fallback 읽기를 유지한다. 실제 fake Gateway E2E에서 최종 답이 GET으로 나타나는 시나리오가 현재 전체 실행에서도 통과했다.
- 코드: [desktop/src/renderer/views/DotChat.tsx](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/views/DotChat.tsx), [desktop/src/renderer/views/dotSend.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/views/dotSend.ts), [desktop/test/e2e/remote-host.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/test/e2e/remote-host.mjs).
- 기존 수정: [a38ffcd0a307](https://github.com/RefinedStone/codex-exec-loop/commit/a38ffcd0a3078ec3763d8465300f5eb1ac5f15a6) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2374

#### 자체 10건 리뷰 및 CI 클릭 대상 대응

- 판단: **기대응** (기대응).
- 리뷰: [6976590104](https://github.com/RefinedStone/codex-exec-loop/pull/2374#issuecomment-6976590104).
- 근거: 원본 일반댓글 6976590104에 빈 검색, 동시 load, scroll anchor, generation, full SHA, diff flags, stdout limit, 구독 범위 등의 수정과 두 반려 사유가 이미 기록되어 있다. stateless 비용은 측정 후 개선으로 남긴 명시적 판단이며 무응답이 아니다.
- 코드: [src/application/service/git_graph.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/git_graph.rs), [desktop/src/renderer/git/gitGraph.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/git/gitGraph.ts).
- 기존 수정: [65fe6a764876](https://github.com/RefinedStone/codex-exec-loop/commit/65fe6a7648769dd038aa1aaee60eaeed31e592fc) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2373

#### 커밋을 읽는 동안 ref가 바뀌었는지도 확인하세요

- 판단: **수용** (후속 PR 생성).
- 리뷰: [4611286152](https://github.com/RefinedStone/codex-exec-loop/pull/2373#discussion_r4611286152).
- 근거: 실제 history 및 backup 분류 뒤 refs digest를 재검사하고 변경되면 페이지·커서를 폐기한다. 첫/다음 페이지와 조회 중 저장소 제거 회귀를 추가했다. Desktop 첫 페이지도 최대 3회 재시도하고 stale을 loaded로 변환하지 않아 background 갱신 중 기존 그래프를 유지한다.
- 코드: [src/application/service/git_graph.rs](https://github.com/RefinedStone/codex-exec-loop/blob/47d3a66c1ebad60ae20e5357be98443efcc34053/src/application/service/git_graph.rs), [desktop/src/renderer/git/gitGraph.ts](https://github.com/RefinedStone/codex-exec-loop/blob/47d3a66c1ebad60ae20e5357be98443efcc34053/desktop/src/renderer/git/gitGraph.ts), [desktop/src/renderer/git/GitGraphPanel.tsx](https://github.com/RefinedStone/codex-exec-loop/blob/47d3a66c1ebad60ae20e5357be98443efcc34053/desktop/src/renderer/git/GitGraphPanel.tsx), [desktop/test/gitGraph.test.ts](https://github.com/RefinedStone/codex-exec-loop/blob/47d3a66c1ebad60ae20e5357be98443efcc34053/desktop/test/gitGraph.test.ts), [desktop/test/e2e/git-graph.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/47d3a66c1ebad60ae20e5357be98443efcc34053/desktop/test/e2e/git-graph.mjs).
- 후속 PR: [#2491](https://github.com/RefinedStone/codex-exec-loop/pull/2491).
- 검증: cargo test --locked --all-features --lib --target x86_64-pc-windows-msvc git_graph::tests::: 18개 성공; planner Rust; Windows Rust gate·clippy 성공; cargo fmt --all; git diff --check 성공; Desktop gitGraph unit 26개 성공(새 3개는 수정 전 실패); 최종 혼합 변경 Full gate 성공; 최종 npm run e2e 전체 성공; 초기 stale 및 반복 stale DOM/스크롤 보존 검증.

### PR #2371

#### 중첩 설정도 변경한 필드 단위로 병합하세요

- 판단: **수용** (후속 PR 생성).
- 리뷰: [4610252343](https://github.com/RefinedStone/codex-exec-loop/pull/2371#discussion_r4610252343).
- 근거: main 최신 authority와 renderer 낙관적 사본 모두 중첩 필드 단위로 병합하고 UI/bridge는 바꾼 필드만 전송한다. stale 창 사본이 다른 창의 변경을 지우지 않는다.
- 코드: [desktop/src/shared/settings.ts](https://github.com/RefinedStone/codex-exec-loop/blob/3eccf720d89cd610137ecf971684e4f2c8412c84/desktop/src/shared/settings.ts), [desktop/src/main/index.ts](https://github.com/RefinedStone/codex-exec-loop/blob/3eccf720d89cd610137ecf971684e4f2c8412c84/desktop/src/main/index.ts), [desktop/src/shared/bridge.ts](https://github.com/RefinedStone/codex-exec-loop/blob/3eccf720d89cd610137ecf971684e4f2c8412c84/desktop/src/shared/bridge.ts), [desktop/src/renderer/session/settingsStore.ts](https://github.com/RefinedStone/codex-exec-loop/blob/3eccf720d89cd610137ecf971684e4f2c8412c84/desktop/src/renderer/session/settingsStore.ts), [desktop/src/renderer/views/SettingsDialog.tsx](https://github.com/RefinedStone/codex-exec-loop/blob/3eccf720d89cd610137ecf971684e4f2c8412c84/desktop/src/renderer/views/SettingsDialog.tsx).
- 후속 PR: [#2499](https://github.com/RefinedStone/codex-exec-loop/pull/2499).
- 검증: appSettings.test.ts: 7개 성공; planner Admin; Windows Admin gate 성공; npm run e2e 전체 등록 spec 성공; 실제 두 창 중첩 설정 저장 확인; git diff --check 성공.

### PR #2370

#### 대소문자 변환 전 원문에 맞는 검색 범위를 계산하세요

- 판단: **수용** (후속 PR 생성).
- 리뷰: [4610159188](https://github.com/RefinedStone/codex-exec-loop/pull/2370#discussion_r4610159188).
- 근거: 소문자 검색 위치를 원문 UTF-16 시작/끝으로 매핑하고 DOM Range에 원문 위치를 적용한다. İ, emoji, 그리스어 문맥 변환과 실제 DOM 강조 회귀를 통과했다.
- 코드: [desktop/src/renderer/conversation/find.ts](https://github.com/RefinedStone/codex-exec-loop/blob/74592b0eec1d9b632f7e183f39ae27afbd92595f/desktop/src/renderer/conversation/find.ts), [desktop/src/renderer/conversation/ConversationView.tsx](https://github.com/RefinedStone/codex-exec-loop/blob/74592b0eec1d9b632f7e183f39ae27afbd92595f/desktop/src/renderer/conversation/ConversationView.tsx).
- 후속 PR: [#2490](https://github.com/RefinedStone/codex-exec-loop/pull/2490).
- 검증: desktopBasics.test.ts: 16개 성공; planner Admin; Windows Admin gate 성공; npm run e2e 전체 등록 spec 성공; Unicode DOM Range 검증 성공; git diff --check 성공.

#### 실패 상태가 반영된 후 종료 알림을 분류하세요

- 판단: **기대응** (기대응).
- 리뷰: [4610159193](https://github.com/RefinedStone/codex-exec-loop/pull/2370#discussion_r4610159193).
- 근거: turn 완료 처리에서 setThreadFailed를 setThreadActive(false)보다 먼저 반영하여 종료 알림이 최신 실패 상태로 분류된다.
- 코드: [desktop/src/renderer/session/desktopSession.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/session/desktopSession.ts), [desktop/src/renderer/session/attention.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/session/attention.ts).
- 기존 수정: [a6bcf69c0bf8](https://github.com/RefinedStone/codex-exec-loop/commit/a6bcf69c0bf8ed6e69e989e2e225f88f9e477a3d) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2366

#### 공지 ID와 패키지 이름을 함께 사용해 중복을 제거하세요

- 판단: **수용** (후속 PR 생성).
- 리뷰: [4608727082](https://github.com/RefinedStone/codex-exec-loop/pull/2366#discussion_r4608727082).
- 근거: audit advisory 중복 제거 키를 GHSA 단독에서 GHSA+영향 패키지로 바꿨다. 한 패키지의 예외가 같은 GHSA를 가진 다른 패키지를 숨기지 않는다.
- 코드: [scripts/npm-audit-gate.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/003ff37aa783e13fb6bfac9c17589fe0fc14e5d3/scripts/npm-audit-gate.mjs), [scripts/npm-audit-gate.test.mjs](https://github.com/RefinedStone/codex-exec-loop/blob/003ff37aa783e13fb6bfac9c17589fe0fc14e5d3/scripts/npm-audit-gate.test.mjs).
- 후속 PR: [#2485](https://github.com/RefinedStone/codex-exec-loop/pull/2485).
- 검증: node --test scripts/npm-audit-gate.test.mjs: 12개 성공; planner Full; Windows Full gate 성공(재실행); git diff --check 성공.

### PR #2363

#### 계정 교체 중 이전 키로 시작된 요청도 무효화하세요

- 판단: **기대응** (기대응).
- 리뷰: [4607993821](https://github.com/RefinedStone/codex-exec-loop/pull/2363#discussion_r4607993821), [4608068332](https://github.com/RefinedStone/codex-exec-loop/pull/2363#discussion_r4608068332).
- 근거: Linear connect 시작뿐 아니라 새 자격 증명 저장 성공 시점에도 invalidate()를 호출한다. 검증 중 이전 계정으로 시작한 pendingSummary/캐시를 폐기한다.
- 코드: [desktop/src/main/linearService.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/linearService.ts).
- 기존 수정: [4e139bbb625e](https://github.com/RefinedStone/codex-exec-loop/commit/4e139bbb625e8b701a7a6248481d94dcee0a9e81) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2362

#### browser_wait의 요청 제한 시간보다 먼저 호출을 취소하지 마세요

- 판단: **기대응** (기대응).
- 리뷰: [4607529033](https://github.com/RefinedStone/codex-exec-loop/pull/2362#discussion_r4607529033), [4607749733](https://github.com/RefinedStone/codex-exec-loop/pull/2362#discussion_r4607749733).
- 근거: browser_wait 서비스/브로커/전송 예산이 각각 55/60/65초로 정렬되어 45초 대기와 후속 처리를 수용한다. 원본 작성자 제어 시계 회귀 검증·resolve 완료.
- 코드: [desktop/src/main/browserAgentService.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/browserAgentService.ts).
- 기존 수정: [2301ca26a858](https://github.com/RefinedStone/codex-exec-loop/commit/2301ca26a858d8537c5a28ae56e64b905d49ade2) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 일시 정지 시 종료된 브리지의 unavailable 상태를 보존하세요

- 판단: **기대응** (기대응).
- 리뷰: [4607529038](https://github.com/RefinedStone/codex-exec-loop/pull/2362#discussion_r4607529038), [4607751915](https://github.com/RefinedStone/codex-exec-loop/pull/2362#discussion_r4607751915).
- 근거: suspend가 unavailable 상태와 종료 사유를 보존하고 resumeRegistration이 해당 종료 상태를 거절한다. 원본 작성자 상태 전이 회귀 검증·resolve 완료.
- 코드: [desktop/src/main/browserAgentService.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/main/browserAgentService.ts).
- 기존 수정: [2301ca26a858](https://github.com/RefinedStone/codex-exec-loop/commit/2301ca26a858d8537c5a28ae56e64b905d49ade2) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2361

#### 캐시 미스 조회를 실제로 대상 서버에 한정하세요

- 판단: **기대응** (기대응).
- 리뷰: [4606969388](https://github.com/RefinedStone/codex-exec-loop/pull/2361#discussion_r4606969388), [4607089453](https://github.com/RefinedStone/codex-exec-loop/pull/2361#discussion_r4607089453).
- 근거: catalog miss는 다른 서버를 끈 ephemeral thread를 만들고 공식 threadId/detail=toolsAndAuthOnly로 statuses_from을 호출한다. 지원하지 않는 serverName 필터를 사용하지 않는다.
- 코드: [src/adapter/outbound/mcp_hub/catalog.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/catalog.rs), [src/adapter/outbound/mcp_hub/mod.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/mod.rs), [src/adapter/outbound/mcp_hub/catalog/tests.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/catalog/tests.rs).
- 기존 수정: [4e9146e856cf](https://github.com/RefinedStone/codex-exec-loop/commit/4e9146e856cfe465df2a16ae42c1a618c2edda75) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2360

#### ABA 전송 중 새 초안 삭제·중복 전송 및 overlay 통합

- 판단: **기대응** (기대응).
- 리뷰: [7029516316](https://github.com/RefinedStone/codex-exec-loop/pull/2360#issuecomment-7029516316).
- 근거: composerAction이 component 밖 thread key별 pending을 소유하고 전송 당시 초안과 현재 초안이 같은 경우만 삭제한다. uiStore overlay 통합과 충돌 해소도 최신 prerelease에 반영되어 있다.
- 코드: [desktop/src/renderer/conversation/composerAction.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/conversation/composerAction.ts), [desktop/src/renderer/conversation/Composer.tsx](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/conversation/Composer.tsx), [desktop/src/renderer/session/uiStore.ts](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/desktop/src/renderer/session/uiStore.ts).
- 기존 수정: [d41daf45b166](https://github.com/RefinedStone/codex-exec-loop/commit/d41daf45b166880be3c516447de8a1f5331d3e51) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2359

#### 정상 응답의 null isError를 허용하세요

- 판단: **기대응** (기대응).
- 리뷰: [4606589303](https://github.com/RefinedStone/codex-exec-loop/pull/2359#discussion_r4606589303), [4606624287](https://github.com/RefinedStone/codex-exec-loop/pull/2359#discussion_r4606624287).
- 근거: ToolResult.is_error가 Option<bool>이고 Some(true)만 오류로 취급하여 명시적 null 정상 응답을 허용한다.
- 코드: [src/adapter/outbound/mcp_hub/notion_resource_version.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/mcp_hub/notion_resource_version.rs).
- 기존 수정: [f44ff2b5bb60](https://github.com/RefinedStone/codex-exec-loop/commit/f44ff2b5bb60473ca04d62cb5dfa7e7d1138ba9e) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2355

#### 낡은 revision으로 거절된 명령도 planning 갱신 뒤에 응답하세요

- 판단: **기대응** (기대응).
- 리뷰: [4605533824](https://github.com/RefinedStone/codex-exec-loop/pull/2355#discussion_r4605533824), [4606493762](https://github.com/RefinedStone/codex-exec-loop/pull/2355#discussion_r4606493762).
- 근거: #2358의 후속 수정이 머지되었다. 거절 결과의 planning_revision도 stored_change_being_read가 잡고 호스트 planning이 도달하거나 5초 마감이 지나야 응답한다. 원본에 작성자 답글이 이미 있다.
- 코드: [src/composition/kernel_host.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/composition/kernel_host.rs).
- 기존 수정: [758ec6f0842a](https://github.com/RefinedStone/codex-exec-loop/commit/758ec6f0842aa4347dbdaaf58e35ef951f0162fd) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

### PR #2331

#### 저장 명령의 응답 전에 planning 갱신을 완료하세요

- 판단: **기대응** (기대응).
- 리뷰: [4560573221](https://github.com/RefinedStone/codex-exec-loop/pull/2331#discussion_r4560573221), [4598752584](https://github.com/RefinedStone/codex-exec-loop/pull/2331#discussion_r4598752584).
- 근거: 초기 동기 갱신 대응 이후 #2355/#2358에서 비동기 호스트가 revision에 도달할 때 답을 보내는 구조로 개선되었다. 현재 stored_change_being_read와 held 답변 계약이 성공/낡은 revision 거절을 모두 포함한다.
- 코드: [src/composition/kernel_host.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/composition/kernel_host.rs), [src/composition/native_client_runtime.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/composition/native_client_runtime.rs).
- 기존 수정: [758ec6f0842a](https://github.com/RefinedStone/codex-exec-loop/commit/758ec6f0842aa4347dbdaaf58e35ef951f0162fd) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

#### 동일 작업의 편집 guard 충돌도 planning_locked로 변환하세요

- 판단: **기대응** (기대응).
- 리뷰: [4560573228](https://github.com/RefinedStone/codex-exec-loop/pull/2331#discussion_r4560573228), [4598752964](https://github.com/RefinedStone/codex-exec-loop/pull/2331#discussion_r4598752964).
- 근거: 동일 작업 편집 claim 삽입 충돌도 PlanningMutationRefusal::locked로 변환된다. runtime_projection.rs의 해당 분기와 기존 작성자 대응을 확인했다.
- 코드: [src/adapter/outbound/db/sqlite_planning_authority_adapter/runtime_projection.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/adapter/outbound/db/sqlite_planning_authority_adapter/runtime_projection.rs), [src/application/service/stored_command.rs](https://github.com/RefinedStone/codex-exec-loop/blob/d1ee3ad0253e9aa1de102342a476dd9c4d35483e/src/application/service/stored_command.rs).
- 기존 수정: [2459191d550a](https://github.com/RefinedStone/codex-exec-loop/commit/2459191d550a632bccba0d78f5dde05fb9ebb849) (조사 기준의 조상 커밋).
- 검증: 현재 audit base의 코드·기존 작성자 대응·후속 커밋 대조.

## 답글 및 스레드 상태 처리

새 수정이 아직 미머지인 원본 스레드는 수용/후속 PR 답글을 남기고 미해결 상태를 유지한다. 이미 기준 코드에 수정된 스레드만 근거를 확인한 뒤 해결 처리한다. 이미 답글과 해결 처리가 된 스레드는 중복 변경하지 않는다.

원본 답글과 해결 처리 결과는 최종 갱신 시 decisions.json의 replyActions에 기록한다.
