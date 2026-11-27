# A03 고정 문서 선택 평가 v1 (#2828)

두 합성 task를 같은 ledger에서 비교한다. 실제 model turn, remote provider, Vault 평문 또는 실제 사용자의 대화는 사용하지 않는다.

실행: `cargo test --locked -p akra-ade --lib context_selection_fixed_cases_measure_four_independent_metrics -- --nocapture`.

비교 기준: 초점 없이 repository에 명시적으로 연결된 6개 문서의 전체 메타데이터 목록 vs 실제 WorkContextService의 task/path 선정과 repository pin/exclude를 반영한 목록. 양쪽은 같은 캐시 시점이며 동일한 {kind,key,url,version} JSON 배열 표현을 사용한다. 정답은 fixture에 직접 고정한 필수 문서 ID 및 외부 source 버전이다. 누락된 버전도 stale로 센다. stale 판정은 선택기의 자체 주장으로 만들지 않는다. 새 source 버전을 아직 확인하지 않은 경우 선택만으로 최신 버전을 알아낼 수 없다.

| 고정 사례 | 방식 | 필수 자료 누락 | 무관 자료 | stale 버전 | 입력 바이트 |
| --- | --- | ---: | ---: | ---: | ---: |
| server (T-1, src/server/api.rs) | 전체 목록 | 1 | 4 | 2 | 511 |
| server | 범위 선택 | 0 | 0 | 1 | 256 |
| UI (src/ui/main.ts) | 전체 목록 | 0 | 4 | 2 | 511 |
| UI | 범위 선택 | 0 | 0 | 0 | 171 |

Windows Rust 1.95.0, prerelease 3e6a4f5f 기반 변경 트리에서 두 사례의 기대값 및 반복 선택 동일성 확인. 입력 크기는 실제 문서 본문이나 토큰 수가 아닌 합성 메타데이터 직렬화 크기이다. 성공률이나 비용에 관한 실모델 비교는 별도의 사용자 승인 아래 선택적으로 수행하며 이 기준의 통과에 포함하지 않는다. 단일 종합 점수나 신규 제품 gate를 만들지 않는다.
