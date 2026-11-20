use std::ops::Range;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use super::issue_masking::{IssueSensitiveSpan, IssueTextField, mask_issue_text};
use super::issue_policy::resolve_issue_policy;
use crate::application::port::inbound::issue_handoff_port::{
    IssueHandoffPort, IssueHandoffRequest,
};
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotResolveRequest,
};
use crate::domain::issue_policy::{IssueDisclosureTarget, IssueSensitiveSpanClass};
use crate::domain::issue_work::{IssueProjectScope, IssueSharedCopy};

/*
 * 이슈 인계의 공유본을 만든다 (#2428). 원격의 제목과 본문은 다른 사람이 고친 글일 수 있다. 그래서 원시 값은 이
 * 서비스 밖으로 나가지 않는다. 나가는 값은 AI 전달 정책으로 가린 공유본뿐이다. 오류에도 원시 값은 없다.
 *
 * 승인한 공유본의 해시가 있으면 원격 값이 그 해시와 같을 때만 인계한다. 다르면 복원 평문이 다시 들어왔을 수
 * 있으므로 막는다.
 */

/// 접두사 뒤에 이 수 이상의 토큰 글자가 있어야 토큰으로 본다.
const MIN_TOKEN_TAIL_CHARS: usize = 16;
/// 키 뒤의 값과 `Bearer` 뒤의 값은 이 수 이상의 글자여야 비밀값으로 본다.
const MIN_SECRET_VALUE_CHARS: usize = 8;
const TOKEN_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "sk-",
    "AKIA",
];
const SECRET_KEYS: &[&str] = &["password", "passwd", "secret", "token", "api_key", "apikey"];

pub struct IssueHandoffService {
    snapshots: Arc<dyn IssueSnapshotPort>,
    policy_source: Arc<dyn IssuePolicySourcePort>,
}

impl IssueHandoffService {
    pub fn new(
        snapshots: Arc<dyn IssueSnapshotPort>,
        policy_source: Arc<dyn IssuePolicySourcePort>,
    ) -> Self {
        Self {
            snapshots,
            policy_source,
        }
    }
}

impl IssueHandoffPort for IssueHandoffService {
    fn handoff_copy(&self, request: IssueHandoffRequest) -> Result<IssueSharedCopy> {
        // 정책을 읽지 못하면 인계를 막는다. 기본값으로 대신하지 않는다.
        let user = self.policy_source.load_user_policy_toml()?;
        let project = match &request.workspace_dir {
            Some(workspace_dir) => self.policy_source.load_project_policy_toml(workspace_dir)?,
            None => None,
        };
        let policy = resolve_issue_policy(user.as_deref(), project.as_deref())?;

        let snapshot = self.snapshots.resolve(IssueSnapshotResolveRequest {
            locator: request.locator,
            project_scope: IssueProjectScope {
                workspace_dir: request.workspace_dir.unwrap_or_default(),
            },
        })?;
        if let Some(approved) = &request.approved
            && snapshot.verified_shared_copy(approved).is_none()
        {
            bail!("the issue changed after its shared copy was approved")
        }

        let title = snapshot.raw_fields.expose_title();
        let body = snapshot.raw_fields.expose_body();
        let mut spans = secret_spans(IssueTextField::Title, title);
        spans.extend(secret_spans(IssueTextField::Body, body));
        let outcome = mask_issue_text(
            title,
            body,
            &spans,
            policy.disclosure_for(IssueDisclosureTarget::AiHandoff),
        )
        .map_err(|blocked| anyhow!("the issue cannot be handed off: {blocked}"))?;
        // 가림 구간의 평문은 여기서 버린다. 인계에는 복원본이 없다.
        Ok(outcome.shared_copy)
    }
}

fn secret_spans(field: IssueTextField, text: &str) -> Vec<IssueSensitiveSpan> {
    secret_ranges(text)
        .into_iter()
        .map(|range| IssueSensitiveSpan {
            field,
            range,
            class: Some(IssueSensitiveSpanClass::Secret),
            user_decision: None,
        })
        .collect()
}

/// 비밀값으로 보이는 구간이다. 겹칠 수 있다. 모든 경계는 ASCII 글자 옆이므로 문자 경계이다.
/// shortcut: 알려진 모양(토큰 접두사, Bearer, URL 의 계정 정보, `키=값`, PEM 개인 키)만 찾는다. 이름과 경로는
/// 찾지 않는다. 다른 모양이 필요하면 사용자 지정 검증기(#2431)에서 구간을 받는다.
fn secret_ranges(text: &str) -> Vec<Range<usize>> {
    // ASCII 만 바꾸므로 바이트 위치가 `text` 와 같다.
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();

    for prefix in TOKEN_PREFIXES {
        for (start, _) in text.match_indices(prefix) {
            let inside_word = text[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
            let tail_start = start + prefix.len();
            let tail = text[tail_start..]
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                .count();
            if !inside_word && tail >= MIN_TOKEN_TAIL_CHARS {
                ranges.push(start..tail_start + tail);
            }
        }
    }

    for (start, matched) in lower.match_indices("bearer ") {
        let value = value_range(text, start + matched.len());
        if value.len() >= MIN_SECRET_VALUE_CHARS {
            ranges.push(value);
        }
    }

    for (start, matched) in text.match_indices("://") {
        let authority_start = start + matched.len();
        let authority = &text[authority_start..];
        let end = authority
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '/' | '?' | '#' | '"' | '\'' | '`' | '<' | '>')
            })
            .unwrap_or(authority.len());
        let authority = &authority[..end];
        if let Some(at) = authority.rfind('@')
            && authority[..at].contains(':')
        {
            ranges.push(authority_start..authority_start + at);
        }
    }

    for key in SECRET_KEYS {
        for (start, _) in lower.match_indices(key) {
            let after_key = text[start + key.len()..].trim_start_matches(['"', '\'', ' ']);
            let Some(after_separator) = after_key.strip_prefix(['=', ':']) else {
                continue;
            };
            let value_text = after_separator.trim_start();
            let value = quoted_value_range(text, text.len() - value_text.len());
            if text[value.clone()].chars().count() >= MIN_SECRET_VALUE_CHARS {
                ranges.push(value);
            }
        }
    }

    for (start, matched) in text.match_indices("-----BEGIN ") {
        let rest = &text[start..];
        let header = rest.lines().next().unwrap_or_default();
        if !header.contains("PRIVATE KEY") {
            continue;
        }
        // 끝 줄이 없으면 글의 끝까지 버린다.
        let end = rest[matched.len()..]
            .find("-----END ")
            .map(|offset| matched.len() + offset)
            .and_then(|end_marker| {
                let after_marker = end_marker + "-----END ".len();
                rest[after_marker..]
                    .find("-----")
                    .map(|offset| after_marker + offset + "-----".len())
            })
            .unwrap_or(rest.len());
        ranges.push(start..start + end);
    }

    ranges
}

/// 인용 값은 공백이나 쉼표를 포함할 수 있다. 닫히지 않은 인용 값은 나머지 전체를 가린다.
fn quoted_value_range(text: &str, start: usize) -> Range<usize> {
    let Some(quote @ ('"' | '\'')) = text[start..].chars().next() else {
        return value_range(text, start);
    };
    let value_start = start + quote.len_utf8();
    let mut escaped = false;
    for (offset, character) in text[value_start..].char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == quote {
            return value_start..value_start + offset;
        }
    }
    value_start..text.len()
}

/// `start` 에서 시작하는 값의 범위이다. 공백, 따옴표, 쉼표, 쌍반점 앞에서 끝난다.
fn value_range(text: &str, start: usize) -> Range<usize> {
    let end = text[start..]
        .find(|character: char| {
            character.is_whitespace() || matches!(character, '"' | '\'' | '`' | ',' | ';')
        })
        .map_or(text.len(), |offset| start + offset);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::issue_snapshot_port::{
        IssueRawFields, IssueSnapshotReadRequest, IssueSnapshotResponse,
    };
    use crate::domain::issue_work::{
        IssueComparisonBasis, IssueFieldHash, IssueLocator, IssueProvider, IssueReference,
    };

    // 시험용으로 만든 값이다. 실제 비밀이 아니다.
    const SYNTHETIC_TOKEN: &str = "ghp_SyntheticCanary0000";
    const SYNTHETIC_PASSWORD: &str = "synthetic-password-BBBB2222";
    const RESTORED_PLAINTEXT: &str = "/home/synthetic-user/app";

    struct FixedSnapshot {
        title: String,
        body: String,
    }

    impl IssueSnapshotPort for FixedSnapshot {
        fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
            let raw_fields = IssueRawFields::new(self.title.clone(), self.body.clone());
            Ok(IssueSnapshotResponse {
                issue: IssueReference {
                    provider: IssueProvider::GitHub,
                    server_id: "github.com".to_string(),
                    repository_id: "R_1".to_string(),
                    issue_id: "I_1".to_string(),
                    project_scope: request.project_scope,
                },
                comparison_basis: IssueComparisonBasis {
                    field_hash: raw_fields.field_hash(),
                    provider_updated_at: "2026-11-20T00:00:00Z".to_string(),
                },
                raw_fields,
            })
        }

        fn read(&self, _request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
            bail!("not used")
        }
    }

    struct FixedPolicySource {
        user: Option<&'static str>,
    }

    impl IssuePolicySourcePort for FixedPolicySource {
        fn load_user_policy_toml(&self) -> Result<Option<String>> {
            Ok(self.user.map(str::to_string))
        }

        fn load_project_policy_toml(&self, _workspace_dir: &str) -> Result<Option<String>> {
            Ok(None)
        }
    }

    fn handoff(
        title: &str,
        body: &str,
        user_policy: Option<&'static str>,
        approved: Option<IssueFieldHash>,
    ) -> Result<IssueSharedCopy> {
        IssueHandoffService::new(
            Arc::new(FixedSnapshot {
                title: title.to_string(),
                body: body.to_string(),
            }),
            Arc::new(FixedPolicySource { user: user_policy }),
        )
        .handoff_copy(IssueHandoffRequest {
            locator: IssueLocator {
                owner: "sample".to_string(),
                repository: "widgets".to_string(),
                number: 42,
            },
            workspace_dir: None,
            approved,
        })
    }

    #[test]
    fn secrets_from_an_external_edit_are_not_in_the_shared_copy() {
        let title = format!("로그인 실패 {SYNTHETIC_TOKEN}");
        let body = format!(
            "한글 본문이다.\npassword: {SYNTHETIC_PASSWORD}\nAuthorization: Bearer synthetic-bearer-CCCC3333\n\
             git clone https://synthetic-user:synthetic-pass-DDDD4444@example.invalid/repo.git\n\
             -----BEGIN RSA PRIVATE KEY-----\nsynthetic-key-material\n-----END RSA PRIVATE KEY-----\n끝."
        );
        let copy = handoff(&title, &body, None, None).expect("handoff should succeed");

        assert_eq!(copy.title, "로그인 실패 [removed]");
        assert_eq!(
            copy.body,
            "한글 본문이다.\npassword: [removed]\nAuthorization: Bearer [removed]\n\
             git clone https://[removed]@example.invalid/repo.git\n[removed]\n끝."
        );
    }

    #[test]
    fn ordinary_text_stays_as_it_is() {
        let body =
            "token: string 을 받는다. https://github.com/sample/widgets 와 task-runner 를 본다.";
        let copy = handoff("Crash on start", body, None, None).expect("handoff should succeed");
        assert_eq!(copy.title, "Crash on start");
        assert_eq!(copy.body, body);
    }

    #[test]
    fn restored_plaintext_from_an_external_edit_blocks_the_handoff() {
        let approved = IssueFieldHash::of_fields("Crash", "[[akra-mask:1]] 에서 실패");
        let copy = handoff(
            "Crash",
            "[[akra-mask:1]] 에서 실패",
            None,
            Some(approved.clone()),
        )
        .expect("the approved copy should be handed off");
        assert_eq!(copy.body, "[[akra-mask:1]] 에서 실패");

        let edited = format!("{RESTORED_PLAINTEXT} 에서 실패");
        let error = handoff("Crash", &edited, None, Some(approved))
            .expect_err("an edited issue should not be handed off");
        assert!(!format!("{error:#}").contains(RESTORED_PLAINTEXT));
    }

    #[test]
    fn a_policy_that_asks_the_user_blocks_the_handoff_without_the_secret() {
        let body = format!("token={SYNTHETIC_TOKEN}");
        let error = handoff(
            "Crash",
            &body,
            Some("[ai_handoff.spans]\nsecret = \"user_confirm\"\n"),
            None,
        )
        .expect_err("an unconfirmed secret should block the handoff");
        assert!(!format!("{error:#}").contains(SYNTHETIC_TOKEN));

        assert!(handoff("Crash", "본문", Some("not toml ["), None).is_err());
    }
    #[test]
    fn explicit_secrets_preserve_no_quoted_or_url_suffixes() {
        for (raw, expected) in [
            (
                r#"password="synthetic123 remaining-private-phrase""#,
                r#"password="[removed]""#,
            ),
            (
                r#"password='synthetic phrase, with spaces'"#,
                r#"password='[removed]'"#,
            ),
            (
                r#"password="synthetic\" still-private""#,
                r#"password="[removed]""#,
            ),
            ("password=correcthorsebatterystaple", "password=[removed]"),
            ("api_key=alphabeticsecret", "api_key=[removed]"),
            (
                "https://user:synthetic123,private@example.invalid/repo",
                "https://[removed]@example.invalid/repo",
            ),
            (
                "https://user:synthetic;private@example.invalid/repo?key=value",
                "https://[removed]@example.invalid/repo?key=value",
            ),
            (
                r#"password="unterminated private value"#,
                r#"password="[removed]"#,
            ),
        ] {
            let copy = handoff(raw, raw, None, None).expect("handoff");
            assert_eq!(copy.title, expected, "{raw}");
            assert_eq!(copy.body, expected, "{raw}");
        }
    }
}
