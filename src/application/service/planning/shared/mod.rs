// 학습 주석: authority_seed는 파일 workspace와 DB authority가 처음 만날 때 기본 direction/task authority를 심는 공통 로직입니다.
pub mod authority_seed;
// 학습 주석: auto_follow_copy는 자동 후속 실행 transcript 문구와 queue-idle evaluator prompt를 한곳에 모으는 사용자-facing copy 계약입니다.
pub mod auto_follow_copy;
// 학습 주석: contract는 active planning 파일 경로와 directory 이름처럼 adapter/application/domain이 함께 지켜야 하는 workspace layout 계약입니다.
pub mod contract;
// 학습 주석: planning_paths는 contract 경로를 실제 workspace path와 연결하는 crate-local helper입니다. 외부 API로 경로 조립 세부를 노출하지 않습니다.
pub(crate) mod planning_paths;
// 학습 주석: prompt_sections는 runtime prompt에 들어가는 section 조각을 구성하는 내부 helper입니다. prompt 문자열 구조를 service 로직과 분리합니다.
pub(crate) mod prompt_sections;
