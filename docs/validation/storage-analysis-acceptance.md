# 커널 확장과 저장 공간 분석기 첫 버전 인수

검토일: 2026-11-26. 대조 기준은 `prerelease@d960d7d62e810f8cd94c7aa78918b0ba0104b6d6`의 문서·회귀 코드와 GitHub 이슈·머지 PR이다. 이 기록은 [검증 묶음 #2621](https://github.com/RefinedStone/codex-exec-loop/issues/2621)과 [상위 #2596](https://github.com/RefinedStone/codex-exec-loop/issues/2596)의 완료 조건을 기존 증거에 연결한다.

**판정: Windows의 읽기 전용 첫 버전에 대해 두 이슈의 완료 조건을 충족한 근거가 있다.** 아래의 제한을 함께 유지하는 조건으로 완료 처리를 제안한다. 이 문서 작성 중 이슈를 닫거나 제품 시험·모델 턴·CI 조회를 새로 실행하지 않았다. 서로 다른 소스에서 얻은 기존 결과를 현재 커밋의 재실행 성공으로 합산하지 않는다.

## 1. 여섯 작업 묶음의 인수 근거

이슈 상태는 검토일의 상태다. K/S/G/D/A와 V의 하위 세 이슈는 `closed/completed`이며, V와 Epic의 상위 완료 판정만 남아 있다. 체크박스가 갱신되지 않은 원문 대신 구현 PR, 실제 실행 기록, 남은 제한을 함께 대조했다.

| 묶음 | 완료 조건과 연결된 근거 | 인수 범위 |
| --- | --- | --- |
| [K #2597](https://github.com/RefinedStone/codex-exec-loop/issues/2597) | 머지된 [#2668](https://github.com/RefinedStone/codex-exec-loop/pull/2668). [호스트 검증](kernel-extension-host.md)의 K1–K6 표에 범위별 설정, Desktop/CLI의 비활성 호출 거절, 동시 접수, 재시작·강제 종료 복구를 연결했다. | 관리 상태와 기능 결과를 구분하며, 취소 요청·워커 종료·색인 정리를 별도 상태로 관찰한다. worker는 프로세스 내부 thread이고 native crash를 별도 프로세스로 격리하지 않는다. |
| [S #2604](https://github.com/RefinedStone/codex-exec-loop/issues/2604) | 머지된 [#2669](https://github.com/RefinedStone/codex-exec-loop/pull/2669). [백엔드 기록](storage-analysis-backend.md)의 S1–S6·V1과 실제 작은 Windows fixture에서 할당량/논리량·하드링크 LCA 귀속·부분 집계·정션 제외·완료 색인 재조회 범위를 확인했다. | 같은 SQLite 색인으로 폴더·파일·요약·임계값을 조회한다. 원자적 파일 시스템 스냅숏이나 OS 전체 볼륨 사용량과의 완전한 일치를 보장하지 않는다. |
| [G #2611](https://github.com/RefinedStone/codex-exec-loop/issues/2611) | [#2669](https://github.com/RefinedStone/codex-exec-loop/pull/2669)에 이어 머지된 [#2753](https://github.com/RefinedStone/codex-exec-loop/pull/2753)이 cached stat·저장소 ignore·Git 잠금 근거를 보강했다. [G2 회귀 범위](storage-analysis-backend.md#g2의-추가-관측과-회귀-범위)에 원래 index/config/index.lock 보존과 실제 Windows 데이터 읽기 거절을 연결했다. | 활성·잠금·변경 흔적·detached·슬롯·미확인을 구분한다. 내부 캐시는 원래 노드의 바이트이며 worktree와 이중 합산하지 않는다. 내용 동일성·완전한 clean·삭제 가능 여부는 확정하지 않는다. |
| [D #2616](https://github.com/RefinedStone/codex-exec-loop/issues/2616) | 머지된 [#2670](https://github.com/RefinedStone/codex-exec-loop/pull/2670). [Desktop 기록](storage-analysis/README.md)의 lifecycle 7개 경합, storage 완료/live/복원, 키보드 행 이동·파일 선택·근거 복귀와 공유 상태를 대조했다. | 커널 상태가 권위이며 enable·패널 열기·재열기로 스캔하지 않는다. 실제 Electron 화면과 fake Gateway 시험의 범위를 구별한다. |
| [V #2621](https://github.com/RefinedStone/codex-exec-loop/issues/2621) | [V1 #2622](https://github.com/RefinedStone/codex-exec-loop/issues/2622)·[V2 #2623](https://github.com/RefinedStone/codex-exec-loop/issues/2623)·[V3 #2624](https://github.com/RefinedStone/codex-exec-loop/issues/2624) 완료. [#2752](https://github.com/RefinedStone/codex-exec-loop/pull/2752)의 [SSD/HDD 측정](storage-analysis-performance.md)이 native 부하·입력·메모리·취소 배정·threshold의 잔여 측정을 보완했다. | 정확성·수명·UI 연결·성능의 증거 수준을 아래처럼 분리한다. 기본 동시 수 1/최대 4와 기존 표시 임계값을 유지한다. |
| [A #2630](https://github.com/RefinedStone/codex-exec-loop/issues/2630) | [#2669](https://github.com/RefinedStone/codex-exec-loop/pull/2669)·[#2670](https://github.com/RefinedStone/codex-exec-loop/pull/2670). [실제 공통 SQLite 왕복](storage-analysis/README.md#v2-같은-sqlite-읽기-서비스의-desktopcli-왕복)과 [원시 값](storage-analysis/service-roundtrip.json)에서 Desktop·CLI·근거 참조가 같은 snapshot/node와 `9663676416 / 12884901888`을 사용하고 75.0% 행으로 복귀한다. | 서비스가 발급한 근거를 고정 도구 호출로 검증했다. 메타데이터 seed, 인증 fixture와 MemoryStore를 실제 12 GiB 스캔·운영 Gateway 인증 전체·모델 실행으로 해석하지 않는다. |

## 2. 상위 완료 조건 판정

| 대상의 원래 조건 | 판정과 근거 |
| --- | --- |
| #2621: 정확성·다중 창·취소 복구를 재현할 수 있다 | 충족. V1의 독립 기대값·native fixture, K의 durable 복구, D의 다중 창·늦은 응답·재연결, A의 동일 서비스 왕복에 각각 재현 명령과 실행 결과가 있다. 전체가 하나의 시험에서 실행된 것처럼 합치지 않는다. |
| #2621: 기본 동시 수·표시 임계값을 실측 결과와 기록한다 | 충족. V3는 결과 동등성을 통과한 SSD/HDD 정상 36회, 취소 12회, native와 겹친 Desktop 4회, 계층형 threshold 6회와 원자료·실행 파일/소스 해시를 보존한다. 기본 1/최대 4, 1 GiB 또는 256 MiB·부모 20%의 기존 기준 유지 결정을 기록했다. |
| #2596: 여섯 작업 묶음의 인수 조건을 확인한다 | 충족. §1의 K/S/G/D/V/A별 구현·검증과 아래 제외 범위를 연결했다. V3 이전에 남겨 둔 HDD·native 부하 UI의 미측정 문구는 후속 #2752를 반영해 현재 상태와 구별한다. |
| #2596: 분석만으로 파일과 worktree를 변경하지 않는 첫 버전을 검증한다 | 충족. 사용자 원본·Git 상태의 변경 기능을 추가하지 않고 분석기가 소유한 색인과 제한된 임시 복사본만 쓰고 정리한다. 실제 Git fixture의 원래 index/config/잠금 바이트 보존, 데이터 읽기를 거절한 Windows fixture, filter/textconv trap 비실행, 소유하지 않은 정리 대상 보존을 근거로 삼는다. §4에 정확한 경계를 적었다. |

## 3. 실행 결과의 소스와 증거 수준

| 기록 | 소스와 당시 실행 | 해석 |
| --- | --- | --- |
| K 호스트 | [#2668 검증 절](https://github.com/RefinedStone/codex-exec-loop/pull/2668)의 제품 소스 `0d639e8f595b8bb32ae2a078e22a26466cfb357a`. 확장·CLI 45개, Windows Full과 Desktop 1,068개/7 skip 등을 기록했다. | 호스트의 저장·중복 억제·복구 경계다. 실제 다른 OS 사용자나 원격 호스트를 함께 운용한 실측은 아니다. |
| S/A/V1 백엔드 | [백엔드 기록](storage-analysis-backend.md)의 `3379ca69e22bcf8fcf02a581f2ffde86ac3c19fb`: `cargo test --locked --lib storage_` 75개/2 ignored, extension 50개, Windows Full 통과. 후속 통합 SHA와 동일성 확인은 원래 기록대로 유지한다. | 결정적 집계, 작은 native fixture와 공통 서비스의 증거다. ignored benchmark를 실행 성공 개수에 넣지 않는다. |
| D/A/V2 연결 | [Desktop 기록](storage-analysis/README.md)의 `75db1f18c7c2b9e82a042975ea62b671502f5fa0`: Windows Full, 관련 화면, `npm --prefix desktop run e2e:storage-service` 통과. 전체 기본 E2E의 별도 소스 `f0eae1ff`도 구분해 보존했다. | 실제 SQLite 읽기·공유·근거·화면 왕복이며, 주입한 12/9/2/1 GiB는 의미와 표시의 fixture다. |
| V3 추가 실측 | [#2752](https://github.com/RefinedStone/codex-exec-loop/pull/2752), [측정 기록](storage-analysis-performance.md), [로컬 단계 결과](storage-analysis-performance/2026-11-26-local-validation.json). 빌드 기준 `39ac5b9c1f5d8a0d65ad50f367d461c25d9d2ece`와 수정 파일·실행 파일 해시를 함께 기록했다. | 커밋 전 수정 파일을 빌드한 결과이므로 기준 커밋 자체의 실행으로 바꾸지 않는다. 최초 Full은 Desktop 단계 실패, 해당 파일 재실행과 미도달 단계 통과를 별도 보존했다. |
| G2 추가 회귀 | [#2753](https://github.com/RefinedStone/codex-exec-loop/pull/2753)의 변경·검증 기록, PR head `2485d16d5bedd0d9e9d4a125080a44fd9a2a3147`. 저장 공간 집중 48개/2 ignored, 실제 Git·Windows fixture와 storage 화면 통과를 기록했다. | 최초 Full의 기존 Desktop Git fixture 실패와 제한 재실행의 잔여 timeout 1개를 통과로 바꾸지 않는다. source 변경 없음과 실패의 인과관계 확정도 구분한다. |

이 판정은 기존 기록과 현재 완료 조건의 대조다. 과거 PR에 기록된 CI 결과를 새로 조회하지 않았으며, 이 문서가 현재 `prerelease` 전체 시험의 성공이나 배포 승인 자체를 뜻하지 않는다. 각 기록의 실패·skip·미측정은 원래 근거에 남는다.

## 4. 읽기 전용의 정확한 경계

- **원본 분석:** 일반 파일은 attributes 기반 메타데이터를 읽는다. 파일 본문 비교·hash·삭제·이동, index refresh·fetch·worktree prune을 분석 기능으로 실행하지 않는다. reparse/offline/recall 대상은 따라가거나 내려받지 않고 미확인 사유를 남긴다.
- **Git 제어 파일:** 검증한 `.git` 경로 포인터·config·cached index와 제한된 `.gitignore`/`info/exclude` 읽기는 보호 근거를 위한 예외다. ignore 평가는 분석기 소유 임시 복사본에서 수행한다. [사용 계약](../reference/storage-analysis.md#6-gitakra-보호-근거)에 크기·개수·시간 상한과 unknown 처리를 명시한다.
- **자체 저장:** job 상태·색인 SQLite·임시 ignore 복사본의 쓰기와 정리는 허용된다. reader/worker 수명 및 소유 확인을 통과한 분석기 파일만 정리하며, 사용자 폴더의 정리 기능을 제공하는 것은 아니다.
- **실제 보존 회귀:** [기존 Git 관측 시험](../../src/adapter/outbound/storage_analysis/tests.rs)의 index 보존과 filter/diff/textconv trap, [G2 시험](../../src/adapter/outbound/storage_analysis/worktree/g2_tests.rs)의 index/config/index.lock 보존과 데이터 읽기 거절, [정리 시험](../../src/adapter/outbound/storage_analysis/evidence_tests.rs)의 나중에 생성된 비소유 파일 보존을 연결한다. 실행 근거는 §3의 해당 소스 기록이다.
- **별도 보장 아님:** 파일 시스템이 분석 도중 바뀌지 않는다는 보장, 같은 stat의 내용 동일성, 모든 클라우드 공급자의 hydration 계측, 다른 OS 사용자·외부 도구까지의 샌드박스는 포함하지 않는다. `status_unknown`과 부분 관측을 보존한다.

## 5. 목표값, 미측정과 후속 범위

[설계 §6](../plan/30-kernel-extensions-and-storage-analysis.md#6-개발-순서와-합격-기준)의 첫 실험 목표와 [V3의 기록 조건](https://github.com/RefinedStone/codex-exec-loop/issues/2624)을 구분한다. 성능 목표의 미달을 숨기거나 완료 조건을 모든 장치의 지연 보장으로 바꾸지 않는다.

| 항목 | 현재 근거와 후속 경계 |
| --- | --- |
| 최초 표시 1초 목표 | SSD native 첫 progress는 18/18, HDD는 6/18만 충족했다. HDD 12/18의 미달은 후속 성능 개선 후보다. Desktop 수락 상태 DOM 54.4–64.9 ms를 실제 native 진행이나 visible paint의 값으로 대체하지 않는다. |
| 취소 후 새 배정 중지 | 측정한 12회 모두 취소 후 신규 배정 0건. 이미 제출한 I/O와 terminal은 별도이며 HDD terminal 대기는 최대 2,392.942 ms였다. 멈춘 장치·임의 외부 취소의 모든 경합에 같은 수치를 보장하지 않는다. |
| 동시 수·임계값 | 기본 1/최대 4, 기존 threshold 유지. 두 장치의 일부 데이터에서 c4가 빨랐어도 SSD 4/HDD 1 자동 정책으로 일반화하지 않는다. |
| 규모·캐시·빌드 | 전체 볼륨·수백만 파일·통제한 cold cache·release 빌드 성능은 미측정이다. 작은 파일 8,192개/큰 파일 12개의 합성 데이터 결과와 구별한다. |
| 실제 앱 동시 부하 | native scan과 Electron 입력의 중첩은 측정했지만 시험 Gateway 전달 경로와 숨김 창이다. 실제 제품 Gateway·모델 턴 동시 부하, 보이는 화면 paint의 근거는 아니다. |
| 장치·OS | 물리 장치 분리·네트워크/이동식/클라우드 공급자별 실행과 macOS/Linux 저장 공간 분석은 이번 Windows 인수의 실측 범위 밖이다. 오류 분류 fixture를 실제 분리 시험으로 바꾸지 않는다. |
| 프로세스·권한 | 실제 disposable 호스트 강제 종료 복구와 scope 계약 회귀가 있다. native scanner의 별도 프로세스 crash 격리, 실제 다중 OS 사용자 운용은 후속 범위다. |

첫 버전의 상위 이슈를 닫을 때에는 §2의 완료 근거와 이 표를 함께 링크한다. 더 큰 데이터·실제 제품 Gateway·물리 장치·첫 native progress 최적화가 필요해지면 대상 환경과 기대값을 정한 후속 작업으로 추적한다. 이미 머지된 HDD 측정·Git 보호 구현을 다시 미구현으로 분류하거나, 이 판정을 새 기능·새 배포의 포괄 승인으로 사용하지 않는다.
