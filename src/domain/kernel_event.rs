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

/// 턴을 낸 쪽이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOrigin {
    /// 사람이 낸 프롬프트이다.
    Manual,
    /// planning 준비를 거친 프롬프트이다 (TUI 의 intake, 원격 thread 지정 프롬프트).
    ManualIntake,
    /// 턴 뒤 평가가 낸 자동 후속 프롬프트이다.
    AutoFollow,
}

/// 턴 뒤 평가가 고른 다음 경로이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostTurnRoute {
    /// 자동 후속 턴을 낸다.
    AutoSubmit,
    /// 병렬 제어부가 자동 후속 프롬프트를 썼다.
    ParallelConsumed,
    /// 이어 가지 않는다.
    NoContinuation,
}

/// thread 지정 프롬프트를 준비 전에 거절한 이유이다. planning 작업은 만들어지지 않았다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadPromptRejection {
    Busy,
    PreparationActive,
    UnknownThread,
    ParallelMode,
    ParallelModeUnknown,
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
    /// thread 지정 프롬프트를 준비 전에 거절했다 (plan 19 Q3-2).
    ThreadPromptRejected {
        thread_id: String,
        reason: ThreadPromptRejection,
    },
    /// 턴 제출을 받아들였다. `submission` 은 그 제출의 세대 번호이다. 새 대화의 첫 턴은 thread id 가 비어 있다.
    TurnSubmitted {
        thread_id: String,
        submission: u64,
        origin: TurnOrigin,
    },
    /// app-server 가 턴을 시작했다.
    TurnStarted { thread_id: String, turn_id: String },
    /// 턴이 끝났다 (완료, 중단, 실패를 가르지 않는다).
    TurnEnded { thread_id: String, turn_id: String },
    /// 턴 뒤 평가의 경로를 정했다.
    PostTurnRouted {
        thread_id: String,
        completed_turn_id: String,
        route: PostTurnRoute,
    },
}

/// 남긴 프롬프트를 되돌릴 때 읽는 사건 종류이다. 다른 사건은 읽지 않는다.
pub const THREAD_PROMPT_QUEUE_KINDS: [&str; 2] = ["thread_prompt_queued", "thread_prompt_settled"];

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
            Self::ThreadPromptRejected { .. } => "thread_prompt_rejected",
            Self::TurnSubmitted { .. } => "turn_submitted",
            Self::TurnStarted { .. } => "turn_started",
            Self::TurnEnded { .. } => "turn_ended",
            Self::PostTurnRouted { .. } => "post_turn_routed",
        }
    }

    pub fn thread_id(&self) -> &str {
        match self {
            Self::ThreadPromptQueued(prompt) => &prompt.thread_id,
            Self::ThreadPromptSettled { thread_id, .. }
            | Self::ThreadPromptRejected { thread_id, .. }
            | Self::TurnSubmitted { thread_id, .. }
            | Self::TurnStarted { thread_id, .. }
            | Self::TurnEnded { thread_id, .. }
            | Self::PostTurnRouted { thread_id, .. } => thread_id,
        }
    }
}

/*
 * thread 요약 두 개를 비교해 턴의 시작과 끝을 낸다 (plan 19 Q3-2). 요약은 보지 않는 thread 를 포함하므로 모든 thread 의
 * 턴을 본다. 끝을 시작보다 먼저 낸다. 같은 thread 에서 턴이 바뀌면 이전 턴이 먼저 끝났기 때문이다. 값은
 * `(thread id, 진행 중 턴 id)` 이다.
 */
pub fn turn_events_between(
    before: &[(String, Option<String>)],
    after: &[(String, Option<String>)],
) -> Vec<KernelEvent> {
    let running = |turns: &[(String, Option<String>)]| {
        turns
            .iter()
            .filter_map(|(thread_id, turn_id)| Some((thread_id.clone(), turn_id.clone()?)))
            .collect::<Vec<_>>()
    };
    let (before, after) = (running(before), running(after));
    let ended = before
        .iter()
        .filter(|turn| !after.contains(turn))
        .map(|(thread_id, turn_id)| KernelEvent::TurnEnded {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
        });
    let started = after
        .iter()
        .filter(|turn| !before.contains(turn))
        .map(|(thread_id, turn_id)| KernelEvent::TurnStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
        });
    ended.chain(started).collect()
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
            _ => {}
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

    fn turn(thread: &str, turn: Option<&str>) -> (String, Option<String>) {
        (thread.to_string(), turn.map(str::to_string))
    }

    #[test]
    fn a_turn_change_ends_the_old_turn_before_it_starts_the_new_one() {
        let events = turn_events_between(
            &[turn("thread-1", Some("turn-1")), turn("thread-2", None)],
            &[
                turn("thread-1", Some("turn-2")),
                turn("thread-2", Some("turn-3")),
            ],
        );
        let ids = events
            .iter()
            .map(|event| match event {
                KernelEvent::TurnEnded { turn_id, .. } => format!("end {turn_id}"),
                KernelEvent::TurnStarted { turn_id, .. } => format!("start {turn_id}"),
                other => panic!("unexpected event {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, ["end turn-1", "start turn-2", "start turn-3"]);
        let same = [turn("thread-1", Some("turn-1"))];
        assert!(turn_events_between(&same, &same).is_empty());
    }

    #[test]
    fn every_kind_matches_its_json_tag() {
        let events = [
            KernelEvent::ThreadPromptRejected {
                thread_id: "thread-1".to_string(),
                reason: ThreadPromptRejection::Busy,
            },
            KernelEvent::TurnSubmitted {
                thread_id: String::new(),
                submission: 3,
                origin: TurnOrigin::AutoFollow,
            },
            KernelEvent::TurnStarted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
            KernelEvent::TurnEnded {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
            KernelEvent::PostTurnRouted {
                thread_id: "thread-1".to_string(),
                completed_turn_id: "turn-1".to_string(),
                route: PostTurnRoute::AutoSubmit,
            },
        ];
        for event in events {
            let json = serde_json::to_value(&event).expect("event should serialize");
            assert_eq!(json["kind"], event.kind());
            let decoded: KernelEvent = serde_json::from_value(json).expect("event should decode");
            assert_eq!(decoded, event);
        }
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
