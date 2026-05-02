// 학습 주석: SessionService는 outbound port를 trait object로 보관합니다. Arc를 쓰면 TUI runtime,
// background loader, test harness가 같은 catalog capability를 cheap clone으로 공유할 수 있습니다.
use std::sync::Arc;

// 학습 주석: session catalog 로딩 실패는 filesystem/app-server/adapter 실패를 포괄하므로 application service
// surface에서는 anyhow::Result로 그대로 올립니다. inbound UI가 이 error를 status copy로 바꿉니다.
use anyhow::Result;

// 학습 주석: SessionCatalogPort는 application -> outbound 경계입니다. service는 이 trait만 알고, 실제
// app-server bridge나 fallback adapter 구현 세부사항은 알지 않습니다.
use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
// 학습 주석: SessionCatalogRequest/SessionCatalog는 domain이 정의한 catalog 조회 계약입니다. service가 이
// domain type을 그대로 받게 해 inbound adapter와 outbound adapter가 같은 요청/응답 언어를 사용합니다.
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};

// 학습 주석: SessionService는 ShellRuntime과 background task 사이에서 복제될 수 있어야 합니다. Clone은 내부
// Arc handle만 복제하므로 catalog port 구현체의 실제 connection/state는 공유됩니다.
#[derive(Clone)]
/*
학습 주석: SessionService는 최근 session catalog 조회 use case의 application facade입니다. TUI나
상위 shell은 "workspace 기준 session 목록을 달라"는 요청만 만들고, 실제 catalog가 local file,
app-server, sqlite, 또는 다른 outbound adapter에서 오는지는 `SessionCatalogPort` 뒤로 숨깁니다.

이 service가 별도 타입으로 존재하는 이유는 얇아 보여도 domain request/response와 outbound
capability 사이의 경계를 고정하기 위해서입니다. 이후 검색, paging, workspace filter 정책이
늘어나더라도 inbound UI가 adapter 구현을 직접 호출하지 않게 합니다.
*/
pub struct SessionService {
    // 학습 주석: session_catalog_port는 최근/재첨부 가능 session 목록을 읽는 유일한 outbound capability입니다.
    // service 메서드는 이 field를 통해서만 외부 catalog를 접근해 adapter 방향성을 유지합니다.
    session_catalog_port: Arc<dyn SessionCatalogPort>,
}

// 학습 주석: 이 impl은 session catalog use case의 application-facing API입니다. 지금은 얇은 위임이지만,
// future paging/search/workspace normalization 정책이 들어올 때 inbound adapter가 바뀌지 않게 하는 자리입니다.
impl SessionService {
    // 학습 주석: constructor는 concrete adapter를 Arc<dyn SessionCatalogPort>로 주입받습니다. shell_entrypoint가
    // app-server adapter를 만들어 넘기고, tests는 fake port를 넘겨 같은 use case contract를 검증합니다.
    pub fn new(session_catalog_port: Arc<dyn SessionCatalogPort>) -> Self {
        /*
        학습 주석: port를 `Arc<dyn ...>`로 받으면 service clone은 cheap handle clone이 됩니다.
        TUI state, command handler, test harness가 같은 catalog capability를 공유할 수 있고,
        application layer는 concrete outbound adapter type을 알지 않아도 됩니다.
        */
        Self {
            session_catalog_port,
        }
    }

    // 학습 주석: load_session_catalog는 TUI session browser가 호출하는 application use case입니다. request에는
    // limit/workspace/cursor 같은 domain-level 조회 조건이 이미 담겨 있으므로 service는 이를 port로 전달합니다.
    pub fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog> {
        /*
        학습 주석: 현재 구현은 request를 그대로 port에 위임합니다. 이 pass-through는 의도적인
        application boundary입니다. `SessionCatalogRequest` 안의 limit, cursor, workspace root 같은
        조회 정책은 domain type으로 고정되고, port는 그 계약을 만족하는 catalog를 반환합니다.
        오류도 변환하지 않고 그대로 올려 caller가 파일 접근 실패나 adapter 실패를 같은 anyhow
        흐름으로 표시하게 합니다.
        */
        self.session_catalog_port.load_session_catalog(request)
    }
}

// 학습 주석: 이 테스트 모듈은 service가 catalog request를 변형하지 않고 outbound port에 위임하는지 고정합니다.
// session browser의 workspace scoping이 깨지면 이 application boundary test에서 먼저 드러나야 합니다.
#[cfg(test)]
mod tests {
    // 학습 주석: fake port는 &self로 호출되므로 요청 기록에 interior mutability가 필요합니다. Mutex는 테스트에서
    // 호출된 request sequence를 안전하게 모으는 가장 단순한 동기화 도구입니다.
    use std::sync::Mutex;

    // 학습 주석: super::*는 실제 SessionService와 port/result imports를 같은 module scope에서 테스트하게 합니다.
    use super::*;
    // 학습 주석: 테스트 fake는 빈 RecentSessions를 SessionCatalog로 변환해 성공 응답을 만듭니다. 핵심 검증은
    // 반환 데이터가 아니라 request delegation이므로 catalog contents는 비워 둡니다.
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};

    // 학습 주석: FakeSessionCatalogPort는 service가 넘긴 request를 기록하는 spy port입니다. outbound adapter를
    // 실제로 실행하지 않고도 application service가 port contract를 어떻게 호출하는지 검증합니다.
    #[derive(Default)]
    struct FakeSessionCatalogPort {
        // 학습 주석: requests는 load_session_catalog가 받은 domain request의 원본 기록입니다. 테스트는 이
        // Vec을 비교해 workspace/limit이 service에서 손실되거나 덮이지 않았음을 확인합니다.
        requests: Mutex<Vec<SessionCatalogRequest>>,
    }

    // 학습 주석: fake port도 production adapter와 같은 SessionCatalogPort trait을 구현합니다. 그래서 service
    // test는 concrete adapter가 아니라 application/outbound boundary 계약을 대상으로 합니다.
    impl SessionCatalogPort for FakeSessionCatalogPort {
        // 학습 주석: load_session_catalog는 request를 기록한 뒤 빈 ready catalog를 반환합니다. 실패 path가
        // 아니라 위임 shape를 보는 테스트라, 성공 응답으로 service call이 끝까지 진행되게 합니다.
        fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog> {
            self.requests
                // 학습 주석: mutex poison은 테스트 자체의 동시성 실패이므로 expect로 즉시 드러냅니다.
                .lock()
                .expect("session request mutex poisoned")
                // 학습 주석: push는 service가 포트로 넘긴 request를 그대로 저장합니다. 이후 assert_eq가 domain
                // request의 limit/workspace 필드가 유지됐는지 확인합니다.
                .push(request);
            // 학습 주석: 빈 RecentSessions를 catalog로 바꾸면 provider-backed ready catalog shape를 얻습니다.
            // service가 반환값을 변형하지 않는 한, caller는 정상 catalog 응답으로 처리할 수 있습니다.
            Ok(RecentSessions {
                // 학습 주석: items가 비어도 catalog load 성공을 표현할 수 있습니다. 이 테스트는 row rendering을
                // 보지 않으므로 sample SessionSummary를 만들 필요가 없습니다.
                items: Vec::new(),
                // 학습 주석: warnings도 비워 warning propagation이 아닌 request delegation에만 초점을 둡니다.
                warnings: Vec::new(),
                // 학습 주석: next_cursor가 None이면 pagination 없이 단일 page catalog로 충분한 fake 응답입니다.
                next_cursor: None,
            }
            // 학습 주석: From<RecentSessions>는 domain catalog의 기본 ready wrapping을 제공합니다.
            .into())
        }
    }

    // 학습 주석: 이 테스트는 SessionService가 capability request를 그대로 포트에 전달하는지 확인합니다.
    // shell runtime이 workspace-scoped session browser를 열 때 이 경계가 깨지면 다른 workspace session이 섞일 수 있습니다.
    #[test]
    fn load_session_catalog_delegates_capability_request() {
        // 학습 주석: 같은 fake port Arc를 service와 assertion side가 공유합니다. service는 trait object로 쓰고,
        // test는 concrete fake handle로 requests 기록을 확인합니다.
        let port = Arc::new(FakeSessionCatalogPort::default());
        // 학습 주석: constructor가 Arc clone을 보관하므로 아래 port variable은 assertion을 위해 계속 사용할 수 있습니다.
        let service = SessionService::new(port.clone());

        // 학습 주석: workspace-scoped request를 하나 실행합니다. 기대값도 같은 factory를 써 domain request
        // construction policy가 테스트와 production call 양쪽에서 일치하게 합니다.
        service
            .load_session_catalog(SessionCatalogRequest::for_workspace(25, "/tmp/root"))
            // 학습 주석: fake port는 성공을 반환해야 하므로 Err가 나오면 service wiring이나 fake 구현이 잘못된 것입니다.
            .expect("load session catalog should succeed");

        // 학습 주석: 기록된 request가 정확히 하나이고, limit/workspace가 호출 입력 그대로임을 검증합니다.
        // service가 filtering, defaulting, workspace replacement를 몰래 수행하면 이 assert가 실패합니다.
        assert_eq!(
            *port
                .requests
                .lock()
                .expect("session request mutex poisoned"),
            vec![SessionCatalogRequest::for_workspace(25, "/tmp/root")]
        );
    }
}
