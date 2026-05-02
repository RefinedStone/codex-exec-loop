// 학습 주석: queue head 반복 판단은 refresh 전/후의 planning runtime snapshot을 비교합니다.
// snapshot은 queue head view와 task authority signature를 함께 들고 있어 "겉보기 head"와 "실제 task 본문"을 모두 검사할 수 있습니다.
use crate::application::service::planning::PlanningRuntimeSnapshot;
// 학습 주석: handoff는 직전 auto-follow가 agent에게 넘긴 task identity입니다. 다음 refresh가 같은
// task를 다시 head로 남기면 queue가 전진하지 않은 것이므로 follow-up을 멈추는 근거가 됩니다.
use crate::application::service::planning::PlanningTaskHandoff;

// 학습 주석: `repeated_queue_head_detail`은 builtin next-task refresh 뒤에 호출되는 guard입니다. refresh가
// snapshot을 만들었더라도 이전 handoff와 같은 queue head가 그대로면 host detail을 반환해 caller가
// `RefreshFailed`와 auto-follow pause reason으로 낮출 수 있게 합니다.
pub(super) fn repeated_queue_head_detail(
    // 학습 주석: previous_handoff가 없으면 비교 기준이 없습니다. 수동 turn이나 첫 auto-follow처럼 아직
    // 넘긴 task가 없는 상황에서는 queue 반복으로 판단하지 않습니다.
    previous_handoff: Option<&PlanningTaskHandoff>,
    // 학습 주석: previous_snapshot은 직전 conversation state의 planning runtime입니다. queue head task
    // signature를 비교해 unrelated ledger edit이 아니라 head task 자체가 바뀌었는지 확인합니다.
    previous_snapshot: &PlanningRuntimeSnapshot,
    // 학습 주석: snapshot은 방금 refresh한 runtime입니다. 이 snapshot의 queue head가 이전 handoff와
    // 같고 task signature도 그대로면 queue-driven auto-follow가 같은 일을 반복할 위험이 있습니다.
    snapshot: &PlanningRuntimeSnapshot,
) -> Option<String> {
    // 학습 주석: handoff가 없으면 None을 전파합니다. 이 helper는 "이전에 넘긴 task와 비교"하는 정책이라
    // queue head 존재만으로 block reason을 만들지 않습니다.
    let previous_handoff = previous_handoff?;
    // 학습 주석: 새 snapshot에 queue head가 없으면 반복된 head도 없습니다. queue idle 처리는 caller의
    // 다른 branch가 담당하므로 여기서는 조용히 None을 반환합니다.
    let queue_head = snapshot.queue_head()?;
    // 학습 주석: task_id가 다르면 queue가 다음 작업으로 전진한 것입니다. 나머지 title/status가 우연히
    // 같아도 같은 handoff 반복이 아니므로 pause detail을 만들지 않습니다.
    if queue_head.task_id.trim() != previous_handoff.task_id.trim() {
        return None;
    }

    // 학습 주석: 같은 task_id라도 title, direction, priority, updated_at, status가 바뀌면 task authority가
    // 의미 있게 갱신됐을 수 있습니다. 이 경우는 queue가 같은 id를 유지하더라도 반복 실패로 보지 않습니다.
    let unchanged = queue_head.task_title.trim() == previous_handoff.task_title.trim()
        && queue_head.direction_id.trim() == previous_handoff.direction_id.trim()
        && queue_head.combined_priority == previous_handoff.combined_priority
        && queue_head.updated_at.trim() == previous_handoff.updated_at.trim()
        && queue_head.status.label() == previous_handoff.status_label;
    // 학습 주석: handoff metadata가 하나라도 달라졌다면 agent가 다시 볼 가치가 있는 변경입니다. 이 helper는
    // "같은 queue head가 완전히 그대로 남은 경우"만 잡도록 보수적으로 빠집니다.
    if !unchanged {
        return None;
    }

    // 학습 주석: metadata가 그대로여도 ledger의 다른 부분이 바뀌었을 수 있습니다. queue head task
    // signature를 따로 비교해, head task 본문/명령이 바뀌었는지 확인하고 unrelated edit은 전진으로 세지 않습니다.
    let queue_head_task_unchanged = match (
        previous_snapshot.queue_head_task_signature(),
        snapshot.queue_head_task_signature(),
    ) {
        // 학습 주석: 양쪽 signature가 있으면 hash/서명 값이 같을 때만 동일 task로 봅니다.
        (Some(previous), Some(current)) => previous == current,
        // 학습 주석: 둘 다 None인 오래된/테스트 snapshot은 추가 증거가 없으므로 metadata 비교 결과를
        // 그대로 신뢰합니다. 이 정책은 기존 snapshot과 새 snapshot을 대칭적으로 다룹니다.
        (None, None) => true,
        // 학습 주석: 한쪽에만 signature가 있으면 비교 기준이 달라진 것입니다. 새로 계산된 signature를
        // queue advancement 가능성으로 보고 반복 block을 만들지 않습니다.
        _ => false,
    };
    // 학습 주석: head task signature가 달라졌다면 같은 task id라도 내용이 바뀐 것입니다. auto-follow는
    // 그 변경을 agent에게 다시 넘길 수 있으므로 반복 detail을 반환하지 않습니다.
    if !queue_head_task_unchanged {
        return None;
    }

    // 학습 주석: 여기까지 왔다는 것은 previous handoff와 refresh 후 queue head가 같은 task이고,
    // metadata와 task signature도 바뀌지 않았다는 뜻입니다. caller는 이 문구를 host detail과 pause reason으로 사용합니다.
    Some(format!(
        "planner refresh kept the previously handed-off task unchanged as the queue head; unrelated ledger edits do not count as queue advancement: {}",
        previous_handoff.task_title
    ))
}

// 학습 주석: 테스트는 signature 존재 여부의 경계값을 고정합니다. 실제 queue advancement guard는
// task metadata 비교와 signature 비교를 함께 쓰므로, signature None/Some 전환을 어떻게 해석하는지가 중요합니다.
#[cfg(test)]
mod tests {
    // 학습 주석: test snapshot은 production `PlanningRuntimeSnapshot::ready`를 사용해 queue_head accessor와
    // signature accessor가 실제 타입 계약을 그대로 통과하게 합니다.
    use crate::application::service::planning::PlanningRuntimeSnapshot;
    // 학습 주석: handoff fixture는 직전 auto-follow가 넘긴 task identity를 재현합니다.
    use crate::application::service::planning::PlanningTaskHandoff;
    // 학습 주석: queue head fixture는 domain priority queue projection의 task row를 그대로 만듭니다.
    use crate::domain::planning::{PriorityQueueTask, TaskStatus};

    use super::repeated_queue_head_detail;

    // 학습 주석: sample queue head는 sample handoff와 모든 비교 필드가 일치하도록 맞춘 기준 task입니다.
    // 테스트는 signature만 바꿔 helper의 마지막 comparison layer를 겨냥합니다.
    fn sample_queue_head() -> PriorityQueueTask {
        PriorityQueueTask {
            // 학습 주석: rank와 rank_reasons는 이 helper의 비교 대상은 아니지만 ready snapshot의 queue row를
            // 구성하는 필수 표시 정보라 fixture에 채웁니다.
            rank: 1,
            // 학습 주석: task identity와 handoff identity가 같아야 signature 비교 단계까지 도달합니다.
            task_id: "task-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_title: "Direction".to_string(),
            task_title: "Queue head".to_string(),
            // 학습 주석: status label은 handoff에 저장된 문자열과 비교됩니다. enum 자체가 아니라 label을
            // 비교하는 이유는 handoff가 prompt/DTO 경계를 지난 표시값이기 때문입니다.
            status: TaskStatus::Ready,
            combined_priority: 80,
            updated_at: "2026-04-23T00:00:00Z".to_string(),
            rank_reasons: vec!["ready".to_string()],
        }
    }

    // 학습 주석: handoff fixture는 sample queue head와 같은 task를 가리킵니다. 이 값이 달라지면 helper가
    // signature 비교까지 가지 않고 None을 반환하므로, signature 정책 테스트의 전제를 명확히 둡니다.
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

    // 학습 주석: snapshot fixture는 queue head를 동일하게 두고 test-only signature만 주입합니다. 이렇게
    // 하면 metadata 비교 결과가 항상 unchanged라 signature 비교 branch를 독립적으로 검증할 수 있습니다.
    fn snapshot_with_signature(signature: Option<u64>) -> PlanningRuntimeSnapshot {
        PlanningRuntimeSnapshot::ready(
            "prompt".to_string(),
            "summary".to_string(),
            Some(sample_queue_head()),
        )
        // 학습 주석: 첫 signature 인자는 proposal 쪽이고, 두 번째가 queue head task signature입니다.
        // 이 helper는 queue head 반복만 보므로 proposal signature는 None으로 고정합니다.
        .with_test_signatures(None, signature)
    }

    // 학습 주석: 이전 snapshot에는 signature가 없고 새 snapshot에는 signature가 있으면, 비교 기준이
    // 달라진 상태입니다. helper는 이를 "unchanged"로 단정하지 않고 None을 반환해야 합니다.
    #[test]
    fn repeated_queue_head_detail_treats_missing_and_present_signatures_as_changed() {
        let detail = repeated_queue_head_detail(
            Some(&sample_handoff()),
            &snapshot_with_signature(None),
            &snapshot_with_signature(Some(7)),
        );

        assert!(detail.is_none());
    }

    // 학습 주석: 양쪽 signature가 모두 없으면 과거 snapshot과 새 snapshot이 같은 정보 수준입니다. 이 경우
    // metadata가 모두 같다는 앞선 비교만으로 반복 queue head detail을 반환합니다.
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
