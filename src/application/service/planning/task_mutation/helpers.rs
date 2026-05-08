use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, PlanningFileKind, PlanningValidationReport,
    TaskAuthorityDocument,
};

/*
 * task mutation의 preview path와 commit path가 함께 쓰는 helper 모음이다. service layer는
 * create/update 중 어떤 operation을 적용할지 결정하고, 이 파일은 operation 종류와 무관하게
 * 필요한 application-side normalization을 한곳에 둔다. active direction과 stable task id
 * policy는 domain으로 내려가고, user input normalization은 semantic validation 전 경계에 남는다.
 */
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
