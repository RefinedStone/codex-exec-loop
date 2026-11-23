use super::*;
use crate::domain::issue_work::IssueProvider;

fn scope() -> &'static str {
    "[binding.jira]\nsite_url='https://fixture.atlassian.net'\ncloud_id='cloud-one'\nproject_id='10000'\nexpected_account_id='account-one'\njql='statusCategory != Done'\nmode='auto'\n"
}
fn transition() -> &'static str {
    "[binding.jira.close_transition]\ntransition_id='31'\nfrom_status_ids=['10','20','10']\nto_status_id='30'\n"
}

#[test]
fn explicit_jira_binding_preserves_github_legacy_bytes_and_uses_human_only_mode() {
    let legacy = resolve_issue_automation_policy(None, None).unwrap();
    assert!(
        !serde_json::to_value(&legacy.policy).unwrap()["binding"]
            .as_object()
            .unwrap()
            .contains_key("jira")
    );
    assert_eq!(
        legacy.policy.mode_for_provider(IssueProvider::Jira),
        IssueAutomationAutonomy::Off
    );
    let jira =
        resolve_issue_automation_policy(Some("[authority]\nautonomy_cap='auto'"), Some(scope()))
            .unwrap();
    assert_eq!(jira.policy.selected_provider(), IssueProvider::Jira);
    assert_eq!(
        jira.policy.selected_mode(),
        IssueAutomationAutonomy::Propose
    );
    assert_eq!(
        jira.policy.selected_autonomy_cap(),
        IssueAutomationAutonomy::Propose
    );
    assert_eq!(
        jira.policy.mode_for_provider(IssueProvider::GitHub),
        IssueAutomationAutonomy::Off
    );
    assert!(!jira.policy.feedback.transition);
    assert!(jira.policy.binding.jira.unwrap().close_transition.is_none());
}

#[test]
fn project_can_only_narrow_the_explicit_user_transition_for_the_same_source() {
    let user = format!(
        "{}{}[feedback]\ntransition=true\n[close]\nwhen='integration'",
        scope(),
        transition()
    );
    let base = resolve_issue_automation_policy(Some(&user), None).unwrap();
    assert_eq!(
        base.policy
            .binding
            .jira
            .as_ref()
            .unwrap()
            .close_transition
            .as_ref()
            .unwrap()
            .from_status_ids,
        ["10", "20"]
    );
    let narrower = "[binding.jira.close_transition]\ntransition_id='31'\nfrom_status_ids=['20']\nto_status_id='30'";
    let resolved = resolve_issue_automation_policy(Some(&user), Some(narrower)).unwrap();
    assert_eq!(
        resolved
            .policy
            .binding
            .jira
            .unwrap()
            .close_transition
            .unwrap()
            .from_status_ids,
        ["20"]
    );
    for project in [
        narrower.replace("['20']", "['99']"),
        narrower.replace("'31'", "'41'"),
        narrower.replace("'30'", "'40'"),
        scope().replace("project_id='10000'", "project_id='20000'"),
    ] {
        assert!(resolve_issue_automation_policy(Some(&user), Some(&project)).is_err());
    }
    let disabled = resolve_issue_automation_policy(
        Some(&user),
        Some("[feedback]\ntransition=false\n[close]\nwhen='never'"),
    )
    .unwrap();
    assert!(!disabled.policy.feedback.transition);
}

#[test]
fn jira_transition_requires_a_scoped_user_grant_and_never_guesses_workflow_names() {
    assert!(
        resolve_issue_automation_policy(None, Some(&format!("{}{}", scope(), transition())))
            .is_err()
    );
    assert!(resolve_issue_automation_policy(Some(transition()), Some(scope())).is_err());
    assert!(
        resolve_issue_automation_policy(Some("[feedback]\ntransition=true"), Some(scope()))
            .is_err()
    );
    for replacement in [
        transition().replace("'31'", "'Done'"),
        transition().replace("['10','20','10']", "[]"),
        transition().replace("to_status_id='30'", "to_status_id='10'"),
    ] {
        assert!(
            resolve_issue_automation_policy(Some(&format!("{}{}", scope(), replacement)), None)
                .is_err()
        );
    }
}

#[test]
fn jira_source_scope_is_complete_and_rejects_unsupported_query_or_write_capabilities() {
    for project in [
        scope().replace("https://", "http://"),
        scope().replace("project_id='10000'", "project_id='KEY'"),
        scope().replace("cloud_id='cloud-one'\n", ""),
        scope().replace(
            "statusCategory != Done",
            "statusCategory != Done ORDER BY updated DESC",
        ),
        format!("{}deployment='server'", scope()),
    ] {
        assert!(resolve_issue_automation_policy(None, Some(&project)).is_err());
    }
    assert!(
        resolve_issue_automation_policy(Some("[feedback]\nlabels=true"), Some(scope())).is_err()
    );
    assert!(
        resolve_issue_automation_policy(Some("[close]\nwhen='provider_link'"), Some(scope()))
            .is_err()
    );
}

#[test]
fn an_existing_auto_runtime_choice_cannot_grant_jira_automatic_start_or_delivery() {
    use crate::domain::issue_automation::autonomy::{IssueAutonomyLevel, IssueAutonomySelection};
    let file =
        resolve_issue_automation_policy(Some("[authority]\nautonomy_cap='auto'"), Some(scope()))
            .unwrap();
    let selection = IssueAutonomySelection {
        requested_level: Some(IssueAutonomyLevel::Auto),
        revision: 4,
        pending: None,
    };
    let effective = crate::application::service::issue_autonomy::effective_policy(file, &selection);
    assert_eq!(
        effective.policy.selected_mode(),
        IssueAutomationAutonomy::Propose
    );
    assert_eq!(
        effective.policy.selected_autonomy_cap(),
        IssueAutomationAutonomy::Propose
    );
}
