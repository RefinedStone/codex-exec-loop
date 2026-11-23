//! A deliberately small projection: free-form diagnostic text may contain paths outside the grant.
use crate::domain::storage_analysis::StorageNode;

const NAME_BYTES: usize = 1024;
const MAX_REASONS: usize = 8;

pub(super) fn shared_node(mut node: StorageNode, root: &str) -> (StorageNode, Vec<String>) {
    let mut omitted = Vec::new();
    if node.name.len() > NAME_BYTES {
        node.name = utf8_prefix(&node.name, NAME_BYTES).to_owned();
        omitted.push("name_suffix".into());
    }
    if node.id == root {
        node.parent_id = None;
        node.ratio.denominator_bytes = None;
        node.ratio.reason = Some("shared_root".into());
        if node.name.contains(['/', '\\', ':']) {
            node.name = "공유 폴더".into();
        }
        omitted.push("outside_parent".into());
    }
    // A hardlink allocation owner can be above the granted root. Retaining its opaque ID would
    // still disclose a relationship outside the granted subtree.
    if node.allocation_owner_id.take().is_some() {
        omitted.push("allocation_owner".into());
    }
    if node.issues.len() > MAX_REASONS {
        omitted.push("additional_issue_reasons".into());
    }
    node.issues.truncate(MAX_REASONS);
    for issue in &mut node.issues {
        issue.code = safe_code(&issue.code);
        issue.message = "공유 범위에서 확인한 집계 사유입니다.".into();
    }
    if node.protection.len() > MAX_REASONS {
        omitted.push("additional_protection_reasons".into());
    }
    node.protection.truncate(MAX_REASONS);
    for reason in &mut node.protection {
        reason.code = safe_code(&reason.code);
        reason.detail = "보호 또는 검토 필요 상태입니다. 삭제 가능 판정이 아닙니다.".into();
        if chrono::DateTime::parse_from_rfc3339(&reason.observed_at).is_err() {
            reason.observed_at.clear();
        }
    }
    if node.classification.len() > MAX_REASONS {
        omitted.push("additional_classifications".into());
    }
    node.classification.truncate(MAX_REASONS);
    for classification in &mut node.classification {
        classification.kind = safe_code(&classification.kind);
        classification.evidence = "스캔 색인의 경로 이름과 프로젝트 표식에 따른 분류입니다.".into();
    }
    (node, omitted)
}

fn safe_code(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        value.to_owned()
    } else {
        "unknown".into()
    }
}

fn utf8_prefix(value: &str, maximum: usize) -> &str {
    let mut end = maximum.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}
