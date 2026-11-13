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
    /// dot 과 사용자가 대화하는 메시지 방이다. 없으면 대화를 읽을 수 없다.
    pub messaging_room_id: Option<String>,
}

/// 메시지 방의 메시지 하나이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DotMessage {
    pub id: String,
    /// 서비스가 준 시각 글자 그대로이다 (RFC 3339).
    pub created_at: String,
    /// 보낸 쪽의 이름이다. 방의 구성원 목록에 없으면 빈 글자이다.
    pub author: String,
    /// dot 이 보낸 메시지이다. false 이면 사람이 보냈다.
    pub from_dot: bool,
    pub text: String,
}

pub trait DotsPort: Send + Sync {
    /// 사용자의 dot 목록이다. 로그인이 없거나 서비스가 거절하면 오류이다.
    fn list_dots(&self) -> Result<Vec<DotSummary>>;

    /// 메시지 방의 최근 메시지이다. 오래된 것부터 놓는다. 읽기만 한다.
    fn room_messages(&self, room_id: &str) -> Result<Vec<DotMessage>>;

    /// 메시지 방에 글 하나를 보낸다. 실계정 쓰기이다: dot 이 읽고 답한다. 사용자가 허락한 뒤에만 부른다 (D-19).
    fn send_room_message(&self, room_id: &str, text: &str) -> Result<()>;
}
