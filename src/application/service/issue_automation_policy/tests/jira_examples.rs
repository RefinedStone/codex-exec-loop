use super::*;
use crate::domain::issue_work::IssueProvider;

fn examples() -> Vec<&'static str> {
    const DOCUMENT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/reference/issue-jira-cloud.md"
    ));
    let examples = DOCUMENT
        .split("```toml\n")
        .skip(1)
        .map(|block| block.split_once("\n```").expect("closed TOML example").0)
        .collect::<Vec<_>>();
    assert_eq!(examples.len(), 4, "two complete user/project examples");
    examples
}

#[test]
fn documented_jira_comment_configuration_preserves_user_grants_and_human_only_mode() {
    let examples = examples();
    let resolved = resolve_issue_automation_policy(Some(examples[0]), Some(examples[1]))
        .expect("the documented user and repository files must form a valid policy");
    let policy = resolved.policy;
    assert_eq!(policy.selected_provider(), IssueProvider::Jira);
    assert_eq!(policy.selected_mode(), IssueAutomationAutonomy::Propose);
    assert_eq!(
        policy.selected_autonomy_cap(),
        IssueAutomationAutonomy::Propose
    );
    assert_eq!(
        policy.mode_for_provider(IssueProvider::GitHub),
        IssueAutomationAutonomy::Off
    );
    assert!(policy.feedback.comment);
    assert!(!policy.feedback.labels);
    assert!(!policy.feedback.transition);
    assert_eq!(policy.close.when, IssueAutomationCloseWhen::Never);
    assert_eq!(policy.select.mode, IssueAutomationSelectionMode::AllOpen);
    let binding = policy.binding.jira.as_ref().unwrap();
    assert_eq!(binding.site_url, "https://example.atlassian.net");
    assert_eq!(binding.cloud_id, "cloud-123");
    assert_eq!(binding.project_id, "10001");
    assert_eq!(binding.expected_account_id, "account-operator-123");
    assert!(binding.close_transition.is_none());
    assert_eq!(policy.select.trusted_actors.len(), 1);
    assert_eq!(
        policy.select.trusted_actors[0].actor_id,
        "account-author-123"
    );

    let project_grant = format!("{}\n[feedback]\ncomment = true\n", examples[1]);
    let no_user = resolve_issue_automation_policy(None, Some(&project_grant)).unwrap();
    assert!(!no_user.policy.feedback.comment);
    assert!(no_user.policy.select.trusted_actors.is_empty());
}

#[test]
fn documented_jira_transition_configuration_only_narrows_the_same_user_scope() {
    let examples = examples();
    let granted = resolve_issue_automation_policy(Some(examples[2]), None).unwrap();
    let narrowed = resolve_issue_automation_policy(Some(examples[2]), Some(examples[3]))
        .expect("the documented repository must only narrow the user's transition grant");
    assert_ne!(narrowed.version, granted.version);
    assert!(narrowed.policy.feedback.transition);
    assert_eq!(
        narrowed.policy.close.when,
        IssueAutomationCloseWhen::Integration
    );
    let transition = narrowed
        .policy
        .binding
        .jira
        .as_ref()
        .unwrap()
        .close_transition
        .as_ref()
        .unwrap();
    assert_eq!(transition.transition_id, "31");
    assert_eq!(transition.from_status_ids, ["10001"]);
    assert_eq!(transition.to_status_id, "10002");
    for project in [
        examples[3].replace(
            "from_status_ids = [\"10001\"]",
            "from_status_ids = [\"99999\"]",
        ),
        examples[3].replace("transition_id = \"31\"", "transition_id = \"41\""),
        examples[3].replace("to_status_id = \"10002\"", "to_status_id = \"10003\""),
    ] {
        assert!(resolve_issue_automation_policy(Some(examples[2]), Some(&project)).is_err());
    }
    assert!(resolve_issue_automation_policy(None, Some(examples[3])).is_err());
    let disabled = format!(
        "{}\n[feedback]\ntransition = false\n",
        examples[3].replace("when = \"integration\"", "when = \"never\"")
    );
    let disabled = resolve_issue_automation_policy(Some(examples[2]), Some(&disabled)).unwrap();
    assert!(!disabled.policy.feedback.transition);
    assert_eq!(disabled.policy.close.when, IssueAutomationCloseWhen::Never);
}
