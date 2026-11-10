use anyhow::Result;

use crate::domain::kernel_event::KernelEvent;

/*
 * 커널의 판단 사건을 저장소별 추가 전용 기록에 쓰고 읽는다 (plan 19 Q3-1). 작업 공간으로 그 저장소의 기록을 찾는다.
 * 읽을 때는 기록 순서를 지킨다. 이 binary 가 모르는 사건(더 새 binary 가 쓴 것)은 건너뛴다.
 */
pub trait KernelEventLogPort: Send + Sync {
    fn append_kernel_events(&self, workspace_dir: &str, events: &[KernelEvent]) -> Result<()>;

    fn load_kernel_events(&self, workspace_dir: &str) -> Result<Vec<KernelEvent>>;
}
