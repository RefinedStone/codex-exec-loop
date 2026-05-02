// SessionService는 outbound port를 trait object로 보관한다. Arc를 쓰면 TUI runtime,
// background loader, test harness가 같은 catalog capability를 cheap clone으로 공유할 수 있다.
use std::sync::Arc;

// session catalog 로딩 실패는 filesystem/app-server/adapter 실패를 포괄하므로 application service
// surface에서는 anyhow::Result로 그대로 올린다. inbound UI가 이 error를 status copy로 바꾼다.
use anyhow::Result;

// SessionCatalogPort는 application -> outbound 경계이다. service는 이 trait만 알고, 실제
// app-server bridge나 fallback adapter 구현 세부사항은 알지 않는다.
use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
// SessionCatalogRequest/SessionCatalog는 domain이 정의한 catalog 조회 계약이다. service가 이
// domain type을 그대로 받게 해 inbound adapter와 outbound adapter가 같은 요청/응답 언어를 사용한다.
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};

// SessionService는 ShellRuntime과 background task 사이에서 복제될 수 있어야 한다. Clone은 내부
// Arc handle만 복제하므로 catalog port 구현체의 실제 connection/state는 공유된다.
#[derive(Clone)]
/*
SessionService는 최근 session catalog 조회 use case의 application facade이다. TUI나
상위 shell은 "workspace 기준 session 목록을 달라"는 요청만 만들고, 실제 catalog가 local file,
app-server, sqlite, 또는 다른 outbound adapter에서 오는지는 `SessionCatalogPort` 뒤로 숨긴다.

이 service가 별도 타입으로 존재하는 이유는 얇아 보여도 domain request/response와 outbound
capability 사이의 경계를 고정하기 위해서이다. 이후 검색, paging, workspace filter 정책이
늘어나더라도 inbound UI가 adapter 구현을 직접 호출하지 않게 한다.
*/
pub struct SessionService {
    // session_catalog_port는 최근/재첨부 가능 session 목록을 읽는 유일한 outbound capability이다.
    // service 메서드는 이 field를 통해서만 외부 catalog를 접근해 adapter 방향성을 유지한다.
    session_catalog_port: Arc<dyn SessionCatalogPort>,
}

// 이 impl은 session catalog use case의 application-facing API이다. 지금은 얇은 위임이지만,
// future paging/search/workspace normalization 정책이 들어올 때 inbound adapter가 바뀌지 않게 하는 자리이다.
impl SessionService {
    // constructor는 concrete adapter를 Arc<dyn SessionCatalogPort>로 주입받는다. shell_entrypoint가
    // app-server adapter를 만들어 넘기고, tests는 fake port를 넘겨 같은 use case contract를 검증한다.
    pub fn new(session_catalog_port: Arc<dyn SessionCatalogPort>) -> Self {
        /*
        port를 `Arc<dyn ...>`로 받으면 service clone은 cheap handle clone이 된다.
        TUI state, command handler, test harness가 같은 catalog capability를 공유할 수 있고,
        application layer는 concrete outbound adapter type을 알지 않아도 된다.
        */
        Self {
            session_catalog_port,
        }
    }

    // load_session_catalog는 TUI session browser가 호출하는 application use case이다. request에는
    // limit/workspace/cursor 같은 domain-level 조회 조건이 이미 담겨 있으므로 service는 이를 port로 전달한다.
    pub fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog> {
        /*
        현재 구현은 request를 그대로 port에 위임한다. 이 pass-through는 의도적인
        application boundary이다. `SessionCatalogRequest` 안의 limit, cursor, workspace root 같은
        조회 정책은 domain type으로 고정되고, port는 그 계약을 만족하는 catalog를 반환한다.
        오류도 변환하지 않고 그대로 올려 caller가 파일 접근 실패나 adapter 실패를 같은 anyhow
        흐름으로 표시하게 한다.
        */
        self.session_catalog_port.load_session_catalog(request)
    }
}

// 이 테스트 모듈은 service가 catalog request를 변형하지 않고 outbound port에 위임하는지 고정한다.
// session browser의 workspace scoping이 깨지면 이 application boundary test에서 먼저 드러나야 한다.
#[cfg(test)]
mod tests {
    // fake port는 &self로 호출되므로 요청 기록에 interior mutability가 필요하다. Mutex는 테스트에서
    // 호출된 request sequence를 안전하게 모으는 가장 단순한 동기화 도구이다.
    use std::sync::Mutex;

    // super::*는 실제 SessionService와 port/result imports를 같은 module scope에서 테스트하게 한다.
    use super::*;
    // 테스트 fake는 빈 RecentSessions를 SessionCatalog로 변환해 성공 응답을 만든다. 핵심 검증은
    // 반환 데이터가 아니라 request delegation이므로 catalog contents는 비워 둔다.
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};

    // FakeSessionCatalogPort는 service가 넘긴 request를 기록하는 spy port이다. outbound adapter를
    // 실제로 실행하지 않고도 application service가 port contract를 어떻게 호출하는지 검증한다.
    #[derive(Default)]
    struct FakeSessionCatalogPort {
        // requests는 load_session_catalog가 받은 domain request의 원본 기록이다. 테스트는 이
        // Vec을 비교해 workspace/limit이 service에서 손실되거나 덮이지 않았음을 확인한다.
        requests: Mutex<Vec<SessionCatalogRequest>>,
    }

    // fake port도 production adapter와 같은 SessionCatalogPort trait을 구현한다. 그래서 service
    // test는 concrete adapter가 아니라 application/outbound boundary 계약을 대상으로 한다.
    impl SessionCatalogPort for FakeSessionCatalogPort {
        // load_session_catalog는 request를 기록한 뒤 빈 ready catalog를 반환한다. 실패 path가
        // 아니라 위임 shape를 보는 테스트라, 성공 응답으로 service call이 끝까지 진행되게 한다.
        fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog> {
            self.requests
                // mutex poison은 테스트 자체의 동시성 실패이므로 expect로 즉시 드러낸다.
                .lock()
                .expect("session request mutex poisoned")
                // push는 service가 포트로 넘긴 request를 그대로 저장한다. 이후 assert_eq가 domain
                // request의 limit/workspace 필드가 유지됐는지 확인한다.
                .push(request);
            // 빈 RecentSessions를 catalog로 바꾸면 provider-backed ready catalog shape를 얻는다.
            // service가 반환값을 변형하지 않는 한, caller는 정상 catalog 응답으로 처리할 수 있다.
            Ok(RecentSessions {
                // items가 비어도 catalog load 성공을 표현할 수 있다. 이 테스트는 row rendering을
                // 보지 않으므로 sample SessionSummary를 만들 필요가 없다.
                items: Vec::new(),
                // warnings도 비워 warning propagation이 아닌 request delegation에만 초점을 둔다.
                warnings: Vec::new(),
                // next_cursor가 None이면 pagination 없이 단일 page catalog로 충분한 fake 응답이다.
                next_cursor: None,
            }
            // From<RecentSessions>는 domain catalog의 기본 ready wrapping을 제공한다.
            .into())
        }
    }

    // 이 테스트는 SessionService가 capability request를 그대로 포트에 전달하는지 확인한다.
    // shell runtime이 workspace-scoped session browser를 열 때 이 경계가 깨지면 다른 workspace session이 섞일 수 있다.
    #[test]
    fn load_session_catalog_delegates_capability_request() {
        // 같은 fake port Arc를 service와 assertion side가 공유한다. service는 trait object로 쓰고,
        // test는 concrete fake handle로 requests 기록을 확인한다.
        let port = Arc::new(FakeSessionCatalogPort::default());
        // constructor가 Arc clone을 보관하므로 아래 port variable은 assertion을 위해 계속 사용할 수 있다.
        let service = SessionService::new(port.clone());

        // workspace-scoped request를 하나 실행한다. 기대값도 같은 factory를 써 domain request
        // construction policy가 테스트와 production call 양쪽에서 일치하게 한다.
        service
            .load_session_catalog(SessionCatalogRequest::for_workspace(25, "/tmp/root"))
            // fake port는 성공을 반환해야 하므로 Err가 나오면 service wiring이나 fake 구현이 잘못된 것이다.
            .expect("load session catalog should succeed");

        // 기록된 request가 정확히 하나이고, limit/workspace가 호출 입력 그대로임을 검증한다.
        // service가 filtering, defaulting, workspace replacement를 몰래 수행하면 이 assert가 실패한다.
        assert_eq!(
            *port
                .requests
                .lock()
                .expect("session request mutex poisoned"),
            vec![SessionCatalogRequest::for_workspace(25, "/tmp/root")]
        );
    }
}
