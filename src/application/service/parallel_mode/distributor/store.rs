/*
 * distributor store는 parallel mode delivery queue의 persistence adapter다. planning
 * authority projection이 application이 읽는 source of truth이고, pool root 아래 JSON mirror는 운영자가
 * queue 상태를 조사하거나 테스트가 recovery/order 보존을 확인할 때 쓰는 durable trace다.
 */
use std::fs;
use std::path::{Path, PathBuf};

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{ParallelModeQueueItemState, ParallelModeSlotLeaseSnapshot};

use super::super::{
    current_timestamp, ensure_directory_exists, record_distributor_failed_session_detail,
};
use super::ParallelModeDistributorQueueRecord;
use super::queue_keys::sanitize_runtime_record_key;

/*
distributor queue root는 pool root 아래의 durable queue mirror이다. 실제 source of
truth는 planning authority의 runtime queue projection이지만, `.distributor-queue/<id>.json` 파일은
운영자가 queue item을 확인하고 테스트가 store-backed recovery를 검증하는 데 쓰인다.
*/
fn distributor_queue_root(pool_root: &Path) -> PathBuf {
    pool_root.join(".distributor-queue")
}

// queue record path 계산을 한 곳에 두어 writer와 test loader가 같은 mirror layout을 공유한다.
fn distributor_queue_record_path(pool_root: &Path, queue_item_id: &str) -> PathBuf {
    distributor_queue_root(pool_root).join(format!("{queue_item_id}.json"))
}

/*
queue item id는 slot, agent, enqueue timestamp를 합친 runtime record key이다.
timestamp까지 포함해 같은 agent/slot이 여러 번 결과를 내더라도 queue record 파일명이 겹치지
않는다. filesystem path로도 쓰이므로 마지막에 sanitize를 거친다.
*/
pub(super) fn distributor_queue_item_id(
    lease: &ParallelModeSlotLeaseSnapshot,
    timestamp: &str,
) -> String {
    sanitize_runtime_record_key(&format!(
        "{}-{}-{}",
        lease.slot_id, lease.agent_id, timestamp
    ))
}

/*
queue order key는 RFC3339 timestamp에서 숫자만 뽑아 만든 정렬 키이다. 문자열
timestamp 정렬도 대체로 가능하지만, u64 key를 함께 저장하면 persistence layer나 UI projection이
명시적인 queue ordering 값을 사용할 수 있다.
*/
pub(super) fn queue_order_key_from_timestamp(timestamp: &str) -> u64 {
    // timezone separator와 punctuation을 제거한 숫자 prefix만 사용해 JSON/DB projection 모두에서 정렬한다.
    timestamp
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .take(20)
        .collect::<String>()
        .parse::<u64>()
        .unwrap_or(0)
}

#[cfg(test)]
/*
테스트용 loader는 파일 미러에서 queue record를 다시 읽어 restart/recovery 시나리오를
검증한다. production flow는 planning authority projection을 우선하지만, 파일 mirror가 깨지지
않고 order를 보존하는지 확인하려면 이 경로가 필요하다.
*/
pub(crate) fn load_distributor_queue_records(
    pool_root: &Path,
) -> Vec<ParallelModeDistributorQueueRecord> {
    let queue_root = distributor_queue_root(pool_root);
    let Ok(entries) = fs::read_dir(queue_root) else {
        // mirror directory가 없다는 것은 아직 distributor queue가 생성되지 않았다는 정상 상태다.
        return Vec::new();
    };

    /*
     * test loader는 corrupt entry를 hard fail하지 않고 건너뛴다. production source는
     * planning authority projection이므로, mirror 검증은 읽을 수 있는 record의 order와 content를 보는 데 집중한다.
     */
    let mut records = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|content| {
            serde_json::from_str::<ParallelModeDistributorQueueRecord>(&content).ok()
        })
        .collect::<Vec<_>>();
    // enqueue timestamp를 primary key로, queue item id를 tie breaker로 써 restart 후 display order를 안정화한다.
    records.sort_by(|left, right| {
        left.enqueued_at
            .cmp(&right.enqueued_at)
            .then_with(|| left.queue_item_id.cmp(&right.queue_item_id))
    });
    records
}

/*
queue record 저장은 planning authority와 filesystem mirror를 모두 갱신한다.
먼저 authority projection을 upsert해 application이 읽는 실시간 상태를 갱신하고, 그 다음 JSON
파일을 temp file + rename으로 쓴다. 이 순서와 atomic-ish rename은 프로세스 중단 시 부분
JSON이 최종 파일명으로 남는 위험을 줄인다.
*/
pub(super) fn write_distributor_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    record: &ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    // 먼저 authority projection을 갱신해야 supervisor snapshot과 distributor processor가 최신 queue를 본다.
    planning_authority
        .upsert_runtime_distributor_queue_record(workspace_dir, record)
        .map_err(|error| {
            format!(
                "failed to store distributor queue record `{}`: {error}",
                record.queue_item_id
            )
        })?;

    let queue_root = distributor_queue_root(pool_root);
    ensure_directory_exists(&queue_root)
        .map_err(|error| format!("failed to create distributor queue directory: {error}"))?;

    let path = distributor_queue_record_path(pool_root, &record.queue_item_id);
    let temp_path = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(record)
        .map_err(|error| format!("failed to serialize distributor queue record: {error}"))?;
    // temp file write가 성공한 뒤 rename해야 partially written JSON이 canonical path에 남지 않는다.
    fs::write(&temp_path, body).map_err(|error| {
        format!(
            "failed to write temporary distributor queue record `{}`: {error}",
            record.queue_item_id
        )
    })?;
    fs::rename(&temp_path, &path).map_err(|error| {
        format!(
            "failed to persist distributor queue record `{}`: {error}",
            record.queue_item_id
        )
    })
}

/*
block_distributor_queue_record는 delivery의 공통 실패 전이이다. queue state와
integration state를 blocked로 바꾸고, 최초 recovery_note를 보존하며, 최신 integration_note에는
현재 실패 원인을 기록한다. lease가 있으면 session detail도 failed로 갱신해 supervisor detail과
completion feed가 queue block을 agent session 관점에서도 보여 준다.

반환 문자열은 caller가 TUI notice로 바로 표시할 수 있는 한 줄 요약이다.
*/
pub(super) fn block_distributor_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
    failure_detail: String,
) -> Result<String, String> {
    record.queue_state = ParallelModeQueueItemState::Blocked;
    record.integration_state = "blocked".to_string();
    // 최초 recovery note는 root cause를 보존하고, integration note는 가장 최근 실패를 계속 갱신한다.
    if record.recovery_note.is_none() {
        record.recovery_note = Some(failure_detail.clone());
    }
    record.integration_note = failure_detail.clone();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(planning_authority, workspace_dir, pool_root, record)?;
    // lease가 있으면 queue item뿐 아니라 session detail history도 failed 상태로 투영한다.
    if let Some(lease) = lease {
        let _ = record_distributor_failed_session_detail(
            planning_authority,
            workspace_dir,
            pool_root,
            lease,
            &failure_detail,
        );
    }

    Ok(format!(
        "distributor queue head blocked / agent: {} / {}",
        record.agent_id, failure_detail
    ))
}
