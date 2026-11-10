use anyhow::Result;

/*
 * 스레드 기록에서 글만 읽는 경계이다 (plan 22). 작업 컨텍스트 수집기가 쓴다. 모델을 부르지 않는 조회이다.
 */

/// 스레드 아이템 하나에서 꺼낸 글이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadTextFragment {
    /// 누가 쓴 글인지이다: `user`, `agent`, `tool`.
    pub source: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThreadTextPage {
    /// 오래된 것부터이다.
    pub fragments: Vec<ThreadTextFragment>,
    /// 가장 새 아이템의 id 이다. 새 아이템이 없으면 `None` 이다.
    pub newest_item_id: Option<String>,
}

pub trait ThreadTranscriptPort: Send + Sync {
    /// `after_item_id` 뒤에 생긴 아이템의 글을 읽는다. `None` 이면 스레드 전체를 읽는다.
    fn text_after(&self, thread_id: &str, after_item_id: Option<&str>) -> Result<ThreadTextPage>;
}
