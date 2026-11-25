use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::service::issue_automation_policy::{
    IssueAutomationPolicyService, resolve_issue_automation_policy,
};
use crate::domain::issue_automation::policy::IssueAutomationTrustedActor;

#[derive(Default)]
struct TextSource {
    user: Option<String>,
    project: Option<String>,
    user_reads: AtomicUsize,
    project_reads: AtomicUsize,
}

impl IssueAutomationPolicySourcePort for TextSource {
    fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
        self.user_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.user.clone())
    }

    fn load_project_policy_toml(&self, _: &str) -> anyhow::Result<Option<String>> {
        self.project_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.project.clone())
    }
}

fn inspect_text(user: Option<&str>, project: Option<&str>) -> IssueAutomationPolicyInspection {
    IssueAutomationPolicyService::new(Arc::new(TextSource {
        user: user.map(str::to_owned),
        project: project.map(str::to_owned),
        ..TextSource::default()
    }))
    .inspect("the-canonical-workspace")
}

fn ready(
    inspection: IssueAutomationPolicyInspection,
) -> (String, Vec<IssueAutomationPolicySetting>) {
    match inspection {
        IssueAutomationPolicyInspection::Ready {
            workspace,
            version,
            settings,
            ..
        } => {
            assert_eq!(workspace, "the-canonical-workspace");
            (version, settings)
        }
        other => panic!("expected a ready inspection, got {other:?}"),
    }
}

fn setting<'a>(
    settings: &'a [IssueAutomationPolicySetting],
    key: &str,
) -> &'a IssueAutomationPolicySetting {
    settings
        .iter()
        .find(|setting| setting.key == key)
        .expect("known setting")
}

fn scopes(setting: &IssueAutomationPolicySetting) -> Vec<IssueAutomationPolicyOriginScope> {
    setting.origins.iter().map(|origin| origin.scope).collect()
}

#[test]
fn inspection_preserves_resolve_hash_and_never_displays_comments_or_missing_paths() {
    let source = Arc::new(TextSource {
        user: Some("# synthetic-private-comment\n[select]\nmode='all_open'".to_owned()),
        ..TextSource::default()
    });
    let inspection =
        IssueAutomationPolicyService::new(source.clone()).inspect("the-canonical-workspace");
    assert!(!format!("{inspection:?}").contains("synthetic-private-comment"));
    let IssueAutomationPolicyInspection::Ready {
        version,
        settings,
        files,
        ..
    } = inspection
    else {
        panic!("valid policy should resolve")
    };
    assert_eq!(
        version,
        resolve_issue_automation_policy(source.user.as_deref(), None)
            .unwrap()
            .version
    );
    assert_eq!(source.user_reads.load(Ordering::SeqCst), 1);
    assert_eq!(source.project_reads.load(Ordering::SeqCst), 1);
    assert_eq!(settings.len(), 74);
    assert_eq!(
        settings
            .iter()
            .filter(|setting| setting.key.starts_with("gates."))
            .count(),
        45
    );
    assert!(settings.iter().all(|setting| {
        !setting.label.is_empty()
            && !setting.origins.is_empty()
            && !setting.resolution_summary.is_empty()
    }));
    let keys: std::collections::BTreeSet<_> = settings.iter().map(|setting| &setting.key).collect();
    assert_eq!(keys.len(), settings.len());
    assert!(
        files
            .iter()
            .all(|file| file.path.is_none() && file.open_target.is_none())
    );
    assert_eq!(files[0].status, IssueAutomationPolicyFileStatus::Readable);
    assert_eq!(files[1].status, IssueAutomationPolicyFileStatus::Missing);
}

#[test]
fn an_explicit_equal_value_keeps_its_declaring_scope_without_changing_the_effect_hash() {
    use IssueAutomationPolicyOriginScope::{Builtin, Project, User};
    let (default_version, defaults) = ready(inspect_text(None, None));
    let (user_version, user) = ready(inspect_text(Some("[timeouts]\nstart='1d'"), None));
    let (project_version, project) = ready(inspect_text(
        Some("[timeouts]\nstart='86400s'"),
        Some("[timeouts]\nstart='1440m'"),
    ));
    assert_eq!(default_version, user_version);
    assert_eq!(default_version, project_version);
    assert_eq!(
        setting(&defaults, "timeouts.start").value,
        setting(&project, "timeouts.start").value
    );
    assert_eq!(scopes(setting(&defaults, "timeouts.start")), [Builtin]);
    assert_eq!(scopes(setting(&user, "timeouts.start")), [User]);
    assert_eq!(scopes(setting(&project, "timeouts.start")), [Project]);
    assert_eq!(
        setting(&project, "timeouts.start").resolution,
        IssueAutomationPolicyResolution::Override
    );
}

#[test]
fn gate_origins_are_tracked_per_leaf_instead_of_per_table() {
    use IssueAutomationPolicyOriginScope::{Builtin, Project, User};
    let (_, settings) = ready(inspect_text(
        Some("[gates.gated.normal]\nstart='human'\nplan='human'"),
        Some("[gates.gated.normal]\nstart='human'"),
    ));
    assert_eq!(
        scopes(setting(&settings, "gates.gated.normal.start")),
        [Project]
    );
    assert_eq!(
        scopes(setting(&settings, "gates.gated.normal.plan")),
        [User]
    );
    assert_eq!(
        scopes(setting(&settings, "gates.gated.normal.spec")),
        [Builtin]
    );
    assert_eq!(
        scopes(setting(&settings, "gates.gated.low.start")),
        [Builtin]
    );
}

#[test]
fn caps_minimum_and_and_intersection_keep_both_declared_inputs() {
    use IssueAutomationPolicyOriginScope::{Project, User};
    use IssueAutomationPolicyResolution::{And, Cap, Intersection, Minimum};
    let user = r#"
        [binding.github]
        mode = 'auto'
        [authority]
        autonomy_cap = 'auto'
        [budget]
        max_running = 8
        max_started_per_day = 10
        max_attempts = 4
        [feedback]
        comment = true
        labels = true
        transition = true
        [select]
        trusted_actors = [
            {provider='github',server_id='github.com',actor_id='U_same'},
            {provider='github',server_id='enterprise.example',actor_id='U_same'},
            {provider='github',server_id='github.com',actor_id='U_other'},
        ]
    "#;
    let project = r#"
        [authority]
        autonomy_cap = 'gated'
        [budget]
        max_running = 3
        max_started_per_day = 99
        max_attempts = 4
        [feedback]
        comment = false
        labels = true
        transition = false
        [select]
        trusted_actors = [
            {provider='github',server_id='github.com',actor_id='U_same'},
            {provider='github',server_id='github.com',actor_id='U_absent'},
        ]
    "#;
    let (_, settings) = ready(inspect_text(Some(user), Some(project)));
    for (key, value, resolution) in [
        ("authority.autonomy_cap", "\"gated\"", Minimum),
        ("binding.github.mode", "\"gated\"", Cap),
        ("budget.max_running", "3", Minimum),
        ("budget.max_started_per_day", "10", Minimum),
        ("budget.max_attempts", "4", Minimum),
        ("feedback.comment", "false", And),
        ("feedback.labels", "true", And),
        ("feedback.transition", "false", And),
    ] {
        let row = setting(&settings, key);
        assert_eq!(row.value, value, "{key}");
        assert_eq!(row.resolution, resolution, "{key}");
        assert_eq!(scopes(row), [User, Project], "{key}");
    }
    let actors = setting(&settings, "select.trusted_actors");
    assert_eq!(actors.resolution, Intersection);
    assert_eq!(scopes(actors), [User, Project]);
    let actors: Vec<IssueAutomationTrustedActor> = serde_json::from_str(&actors.value).unwrap();
    assert_eq!(actors.len(), 1);
    assert_eq!(actors[0].server_id, "github.com");
    assert_eq!(actors[0].actor_id, "U_same");
}

#[test]
fn arbiter_provenance_explains_maximum_confidence_and_minimum_call_limits() {
    let user =
        "[arbiter]\nmin_confidence=9200\nmax_running=2\nmax_started_per_day=8\ntimeout_seconds=150";
    let project =
        "[arbiter]\nmin_confidence=9500\nmax_running=3\nmax_started_per_day=4\ntimeout_seconds=60";
    let (_, settings) = ready(inspect_text(Some(user), Some(project)));
    for (key, value, resolution) in [
        (
            "arbiter.min_confidence",
            "9500",
            IssueAutomationPolicyResolution::Maximum,
        ),
        (
            "arbiter.max_running",
            "2",
            IssueAutomationPolicyResolution::Minimum,
        ),
        (
            "arbiter.max_started_per_day",
            "4",
            IssueAutomationPolicyResolution::Minimum,
        ),
        (
            "arbiter.timeout_seconds",
            "60",
            IssueAutomationPolicyResolution::Minimum,
        ),
    ] {
        let row = setting(&settings, key);
        assert_eq!(row.value, value);
        assert_eq!(row.resolution, resolution);
        assert_eq!(
            row.origins
                .iter()
                .map(|origin| origin.scope)
                .collect::<Vec<_>>(),
            vec![
                IssueAutomationPolicyOriginScope::User,
                IssueAutomationPolicyOriginScope::Project
            ]
        );
    }
}

#[test]
fn an_explicit_empty_trust_set_differs_from_an_omitted_set_even_when_the_hash_is_equal() {
    use IssueAutomationPolicyOriginScope::{Builtin, Project, User};
    let (baseline, defaults) = ready(inspect_text(None, None));
    let (user_hash, user) = ready(inspect_text(Some("[select]\ntrusted_actors=[]"), None));
    let (project_hash, project) = ready(inspect_text(None, Some("[select]\ntrusted_actors=[]")));
    assert_eq!(baseline, user_hash);
    assert_eq!(baseline, project_hash);
    assert_eq!(setting(&defaults, "select.trusted_actors").value, "[]");
    assert_eq!(
        scopes(setting(&defaults, "select.trusted_actors")),
        [Builtin]
    );
    assert_eq!(scopes(setting(&user, "select.trusted_actors")), [User]);
    assert_eq!(
        scopes(setting(&project, "select.trusted_actors")),
        [Builtin, Project]
    );
    assert_eq!(
        setting(&project, "select.trusted_actors").resolution,
        IssueAutomationPolicyResolution::Intersection
    );
}

#[test]
fn union_keeps_builtin_and_both_file_origins_after_normalizing_duplicates() {
    use IssueAutomationPolicyOriginScope::{Builtin, Project, User};
    let user = r#"
        [protection]
        paths = ['src/private/**', 'src/shared/**']
        [human_required]
        rules = []
        additional = [{labels_any=['local-review'],gates=['start']}]
    "#;
    let project = r#"
        [protection]
        paths = ['src/shared/**', 'src/project/**']
        [human_required]
        rules = ['high_risk']
        additional = [{labels_any=['local-review'],gates=['start']}]
    "#;
    let (_, settings) = ready(inspect_text(Some(user), Some(project)));
    for key in [
        "protection.paths",
        "human_required.rules",
        "human_required.additional",
    ] {
        let row = setting(&settings, key);
        assert_eq!(row.resolution, IssueAutomationPolicyResolution::Union);
        assert_eq!(scopes(row), [Builtin, User, Project]);
    }
    let paths: Vec<String> =
        serde_json::from_str(&setting(&settings, "protection.paths").value).unwrap();
    assert!(
        paths
            .iter()
            .any(|path| path == ".akra/issue-automation.toml")
    );
    assert_eq!(
        paths.iter().filter(|path| *path == "src/shared/**").count(),
        1
    );
    let additional: Vec<
        crate::domain::issue_automation::policy::IssueAutomationAdditionalHumanRule,
    > = serde_json::from_str(&setting(&settings, "human_required.additional").value).unwrap();
    assert_eq!(additional.len(), 1);
}

#[test]
fn different_capped_requests_have_one_effect_hash_and_distinct_provenance() {
    use IssueAutomationPolicyOriginScope::{Project, User};
    let user = "[binding.github]\nmode='auto'\n[authority]\nautonomy_cap='propose'";
    let (first_hash, first) = ready(inspect_text(Some(user), None));
    let (second_hash, second) = ready(inspect_text(
        Some(user),
        Some("[binding.github]\nmode='gated'"),
    ));
    assert_eq!(first_hash, second_hash);
    assert_eq!(setting(&first, "binding.github.mode").value, "\"propose\"");
    assert_eq!(scopes(setting(&first, "binding.github.mode")), [User]);
    assert_eq!(
        scopes(setting(&second, "binding.github.mode")),
        [User, Project]
    );
}

struct SnapshotSource {
    user: Option<&'static str>,
    project: Option<&'static str>,
    fail_user: bool,
    user_path: Option<PathBuf>,
    project_path: Option<PathBuf>,
    workspaces: Mutex<Vec<String>>,
}

impl SnapshotSource {
    fn new() -> Self {
        let base = crate::test_utils::platform_safe_temp_dir().join("policy-inspection-location");
        Self {
            user: None,
            project: None,
            fail_user: false,
            user_path: Some(base.join("home/issue-automation.toml")),
            project_path: Some(base.join("workspace/.akra/issue-automation.toml")),
            workspaces: Mutex::new(Vec::new()),
        }
    }
}

impl IssueAutomationPolicySourcePort for SnapshotSource {
    fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
        panic!("inspect must consume one source snapshot");
    }

    fn load_project_policy_toml(&self, _: &str) -> anyhow::Result<Option<String>> {
        panic!("inspect must not reopen a policy for its location");
    }

    fn read_policy_snapshot(&self, workspace_dir: &str) -> IssueAutomationPolicySourceSnapshot {
        self.workspaces
            .lock()
            .unwrap()
            .push(workspace_dir.to_owned());
        IssueAutomationPolicySourceSnapshot {
            user: IssueAutomationPolicySourceDocument::from_read(
                self.user_path.clone(),
                if self.fail_user {
                    Err(anyhow::anyhow!(
                        "synthetic-private-error /not-the-policy-path"
                    ))
                } else {
                    Ok(self.user.map(str::to_owned))
                },
            ),
            project: IssueAutomationPolicySourceDocument::from_read(
                self.project_path.clone(),
                Ok(self.project.map(str::to_owned)),
            ),
        }
    }
}

#[test]
fn blocked_inspection_reports_static_errors_and_snapshot_locations_without_a_second_read() {
    let source = Arc::new(SnapshotSource {
        user: Some("synthetic_private_marker = 'secret-value'"),
        project: Some("schema_version=1"),
        ..SnapshotSource::new()
    });
    let inspection =
        IssueAutomationPolicyService::new(source.clone()).inspect("the-canonical-workspace");
    assert!(!format!("{inspection:?}").contains("synthetic_private_marker"));
    assert!(!format!("{inspection:?}").contains("secret-value"));
    let IssueAutomationPolicyInspection::PolicyBlocked {
        workspace,
        blocked,
        files,
    } = inspection
    else {
        panic!("an invalid user layer must not be hidden by the project")
    };
    assert_eq!(workspace, "the-canonical-workspace");
    assert_eq!(blocked.scope, IssueAutomationPolicyScope::User);
    assert_eq!(blocked.reason, "invalid_toml_or_shape");
    assert_eq!(blocked.file_scopes, [IssueAutomationPolicyFileScope::User]);
    assert!(!blocked.summary.is_empty() && !blocked.next_summary.is_empty());
    assert_eq!(
        files[0].path.as_deref(),
        source.user_path.as_ref().and_then(|path| path.to_str())
    );
    assert_eq!(
        files[1].path.as_deref(),
        source.project_path.as_ref().and_then(|path| path.to_str())
    );
    assert!(
        files
            .iter()
            .all(|file| file.status == IssueAutomationPolicyFileStatus::Readable)
    );
    assert_eq!(
        files[0].open_target,
        Some(IssueAutomationPolicyFileScope::User)
    );
    assert_eq!(
        *source.workspaces.lock().unwrap(),
        ["the-canonical-workspace"]
    );
}

#[test]
fn an_effective_cross_layer_conflict_names_both_files_and_never_returns_default_settings() {
    let source = SnapshotSource {
        user: Some("[timeouts]\nstart='1h'"),
        project: Some("[timeouts]\nremind_before='2h'"),
        ..SnapshotSource::new()
    };
    let inspection =
        IssueAutomationPolicyService::new(Arc::new(source)).inspect("the-canonical-workspace");
    let IssueAutomationPolicyInspection::PolicyBlocked { blocked, files, .. } = inspection else {
        panic!("a cross-layer conflict must block")
    };
    assert_eq!(blocked.scope, IssueAutomationPolicyScope::Effective);
    assert_eq!(blocked.reason, "must_precede_start");
    assert_eq!(
        blocked.file_scopes,
        [
            IssueAutomationPolicyFileScope::User,
            IssueAutomationPolicyFileScope::Project
        ]
    );
    assert!(files.iter().all(|file| file.path.is_some()));
}

#[test]
fn missing_and_unavailable_files_have_distinct_safe_open_targets() {
    let source = Arc::new(SnapshotSource {
        fail_user: true,
        ..SnapshotSource::new()
    });
    let inspection =
        IssueAutomationPolicyService::new(source.clone()).inspect("the-canonical-workspace");
    assert!(!format!("{inspection:?}").contains("synthetic-private-error"));
    assert!(!format!("{inspection:?}").contains("not-the-policy-path"));
    let IssueAutomationPolicyInspection::PolicyBlocked { blocked, files, .. } = inspection else {
        panic!("unavailable source must block")
    };
    assert_eq!(blocked.reason, "source_unavailable");
    assert_eq!(
        files[0].status,
        IssueAutomationPolicyFileStatus::Unavailable
    );
    assert_eq!(files[0].open_target, None);
    assert_eq!(
        files[0].path.as_deref(),
        source.user_path.as_ref().and_then(|path| path.to_str())
    );
    assert_eq!(files[1].status, IssueAutomationPolicyFileStatus::Missing);
    assert_eq!(
        files[1].open_target,
        Some(IssueAutomationPolicyFileScope::Project)
    );
}

#[test]
fn an_unknown_or_relative_source_location_is_not_invented_for_the_open_action() {
    let source = SnapshotSource {
        user_path: None,
        project_path: Some(PathBuf::from("relative/.akra/issue-automation.toml")),
        ..SnapshotSource::new()
    };
    let inspection =
        IssueAutomationPolicyService::new(Arc::new(source)).inspect("the-canonical-workspace");
    let IssueAutomationPolicyInspection::Ready { files, .. } = inspection else {
        panic!("location absence does not invent a source read failure")
    };
    assert!(
        files
            .iter()
            .all(|file| file.path.is_none() && file.open_target.is_none())
    );
}

#[test]
fn implicit_selection_records_the_final_label_inputs_and_their_origins() {
    use IssueAutomationPolicyOriginScope::{Builtin, Project, User};
    let (_, settings) = ready(inspect_text(
        Some("[labels]\nauto='user:auto'"),
        Some("[labels]\ngated='project:gated'\nhold='project:hold'"),
    ));
    assert_eq!(scopes(setting(&settings, "labels.auto")), [User]);
    assert_eq!(scopes(setting(&settings, "labels.gated")), [Project]);
    assert_eq!(scopes(setting(&settings, "labels.manual")), [Builtin]);
    let selected = setting(&settings, "select.labels_any");
    assert_eq!(
        selected.resolution,
        IssueAutomationPolicyResolution::Derived
    );
    assert_eq!(scopes(selected), [Builtin, User, Project]);
    let names: Vec<String> = serde_json::from_str(&selected.value).unwrap();
    assert_eq!(names, ["akra:manual", "project:gated", "user:auto"]);
    assert!(!selected.value.contains("project:hold"));
    let (_, hold_only) = ready(inspect_text(None, Some("[labels]\nhold='project:hold'")));
    assert_eq!(scopes(setting(&hold_only, "select.labels_any")), [Builtin]);
}

#[test]
fn explicit_selection_origins_do_not_claim_execution_label_inputs() {
    use IssueAutomationPolicyOriginScope::User;
    for selection in ["[]", "['akra:auto']"] {
        let user = format!("[select]\nlabels_any={selection}");
        let (_, settings) = ready(inspect_text(
            Some(&user),
            Some("[labels]\nauto='project:auto'"),
        ));
        let selected = setting(&settings, "select.labels_any");
        assert_eq!(
            selected.resolution,
            IssueAutomationPolicyResolution::Override
        );
        assert_eq!(scopes(selected), [User]);
    }
}

#[test]
fn equal_label_declarations_keep_origins_without_changing_the_effect_hash() {
    use IssueAutomationPolicyOriginScope::{Builtin, User};
    let (default_version, _) = ready(inspect_text(None, None));
    let (version, settings) = ready(inspect_text(Some("[labels]\nauto='akra:auto'"), None));
    assert_eq!(default_version, version);
    assert_eq!(scopes(setting(&settings, "labels.auto")), [User]);
    assert_eq!(
        scopes(setting(&settings, "select.labels_any")),
        [Builtin, User]
    );
    assert_eq!(
        setting(&settings, "select.labels_any").resolution,
        IssueAutomationPolicyResolution::Derived
    );
}
