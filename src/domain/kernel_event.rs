//! 커널의 판단 사건이다 (plan 19 Q3-1). 추가 전용 기록에 쓰고, 커널을 다시 조립할 때 판단 상태를 되돌린다.
//! 대화 원본, 화면 상태, 스트림 delta 는 담지 않는다 (plan 19 §5.1).

use serde::{Deserialize, Serialize};

use crate::domain::planning::ManualPromptMainSessionHandoff;

/*
 * 준비가 끝났지만 제출이 거절된 thread 지정 프롬프트이다 (plan 19 Q2-3). 준비가 만든 작업은 이미 큐에 있다. 커널이
 * 쉬면 같은 handoff 로 한 번 다시 낸다. 기록에서 되돌릴 수 있게 직렬화한다 (Q3-1).
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedThreadPrompt {
    pub thread_id: String,
    pub workspace_directory: String,
    pub handoff: ManualPromptMainSessionHandoff,
    pub task_id: String,
}

/// 남긴 프롬프트가 어떻게 끝났는가이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadPromptSettlement {
    /// 다시 내서 턴을 제출했다.
    Submitted,
    /// 병렬 모드가 켜져 내지 않았다. 작업은 병렬 slot 이 맡는다.
    ParallelSlots,
    /// planning 투영이 작업을 완료나 취소로 보여 내지 않았다.
    TaskFinished,
    /// 다시 냈지만 또 거절되었다. 작업은 큐에 남는다.
    RejectedAgain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KernelEvent {
    /// 커널이 프롬프트를 남겼다. 쉴 때 한 번 다시 낸다.
    ThreadPromptQueued(QueuedThreadPrompt),
    /// 남긴 프롬프트가 끝났다. 작업 id 로 남김과 짝을 맞춘다.
    ThreadPromptSettled {
        thread_id: String,
        workspace_directory: String,
        task_id: String,
        settlement: ThreadPromptSettlement,
    },
}

impl QueuedThreadPrompt {
    /// 이 남김이 끝났다는 사건이다.
    pub fn settled(&self, settlement: ThreadPromptSettlement) -> KernelEvent {
        KernelEvent::ThreadPromptSettled {
            thread_id: self.thread_id.clone(),
            workspace_directory: self.workspace_directory.clone(),
            task_id: self.task_id.clone(),
            settlement,
        }
    }
}

impl KernelEvent {
    /// 기록 표의 `kind` 열 값이다. 직렬화의 `kind` 와 같다.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ThreadPromptQueued(_) => "thread_prompt_queued",
            Self::ThreadPromptSettled { .. } => "thread_prompt_settled",
        }
    }

    pub fn thread_id(&self) -> &str {
        match self {
            Self::ThreadPromptQueued(prompt) => &prompt.thread_id,
            Self::ThreadPromptSettled { thread_id, .. } => thread_id,
        }
    }

    /// 이 사건을 기록할 저장소를 찾는 작업 공간이다.
    pub fn workspace_directory(&self) -> &str {
        match self {
            Self::ThreadPromptQueued(prompt) => &prompt.workspace_directory,
            Self::ThreadPromptSettled {
                workspace_directory,
                ..
            } => workspace_directory,
        }
    }
}

/// 기록 순서대로 읽은 사건에서 아직 끝나지 않은 남김을 남긴 차례대로 돌려준다.
pub fn open_queued_thread_prompts(
    events: impl IntoIterator<Item = KernelEvent>,
) -> Vec<QueuedThreadPrompt> {
    let mut open: Vec<QueuedThreadPrompt> = Vec::new();
    for event in events {
        match event {
            KernelEvent::ThreadPromptQueued(prompt) => {
                open.retain(|queued| queued.task_id != prompt.task_id);
                open.push(prompt);
            }
            KernelEvent::ThreadPromptSettled { task_id, .. } => {
                open.retain(|queued| queued.task_id != task_id);
            }
        }
    }
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(task_id: &str) -> QueuedThreadPrompt {
        QueuedThreadPrompt {
            thread_id: "thread-1".to_string(),
            workspace_directory: "/workspace".to_string(),
            handoff: ManualPromptMainSessionHandoff {
                prompt: "planned prompt".to_string(),
                transcript_text: "fix the test".to_string(),
                task: None,
            },
            task_id: task_id.to_string(),
        }
    }

    fn settled(task_id: &str) -> KernelEvent {
        queued(task_id).settled(ThreadPromptSettlement::Submitted)
    }

    #[test]
    fn only_unsettled_queued_prompts_stay_open_in_queue_order() {
        let open = open_queued_thread_prompts([
            KernelEvent::ThreadPromptQueued(queued("task-1")),
            KernelEvent::ThreadPromptQueued(queued("task-2")),
            settled("task-1"),
            KernelEvent::ThreadPromptQueued(queued("task-3")),
            settled("task-9"),
        ]);
        assert_eq!(open, vec![queued("task-2"), queued("task-3")]);
    }

    #[test]
    fn an_event_round_trips_through_json_with_its_kind() {
        let event = KernelEvent::ThreadPromptQueued(queued("task-1"));
        let json = serde_json::to_value(&event).expect("event should serialize");
        assert_eq!(json["kind"], event.kind());
        let decoded: KernelEvent = serde_json::from_value(json).expect("event should decode");
        assert_eq!(decoded, event);
        assert_eq!(
            serde_json::to_value(settled("task-1")).expect("event should serialize")["kind"],
            settled("task-1").kind()
        );
    }
}
