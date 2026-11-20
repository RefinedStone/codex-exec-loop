use std::sync::Mutex;

use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::{
    IssueAutomationCloseWhen, IssueAutomationGateAction, IssueAutomationHumanRule,
    IssueAutomationRiskClass, IssueAutomationSelectionMode,
};

use super::*;

#[test]
fn ordinary_values_merge_per_leaf_and_risk_rules_replace_as_an_ordered_list() {
    let user = r#"
        [binding.github]
        repository = "user/repository"
        mode = "auto"
        [authority]
        autonomy_cap = "auto"
        [select]
        mode = "all_open"
        labels_any = ["user-label"]
        [[classify.rule]]
        class = "high"
        labels_any = ["user-high"]
        [[classify.rule]]
        class = "low"
        labels_any = ["user-low"]
        [gates.gated.normal]
        start = "timeout_arbiter"
        plan = "timeout_arbiter"
        [timeouts]
        start = "48h"
        remind_before = "4h"
        [close]
        when = "provider_link"
    "#;
    let project = r#"
        [binding.github]
        repository = "project/repository"
        mode = "gated"
        [select]
        labels_any = ["project-label"]
        [[classify.rule]]
        class = "normal"
        paths_any = ["src/**"]
        [gates.gated.normal]
        start = "human"
        [timeouts]
        remind_before = "2h"
        [close]
        when = "never"
    "#;
    let resolved = resolve_issue_automation_policy(Some(user), Some(project)).expect("policy");
    let policy = resolved.policy;
    assert_eq!(
        policy.binding.github.repository.as_deref(),
        Some("project/repository")
    );
    assert_eq!(policy.binding.github.mode, IssueAutomationAutonomy::Gated);
    assert_eq!(policy.select.mode, IssueAutomationSelectionMode::AllOpen);
    assert_eq!(policy.select.labels_any, ["project-label"]);
    assert_eq!(policy.classify.rule.len(), 1);
    assert_eq!(
        policy.classify.rule[0].class,
        IssueAutomationRiskClass::Normal
    );
    assert_eq!(policy.classify.rule[0].paths_any, ["src/**"]);
    assert_eq!(
        policy.gates.gated.normal.start,
        IssueAutomationGateAction::Human
    );
    assert_eq!(
        policy.gates.gated.normal.plan,
        IssueAutomationGateAction::TimeoutArbiter
    );
    assert_eq!(
        policy.gates.gated.low.start,
        IssueAutomationGateAction::Human
    );
    assert_eq!(policy.timeouts.start_seconds, 48 * 60 * 60);
    assert_eq!(policy.timeouts.remind_before_seconds, 2 * 60 * 60);
    assert_eq!(policy.close.when, IssueAutomationCloseWhen::Never);
}

#[test]
fn duration_units_have_one_effect_value_and_default_hash() {
    let default = resolve_issue_automation_policy(None, None).expect("default");
    for (start, remind) in [
        ("24h", "2h"),
        ("1440m", "120m"),
        ("86400s", "7200s"),
        ("1d", "7200s"),
    ] {
        let text = format!("[timeouts]\nstart = \"{start}\"\nremind_before = \"{remind}\"");
        let resolved = resolve_issue_automation_policy(Some(&text), None).expect("duration");
        assert_eq!(resolved, default, "{start}/{remind}");
    }
    assert_eq!(
        resolve_issue_automation_policy(Some("# file\nschema_version = 1"), Some(""))
            .expect("empty files"),
        default,
    );
}

#[test]
fn durations_accept_the_documented_bounds_and_zero_disables_reminders() {
    for (duration, seconds) in [
        ("1s", 1),
        ("1m", 60),
        ("1h", 3600),
        ("1d", 86400),
        ("2592000s", 2592000),
        ("43200m", 2592000),
        ("720h", 2592000),
        ("30d", 2592000),
    ] {
        let text = format!("[timeouts]\nstart = \"{duration}\"\nremind_before = \"0s\"");
        let resolved = resolve_issue_automation_policy(Some(&text), None).expect("duration bound");
        assert_eq!(resolved.policy.timeouts.start_seconds, seconds);
        assert_eq!(resolved.policy.timeouts.remind_before_seconds, 0);
    }
    for zero in ["0s", "0m", "0h", "0d"] {
        let text = format!("[timeouts]\nstart = \"1s\"\nremind_before = \"{zero}\"");
        assert!(resolve_issue_automation_policy(Some(&text), None).is_ok());
    }
}

#[test]
fn duration_syntax_and_overflow_are_rejected_without_panicking() {
    for duration in [
        "",
        "0s",
        "01s",
        "1",
        "1.5h",
        "-1h",
        "+1h",
        " 1h",
        "1h ",
        "1 h",
        "1H",
        "31d",
        "721h",
        "43201m",
        "2592001s",
        "18446744073709551615d",
        "18446744073709551616s",
        "9999999999999999999999999999999999999999999h",
        "一s",
        "1🙂",
        "🙂s",
    ] {
        let text = format!("[timeouts]\nstart = \"{duration}\"\nremind_before = \"0s\"");
        let error = resolve_issue_automation_policy(Some(&text), None).expect_err("bad duration");
        assert_eq!(error.scope, IssueAutomationPolicyScope::User);
    }
}

#[test]
fn reminder_constraints_apply_to_explicit_layers_and_inherited_values() {
    let user = "[timeouts]\nstart = \"1h\"\nremind_before = \"2h\"";
    let project = "[timeouts]\nstart = \"3h\"\nremind_before = \"0s\"";
    let error =
        resolve_issue_automation_policy(Some(user), Some(project)).expect_err("invalid user layer");
    assert_eq!(error.scope, IssueAutomationPolicyScope::User);
    assert_eq!(error.reason, "must_precede_start");

    let error = resolve_issue_automation_policy(None, Some("[timeouts]\nstart = \"2h\""))
        .expect_err("default reminder equals start");
    assert_eq!(error.scope, IssueAutomationPolicyScope::Effective);
    assert_eq!(error.reason, "must_precede_start");

    let resolved = resolve_issue_automation_policy(
        Some("[timeouts]\nstart = \"1h\""),
        Some("[timeouts]\nremind_before = \"30m\""),
    )
    .expect("partial layers are validated together");
    assert_eq!(resolved.policy.timeouts.start_seconds, 3600);
    assert_eq!(resolved.policy.timeouts.remind_before_seconds, 1800);
}

#[test]
fn unknown_keys_enums_and_invalid_field_shapes_block_the_file() {
    let invalid_documents = [
        "schema_version = 2",
        "schema_version = -1",
        "schema_version = 1.0",
        "unexpected = 'value'",
        "[binding.gitlab]\nrepository = 'owner/repo'",
        "[binding.github]\nmode = 'administrator'",
        "[binding.github]\nrepository = 'https://github.com/owner/repo'",
        "[authority]\nautonomy_cap = 'unlimited'",
        "[select]\nmode = 'all'",
        "[select]\nlabels_any = [42]",
        "[select]\nlabels_any = [' padded']",
        "[select]\nlabels_any = ['']",
        "[select]\ntrusted_actors = ['login']",
        "[select]\ntrusted_actors = [{provider='github',server_id='github.com',actor_id='U_1',login='name'}]",
        "[select]\ntrusted_actors = [{provider='gitlab',server_id='host',actor_id='U_1'}]",
        "[select]\ntrusted_actors = [{provider='github',server_id=' ',actor_id='U_1'}]",
        "[select]\ntrusted_actors = [{provider='github',server_id='github.com',actor_id=''}]",
        "[classify]\nrule = [{class='critical',labels_any=['security']}]",
        "[classify]\nrule = [{class='high'}]",
        "[classify]\nrule = [{class='high',paths_any=['../outside']}]",
        "[gates.observe.low]\nstart = 'human'",
        "[gates.gated.critical]\nstart = 'human'",
        "[gates.auto.low]\nclose = 'human'",
        "[gates.auto.low]\nstart = 'allow'",
        "[timeouts]\nstart = 86400",
        "[timeouts]\nplan = '24h'",
        "[budget]\nmax_running = -1",
        "[budget]\nmax_started_per_day = 4294967296",
        "[budget]\nmax_attempts = 1.5",
        "[feedback]\ncomment = 'yes'",
        "[close]\nwhen = 'always'",
        "[protection]\npaths = ['/absolute']",
        "[human_required]\nrules = ['unknown']",
        "[human_required]\nadditional = [{labels_any=['hold'],gates=[]}]",
        "[human_required]\nadditional = [{gates=['start']}]",
        "[human_required]\nadditional = [{labels_any=['hold'],gates=['execute']}]",
        "[human_required]\nadditional = [{id='replace',labels_any=['hold'],gates=['start']}]",
        "[human_required]\nadditional = [{labels_any=['hold'],paths_any=['a**b'],gates=['start']}]",
    ];
    for document in invalid_documents {
        for scope in [
            IssueAutomationPolicyScope::User,
            IssueAutomationPolicyScope::Project,
        ] {
            let result = match scope {
                IssueAutomationPolicyScope::User => {
                    resolve_issue_automation_policy(Some(document), None)
                }
                IssueAutomationPolicyScope::Project => {
                    resolve_issue_automation_policy(None, Some(document))
                }
                IssueAutomationPolicyScope::Effective => unreachable!(),
            };
            assert_eq!(result.expect_err(document).scope, scope);
        }
    }
}

#[test]
fn mode_and_gate_constraints_are_checked_before_a_later_override() {
    for (mode, gate, action) in [
        ("propose", "start", "auto"),
        ("propose", "delivery", "timeout_default"),
        ("gated", "delivery", "auto"),
        ("gated", "merge", "auto"),
        ("auto", "spec", "auto"),
        ("auto", "plan", "timeout_default"),
        ("auto", "merge", "timeout_arbiter"),
        ("auto", "merge", "timeout_default"),
    ] {
        let user = format!("[gates.{mode}.low]\n{gate} = \"{action}\"");
        let project = format!("[gates.{mode}.low]\n{gate} = \"human\"");
        let error = resolve_issue_automation_policy(Some(&user), Some(&project))
            .expect_err("invalid explicit gate remains invalid");
        assert_eq!(error.scope, IssueAutomationPolicyScope::User);
        assert_eq!(error.reason, "action_not_allowed_for_gate_or_mode");
    }
}

#[test]
fn empty_selection_is_distinct_from_all_open_and_counts_allow_zero() {
    let empty = resolve_issue_automation_policy(None, Some("[select]\nlabels_any=[]"))
        .expect("empty selection");
    assert_eq!(
        empty.policy.select.mode,
        IssueAutomationSelectionMode::LabelsAny
    );
    assert!(empty.policy.select.labels_any.is_empty());
    let all =
        resolve_issue_automation_policy(None, Some("[select]\nmode='all_open'\nlabels_any=[]"))
            .expect("explicit all");
    assert_eq!(
        all.policy.select.mode,
        IssueAutomationSelectionMode::AllOpen
    );
    for number in [0, u32::MAX] {
        let document = format!(
            "[budget]\nmax_running={number}\nmax_started_per_day={number}\nmax_attempts={number}"
        );
        let resolved =
            resolve_issue_automation_policy(Some(&document), None).expect("budget bound");
        assert_eq!(resolved.policy.budget.max_running, number);
        assert_eq!(resolved.policy.budget.max_started_per_day, number);
        assert_eq!(resolved.policy.budget.max_attempts, number);
    }
}

#[test]
fn schema_examples_resolve_with_the_documented_restrictions() {
    let user = include_str!("../../../../schema/akra-issue-automation.user.example.toml");
    let project = include_str!("../../../../schema/akra-issue-automation.project.example.toml");
    resolve_issue_automation_policy(Some(user), None).expect("user example");
    resolve_issue_automation_policy(None, Some(project)).expect("project example");
    let resolved =
        resolve_issue_automation_policy(Some(user), Some(project)).expect("both examples");
    assert_eq!(
        resolved.policy.binding.github.mode,
        IssueAutomationAutonomy::Observe
    );
    assert_eq!(
        resolved.policy.authority.autonomy_cap,
        IssueAutomationAutonomy::Propose
    );
    assert_eq!(resolved.policy.select.trusted_actors.len(), 1);
    assert_eq!(
        resolved.policy.select.trusted_actors[0].actor_id,
        "U_example_maintainer"
    );
    assert_eq!(resolved.policy.budget.max_running, 1);
    assert_eq!(resolved.policy.budget.max_started_per_day, 3);
    assert_eq!(resolved.policy.budget.max_attempts, 2);
    assert!(
        resolved
            .policy
            .protection
            .paths
            .contains(&"src/auth/**".to_string())
    );
    assert!(
        resolved
            .policy
            .protection
            .paths
            .contains(&"src/payments/**".to_string())
    );
    assert_eq!(
        resolved.policy.human_required.rules,
        IssueAutomationHumanRule::ALL
    );
    assert_eq!(resolved.policy.human_required.additional.len(), 2);
    assert_eq!(resolved.policy.timeouts.start_seconds, 12 * 3600);
    assert!(!resolved.policy.feedback.transition);
}

struct FakeSource {
    user: Option<&'static str>,
    project: Option<&'static str>,
    failed_scope: Option<IssueAutomationPolicyScope>,
    projects_read: Mutex<Vec<String>>,
}

impl IssueAutomationPolicySourcePort for FakeSource {
    fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
        if self.failed_scope == Some(IssueAutomationPolicyScope::User) {
            anyhow::bail!("private-token-value private-path user parse details");
        }
        Ok(self.user.map(str::to_string))
    }

    fn load_project_policy_toml(&self, workspace_dir: &str) -> anyhow::Result<Option<String>> {
        self.projects_read
            .lock()
            .expect("read calls")
            .push(workspace_dir.to_string());
        if self.failed_scope == Some(IssueAutomationPolicyScope::Project) {
            anyhow::bail!("private-token-value private-path project parse details");
        }
        Ok(self.project.map(str::to_string))
    }
}

#[test]
fn service_uses_only_its_source_and_keeps_io_errors_out_of_policy_blocked() {
    for scope in [
        IssueAutomationPolicyScope::User,
        IssueAutomationPolicyScope::Project,
    ] {
        let source = Arc::new(FakeSource {
            user: None,
            project: None,
            failed_scope: Some(scope),
            projects_read: Mutex::new(Vec::new()),
        });
        let service = IssueAutomationPolicyService::new(source.clone());
        let error = service.resolve("workspace-input").expect_err("unreadable");
        assert_eq!(error.scope, scope);
        assert_eq!(error.reason, "source_unavailable");
        assert!(!format!("{error} {error:?}").contains("private-"));
        let calls = source.projects_read.lock().expect("read calls");
        if scope == IssueAutomationPolicyScope::User {
            assert!(calls.is_empty());
        } else {
            assert_eq!(*calls, ["workspace-input"]);
        }
    }
    let source = Arc::new(FakeSource {
        user: Some("[binding.github]\nmode='auto'"),
        project: Some("[authority]\nautonomy_cap='observe'"),
        failed_scope: None,
        projects_read: Mutex::new(Vec::new()),
    });
    let resolved = IssueAutomationPolicyService::new(source.clone())
        .resolve("one/workspace")
        .expect("source policy");
    assert_eq!(
        resolved.policy.binding.github.mode,
        IssueAutomationAutonomy::Observe
    );
    assert_eq!(
        *source.projects_read.lock().expect("read calls"),
        ["one/workspace"]
    );
}

#[test]
fn execution_label_names_merge_per_leaf_and_supply_only_an_implicit_selection() {
    let defaults = resolve_issue_automation_policy(None, None).unwrap();
    assert_eq!(defaults.policy.labels.auto, "akra:auto");
    assert_eq!(defaults.policy.labels.gated, "akra:gated");
    assert_eq!(defaults.policy.labels.manual, "akra:manual");
    assert_eq!(defaults.policy.labels.hold, "akra:hold");
    let resolved = resolve_issue_automation_policy(
        Some("[labels]\nauto='user:auto'\ngated='user:gated'\nhold='user:hold'"),
        Some("[labels]\ngated='project:gated'\nmanual='project:manual'"),
    )
    .unwrap();
    assert_eq!(resolved.policy.labels.auto, "user:auto");
    assert_eq!(resolved.policy.labels.gated, "project:gated");
    assert_eq!(resolved.policy.labels.manual, "project:manual");
    assert_eq!(resolved.policy.labels.hold, "user:hold");
    assert_eq!(
        resolved.policy.select.labels_any,
        ["project:gated", "project:manual", "user:auto"]
    );
    assert!(
        !resolved
            .policy
            .select
            .labels_any
            .contains(&resolved.policy.labels.hold)
    );
}

#[test]
fn explicit_empty_and_older_selection_names_survive_execution_label_renaming() {
    for (declared, expected) in [
        ("[]", Vec::<String>::new()),
        ("['akra:auto']", vec!["akra:auto".to_owned()]),
        ("['selected']", vec!["selected".to_owned()]),
    ] {
        let user = format!("[select]\nlabels_any={declared}");
        let resolved = resolve_issue_automation_policy(
            Some(&user),
            Some("[labels]\nauto='renamed:auto'\ngated='renamed:gated'\nmanual='renamed:manual'"),
        )
        .unwrap();
        assert_eq!(resolved.policy.select.labels_any, expected);
    }
    let resolved = resolve_issue_automation_policy(
        Some("[select]\nlabels_any=['user:selection']"),
        Some("[select]\nlabels_any=[]\n[labels]\nauto='renamed:auto'"),
    )
    .unwrap();
    assert!(resolved.policy.select.labels_any.is_empty());
}

#[test]
fn execution_label_names_reject_invalid_or_duplicate_declarations_and_effects() {
    for text in [
        "[labels]\nauto=''",
        "[labels]\nauto=' leading'",
        "[labels]\nmanual='trailing '",
        "[labels]\nhold=\"bad\\u007fvalue\"",
        "[labels]\nauto='same'\ngated='same'",
        "[labels]\nmanual='same'\nhold='same'",
        "[labels]\nautomatic='unknown'",
        "[labels]\nauto=true",
    ] {
        assert!(
            resolve_issue_automation_policy(Some(text), None).is_err(),
            "{text}"
        );
    }
    let conflict = resolve_issue_automation_policy(
        Some("[labels]\nauto='selected'"),
        Some("[labels]\nmanual='selected'"),
    )
    .unwrap_err();
    assert_eq!(conflict.scope, IssueAutomationPolicyScope::Effective);
    assert_eq!(conflict.reason, "duplicate_label_name");

    // 뒤의 정상 선언으로 앞의 파일 안에 있는 잘못된 중복을 덮어 숨기지 않는다.
    let conflict = resolve_issue_automation_policy(
        Some("[labels]\nauto='same'\ngated='same'"),
        Some("[labels]\ngated='different'"),
    )
    .unwrap_err();
    assert_eq!(conflict.scope, IssueAutomationPolicyScope::User);
    assert_eq!(conflict.reason, "duplicate_label_name");
}

#[test]
fn execution_label_names_are_part_of_effect_hash_without_tracking_explicitness() {
    let defaults = resolve_issue_automation_policy(None, None).unwrap();
    let explicit = resolve_issue_automation_policy(
        Some("[labels]\nauto='akra:auto'\ngated='akra:gated'\nmanual='akra:manual'\nhold='akra:hold'"),
        None,
    ).unwrap();
    assert_eq!(defaults, explicit);
    let first = resolve_issue_automation_policy(Some("[select]\nlabels_any=[]"), None).unwrap();
    let renamed = resolve_issue_automation_policy(
        Some("[select]\nlabels_any=[]\n[labels]\nhold='custom:hold'"),
        None,
    )
    .unwrap();
    assert_eq!(
        first.policy.select.labels_any,
        renamed.policy.select.labels_any
    );
    assert_ne!(first.version, renamed.version);
}
