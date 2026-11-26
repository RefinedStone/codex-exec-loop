//! Gateway가 관찰한 턴 상태를 애플리케이션에 전달하는 진입 계약이다.
//! JSON-RPC 해석은 adapter에, 공유 기록과 연결별 수명은 구현 서비스에 둔다.
use std::sync::Arc;

pub(crate) trait TurnActivityPort: Send + Sync {
    /// 같은 기록을 공유하되 독립된 연결 관찰자를 만든다.
    fn observe_connection(self: Arc<Self>) -> Box<dyn TurnActivityObserverPort>;
}

/// 하나의 연결이 본 상태만 바꾼다. 이 핸들을 버리면 그 연결의 기록만 정리한다.
pub(crate) trait TurnActivityObserverPort: Send + Sync {
    /// 폴더와 실행 상태를 한 번에 갱신한다. 상태가 없으면 이전 턴 알림을 보존한다.
    fn thread_snapshot(&self, thread_id: &str, folder: &str, running: Option<bool>);

    fn turn_running(&self, thread_id: &str, running: bool);

    /// 해당 연결에서 더는 구독하지 않는 스레드의 기록만 지운다.
    fn forget_thread(&self, thread_id: &str);
}
