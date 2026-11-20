use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use super::issue_policy::resolve_issue_policy;
use crate::application::port::inbound::issue_handoff_port::{
    IssueHandoffPort, IssueHandoffRequest,
};
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotResolveRequest,
};
use crate::domain::issue_masking::{IssueTextField, plan_issue_masking};
use crate::domain::issue_policy::IssueDisclosureTarget;
use crate::domain::issue_secret_spans::secret_spans;
use crate::domain::issue_work::{IssueProjectScope, IssueSharedCopy};

/*
 * 이슈 인계의 공유본을 만든다 (#2428). 원격의 제목과 본문은 다른 사람이 고친 글일 수 있다. 그래서 원시 값은 이
 * 서비스 밖으로 나가지 않는다. 나가는 값은 AI 전달 정책으로 가린 공유본뿐이다. 오류에도 원시 값은 없다.
 *
 * 승인한 공유본의 해시가 있으면 원격 값이 그 해시와 같을 때만 인계한다. 다르면 복원 평문이 다시 들어왔을 수
 * 있으므로 막는다.
 */

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
        let plan = plan_issue_masking(
            title,
            body,
            &spans,
            policy.disclosure_for(IssueDisclosureTarget::AiHandoff),
        )
        .map_err(|blocked| anyhow!("the issue cannot be handed off: {blocked}"))?;
        // 인계에는 공유본만 쓰고 복원용 평문은 만들지 않는다.
        Ok(plan.shared_copy)
    }
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
