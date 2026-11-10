use anyhow::Result;

use crate::domain::kernel_event::{KernelEvent, KernelEventPage};

/*
 * 커널의 판단 사건을 저장소별 추가 전용 기록에 쓰고 읽는다 (plan 19 Q3-1). 작업 공간으로 그 저장소의 기록을 찾는다.
 * 읽을 때는 기록 순서를 지킨다. 이 binary 가 모르는 사건(더 새 binary 가 쓴 것)은 건너뛴다.
 */
pub trait KernelEventLogPort: Send + Sync {
    /// 사건을 쓴다. 남김 종류가 아닌 오래된 사건은 최근 `RETAINED_KERNEL_EVENTS` 개만 남긴다 (Q3-3).
    fn append_kernel_events(&self, workspace_dir: &str, events: &[KernelEvent]) -> Result<()>;

    /// 이 종류(`KernelEvent::kind`)의 사건만 기록 순서대로 읽는다.
    fn load_kernel_events(&self, workspace_dir: &str, kinds: &[&str]) -> Result<Vec<KernelEvent>>;

    /// `after` 보다 뒤의 사건을 최대 `limit` 개 기록 순서대로 읽는다 (plan 19 Q3-3). `limit` 은
    /// `MAX_KERNEL_EVENT_PAGE` 를 넘지 않는다. 기본값은 빈 쪽이다 (조회를 쓰지 않는 시험용 기록).
    fn load_kernel_events_after(
        &self,
        _workspace_dir: &str,
        after: i64,
        _limit: usize,
    ) -> Result<KernelEventPage> {
        Ok(KernelEventPage {
            records: Vec::new(),
            next_after: after,
        })
    }
}
