use super::*;
use crate::domain::issue_work::IssueProvider;

const TEAM: &str = "6f1c1a52-0000-4000-8000-00000000a001";
const VIEWER: &str = "6f1c1a52-0000-4000-8000-00000000b001";
const STARTED: &str = "6f1c1a52-0000-4000-8000-00000000d001";
const REVIEW: &str = "6f1c1a52-0000-4000-8000-00000000d003";
const DONE: &str = "6f1c1a52-0000-4000-8000-00000000d002";

fn scope() -> String {
    format!("[binding.linear]\nteam_id='{TEAM}'\nexpected_viewer_id='{VIEWER}'\nmode='auto'\n")
}
fn close_state(from: &[&str]) -> String {
    format!("[binding.linear.close_state]\nfrom_state_ids={from:?}\nto_state_id='{DONE}'\n")
}

#[test]
fn a_repository_opt_in_selects_the_user_team_with_a_propose_ceiling() {
    let user = format!("[authority]\nautonomy_cap='auto'\n{}", scope());
    // A user file alone never binds another repository to the team.
    let unselected = resolve_issue_automation_policy(Some(&user), None).unwrap();
    assert!(unselected.policy.binding.linear.is_none());
    assert_eq!(unselected.policy.selected_provider(), IssueProvider::GitHub);
    let selected =
        resolve_issue_automation_policy(Some(&user), Some("[binding.linear]\n")).unwrap();
    assert_eq!(selected.policy.selected_provider(), IssueProvider::Linear);
    assert_eq!(
        selected.policy.selected_mode(),
        IssueAutomationAutonomy::Propose
    );
    assert_eq!(
        selected.policy.selected_autonomy_cap(),
        IssueAutomationAutonomy::Propose
    );
    assert_eq!(
        selected.policy.mode_for_provider(IssueProvider::GitHub),
        IssueAutomationAutonomy::Off
    );
    let binding = selected.policy.binding.linear.unwrap();
    assert_eq!(
        (
            binding.team_id.as_str(),
            binding.expected_viewer_id.as_str()
        ),
        (TEAM, VIEWER)
    );
}

#[test]
fn half_a_source_scope_two_sources_or_unsupported_feedback_are_blocked() {
    let blocked: [(Option<&str>, String); 6] = [
        (None, format!("[binding.linear]\nteam_id='{TEAM}'\n")),
        (
            None,
            format!(
                "{}[binding.jira]\nsite_url='https://fixture.atlassian.net'\ncloud_id='c'\nproject_id='1'\nexpected_account_id='a'\njql='x = 1'\n",
                scope()
            ),
        ),
        (Some("[feedback]\nlabels=true\n"), scope()),
        (Some("[close]\nwhen='provider_link'\n"), scope()),
        // A state change needs the user's explicit grant.
        (
            Some("[feedback]\ntransition=true\n[close]\nwhen='integration'\n"),
            scope(),
        ),
        (
            None,
            format!("[binding.linear]\nteam_id='../team'\nexpected_viewer_id='{VIEWER}'\n"),
        ),
    ];
    for (user, project) in blocked {
        assert!(
            resolve_issue_automation_policy(user, Some(&project)).is_err(),
            "{project}"
        );
    }
}

#[test]
fn a_project_can_only_narrow_the_user_state_grant_for_the_same_team() {
    let user = format!(
        "{}{}[feedback]\ntransition=true\n[close]\nwhen='integration'\n",
        scope(),
        close_state(&[STARTED, REVIEW])
    );
    let narrowed = resolve_issue_automation_policy(
        Some(&user),
        Some(&format!("[binding.linear]\n{}", close_state(&[REVIEW]))),
    )
    .unwrap();
    let close = narrowed.policy.binding.linear.unwrap().close_state.unwrap();
    assert_eq!(close.from_state_ids, [REVIEW]);
    assert_eq!(close.to_state_id, DONE);
    let expanded = format!(
        "[binding.linear]\n{}",
        close_state(&[STARTED, REVIEW, "6f1c1a52-0000-4000-8000-00000000d004"])
    );
    assert!(resolve_issue_automation_policy(Some(&user), Some(&expanded)).is_err());
    let moved = format!(
        "[binding.linear]\nteam_id='6f1c1a52-0000-4000-8000-00000000a999'\nexpected_viewer_id='{VIEWER}'\n"
    );
    assert!(resolve_issue_automation_policy(Some(&user), Some(&moved)).is_err());
    // A project file cannot create a grant the user did not make.
    assert!(
        resolve_issue_automation_policy(
            Some(&scope()),
            Some(&format!("[binding.linear]\n{}", close_state(&[STARTED])))
        )
        .is_err()
    );
}
