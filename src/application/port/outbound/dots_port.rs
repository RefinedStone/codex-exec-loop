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

/// 메시지 방의 실시간 상태이다. dot 이 답을 만드는 중인지 알려 준다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DotLiveState {
    pub room_id: String,
    /// 실시간 통로가 붙어 있다. false 이면 아래 값은 믿을 수 없다. 다시 붙으면 true 가 온다.
    pub connected: bool,
    /// dot 이 지금 답을 만들고 있다.
    pub working: bool,
    /// 서비스가 준 상태 글자이다 (예: `Thinking`). 일하는 중이 아니면 없다.
    pub activity_label: Option<String>,
    /// dot 이 클라우드 컴퓨터를 쓰고 있다.
    pub using_computer: bool,
}

impl DotLiveState {
    /// 통로가 붙었다. 서비스의 첫 상태가 오기 전이다.
    pub fn connected(room_id: &str) -> Self {
        Self {
            room_id: room_id.to_string(),
            connected: true,
            working: false,
            activity_label: None,
            using_computer: false,
        }
    }

    pub fn disconnected(room_id: &str) -> Self {
        Self {
            connected: false,
            ..Self::connected(room_id)
        }
    }
}

/// 실시간 상태를 받는 쪽이다. 상태가 바뀔 때마다 불린다.
pub type DotLiveListener = Box<dyn Fn(DotLiveState) + Send + Sync>;

/// 방의 메시지와, dot 이 어디까지 읽었는지이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DotRoomMessages {
    pub messages: Vec<DotMessage>,
    /// dot 이 방을 마지막으로 읽은 시각이다 (RFC 3339). 이 시각 이전의 내 메시지는 "읽음"이다. 모르면 없다.
    pub dot_read_at: Option<String>,
}

pub trait DotsPort: Send + Sync {
    /// 사용자의 dot 목록이다. 로그인이 없거나 서비스가 거절하면 오류이다.
    fn list_dots(&self) -> Result<Vec<DotSummary>>;

    /// 메시지 방의 최근 메시지이다. 오래된 것부터 놓는다. 읽기만 한다.
    fn room_messages(&self, room_id: &str) -> Result<DotRoomMessages>;

    /// 방의 실시간 상태를 본다. 읽기만 한다. 돌려준 값을 버리면 멈춘다. tokio 런타임 안에서 부른다.
    fn watch_room(&self, room_id: &str, on_state: DotLiveListener) -> Result<Box<dyn Send>>;

    /// 메시지 방에 글 하나를 보낸다. 실계정 쓰기이다: dot 이 읽고 답한다. 사용자가 허락한 뒤에만 부른다 (D-19).
    fn send_room_message(&self, room_id: &str, text: &str) -> Result<()>;
}
