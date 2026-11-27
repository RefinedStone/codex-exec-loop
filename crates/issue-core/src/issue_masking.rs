//! 공개 정책에 따라 공유본과 복원 위치만 만든다. 원문 평문이나 Vault 타입은 결과에 없다.

use std::fmt;
use std::ops::Range;

use crate::issue_policy::{
    IssueDisclosurePolicy, IssueSensitiveSpanAction, IssueSensitiveSpanClass,
};
use crate::issue_work::IssueSharedCopy;

/// 제거한 값의 자리에 두는 글이다. 복원할 수 없다.
pub const ISSUE_REMOVED_PLACEHOLDER: &str = "[removed]";

/// 구간이 있는 칸이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueTextField {
    Title,
    Body,
}

/// 호출자가 지정한 민감 구간이다. 값은 없고 위치만 있다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSensitiveSpan {
    pub field: IssueTextField,
    /// 칸의 글에서 본 바이트 범위이다. 문자 경계에 있어야 한다.
    pub range: Range<usize>,
    /// 구간의 종류이다. 분류가 불확실하면 `None` 이다.
    pub class: Option<IssueSensitiveSpanClass>,
    /// 사용자의 답이다. 정책이 `UserConfirm` 이거나 분류가 불확실할 때만 쓴다.
    pub user_decision: Option<IssueSensitiveSpanAction>,
}

/// 공유본과 복원에 필요한 위치 계획이다. 원문이나 원문 참조를 보관하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMaskingPlan {
    pub shared_copy: IssueSharedCopy,
    pub restorable_spans: Vec<IssueRestorableSpanPlan>,
}

/// 복원용 평문을 만들지 않고 가림 묶음의 위치만 기록한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRestorableSpanPlan {
    pub marker: String,
    pub field: IssueTextField,
    /// 공유본의 해당 필드에서 표식이 시작하는 바이트 위치이다.
    pub shared_position: usize,
    /// 원본의 해당 필드에서 본 절대 바이트 범위이다. UTF-8 문자 경계에 있다.
    pub source_range: Range<usize>,
    /// 원본 필드의 절대 바이트 범위이다. 이 묶음 안에서만 겹침과 인접을 병합했다.
    /// 이 값들은 복원용 평문에도 넣지 않고 제거 자리 글로 바꾼다.
    pub removal_ranges: Vec<Range<usize>>,
}

/// 공유본을 만들 수 없는 상태이다. 구간의 번호만 있고 글은 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueMaskingBlocked {
    /// 사용자의 답이 필요한 구간이다. 값은 입력 `spans` 의 번호이다.
    NeedsUserConfirmation { span_indexes: Vec<usize> },
    /// 범위가 비었거나, 글 밖이거나, 문자 경계가 아닌 구간이다.
    InvalidSpan { span_index: usize },
}

impl fmt::Display for IssueMaskingBlocked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedsUserConfirmation { span_indexes } => write!(
                formatter,
                "{} sensitive span(s) need user confirmation",
                span_indexes.len()
            ),
            Self::InvalidSpan { span_index } => {
                write!(
                    formatter,
                    "sensitive span {span_index} has an invalid range"
                )
            }
        }
    }
}

impl std::error::Error for IssueMaskingBlocked {}

/// `policy` 에 따라 제목과 본문을 가린다. `policy` 는 대상(게시, AI 전달)의 공개 정책이다.
pub fn plan_issue_masking(
    title: &str,
    body: &str,
    spans: &[IssueSensitiveSpan],
    policy: &IssueDisclosurePolicy,
) -> Result<IssueMaskingPlan, IssueMaskingBlocked> {
    let mut unconfirmed = Vec::new();
    let mut title_spans = Vec::new();
    let mut body_spans = Vec::new();
    for (span_index, span) in spans.iter().enumerate() {
        let text = match span.field {
            IssueTextField::Title => title,
            IssueTextField::Body => body,
        };
        let range = &span.range;
        if range.start >= range.end
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            return Err(IssueMaskingBlocked::InvalidSpan { span_index });
        }
        match resolved_action(span, policy) {
            Some(remove) => match span.field {
                IssueTextField::Title => title_spans.push((range.clone(), remove)),
                IssueTextField::Body => body_spans.push((range.clone(), remove)),
            },
            None => unconfirmed.push(span_index),
        }
    }
    if !unconfirmed.is_empty() {
        return Err(IssueMaskingBlocked::NeedsUserConfirmation {
            span_indexes: unconfirmed,
        });
    }

    let mut markers = MarkerSource {
        title,
        body,
        next: 1,
    };
    let mut restorable_spans = Vec::new();
    let shared_title = plan_field(
        IssueTextField::Title,
        title,
        title_spans,
        &mut markers,
        &mut restorable_spans,
    );
    let shared_body = plan_field(
        IssueTextField::Body,
        body,
        body_spans,
        &mut markers,
        &mut restorable_spans,
    );
    Ok(IssueMaskingPlan {
        shared_copy: IssueSharedCopy {
            title: shared_title,
            body: shared_body,
        },
        restorable_spans,
    })
}

/// 구간을 제거하면 `Some(true)`, 가리면 `Some(false)`, 사용자의 답이 필요하면 `None` 이다.
fn resolved_action(span: &IssueSensitiveSpan, policy: &IssueDisclosurePolicy) -> Option<bool> {
    let policy_action = span
        .class
        .map_or(IssueSensitiveSpanAction::UserConfirm, |class| {
            policy.action_for(class)
        });
    let action = match policy_action {
        IssueSensitiveSpanAction::UserConfirm => span.user_decision?,
        decided => decided,
    };
    match action {
        IssueSensitiveSpanAction::Remove => Some(true),
        IssueSensitiveSpanAction::RestorableMask => Some(false),
        IssueSensitiveSpanAction::UserConfirm => None,
    }
}

/// 입력에 이미 있는 글과 겹치지 않는 표식을 차례로 준다.
struct MarkerSource<'a> {
    title: &'a str,
    body: &'a str,
    next: u64,
}

impl MarkerSource<'_> {
    fn next_marker(&mut self) -> String {
        loop {
            let marker = format!("[[akra-mask:{}]]", self.next);
            self.next += 1;
            if !self.title.contains(&marker) && !self.body.contains(&marker) {
                return marker;
            }
        }
    }
}

/// 실제로 겹친 구간만 한 묶음으로 만든다. 인접한 묶음은 표식과 제거 자리 글을 따로 유지한다.
fn plan_field(
    field: IssueTextField,
    text: &str,
    mut spans: Vec<(Range<usize>, bool)>,
    markers: &mut MarkerSource<'_>,
    restorable_spans: &mut Vec<IssueRestorableSpanPlan>,
) -> String {
    spans.sort_by_key(|(range, _)| range.start);
    let mut shared = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut index = 0;
    while index < spans.len() {
        let group_start = spans[index].0.start;
        let mut group_end = group_start;
        let mut has_mask = false;
        let mut removal_ranges: Vec<Range<usize>> = Vec::new();
        while index < spans.len() && (spans[index].0.start < group_end || group_end == group_start)
        {
            let (range, remove) = &spans[index];
            group_end = group_end.max(range.end);
            if *remove {
                if let Some(previous) = removal_ranges.last_mut()
                    && range.start <= previous.end
                {
                    previous.end = previous.end.max(range.end);
                } else {
                    removal_ranges.push(range.clone());
                }
            } else {
                has_mask = true;
            }
            index += 1;
        }

        shared.push_str(&text[cursor..group_start]);
        if has_mask {
            let marker = markers.next_marker();
            restorable_spans.push(IssueRestorableSpanPlan {
                marker: marker.clone(),
                field,
                shared_position: shared.len(),
                source_range: group_start..group_end,
                removal_ranges,
            });
            shared.push_str(&marker);
        } else {
            // A connected group containing only removals covers this complete range.
            shared.push_str(ISSUE_REMOVED_PLACEHOLDER);
        }
        cursor = group_end;
    }
    shared.push_str(&text[cursor..]);
    shared
}

#[cfg(test)]
mod tests;
