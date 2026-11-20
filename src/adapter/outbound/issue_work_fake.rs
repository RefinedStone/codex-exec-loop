/*
 * Issue Core 와 Evidence Vault 계약의 메모리 가짜 구현이다 (#2415). 공급자를 부르지 않고 암호화도 하지 않는다.
 * 실제 구현(GitHub 조회와 게시, Vault 암호화)이 오기 전에 Kernel 과 클라이언트가 계약에 맞춰 일하게 한다.
 */
use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultRestorePort, VaultRestoreRequest, VaultRestoreResponse,
};
use crate::application::port::outbound::issue_core_port::{
    IssueCorePort, IssuePublishRequest, IssuePublishResponse, IssueSharedCopyRequest,
    IssueSharedCopyResponse,
};
use crate::domain::issue_work::{IssueReference, IssueSharedCopy};

struct FakeIssue {
    shared_copy: IssueSharedCopy,
    restored: VaultRestoreResponse,
}

#[derive(Default)]
pub struct FakeIssueWorkAdapter {
    issues: Mutex<HashMap<IssueReference, FakeIssue>>,
}

impl FakeIssueWorkAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 이미 있는 이슈 한 건을 넣는다: 공급자가 가진 공유본과 Vault 가 가진 복원본이다.
    pub fn seed_issue(
        &self,
        issue: IssueReference,
        shared_copy: IssueSharedCopy,
        restored: VaultRestoreResponse,
    ) -> Result<()> {
        self.with_issues(|issues| {
            issues.insert(
                issue,
                FakeIssue {
                    shared_copy,
                    restored,
                },
            );
            Ok(())
        })
    }

    fn with_issues<T>(
        &self,
        action: impl FnOnce(&mut HashMap<IssueReference, FakeIssue>) -> Result<T>,
    ) -> Result<T> {
        let mut issues = self
            .issues
            .lock()
            .map_err(|_| anyhow!("the fake issue store lock is poisoned"))?;
        action(&mut issues)
    }
}

fn unknown_issue() -> anyhow::Error {
    anyhow!("the fake issue store does not have this issue")
}

impl IssueCorePort for FakeIssueWorkAdapter {
    fn read_shared_copy(&self, request: IssueSharedCopyRequest) -> Result<IssueSharedCopyResponse> {
        self.with_issues(|issues| {
            let shared_copy = issues
                .get(&request.issue)
                .ok_or_else(unknown_issue)?
                .shared_copy
                .clone();
            Ok(IssueSharedCopyResponse {
                issue: request.issue,
                shared_copy,
            })
        })
    }

    fn publish(&self, request: IssuePublishRequest) -> Result<IssuePublishResponse> {
        self.with_issues(|issues| {
            let stored = issues.get_mut(&request.issue).ok_or_else(unknown_issue)?;
            stored.shared_copy = request.shared_copy.clone();
            Ok(IssuePublishResponse {
                issue: request.issue,
                published_copy: request.shared_copy,
            })
        })
    }
}

impl EvidenceVaultRestorePort for FakeIssueWorkAdapter {
    fn restore(&self, request: VaultRestoreRequest) -> Result<VaultRestoreResponse> {
        self.with_issues(|issues| {
            issues
                .get(request.confirmation.issue())
                .map(|stored| stored.restored.clone())
                .ok_or_else(unknown_issue)
        })
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;
    use crate::application::port::outbound::evidence_vault_port::{
        EVIDENCE_VAULT_CONTRACT_VERSION, VaultKey, VaultPlaintext, VaultRecordKey,
        VaultUserConfirmation,
    };
    use crate::application::port::outbound::issue_core_port::ISSUE_CORE_CONTRACT_VERSION;
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

    const PLAINTEXT_TITLE: &str = "restore-title-sentinel";
    const PLAINTEXT_BODY: &str = "restore-body-sentinel";
    const KEY: &[u8] = b"vault-key-sentinel";

    fn issue(issue_id: &str) -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_1".to_string(),
            issue_id: issue_id.to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_string(),
            },
        }
    }

    fn record_key() -> VaultRecordKey {
        VaultRecordKey {
            key_id: "key-1".to_string(),
            key: VaultKey::new(KEY.to_vec()),
        }
    }

    fn shared_copy(body: &str) -> IssueSharedCopy {
        IssueSharedCopy {
            title: "masked title".to_string(),
            body: body.to_string(),
        }
    }

    fn seeded_adapter() -> FakeIssueWorkAdapter {
        let adapter = FakeIssueWorkAdapter::new();
        adapter
            .seed_issue(
                issue("I_1"),
                shared_copy("masked body"),
                VaultRestoreResponse {
                    title: VaultPlaintext::new(PLAINTEXT_TITLE.to_string()),
                    body: VaultPlaintext::new(PLAINTEXT_BODY.to_string()),
                },
            )
            .expect("seed should succeed");
        adapter
    }

    fn assert_no_restore_secret(text: &str) {
        for secret in [PLAINTEXT_TITLE, PLAINTEXT_BODY, "vault-key-sentinel"] {
            assert!(!text.contains(secret), "`{secret}` leaked into `{text}`");
        }
        assert!(!text.contains(&format!("{KEY:?}")));
    }

    // `T: Serialize` 이면 두 impl 이 겹쳐서 `_` 를 정할 수 없다. 그러면 컴파일이 실패한다.
    trait NotSerialize<Marker> {
        fn proven() {}
    }
    impl<T: ?Sized> NotSerialize<()> for T {}
    impl<T: ?Sized + Serialize> NotSerialize<u8> for T {}

    #[test]
    fn restore_values_cannot_enter_serialized_kernel_events_or_gateway_responses() {
        // Kernel 이벤트와 Gateway 응답은 serde 로 만든다. 아래 줄은 복원 타입이 `Serialize` 가 되면 컴파일되지 않는다.
        <VaultPlaintext as NotSerialize<_>>::proven();
        <VaultKey as NotSerialize<_>>::proven();
        <VaultRestoreResponse as NotSerialize<_>>::proven();
        <VaultRestoreRequest as NotSerialize<_>>::proven();
        <VaultUserConfirmation as NotSerialize<_>>::proven();
    }

    #[test]
    fn restore_values_are_redacted_in_debug_output() {
        let restored = seeded_adapter()
            .restore(VaultRestoreRequest {
                confirmation: VaultUserConfirmation::for_issue_in_test(issue("I_1")),
                key: record_key(),
            })
            .expect("restore should succeed");

        assert_no_restore_secret(&format!("{restored:?}"));
        assert_no_restore_secret(&format!("{:?}", VaultKey::new(KEY.to_vec())));
    }

    #[test]
    fn general_responses_carry_only_the_shared_copy() {
        let adapter = seeded_adapter();

        let read = adapter
            .read_shared_copy(IssueSharedCopyRequest {
                issue: issue("I_1"),
            })
            .expect("read should succeed");
        let published = adapter
            .publish(IssuePublishRequest {
                issue: issue("I_1"),
                shared_copy: shared_copy("masked body v2"),
            })
            .expect("publish should succeed");
        let read_after_publish = adapter
            .read_shared_copy(IssueSharedCopyRequest {
                issue: issue("I_1"),
            })
            .expect("read should succeed");

        assert_eq!(read.shared_copy, shared_copy("masked body"));
        assert_eq!(published.published_copy, shared_copy("masked body v2"));
        assert_eq!(
            read_after_publish.shared_copy,
            shared_copy("masked body v2")
        );
        assert_no_restore_secret(&serde_json::to_string(&read).expect("read serializes"));
        assert_no_restore_secret(&serde_json::to_string(&published).expect("publish serializes"));
    }

    #[test]
    fn restore_returns_plaintext_only_for_the_confirmed_issue() {
        let adapter = seeded_adapter();

        let restored = adapter
            .restore(VaultRestoreRequest {
                confirmation: VaultUserConfirmation::for_issue_in_test(issue("I_1")),
                key: record_key(),
            })
            .expect("restore should succeed");
        let other = adapter.restore(VaultRestoreRequest {
            confirmation: VaultUserConfirmation::for_issue_in_test(issue("I_2")),
            key: record_key(),
        });

        assert_eq!(restored.title.expose(), PLAINTEXT_TITLE);
        assert_eq!(restored.body.expose(), PLAINTEXT_BODY);
        assert!(other.is_err());
    }

    #[test]
    fn issue_reference_keeps_a_stable_wire_shape() {
        let value = serde_json::to_value(issue("I_1")).expect("reference serializes");

        assert_eq!(
            value,
            serde_json::json!({
                "provider": "github",
                "serverId": "github.com",
                "repositoryId": "R_1",
                "issueId": "I_1",
                "projectScope": { "workspaceDir": "/work/project" },
            })
        );
    }

    #[test]
    fn fake_adapter_reports_the_contract_versions() {
        let adapter = FakeIssueWorkAdapter::new();

        assert_eq!(
            IssueCorePort::contract_version(&adapter),
            ISSUE_CORE_CONTRACT_VERSION
        );
        assert_eq!(
            EvidenceVaultRestorePort::contract_version(&adapter),
            EVIDENCE_VAULT_CONTRACT_VERSION
        );
    }
}
