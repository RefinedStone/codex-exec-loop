// 학습 주석: facade는 runtime prompt, reconciliation, policy, handoff 조립을 하나의 application 진입점으로 묶습니다. 외부 use case는
// 세부 runtime 부품 대신 이 facade를 통해 턴 실행 관련 판단을 요청합니다.
pub(crate) mod facade;
// 학습 주석: intake는 사용자 prompt에서 planning task draft/proposal/commit 흐름을 만드는 runtime 입력 계층입니다.
pub(crate) mod intake;
// 학습 주석: policy는 auto-follow 가능 여부, workspace invalid 처리, queue head 필요 조건처럼 실행 결정을 순수 규칙으로 분리합니다.
pub(crate) mod policy;
// 학습 주석: prompt는 workspace 파일과 task authority를 읽어 runtime snapshot과 worker prompt context로 바꾸는 읽기 모델 계층입니다.
pub(crate) mod prompt;
// 학습 주석: validation은 runtime/intake/repair가 공유하는 planning 상태 검증 규칙입니다. invalid snapshot과 task 입력 오류의 기준점입니다.
pub(crate) mod validation;
