# 저장 공간 분석기

저장 공간 분석기(`akra.storage-analysis`, API 버전 `1`)는 사용자가 선택한 Windows 로컬 볼륨의 메타데이터를 읽고, 같은 작업 색인으로 폴더 점유율·파일 목록·큰 폴더 요약·worktree 보호 근거를 제공합니다. 확장을 켜거나 패널을 여는 것만으로 파일 시스템 순회를 시작하지 않습니다. 사용자가 분석을 요청해야 새 작업이 생깁니다.

분석 결과는 관측 시작·종료 시각이 있는 고정 결과입니다. 파일 시스템 전체를 한순간에 동결한 스냅숏은 아닙니다. 실행 중 파일이 바뀌거나 일부 경로를 읽지 못하면 확인한 값과 누락 사유를 함께 제공합니다. 구현 및 검증 범위는 [백엔드 검증 기록](../validation/storage-analysis-backend.md), 전체 설계는 [plan 30](../plan/30-kernel-extensions-and-storage-analysis.md)을 참고하세요.

## 1. 실행 범위와 기본 동작

| 항목 | 계약 |
| --- | --- |
| 실행 범위 | 같은 로컬 호스트의 같은 OS 사용자에 속한 `local_user` 확장입니다. 저장소를 attach하기 전에도 연결할 수 있습니다. |
| 대상 선택 | `volumes`가 반환한 안정적인 `volume_id`를 선택합니다. 임의 파일 경로를 분석 입력으로 받지 않습니다. 현재 Windows fixed/removable 로컬 볼륨이 대상입니다. |
| 명시적 실행 | `scan`이 작업을 접수합니다. 같은 볼륨·정규화된 옵션의 진행 중 요청은 같은 작업을 공유합니다. |
| 재시도와 재분석 | 같은 `idempotency_key`의 재시도와 진행 중 조건 중복을 구분합니다. 완료 후 새 키로 요청한 재분석은 새로운 작업·스냅숏 ID를 만듭니다. |
| 창 닫기 | 패널 또는 클라이언트 연결을 닫아도 살아 있는 호스트의 작업은 계속됩니다. 다시 연결하면 `jobs`/`job`으로 상태를 읽습니다. |
| 비활성화 | 신규 기능 호출을 막고 진행 중 작업의 취소를 요청합니다. 워커 종료와 색인 정리가 끝날 때까지 `disabling`입니다. |
| 읽기 전용 범위 | 원본 파일 본문·파일 삭제·이동·worktree 정리를 제공하지 않습니다. 쓰기는 분석기 소유 색인과 호스트 상태에 한정됩니다. Git 근거를 위해 확인된 Git 메타데이터를 제한적으로 읽습니다. |
| 재시작 | 이전 미완료 작업은 `interrupted`로 복원합니다. 자동으로 다시 분석하지 않습니다. 공유 grant는 메모리에만 있어 호스트 재시작 후 다시 발급해야 합니다. |

호스트는 작업 스레드를 커널 명령 처리 루프 밖에서 실행합니다. 현재 native 스캐너는 별도 프로세스로 격리되지 않습니다. Rust 오류·unwind 처리와 호스트 재시작 복구를 지원하지만, native crash를 별도 프로세스 안에 가둔다고 보장하지 않습니다.

근거: [확장 조립](../../src/composition/storage_extension.rs), [공통 작업 호스트](../../src/application/service/kernel_extensions/mod.rs), [작업 수명](../../src/application/service/kernel_extensions/jobs.rs), [native 어댑터](../../src/adapter/outbound/storage_analysis/mod.rs).

분석을 시작할 때에는 경로를 다시 조회해 얻은 serial만으로 선택을 대신하지 않습니다. 실제로 연 최초 루트 핸들의 볼륨 GUID를 요청한 volume_id와 비교한 뒤에만 루트 identity와 진행 상태를 기록합니다. 선택 후 드라이브 문자가 다른 볼륨으로 바뀌는 경계에서는 열거를 시작하지 않습니다. 이후 하위 항목도 검증된 볼륨 identity와 대조합니다.

## 2. 상태, 취소와 결과 보존

작업의 실행 상태와 분석 결과의 완전성은 다른 필드입니다.

| 위치 | 상태와 의미 |
| --- | --- |
| `ExtensionJob.state` | `queued` → `running` → `completed`, 또는 `cancelling` → `cancelled`. 오류는 `failed`, 재시작 중단은 `interrupted`입니다. |
| `StorageSnapshot.state` | 정상 관측은 `complete`, 일부 미확인은 `partial`입니다. DTO에는 `pending/running/cancelled/failed/interrupted`도 정의되어 있습니다. |
| `StorageNode.state` | 해당 노드의 집계가 완전한지 표시합니다. 상위 스냅숏 상태와 비율의 `provisional`도 함께 읽어야 합니다. |
| `ExtensionJob.cleanup` | `pending/complete/failed`로 색인 정리 상태를 별도로 표시합니다. |

`completed` 작업 안의 스냅숏이 `partial`일 수 있습니다. 반대로 취소 요청이 접수되었다고 즉시 워커 종료나 색인 삭제가 끝난 것은 아닙니다.

호스트의 `cancel`은 취소된 작업 결과를 `result: null`로 반환합니다. 스캐너 내부에서 잠정 취소 결과를 만들 수 있어도, 클라이언트가 이를 임의의 완료 스냅숏으로 만들어 표시해서는 안 됩니다. 실패·취소·중단 작업은 워커 종료 후 정리 대상이며, 일반 사용자는 다시 명시적으로 분석해야 합니다.

성공 결과의 기본 보존 기간은 종료 기록 시점부터 **24시간**입니다. 호스트는 60초 간격과 새 작업 접수 시 만료를 확인합니다. 보존 job 수 상한 128개가 모두 아직 만료되지 않은 결과이면 새 작업을 거부하며, 용량 확보를 위해 유효한 결과를 먼저 삭제하지 않습니다. 만료 또는 명시적 비활성화로 cleanup이 완료된 terminal 메타데이터만 새 job과 같은 영속 저장에서 교체하고, 재시도 receipt tombstone은 유효 기간까지 유지합니다. 확장 비활성화는 결과 정리를 요청하므로 영구 보관용 저장소는 아닙니다.

정리는 원래 job ID에 귀속된 색인에만 적용합니다. 해당 스냅숏의 공유를 먼저 무효화하고 사용 중인 reader/worker와 조정한 후, 소유가 확인된 SQLite 파일만 삭제합니다. 다른 스냅숏의 grant나 원본 폴더를 함께 지우지 않습니다.

근거: [보존 정책](../../src/application/service/kernel_extensions/retention.rs), [terminal 처리](../../src/application/service/kernel_extensions/jobs.rs), [색인 정리](../../src/adapter/outbound/storage_analysis/mod.rs).

## 3. 공통 데이터 모델

완료 결과의 기본 revision은 `1`이며 스냅숏마다 고정됩니다. 노드 식별에는 `snapshot_id + revision + node.id`를 함께 사용합니다. 노드 ID는 같은 스냅숏 안에서 안정적이며, 같은 이름이나 다른 스냅숏의 같은 ID를 동일한 사실로 취급하지 않습니다.

| 필드 | 해석 |
| --- | --- |
| `allocated_bytes` | 완전히 확인한 할당 바이트의 십진 정수 문자열입니다. 미확인이면 `null`이고 확인한 0은 `"0"`입니다. |
| `known_allocated_bytes` | 미확인이 섞여 있어도 확인한 부분의 할당량을 보존합니다. |
| `logical_bytes` | 논리 크기입니다. 식별한 하드링크의 폴더 집계는 할당량과 같은 소유 규칙으로 중복을 제거합니다. |
| `ratio.numerator_bytes` | 표시할 점유율의 원시 분자입니다. |
| `ratio.denominator_bytes` | 같은 스냅숏에서 관측한 직속 부모의 분모입니다. 현재 페이지에 표시된 행들의 합으로 다시 계산하지 않습니다. |
| `ratio.reason` | `root`, `zero_denominator`, `unknown` 또는 정상 비율의 `null`입니다. 공유 루트에서는 외부 부모를 숨겨 `shared_root`가 됩니다. |
| `ratio.provisional` | 일부 관측에 기초한 잠정 비율인지 표시합니다. |
| `issues` | 접근 실패·소실·연결 해제·미확인 등의 코드, 설명, 건수입니다. |
| `protection` | Git·Akra 보호 근거와 관측 시각입니다. 여러 사유가 함께 존재할 수 있습니다. |
| `classification` | 프로젝트 표식에 기초한 분류입니다. 삭제 가능 여부가 아닙니다. |

바이트는 내부 `u64` 범위의 정수이며 합계 초과는 오류로 처리합니다. 비율 임계값 비교에는 더 넓은 정수 연산을 사용합니다. JavaScript 클라이언트는 원시 문자열을 보존하고 정렬·비교에 `BigInt` 등 정확한 정수 처리를 사용해야 합니다. `Number`로 변환한 표시값으로 원시 바이트를 덮어쓰면 안 됩니다.

예를 들어 9 GiB/12 GiB의 원시 값은 `"9663676416"`/`"12884901888"`이며 비율은 75%입니다. `null`이나 미확인을 0으로 채우지 않습니다.

노드 종류는 `directory/file/direct_files/shared_files`입니다. 하드링크의 경로는 파일 목록에 모두 남지만, 볼륨·파일 ID가 같은 파일의 합계는 공통 조상에 한 번만 귀속합니다. 나중에 형제 경로의 링크를 발견하면 이전 귀속을 공통 조상으로 옮깁니다. 직속 파일·공유 파일 묶음과 상세 파일 목록을 동시에 합산하면 중복되므로, 폴더 합계 또는 겹치지 않는 요약을 기준으로 사용합니다.

근거: [공통 DTO](../../src/domain/storage_analysis/mod.rs), [집계·페이지 색인](../../src/adapter/outbound/storage_analysis/index.rs), [임계값 연산](../../src/domain/storage_analysis/hotspots.rs).

## 4. 일반 조회 API

호출 전 카탈로그에서 현재 action과 입력 스키마를 확인합니다. 대부분 입력은 알 수 없는 필드를 거절하므로, 예를 들어 `snapshot`에 `job_id`를 더하거나 `get_node`에 임의 `revision` 필드를 더하지 않습니다. revision은 응답과 cursor·grant에 연결됩니다.

공통 envelope는 `akra/kernel/extensions/call`의 `kernel, extension, version, action, input, idempotency_key?`입니다. `kernel`과 호스트 범위는 Gateway가 확정합니다.

| action | 주요 입력 | 결과 |
| --- | --- | --- |
| `volumes` | `{}` | `{volumes: StorageVolume[]}` |
| `scan` | `{volume_id, options?: {concurrency}}` | 접수된 `ExtensionJob`. 성공 terminal의 `result`가 `StorageSnapshot`입니다. |
| `jobs` / `job` | `{}` / `{job_id}` | 호스트가 보유한 작업 목록 또는 한 작업 |
| `cancel` | `{job_id}` | 취소 요청/terminal 상태. 취소된 결과는 `null`입니다. |
| `snapshot` | `{snapshot_id}` | `StorageSnapshot` |
| `get_node` | `{snapshot_id, node_id}` | `StorageNode` |
| `list_children` | `{snapshot_id, node_id, limit?, cursor?, sort?, name_filter?}` | 직속 폴더 및 합성 묶음의 페이지 |
| `list_files` | 위 입력에 `extension_filter?, recursive?` | 직속 또는 하위 트리 파일의 페이지 |
| `find_hotspots` | `{snapshot_id, node_id, thresholds?, limit?, cursor?}` | 최종 폴더·나머지의 페이지 |
| `ancestors` | `{snapshot_id, node_id}` | `{snapshot_id, node_id, ancestors: StorageNode[]}` |
| `reveal_path` | `{snapshot_id, node_id}` | 현재 파일 ID와 경계를 재확인한 `{path, host_id, snapshot_id, node_id}` |

일반 페이지 크기는 기본 10, 최대 100입니다. `list_children/list_files`의 정렬은 `allocated_desc` 또는 `name`이고 동률은 안정적인 노드 ID로 해소합니다. 이름·확장자 필터와 하위 파일 조회는 같은 SQLite 색인을 읽습니다. 완료 뒤 탐색·정렬·필터·임계값 변경이 원본 디스크 재순회를 시작하지 않습니다. `reveal_path`만 실제 경로가 교체되지 않았는지 다시 확인하며, 탐색기 열기는 Desktop 측 동작입니다.

`find_hotspots`는 포함 바이트 내림차순 요약입니다. 이 호출에는 이름 필터나 이름 정렬을 사용하지 않습니다. 공통 입력 구조가 존재한다고 해서 모든 조회 옵션이 이 요약에도 적용되는 것은 아닙니다.

스캔 중 일반 고정 결과 조회는 `storage_scan_in_progress`로 거절될 수 있습니다. 파일 기반 SQLite 임시 작업이 동시에 진행하는 스캔에 섞이지 않도록 하는 보수적인 호스트 가드입니다. 실행 중 탐색에는 다음의 독립 live API를 사용합니다.

근거: [등록 스키마](../../src/composition/storage_extension/schemas.rs), [서비스](../../src/application/service/storage_analysis/mod.rs), [색인 조회](../../src/adapter/outbound/storage_analysis/index.rs).

### 큰 폴더 요약

기본 기준은 자식 폴더가 **1 GiB 이상이거나**, **256 MiB 이상이면서 부모의 20% 이상**인 경우 더 내려가는 것입니다. 볼륨 루트는 자식 폴더부터 평가합니다. 조건을 만족하는 자식이 없으면 현재 폴더에서 멈춥니다. 내려간 자식 외의 직속·공유 파일과 펼치지 않은 자식은 현재 폴더의 나머지로 남깁니다.

요약 행에는 `node`와 별개로 `included_bytes, role, reason, provisional`이 있습니다. `node.allocated_bytes`는 원래 폴더의 전체 근거량이며, **요약 합산은 `included_bytes`만 사용**합니다. 부모의 전체량을 나머지 값으로 덮어쓰지 않습니다. 공유 조회에서도 이 정보를 `hotspot` 필드로 유지합니다.

## 5. 실행 중 live 보기와 우선순위

| action | 입력 | 동작 |
| --- | --- | --- |
| `live_children` | `{job_id, node_id, cursor?, limit?}` | 발견한 직속 디렉터리의 이름·ID·열거 완료 여부를 읽습니다. |
| `prioritize` | `{job_id, node_id}` | 아직 배정되지 않은 발견 디렉터리의 우선순위를 올립니다. |

live 응답은 `revision: 0`이며 루트는 `n1`입니다. 확정 바이트·비율·공유 근거 참조를 제공하지 않습니다. `enumeration_complete: false`인 부모는 `total_count: null`일 수 있고, 현재 행이 없어도 다음 cursor로 이후 발견분을 읽을 수 있습니다. 고정 revision의 스냅숏 공유와 혼용하지 않습니다.

우선순위 큐는 64개이며 중복 요청은 새 항목을 만들지 않습니다. 한도를 넘으면 `storage_priority_queue_full`을 반환합니다. 우선 요청을 최대 3개 배정한 뒤 오래 기다린 FIFO 항목 1개를 강제로 배정합니다. 패널을 열거나 자동 새로고침하는 것만으로 우선 요청을 생성하지 않고, 사용자가 경로를 펼친 동작에 연결합니다.

현재 보수적인 자원 정책은 **호스트 전체에서 scan job을 직렬화하고, 한 job 내부의 디렉터리 작업을 1~4개로 제한**하는 것입니다. 기본값은 1입니다. 물리 장치별 SSD/HDD 자동 최적값은 확정하지 않았습니다. 단일 거대 디렉터리의 개별 파일 처리를 별도 병렬 파이프라인으로 나누지는 않으므로 동시 수 4가 모든 데이터 구조에서 빠르다고 보장하지 않습니다.

근거: [live DTO](../../src/domain/storage_analysis/live.rs), [SQLite live 조회·공정 배정](../../src/adapter/outbound/storage_analysis/live.rs), [스캐너](../../src/adapter/outbound/storage_analysis/scanner.rs).

## 6. Git·Akra 보호 근거

Git 명령을 시작하기 전에 관측된 Git 메타데이터를 검사합니다. `.git` 및 `gitdir/commondir`를 대조하고, placeholder/reparse·미관측 외부 common-dir·변경된 ID·부분 관측이면 조회를 생략합니다. 경로 파일은 최대 4KiB, config는 최대 64KiB로 읽으며 외부 config include와 object alternates가 있으면 미확인 처리합니다.

Windows에서는 검증한 기존 config·경로 포인터 파일의 읽기 handle을 Git 조회 종료까지 유지하여 해당 파일의 쓰기·교체를 막습니다. 부재하던 optional 파일의 동시 생성이나 Git 메타데이터 전체를 원자적으로 고정하는 보장은 아닙니다. 이 검사는 별도 프로세스의 모든 파일 접근을 격리하는 OS 샌드박스가 아닙니다.

공통 Git 디렉터리마다 `git worktree list --porcelain -z`를 한 번 수행하고 결과를 작업 SQLite에 캐시합니다. 명령 시간은 3초, 표준 출력은 1MiB, 목록은 4,096개 record로 제한합니다. NUL 구분 경로를 파싱하고 main/linked/bare/detached/locked/prunable 근거를 보존합니다. 사라진 등록이나 `prunable` 표시에 회수 가능 바이트를 부여하지 않습니다. 분리된 Git 디렉터리가 main 경로로 반환되면, 고정한 local config의 `core.worktree`가 관측한 작업 폴더 경로와 파일 ID에 대응하는지도 확인합니다. 서브모듈은 검증한 부모 cached index의 stage 0·mode `160000` gitlink가 같은 작업에서 관측하고 Git 등록을 확인한 자식 디렉터리의 경로·파일 ID와 대응할 때만 표시합니다. 인덱스 등록을 SQLite에 보존한 뒤 전체 후보 관측이 끝나면 대조하므로 부모·자식 발견 순서에 의존하지 않습니다. `.git/modules/...`라는 경로 모양만으로 서브모듈을 확정하지 않으며, 이를 위해 `.gitmodules`나 원본 본문을 읽거나 외부 부모 트리를 새로 순회하지 않습니다. 각 Git 명령을 시작할 때와 명령 사이에 취소를 확인합니다. 취소 전에 이미 시작한 명령은 기존 timeout 안에서 종료를 기다릴 수 있습니다.

**작업 파일 본문에 기초한 수정 판정과 ignore 규칙을 적용한 완전한 미추적 목록은 제공하지 않습니다.** `git status` 대신 cached index와 로컬 HEAD를 비교하여 `index_changes/index_unchanged/index_unknown`을 남기고, 항상 `status_unknown`을 유지합니다. `index_unchanged`를 깨끗한 worktree나 삭제 가능 판정으로 해석하면 안 됩니다. 사용자 clean filter·external diff·textconv를 실행하는 원본 본문 검사를 하지 않습니다.

`git ls-files --cached --stage -z --sparse`의 경로·stage를 같은 SQLite의 관측 경로와 대조합니다. 출력은 1MiB, entry는 10,000개까지이며 초과·조회 실패·모호한 경로는 `index_paths_unknown`입니다. 확인한 파일 경로가 인덱스에 없으면 `untracked_observed`, 충돌 stage가 있으면 `index_conflicts`를 표시합니다. 미추적 관측은 ignore 규칙을 적용하지 않은 부분 근거이므로 무시된 빌드 파일도 포함될 수 있습니다. `.git` 메타데이터와 gitlink·sparse 디렉터리 아래는 제외하고, Windows 경로의 대소문자는 정규화하되 인덱스에서 모호하게 충돌하면 미확인으로 남깁니다. 이 대조는 원본 트리를 다시 순회하지 않습니다.

로컬 upstream 참조와의 ahead 여부는 기록할 수 있지만 fetch를 수행하지 않으므로 원격 최신 게시 상태를 보장하지 않습니다. Akra 런타임은 이미 열린 커널에만 읽기 조회를 보냅니다. 조회를 위해 attach·세션·슬롯을 새로 만들지 않으며, 응답 실패나 관측 부재를 inactive로 바꾸지 않습니다. 이미 열린 커널의 알려진 root도 최대 256개·목록 조회 1초 범위에서 수집하여 선택 볼륨의 관측 디렉터리 ID와 대조합니다. `.git` 발견에 실패해도 확인한 runtime 보호 근거를 유지합니다. 범위 밖 경로는 관측 노드에 연결하지 않으며, 목록 상한·timeout 등은 `runtime_candidates_partial`로 남깁니다.

`target + Cargo.toml`, `node_modules + package.json`, `Library + ProjectSettings`는 각각 `candidate_target/candidate_node_modules/candidate_library` 분류 근거입니다. 이 표식은 빌드 결과임을 완전히 증명하거나 삭제를 허가하지 않습니다.

근거: [Git 관측](../../src/adapter/outbound/storage_analysis/worktree.rs), [Git 메타데이터 가드](../../src/adapter/outbound/storage_analysis/worktree/git_metadata.rs), [인덱스 경로 대조](../../src/adapter/outbound/storage_analysis/worktree/tracked.rs), [기존 런타임 읽기](../../src/composition/repository_kernel/storage.rs), [Git subprocess 공통 강화](../../src/git_subprocess.rs).

## 7. 대화에 선택 폴더 공유하기

공유는 다음 순서로 진행합니다.

1. 사용자가 폴더와 대상 대화를 명시적으로 선택합니다.
2. `share_preview`가 실제 공유 필드와 범위를 보여줍니다.
3. 같은 신뢰된 연결에서 `share_create`가 grant를 발급합니다.
4. 대화에는 조회 명령과 `akra-storage:` 근거 참조를 전달합니다.
5. 모델·CLI의 `shared_*` 조회가 같은 색인의 허용된 하위 트리만 읽습니다.
6. 사용자가 근거를 클릭하면 `resolve_evidence`가 같은 스냅숏·노드와 허용된 조상을 반환합니다.

일반 클릭·키보드 이동 자체는 공유나 대화 전송을 뜻하지 않습니다. preview는 5분, grant는 1시간 유효하며 각각 최대 256개를 유지합니다. preview는 호스트·대화·연결·대화 epoch·활성 revision에 묶이고, grant 조회는 현재 호스트·대화·활성 상태·만료·스냅숏·허용 루트를 검사합니다. 대화 ID 문자열을 입력했다는 사실만으로 인증하지 않습니다.

| action | 입력 |
| --- | --- |
| `share_preview` | `{snapshot_id, node_id, thread_id}` |
| `share_create` | `{preview_id, thread_id}` |
| `share_revoke` | `{grant_id}` |
| `shared_get_node` | `{grant_id, node_id}` |
| `shared_list_children` | `{grant_id, node_id, limit?, cursor?, sort?, name_filter?}` |
| `shared_find_hotspots` | `{grant_id, node_id, thresholds?, limit?, cursor?}` |
| `resolve_evidence` | `{reference, thread_id}` |

공유 루트의 외부 부모 ID·부모 분모·절대 경로는 제거합니다. 외부 위치가 들어갈 수 있는 자유 형식 사유 설명도 정제합니다. 허용 범위를 먼저 적용한 뒤 조회·집계·페이지를 계산하며, 없는 노드와 범위 밖 노드는 동일한 `storage_share_unavailable`로 거절합니다.

공유 페이지는 기본 10개이며 요청 한도는 100개입니다. 총 응답 예산 때문에 실제 한 번의 반환은 최대 20개·64KiB로 제한하고 `response_limited, requested_limit, next_cursor`를 제공합니다. 이름은 UTF-8 경계를 지켜 최대 1,024바이트로 줄일 수 있고 사유·분류 목록은 각 8개로 제한합니다. 생략은 `omitted_fields`에 표시하며 ID·정수 바이트는 잘라 반환하지 않습니다. 전체 응답이 여전히 크면 더 작은 페이지나 더 깊은 공유 루트를 요청하도록 오류를 반환합니다.

공유 cursor는 HMAC으로 서명하고 grant·호스트·대화·스냅숏·revision·루트·대상 노드·조회 종류·정렬·필터·임계값에 묶습니다. 다른 grant나 조건의 cursor를 재해석하지 않습니다. 재분석, revoke, 비활성화, 보존 기간 정리, 대화 문맥 변경 후에는 권한을 다시 검사하며 늦은 응답도 최종 권한 검사 대상입니다.

`akra-storage:`는 검증할 내부 이동 힌트이며 단독 bearer 권한이나 실행 명령이 아닙니다. 실패한 참조를 같은 이름의 새 폴더로 자동 대체하지 않습니다. 이미 전송한 대화 내용은 공유 해제로 회수되지 않습니다. 이 경계는 저장 공간 조회 서비스에 적용되며 같은 OS 사용자의 다른 shell·파일 도구까지 제한하는 OS 샌드박스는 아닙니다.

Gateway 일반 recorder에서는 확장 요청·응답을 제외합니다. 사용자가 명시적으로 대화에 보낸 내용과 CLI 출력은 해당 대화·터미널에 남을 수 있습니다.

근거: [공유 서비스](../../src/application/service/storage_sharing/mod.rs), [출력 투영](../../src/application/service/storage_sharing/projection.rs), [공유 DTO](../../src/application/service/storage_sharing/model.rs), [Gateway 대화 검증](../../src/adapter/inbound/gateway/storage_context.rs), [Gateway 응답 기록](../../src/adapter/inbound/gateway/kernel.rs).

### CLI에서 이미 발급된 공유 읽기

Gateway가 실행 중이고 Desktop에서 해당 대화에 grant를 발급한 상태에서 다음 PowerShell 예제를 사용할 수 있습니다.

```powershell
$threadId = Read-Host '공유 대상 대화 ID'
$grantId = Read-Host 'Desktop에서 발급한 grant ID'
$nodeId = Read-Host '공유 루트 node ID'
$inputJson = @{ grant_id = $grantId; node_id = $nodeId } | ConvertTo-Json -Compress

akra extensions call akra.storage-analysis shared_get_node --scope local_user --thread-id $threadId --input $inputJson --json
akra extensions call akra.storage-analysis shared_list_children --scope local_user --thread-id $threadId --input $inputJson --json
akra extensions call akra.storage-analysis shared_find_hotspots --scope local_user --thread-id $threadId --input $inputJson --json
```

`--thread-id`는 같은 연결에서 서버의 `thread/resume` 응답을 검증하며 `excludeTurns: true`를 사용합니다. 새 모델 턴을 시작하지 않습니다. ID가 다르거나 서버가 확인하지 못하면 공유 조회를 허용하지 않습니다. 임의 경로나 cwd로 grant를 보완하지 않습니다.

preview와 create는 같은 연결에 묶이므로 별도의 단발 CLI 프로세스 두 번으로 발급 단계를 이어가는 용도로 사용하지 않습니다. 일반적인 발급은 Desktop의 지속 연결에서 수행하고, 단발 CLI는 발급된 grant를 읽습니다. 전용 MCP 어댑터를 새로 추가한 것은 아닙니다.

근거: [CLI 입력](../../src/adapter/inbound/cli/extensions.rs), [Gateway CLI 클라이언트](../../src/adapter/outbound/extension_gateway_client.rs), [공유 조회 명령 생성](../../src/application/service/storage_sharing/prompt.rs).

## 8. 현재의 실용적 제한

- 스캔 합계는 관측한 파일의 할당량입니다. Windows가 표시하는 전체 볼륨 사용량과의 차이, 예약 공간·파일 시스템 메타데이터를 완전히 조정한 값은 아닙니다.
- reparse·offline·recall 대상은 따라가거나 내려받지 않으며 미확인으로 표시합니다. 실제 개인 클라우드 자료를 다운로드해 검증하지 않았습니다.
- live 보기는 발견 디렉터리 탐색이며 실행 중인 바이트 트리의 확정본이 아닙니다.
- 완료 색인 조회는 새 스캔과 동시에 제한될 수 있습니다.
- Git 보호 정보는 관측 사실과 미확인을 함께 보여줍니다. 본문 기반 수정 판정·ignore 적용 미추적 전체 상태와 삭제 가능 판정은 제공하지 않습니다.
- SSD/HDD별 최적 동시 수, cold-cache 성능, 실제 native 스캔 부하 중 Desktop 입력 지연은 별도 측정이 필요합니다. [검증 기록의 미측정 항목](../validation/storage-analysis-backend.md#5-v3-성능-측정-범위)을 확인하세요.
