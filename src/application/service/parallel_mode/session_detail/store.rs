/*
 * session_detail/store.rs는 parallel agent session detail의 persistence adapter다.
 * planning authority runtime store를 source of truth로 갱신하고, pool root의 `.agent-sessions`
 * JSON mirror를 recovery/debug용 durable trace로 유지한다.
 */
use std::fs;
use std::path::{Path, PathBuf};

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeSlotLeaseSnapshot,
};

use super::super::ensure_directory_exists;
use super::lease_session_key;

pub(super) fn push_session_history(
    detail: &mut ParallelModeAgentSessionDetailSnapshot,
    state_label: &str,
    timestamp: String,
    summary: String,
) {
    /*
    session detail의 history는 supervisor detail, distributor completion feed,
    recovery 화면이 공통으로 읽는 시간순 사건 목록이다. 같은 상태와 같은 summary가 연속으로
    반복되면 UI에는 의미 없는 중복 줄만 늘어나므로, 마지막 entry와 완전히 같은 전이는 기록하지
    않는다. timestamp가 달라도 운영자가 보는 의미가 같으면 하나의 사건으로 취급하는 설계이다.
    */
    if detail
        .history
        .last()
        .is_some_and(|entry| entry.state_label == state_label && entry.summary == summary)
    {
        return;
    }

    detail
        .history
        .push(ParallelModeAgentSessionHistoryEntry::new(
            state_label,
            timestamp,
            summary,
        ));
}

/*
이 helper는 session detail의 read-modify-write 패턴을 한곳에 모은다. 상위
`session_detail.rs` 함수들은 assigned, running, commit_ready, integrating 같은 구체 상태만
정하고, 기존 detail을 읽고 mutate closure를 적용하고 authority store와 파일 mirror에 쓰는
반복 규칙은 여기로 위임한다.

closure가 `Option<ParallelModeAgentSessionDetailSnapshot>`을 받는 이유는 새 session의 최초
기록과 기존 session의 상태 갱신을 같은 함수로 처리하기 위해서이다. caller가 "없으면 새로
만들기"와 "있으면 이어 쓰기"를 각 상태별 의미에 맞게 결정한다.
*/
pub(super) fn update_agent_session_detail_record<F>(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    mutate: F,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String>
where
    F: FnOnce(
        Option<ParallelModeAgentSessionDetailSnapshot>,
    ) -> ParallelModeAgentSessionDetailSnapshot,
{
    /*
    session_key는 lease의 slot, agent, task, branch 정체성을 묶는 안정 키이다.
    workspace path는 cleanup이나 worktree 재생성 과정에서 달라질 수 있으므로, detail record를
    찾을 때는 lease에서 계산한 session_key를 사용한다. 이렇게 해야 recovery가 store-backed
    queue record와 session detail을 같은 logical session으로 다시 연결할 수 있다.
    */
    let session_key = lease_session_key(lease);
    let current = read_agent_session_detail_record(pool_root, &session_key);
    let detail = mutate(current);
    write_agent_session_detail_record(planning_authority, workspace_dir, pool_root, &detail)?;
    Ok(detail)
}

/*
read 경로는 파일 mirror를 best-effort projection으로 읽는다. authoritative 최신
상태는 planning authority store에 있지만, parallel supervisor와 여러 recovery 테스트는 pool
root 아래의 JSON mirror를 통해 runtime session history를 빠르게 복원한다. 파일이 없거나
깨졌으면 `None`으로 접어 caller가 새 detail을 만들거나 authority-backed snapshot을 계속 쓰게
한다.
*/
pub(crate) fn read_agent_session_detail_record(
    pool_root: &Path,
    session_key: &str,
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    let path = agent_session_detail_record_path(pool_root, session_key);
    let content = fs::read_to_string(path).ok()?;
    // mirror parse 실패는 authority-backed state를 계속 쓰게 하기 위해 absence로 접는다.
    serde_json::from_str(&content).ok()
}

/*
write 경로는 두 저장소를 함께 갱신한다. 먼저 planning authority의 runtime
session detail store에 upsert해 application의 source of truth를 갱신하고, 그 다음 pool root
아래 JSON mirror를 쓴다. authority write가 실패하면 파일 mirror만 앞서가는 split-brain
상태가 생길 수 있으므로 즉시 오류를 반환한다.

반대로 파일 mirror 쓰기는 authority 성공 뒤에 수행된다. mirror 실패는 caller에게 오류로
전파되지만, 이미 authority store에는 최신 detail이 남아 있다. 이 비대칭은 queue recovery와
supervisor rendering이 authority를 우선으로 보고 mirror는 호환성과 검사 편의를 위한 보조물로
다루는 현재 구조를 반영한다.
*/
pub(super) fn write_agent_session_detail_record(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> Result<(), String> {
    /*
    `workspace_dir`를 authority port에 넘기는 이유는 adapter가 어느 repo와 runtime
    namespace에 기록해야 하는지 결정하게 하기 위해서이다. application service는 sqlite,
    file-backed store, test fake 같은 실제 구현을 알지 않고, port contract만 호출한다.
    */
    planning_authority
        .upsert_runtime_session_detail(workspace_dir, detail)
        .map_err(|error| {
            format!(
                "failed to store agent session detail `{}`: {error}",
                detail.session_key
            )
        })?;

    /*
    `.agent-sessions` 디렉터리는 pool root와 함께 움직이는 runtime mirror이다.
    slot worktree 내부가 아니라 pool root 아래에 두면 cleanup으로 worktree를 reset해도 session
    history가 보존되고, supervisor snapshot이 idle로 돌아간 slot의 직전 작업 이력을 계속 보여
    줄 수 있다.
    */
    let history_dir = agent_session_history_dir(pool_root);
    ensure_directory_exists(&history_dir)
        .map_err(|error| format!("failed to create agent session history directory: {error}"))?;

    let path = agent_session_detail_record_path(pool_root, &detail.session_key);
    let temp_path = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(detail)
        .map_err(|error| format!("failed to serialize agent session detail: {error}"))?;
    /*
    파일 mirror는 temp 파일에 먼저 쓰고 rename으로 교체한다. 중간에 프로세스가
    종료되어도 기존 JSON을 절반만 덮어쓴 상태로 남기지 않기 위한 최소한의 원자성 장치이다.
    session detail은 recovery와 UI가 바로 읽는 파일이므로, partially-written JSON을 피하는 것이
    중요하다.
    */
    fs::write(&temp_path, body).map_err(|error| {
        format!(
            "failed to write temporary agent session detail `{}`: {error}",
            detail.session_key
        )
    })?;
    fs::rename(&temp_path, &path).map_err(|error| {
        format!(
            "failed to persist agent session detail `{}`: {error}",
            detail.session_key
        )
    })
}

/*
session history directory 이름은 pool root 내부의 runtime projection namespace이다.
lease 파일, distributor queue mirror와 같은 pool-local 운영 파일들과 나란히 두어, canonical repo
root와 별개로 parallel mode의 실행 상태를 한 위치에서 검사할 수 있게 한다.
*/
fn agent_session_history_dir(pool_root: &Path) -> PathBuf {
    pool_root.join(".agent-sessions")
}

/*
session_key는 slot/task/agent/branch 정보가 섞인 logical key라서 파일명으로 안전하지
않은 문자가 들어올 수 있다. 이 함수는 store key의 의미는 유지하되, 파일 경로에서는 ASCII
문자와 `-`, `_`만 남겨 platform-neutral JSON filename을 만든다. 같은 sanitization을 read와
write가 공유하므로 mirror lookup이 안정적으로 맞물린다.
*/
pub(crate) fn agent_session_detail_record_path(pool_root: &Path, session_key: &str) -> PathBuf {
    let mut filename = String::new();
    // session_key의 의미 있는 ASCII segment는 보존하고 path separator나 기호는 `_`로 접는다.
    for ch in session_key.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            filename.push(ch);
        } else {
            filename.push('_');
        }
    }

    agent_session_history_dir(pool_root).join(format!("{filename}.json"))
}
