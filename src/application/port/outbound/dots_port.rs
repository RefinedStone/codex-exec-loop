// dots port 는 사용자의 클라우드 에이전트(dot) 목록을 읽는 능력이다 (D-19). 어느 서비스에 어떤 요청으로 묻는지는
// outbound adapter 가 소유한다. Gateway 는 이 목록을 `akra/dots/list` 로 클라이언트에 준다.
use anyhow::Result;
use serde::Serialize;

/// 사이드바에 보일 dot 하나이다. 외부 서비스의 필드 이름은 adapter 가 이 모양으로 옮긴다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DotSummary {
    pub id: String,
    pub display_name: String,
    /// 서비스가 준 상태 글자 그대로이다 (예: `active`).
    pub status: String,
    /// 에이전트 종류이다 (`orbit` = dot).
    pub kind: String,
    pub paused: bool,
    /// dot 의 현재 작업 thread 이다. 클라우드 호스트의 `thread/read` 로 연다.
    pub root_thread_id: Option<String>,
}

pub trait DotsPort: Send + Sync {
    /// 사용자의 dot 목록이다. 로그인이 없거나 서비스가 거절하면 오류이다.
    fn list_dots(&self) -> Result<Vec<DotSummary>>;
}
