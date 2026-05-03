// Queue head 반복 판단은 refresh 전/후의 planning runtime snapshot을 비교한다.
// Snapshot은 queue head projection과 task authority signature를 함께 들고 있어,
// "겉보기 head row"와 "실제 task 본문"을 따로 검증할 수 있다.
use crate::application::service::planning::PlanningRuntimeSnapshot;
// Handoff는 직전 auto-follow가 agent에게 넘긴 task identity다. 다음 refresh가 같은
// task를 다시 head로 남기면 queue가 전진하지 않은 것이므로 follow-up pause 근거가 된다.
use crate::application::service::planning::PlanningTaskHandoff;

// `repeated_queue_head_detail`은 builtin next-task refresh 뒤에 호출되는 guard다.
// Refresh가 성공 snapshot을 만들었더라도 이전 handoff와 같은 queue head가 그대로면,
// caller가 이를 host detail과 auto-follow pause reason으로 낮출 수 있게 설명을 반환한다.
pub(super) fn repeated_queue_head_detail(
    // 비교 기준이 되는 직전 handoff다. 수동 turn이나 첫 auto-follow처럼 넘긴 task가 없으면 no-op다.
    previous_handoff: Option<&PlanningTaskHandoff>,
    // 직전 conversation state의 planning runtime이다. Head task signature 비교의 기준점이 된다.
    previous_snapshot: &PlanningRuntimeSnapshot,
    // 방금 refresh한 runtime이다. 이 head가 이전 handoff와 같고 signature도 같으면 반복 위험이다.
    snapshot: &PlanningRuntimeSnapshot,
) -> Option<String> {
    // 이 helper는 "이전에 넘긴 task와 비교"하는 정책이라 queue head 존재만으로 block하지 않는다.
    let previous_handoff = previous_handoff?;
    // 새 snapshot에 queue head가 없으면 반복된 head도 없다. Queue idle 처리는 caller branch가 담당한다.
    let queue_head = snapshot.queue_head()?;
    // task_id가 다르면 queue가 다음 작업으로 전진한 것이다. 나머지 표시값이 같아도 반복이 아니다.
    if queue_head.task_id.trim() != previous_handoff.task_id.trim() {
        return None;
    }

    // 같은 task_id라도 title, direction, priority, updated_at, status가 바뀌면 agent가
    // 다시 볼 가치가 있는 authority change일 수 있다. 같은 id만으로 반복 실패로 보지 않는다.
    let unchanged = queue_head.task_title.trim() == previous_handoff.task_title.trim()
        && queue_head.direction_id.trim() == previous_handoff.direction_id.trim()
        && queue_head.combined_priority == previous_handoff.combined_priority
        && queue_head.updated_at.trim() == previous_handoff.updated_at.trim()
        && queue_head.status.label() == previous_handoff.status_label;
    // Handoff metadata가 하나라도 달라졌다면 "완전히 같은 head"가 아니므로 보수적으로 빠진다.
    if !unchanged {
        return None;
    }

    // Metadata가 그대로여도 ledger의 다른 부분이 바뀌었을 수 있다. Queue head task
    // signature를 따로 비교해 head task 본문/명령 변경만 전진으로 인정하고 unrelated edit은 제외한다.
    let queue_head_task_unchanged = match (
        previous_snapshot.queue_head_task_signature(),
        snapshot.queue_head_task_signature(),
    ) {
        // 양쪽 signature가 있으면 서명 값이 같을 때만 동일 task로 본다.
        (Some(previous), Some(current)) => previous == current,
        // 둘 다 None이면 정보 수준이 대칭적이다. 추가 증거가 없으므로 metadata 비교 결과를 신뢰한다.
        (None, None) => true,
        // 한쪽에만 signature가 있으면 비교 기준이 달라진 것이다. Advancement 가능성으로 보고 block하지 않는다.
        _ => false,
    };
    // Head task signature가 달라졌다면 같은 task id라도 내용이 바뀐 것이므로 agent에게 다시 넘길 수 있다.
    if !queue_head_task_unchanged {
        return None;
    }

    // 여기까지 왔다는 것은 previous handoff와 refresh 후 queue head가 같은 task이고,
    // metadata와 task signature도 바뀌지 않았다는 뜻이다.
    Some(format!(
        "planner refresh kept the previously handed-off task unchanged as the queue head; unrelated ledger edits do not count as queue advancement: {}",
        previous_handoff.task_title
    ))
}

// Tests pin signature-presence boundaries. The production guard combines task metadata
// and signature comparison, so None/Some transitions must stay intentionally conservative.
#[cfg(test)]
mod tests {
    // Test snapshots use production `PlanningRuntimeSnapshot::ready` so accessors keep their real contract.
    use crate::application::service::planning::PlanningRuntimeSnapshot;
    // Handoff fixture reproduces the task identity from the previous auto-follow prompt.
    use crate::application::service::planning::PlanningTaskHandoff;
    // Queue head fixture uses the domain priority queue row shape.
    use crate::domain::planning::{PriorityQueueTask, TaskStatus};

    use super::repeated_queue_head_detail;

    // Sample queue head matches the handoff across every metadata field so tests isolate signature policy.
    fn sample_queue_head() -> PriorityQueueTask {
        PriorityQueueTask {
            // Rank fields are display-only for this helper but required by the queue row.
            rank: 1,
            // Task identity must match the handoff before the helper reaches signature comparison.
            task_id: "task-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_title: "Direction".to_string(),
            task_title: "Queue head".to_string(),
            // Handoff stores status as prompt/DTO copy, so comparison goes through the public label.
            status: TaskStatus::Ready,
            combined_priority: 80,
            updated_at: "2026-04-23T00:00:00Z".to_string(),
            rank_reasons: vec!["ready".to_string()],
        }
    }

    // Handoff fixture points at the same task; changing it would bypass the signature layer entirely.
    fn sample_handoff() -> PlanningTaskHandoff {
        PlanningTaskHandoff {
            task_id: "task-1".to_string(),
            task_title: "Queue head".to_string(),
            direction_id: "direction-1".to_string(),
            combined_priority: 80,
            updated_at: "2026-04-23T00:00:00Z".to_string(),
            status_label: "ready".to_string(),
        }
    }

    // Snapshot fixture keeps the queue head identical and injects only the test signature.
    fn snapshot_with_signature(signature: Option<u64>) -> PlanningRuntimeSnapshot {
        PlanningRuntimeSnapshot::ready(
            "prompt".to_string(),
            "summary".to_string(),
            Some(sample_queue_head()),
        )
        // First signature is proposal-level; this helper only needs the queue head task signature.
        .with_test_signatures(None, signature)
    }

    // Missing -> present signature changes evidence quality, so the guard must not call it unchanged.
    #[test]
    fn repeated_queue_head_detail_treats_missing_and_present_signatures_as_changed() {
        let detail = repeated_queue_head_detail(
            Some(&sample_handoff()),
            &snapshot_with_signature(None),
            &snapshot_with_signature(Some(7)),
        );

        assert!(detail.is_none());
    }

    // Both missing signatures are symmetric; metadata equality is the only available evidence.
    #[test]
    fn repeated_queue_head_detail_accepts_both_missing_signatures_as_unchanged() {
        let detail = repeated_queue_head_detail(
            Some(&sample_handoff()),
            &snapshot_with_signature(None),
            &snapshot_with_signature(None),
        );

        assert!(detail.is_some());
    }
}
