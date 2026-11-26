/*
 * Gateway 가 넘기는 턴의 시작과 끝을 스레드 폴더와 함께 기억한다 (#2850, plan 27 §9.3). git 작업 엔진은 실행 직전에
 * 이 기록을 읽어, 턴이 실행 중인 작업 트리를 바꾸는 작업을 거절한다.
 *
 * 기록은 접속(관찰자)마다 따로 둔다. 접속이 끝나면 그 접속이 본 기록을 지운다: 그 뒤에는 턴의 끝을 볼 수 없으므로,
 * 남겨 두면 끝난 턴이 작업을 영영 막는다. 폴더를 모르는 스레드의 턴은 막지 않는다.
 */
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::application::port::inbound::turn_activity_port::{
    TurnActivityObserverPort, TurnActivityPort,
};

#[derive(Default)]
struct ThreadActivity {
    folder: Option<String>,
    running: bool,
}

#[derive(Default)]
struct Inner {
    next_observer: u64,
    threads: HashMap<(u64, String), ThreadActivity>,
}

#[derive(Default)]
pub struct TurnActivityRegistry {
    inner: Mutex<Inner>,
}

impl TurnActivityRegistry {
    /// 접속 하나의 관찰자를 만든다. 관찰자를 버리면 그 기록도 지운다.
    pub fn observer(self: &Arc<Self>) -> TurnActivityObserver {
        let mut inner = self.lock();
        inner.next_observer += 1;
        TurnActivityObserver {
            registry: self.clone(),
            id: inner.next_observer,
        }
    }

    /// 턴이 실행 중인 스레드의 폴더들이다. 같은 폴더는 한 번만 있다.
    pub fn running_folders(&self) -> Vec<String> {
        let inner = self.lock();
        let mut folders: Vec<String> = inner
            .threads
            .values()
            .filter(|thread| thread.running)
            .filter_map(|thread| thread.folder.clone())
            .collect();
        folders.sort();
        folders.dedup();
        folders
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(&self, observer: u64, thread_id: &str, change: impl FnOnce(&mut ThreadActivity)) {
        let mut inner = self.lock();
        change(
            inner
                .threads
                .entry((observer, thread_id.to_string()))
                .or_default(),
        );
    }
}

/// 접속 하나가 본 스레드와 턴이다.
pub struct TurnActivityObserver {
    registry: Arc<TurnActivityRegistry>,
    id: u64,
}

impl TurnActivityObserver {
    pub fn thread_folder(&self, thread_id: &str, folder: &str) {
        self.thread_snapshot(thread_id, folder, None);
    }

    /// 구독 스냅샷의 폴더와 상태는 함께 보인다. 상태가 없으면 이미 본 턴 알림을 보존한다.
    pub fn thread_snapshot(&self, thread_id: &str, folder: &str, running: Option<bool>) {
        self.registry.update(self.id, thread_id, |thread| {
            thread.folder = Some(folder.to_string());
            if let Some(running) = running {
                thread.running = running;
            }
        });
    }

    /// 이 접속이 더는 관찰하지 않는 스레드만 지운다. 다른 접속의 관찰 기록은 그대로이다.
    pub fn forget_thread(&self, thread_id: &str) {
        self.registry
            .lock()
            .threads
            .remove(&(self.id, thread_id.to_string()));
    }

    pub fn turn_running(&self, thread_id: &str, running: bool) {
        self.registry.update(self.id, thread_id, |thread| {
            thread.running = running;
        });
    }
}

impl Drop for TurnActivityObserver {
    fn drop(&mut self) {
        self.registry
            .lock()
            .threads
            .retain(|(observer, _), _| *observer != self.id);
    }
}

impl TurnActivityPort for TurnActivityRegistry {
    fn observe_connection(self: Arc<Self>) -> Box<dyn TurnActivityObserverPort> {
        Box::new(Self::observer(&self))
    }
}

impl TurnActivityObserverPort for TurnActivityObserver {
    fn thread_snapshot(&self, thread_id: &str, folder: &str, running: Option<bool>) {
        Self::thread_snapshot(self, thread_id, folder, running);
    }

    fn turn_running(&self, thread_id: &str, running: bool) {
        Self::turn_running(self, thread_id, running);
    }

    fn forget_thread(&self, thread_id: &str) {
        Self::forget_thread(self, thread_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_folders_follow_turns_and_forget_a_closed_connection() {
        let registry = Arc::new(TurnActivityRegistry::default());
        let first = registry.observer();
        let second = registry.observer();
        first.thread_folder("a", "/repo/src");
        first.turn_running("a", true);
        // The folder may arrive after the turn starts.
        second.turn_running("b", true);
        second.thread_folder("b", "/other");
        first.turn_running("unknown-folder", true);
        assert_eq!(registry.running_folders(), ["/other", "/repo/src"]);

        first.turn_running("a", false);
        assert_eq!(registry.running_folders(), ["/other"]);

        first.turn_running("a", true);
        drop(first);
        assert_eq!(registry.running_folders(), ["/other"]);
        drop(second);
        assert!(registry.running_folders().is_empty());
    }
}
