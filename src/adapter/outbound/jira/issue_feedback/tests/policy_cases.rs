use super::*;
use crate::adapter::outbound::db::sqlite_issue_automation_store::SqliteIssueAutomationStoreAdapter;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_autonomy::resolve_runtime_policy;
use crate::application::service::issue_feedback::validate_send_policy;
use crate::application::service::issue_policy::{IssuePolicyService, issue_policy_version};
use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};

const POLICY: &str = r#"
[binding.jira]
site_url = "https://example.atlassian.net"
cloud_id = "cloud-1"
project_id = "10"
expected_account_id = "account-1"
jql = "assignee = currentUser()"
mode = "propose"
[binding.jira.close_transition]
transition_id = "31"
from_status_ids = ["1"]
to_status_id = "3"
[feedback]
comment = true
transition = true
[close]
when = "integration"
"#;

struct Policies {
    automation: Mutex<String>,
    disclosure: Mutex<Option<String>>,
    unreadable: AtomicBool,
}
impl IssueAutomationPolicySourcePort for Policies {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        if self.unreadable.load(Ordering::SeqCst) {
            bail!("synthetic unreadable policy");
        }
        Ok(Some(self.automation.lock().unwrap().clone()))
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}
impl IssuePolicySourcePort for Policies {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(self.disclosure.lock().unwrap().clone())
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

struct PolicyFixture {
    base: Fixture,
    policies: Arc<Policies>,
    store: Arc<SqliteIssueAutomationStoreAdapter>,
    workspace: String,
    directory: std::path::PathBuf,
}
impl Drop for PolicyFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl PolicyFixture {
    fn new() -> Self {
        let directory = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-jira-send-policy-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&directory).unwrap();
        let directory = std::fs::canonicalize(directory).unwrap();
        let runtime = directory.join("runtime");
        crate::private_directory::ensure_private_directory(&runtime).unwrap();
        let workspace = directory.to_string_lossy().into_owned();
        let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
            Ok(runtime.clone())
        }));
        let policies = Arc::new(Policies {
            automation: Mutex::new(POLICY.into()),
            disclosure: Mutex::new(None),
            unreadable: AtomicBool::new(false),
        });
        let automation = IssueAutomationPolicyService::new(policies.clone());
        let selection = IssueAutonomyIntent {
            command_id: "select-propose".into(),
            level: IssueAutonomyLevel::Propose,
            expected_revision: 0,
            policy_version: automation.resolve(&workspace).unwrap().version,
        };
        store.begin_issue_autonomy(&workspace, &selection).unwrap();
        store.finish_issue_autonomy(&workspace, &selection).unwrap();
        let disclosure = IssuePolicyService::new(policies.clone());
        let selected_workspace = workspace.clone();
        let selected_store = store.clone();
        let mut base = Fixture::new();
        base.adapter.validate_send_policy = Arc::new(move |intent| {
            validate_send_policy(
                &selected_workspace,
                selected_store.as_ref(),
                &automation,
                &disclosure,
                intent,
            )
        });
        Self {
            base,
            policies,
            store,
            workspace,
            directory,
        }
    }

    fn stamp(&self, mut intent: IssueFeedbackIntent) -> IssueFeedbackIntent {
        intent.issue.project_scope.workspace_dir = self.workspace.clone();
        if let IssueFeedbackOperation::UpdateComment { owner, .. } = &mut intent.operation {
            owner.issue = intent.issue.clone();
        }
        let automation = IssueAutomationPolicyService::new(self.policies.clone());
        intent.automation_policy_version =
            resolve_runtime_policy(&self.workspace, self.store.as_ref(), &automation)
                .unwrap()
                .0
                .version;
        assert_ne!(
            intent.automation_policy_version,
            automation.resolve(&self.workspace).unwrap().version,
            "the durable runtime choice must be included in the send fence"
        );
        intent.disclosure_policy_version = issue_policy_version(
            &IssuePolicyService::new(self.policies.clone())
                .resolve(&self.workspace)
                .unwrap(),
        );
        intent.validate().unwrap();
        intent
    }

    fn intent(&self, operation: u8) -> IssueFeedbackIntent {
        match operation {
            0 => self.stamp(self.base.intent("create-policy", "Shared status")),
            1 => {
                let first = self.stamp(self.base.intent("original-policy", "Original status"));
                assert!(matches!(
                    self.base.adapter.write(&first).unwrap(),
                    IssueFeedbackWriteResult::Acknowledged { .. }
                ));
                self.stamp(comment_cases::update(&self.base, &first, "1000"))
            }
            2 => self.stamp(self.base.close_intent()),
            _ => unreachable!(),
        }
    }

    fn assert_blocked(&self, intent: &IssueFeedbackIntent, writes_before: usize) {
        assert_eq!(
            self.base.adapter.write(intent).unwrap(),
            IssueFeedbackWriteResult::Unconfirmed
        );
        assert_eq!(self.base.api.writes.load(Ordering::SeqCst), writes_before);
        for _ in 0..2 {
            assert!(!matches!(
                self.base.adapter.reconcile(intent).unwrap(),
                IssueFeedbackReadResult::Confirmed(_)
            ));
        }
        assert_eq!(self.base.api.writes.load(Ordering::SeqCst), writes_before);
    }
}

#[test]
fn comment_or_transition_policy_changes_during_preflight_send_nothing() {
    for operation in [0, 2] {
        let fixture = PolicyFixture::new();
        let intent = fixture.intent(operation);
        let policies = fixture.policies.clone();
        *fixture.base.api.after_read.lock().unwrap() = Some(Box::new(move |request| {
            if matches!(
                (operation, request),
                (0, JiraIssueFeedbackReadRequest::Comments { .. })
                    | (2, JiraIssueFeedbackReadRequest::Transitions { .. })
            ) {
                let flag = if operation == 0 {
                    "comment"
                } else {
                    "transition"
                };
                *policies.automation.lock().unwrap() =
                    POLICY.replace(&format!("{flag} = true"), &format!("{flag} = false"));
            }
        }));
        fixture.assert_blocked(&intent, 0);
    }
}

#[test]
fn sharing_policy_changes_at_actual_send_block_create_update_and_transition() {
    for operation in 0..3 {
        let fixture = PolicyFixture::new();
        let intent = fixture.intent(operation);
        let before = fixture.base.api.writes.load(Ordering::SeqCst);
        let policies = fixture.policies.clone();
        *fixture.base.api.before_send.lock().unwrap() = Some(Box::new(move || {
            *policies.disclosure.lock().unwrap() =
                Some("[publish.spans]\npath = 'remove'\n".into());
        }));
        fixture.assert_blocked(&intent, before);
    }
}

#[test]
fn unreadable_or_malformed_policy_at_actual_send_cannot_send() {
    for malformed_disclosure in [false, true] {
        let fixture = PolicyFixture::new();
        let intent = fixture.intent(0);
        let policies = fixture.policies.clone();
        *fixture.base.api.before_send.lock().unwrap() = Some(Box::new(move || {
            if malformed_disclosure {
                *policies.disclosure.lock().unwrap() = Some("malformed = [".into());
            } else {
                policies.unreadable.store(true, Ordering::SeqCst);
            }
        }));
        fixture.assert_blocked(&intent, 0);
    }
}

#[test]
fn changed_or_pending_runtime_selection_at_actual_send_cannot_send() {
    for finish in [false, true] {
        let fixture = PolicyFixture::new();
        let intent = fixture.intent(0);
        let workspace = fixture.workspace.clone();
        let store = fixture.store.clone();
        let selection = IssueAutonomyIntent {
            command_id: "select-off".into(),
            level: IssueAutonomyLevel::Off,
            expected_revision: store.issue_autonomy_selection(&workspace).unwrap().revision,
            policy_version: intent.automation_policy_version.clone(),
        };
        *fixture.base.api.before_send.lock().unwrap() = Some(Box::new(move || {
            store.begin_issue_autonomy(&workspace, &selection).unwrap();
            if finish {
                store.finish_issue_autonomy(&workspace, &selection).unwrap();
            }
        }));
        fixture.assert_blocked(&intent, 0);
    }
}

#[test]
fn unchanged_effective_policy_allows_create_update_and_transition() {
    for operation in 0..3 {
        let fixture = PolicyFixture::new();
        let intent = fixture.intent(operation);
        let before = fixture.base.api.writes.load(Ordering::SeqCst);
        assert!(matches!(
            fixture.base.adapter.write(&intent).unwrap(),
            IssueFeedbackWriteResult::Acknowledged { .. }
        ));
        assert_eq!(fixture.base.api.writes.load(Ordering::SeqCst), before + 1);
        assert!(matches!(
            fixture.base.adapter.reconcile(&intent).unwrap(),
            IssueFeedbackReadResult::Confirmed(_)
        ));
    }
}
