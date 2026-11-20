use super::*;
use crate::adapter::inbound::kernel_protocol::{
    KernelQuery, KernelQueryAnswer, QueryRequest, describe,
};
use crate::application::port::outbound::issue_automation_policy_source_port::{
    IssueAutomationPolicySourceDocument, IssueAutomationPolicySourcePort,
    IssueAutomationPolicySourceSnapshot,
};
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

struct Source {
    user: Option<&'static str>,
    project: Option<&'static str>,
    unreadable: bool,
}

impl IssueAutomationPolicySourcePort for Source {
    fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
        if self.unreadable {
            anyhow::bail!("private-error-sentinel");
        }
        Ok(self.user.map(str::to_owned))
    }

    fn load_project_policy_toml(&self, _workspace_dir: &str) -> anyhow::Result<Option<String>> {
        Ok(self.project.map(str::to_owned))
    }

    fn read_policy_snapshot(&self, workspace_dir: &str) -> IssueAutomationPolicySourceSnapshot {
        IssueAutomationPolicySourceSnapshot {
            user: IssueAutomationPolicySourceDocument::from_read(
                Some(
                    std::env::temp_dir()
                        .join("akra-policy-wire")
                        .join("issue-automation.toml"),
                ),
                self.load_user_policy_toml(),
            ),
            project: IssueAutomationPolicySourceDocument::from_read(
                Some(
                    PathBuf::from(workspace_dir)
                        .join(".akra")
                        .join("issue-automation.toml"),
                ),
                self.load_project_policy_toml(workspace_dir),
            ),
        }
    }
}

fn wire(source: Source) -> Value {
    let workspace = std::env::temp_dir().join("akra-policy-workspace");
    let inspection =
        IssueAutomationPolicyService::new(Arc::new(source)).inspect(workspace.to_str().unwrap());
    serde_json::to_value(KernelQueryAnswer::IssueAutomationPolicy(
        IssueAutomationPolicyState::from_domain(&inspection),
    ))
    .unwrap()
}

fn assert_schema(value: &Value) {
    let spec = describe();
    let mut schema = spec
        .queries
        .iter()
        .find(|query| query.name == "issue_automation_policy")
        .unwrap()
        .output
        .clone();
    schema
        .as_object_mut()
        .unwrap()
        .insert("$defs".to_owned(), Value::Object(spec.definitions));
    let validator = jsonschema::draft202012::options()
        .offline()
        .build(&schema)
        .unwrap();
    let errors: Vec<_> = validator
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    serde_json::from_value::<KernelQueryAnswer>(value.clone()).unwrap();
}

#[test]
fn policy_query_is_read_only_and_rejects_client_supplied_policy_or_paths() {
    let request = json!({"query":{"type":"issue_automation_policy"}});
    let parsed: QueryRequest = serde_json::from_value(request.clone()).unwrap();
    assert_eq!(parsed.query, KernelQuery::IssueAutomationPolicy {});
    assert_eq!(serde_json::to_value(parsed).unwrap(), request);
    for field in ["workspace", "path", "content", "mode", "scope"] {
        let mut invalid = request.clone();
        invalid["query"][field] = json!("untrusted");
        assert!(
            serde_json::from_value::<QueryRequest>(invalid).is_err(),
            "{field}"
        );
    }
    let unavailable = serde_json::to_value(KernelQueryAnswer::IssueAutomationPolicy(
        IssueAutomationPolicyState::unavailable(),
    ))
    .unwrap();
    assert_eq!(
        unavailable,
        json!({"type":"issue_automation_policy","status":"unavailable"})
    );
    assert_schema(&unavailable);
}

#[test]
fn policy_wire_has_effective_values_origins_and_same_snapshot_file_locations() {
    let value = wire(Source {
        user: Some("# private-comment-sentinel\n"),
        project: None,
        unreadable: false,
    });
    assert_schema(&value);
    assert_eq!(value["status"], "ready");
    assert_eq!(
        value["workspace"],
        std::env::temp_dir()
            .join("akra-policy-workspace")
            .to_str()
            .unwrap()
    );
    assert_eq!(value["version"].as_str().unwrap().len(), 64);
    assert!(!value["settings"].as_array().unwrap().is_empty());
    for setting in value["settings"].as_array().unwrap() {
        for key in ["key", "label", "value", "resolution", "resolution_summary"] {
            assert!(!setting[key].as_str().unwrap().trim().is_empty(), "{key}");
        }
        assert!(!setting["origins"].as_array().unwrap().is_empty());
    }
    assert_eq!(value["files"][0]["scope"], "user");
    assert_eq!(value["files"][0]["status"], "readable");
    assert_eq!(value["files"][0]["open_target"], "user");
    assert_eq!(value["files"][1]["scope"], "project");
    assert_eq!(value["files"][1]["status"], "missing");
    assert_eq!(value["files"][1]["open_target"], "project");
    assert!(!value.to_string().contains("private-comment-sentinel"));
    assert!(value.get("content").is_none());
}

#[test]
fn invalid_policy_exposes_problem_and_locations_without_fallback_values_or_error_text() {
    for (source, scope, status, open_target) in [
        (
            Source {
                user: None,
                project: Some("secret = ['private-source-sentinel"),
                unreadable: false,
            },
            "project",
            "readable",
            json!("project"),
        ),
        (
            Source {
                user: None,
                project: None,
                unreadable: true,
            },
            "user",
            "unavailable",
            Value::Null,
        ),
    ] {
        let value = wire(source);
        assert_schema(&value);
        assert_eq!(value["status"], "policy_blocked");
        assert_eq!(value["blocked"]["scope"], scope);
        assert!(!value["blocked"]["summary"].as_str().unwrap().is_empty());
        assert!(
            !value["blocked"]["next_summary"]
                .as_str()
                .unwrap()
                .is_empty()
        );
        let file = value["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["scope"] == scope)
            .unwrap();
        assert_eq!(file["status"], status);
        assert_eq!(file["open_target"], open_target);
        assert!(file["path"].as_str().is_some());
        assert!(value.get("settings").is_none());
        assert!(value.get("version").is_none());
        for secret in ["private-source-sentinel", "private-error-sentinel"] {
            assert!(!value.to_string().contains(secret));
        }
    }
}
