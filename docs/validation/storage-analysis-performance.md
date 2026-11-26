# SSD/HDD 성능과 초기 기본값 검증

이 기록은 [V3 #2624](https://github.com/RefinedStone/codex-exec-loop/issues/2624)와 [V #2621](https://github.com/RefinedStone/codex-exec-loop/issues/2621)의 성능 측정 범위를 다룹니다. 기존 2026-11-23 NVMe 측정은 [백엔드 기록](storage-analysis-backend.md)의 당시 소스에 연결해 유지하고, 아래 측정을 별도 실행으로 기록합니다.

## 1. 측정 소스와 장치

2026-11-26 Windows 호스트에서 합성 fixture만 사용했습니다. 기준 커밋은 `39ac5b9c1f5d8a0d65ad50f367d461c25d9d2ece`입니다. 각 native·threshold 원시 JSON의 `provenance`에는 기준 커밋, 변경한 Rust 파일의 SHA-256, 실행 파일 SHA-256과 오케스트레이터 SHA-256을 함께 저장합니다. UI 네 실행의 소스·원자료 해시는 별도 [UI provenance](storage-analysis-performance/2026-11-26-ui-provenance.json)에 기록합니다. 커밋 전 파일을 빌드한 결과를 기준 커밋 자체의 실행으로 표시하지 않습니다.

| 장치 | 실제 식별 근거 | 파일 시스템과 배치 |
| --- | --- | --- |
| C: NVMe SSD | SHPP41-2000GM, 물리 disk 3, MSFT_PhysicalDisk MediaType=4, NVMe | NTFS. fixture와 각 실행의 별도 SQLite 색인을 같은 볼륨에 배치 |
| I: 7200 RPM SATA HDD | WDC WD10EALX-009BA0, 물리 disk 2. OS의 normal-seek 응답과 제조사 WD10EALX 사양 대조 | NTFS. fixture와 각 실행의 별도 SQLite 색인을 같은 볼륨에 배치 |

I:의 `GetPhysicalDisk` 자체는 MediaType=Unspecified, SpindleSpeed=0을 반환했습니다. 이를 HDD 자동 감지 성공으로 해석하지 않습니다. `fsutil fsinfo sectorinfo I:`의 일반 검색 수행·TRIM 미지원 결과와 [Western Digital Caviar Blue 원본 데이터시트의 유통사 보관본](https://cdn.centralpoint.be/objects/pdf/5/596/2206533_1_interne-harde-schijven-western-digital-caviar-blue-1tb-wd10ealx-rfb.pdf)에 있는 WD10EALX·7200 RPM을 함께 사용했습니다. seek 플래그의 의미는 [Microsoft FILE_FS_SECTOR_SIZE_INFORMATION](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/3e75d97f-1d0b-4e47-b435-73c513837a57)에 정의되어 있습니다. 원시 JSON에는 선택한 두 장치에 관한 OS 응답만 보관합니다.

## 2. 데이터와 측정 구간

| 데이터 | 파일 | 디렉터리(루트 포함) | 논리 크기 | 구조 |
| --- | ---: | ---: | ---: | --- |
| 작은 파일 `small_many` | 8,192 | 17 | 8 MiB | 16개 폴더마다 1 KiB 파일 512개 |
| 큰 파일 `large_few` | 12 | 6 | 384 MiB | 32 MiB 파일을 8/1/1/1/1개씩 다섯 폴더에 배치 |

큰 파일은 sparse 파일이 아니라 정해진 바이트를 실제로 쓴 파일입니다. 스캐너는 기존 native metadata 경로를 사용하므로 파일 본문 전체를 읽는 처리량 시험이 아닙니다. 데이터 생성과 종료 시 후속 페이지·요약 조회는 scan 시간에서 제외합니다. 각 scan은 새로운 libtest 프로세스에서 실행하고, 동일 fixture는 재사용하며 색인은 실행별 디렉터리에 새로 만듭니다. index I/O를 포함한 adapter 전체 시간이지 디렉터리 열거만의 시간은 아닙니다.

각 장치에서 데이터별 concurrency 1/2/4를 세 번 실행합니다. 실행 순서는 회차별 `1,4,2 → 4,2,1 → 2,1,4`이며 원자료에 전체 실행 순번을 남깁니다. 첫 실행은 이 세션에서 해당 데이터를 처음 분석한 실행입니다. 파일 생성은 write/close까지만 수행해 OS writeback 종료를 별도로 기다리지 않았습니다. 파일 생성 직후의 OS cache가 남을 수 있고, 다른 concurrency 실행도 같은 fixture를 미리 읽습니다. **cold-cache 측정은 수행하지 않았습니다.** 캐시 삭제·퇴거, 사용자 데이터 스캔·변경, 실제 모델 턴은 사용하지 않았습니다.

완료된 모든 실행에서 파일 수, 디렉터리 수, 논리·할당 합계, 자식의 이름·크기·상태, threshold별 요약 결과를 순차 실행 결과와 비교합니다. 동시 열거 순서에 따라 달라질 수 있는 노드 ID는 동일성 비교에서 제외합니다. 별도로 데이터별 concurrency 1/2/4의 첫 파일 진행 callback에서 취소하는 6개 실행을 기록합니다.

| 지표 | 정확한 범위 |
| --- | --- |
| `elapsed_us` | begin JSON 기록 직전부터 adapter.scan 반환까지. 진행 JSON 출력과 test-only 계측 비용을 포함하고 프로세스 시작·fixture 생성·완료 후 조회는 제외 |
| `first_progress_us` | native adapter의 첫 진행 callback. Desktop의 실제 표시 시간과 별도 |
| `first_file_observation_us` | 1개 이상의 파일을 기록한 첫 native callback |
| `process_peak_working_set_bytes` | 새 native 프로세스의 OS lifetime peak working set을 scan 반환 직후 조회. libtest·runtime·SQLite 포함. Node·Gateway·Electron·OS 파일 캐시는 제외 |
| `process_peak_with_queries_bytes` | 완료 후 자식 페이지와 세 threshold 조회까지 포함한 같은 프로세스의 peak |
| `new_dispatch_stop_us` | 취소 요청을 기준으로 마지막 thread 배정 완료 시각을 별도 계측한 값의 양수 부분. 취소→terminal 시간을 대신 쓰지 않음 |

native 메모리 값은 `K32GetProcessMemoryInfo`의 PeakWorkingSetSize입니다. 이 측정용 프로세스 분리는 제품 스캐너의 별도 프로세스 격리를 의미하지 않습니다. 제품 스캐너의 thread 모델은 유지됩니다.

## 3. 실행 결과

원자료: [SSD native](storage-analysis-performance/2026-11-26-ssd-native.json), [HDD native](storage-analysis-performance/2026-11-26-hdd-native.json). 장치별 정상 18개와 취소 6개, 합계 정상 36개·취소 12개가 완료됐습니다. 모든 정상 실행에서 결과 동등성을 통과했습니다. 작은 파일은 논리 8 MiB / 실제 관측 할당 32 MiB, 큰 파일은 논리·할당 모두 384 MiB였습니다.

시간 단위는 ms입니다. 각 행의 전체 분포는 3개 표본, 재실행 중앙값은 뒤의 2개 표본입니다. “해당 c 첫 실행” 역시 다른 concurrency 실행의 캐시 효과를 받을 수 있습니다.

| 장치 | 데이터 | c | 해당 c 첫 실행 | 재실행 중앙값 | 전체 중앙값 (최소–최대) | 첫 progress 최대 | native peak 최대 MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| SSD | small_many | 1 | 764.922 | 807.355 | 775.308 (764.922–839.401) | 17.816 | 24.23 |
| SSD | small_many | 2 | 587.198 | 572.379 | 584.176 (560.581–587.198) | 17.904 | 24.57 |
| SSD | small_many | 4 | 503.346 | 514.250 | 508.883 (503.346–519.616) | 17.846 | 25.02 |
| SSD | large_few | 1 | 51.407 | 51.727 | 51.407 (50.444–53.011) | 18.749 | 17.54 |
| SSD | large_few | 2 | 46.362 | 46.891 | 46.362 (46.244–47.537) | 18.477 | 17.61 |
| SSD | large_few | 4 | 45.242 | 50.068 | 45.242 (44.379–55.757) | 25.715 | 17.71 |
| HDD | small_many | 1 | 31679.321 | 30740.429 | 31551.897 (29928.960–31679.321) | 6163.066 | 23.82 |
| HDD | small_many | 2 | 26597.314 | 27086.110 | 26597.314 (26524.743–27647.478) | 1201.243 | 24.37 |
| HDD | small_many | 4 | 22448.364 | 24316.548 | 24233.714 (22448.364–24399.382) | 1183.778 | 24.86 |
| HDD | large_few | 1 | 3188.034 | 3150.767 | 3188.034 (2873.848–3427.686) | 1282.637 | 17.53 |
| HDD | large_few | 2 | 2995.565 | 3449.615 | 3243.651 (2995.565–3655.578) | 1553.143 | 17.59 |
| HDD | large_few | 4 | 2474.944 | 2878.881 | 2636.961 (2474.944–3120.801) | 1253.865 | 17.62 |

**native 첫 progress 기준으로 SSD는 18/18개가 1초 이내였지만 HDD는 6/18개만 충족했습니다.** HDD 첫 progress 범위는 840.808–6,163.066ms이며 이 수치를 UI 수락 표시 시간으로 바꾸어 해석하지 않습니다. adapter.scan은 SQLite 색인 초기화와 같은 볼륨의 쓰기 비용을 포함하므로, 이 결과만으로 순수 디렉터리 읽기나 파일 본문 처리량의 장치 비율을 계산하지 않습니다.

정상 실행의 native peak 최대는 26,238,976바이트 (25.02MiB)였습니다. 프로세스 시작부터의 peak이며 사용자에게 보이는 Desktop 전체나 OS cache의 메모리 사용량은 아닙니다.

## 4. 취소와 신규 배정 중지

취소 flag를 설정한 뒤 이미 배정한 worker의 I/O·채널 이벤트가 끝날 수 있습니다. 따라서 취소 시 파일 수와 terminal 파일 수의 차이를 신규 배정 수로 계산하지 않습니다. `dispatches_after_cancel`, 마지막 배정 시각, 스케줄러의 최초 취소 관측 시각, terminal 도달 시각을 각각 기록합니다. 배정 시각 계측은 `#[cfg(test)]`이며 제품 빌드에는 포함되지 않습니다.

이 probe의 취소 조건은 첫 파일 진행 callback입니다. 외부 UI에서 임의 시점에 취소하는 모든 경합이나 멈춘 장치 I/O의 최대 시간을 증명하는 시험은 아닙니다. 1초 목표 판정에는 실제 새 배정 시각을 사용하고, 종료 대기 시간은 별도 수치로 표시합니다.


| 장치 | 데이터 | c | 취소 후 새 배정 | 최초 취소 관측 ms | 취소→terminal ms | 관측 파일 수 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| SSD | small_many | 1 | 0 | 1.464 | 33.697 | 1 → 3 |
| SSD | small_many | 2 | 0 | 1.661 | 31.943 | 1 → 6 |
| SSD | small_many | 4 | 0 | 1.581 | 35.852 | 1 → 9 |
| SSD | large_few | 1 | 0 | 1.509 | 14.331 | 1 → 3 |
| SSD | large_few | 2 | 0 | 1.556 | 15.106 | 1 → 4 |
| SSD | large_few | 4 | 0 | 1.522 | 18.560 | 1 → 5 |
| HDD | small_many | 1 | 0 | 126.591 | 2392.942 | 1 → 3 |
| HDD | small_many | 2 | 0 | 98.849 | 2325.990 | 1 → 6 |
| HDD | small_many | 4 | 0 | 121.374 | 2201.417 | 1 → 11 |
| HDD | large_few | 1 | 0 | 95.384 | 708.490 | 1 → 3 |
| HDD | large_few | 2 | 0 | 147.380 | 1148.067 | 1 → 4 |
| HDD | large_few | 4 | 0 | 187.806 | 1279.826 | 1 → 6 |

모든 표본에서 마지막 배정은 취소 요청 전에 끝났고, 취소 뒤 새 배정은 0건이었습니다. 따라서 관측된 신규 배정 중지 지연은 0이며 1초 목표를 충족했습니다. **이 0은 취소 수신이나 작업 종료가 즉시라는 뜻이 아닙니다.** 최초 스케줄러 취소 관측은 SSD 1.464–1.661ms, HDD 95.384–187.806ms이고, HDD terminal 대기는 최대 2,392.942ms였습니다.

## 5. native 부하와 Desktop 입력

별도 Desktop probe는 기존 Electron renderer에 실제 native worker의 진행을 연결한 fixture transport를 사용합니다. 실제 제품 Gateway 서버나 사용자 세션을 사용하지 않습니다. `AKRA_DESKTOP_HIDDEN=1`인 격리된 창에서 고정 대화 draft를 입력하며 `turn/start`를 호출하지 않습니다.

입력 이벤트→DOM 관측, 자동화 왕복, rAF paint opportunity를 분리합니다. 숨김 창의 rAF 지연은 visible paint 지연으로 해석하지 않습니다. 실제 중첩은 첫 native progress의 UNIX 시각부터 `scan_finished_unix_us` 사이에 들어온 입력만 계산하고, query·JSON 결과 조립·프로세스 종료 대기 시간을 scan 부하로 합산하지 않습니다. 짧은 scan의 표본이 부족하면 그 수와 제한을 그대로 남깁니다. Electron main·renderer 메모리는 native peak와 별도 범위입니다.


원자료: [SSD c1](storage-analysis-performance/2026-11-26-ssd-ui-c1.json), [SSD c4](storage-analysis-performance/2026-11-26-ssd-ui-c4.json), [HDD c1](storage-analysis-performance/2026-11-26-hdd-ui-c1.json), [HDD c4](storage-analysis-performance/2026-11-26-hdd-ui-c4.json). [UI 소스·원자료 해시 기록](storage-analysis-performance/2026-11-26-ui-provenance.json)은 측정한 두 JS 파일과 native 행렬 실행 파일의 연속성을 구분해 기록합니다. Rust 재빌드 전에 UI 4개를 끝냈으며, UI 실행 중 해시를 직접 수집한 것으로 표시하지 않습니다.

| 장치 | c | 첫 상태 DOM ms | 첫 native progress DOM ms | 그때 파일 / 폴더 | 실제 scan 중 입력 표본 | 자동화 왕복 p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| SSD | 1 | 63.5 | 82.3 | 1 / 1 | 12 | 5.882 |
| SSD | 4 | 54.4 | 69.4 | 1 / 1 | 8 | 6.891 |
| HDD | 1 | 64.9 | 1018.5 | 0 / 0 | 184 | 4.451 |
| HDD | 4 | 57.4 | 1137.3 | 0 / 0 | 182 | 4.643 |

4개 모두 수락 상태 DOM은 1초 이내입니다. **첫 native 진행 DOM은 HDD 두 실행에서 각각 1,018.5ms·1,137.3ms로 1초를 넘었습니다.** HDD의 이 표시에는 아직 파일·폴더가 0개였으므로 첫 파일 결과를 얻은 시간으로 사용하지 않습니다. native 순수 행렬의 첫 callback과 Desktop DOM 표시는 서로 다른 실행·경계입니다.

scan 중 입력 이벤트→값 관측 microtask의 p95는 네 실행 모두 약 0.1ms였습니다. renderer 타이머 해상도 때문에 0/0.1ms가 흔하며, input 이벤트는 값 변경 후 발생하므로 이 값을 물리 키 입력부터 제품 화면까지의 총응답 시간으로 해석하지 않습니다. Playwright fill/값 읽기 왕복은 별도 열의 수치입니다. 50ms 간격으로 최대 200개를 시도했으므로 HDD 전체 23–29초 중 초반 약 10초 구간의 입력 관측이며, 마지막까지 균등 표집한 값은 아닙니다. 모델 요청·renderer 예외·메모리 질의 오류·소유 프로세스 정리 오류는 모두 0건입니다.

BrowserWindow.isVisible=false를 확인했지만 document.visibilityState는 visible을 반환했습니다. 이 OS/Electron 관측을 그대로 보관합니다. 첫 rAF p95는 약 921–962ms, 두 번째는 약 1,920–1,966ms였습니다. SSD는 두 번째 rAF가 scan과 겹친 표본이 0개이고, 보이는 화면의 paint 목표는 이 probe로 판정하지 않습니다.

Electron process-lifetime peak는 각 실행마다 main 124,796 / 129,700 / 134,220 / 125,736KB, renderer 127,188 / 123,392 / 143,372 / 146,888KB였습니다(위 표 순서). 같은 프로세스 안의 공유 페이지가 있을 수 있으므로 단순 합을 전체 메모리로 게시하지 않습니다. UI 구간의 native peak 최대는 26,050,560바이트이며 별도 프로세스 범위입니다.

## 6. 초기 기본값과 해석

제품의 현재 기본 concurrency는 1, 허용 범위는 1~4입니다. 요약 threshold는 절대 1 GiB 또는 최소 256 MiB이면서 부모의 20%입니다. 이번 측정에서는 더 많은 행 후보(128 MiB / 32 MiB / 5%)와 더 적은 행 후보(2 GiB / 512 MiB / 25%)도 동일 색인에서 비교합니다. 표시 행 수와 쿼리 시간을 함께 기록해 선택 근거를 남깁니다.

**제품 기본값은 변경하지 않고 concurrency 1 / 최대 4를 초기 정책으로 유지합니다.** [ScanOptions의 기본값과 검증](../../src/domain/storage_analysis/mod.rs)이 실제 동작의 기준입니다. SSD 작은 파일에서 c2는 c1 대비 약24.7%, c4는 약34.4% 빨랐고, HDD에서는 각각 약15.7%·23.2% 빨랐습니다(각 3개 표본의 중앙값 비교). SSD 큰 파일은 c4가 약12.0% 빨랐지만 차이가 약6ms이며, HDD 큰 파일은 c2가 c1보다 약1.7% 느리고 c4가 약17.3% 빨랐습니다.

따라서 이 두 장치에서 다중 폴더를 빠르게 분석하려는 경우 c4를 선택할 실측 근거는 있습니다. 그러나 한 호스트·cache 미통제·같은 디스크의 SQLite·작은 합성 트리·dev profile만 측정했고, 전체 볼륨에서 사용자의 다른 작업에 주는 I/O 경쟁과 tail latency를 평가하지 않았습니다. 관측된 이득을 모든 장치의 자동 기본값으로 일반화할 근거는 부족하므로 전역 기본1을 유지하고 상한4를 허용하는 정책을 채택합니다. c2를 보편적인 절충값으로 올리지도 않습니다. 장치 자동 분류나 장치별 설정 변경은 추가하지 않았습니다.


### 계층형 threshold 비교

기본 행렬의 얕은 트리는 세 후보 모두 작은 파일16행·큰 파일5행을 반환하므로, 그 값만으로 임계값을 선택할 수 없습니다. 원래 큰 파일 fixture의 32MiB 파일12개를 새 계층 안에 각각 한 번씩 hardlink로 연결해 별도로 native scan했습니다. 새 데이터는 루트 포함5개 폴더이며 giant=320MiB(256MiB child_big, 32MiB child_small, 직속32MiB), moderate=64MiB입니다. 실제 파일 바이트는 재작성하지 않고, 기존 fixture와 물리 저장 공간을 공유합니다. 각 분석 범위 안에서는 파일ID가 한 번씩만 나타납니다.

원자료: [SSD 계층형](storage-analysis-performance/2026-11-26-ssd-threshold.json), [HDD 계층형](storage-analysis-performance/2026-11-26-hdd-threshold.json). 별도 [threshold worker](../../src/adapter/outbound/storage_analysis/benchmark/thresholds.rs)를 빌드해 concurrency1로 장치별3회 실행했습니다. 기존 [행렬 worker](../../src/adapter/outbound/storage_analysis/benchmark/performance.rs)는 당시 SHA-256과 바이트 단위 동일하게 보존했습니다. threshold 추가 후 실행 파일은 달라졌으므로 각각의 원자료에 별도 executable/source 해시를 남겼고, 기존 전체 행렬을 새 실행 파일에서 다시 측정했다고 표시하지 않습니다.

| 후보(절대 / 상대 최소 / 부모 비율) | 반환행과 포함량 MiB | 행 수 | SSD query 중앙값 ms | HDD query 중앙값 ms |
| --- | --- | ---: | ---: | ---: |
| 기본 1GiB / 256MiB / 20% | child_big256 + giant remainder64 + moderate64 | 3 | 1.980 | 1.359 |
| 완화 128MiB / 32MiB / 5% | child_big256 + moderate64 + giant remainder32 + child_small32 | 4 | 2.209 | 1.451 |
| 강화 2GiB / 512MiB / 25% | giant320 + moderate64 | 2 | 1.552 | 1.028 |

모든 후보·모든 실행에서 포함량 합계384MiB를 유지했습니다. query는 완료된 같은 색인에서 기본→완화→강화 순서로 실행했으므로 query 시간의 작은 차이를 장치 속도 순위나 독립적인 cold 조회 성능으로 해석하지 않습니다. 이 별도 worker는 메모리를 측정하지 않으며 원자료의 peak 항목은 null입니다.

**요약 기본값 1GiB / 256MiB / 20%도 유지합니다.** 이 계층에서 기본값은 큰256MiB 하위를 드러내면서32MiB 하위는 상위 remainder에 남깁니다. 완화값은 작은 하위의 행을 추가하고 강화값은 큰 하위까지 상위에 합치므로, 초기 요약의 자세함을 중간 수준으로 두는 근거로 채택합니다. 전체384MiB이므로 절대1GiB 분기는 이번 native fixture에서 발동하지 않았습니다. 절대·상대 임계값의 경계 논리는 기존 [독립 경계 회귀](../../src/domain/storage_analysis/hotspots.rs)가 확인하며, 이번 결과로 절대1GiB가 모든 볼륨의 통계적 최적점이라고 주장하지 않습니다.

## 7. 재현

기존 fixture와 출력 파일을 덮어쓰지 않는 새 경로를 선택합니다. 측정 도구는 자동 삭제하지 않으며 fixture는 고정 manifest를 포함합니다. 이번 세션의 두 fixture는 모든 native/UI/threshold 측정 종료 후 manifest, 전체 고정 파일 목록, 원자료의 job ID, 하드링크의 파일ID를 대조하고 정리했습니다. 소스·Cargo target·node_modules·원자료는 유지했습니다.

```powershell
cargo test --locked --all-features --lib --no-run --target x86_64-pc-windows-msvc --message-format=json
# compiler-artifact의 executable 값을 사용합니다.
$nativeExecutable = "<libtest executable>"
node scripts/storage-analysis-performance.mjs --native-executable $nativeExecutable --fixture-root "C:/Temp/akra-storage-performance-unique-ssd" --device-label nvme_ssd --output "C:/Temp/ssd-native.json"
node scripts/storage-analysis-performance.mjs --native-executable $nativeExecutable --fixture-root "I:/akra-storage-performance-unique-hdd" --device-label sata_hdd --output "C:/Temp/hdd-native.json"
```

fixture 부모 디렉터리는 미리 존재해야 합니다. 기존 합성 fixture로 다시 측정할 때만 `--reuse-fixture`를 사용하고 결과에 재사용 사실을 남깁니다. 고정 manifest와 데이터 크기가 맞지 않거나 순차·병렬 결과가 달라지면 측정은 실패로 끝나고 마지막 checkpoint가 남습니다.


같은 fixture의 UI 측정은 전체 행렬 뒤에 수행합니다. 다음 예시는 SSD concurrency1이며, fixture_root와 concurrency를 바꾸어 SSD/HDD 각각1·4를 실행하고 출력 파일과 run_id를 매번 새로 정합니다. probe는 자체적으로 숨김 창·격리 AKRA_HOME/userData를 설정합니다.

```powershell
npm --prefix desktop ci
npm --prefix desktop run build
$requestPath = Join-Path $env:TEMP "akra-storage-ui-request.json"
$request = @{
  mode = "run"
  fixture_root = "C:/Temp/akra-storage-performance-unique-ssd"
  case = "small_many"
  concurrency = 1
  run_id = "ui-ssd-c1-unique"
  cancel_after_first_file = $false
} | ConvertTo-Json -Compress
[System.IO.File]::WriteAllText($requestPath, $request, [System.Text.UTF8Encoding]::new($false))
node desktop/test/storage-performance-ui.mjs --native-executable $nativeExecutable --request-json $requestPath --output "C:/Temp/ssd-ui-c1.json"

# 기존 worker 소유 fixture에서 별도 계층을 만들고 3회 측정합니다.
# threshold_tree가 이미 있으면 새 fixture를 만들어 실행합니다.
node scripts/storage-analysis-performance.mjs --native-executable $nativeExecutable --fixture-root "C:/Temp/akra-storage-performance-unique-ssd" --device-label nvme_ssd --threshold-only --output "C:/Temp/ssd-threshold.json"
node scripts/storage-analysis-performance.mjs --native-executable $nativeExecutable --fixture-root "I:/akra-storage-performance-unique-hdd" --device-label sata_hdd --threshold-only --output "C:/Temp/hdd-threshold.json"
```

## 8. 검증과 남은 범위

로컬 검증은 다음 단계의 실제 결과를 구분해 기록합니다. [명령·결과·로그 해시](storage-analysis-performance/2026-11-26-local-validation.json)를 함께 보관합니다. 원격 CI 상태는 조회하거나 기다리지 않았습니다.

| 단계 | 결과 |
| --- | --- |
| native worker build | Rust1.95.0, Windows MSVC target, all-features libtest 컴파일 성공. threshold 추가 후 별도 재빌드도 성공 |
| storage_analysis 집중 회귀 | 최초 행렬 worker 소스에서 41 passed / 0 failed / 3 ignored, 6.67초. threshold용 ignored test를 추가한 뒤 같은 전체 필터를 반복한 것으로 계산하지 않음 |
| 실제 측정 | 행렬 정상36개·취소12개, 별도 계층형6개, native와 겹친 Desktop4개 모두 기록 완료 및 해당 품질 assertion 통과 |
| JS 문법 / Rust 포맷 | 새 probe·helper·오케스트레이터 node --check와 cargo fmt 통과 |
| 최초 Windows Full | Desktop 테스트 단계에서 종료코드1. 1,200 passed / 6 failed / 7 skipped. 앞선 policy·Admin·의존성 감사·타입 검사는 통과 |
| 실패 파일만 재실행 | gitBranch/gitHunks/gitPublish/gitStatus 4개 파일에 --maxWorkers=1 적용: 73 passed / 1 skipped, 종료코드0, 113.78초 |
| 최초 Full에서 미도달한 단계 | 종료코드0. Desktop build·offline asset policy·Windows target check·원본 계약104개 필터·Clippy(all-targets/all-features, -D warnings) 통과 |

최초 실패6개 중5개는 `Test timed out in 5000ms.`, 나머지1개는 다음 gitBranch 사례의 gh mock 호출 배열이 비어 있다는 assertion입니다. gitHunks의 timeout 정리 과정에서 `EPERM, Permission denied`도 발생했습니다. 해당 Git fixture 소스는 이 변경에 포함하지 않았지만, 그 사실만으로 실패 원인을 확정하지 않습니다. 재실행에는 timeout 완화나 소스 수정 없이 파일 단위 병렬 수만1로 제한했습니다. 재실행 일부는 다른 작업의 Desktop 검사와 겹쳤으므로 완전히 무부하인 격리 재현으로 표시하지 않습니다. **최초 Full 단일 실행이 성공했다고 표시하지 않으며, 실패 재시도와 미도달 단계의 결과를 따로 보존합니다.**

측정하지 않은 범위는 OS cold cache, 전체 볼륨·수백만 파일, release 빌드, 네트워크·이동식·클라우드 장치, 장치가 중간에 멈추는 경우, 실제 제품 Gateway와 모델 턴의 동시 부하, visible Desktop paint, macOS/Linux입니다. 팀의 대형 빌드는 측정 동안 멈췄지만 호스트의 다른 사용자 프로세스나 백그라운드 OS 활동을 종료하거나 완전히 통제하지는 않았습니다.
