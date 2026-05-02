/*
 * Admin module map은 inbound admin API, Telegram control surface, CLI control path가 의존하는
 * application-layer 관리 표면을 한곳에 모은다. 하위 파일은 구현 책임별로 private하게 나누고,
 * 이 파일은 facade와 DTO만 public re-export해 adapter가 내부 service 분해에 묶이지 않게 한다.
 */

// construction은 planning admin facade가 필요로 하는 port와 저장소 의존성을 조립하는 생성 경로다.
// admin service의 public API를 쓰는 쪽은 이 세부 wiring을 직접 알 필요가 없다.
mod construction;
// crud는 planning task와 direction 같은 관리 대상의 생성, 조회, 수정, 삭제 흐름을 담당하는 하위 service 영역이다.
// direction/task mutation의 결과를 다시 management read model로 돌려주는 admin command layer다.
mod crud;
// direction_mutation은 planning direction 문서의 의도적 변경을 따로 모은다. direction은 agent 작업 방향을 바꾸는 값이라
// 일반 task CRUD보다 더 좁은 의미의 mutation으로 분리한다.
mod direction_mutation;
// documents는 active document와 draft document처럼 planning workspace의 파일 기반 문서 상태를 application service 관점에서 다룬다.
// admin draft/session flow가 raw filesystem layout을 직접 알지 않도록 문서 단위 helper를 제공한다.
mod documents;
// draft_session은 사용자가 planning draft를 만들고 이어서 편집하는 세션 단위 흐름을 관리한다.
// 단순 파일 저장이 아니라 작업 중인 초안의 lifecycle과 validation/queue preview를 함께 표현한다.
mod draft_session;
// facade는 위 하위 service들을 하나의 `PlanningAdminFacadeService`로 묶는 application entry point다.
// inbound adapter는 보통 이 facade만 의존하고, 내부 module 간 협업은 facade method 뒤로 숨긴다.
mod facade;
// file_sync는 DB authority와 workspace file 상태를 맞추는 동기화 흐름이다.
// planning admin이 파일과 authoritative storage 사이의 drift를 다룰 때 이 영역을 통과한다.
mod file_sync;
// overview는 admin 화면이나 API가 한 번에 보여 줄 요약 projection을 조립한다.
// 세부 mutation과 분리해 read model 성격을 분명히 한다.
mod overview;
// projection은 domain/application 상태를 inbound adapter가 쓰기 좋은 view 형태로 변환하는 mapping layer다.
// domain type을 HTML/API DTO로 직접 노출하지 않는 adapter-facing 변환 책임을 맡는다.
mod projection;
// reset은 planning workspace나 authority state를 재초기화하는 위험도가 높은 관리 동작을 따로 모은다.
// 일반 mutation과 분리해 호출 의도를 더 잘 드러낸다.
mod reset;
// surface는 facade 바깥으로 노출되는 Request/Response/State DTO를 모으는 public contract 영역이다.
// 하위 구현 module의 private type이 inbound adapter로 새지 않게 한다.
mod surface;

// facade service re-export는 caller가 `planning::admin::PlanningAdminFacadeService`만 import하면 되게 해 준다.
// 내부 파일 구조를 public dependency로 만들지 않는 module 표면이다.
pub use self::facade::PlanningAdminFacadeService;
// surface DTO 전체를 re-export해 inbound adapter가 admin command/result type을 이 module 경계에서 가져오게 한다.
// 구현 module은 숨기고 contract type만 넓게 공개하는 역할이다.
pub use self::surface::*;
