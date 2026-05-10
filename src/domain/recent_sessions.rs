/*
이 파일은 최근 세션 catalog의 domain contract이다. outbound app-server adapter는
provider thread list를 `RecentSessions`로 낮추고, application service와 TUI shell chrome은
`SessionCatalog`의 상태만 보고 "목록을 열 수 있는지", "부분 기능인지", "왜 목록이 없는지"를 판단한다.
*/
use crate::domain::session_summary::SessionSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
// SessionCatalogRequest는 TUI가 "최근 세션 목록을 보여 달라"는 intent를 application boundary로
// 넘길 때 쓰는 입력 DTO이다. domain 타입으로 두기 때문에 runtime thread, shell chrome effect,
// outbound port fake가 모두 같은 요청 shape를 검증할 수 있다.
pub struct SessionCatalogRequest {
    // limit은 provider catalog에서 가져올 최대 세션 수이다. TUI list가 한 화면에서 다룰 수 있는
    // 크기를 제한하고, app-server adapter에는 ThreadListParams.limit으로 전달된다.
    pub limit: usize,
    // current_workspace_directory는 현재 shell workspace를 함께 실어 보내는 선택적 context이다.
    // 지금 app-server compatibility adapter는 limit만 쓰지만, shell runtime tests는 이 값이 port까지
    // 보존되는지 검증해 이후 workspace-filtered catalog로 확장할 수 있게 한다.
    pub current_workspace_directory: Option<String>,
}

impl SessionCatalogRequest {
    /*
     * `for_workspace` preserves the shell's current workspace alongside the provider
     * limit. Some adapters still ignore this field, but the request shape carries it
     * through application tests so workspace-scoped catalog behavior can be added
     * without changing inbound effect contracts later.
     */
    pub fn for_workspace(limit: usize, current_workspace_directory: impl Into<String>) -> Self {
        Self {
            limit,
            current_workspace_directory: Some(current_workspace_directory.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// RecentSessions는 catalog가 실제로 준비됐을 때의 payload이다. 각 item은 `SessionSummary`
// display/domain helper를 갖고, warnings와 next_cursor는 provider catalog의 부가 신호를 그대로 보존한다.
pub struct RecentSessions {
    // items는 session browser가 row projection, 검색, 선택 이동에 사용하는 주 데이터이다.
    // shell chrome의 selection reducer도 이 길이를 기준으로 선택 index를 clamp한다.
    pub items: Vec<SessionSummary>,
    // warnings는 app-server가 catalog를 만들며 발견한 비치명 문제이다. Ready catalog에서도
    // 경고를 유지해야 capability/status copy가 "목록은 있지만 일부 record가 불완전함"을 보여 줄 수 있다.
    pub warnings: Vec<String>,
    // next_cursor는 provider가 pagination을 지원할 때 다음 page를 가리키는 cursor이다.
    // 현재 UI가 load-more를 쓰지 않더라도 domain에 보존해 adapter 응답을 손실 없이 표현한다.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// SessionCatalogTier는 "최근 세션 기능이 어느 수준까지 가능한가"를 나타낸다. TUI copy는
// 이 tier를 사용해 attach-only fallback, handle 재첨부, provider catalog를 서로 다른 안내문으로 보여 준다.
pub enum SessionCatalogTier {
    // AttachOnly는 provider가 목록 API를 제공하지 않고 사용자가 session id/handle을 직접 넣어야 하는 수준이다.
    AttachOnly,
    // HandleBasedReattach는 최근 목록은 제한적이지만 기존 handle을 기반으로 재첨부할 수 있는 중간 단계이다.
    HandleBasedReattach,
    // ProviderBackedCatalog는 app-server list_threads 같은 실제 catalog backend가 목록을 제공하는 상태이다.
    ProviderBackedCatalog,
}

impl SessionCatalogTier {
    /*
     * Labels are stable product copy, not derived Rust variant names. Keeping the
     * strings here lets capability panels, inline tails, and tests agree on the same
     * tier wording even if enum names or presentation surfaces change.
     */
    pub fn label(self) -> &'static str {
        match self {
            Self::AttachOnly => "attach-only",
            Self::HandleBasedReattach => "handle-based reattach",
            Self::ProviderBackedCatalog => "provider-backed catalog",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// SessionCatalogStatus는 목록 payload가 없거나 제한적일 때도 UI가 이유와 tier를 잃지 않도록
// 묶는 상태 객체이다. Unsupported와 Partial이 같은 필드를 공유하므로 별도 struct로 중복을 줄인다.
pub struct SessionCatalogStatus {
    // tier는 실패/부분 성공이 어느 capability level에서 발생했는지 알려 준다.
    pub tier: SessionCatalogTier,
    // detail은 operator-facing 설명이다. app-server unavailable, unsupported API, fallback 사유 같은
    // 문장을 여기에 보존하고 presentation layer가 필요할 때 그대로 보여 준다.
    pub detail: String,
    // warnings는 detail보다 낮은 심각도의 부가 정보이다. Partial catalog에서는 사용 가능한 기능과
    // 함께 제한 사항을 나열하는 데 쓰인다.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// SessionCatalog는 recent-session capability의 전체 결과를 세 가지 상태로 닫아 둔다.
// shell chrome reducer와 presentation은 이 enum만 보면 목록 UI를 열지, fallback copy를 보여 줄지 결정한다.
pub enum SessionCatalog {
    // Unsupported는 catalog payload가 전혀 없고, 사용자가 attach-only 또는 다른 fallback을 따라야 하는 상태이다.
    Unsupported(SessionCatalogStatus),
    // Partial은 일부 재첨부 기능이나 진단 정보는 있지만 full RecentSessions payload는 없는 상태이다.
    Partial(SessionCatalogStatus),
    // Ready는 session browser가 실제 row를 만들 수 있는 상태이다. tier를 payload 밖에 둬
    // provider-backed 외의 ready catalog가 생겨도 같은 shape를 유지할 수 있게 한다.
    Ready {
        tier: SessionCatalogTier,
        recent_sessions: RecentSessions,
    },
}

impl SessionCatalog {
    /*
     * Unsupported is a deliberate capability result, not an error. Adapters use this
     * constructor when no row payload can be produced but the UI should still explain
     * the available fallback tier and any diagnostics.
     */
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

    /*
     * Partial keeps degraded capability separate from full browser readiness. It is
     * useful for handle-based reattach or future providers that can expose diagnostic
     * context before they can return a complete RecentSessions payload.
     */
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

    /*
     * Ready is the only variant that carries row data. Keeping construction centralized
     * prevents callers from drifting on where tier lives versus where provider warning
     * payload lives inside RecentSessions.
     */
    pub fn ready(tier: SessionCatalogTier, recent_sessions: RecentSessions) -> Self {
        Self::Ready {
            tier,
            recent_sessions,
        }
    }

    /*
     * Tier is the cross-variant capability signal. Presentation code can ask for it
     * without caring whether the catalog is unsupported, partial, or ready, which keeps
     * fallback copy and ready browser chrome aligned.
     */
    pub fn tier(&self) -> SessionCatalogTier {
        match self {
            Self::Unsupported(status) | Self::Partial(status) => status.tier,
            Self::Ready { tier, .. } => *tier,
        }
    }

    /*
     * Warnings live in different structs depending on state, but callers should treat
     * them as a single diagnostic stream. This accessor hides storage differences so
     * capability projection does not duplicate enum-specific warning plumbing.
     */
    pub fn warnings(&self) -> &[String] {
        match self {
            Self::Unsupported(status) | Self::Partial(status) => status.warnings.as_slice(),
            Self::Ready {
                recent_sessions, ..
            } => recent_sessions.warnings.as_slice(),
        }
    }

    /*
     * Borrowing RecentSessions only from Ready makes row projection an explicit
     * capability check. Input handlers can stop selection movement on None, while
     * renderers can switch to unsupported/partial panels without inspecting payload
     * internals.
     */
    pub fn recent_sessions(&self) -> Option<&RecentSessions> {
        match self {
            Self::Ready {
                recent_sessions, ..
            } => Some(recent_sessions),
            Self::Unsupported(_) | Self::Partial(_) => None,
        }
    }
}

/*
 * From<RecentSessions> encodes the convention that a raw provider payload is a
 * provider-backed ready catalog. Test fakes and adapters can use `.into()` without
 * accidentally producing an attach-only or partial tier.
 */
impl From<RecentSessions> for SessionCatalog {
    fn from(recent_sessions: RecentSessions) -> Self {
        Self::ready(SessionCatalogTier::ProviderBackedCatalog, recent_sessions)
    }
}
