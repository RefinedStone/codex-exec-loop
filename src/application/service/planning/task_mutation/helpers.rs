use std::collections::{BTreeSet, HashSet};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, SecondsFormat, Utc};

use super::{PlanningTaskMutationSource, TASK_ID_HASH_CHARS};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PlanningFileKind,
    PlanningValidationReport, TaskAuthorityDocument,
};

/*
 * task mutation의 preview path와 commit path가 함께 쓰는 helper 모음이다. service layer는
 * create/update 중 어떤 operation을 적용할지 결정하고, 이 파일은 operation 종류와 무관하게
 * 항상 지켜야 하는 cross-cutting invariant를 한곳에 둔다. active direction 선택, reference
 * integrity, priority bound, stable task id, user input normalization이 여기서 먼저 정리된 뒤
 * task authority document가 persistence 경계로 넘어간다.
 */
pub(super) fn select_direction<'a>(
    requested_direction_id: Option<&str>,
    directions: &'a DirectionCatalogDocument,
) -> Result<&'a DirectionDefinition> {
    // 명시 direction은 권위 있는 선택이지만 paused/done direction을 target할 수 없다.
    // 명시값이 없으면 default lane을 먼저 쓰고, 그 lane이 없는 오래된 catalog를 위해 임의의
    // active lane으로 후퇴한다.
    if let Some(requested_direction_id) = requested_direction_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let direction = find_direction(requested_direction_id, directions)?;
        if direction.state != DirectionState::Active {
            bail!(
                "direction `{}` is not active; task mutations can only create tasks for active directions",
                direction.id.trim()
            );
        }
        return Ok(direction);
    }
    if let Some(direction) = directions.directions.iter().find(|direction| {
        direction.id.trim() == "general-workstream" && direction.state == DirectionState::Active
    }) {
        return Ok(direction);
    }

    directions
        .directions
        .iter()
        .find(|direction| direction.state == DirectionState::Active)
        .ok_or_else(|| anyhow!("task mutation requires an active planning direction"))
}

pub(super) fn find_direction<'a>(
    direction_id: &str,
    directions: &'a DirectionCatalogDocument,
) -> Result<&'a DirectionDefinition> {
    // direction id는 file/command identifier로도 쓰인다. catalog 검색 전에 shape를 검증해
    // path fragment나 공백이 diagnostic, authority record, supporting file path로 흘러가지 못하게 한다.
    let direction_id = required_id(direction_id, "direction id")?;
    directions
        .directions
        .iter()
        .find(|direction| direction.id.trim() == direction_id)
        .ok_or_else(|| anyhow!("direction `{direction_id}` does not exist"))
}

pub(super) fn direction_title(
    directions: &DirectionCatalogDocument,
    direction_id: &str,
) -> Option<String> {
    directions
        .directions
        .iter()
        .find(|direction| direction.id.trim() == direction_id.trim())
        .map(|direction| direction.title.trim().to_string())
}

pub(super) fn default_relation_note(
    raw_note: Option<&str>,
    direction: &DirectionDefinition,
) -> String {
    // relation note는 authority schema의 audit field다. caller가 더 강한 설명을 주지 않으면
    // direction summary에 task를 묶어, 나중에 operator가 왜 이 task가 해당 lane에 속하는지
    // 최소한의 근거를 볼 수 있게 한다.
    raw_note
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "Task supports direction `{}`: {}",
                direction.id.trim(),
                direction.summary.trim()
            )
        })
}

pub(super) fn validate_task_reference(
    link_kind: &'static str,
    task_id: &str,
    target_task_id: &str,
    task_ids: &HashSet<String>,
) -> Result<()> {
    let normalized = target_task_id.trim();
    // dependency/blocker link는 같은 authority document 안의 graph edge다. blank/self/unknown
    // edge가 있으면 queue projection이 runnable 여부를 추측하게 되므로, validation 재실행 전에
    // mutation layer에서 먼저 거부한다.
    if normalized.is_empty() {
        bail!("task `{task_id}` contains a blank {link_kind}");
    }
    if normalized == task_id {
        bail!("task `{task_id}` cannot reference itself as a {link_kind}");
    }
    if !task_ids.contains(normalized) {
        bail!("task `{task_id}` references unknown {link_kind} `{normalized}`");
    }
    Ok(())
}

pub(super) fn validate_priorities(task_authority: &TaskAuthorityDocument) -> Result<()> {
    // domain은 base priority와 dynamic delta를 합쳐 combined priority를 계산한다. 입력값과
    // 최종 projection이 모두 UI/ranking 범위 안에 있어야 queue ordering이 음수/초과값에 흔들리지 않는다.
    for task in &task_authority.tasks {
        if !(0..=100).contains(&task.base_priority) {
            bail!(
                "task `{}` base_priority must be within 0..100",
                task.id.trim()
            );
        }
        if !(-100..=100).contains(&task.dynamic_priority_delta) {
            bail!(
                "task `{}` dynamic_priority_delta must be within -100..100",
                task.id.trim()
            );
        }
        if !(0..=100).contains(&task.combined_priority()) {
            bail!(
                "task `{}` combined priority must stay within 0..100",
                task.id.trim()
            );
        }
    }
    Ok(())
}

pub(super) fn reject_task_validation_errors(report: &PlanningValidationReport) -> Result<()> {
    // mutation service는 direction/result-output warning과 공존할 수 있지만, task-authority
    // error는 proposed mutation이 invalid ledger를 저장한다는 뜻이다. task error를 하나의
    // operator message로 접어 preview와 commit path가 같은 실패 표면을 갖게 한다.
    let errors = report
        .errors()
        .into_iter()
        .filter(|issue| issue.file_kind == PlanningFileKind::TaskAuthority)
        .map(|issue| issue.message.as_str())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        return Ok(());
    }
    bail!(
        "planning task mutation failed validation: {}",
        errors.join("; ")
    )
}

pub(super) fn build_task_id(
    source: PlanningTaskMutationSource,
    generated_at: DateTime<Utc>,
    title: &str,
    collision_suffix: Option<u32>,
) -> String {
    let timestamp = generated_at.format("%Y%m%dT%H%M%SZ");
    // task id는 ledger에서 사람이 읽을 수 있어야 하지만 preview/retry flow에서는 충분히
    // deterministic해야 한다. source + timestamp + title hash가 base이고, collision suffix는
    // repository가 실제 충돌을 보고한 뒤에만 붙는다.
    let base = format!(
        "task-{}-{timestamp}-{}",
        source.id_slug(),
        stable_short_hash(title)
    );
    match collision_suffix {
        Some(suffix) => format!("{base}-{suffix}"),
        None => base,
    }
}

fn stable_short_hash(value: &str) -> String {
    // ID suffix 가독성을 위한 deterministic FNV 축약값이다. 보안용 digest가 아니며, 충돌은
    // 상위 id allocation retry가 numeric suffix로 해결한다.
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..TASK_ID_HASH_CHARS].to_string()
}

pub(super) fn increment_suffix(suffix: Option<u32>) -> Option<u32> {
    // collision retry는 suffix 없음 -> 1 -> 2 순서로 이동한다. preview와 commit path가 같은
    // helper를 써야 충돌 처리 로그와 최종 id가 같은 규칙을 따른다.
    Some(suffix.unwrap_or(0) + 1)
}

pub(super) fn task_id_exists(task_authority: &TaskAuthorityDocument, task_id: &str) -> bool {
    // authority 안의 id 비교는 trim된 값으로 한다. hand-authored 문서의 주변 공백 때문에
    // collision guard가 빠지지 않게 하려는 방어적 비교다.
    task_authority
        .tasks
        .iter()
        .any(|task| task.id.trim() == task_id.trim())
}

pub(super) fn required_id<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    // id는 authority graph와 file/command surface를 오가므로 text보다 더 엄격하다. 공백과 path
    // separator를 금지해 later diagnostics나 generated path가 애매해지는 일을 막는다.
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    if value.contains(char::is_whitespace) || value.contains('/') || value.contains('\\') {
        bail!("{label} `{value}` must not contain whitespace or path separators");
    }
    Ok(value)
}

pub(super) fn required_text<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    // free-form text field도 빈 값은 service boundary에서 막는다. 이후 layer가 blank title이나
    // blank description을 default로 추측하지 않게 하는 최소 guard다.
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value)
}

pub(super) fn normalize_references(values: &[String]) -> Vec<String> {
    // reference array는 user-visible ordered list가 아니라 semantic set이다. trim, blank 제거,
    // 중복 제거, 정렬을 적용해 반복 preview가 안정적인 authority JSON과 읽기 쉬운 diff를 만든다.
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    // authority timestamp는 초 단위 RFC3339 UTC 문자열로 고정한다. mutation source가 달라도
    // ledger diff와 queue tie-breaker가 같은 time format을 쓰게 한다.
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}
