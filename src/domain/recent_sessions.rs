/*
학습 주석: 이 파일은 최근 세션 catalog의 domain contract입니다. outbound app-server adapter는
provider thread list를 `RecentSessions`로 낮추고, application service와 TUI shell chrome은
`SessionCatalog`의 상태만 보고 "목록을 열 수 있는지", "부분 기능인지", "왜 목록이 없는지"를 판단합니다.
*/
use crate::domain::session_summary::SessionSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: SessionCatalogRequest는 TUI가 "최근 세션 목록을 보여 달라"는 intent를 application boundary로
// 넘길 때 쓰는 입력 DTO입니다. domain 타입으로 두기 때문에 runtime thread, shell chrome effect,
// outbound port fake가 모두 같은 요청 shape를 검증할 수 있습니다.
pub struct SessionCatalogRequest {
    // 학습 주석: limit은 provider catalog에서 가져올 최대 세션 수입니다. TUI list가 한 화면에서 다룰 수 있는
    // 크기를 제한하고, app-server adapter에는 ThreadListParams.limit으로 전달됩니다.
    pub limit: usize,
    // 학습 주석: current_workspace_directory는 현재 shell workspace를 함께 실어 보내는 선택적 context입니다.
    // 지금 app-server compatibility adapter는 limit만 쓰지만, shell runtime tests는 이 값이 port까지
    // 보존되는지 검증해 이후 workspace-filtered catalog로 확장할 수 있게 합니다.
    pub current_workspace_directory: Option<String>,
}

impl SessionCatalogRequest {
    // 학습 주석: new는 workspace context 없이 전체 recent catalog를 요청하는 기본 constructor입니다.
    // shell chrome에서 세션 목록을 처음 열 때처럼 현재 workspace 필터가 필요 없을 때 사용합니다.
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            current_workspace_directory: None,
        }
    }

    // 학습 주석: for_workspace는 현재 작업 디렉터리를 catalog 요청과 함께 묶습니다. runtime layer가
    // ShellChromeEffect::LoadSessionCatalog를 처리할 때 workspace 문자열을 잃지 않도록 이 생성자를 씁니다.
    pub fn for_workspace(limit: usize, current_workspace_directory: impl Into<String>) -> Self {
        Self {
            limit,
            current_workspace_directory: Some(current_workspace_directory.into()),
        }
    }
}

#[derive(Debug, Clone)]
// 학습 주석: RecentSessions는 catalog가 실제로 준비됐을 때의 payload입니다. 각 item은 `SessionSummary`
// display/domain helper를 갖고, warnings와 next_cursor는 provider catalog의 부가 신호를 그대로 보존합니다.
pub struct RecentSessions {
    // 학습 주석: items는 session browser가 row projection, 검색, 선택 이동에 사용하는 주 데이터입니다.
    // shell chrome의 selection reducer도 이 길이를 기준으로 선택 index를 clamp합니다.
    pub items: Vec<SessionSummary>,
    // 학습 주석: warnings는 app-server가 catalog를 만들며 발견한 비치명 문제입니다. Ready catalog에서도
    // 경고를 유지해야 capability/status copy가 "목록은 있지만 일부 record가 불완전함"을 보여 줄 수 있습니다.
    pub warnings: Vec<String>,
    // 학습 주석: next_cursor는 provider가 pagination을 지원할 때 다음 page를 가리키는 cursor입니다.
    // 현재 UI가 load-more를 쓰지 않더라도 domain에 보존해 adapter 응답을 손실 없이 표현합니다.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 학습 주석: SessionCatalogTier는 "최근 세션 기능이 어느 수준까지 가능한가"를 나타냅니다. TUI copy는
// 이 tier를 사용해 attach-only fallback, handle 재첨부, provider catalog를 서로 다른 안내문으로 보여 줍니다.
pub enum SessionCatalogTier {
    // 학습 주석: AttachOnly는 provider가 목록 API를 제공하지 않고 사용자가 session id/handle을 직접 넣어야 하는 수준입니다.
    AttachOnly,
    // 학습 주석: HandleBasedReattach는 최근 목록은 제한적이지만 기존 handle을 기반으로 재첨부할 수 있는 중간 단계입니다.
    HandleBasedReattach,
    // 학습 주석: ProviderBackedCatalog는 app-server list_threads 같은 실제 catalog backend가 목록을 제공하는 상태입니다.
    ProviderBackedCatalog,
}

impl SessionCatalogTier {
    // 학습 주석: label은 capability/status 문구에 들어가는 안정적인 짧은 tier 이름입니다. UI 함수들이
    // enum variant 이름을 직접 문자열화하지 않게 해 copy 변경 지점을 domain helper 하나로 모읍니다.
    pub fn label(self) -> &'static str {
        match self {
            Self::AttachOnly => "attach-only",
            Self::HandleBasedReattach => "handle-based reattach",
            Self::ProviderBackedCatalog => "provider-backed catalog",
        }
    }
}

#[derive(Debug, Clone)]
// 학습 주석: SessionCatalogStatus는 목록 payload가 없거나 제한적일 때도 UI가 이유와 tier를 잃지 않도록
// 묶는 상태 객체입니다. Unsupported와 Partial이 같은 필드를 공유하므로 별도 struct로 중복을 줄입니다.
pub struct SessionCatalogStatus {
    // 학습 주석: tier는 실패/부분 성공이 어느 capability level에서 발생했는지 알려 줍니다.
    pub tier: SessionCatalogTier,
    // 학습 주석: detail은 operator-facing 설명입니다. app-server unavailable, unsupported API, fallback 사유 같은
    // 문장을 여기에 보존하고 presentation layer가 필요할 때 그대로 보여 줍니다.
    pub detail: String,
    // 학습 주석: warnings는 detail보다 낮은 심각도의 부가 정보입니다. Partial catalog에서는 사용 가능한 기능과
    // 함께 제한 사항을 나열하는 데 쓰입니다.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
// 학습 주석: SessionCatalog는 recent-session capability의 전체 결과를 세 가지 상태로 닫아 둡니다.
// shell chrome reducer와 presentation은 이 enum만 보면 목록 UI를 열지, fallback copy를 보여 줄지 결정합니다.
pub enum SessionCatalog {
    // 학습 주석: Unsupported는 catalog payload가 전혀 없고, 사용자가 attach-only 또는 다른 fallback을 따라야 하는 상태입니다.
    Unsupported(SessionCatalogStatus),
    // 학습 주석: Partial은 일부 재첨부 기능이나 진단 정보는 있지만 full RecentSessions payload는 없는 상태입니다.
    Partial(SessionCatalogStatus),
    // 학습 주석: Ready는 session browser가 실제 row를 만들 수 있는 상태입니다. tier를 payload 밖에 둬
    // provider-backed 외의 ready catalog가 생겨도 같은 shape를 유지할 수 있게 합니다.
    Ready {
        tier: SessionCatalogTier,
        recent_sessions: RecentSessions,
    },
}

impl SessionCatalog {
    // 학습 주석: unsupported constructor는 adapter/service가 "목록 없음" 상태를 만들 때 status struct 조립을
    // 반복하지 않도록 합니다. detail은 Into<String>으로 받아 테스트와 production copy 모두 간단히 넣을 수 있습니다.
    pub fn unsupported(
        tier: SessionCatalogTier,
        detail: impl Into<String>,
        warnings: Vec<String>,
    ) -> Self {
        Self::Unsupported(SessionCatalogStatus {
            tier,
            detail: detail.into(),
            warnings,
        })
    }

    // 학습 주석: partial constructor는 capability가 완전히 닫힌 것은 아니지만 browser payload가 아직 없는
    // 중간 상태를 표현합니다. presentation layer는 이 상태를 Ready처럼 row로 그리지 않고, partial 안내문으로 처리합니다.
    pub fn partial(
        tier: SessionCatalogTier,
        detail: impl Into<String>,
        warnings: Vec<String>,
    ) -> Self {
        Self::Partial(SessionCatalogStatus {
            tier,
            detail: detail.into(),
            warnings,
        })
    }

    // 학습 주석: ready constructor는 실제 session list를 enum 안에 감싸는 단일 진입점입니다.
    // app-server adapter와 tests가 이 함수를 쓰면 Ready 상태의 tier/payload 배치가 일관됩니다.
    pub fn ready(tier: SessionCatalogTier, recent_sessions: RecentSessions) -> Self {
        Self::Ready {
            tier,
            recent_sessions,
        }
    }

    // 학습 주석: tier accessor는 상태 variant와 무관하게 capability level을 꺼내게 합니다. capability copy는
    // Unsupported/Partial/Ready를 따로 match하지 않아도 같은 tier label을 만들 수 있습니다.
    pub fn tier(&self) -> SessionCatalogTier {
        match self {
            Self::Unsupported(status) | Self::Partial(status) => status.tier,
            Self::Ready { tier, .. } => *tier,
        }
    }

    // 학습 주석: detail은 목록 payload가 없는 상태에서만 의미 있는 설명입니다. Ready에서는 None을 돌려
    // renderer가 stale error/detail 문구를 ready browser 위에 겹쳐 표시하지 않도록 합니다.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Unsupported(status) | Self::Partial(status) => Some(status.detail.as_str()),
            Self::Ready { .. } => None,
        }
    }

    // 학습 주석: warnings accessor는 status 기반 catalog와 ready payload의 warning 위치 차이를 감춥니다.
    // capability projection과 session browser copy는 이 함수만 호출해 모든 상태의 경고를 같은 방식으로 읽습니다.
    pub fn warnings(&self) -> &[String] {
        match self {
            Self::Unsupported(status) | Self::Partial(status) => status.warnings.as_slice(),
            Self::Ready {
                recent_sessions, ..
            } => recent_sessions.warnings.as_slice(),
        }
    }

    // 학습 주석: recent_sessions는 Ready catalog에서만 browser projection이 필요한 payload를 빌려 줍니다.
    // shell chrome reducer는 None이면 selection 이동을 중단하고, renderer는 unsupported/partial panel을 그립니다.
    pub fn recent_sessions(&self) -> Option<&RecentSessions> {
        match self {
            Self::Ready {
                recent_sessions, ..
            } => Some(recent_sessions),
            Self::Unsupported(_) | Self::Partial(_) => None,
        }
    }
}

// 학습 주석: From<RecentSessions>는 provider-backed ready catalog가 기본 성공 shape라는 convention을
// 코드로 고정합니다. tests와 fake ports가 간단히 `RecentSessions.into()`를 써도 production adapter와
// 같은 tier를 얻게 하는 작은 연결점입니다.
impl From<RecentSessions> for SessionCatalog {
    fn from(recent_sessions: RecentSessions) -> Self {
        Self::ready(SessionCatalogTier::ProviderBackedCatalog, recent_sessions)
    }
}
