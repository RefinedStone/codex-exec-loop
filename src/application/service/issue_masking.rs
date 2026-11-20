use std::fmt;
use std::ops::Range;

use crate::application::port::outbound::evidence_vault_port::VaultPlaintext;
use crate::domain::issue_policy::{
    IssueDisclosurePolicy, IssueSensitiveSpanAction, IssueSensitiveSpanClass,
};
use crate::domain::issue_work::IssueSharedCopy;

/*
 * 이슈의 제목과 본문에서 민감 구간을 처리해 공유본과 가림 구간을 만든다 (#2419).
 *
 * 제거 대상의 값은 이 함수 밖으로 나가지 않는다. 공유본, 가림 구간의 평문, 오류에 모두 없다. 가림 구간의
 * 평문은 `VaultPlaintext` 에만 있다. 호출자는 이 결과만 초안, 자동 저장, Vault 에 쓴다.
 *
 * 구간을 찾는 일은 하지 않는다. 호출자가 구간과 종류를 준다. 처리는 공개 정책이 정한다.
 */

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

/// Vault 에 맡길 가림 구간이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMaskedSpan {
    /// 공유본에서 이 구간을 대신하는 표식이다. 제목과 본문 전체에서 하나뿐이다.
    pub marker: String,
    pub field: IssueTextField,
    /// 공유본의 칸에서 표식이 시작하는 바이트 위치이다.
    pub position: usize,
    /// 가린 평문이다. 그 안의 제거 대상은 이미 `ISSUE_REMOVED_PLACEHOLDER` 이다.
    pub plaintext: VaultPlaintext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMaskingOutcome {
    pub shared_copy: IssueSharedCopy,
    pub masked_spans: Vec<IssueMaskedSpan>,
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
pub fn mask_issue_text(
    title: &str,
    body: &str,
    spans: &[IssueSensitiveSpan],
    policy: &IssueDisclosurePolicy,
) -> Result<IssueMaskingOutcome, IssueMaskingBlocked> {
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
    let mut masked_spans = Vec::new();
    let shared_title = mask_field(
        IssueTextField::Title,
        title,
        title_spans,
        &mut markers,
        &mut masked_spans,
    );
    let shared_body = mask_field(
        IssueTextField::Body,
        body,
        body_spans,
        &mut markers,
        &mut masked_spans,
    );
    Ok(IssueMaskingOutcome {
        shared_copy: IssueSharedCopy {
            title: shared_title,
            body: shared_body,
        },
        masked_spans,
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

/// 칸 하나를 가린다. 겹친 구간은 한 묶음이다. 묶음 안의 제거 범위는 먼저 버리고, 가림이 하나라도 있으면
/// 묶음 전체를 표식 하나로 바꾼다.
fn mask_field(
    field: IssueTextField,
    text: &str,
    mut spans: Vec<(Range<usize>, bool)>,
    markers: &mut MarkerSource<'_>,
    masked_spans: &mut Vec<IssueMaskedSpan>,
) -> String {
    spans.sort_by_key(|(range, _)| range.start);
    let mut shared = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut index = 0;
    while index < spans.len() {
        let group_start = spans[index].0.start;
        let mut group_end = group_start;
        let mut has_mask = false;
        // 제거 범위를 버린 묶음의 글이다.
        let mut kept = String::new();
        let mut kept_cursor = group_start;
        while index < spans.len() && (spans[index].0.start < group_end || group_end == group_start)
        {
            let (range, remove) = &spans[index];
            group_end = group_end.max(range.end);
            if *remove {
                if range.start > kept_cursor {
                    kept.push_str(&text[kept_cursor..range.start]);
                }
                // 앞의 제거 범위와 이어지면 자리 글을 다시 넣지 않는다.
                if range.start > kept_cursor || kept.is_empty() {
                    kept.push_str(ISSUE_REMOVED_PLACEHOLDER);
                }
                kept_cursor = kept_cursor.max(range.end);
            } else {
                has_mask = true;
            }
            index += 1;
        }
        if group_end > kept_cursor {
            kept.push_str(&text[kept_cursor..group_end]);
        }

        shared.push_str(&text[cursor..group_start]);
        if has_mask {
            let marker = markers.next_marker();
            masked_spans.push(IssueMaskedSpan {
                marker: marker.clone(),
                field,
                position: shared.len(),
                plaintext: VaultPlaintext::new(kept),
            });
            shared.push_str(&marker);
        } else {
            shared.push_str(&kept);
        }
        cursor = group_end;
    }
    shared.push_str(&text[cursor..]);
    shared
}

#[cfg(test)]
mod tests {
    use super::*;

    // 시험용으로 만든 값이다. 실제 비밀이 아니다.
    const SYNTHETIC_TOKEN: &str = "synthetic-token-AAAA1111";
    const SYNTHETIC_PASSWORD: &str = "synthetic-password-BBBB2222";

    fn span(
        field: IssueTextField,
        text: &str,
        needle: &str,
        class: Option<IssueSensitiveSpanClass>,
    ) -> IssueSensitiveSpan {
        let start = text.find(needle).expect("needle should be in the text");
        IssueSensitiveSpan {
            field,
            range: start..start + needle.len(),
            class,
            user_decision: None,
        }
    }

    fn mask(
        title: &str,
        body: &str,
        spans: &[IssueSensitiveSpan],
    ) -> Result<IssueMaskingOutcome, IssueMaskingBlocked> {
        mask_issue_text(title, body, spans, &IssueDisclosurePolicy::builtin())
    }

    #[test]
    fn names_and_paths_get_unique_markers_in_title_and_body() {
        let title = "alice 의 빌드 실패";
        let body = "alice 가 /home/alice/app 에서 실행했다";
        let outcome = mask(
            title,
            body,
            &[
                span(
                    IssueTextField::Title,
                    title,
                    "alice",
                    Some(IssueSensitiveSpanClass::Name),
                ),
                span(
                    IssueTextField::Body,
                    body,
                    "alice",
                    Some(IssueSensitiveSpanClass::Name),
                ),
                span(
                    IssueTextField::Body,
                    body,
                    "/home/alice/app",
                    Some(IssueSensitiveSpanClass::Path),
                ),
            ],
        )
        .expect("masking should succeed");

        assert_eq!(outcome.shared_copy.title, "[[akra-mask:1]] 의 빌드 실패");
        assert_eq!(
            outcome.shared_copy.body,
            "[[akra-mask:2]] 가 [[akra-mask:3]] 에서 실행했다"
        );
        let plaintexts: Vec<_> = outcome
            .masked_spans
            .iter()
            .map(|masked| (masked.field, masked.plaintext.expose()))
            .collect();
        assert_eq!(
            plaintexts,
            [
                (IssueTextField::Title, "alice"),
                (IssueTextField::Body, "alice"),
                (IssueTextField::Body, "/home/alice/app"),
            ]
        );
        for masked in &outcome.masked_spans {
            let shared = match masked.field {
                IssueTextField::Title => &outcome.shared_copy.title,
                IssueTextField::Body => &outcome.shared_copy.body,
            };
            assert!(shared[masked.position..].starts_with(&masked.marker));
        }
    }

    #[test]
    fn removed_secrets_stay_out_of_every_output() {
        let title = format!("로그인 실패 {SYNTHETIC_PASSWORD}");
        let body = format!("log: user=alice token={SYNTHETIC_TOKEN} end");
        let log = format!("user=alice token={SYNTHETIC_TOKEN}");
        let outcome = mask(
            &title,
            &body,
            &[
                span(
                    IssueTextField::Title,
                    &title,
                    SYNTHETIC_PASSWORD,
                    Some(IssueSensitiveSpanClass::Secret),
                ),
                // 로그 구간 안에 토큰이 있다. 토큰은 Vault 평문에도 없어야 한다.
                span(
                    IssueTextField::Body,
                    &body,
                    &log,
                    Some(IssueSensitiveSpanClass::Log),
                ),
                span(
                    IssueTextField::Body,
                    &body,
                    SYNTHETIC_TOKEN,
                    Some(IssueSensitiveSpanClass::Secret),
                ),
            ],
        )
        .expect("masking should succeed");

        assert_eq!(outcome.shared_copy.title, "로그인 실패 [removed]");
        assert_eq!(outcome.shared_copy.body, "log: [[akra-mask:1]] end");
        assert_eq!(outcome.masked_spans.len(), 1);
        assert_eq!(
            outcome.masked_spans[0].plaintext.expose(),
            "user=alice token=[removed]"
        );
        // 일반 이벤트와 Gateway 응답은 공유본의 serde 출력이다.
        let event = serde_json::to_string(&outcome.shared_copy).expect("shared copy serializes");
        let debug = format!("{outcome:?}");
        for output in [&event, &debug] {
            assert!(!output.contains(SYNTHETIC_TOKEN));
            assert!(!output.contains(SYNTHETIC_PASSWORD));
            assert!(!output.contains("alice"));
        }
    }

    #[test]
    fn overlapping_spans_become_one_marker() {
        let body = "host build-01.internal path /srv/build-01/out";
        let outcome = mask(
            "t",
            body,
            &[
                span(
                    IssueTextField::Body,
                    body,
                    "build-01.internal path /srv",
                    Some(IssueSensitiveSpanClass::Name),
                ),
                span(
                    IssueTextField::Body,
                    body,
                    "/srv/build-01/out",
                    Some(IssueSensitiveSpanClass::Path),
                ),
            ],
        )
        .expect("masking should succeed");

        assert_eq!(outcome.shared_copy.body, "host [[akra-mask:1]]");
        assert_eq!(
            outcome.masked_spans[0].plaintext.expose(),
            "build-01.internal path /srv/build-01/out"
        );
    }

    #[test]
    fn overlapping_removals_leave_one_placeholder() {
        let body = format!("a {SYNTHETIC_TOKEN}{SYNTHETIC_PASSWORD} b");
        let both = format!("{SYNTHETIC_TOKEN}{SYNTHETIC_PASSWORD}");
        let outcome = mask(
            "t",
            &body,
            &[
                span(
                    IssueTextField::Body,
                    &body,
                    &both,
                    Some(IssueSensitiveSpanClass::Secret),
                ),
                span(
                    IssueTextField::Body,
                    &body,
                    SYNTHETIC_PASSWORD,
                    Some(IssueSensitiveSpanClass::Secret),
                ),
            ],
        )
        .expect("masking should succeed");

        assert_eq!(outcome.shared_copy.body, "a [removed] b");
        assert!(outcome.masked_spans.is_empty());
    }

    #[test]
    fn markers_skip_text_already_in_the_input() {
        let title = "[[akra-mask:1]] 은 예전 표식이다";
        let body = "alice 와 [[akra-mask:2]]";
        let outcome = mask(
            title,
            body,
            &[span(
                IssueTextField::Body,
                body,
                "alice",
                Some(IssueSensitiveSpanClass::Name),
            )],
        )
        .expect("masking should succeed");

        assert_eq!(outcome.masked_spans[0].marker, "[[akra-mask:3]]");
        assert_eq!(outcome.shared_copy.title, title);
        assert_eq!(
            outcome.shared_copy.body,
            "[[akra-mask:3]] 와 [[akra-mask:2]]"
        );
    }

    #[test]
    fn uncertain_spans_need_user_confirmation_and_errors_carry_no_text() {
        let body = format!("alice {SYNTHETIC_TOKEN}");
        let mut policy = IssueDisclosurePolicy::builtin();
        policy.name = IssueSensitiveSpanAction::UserConfirm;
        let mut spans = vec![
            span(
                IssueTextField::Body,
                &body,
                "alice",
                Some(IssueSensitiveSpanClass::Name),
            ),
            span(IssueTextField::Body, &body, SYNTHETIC_TOKEN, None),
        ];

        let blocked =
            mask_issue_text("t", &body, &spans, &policy).expect_err("masking should be blocked");
        assert_eq!(
            blocked,
            IssueMaskingBlocked::NeedsUserConfirmation {
                span_indexes: vec![0, 1]
            }
        );
        for output in [format!("{blocked:?}"), blocked.to_string()] {
            assert!(!output.contains(SYNTHETIC_TOKEN));
            assert!(!output.contains("alice"));
        }
        // 입력 구간의 Debug 에도 글이 없다.
        assert!(!format!("{spans:?}").contains(SYNTHETIC_TOKEN));

        spans[0].user_decision = Some(IssueSensitiveSpanAction::RestorableMask);
        spans[1].user_decision = Some(IssueSensitiveSpanAction::Remove);
        let outcome = mask_issue_text("t", &body, &spans, &policy).expect("masking should succeed");
        assert_eq!(outcome.shared_copy.body, "[[akra-mask:1]] [removed]");
    }

    #[test]
    fn user_decision_does_not_weaken_a_decided_policy() {
        let body = format!("token {SYNTHETIC_TOKEN}");
        let mut secret = span(
            IssueTextField::Body,
            &body,
            SYNTHETIC_TOKEN,
            Some(IssueSensitiveSpanClass::Secret),
        );
        secret.user_decision = Some(IssueSensitiveSpanAction::RestorableMask);

        let outcome = mask("t", &body, &[secret]).expect("masking should succeed");
        assert_eq!(outcome.shared_copy.body, "token [removed]");
        assert!(outcome.masked_spans.is_empty());
    }

    #[test]
    fn invalid_ranges_are_rejected() {
        let body = "한글 본문";
        for range in [0..0, 0..1, Range { start: 3, end: 2 }, 0..100] {
            let blocked = mask(
                "t",
                body,
                &[IssueSensitiveSpan {
                    field: IssueTextField::Body,
                    range,
                    class: Some(IssueSensitiveSpanClass::Name),
                    user_decision: None,
                }],
            )
            .expect_err("masking should be blocked");
            assert_eq!(blocked, IssueMaskingBlocked::InvalidSpan { span_index: 0 });
        }
    }
}
