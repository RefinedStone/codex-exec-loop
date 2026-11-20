use super::*;
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::domain::issue_automation::{
    IssueAutomationBinding, IssueAutomationBindingId, IssueAutomationSnapshot, TrackedIssue,
};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
    IssueSharedCopy,
};

#[test]
fn offline_issue_detail_reads_existing_missing_and_rejects_foreign_scope_without_work() {
    let (_directory, kernel) = fixture();
    let planning_before = planning_revision(&kernel);
    let store = SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    });
    let reference = IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_offline_triage".into(),
        issue_id: "I_offline_triage".into(),
        project_scope: IssueProjectScope {
            workspace_dir: kernel.workspace.clone(),
        },
    };
    let binding = IssueAutomationBinding::new(
        IssueAutomationBindingId::new("offline-triage").unwrap(),
        reference.provider,
        &reference.server_id,
        &reference.repository_id,
        reference.project_scope.clone(),
    )
    .unwrap();
    store.insert_binding(&kernel.workspace, &binding).unwrap();
    let tracked = TrackedIssue::discovered(
        reference.clone(),
        &binding,
        IssueAutomationSnapshot {
            basis: IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields("legacy title", "legacy body"),
                provider_updated_at: "2026-11-20T00:00:00Z".into(),
            },
            shared_copy: IssueSharedCopy {
                title: "legacy title".into(),
                body: "legacy body".into(),
            },
            policy_version: "legacy-policy".into(),
        },
    )
    .unwrap();
    store.insert_issue(&kernel.workspace, &tracked).unwrap();

    let request = |issue: &IssueReference| {
        json!({
            "type": "issue",
            "issue": crate::domain::kernel_event::IssueEventReference::from(issue),
        })
    };
    // No Gateway or observer exists. A stored read remains available even while a TUI owns the kernel.
    let location =
        SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&kernel.workspace)
            .unwrap();
    let _tui = RepositoryKernelLockAdapter::acquire_for_owner(
        location,
        &kernel.workspace,
        RepositoryKernelOwner::Tui,
    )
    .unwrap();
    let found = kernel
        .query(request(&reference), false)
        .expect("stored issue detail");
    assert_eq!(found["status"], "ready");
    assert_eq!(found["item"]["issue"]["issue_id"], reference.issue_id);
    assert!(!found.to_string().contains("legacy body"));
    let list = kernel
        .query(json!({"type": "issues", "limit": 10}), false)
        .expect("stored issue list");
    assert_eq!(found["revision"], list["revision"]);
    assert_eq!(found["item"], list["items"][0]);

    let mut absent = reference.clone();
    absent.issue_id = "I_absent".into();
    let missing = kernel
        .query(request(&absent), false)
        .expect("missing detail");
    assert_eq!(missing["status"], "not_found");
    assert_eq!(missing["revision"], found["revision"]);

    let mut foreign = reference;
    foreign.project_scope.workspace_dir = PathBuf::from(&kernel.workspace)
        .join("foreign")
        .to_string_lossy()
        .into_owned();
    let error = kernel.query(request(&foreign), false).unwrap_err();
    assert!(
        error.to_string().contains("invalid issue reference"),
        "{error:#}"
    );
    assert_eq!(planning_revision(&kernel), planning_before);
    assert!(
        kernel
            .records(super::super::super::production::kernel_event_log_port().as_ref())
            .unwrap()
            .is_empty(),
        "stored detail queries do not create commands, turns, or audit events"
    );
}

#[test]
fn offline_issue_policy_query_reads_current_policy_without_creating_runtime_state() {
    use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
    use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;

    struct Policy(std::sync::Mutex<Option<String>>);
    impl IssueAutomationPolicySourcePort for Policy {
        fn load_user_policy_toml(&self) -> Result<Option<String>> {
            Ok(None)
        }
        fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    let root = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-offline-policy-query-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>(),
    ));
    crate::private_directory::ensure_private_directory(&root).unwrap();
    let _cleanup = Workspace(root.clone());
    let workspace = std::fs::canonicalize(&root).unwrap();
    let kernel = CommandOnlyKernel {
        workspace: workspace.to_str().unwrap().to_owned(),
        caller: CALLER_CLI,
    };
    let source = Arc::new(Policy(std::sync::Mutex::new(None)));
    let service = IssueAutomationPolicyService::new(source.clone());
    let query = || {
        kernel
            .query_with_policy_inspector(
                json!({ "type": "issue_automation_policy" }),
                false,
                |workspace| Some(service.inspect(workspace)),
            )
            .expect("local policy query")
    };
    let ready = query();
    assert_eq!(ready["type"], "issue_automation_policy");
    assert_eq!(ready["status"], "ready");
    assert_eq!(ready["workspace"], kernel.workspace);
    assert_eq!(ready["settings"].as_array().unwrap().len(), 65);

    *source.0.lock().unwrap() = Some("[authority]\nautonomy_cap = \"UNSHARED_QUERY_INPUT\"".into());
    let blocked = query();
    assert_eq!(blocked["status"], "policy_blocked");
    assert_eq!(blocked["workspace"], kernel.workspace);
    assert_eq!(blocked["blocked"]["scope"], "project");
    assert!(!blocked.to_string().contains("UNSHARED_QUERY_INPUT"));
    assert!(blocked["settings"].is_null());

    let unavailable = kernel
        .query_with_policy_inspector(json!({ "type": "issue_automation_policy" }), false, |_| {
            None
        })
        .unwrap();
    assert_eq!(unavailable["status"], "unavailable");
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        0,
        "read-only policy queries create no ledger, planning state, or audit log",
    );
}
