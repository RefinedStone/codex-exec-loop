// session catalog 조회는 app-server/provider/session store 같은 외부 경계에 닿으므로 실패할 수 있다.
// 오류는 `SessionService`를 거쳐 TUI background message로 올라가 session overlay 상태에 반영된다.
use anyhow::Result;

// request와 catalog는 domain recent-sessions 모델이다. port가 adapter 전용 DTO를 노출하지 않기 때문에
// TUI는 catalog 출처가 app-server인지, provider-backed store인지, fake test port인지 구분하지 않아도 된다.
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};

// `SessionCatalogPort`는 최근/재첨부 가능한 session 목록을 읽는 outbound 계약이다.
// `SessionService`는 이 trait 하나만 보고 catalog를 요청하고, app-server adapter는 legacy `CodexAppServerPort`
// 구현을 blanket impl로 이 작은 use-case port에 연결한다.
//
// startup probe, interactive turn runtime과 별도 port로 나눈 이유는 TUI가 session overlay를 열 때
// 긴 turn stream과 무관하게 짧은 catalog 조회만 수행할 수 있게 하기 위해서이다. 테스트도 fake port로 request mapping만
// 좁게 검증할 수 있다.
pub trait SessionCatalogPort: Send + Sync {
    // 주어진 workspace/filter request에 맞는 session catalog를 읽는다.
    // 반환값에는 catalog tier, session rows, unavailable reason 같은 domain projection이 들어가며,
    // TUI rendering은 이 값을 그대로 session overlay와 status line으로 바꾼다.
    fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog>;
}
