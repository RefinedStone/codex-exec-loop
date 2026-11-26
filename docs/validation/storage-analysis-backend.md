# 저장 공간 분석기 백엔드 검증 기록

이 문서는 저장 공간 분석기의 구현 계약, 이슈 댓글 반영, 재현 가능한 시험 범위와 남은 검증을 구분합니다. 사용자용 API·상태·공유 설명은 [저장 공간 분석기](../reference/storage-analysis.md), 공통 확장 호스트의 별도 검증은 [커널 확장 호스트](kernel-extension-host.md)를 참고하세요.

**검증 기준: 소스 `3379ca69`에서 backend 회귀 75개, 확장 필터 50개와 로컬 Windows Full을 통과했습니다. 후속 `d7d165bb`·K `0d639e8f` 통합은 저장 공간 구성요소의 동일성 확인과 원격 Full로 검증하고, 마지막 Windows Full은 최종 Desktop 통합 기록에서 확인합니다. V3 native 측정은 `c690571b`의 결과로 유지합니다.** 아래의 중간 성공 이력을 최종 커밋 전체의 통과로 계산하지 않습니다. 실제 모델 턴은 검증의 전제 조건에 포함하지 않습니다.

## 1. 기록 원칙과 최종 결과

[V #2621의 피드백](https://github.com/RefinedStone/codex-exec-loop/issues/2621#issuecomment-7075194380)에 따라 다음 세 증거 층을 분리합니다.

| 층 | 증명하는 범위 | 증명하지 않는 범위 |
| --- | --- | --- |
| 결정적 집계·직렬화 fixture | 정확한 정수 바이트, LCA 귀속, threshold 경계, scope·cursor·권한 경합 | 실제 장치의 할당량, Windows 파일 시스템 기능, 실제 UI 지연 |
| native 파일 시스템 fixture | 테스트가 만든 작은 트리의 실제 메타데이터·하드링크·정션·Git index 및 helper 보호 | 전체 볼륨 정확도, 모든 클라우드 provider, 물리 HDD, 실제 장치 분리 |
| 등록 호스트·Gateway·Desktop 연결 | 동일 읽기 서비스 연결, job 수명, 검증된 대화, 재연결·권한 검사 | 별도 native 스캐너 프로세스 격리, 측정하지 않은 native 부하 중 UI 반응 |

### 최종 검증 결과

| 항목 | 기록 |
| --- | --- |
| 최신 통합 제품 소스 | 고정 upstream `d7d165bbb76ae06f9c51768640e2bee6a6d721a9`를 포함한 K `0d639e8f595b8bb32ae2a078e22a26466cfb357a`를 저장 공간 브랜치에 병합했습니다. 이 통합의 정확한 제품 SHA와 원격 Full 결과는 [PR #2669 검증 절](https://github.com/RefinedStone/codex-exec-loop/pull/2669)에 기록합니다. |
| 로컬 회귀·Full source commit | `3379ca69e22bcf8fcf02a581f2ffde86ac3c19fb`. 아래의 75개/50개 집중 검사와 Desktop 1,066개를 포함한 Windows Full은 이 소스의 결과입니다. 후속 통합에서 같은 로컬 Cargo 검사를 반복한 것으로 표시하지 않습니다. |
| native 측정 source commit | `c690571b778a51e90dae8cef16c9d8a6ce5170fc`. 원시 측정 JSON의 SHA와 값은 유지합니다. 아래 소스 동일성 확인에 따라 블로그 계약 통합 후 재측정하지 않았습니다. |
| Windows/파일 시스템/장치 식별 | Windows 11 Pro 10.0.26200, NTFS, SHPP41-2000GM NVMe SSD. 2026-11-23 측정. |
| backend 집중 검사 | `3379ca69`에서 `cargo test --locked --lib storage_`: 75 passed, 0 failed, 2 ignored. ignored 두 개는 별도 실행용 benchmark입니다. |
| 확장·연결 회귀 검사 | `3379ca69`에서 `cargo test --locked --lib extension`: 50 passed, 0 failed. K 통합 소스 `0f4b26be`의 아키텍처 144개·프로토콜 32개·시뮬레이션 24개 검증은 [호스트 검증 기록](kernel-extension-host.md)에서 별도 확인합니다. |
| Windows Full | `3379ca69`에서 `powershell -File scripts/check_native_pr.ps1 -Mode Full`: exit 0. Node/Admin, Windows 계약, fmt·Clippy, Desktop 타입 검사·빌드·1,066개 테스트 통과(7 skip). |
| Desktop 확장 UI·실제 공통 서비스 왕복 | 후속 Desktop PR과 [별도 검증 기록](https://github.com/RefinedStone/codex-exec-loop/blob/codex/storage-desktop/docs/validation/storage-analysis/README.md)에 기록합니다. 이 행은 위 기존 Desktop gate와 구분합니다. |
| 원격 전체 CI | 최신 통합의 정확한 제품 SHA·Full CI 링크·최종 상태는 [PR #2669 검증 절](https://github.com/RefinedStone/codex-exec-loop/pull/2669)에 연결합니다. 위 `3379ca69`의 로컬 Full 결과와 최신 통합의 원격 검증을 구분합니다. |
| V3 세 데이터셋 반복 측정 | 18회 scan + 3회 취소 비교, benchmark 1 passed/0 failed, native process exit 0. [원시 측정값](storage-analysis-native-metrics.json). |
| 실제 모델 턴 | 실행하지 않음 |
| 실제 사용자 전체 볼륨 분석 | 이 검증을 위해 실행하지 않음 |

### native 측정 소스와 최종 통합 소스의 동일성

`c690571b`와 `3379ca69` 사이에 이어, `d7d165bb`·K `0d639e8f`를 반영한 후속 통합에서도 저장 공간 domain·service·native adapter·공유 service와 해당 composition 파일의 diff가 없음을 확인했습니다. 최신 통합 checkout에서 아래 명령은 종료 코드 0을 반환합니다. native 측정 원시 JSON과 18회 scan·3회 취소의 관측값은 측정 당시 SHA에 그대로 연결하며, 이를 새 통합 커밋에서 다시 측정한 결과로 표시하지 않습니다.

```text
git diff --exit-code c690571b HEAD -- src/domain/storage_analysis src/application/service/storage_analysis src/application/service/storage_sharing src/adapter/outbound/storage_analysis src/composition/storage_extension.rs src/composition/storage_extension src/composition/repository_kernel/storage.rs
```

### 중간 이력 — 최종 게이트가 아님

| 이력 | 확인된 사실 | 해석 제한 |
| --- | --- | --- |
| 중간 backend 통합 실행 | `46 passed, 0 failed, 1 ignored` | 당시 소스의 결과입니다. 이후 Git 메타데이터 가드·응답 경합 보호·신규 benchmark 등을 포함한 최종 SHA의 결과로 재사용하지 않습니다. |
| 기존 단일 혼합 fixture | 작은 파일 2,048개와 큰 파일 32개, 합계 2,080개로 반복 측정한 개발 이력이 있습니다. | 현재 V3의 세 독립 데이터셋 비교를 대체하지 않습니다. 여기서는 이전 시간값을 최종 성능 수치로 게시하지 않습니다. |
| 최종 측정 전 source 확인 | 등록된 시험과 재현 명령을 아래에 연결합니다. | 시험 코드가 존재하는 것과 현재 커밋에서 실행을 통과한 것은 구분합니다. |

## 2. 이슈별 충족 범위와 제한

표의 “구현 범위”는 현재 코드의 동작이며 이슈 자동 종료나 모든 출시 환경 검증 완료를 뜻하지 않습니다. 특히 G2와 V3의 남은 요구를 명시합니다.

### S — 볼륨 분석

| 이슈 | 구현으로 충족한 범위 | 제한·후속 검증 |
| --- | --- | --- |
| [S #2604](https://github.com/RefinedStone/codex-exec-loop/issues/2604) | 한 번 관측한 작업 SQLite를 폴더·파일·요약이 함께 사용합니다. 확인량과 미확인 사유를 구분합니다. | 원자적 파일 시스템 스냅숏이 아닙니다. OS 전체 볼륨 사용량의 모든 차이를 조정하지 않습니다. |
| [S1 #2605](https://github.com/RefinedStone/codex-exec-loop/issues/2605) | 안정적인 볼륨 ID, 사용자 선택 볼륨, 루트 직속 파일, attributes 기반 metadata-only 읽기, `access_denied/disappeared/drive_disconnected/volume_changed` 구분을 구현했습니다. | 실제 removable 장치 분리와 재연결, 각 Windows 파일 시스템·권한 조합은 별도 native 검증이 필요합니다. 오류 코드 분류 시험과 물리 분리 시험을 구별합니다. |
| [S2 #2606](https://github.com/RefinedStone/codex-exec-loop/issues/2606) | 1~4 디렉터리 worker, bounded event 채널, SQLite pending 큐, 취소 검사, 64개 우선순위 mailbox, 3 priority:1 FIFO 공정성, 독립 live 페이지를 연결했습니다. | host-global 직렬 scan 정책이며 SSD/HDD별 자동 예산은 아닙니다. 기본은 1입니다. 단일 평면 디렉터리의 개별 파일을 여러 worker로 나누지 않습니다. |
| [S3 #2607](https://github.com/RefinedStone/codex-exec-loop/issues/2607) | 논리·할당량 분리, 볼륨+파일 ID 중복 제거, 늦은 하드링크 발견 시 LCA 재귀속, reparse/offline/recall 제외, 알려진 0과 unknown 구분입니다. | 실제 sparse·압축·클라우드 provider별 동작은 환경별 증거가 필요합니다. 개인 클라우드 자료를 내려받아 시험하지 않습니다. |
| [S4 #2608](https://github.com/RefinedStone/codex-exec-loop/issues/2608) | 작업별 색인, 정렬·필터·페이지·cursor binding, 고정 revision, 자체 색인 제외, reader/worker와 조정한 소유 파일 정리, 24시간 보존 정책을 구현했습니다. | 일반 고정 조회는 scan과 동시 실행이 제한됩니다. disk-full과 파일 잠금의 실제 강제 장애 시험은 최종 검증 항목에 별도로 기록해야 합니다. |
| [S5 #2609](https://github.com/RefinedStone/codex-exec-loop/issues/2609) | 정수 threshold, 최종 폴더와 remainder partition, `included_bytes` 분리, 잠정 표시, 동일 색인의 threshold 재계산을 구현했습니다. | 요약은 포함량 내림차순입니다. 이름 필터·이름 정렬을 요약에 적용한다고 주장하지 않습니다. |
| [S6 #2610](https://github.com/RefinedStone/codex-exec-loop/issues/2610) | 공통 등록 스키마, 명시적 scan job, 켜기만 했을 때 순회 0, 진행 중 dedupe, host worker에서 스캐너 실행, 비활성화·취소·정리를 연결했습니다. | 스캐너는 thread 기반입니다. native crash가 별도 스캐너 프로세스에 격리된다는 보장은 없습니다. |

소스: [스캐너](../../src/adapter/outbound/storage_analysis/scanner.rs), [native 메타데이터](../../src/adapter/outbound/storage_analysis/native.rs), [색인](../../src/adapter/outbound/storage_analysis/index.rs), [live 배정](../../src/adapter/outbound/storage_analysis/live.rs), [확장 조립](../../src/composition/storage_extension.rs).

### G — Git과 런타임 보호

| 이슈 | 구현으로 충족한 범위 | 제한·후속 검증 |
| --- | --- | --- |
| [G #2611](https://github.com/RefinedStone/codex-exec-loop/issues/2611) | 여러 보호 사유와 관측 시각을 유지하고 unknown을 inactive로 바꾸지 않습니다. | 보호 정보는 삭제 허가 또는 회수 가능량이 아닙니다. |
| [G1 #2613](https://github.com/RefinedStone/codex-exec-loop/issues/2613) | 관측한 Git 메타데이터를 확인하고 common-dir당 porcelain-z 목록을 한 번 읽어 SQLite에 캐시합니다. main/linked/bare/detached/locked/prunable를 보존합니다. 분리된 Git 디렉터리의 main 경로는 고정한 local config의 core.worktree와 관측한 디렉터리 ID도 대조합니다. 서브모듈은 부모의 검증한 cached index stage 0·mode `160000` gitlink와 등록이 확인된 관측 자식의 경로·파일 ID를 전체 후보 관측 후 SQLite에서 대조합니다. `.git/modules` 경로 모양만으로 확정하지 않습니다. 기존 커널 root도 최대 256개·1초의 목록으로 받아 관측 디렉터리 ID와 대조하므로 Git 발견 실패에도 runtime 근거를 붙입니다. | 외부·미관측 common-dir, placeholder/reparse, config include, alternates는 unknown입니다. 런타임 후보 상한·조회 실패는 별도 미확인 근거를 남깁니다. 후보 수에 비례한 메모리 캐시가 없고 Git 단일 출력/record 상한이 있습니다. |
| [G2 #2614](https://github.com/RefinedStone/codex-exec-loop/issues/2614) | 기존 cached index/HEAD·로컬 upstream·활성 세션/슬롯 보호 근거에, cached stat과 안전한 파일 메타데이터의 없음·크기·유형·시각 차이, Git 작업 잠금 경로, 저장소 ignore 규칙 적용 미추적 관측을 추가했습니다. 검증한 제어 파일의 제한된 복사본을 Git check-ignore로 평가하며 일반 작업 파일의 본문은 대조하지 않습니다. | 같은 stat은 내용 동일성·clean을 증명하지 않습니다. 특수 인덱스 항목·링크·placeholder·조회 한도는 unknown이며, 사용자 전역 exclude와 스캔 후 변경은 전체 검증 범위가 아닙니다. 기존 보호 근거와 status_unknown을 함께 유지하고 삭제 가능 boolean을 만들지 않습니다. |
| [G3 #2615](https://github.com/RefinedStone/codex-exec-loop/issues/2615) | `target/Cargo.toml`, `node_modules/package.json`, `Library/ProjectSettings`의 이름·표식 분류를 원래 노드에 부착합니다. 동일 바이트를 별도 회수량으로 합산하지 않습니다. | 이름·표식만으로 안전한 삭제를 판단하지 않습니다. 빌드 명령, dependency manager, 외부 tree 스캔을 실행하지 않습니다. |

소스: [Git 관측](../../src/adapter/outbound/storage_analysis/worktree.rs), [Git metadata 가드](../../src/adapter/outbound/storage_analysis/worktree/git_metadata.rs), [cached 인덱스 대조](../../src/adapter/outbound/storage_analysis/worktree/tracked.rs), [런타임 관찰](../../src/composition/repository_kernel/storage.rs).

#### G2의 추가 관측과 회귀 범위

- 인덱스의 stat 필드는 엄격하게 파싱합니다. 공개된 안정 스키마가 아닌 `ls-files --debug` 출력이 바뀌거나 잘리면 미확인으로 반환합니다. 경로는 NUL 구분을 유지하고 경로 이탈·플랫폼에서 모호한 이름을 거절합니다.
- stat 대조는 읽기 전용 attributes 조회입니다. missing/size/mode/time 차이와 특수 인덱스·접근·경로 검증 실패를 구분합니다. 일반 작업 파일의 본문 hash, clean filter, fsmonitor, hook을 실행하는 변경은 추가하지 않았습니다.
- ignore 제어 파일은 파일당 64KiB·합계 1MiB·256개, 후보 경로와 부모는 10,000개로 제한합니다. 파일 ID·부모 ID·일반 파일 여부·읽기 전후 크기와 시각, reparse/placeholder를 확인합니다. 원본 규칙을 Git으로 재탐색하지 않고 분석기 소유 복사본에서만 평가하며 결과에는 사유와 건수만 남깁니다.
- 실제 Git fixture는 중첩 규칙·negation·디렉터리 패턴·공백 경로를 `ls-files --others --exclude-standard -z`와 대조합니다. Unix fixture는 개행 경로도 포함합니다. 범위 밖 `core.excludesFile`, 큰 규칙 파일, 잘린 출력은 규칙 적용 완료로 표시하지 않습니다.
- 기존 index 바이트·config·다른 작업의 index.lock을 보존하는지 확인합니다. assume-unchanged/skip-worktree를 삭제로 오인하지 않고, upstream 누락·로컬 ahead·detached·외부 런타임 소유 미확인이 확인된 보호 사유와 함께 남는지 검증합니다.
- Windows에서는 작업 파일의 데이터 읽기를 공유 잠금으로 거절한 상태에서도 attributes만으로 크기 변화를 관측하는 회귀를 추가했습니다. 이 시험은 내용 읽기에 성공한 뒤 결과만 숨기는 구현을 허용하지 않습니다.

소스와 재현 대상: [stat 관측](../../src/adapter/outbound/storage_analysis/worktree/stat.rs), [ignore 복사본](../../src/adapter/outbound/storage_analysis/worktree/ignore.rs), [실제 Git/native 회귀](../../src/adapter/outbound/storage_analysis/worktree/g2_tests.rs). 이 절은 추가된 검증 범위이며 실행 결과와 환경은 해당 PR의 로컬 검증 기록으로 연결합니다.

### A — 공동 탐색

| 이슈 | 구현으로 충족한 범위 | 제한·후속 검증 |
| --- | --- | --- |
| [A #2630](https://github.com/RefinedStone/codex-exec-loop/issues/2630) | preview→grant→공유 읽기→근거 resolve가 하나의 읽기 서비스와 스냅숏·노드 ID를 사용합니다. | 실제 모델 턴은 실행하지 않습니다. backend 동일 서비스 시험과 Desktop 표시 fixture는 각 증거 수준을 기록합니다. |
| [A1 #2631](https://github.com/RefinedStone/codex-exec-loop/issues/2631) | 십진 바이트 문자열, 원시 분자·분모, 큰 정수, 루트/0/unknown/partial, 하드링크 귀속, stable page를 공통 DTO로 정의했습니다. | 내부 `u64` 상한 초과는 오류입니다. 클라이언트가 원시 값을 부동소수점으로 덮어쓰지 않아야 합니다. |
| [A2 #2632](https://github.com/RefinedStone/codex-exec-loop/issues/2632) | server-verified 대화·연결 epoch, 명시 preview, grant scope, 외부 부모/사유 정제, 동일 존재 오류, revoke·재분석·정리·대화 변경 경합 검사를 구현했습니다. 확장 payload는 일반 recorder에서 제외합니다. | grant는 해당 조회 서비스의 권한 경계입니다. 같은 OS 사용자의 다른 shell을 격리하지 않으며 이미 보낸 대화 내용은 회수하지 않습니다. |
| [A3 #2633](https://github.com/RefinedStone/codex-exec-loop/issues/2633) | 동일 스키마·CLI 경로, `--thread-id` 서버 확인, scope-first 조회·hotspot 계산, HMAC cursor, 기본 10/요청 최대 100, 실제 페이지 최대 20·64KiB, 생략 표시를 구현했습니다. | 전용 신규 MCP 어댑터는 없습니다. CLI는 발급된 grant를 읽으며 preview와 create를 서로 다른 단발 연결에서 이어 붙이지 않습니다. |
| [A4 #2634](https://github.com/RefinedStone/codex-exec-loop/issues/2634) | 근거 참조의 host/thread/grant/snapshot/revision/node를 검사하고 허용 조상을 반환합니다. 같은 이름 검색으로 대체하지 않습니다. | 최신 사용자 탐색만 선택·초점을 변경하는 Desktop 동작과 키보드 왕복은 Desktop 검증 기록에서 따로 확인합니다. backend resolve 성공만으로 UI 검증을 통과시키지 않습니다. |

소스: [공유 서비스](../../src/application/service/storage_sharing/mod.rs), [공유 projection](../../src/application/service/storage_sharing/projection.rs), [공유 테스트](../../src/application/service/storage_sharing/tests.rs), [실제 등록 서비스 통합 시험](../../src/composition/storage_extension/tests.rs), [CLI thread 확인](../../src/adapter/outbound/extension_gateway_client.rs).

### V — 증거와 측정

| 이슈 | 준비된 검증 | 남은 판정 |
| --- | --- | --- |
| [V #2621](https://github.com/RefinedStone/codex-exec-loop/issues/2621) | 결정적·native·통합 증거 층, 재현 명령, 이슈별 제한을 문서화했습니다. | 최종 SHA·환경·실행 결과를 같은 기록으로 묶어야 합니다. |
| [V1 #2622](https://github.com/RefinedStone/codex-exec-loop/issues/2622) | 독립 기대값의 설계 예시, LCA 순열, 0/unknown/부분 집계, 실제 작은 하드링크·정션, 재열거 없는 페이지를 검사합니다. | 실제 파일 크기를 지정한 것을 실제 할당량 기대값으로 간주하지 않습니다. ACL·압축·sparse·cloud별 native 범위는 실행 여부를 구분합니다. |
| [V2 #2623](https://github.com/RefinedStone/codex-exec-loop/issues/2623) | 등록 호스트+실제 SQLite 경로, idempotency/dedupe, 취소·disable·정리, grant 경합 barrier, 대화 epoch, 근거 복귀 계약을 검사합니다. | 객체 복원·자식 호스트 프로세스 종료·실제 native 스캐너 crash·Desktop E2E의 강도를 혼동하지 않습니다. 상세 호스트 시험은 별도 문서와 연결합니다. |
| [V3 #2624](https://github.com/RefinedStone/codex-exec-loop/issues/2624) | 세 독립 fixture, concurrency 1/4 각각 3회, 정확성 동일 조건, 최초 progress/파일 관측/전체/페이지/필터/색인/취소 로그를 출력하는 ignored benchmark를 준비했습니다. | **NVMe SSD의 세 fixture 반복과 native 시험 프로세스 peak working set을 측정했습니다. HDD, 통제한 cold cache, 실제 native 부하 중 대화 UI 반응은 미측정입니다.** 배정 중지는 terminal 시각의 상한만 기록합니다. |

## 3. 회귀 시험과 기대 경계

### 결정적 집계·공유

- 정확한 바이트를 넣는 작은 fixture에서 설계 예시의 최종 폴더와 나머지를 독립적으로 검산합니다. 기대값 계산에 제품 집계 함수를 다시 호출하지 않습니다.
- 하드링크 발견 순열마다 총량과 LCA 소유가 같고 부모·자식·직속·공유 그룹의 합산이 중복되지 않는지 검사합니다.
- threshold의 절대 기준·상대 최소·부모 비율 경계와 `u64` 큰 정수·초과를 검사합니다.
- 공유된 12 GiB의 9 GiB 자식이 같은 node ID, `9663676416/12884901888` 원시 비율로 반환되는지 확인합니다.
- 다른 host/thread/activation, 범위 밖·없는 node, 변조·다른 grant cursor, 외부 부모·사유 문자열, 긴 이름과 응답 예산을 검사합니다.
- 조회 I/O 및 최종 snapshot 확인 중 revoke, preview 도중 새 scan, 대화 이동, 이전 snapshot 정리를 barrier로 재현합니다.

근거: [domain 시험](../../src/domain/storage_analysis/mod.rs), [threshold 시험](../../src/domain/storage_analysis/hotspots.rs), [adapter fixture](../../src/adapter/outbound/storage_analysis/tests.rs), [공유 fixture](../../src/application/service/storage_sharing/tests.rs).

### 실제 작은 Windows fixture

- 실제 메타데이터·하드링크와 directory junction을 테스트 전용 임시 트리에 생성합니다. 사용자 데이터 트리에 장애를 만들지 않습니다.
- 원본 트리를 fixture가 명시적으로 제거한 후에도 완료 색인 조회가 같은 결과를 반환하는지 확인합니다. 이 제거는 “원본을 재열거하지 않음”을 검증하기 위한 테스트 동작이며 분석기 동작이 아닙니다.
- 최초 루트 핸들의 volume GUID를 선택한 ID와 대조합니다. 불일치 시 첫 progress·루트 파일 ID 기록·열거 전에 거절하며, fixture 문자열로 production 검사를 우회하지 못하는지 확인합니다. [볼륨 바인딩 회귀](../../src/adapter/outbound/storage_analysis/volume_tests.rs)를 실제 장치 교체 시험과 구분합니다.
- 일반 드라이브 표기로 저장한 SQLite 루트와 Windows 최종 경로를 함께 검증합니다. Git metadata가 없어도 알려진 root와 child 두 곳에 활성 세션·기존 저장소 보호 근거가 붙는지 확인하며, 수정 전에는 두 조회가 모두 누락되는 실패를 재현했습니다.
- reveal 시 같은 경로가 다른 파일 ID로 교체되면 거절합니다.
- cleanup 경로에 분석기가 소유하지 않은 항목이 있으면 보존하고 오류를 반환합니다.
- Git index의 분석 전후 바이트를 비교합니다. clean filter·external diff·textconv trap이 실행되지 않는지, main/linked가 common-dir 캐시 한 행을 사용하는지 검사합니다.
- cached 경로 대조는 gitlink·sparse·`.git` 제외, Windows 대소문자 일치와 모호한 충돌, 1MiB/10,000 entry 초과의 unknown fallback을 검사합니다.
- 실제 Git submodule 등록 자식과 `.git/modules` 아래 메타데이터만 둔 미등록 저장소를 같은 fixture에서 비교합니다. 두 자식 모두 Git 메타데이터·worktree 등록 검증을 통과하되 부모 index에 stage 0 gitlink가 있는 자식에만 `submodule` 근거가 붙고, 부모 index 바이트는 보존되는지 검사합니다.
- 기존 config·commondir의 읽기 handle을 유지하는 동안 실제 Windows 쓰기·교체가 거절되고, 해제 후 다시 열리는지 확인합니다. 취소된 token이 공통 Git 실행 경계에서 다음 명령을 거절하는지도 검사합니다.
- Git 메타데이터가 없는 알려진 runtime root를 관측 노드와 연결하고, 관측 범위 밖 경로에는 runtime 조회를 보내지 않는지 확인합니다.
- 미관측 외부 Git dir 및 reparse `.git`가 Git 명령으로 넘어가지 않는지 확인합니다.
- 오류 코드 분류 시험은 실제 물리 드라이브를 분리한 시험과 구분합니다. 실제 cloud hydration 방지의 모든 provider를 이 fixture가 증명하지는 않습니다.

근거: [native fixture와 읽기 전용 회귀](../../src/adapter/outbound/storage_analysis/tests.rs), [등록·런타임 근거 회귀](../../src/adapter/outbound/storage_analysis/evidence_tests.rs), [Git metadata guard 시험](../../src/adapter/outbound/storage_analysis/worktree/git_metadata.rs).

### 통합과 실행 중 탐색

등록된 `ExtensionService`를 통해 explicit scan을 호출하고 실제 adapter·SQLite 결과를 공유 서비스가 읽는 경로가 있습니다. enable만 했을 때 job index 생성 0, 같은 요청 키 재시도, scope 밖 node 차단, 대화 epoch 변경, revoke·disable cleanup을 확인합니다.

live 페이지는 SQL 인덱스 기반의 bounded keyset 조회이며 TEMP B-TREE를 사용하지 않는지 검사합니다. 실행 중 실제 스캔에서 발견 디렉터리를 조회하고 우선순위를 넣는 fixture, 64개 상한·중복·3:1 FIFO 공정성·중복 재배정 방지 시험이 있습니다. live revision 0은 공유 근거로 승격하지 않습니다.

근거: [등록 통합 시험](../../src/composition/storage_extension/tests.rs), [live 시험](../../src/adapter/outbound/storage_analysis/tests.rs), [대화 문맥 시험](../../src/adapter/inbound/gateway/storage_context.rs).

## 4. 재현 명령

저장소 루트에서 프로젝트가 지정한 Rust toolchain과 Windows native 환경으로 실행합니다. 실제 볼륨을 입력하지 않고 시험이 소유한 fixture만 사용합니다.

```text
cargo test --locked --lib storage_
cargo test --locked --lib storage_analysis_git_observation_preserves_index_and_protection_reasons -- --nocapture
cargo test --locked --lib storage_analysis_comparison_benchmark -- --ignored --nocapture
```

기존 혼합 fixture 이력을 다시 확인할 때만 다음 별도 시험을 사용합니다.

```text
cargo test --locked --lib storage_analysis_bounded_fixture_benchmark -- --ignored --nocapture
```

ignored benchmark는 일반 단위 시험 성공 개수에 포함하지 않습니다. Windows 전용 fixture가 다른 OS에서 컴파일되지 않았거나 실행되지 않은 경우 “통과”로 합산하지 않습니다. 최종 검증은 해당 커밋에서 실행한 명령·종료 코드·검사 개수와 함께 기록합니다.

## 5. V3 성능 측정 범위

새 기준은 [comparison benchmark](../../src/adapter/outbound/storage_analysis/benchmark.rs)의 세 독립 데이터셋입니다. 각 데이터셋에서 concurrency 1과 4를 각각 3회 실행하고, 같은 파일 수·논리량·할당량·완전성·오류 수를 확인한 실행끼리 비교합니다.

| case | 구성 | 기대 파일 수 | 기대 디렉터리 수 | 반복 |
| --- | --- | ---: | ---: | --- |
| `small_many` | 32개 하위 폴더 × 128개 × 1KiB | 4,096 | 33 | concurrency 1/4 각각 3회 |
| `large_few` | 4개 하위 폴더 × 8개 × 1MiB | 32 | 5 | concurrency 1/4 각각 3회 |
| `flat_many` | 하나의 디렉터리 안에 2,048개 × 1KiB | 2,048 | 1 | concurrency 1/4 각각 3회 |

이는 생성 조건입니다. 실제 할당량을 논리 크기와 같다고 가정하지 않습니다. 각 실행에서 관측한 할당량을 출력하고 동일 조건 결과가 같은지 검증합니다.

### 수집 필드

| 필드 | 의미와 해석 |
| --- | --- |
| `first_progress_us` | scan 호출부터 최초 backend progress callback까지입니다. 실제 화면 표시 지연이 아닙니다. |
| `first_file_observation_us` | 최초로 `files_scanned > 0`인 callback까지입니다. 접수 표시와 실제 관측을 구분합니다. |
| `elapsed_us` | 해당 native fixture scan의 전체 종료 시간입니다. |
| `page_10_us / filter_10_us` | 완료 색인에서 파일 10행 조회와 이름 필터 조회 시간입니다. |
| `index_bytes` | 해당 실행의 SQLite 파일 크기입니다. |
| `files/directories/logical_bytes/known_allocated_bytes/state/issue_count` | 속도 비교의 품질 조건입니다. 누락·오류가 늘어난 실행을 같은 정확도의 개선으로 계산하지 않습니다. |
| `cancel_to_terminal_us` | 첫 파일 관측 시 취소를 요청한 뒤 terminal 반환까지입니다. |
| `new_dispatch_stop_upper_bound_us` | 현재는 terminal 시간에서 얻은 상한입니다. 배정 중지 시각을 별도로 계측한 값이 아닙니다. |
| `observed_when_cancelled/observed_at_terminal` | 취소 당시 관측 수와 이미 진행 중이던 I/O가 종료할 때까지 추가 반영된 관측 수입니다. |

각 데이터셋의 반복 종료 후 별도의 취소 실행도 수행합니다. 취소 이후 진행 중이던 OS I/O 결과가 도착할 수 있으므로 “취소 후 관측 수 증가”와 “취소 후 새 디렉터리 배정”을 같은 것으로 해석하지 않습니다.

### 측정 수치 — 2026-11-23 NVMe SSD

측정 소스: `c690571b778a51e90dae8cef16c9d8a6ce5170fc`. 전체 benchmark는 1 passed/0 failed, native process exit 0입니다. 해당 시험 동안 이 작업의 다른 로컬 빌드·E2E는 실행하지 않았습니다.

| case | concurrency 1: 3회 / 중앙값 (ms) | concurrency 4: 3회 / 중앙값 (ms) | 취소→terminal (ms) |
| --- | --- | --- | ---: |
| `small_many` | 619.779, 602.355, 604.454 / **604.454** | 408.516, 400.338, 407.383 / **407.383** | 57.729 |
| `large_few` | 45.215, 68.715, 125.917 / **68.715** | 37.831, 37.441, 36.549 / **37.441** | 16.764 |
| `flat_many` | 165.607, 167.750, 169.690 / **167.750** | 166.631, 174.065, 170.256 / **170.256** | 6.212 |

| case / concurrency | 최초 progress 중앙값 (ms) | 최초 파일 관측 중앙값 (ms) | 10행 조회 중앙값 (ms) | 필터 중앙값 (ms) | 색인 크기 범위 (B) |
| --- | ---: | ---: | ---: | ---: | ---: |
| small_many / 1 | 17.060 | 22.603 | 6.954 | 6.429 | 2818048–2818048 |
| small_many / 4 | 17.039 | 22.957 | 8.144 | 7.614 | 2813952–2822144 |
| large_few / 1 | 17.128 | 20.112 | 3.212 | 2.811 | 61440–61440 |
| large_few / 4 | 16.269 | 19.491 | 2.843 | 2.807 | 61440–61440 |
| flat_many / 1 | 16.663 | 17.496 | 4.710 | 4.374 | 1372160–1372160 |
| flat_many / 4 | 16.567 | 17.574 | 4.722 | 4.313 | 1372160–1372160 |

각 case의 6회에서 파일·디렉터리 수, 논리 바이트, 할당 바이트가 동일했고 상태는 complete, issue 수는 0이었습니다. 확인 할당량은 small_many 16,777,216 B, large_few 33,554,432 B, flat_many 8,388,608 B입니다. SQLite 삽입 순서와 저장 경로 길이에 따라 색인 파일 크기는 달라질 수 있으며, 이를 집계 정확도 변화로 계산하지 않습니다.

테스트 프로세스 peak working set은 **22,204,416 B (21.18 MiB)**였습니다. .NET System.Diagnostics.Process.PeakWorkingSet64를 요청 간격 20ms로 173회 수집했습니다. fixture 생성, native worker, SQLite와 시험 harness를 포함하며 전체 Gateway나 Desktop의 메모리는 포함하지 않습니다. debug/test 비최적화 빌드입니다.

파일 분포에 따라 동시 수별 결과가 달라지며, 평면 폴더는 나눌 하위 디렉터리가 없습니다. 작은 반복 수와 캐시 미통제 조건이므로 SSD 일반 최적값으로 확정하지 않고 기본 동시 수 1을 유지합니다. 느린 회차의 원인은 별도 프로파일링으로 확정하지 않았습니다. 원시 회차와 종료 코드는 [JSON 기록](storage-analysis-native-metrics.json)에 있습니다.

첫 progress·첫 파일 관측·전체 시간은 서로 다른 지표이며 실제 UI 표시 지연은 별도로 측정해야 합니다.

### 미측정과 보수적인 기본값

| 항목 | 상태 |
| --- | --- |
| 실제 HDD | 미측정. 장치가 회전식인지 확인하지 않은 디스크를 HDD 결과로 기록하지 않습니다. |
| 통제한 cold cache | 미측정. 생성 직후 첫 실행을 cold run으로 부르지 않습니다. |
| 실제 native scan 부하 중 Desktop 대화 입력·렌더링 | 미측정. 모형 이벤트 E2E의 응답성을 실제 디스크 부하 측정으로 대체하지 않습니다. |
| 최대 메모리 | native 시험 프로세스 peak working set 21.18 MiB. worker·SQLite·fixture harness 포함, 전체 Gateway·renderer 제외. |
| 취소 입력→새 배정 중지 시각 | 별도 계측 없음. terminal까지의 시간은 상한 자료입니다. |
| 느린 장치·분리 중 취소 | 미측정. 모든 장치에서 종료 1초 이내를 보장하지 않습니다. |
| concurrency 기본값 | 1. host-global scan 직렬화와 job 내부 1~4 상한을 사용합니다. SSD 4/HDD 1의 자동 최적값을 확정하지 않았습니다. |
| 요약 threshold | 설계 기본 1GiB 또는 256MiB·부모 20%를 사용합니다. 성능 측정으로 사용자 최적 기준을 입증한 값은 아닙니다. |

[V3 피드백](https://github.com/RefinedStone/codex-exec-loop/issues/2624#issuecomment-7075202862)의 1초 목표는 새 배정을 멈추는 목표로 해석합니다. 대기 중인 OS I/O·Git metadata 조회·cleanup의 최종 종료까지 모든 환경에서 같은 제한을 보장하는 수치로 사용하지 않습니다.

## 6. 검증 기록의 해석

제품 소스 SHA와 시험·측정 대상을 연결하고 중간 이력은 별도로 보존했습니다. 문서 전용 변경은 제품 소스 검증과 구분합니다. Git working tree 상세 미확인, native crash 프로세스 격리 부재, 물리 장치·cold cache·native 부하 UI 미측정을 PR과 이슈 표에서 함께 유지합니다.

민감 경로·파일명·사용 가능한 grant 원문을 검증 보고서에 복제하지 않습니다. 소스는 이 문서의 저장소 상대 링크로 연결하고 fixture의 의미·범위·결과만 기록합니다.
