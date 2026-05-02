// 학습 주석: construction은 planning admin facade가 필요로 하는 port와 저장소 의존성을 조립하는
// 생성 경로입니다. admin service의 public API를 쓰는 쪽은 이 세부 wiring을 직접 알 필요가 없습니다.
mod construction;
// 학습 주석: crud는 planning task와 direction 같은 관리 대상의 생성, 조회, 수정, 삭제 흐름을
// 담당하는 하위 service 영역입니다.
mod crud;
// 학습 주석: direction_mutation은 planning direction 문서의 의도적 변경을 따로 모읍니다. direction은
// agent 작업 방향을 바꾸는 값이라 일반 task CRUD보다 더 좁은 의미의 mutation으로 분리합니다.
mod direction_mutation;
// 학습 주석: documents는 active document와 draft document처럼 planning workspace의 파일 기반
// 문서 상태를 application service 관점에서 다루는 영역입니다.
mod documents;
// 학습 주석: draft_session은 사용자가 planning draft를 만들고 이어서 편집하는 세션 단위 흐름을
// 관리합니다. 단순 파일 저장이 아니라 작업 중인 초안의 lifecycle을 표현합니다.
mod draft_session;
// 학습 주석: facade는 위 하위 service들을 하나의 `PlanningAdminFacadeService`로 묶는 application
// entry point입니다. inbound adapter는 보통 이 facade만 의존합니다.
mod facade;
// 학습 주석: file_sync는 DB authority와 workspace file 상태를 맞추는 동기화 흐름입니다. planning
// admin이 파일과 authoritative storage 사이의 drift를 다룰 때 이 영역을 통과합니다.
mod file_sync;
// 학습 주석: overview는 admin 화면이나 API가 한 번에 보여 줄 요약 projection을 조립합니다.
// 세부 mutation과 분리해 read model 성격을 분명히 합니다.
mod overview;
// 학습 주석: projection은 domain/application 상태를 inbound adapter가 쓰기 좋은 view 형태로
// 변환하는 mapping layer입니다.
mod projection;
// 학습 주석: reset은 planning workspace나 authority state를 재초기화하는 위험도가 높은 관리 동작을
// 따로 모읍니다. 일반 mutation과 분리해 호출 의도를 더 잘 드러냅니다.
mod reset;
// 학습 주석: surface는 facade 바깥으로 노출되는 Request/Response/State DTO를 모으는 public contract
// 영역입니다. 하위 구현 module의 private type이 inbound adapter로 새지 않게 합니다.
mod surface;

// 학습 주석: facade service re-export는 caller가 `planning::admin::PlanningAdminFacadeService`만
// import하면 되게 해 줍니다. 내부 파일 구조를 public dependency로 만들지 않는 module 표면입니다.
pub use self::facade::PlanningAdminFacadeService;
// 학습 주석: surface DTO 전체를 re-export해 inbound adapter가 admin command/result type을 이
// module 경계에서 가져오게 합니다. 구현 module은 숨기고 contract type만 넓게 공개하는 역할입니다.
pub use self::surface::*;
