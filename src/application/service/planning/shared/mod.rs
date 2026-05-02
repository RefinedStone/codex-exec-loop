// authority_seed는 파일 workspace와 DB authority가 처음 만날 때 기본 direction/task authority를 심는 공통 로직이다.
// authoring, repair, admin 진입점이 서로 다른 경로로 초기화되더라도 같은 seed contract를 공유하게 한다.
pub mod authority_seed;
// auto_follow_copy는 자동 후속 실행 transcript 문구와 queue-idle evaluator prompt를 한곳에 모으는 사용자-facing copy 계약이다.
// runtime prompt와 conversation transcript가 같은 큐 상태 표현을 쓰도록 copy drift를 막는다.
pub mod auto_follow_copy;
// contract는 active planning 파일 경로와 directory 이름처럼 adapter/application/domain이 함께 지켜야 하는 workspace layout 계약이다.
// file adapter와 service 계층이 같은 canonical path를 기준으로 움직이게 하는 공개 경계다.
pub mod contract;
// planning_paths는 contract 경로를 실제 workspace path와 연결하는 crate-local helper다. 외부 API로 경로 조립 세부를 노출하지 않는다.
// path traversal guard와 prefix 판정을 service 내부에 묶어 adapter가 문자열 조합 정책을 중복하지 않게 한다.
pub(crate) mod planning_paths;
// prompt_sections는 runtime prompt에 들어가는 section 조각을 구성하는 내부 helper다. prompt 문자열 구조를 service 로직과 분리한다.
// worker, repair, task mutation prompt가 같은 authority contract 문단을 재사용하도록 crate-local로 제한한다.
pub(crate) mod prompt_sections;
