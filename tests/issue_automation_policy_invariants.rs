//! 사용자/프로젝트 파일을 공개 resolver에 넣어 정책 병합의 경계 조합을 검증한다.
use codex_exec_loop_native::application::service::issue_automation_policy::{
    IssueAutomationPolicyBlocked, IssueAutomationPolicyScope, ResolvedIssueAutomationPolicy,
    resolve_issue_automation_policy,
};
use serde_json::{Value, json};

fn resolve_layers(
    user: Option<&str>,
    project: Option<&str>,
) -> Result<ResolvedIssueAutomationPolicy, IssueAutomationPolicyBlocked> {
    let user = user.map(|body| format!("schema_version = 1\n{body}"));
    let project = project.map(|body| format!("schema_version = 1\n{body}"));
    resolve_issue_automation_policy(user.as_deref(), project.as_deref())
}

fn effective(resolved: &ResolvedIssueAutomationPolicy) -> Value {
    serde_json::to_value(&resolved.policy).expect("typed policy is serializable")
}

#[test]
fn repository_can_only_restrict_user_authority_budget_and_write_grants() {
    let user = r#"
[binding.github]
mode = "auto"
[authority]
autonomy_cap = "gated"
[budget]
max_running = 1
max_started_per_day = 2
max_attempts = 1
[feedback]
comment = false
labels = true
transition = false
"#;
    let expand = r#"
[binding.github]
mode = "auto"
[authority]
autonomy_cap = "auto"
[budget]
max_running = 999
max_started_per_day = 999
max_attempts = 999
[feedback]
comment = true
labels = true
transition = true
"#;
    let restrict = r#"
[binding.github]
mode = "auto"
[authority]
autonomy_cap = "observe"
[budget]
max_running = 0
max_started_per_day = 0
max_attempts = 0
[feedback]
comment = false
labels = false
transition = false
"#;

    for (label, user, project, cap, budget, feedback) in [
        (
            "repository expansion is capped by explicit user grants",
            Some(user),
            expand,
            "gated",
            [1, 2, 1],
            [false, true, false],
        ),
        (
            "repository can reduce authority to observe and all budgets to zero",
            Some(user),
            restrict,
            "observe",
            [0, 0, 0],
            [false, false, false],
        ),
        (
            "repository cannot grant authority absent from the user file",
            None,
            expand,
            "propose",
            [2, 5, 3],
            [false, false, false],
        ),
    ] {
        let policy = effective(&resolve_layers(user, Some(project)).unwrap());
        assert_eq!(policy["authority"]["autonomy_cap"], json!(cap), "{label}");
        assert_eq!(policy["binding"]["github"]["mode"], json!(cap), "{label}");
        assert_eq!(
            policy["budget"],
            json!({
                "max_running": budget[0],
                "max_started_per_day": budget[1],
                "max_attempts": budget[2],
            }),
            "{label}"
        );
        assert_eq!(
            policy["feedback"],
            json!({
                "comment": feedback[0],
                "labels": feedback[1],
                "transition": feedback[2],
            }),
            "{label}"
        );
    }
}

#[test]
fn trusted_actor_restrictions_distinguish_absence_empty_and_all_identity_coordinates() {
    let user = r#"
[select]
trusted_actors = [
    { provider = "github", server_id = "github.com", actor_id = "U_a" },
    { provider = "github", server_id = "github.com", actor_id = "U_b" },
]
"#;
    let actor_a = json!({"provider":"github","server_id":"github.com","actor_id":"U_a"});
    let actor_b = json!({"provider":"github","server_id":"github.com","actor_id":"U_b"});
    let exact_with_ungranted_actor = r#"
[select]
trusted_actors = [
    { provider = "github", server_id = "github.com", actor_id = "U_a" },
    { provider = "github", server_id = "github.com", actor_id = "U_ungranted" },
]
"#;

    for (label, project, expected) in [
        (
            "missing layer",
            None,
            json!([actor_a.clone(), actor_b.clone()]),
        ),
        (
            "missing field in an otherwise present section",
            Some("[select]\nmode = \"all_open\"\n"),
            json!([actor_a.clone(), actor_b.clone()]),
        ),
        (
            "explicit empty restriction",
            Some("[select]\ntrusted_actors = []\n"),
            json!([]),
        ),
        (
            "only the exact granted actor survives",
            Some(exact_with_ungranted_actor),
            json!([actor_a]),
        ),
        (
            "provider differs",
            Some(
                "[select]\ntrusted_actors = [{provider=\"jira\",server_id=\"github.com\",actor_id=\"U_a\"}]\n",
            ),
            json!([]),
        ),
        (
            "server differs",
            Some(
                "[select]\ntrusted_actors = [{provider=\"github\",server_id=\"github.example\",actor_id=\"U_a\"}]\n",
            ),
            json!([]),
        ),
        (
            "stable actor id differs",
            Some(
                "[select]\ntrusted_actors = [{provider=\"github\",server_id=\"github.com\",actor_id=\"u_a\"}]\n",
            ),
            json!([]),
        ),
    ] {
        let policy = effective(&resolve_layers(Some(user), project).unwrap());
        assert_eq!(policy["select"]["trusted_actors"], expected, "{label}");
    }

    let project_only = effective(&resolve_layers(None, Some(exact_with_ungranted_actor)).unwrap());
    assert_eq!(project_only["select"]["trusted_actors"], json!([]));
}

#[test]
fn project_empty_sets_cannot_remove_builtin_or_user_protection_and_human_requirements() {
    let builtin = effective(&resolve_layers(None, None).unwrap());
    let user = r#"
[protection]
paths = ["src/private/**"]
[human_required]
rules = ["high_risk"]
additional = [{labels_any=["approval-required"],gates=["start"]}]
"#;
    let empty_restrictions = r#"
[protection]
paths = []
[human_required]
rules = []
additional = []
"#;
    let extra_restrictions = r#"
[protection]
paths = ["src/public/**", "src/private/**"]
[human_required]
rules = []
additional = [
    {labels_any=["approval-required"],gates=["start"]},
    {paths_any=["src/public/**"],gates=["merge"]},
]
"#;
    let user_rule = json!({
        "labels_any":["approval-required"],
        "paths_any":[],
        "gates":["start"],
    });
    let project_rule = json!({
        "labels_any":[],
        "paths_any":["src/public/**"],
        "gates":["merge"],
    });

    for (project, added_paths, added_rules) in [
        (
            empty_restrictions,
            vec!["src/private/**"],
            vec![user_rule.clone()],
        ),
        (
            extra_restrictions,
            vec!["src/private/**", "src/public/**"],
            vec![user_rule, project_rule],
        ),
    ] {
        let policy = effective(&resolve_layers(Some(user), Some(project)).unwrap());
        let paths = policy["protection"]["paths"].as_array().unwrap();
        let builtin_paths = builtin["protection"]["paths"].as_array().unwrap();
        assert!(builtin_paths.iter().all(|path| paths.contains(path)));
        assert!(added_paths.iter().all(|path| paths.contains(&json!(path))));
        assert_eq!(paths.len(), builtin_paths.len() + added_paths.len());
        assert_eq!(
            policy["human_required"]["rules"],
            builtin["human_required"]["rules"]
        );
        let additional = policy["human_required"]["additional"].as_array().unwrap();
        assert_eq!(additional.len(), added_rules.len());
        assert!(added_rules.iter().all(|rule| additional.contains(rule)));
    }
}

#[test]
fn invalid_values_in_either_layer_cannot_be_hidden_by_valid_replacements() {
    for (label, invalid, replacement) in [
        (
            "unknown key",
            "[authority]\nautonomy_cap = \"observe\"\nallow_everything = true\n",
            "[authority]\nautonomy_cap = \"observe\"\n",
        ),
        (
            "unknown enum",
            "[binding.github]\nmode = \"unbounded\"\n",
            "[binding.github]\nmode = \"observe\"\n",
        ),
        (
            "invalid budget",
            "[budget]\nmax_running = -1\n",
            "[budget]\nmax_running = 0\n",
        ),
        (
            "invalid duration",
            "[timeouts]\nstart = \"0s\"\n",
            "[timeouts]\nstart = \"24h\"\n",
        ),
        (
            "empty risk condition",
            "[[classify.rule]]\nclass = \"low\"\nlabels_any = []\npaths_any = []\n",
            "[classify]\nrule = []\n",
        ),
        (
            "invalid repository glob",
            "[protection]\npaths = [\"../outside/**\"]\n",
            "[protection]\npaths = []\n",
        ),
        (
            "contradictory declarations inside one layer",
            "[feedback]\ntransition = false\n[close]\nwhen = \"integration\"\n",
            "[feedback]\ntransition = true\n[close]\nwhen = \"never\"\n",
        ),
    ] {
        for (user, project, expected_scope) in [
            (
                Some(invalid),
                Some(replacement),
                IssueAutomationPolicyScope::User,
            ),
            (
                Some(replacement),
                Some(invalid),
                IssueAutomationPolicyScope::Project,
            ),
        ] {
            let error = resolve_layers(user, project).unwrap_err();
            assert_eq!(error.scope, expected_scope, "{label}");
        }
    }
}

#[test]
fn individually_valid_layers_must_also_pass_final_cross_field_validation() {
    for (label, user, project, field) in [
        (
            "transition restriction conflicts with inherited integration close",
            "[feedback]\ntransition = true\n[close]\nwhen = \"integration\"\n",
            "[feedback]\ntransition = false\n",
            "close.when",
        ),
        (
            "project reminder exceeds inherited shorter start",
            "[timeouts]\nstart = \"1h\"\nremind_before = \"30m\"\n",
            "[timeouts]\nremind_before = \"2h\"\n",
            "timeouts.remind_before",
        ),
        (
            "project start precedes inherited longer reminder",
            "[timeouts]\nremind_before = \"6h\"\n",
            "[timeouts]\nstart = \"4h\"\n",
            "timeouts.remind_before",
        ),
    ] {
        assert!(
            resolve_layers(Some(user), None).is_ok(),
            "{label}: user alone"
        );
        assert!(
            resolve_layers(None, Some(project)).is_ok(),
            "{label}: project alone"
        );
        let error = resolve_layers(Some(user), Some(project)).unwrap_err();
        assert_eq!(
            error.scope,
            IssueAutomationPolicyScope::Effective,
            "{label}"
        );
        assert_eq!(error.field, field, "{label}");
    }
}

#[test]
fn equivalent_layout_sets_duplicates_and_duration_units_share_the_effective_version() {
    let first = r#"
[binding.github]
repository = "sample/widgets"
mode = "auto"
[authority]
autonomy_cap = "auto"
[select]
mode = "labels_any"
labels_any = ["akra:auto", "security"]
trusted_actors = [
    {provider="github",server_id="github.com",actor_id="U_one"},
    {provider="jira",server_id="jira.example",actor_id="J_two"},
]
[timeouts]
start = "2d"
remind_before = "1h"
[budget]
max_running = 2
max_started_per_day = 5
max_attempts = 3
[feedback]
comment = true
labels = true
transition = true
[close]
when = "integration"
[protection]
paths = ["src/auth/**", "src/billing/**"]
[human_required]
rules = ["high_risk", "untrusted_actor"]
additional = [
    {labels_any=["security","privacy"],paths_any=["src/auth/**","src/billing/**"],gates=["start","merge"]},
    {paths_any=["migrations/**"],gates=["plan"]},
]
[[classify.rule]]
class = "high"
labels_any = ["security", "privacy"]
paths_any = ["src/auth/**", "src/billing/**"]
[[classify.rule]]
class = "low"
labels_any = ["docs"]
"#;
    let equivalent = r#"
# A different TOML layout expresses the same effective decision inputs.
[classify]
rule = [
    {paths_any=['src/billing/**','src/auth/**','src/auth/**'],labels_any=['privacy','security','privacy'],class='high'},
    {labels_any=['docs','docs'],class='low',paths_any=[]},
]
[human_required]
additional = [
    {gates=['plan'],paths_any=['migrations/**'],labels_any=[]},
    {gates=['merge','start','merge'],paths_any=['src/billing/**','src/auth/**'],labels_any=['privacy','security','privacy']},
    {paths_any=['migrations/**'],gates=['plan','plan']},
]
rules = ['untrusted_actor','high_risk','high_risk']
[protection]
paths = ['src/billing/**','src/auth/**','src/billing/**']
[close]
when = 'integration'
[feedback]
transition=true
labels=true
comment=true
[budget]
max_attempts=3
max_started_per_day=5
max_running=2
[timeouts]
remind_before = '60m'
start = '48h'
[select]
trusted_actors = [
    {actor_id='J_two',server_id='jira.example',provider='jira'},
    {server_id='github.com',provider='github',actor_id='U_one'},
    {actor_id='J_two',provider='jira',server_id='jira.example'},
]
labels_any = ['security','akra:auto','security']
mode = 'labels_any'
[authority]
autonomy_cap = 'auto'
[binding.github]
mode = 'auto'
repository = 'sample/widgets'
"#;
    let first = resolve_layers(Some(first), None).unwrap();
    let equivalent = resolve_layers(Some(equivalent), None).unwrap();

    assert_eq!(effective(&first), effective(&equivalent));
    assert_eq!(first.version, equivalent.version);
    assert_eq!(first.version.len(), 64);
    assert!(
        first
            .version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
}

#[test]
fn requests_clipped_by_the_same_grants_do_not_change_the_effective_version() {
    let expanded_request = r#"
[binding.github]
mode = "auto"
[authority]
autonomy_cap = "auto"
[budget]
max_running = 999
max_started_per_day = 999
max_attempts = 999
[feedback]
comment = true
labels = true
transition = true
[select]
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_ungranted"}]
"#;
    let equivalent_request = r#"
[binding.github]
mode = "gated"
[authority]
autonomy_cap = "gated"
[budget]
max_running = 1
max_started_per_day = 3
max_attempts = 2
[feedback]
comment = false
labels = false
transition = false
[select]
trusted_actors = []
"#;

    for cap in ["observe", "propose"] {
        let user = format!(
            "[binding.github]\nmode = \"auto\"\n[authority]\nautonomy_cap = \"{cap}\"\n\
             [budget]\nmax_running = 1\nmax_started_per_day = 3\nmax_attempts = 2\n"
        );
        let baseline = resolve_layers(Some(&user), None).unwrap();
        for project in [expanded_request, equivalent_request] {
            let clipped = resolve_layers(Some(&user), Some(project)).unwrap();
            assert_eq!(effective(&clipped), effective(&baseline), "{cap}");
            assert_eq!(clipped.version, baseline.version, "{cap}");
        }
    }
}

#[test]
fn overlapping_risk_rule_order_remains_part_of_the_effective_version() {
    let high_first = r#"
[[classify.rule]]
class = "high"
labels_any = ["shared-condition"]
[[classify.rule]]
class = "low"
labels_any = ["shared-condition"]
"#;
    let low_first = r#"
[[classify.rule]]
class = "low"
labels_any = ["shared-condition"]
[[classify.rule]]
class = "high"
labels_any = ["shared-condition"]
"#;
    let high_first = resolve_layers(Some(high_first), None).unwrap();
    let low_first = resolve_layers(Some(low_first), None).unwrap();

    assert_eq!(
        effective(&high_first)["classify"]["rule"][0]["class"],
        json!("high")
    );
    assert_eq!(
        effective(&low_first)["classify"]["rule"][0]["class"],
        json!("low")
    );
    assert_ne!(high_first.version, low_first.version);
}

#[test]
fn blocked_errors_expose_no_original_marker_or_parser_error_chain() {
    const MARKER: &str = "UNSHARED_POLICY_MARKER_2540";
    let invalid_inputs = [
        format!("[broken\nvalue = \"{MARKER}\"\n"),
        format!("{MARKER} = true\n"),
        format!("[binding.github]\nmode = \"{MARKER}\"\n"),
        format!("[binding.github]\nrepository = \"sample/{MARKER}/extra\"\n"),
        format!("[protection]\npaths = [\"../{MARKER}/**\"]\n"),
        format!(
            "[select]\ntrusted_actors = [{{provider=\"github\",server_id=\"github.com\",actor_id=\"U_a\",login=\"{MARKER}\"}}]\n"
        ),
        format!("[timeouts]\nstart = \"{MARKER}🔑\"\n"),
    ];

    for input in &invalid_inputs {
        for (user, project, scope) in [
            (Some(input.as_str()), None, IssueAutomationPolicyScope::User),
            (
                None,
                Some(input.as_str()),
                IssueAutomationPolicyScope::Project,
            ),
        ] {
            let error = resolve_layers(user, project).unwrap_err();
            assert_eq!(error.scope, scope);
            for surface in [
                error.to_string(),
                format!("{error:?}"),
                error.field.to_owned(),
                error.reason.to_owned(),
            ] {
                assert!(!surface.contains(MARKER));
            }
            assert!(std::error::Error::source(&error).is_none());
        }
    }
}
