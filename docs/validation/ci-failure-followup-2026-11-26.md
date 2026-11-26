# 2026-11-26 선머지 CI 실패 후속 인수

대상: [#2749 CI 실패 총괄](https://github.com/RefinedStone/codex-exec-loop/issues/2749).
인수 기준은 `prerelease@d960d7d62e810f8cd94c7aa78918b0ba0104b6d6`이며, 날짜는 한국 시간 기준이다.

## 판정과 증거의 범위

**#2749의 다섯 후속 이슈는 모두 `closed/completed`이고, 수정 PR은 기준 커밋에 반영됐다.**
마지막 잔여였던 #2746의 macOS 실행도 후속 댓글에 기록되어 있으므로, 총괄의 이전
“macOS 실행 대기” 상태를 정정하고 이 범위의 종료를 권고한다. 이 문서가 제안된 PR을 만드는
것만으로 총괄 이슈를 닫지는 않는다.

이번 인수는 이슈 상태, PR의 병합 상태와 반영 커밋의 조상 관계, 이미 남아 있는 검증 기록을
대조한 것이다. **CI 상태·Actions 로그를 새로 조회하거나 CI·시험을 재실행하지 않았다.**
아래 통과 수치는 링크한 이전 실행 기록이며, 기준 커밋에서 이번에 전체 시험을 다시 통과했다는
뜻이 아니다. 과거 실패 원인이 확인되지 않은 경우도 진단·동기화 보강의 완료와 구분한다.

## 후속 이슈별 완료 근거

| 이슈 / 확인한 상태 | 반영 PR / 커밋 | 구현·보호 계약 | 기존 검증 기록과 한계 |
| --- | --- | --- | --- |
| [#2744](https://github.com/RefinedStone/codex-exec-loop/issues/2744) / completed | [#2758](https://github.com/RefinedStone/codex-exec-loop/pull/2758), `b45f69c405c66d53be6e659150a85b23b314034c` | client 없는 재시작의 runtime·discovery·저장·host mailbox 단계 동기화와 안전한 진단. durable 10초·host 5초 watchdog, 작업·permit 1회 유지 | Windows 집중 23개와 Rust gate 104개 계약 묶음 통과 기록. 저장 adapter 진입 지연을 제어했으며, 과거 CI 지연 단계나 실제 SQLite BUSY를 재현했다는 주장은 하지 않음 |
| [#2745](https://github.com/RefinedStone/codex-exec-loop/issues/2745) / completed | [#2759](https://github.com/RefinedStone/codex-exec-loop/pull/2759), `67f3b5fa0e6122855cd24a5d09affb7ecbc1a24d` | owner-only ACL 원자 생성, 기존 private 부모 ACL 비수정, 공유 부모의 안정 신원 검사. 일반 파일·store의 strict 검사와 reparse 거부 유지 | Windows 집중 35개, Full gate 105개 계약 묶음 및 Desktop 1,256개 통과·기존 skip 7개 기록. 최초 Desktop timeout과 worker 수를 줄인 재검증은 PR에 보존됨. 과거 ACL 실패의 정확한 조건까지 확정한 것은 아님 |
| [#2746](https://github.com/RefinedStone/codex-exec-loop/issues/2746) / completed | [#2757](https://github.com/RefinedStone/codex-exec-loop/pull/2757), `50b73e21a1e6fe936ffb4946362ad90256406cb4` | 초기 barrier의 예상 밖 Notice·worker·journal/RPC 단계·SQLite/native 오류 코드와 제한된 자식 진단. 초기 load 두 개, 저장 요청·적용 각 1회, 동일 키·복호화·새 프로세스 조회 유지 | Windows process 10개·Vault 13개 및 Rust gate 104개 묶음 기록에 더해, 아래 macOS 후속 기록이 추가됨. 과거 실패의 직접 원인은 미확정 |
| [#2747](https://github.com/RefinedStone/codex-exec-loop/issues/2747) / completed | [#2761](https://github.com/RefinedStone/codex-exec-loop/pull/2761), `d259b54a0602e9555fe1eadf3ab8de959742b377` | 첫 쓰기 전 one-shot gate와 실제 Core mailbox 완료 관찰. startup 부수 효과 없음, double Enter 중복 없음, 원래 2초 watchdog 유지 | 최종 rebase 후 집중 38개와 Rust gate 105개 묶음 통과 기록. 과거 실행의 실제 지연 원인을 확정하지 않음 |
| [#2748](https://github.com/RefinedStone/codex-exec-loop/issues/2748) / completed | [#2763](https://github.com/RefinedStone/codex-exec-loop/pull/2763), `d960d7d62e810f8cd94c7aa78918b0ba0104b6d6` | cleanup inventory의 설정 거부·실행 실패·timeout·비정상 종료·UTF-8·빈 출력·malformed 출력 7종 구분. 실패 시 cleanup/원격 변경 중단, branch·lease·workspace 보호 유지 | verification 10개·inventory 3개·실제 Git drift 4개와 Rust gate 105개 묶음 통과 기록. 과거 두 Git 실패의 원문이 없어 직접 원인은 미확정 |

네 선행 반영 커밋은 GitHub compare에서 기준 커밋의 조상이고, #2763의 반영 커밋은 기준
커밋 자체이다. 특정 PR의 이전 검증 SHA, rebase 후 반영 SHA, 현재 인수 기준 SHA를 혼동하지 않는다.

## #2746 macOS 후속 증거

[#2746 종료 댓글](https://github.com/RefinedStone/codex-exec-loop/issues/2746#issuecomment-7184790379)은
Darwin 25.5 / 8코어에서 다음 결과를 기록한다.

| 기록된 코드·조건 | 결과 |
| --- | --- |
| `d259b54a` (#2757 포함), `two_processes_share_one_durable_initialization_key`, 병렬 8개와 CPU 부하 | 800 / 800 통과 |
| 같은 코드, `evidence_vault_unlock::` 및 `vault_key_store::` 묶음 | 80 / 80 통과 |
| 이전 기준 + 진단, 순차·병렬·부하 반복 | 1,850 / 1,850 통과 |
| 이전 기준 + 진단, 관련 묶음 병렬·부하 실행 | 360 / 360 통과 |

“이전 기준 + 진단”의 정확한 전체 SHA는 해당 댓글에 없으므로, 두 이전 기준 행을
정확한 커밋 간 인과 비교나 원인 수정의 증명으로 사용하지 않는다. 이번에 직접 실행한 결과도 아니다.

기존 오류는 재현되지 않았다. 첫 credential load 이전 journal open/schema/snapshot 경계에서
worker가 종료했을 가능성은 여전히 가설이며 BUSY/LOCKED는 확인되지 않았다. 근거 없는 재시도,
timeout 연장 또는 초기화 검증 완화는 추가하지 않았다.
이 시험은 합성 TCP credential 서버를 사용하므로 실제 macOS Keychain·OS 사용자 승인 검증을
대신하지 않는다. 실제 OS 제품 경로는 [#2704](https://github.com/RefinedStone/codex-exec-loop/issues/2704)의 범위다.

## 총괄 상태 정정과 종료 조건

[#2749의 기존 후속 댓글](https://github.com/RefinedStone/codex-exec-loop/issues/2749#issuecomment-7184586173)에
적힌 두 차례 macOS 연결 실패는 그 시점의 사실이다. 이후 실행 근거가 생겼으므로 그 이력을
삭제하지 않고, 최신 상태가 “macOS 미실행”이 아님을 날짜·출처와 함께 정정한다.

| 총괄 종료 판단 항목 | 인수 결과 |
| --- | --- |
| 별도 추적한 실패 다섯 종류에 구현·회귀 근거가 있는가 | 위 PR·검증 기록으로 연결됨 |
| 수정이 기준 브랜치에 반영됐는가 | 다섯 PR 모두 merged이며 기준 커밋에 포함 |
| 마지막 보류 사유였던 macOS 실행이 남아 있는가 | 후속 실행 기록이 추가됐고 #2746은 completed |
| 원인 미확정·검증 한계가 보존되는가 | 위 표와 macOS 항목에 명시; 재현되지 않은 원인을 확정하지 않음 |
| 인프라 실패·성능 개선을 제품 회귀와 혼합하지 않는가 | 기존 감사의 runner 미할당 분류를 유지하고 #2690은 별도 진행 |

원래 감사에서 별도 정리한 기존 수정 9개 PR와 runner 미할당 1개 PR의 기록은
[#2749 본문](https://github.com/RefinedStone/codex-exec-loop/issues/2749)에 보존한다.
그 과거 기록을 이번 최신 CI 결과로 다시 표기하지 않는다.
CI 시간·캐시·nextest/Workspace 채택 판단은 [#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690)과
[계측 문서](../reference/ci-performance.md)의 독립 범위다.

## 재발 시 연결할 진단

재발하면 해당 하위 이슈를 다시 열거나 새 증거를 연결하고, 실패 SHA·플랫폼·정확한 시험명과
다음 진단을 먼저 보존한다. 단일 재실행 성공으로 기존 실패 원인을 단정하지 않는다.

- #2744: 마지막 runtime/discovery/store/execution 단계, revision·작업·permit 수, host 응답 오류.
- #2745: ACL 실패 API/안전한 오류 코드와 신원 비교 결과. 원시 SID·경로·ID 값을 공개 로그에 추가하지 않는다.
- #2746: 예상 밖 Notice·worker·초기화 단계·SQLite/native 오류 코드와 제한된 자식 진단. 키·원문은 기록하지 않는다.
- #2747: request ID·generation·준비 단계·마지막 안전한 이벤트. prompt 본문은 기록하지 않는다.
- #2748: `parallel_worktree_inventory_failed`의 실패 종류·종료 코드·고정 분류·byte 수. stderr 원문·경로·URL·토큰은 기록하지 않는다.

기존 timeout·시험 선택·격리·정리 거절 조건을 완화하는 것은 이 인수의 종료 방법이 아니다.
