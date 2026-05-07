// bootstrap은 새 planning workspace를 만들고 기본 supporting file을 seed하는 작성 시작점이다. authoring 안에서도
// 초기화 전용 책임이라 별도 모듈로 둔다.
pub(crate) mod bootstrap;
// directions는 planning direction 문서, queue-idle prompt, supporting file summary를 다룬다. 사용자가 작업 방향을
// 검토하고 선택하는 authoring의 중심 모듈이다.
pub(crate) mod directions;
// init은 TUI planning init overlay가 draft editor와 promote/save flow를 열 때 쓰는 application service다.
// inbound UI가 파일 포맷 세부 대신 draft session 중심의 use case를 호출하게 하는 경계다.
pub(crate) mod init;
// proposal_promotion은 작성된 draft/proposal 파일을 active planning 문서로 승격한다. authoring 결과를 runtime이 읽는
// authoritative workspace 문서로 넘기는 경계다.
pub(crate) mod proposal_promotion;
