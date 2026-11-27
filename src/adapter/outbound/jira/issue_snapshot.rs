//! Jira Cloud 이슈 한 건의 안정 ID, 지금 본문, 필수 필드를 REST API v3 로 읽는다 (#2433). 읽기만 한다. 답을
//! 저장하지 않는다. 읽지 못하면 오류이고, 이전 답은 없다.
//!
//! 안정 ID 는 사이트의 cloud ID (`server_id`), 프로젝트 ID (`repository_id`), 이슈 ID (`issue_id`) 이다. 사이트
//! 주소와 이슈 키는 바뀔 수 있다. 읽을 때마다 사이트의 cloud ID 를 다시 확인한다.
//!
//! 본문의 원시 값은 ADF 를 Markdown 으로 옮긴 글이다 (`adf`). 필드 해시는 이 글로 만든다. 옮기지 못한 노드는
//! 글에 `[unsupported: <type>]` 으로 남는다.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::Value;

use super::adf::adf_to_markdown;
use crate::application::port::outbound::issue_field_requirement_port::{
    IssueFieldRequirementPort, IssueFieldRequirementRequest, IssueFieldRequirementResponse,
};
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
    IssueSnapshotResponse,
};
use crate::domain::issue_template::IssueTemplateUnsupportedInput;
use crate::domain::issue_validation::{IssueFieldFormat, IssueFieldRequirement};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueProjectScope, IssueProvider, IssueReference,
};

const MAX_ID_CHARS: usize = 64;
const MAX_PROJECT_KEY_CHARS: usize = 32;
const MAX_UPDATE_TIME_CHARS: usize = 64;
const TENANT_INFO_PATH: &str = "/_edge/tenant_info";
const ISSUE_FIELDS: &str = "summary,description,updated,project,issuetype";

/// 사이트 주소 뒤의 경로를 받아 GET 응답의 본문을 준다. 로그인은 이 함수의 주인이 가진다. 시험은 가짜를 넣는다.
pub type JiraApiRunner = Box<dyn Fn(&str) -> Result<Vec<u8>> + Send + Sync>;

/// Jira Cloud 사이트 하나의 이슈를 읽는다.
pub struct JiraIssueSnapshotAdapter {
    site: String,
    run_jira_api: JiraApiRunner,
}

impl JiraIssueSnapshotAdapter {
    /// `site` 는 `run_jira_api` 가 묻는 사이트의 주소이다. `IssueLocator::owner` 와 비교할 때만 쓴다.
    pub fn new(site: String, run_jira_api: JiraApiRunner) -> Self {
        Self { site, run_jira_api }
    }

    pub(super) fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let body = (self.run_jira_api)(path)?;
        // serde 의 오류 글에는 답의 값이 들어갈 수 있다. 남기지 않는다.
        serde_json::from_slice(&body).map_err(|_| anyhow!("Jira did not return the expected data"))
    }

    /// Requiredness is a provider assertion. An omitted custom-field flag cannot authorize a write.
    pub(super) fn edit_metadata(&self, issue_id: &str) -> Result<EditMeta> {
        let meta: EditMeta = self.get(&format!("/rest/api/3/issue/{issue_id}/editmeta"))?;
        if meta
            .fields
            .iter()
            .any(|(key, field)| field.required_for(key).is_none())
        {
            bail!("Jira did not confirm which fields are required")
        }
        Ok(meta)
    }

    fn cloud_id(&self) -> Result<String> {
        let tenant: TenantInfo = self.get(TENANT_INFO_PATH)?;
        ensure_cloud_id(&tenant.cloud_id)?;
        Ok(tenant.cloud_id)
    }

    /// 아는 이슈의 서버가 이 사이트인지 본다.
    pub(super) fn ensure_this_site(&self, issue: &IssueReference) -> Result<()> {
        if issue.provider != IssueProvider::Jira {
            bail!("the issue is not on Jira")
        }
        ensure_cloud_id(&issue.server_id)?;
        ensure_numeric_id(&issue.repository_id)?;
        ensure_numeric_id(&issue.issue_id)?;
        if self.cloud_id()? != issue.server_id {
            bail!("the issue is on another Jira site")
        }
        Ok(())
    }

    fn issue(&self, id_or_key: &str) -> Result<RemoteIssue> {
        self.get(&format!(
            "/rest/api/3/issue/{id_or_key}?fields={ISSUE_FIELDS}"
        ))
    }

    /// 아는 이슈를 안정 ID 로 읽는다. 답이 다른 이슈나 다른 프로젝트이면 오류이다.
    pub(super) fn known_issue(&self, expected: &IssueReference) -> Result<RemoteIssue> {
        let issue = self.issue(&expected.issue_id)?;
        if issue.id != expected.issue_id || issue.fields.project.id != expected.repository_id {
            bail!("the target moved to another project or issue")
        }
        Ok(issue)
    }
}

impl IssueSnapshotPort for JiraIssueSnapshotAdapter {
    /// 주소의 `owner` 는 사이트 주소, `repository` 는 프로젝트 키, `number` 는 이슈 키의 번호이다.
    fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
        let locator = &request.locator;
        if locator.owner != self.site {
            bail!("the issue is on another Jira site")
        }
        ensure_project_key(&locator.repository)?;
        let key = format!("{}-{}", locator.repository, locator.number);
        let cloud_id = self.cloud_id()?;
        let issue = self.issue(&key)?;
        // 옮겨진 이슈는 새 키의 이슈로 답이 온다.
        if issue.key != key {
            bail!("the target moved to another project or issue")
        }
        snapshot(cloud_id, issue, request.project_scope)
    }

    fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
        let expected = request.issue;
        self.ensure_this_site(&expected)?;
        let issue = self.known_issue(&expected)?;
        snapshot(expected.server_id, issue, expected.project_scope)
    }
}

impl IssueFieldRequirementPort for JiraIssueSnapshotAdapter {
    /// 이슈의 편집 메타데이터를 읽는다. Jira 는 이슈의 프로젝트와 이슈 유형으로 이 답을 정한다.
    fn read_requirements(
        &self,
        request: IssueFieldRequirementRequest,
    ) -> Result<IssueFieldRequirementResponse> {
        self.ensure_this_site(&request.issue)?;
        self.known_issue(&request.issue)?;
        let meta = self.edit_metadata(&request.issue.issue_id)?;
        Ok(requirements(meta))
    }
}

fn snapshot(
    cloud_id: String,
    issue: RemoteIssue,
    project_scope: IssueProjectScope,
) -> Result<IssueSnapshotResponse> {
    ensure_numeric_id(&issue.id)?;
    ensure_numeric_id(&issue.fields.project.id)?;
    ensure_update_time(&issue.fields.updated)?;
    let body = adf_to_markdown(&issue.fields.description).markdown;
    let raw_fields = IssueRawFields::new(issue.fields.summary, body);
    Ok(IssueSnapshotResponse {
        issue: IssueReference {
            provider: IssueProvider::Jira,
            server_id: cloud_id,
            repository_id: issue.fields.project.id,
            issue_id: issue.id,
            project_scope,
        },
        comparison_basis: IssueComparisonBasis {
            field_hash: raw_fields.field_hash(),
            provider_updated_at: issue.fields.updated,
        },
        raw_fields,
    })
}

/// 필수 필드를 검증기의 요구로 옮긴다. 필드 키의 순서이다. 필수가 아닌 필드는 요구가 아니다.
fn requirements(meta: EditMeta) -> IssueFieldRequirementResponse {
    let mut response = IssueFieldRequirementResponse::default();
    for (key, field) in meta.fields {
        // `summary` 는 제목이다. 검증기가 항상 검사한다. `issuetype` 과 `project` 는 대상을 정하는 값이다.
        if field.required == Some(false)
            || ["summary", "issuetype", "project"].contains(&key.as_str())
        {
            continue;
        }
        let label = field.name.unwrap_or_else(|| key.clone());
        match field_format(&key, &field.schema, &field.allowed_values) {
            Ok(format) => response.required_fields.push(IssueFieldRequirement {
                label,
                required: true,
                format,
            }),
            Err(reason) => response
                .unsupported_required_fields
                .push(IssueTemplateUnsupportedInput { label, reason }),
        }
    }
    response
}

/// 지원하는 필드의 형식을 준다. 지원하지 않으면 이유를 준다.
pub(super) fn field_format(
    key: &str,
    schema: &FieldSchema,
    allowed_values: &[AllowedValue],
) -> Result<Option<IssueFieldFormat>, String> {
    if key == "description" {
        // 본문은 섹션이 아니다. 검증기에는 본문 전체가 비었는지 보는 규칙이 없다.
        return Err("the validator has no rule for a required body".to_string());
    }
    match schema.field_type.as_str() {
        "string" => {
            let multi_line = schema.system.as_deref() == Some("environment")
                || schema
                    .custom
                    .as_deref()
                    .is_some_and(|custom| custom.ends_with(":textarea"));
            Ok((!multi_line).then_some(IssueFieldFormat::SingleLine))
        }
        "option" | "priority" => {
            let options: Vec<String> = allowed_values
                .iter()
                .filter_map(|allowed| allowed.value.clone().or_else(|| allowed.name.clone()))
                .collect();
            if options.is_empty() {
                return Err("Jira did not return the allowed values".to_string());
            }
            Ok(Some(IssueFieldFormat::OneOf(options)))
        }
        other => Err(format!("Jira field type `{other}` is not supported")),
    }
}

fn ensure_cloud_id(id: &str) -> Result<()> {
    let is_valid = !id.is_empty()
        && id.len() <= MAX_ID_CHARS
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    if !is_valid {
        bail!("the Jira cloud ID is not valid")
    }
    Ok(())
}

/// 프로젝트 ID 와 이슈 ID 는 요청 경로와 직렬화되는 참조에 들어간다. 숫자만 받는다.
fn ensure_numeric_id(id: &str) -> Result<()> {
    let is_valid = !id.is_empty()
        && id.len() <= MAX_ID_CHARS
        && id.chars().all(|character| character.is_ascii_digit());
    if !is_valid {
        bail!("the Jira ID is not valid")
    }
    Ok(())
}

fn ensure_project_key(key: &str) -> Result<()> {
    let is_valid = key.starts_with(|character: char| character.is_ascii_uppercase())
        && key.len() <= MAX_PROJECT_KEY_CHARS
        && key.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        });
    if !is_valid {
        bail!("the Jira project key is not valid")
    }
    Ok(())
}

fn ensure_update_time(time: &str) -> Result<()> {
    let looks_like_a_time = !time.is_empty()
        && time.len() <= MAX_UPDATE_TIME_CHARS
        && time
            .chars()
            .all(|character| character.is_ascii_digit() || "TZ:.+-".contains(character));
    if !looks_like_a_time {
        bail!("Jira did not return an update time")
    }
    Ok(())
}

// 답의 타입은 원시 값을 가진다. `Debug` 를 구현하지 않는다.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TenantInfo {
    cloud_id: String,
}

#[derive(Deserialize)]
pub(super) struct RemoteIssue {
    id: String,
    key: String,
    pub(super) fields: RemoteIssueFields,
}

#[derive(Deserialize)]
pub(super) struct RemoteIssueFields {
    pub(super) summary: String,
    /// ADF 문서이다. 본문이 없으면 `null` 이다.
    #[serde(default)]
    pub(super) description: Value,
    updated: String,
    project: RemoteProject,
}

#[derive(Deserialize)]
struct RemoteProject {
    id: String,
}

#[derive(Deserialize)]
pub(super) struct EditMeta {
    pub(super) fields: BTreeMap<String, EditMetaField>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EditMetaField {
    required: Option<bool>,
    pub(super) name: Option<String>,
    pub(super) schema: FieldSchema,
    #[serde(default)]
    pub(super) allowed_values: Vec<AllowedValue>,
}

impl EditMetaField {
    /// Jira always requires the summary and target coordinates. Other fields need explicit metadata.
    pub(super) fn required_for(&self, key: &str) -> Option<bool> {
        if matches!(key, "summary" | "project" | "issuetype") {
            Some(true)
        } else {
            self.required
        }
    }
}

#[derive(Deserialize)]
pub(super) struct FieldSchema {
    #[serde(rename = "type")]
    field_type: String,
    system: Option<String>,
    custom: Option<String>,
}

/// 선택 필드의 값은 `value`, 우선순위 같은 시스템 필드의 값은 `name` 에 있다.
#[derive(Deserialize)]
pub(super) struct AllowedValue {
    value: Option<String>,
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::domain::issue_work::{IssueFieldHash, IssueLocator, IssueSharedCopy};

    const SITE: &str = "https://jira.example.invalid";
    const CLOUD_ID: &str = "11111111-2222-3333-4444-555555555555";
    const ISSUE: &str = include_str!("fixtures/issue.json");
    const EDIT_META: &str = include_str!("fixtures/editmeta.json");
    const BODY: &str = "Body **text**\n\n[unsupported: panel]";
    const ISSUE_PATH: &str =
        "/rest/api/3/issue/10002?fields=summary,description,updated,project,issuetype";

    type Calls = Arc<Mutex<Vec<String>>>;

    /// Jira 의 답을 순서대로 주는 가짜이다. 네트워크가 없다.
    fn adapter_with(responses: Vec<Result<String>>) -> (JiraIssueSnapshotAdapter, Calls) {
        let calls = Calls::default();
        let recorded = calls.clone();
        let responses = Mutex::new(VecDeque::from(responses));
        let adapter = JiraIssueSnapshotAdapter::new(
            SITE.to_string(),
            Box::new(move |path| {
                recorded.lock().unwrap().push(path.to_string());
                let response = responses.lock().unwrap().pop_front();
                response
                    .expect("the fake has no more answers")
                    .map(String::into_bytes)
            }),
        );
        (adapter, calls)
    }

    fn tenant(cloud_id: &str) -> Result<String> {
        Ok(format!(r#"{{"cloudId":"{cloud_id}"}}"#))
    }

    fn issue() -> Result<String> {
        Ok(ISSUE.to_string())
    }

    fn project_scope() -> IssueProjectScope {
        IssueProjectScope {
            workspace_dir: "/work/project".to_string(),
        }
    }

    fn resolve_request(site: &str, project: &str, number: u64) -> IssueSnapshotResolveRequest {
        IssueSnapshotResolveRequest {
            locator: IssueLocator {
                owner: site.to_string(),
                repository: project.to_string(),
                number,
            },
            project_scope: project_scope(),
        }
    }

    fn reference() -> IssueReference {
        IssueReference {
            provider: IssueProvider::Jira,
            server_id: CLOUD_ID.to_string(),
            repository_id: "10000".to_string(),
            issue_id: "10002".to_string(),
            project_scope: project_scope(),
        }
    }

    fn read_request() -> IssueSnapshotReadRequest {
        IssueSnapshotReadRequest { issue: reference() }
    }

    #[test]
    fn resolve_reads_stable_ids_and_the_converted_body() {
        let (adapter, calls) = adapter_with(vec![tenant(CLOUD_ID), issue()]);

        let snapshot = adapter.resolve(resolve_request(SITE, "DEMO", 7)).unwrap();

        assert_eq!(snapshot.issue, reference());
        // 제목은 그대로이다. 본문은 ADF 를 옮긴 글이고, 옮기지 못한 노드가 글에 남는다.
        assert_eq!(snapshot.raw_fields.expose_title(), "Raw summary ");
        assert_eq!(snapshot.raw_fields.expose_body(), BODY);
        assert_eq!(
            snapshot.comparison_basis,
            IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields("Raw summary ", BODY),
                provider_updated_at: "2026-11-19T07:19:56.000+0000".to_string(),
            }
        );
        assert_eq!(
            *calls.lock().unwrap(),
            [
                "/_edge/tenant_info",
                "/rest/api/3/issue/DEMO-7?fields=summary,description,updated,project,issuetype"
            ]
        );
    }

    #[test]
    fn read_asks_by_stable_id_and_returns_the_same_snapshot() {
        let (adapter, calls) =
            adapter_with(vec![tenant(CLOUD_ID), issue(), tenant(CLOUD_ID), issue()]);

        let resolved = adapter.resolve(resolve_request(SITE, "DEMO", 7)).unwrap();
        let read = adapter.read(read_request()).unwrap();

        assert_eq!(read, resolved);
        assert_eq!(
            calls.lock().unwrap()[2..],
            ["/_edge/tenant_info", ISSUE_PATH]
        );
    }

    #[test]
    fn only_the_approved_copy_leaves_the_trusted_path() {
        let (adapter, _) = adapter_with(vec![tenant(CLOUD_ID), issue()]);
        let snapshot = adapter.read(read_request()).unwrap();

        assert_eq!(
            snapshot.verified_shared_copy(&IssueFieldHash::of_fields("Raw summary ", BODY)),
            Some(IssueSharedCopy {
                title: "Raw summary ".to_string(),
                body: BODY.to_string(),
            })
        );
        // 다른 사람이 고친 글은 공유본이 아니다. `Debug` 는 값을 가린다.
        assert_eq!(
            snapshot.verified_shared_copy(&IssueFieldHash::of_fields("Raw summary ", "Body")),
            None
        );
        assert!(!format!("{snapshot:?}").contains("Raw summary"));
    }

    #[test]
    fn a_missing_description_is_an_empty_body() {
        let without_description: Value = {
            let mut issue: Value = serde_json::from_str(ISSUE).unwrap();
            issue["fields"]["description"] = Value::Null;
            issue
        };
        let (adapter, _) =
            adapter_with(vec![tenant(CLOUD_ID), Ok(without_description.to_string())]);

        let snapshot = adapter.read(read_request()).unwrap();

        assert_eq!(snapshot.raw_fields.expose_body(), "");
    }

    #[test]
    fn a_failed_remote_read_is_an_error_and_never_an_earlier_snapshot() {
        let (adapter, calls) = adapter_with(vec![
            tenant(CLOUD_ID),
            issue(),
            tenant(CLOUD_ID),
            Err(anyhow!("Jira could not read the issue")),
            tenant(CLOUD_ID),
            Ok("not json".to_string()),
            Err(anyhow!("Jira could not read the site")),
        ]);

        assert!(adapter.read(read_request()).is_ok());
        for _ in 0..3 {
            assert!(adapter.read(read_request()).is_err());
        }
        // 실패마다 원격을 다시 불렀다. 저장한 답이 없다.
        assert_eq!(calls.lock().unwrap().len(), 7);
    }

    #[test]
    fn a_target_on_another_site_project_or_issue_is_rejected() {
        let other_project = ISSUE.replace(r#""id": "10000""#, r#""id": "10009""#);
        let other_issue = ISSUE.replace(r#""id": "10002""#, r#""id": "10003""#);
        let other_key = ISSUE.replace("DEMO-7", "OTHER-3");
        let (adapter, calls) = adapter_with(vec![
            tenant(CLOUD_ID),
            Ok(other_project),
            tenant(CLOUD_ID),
            Ok(other_issue),
            // 옮겨진 이슈: 물은 키와 답의 키가 다르다.
            tenant(CLOUD_ID),
            Ok(other_key),
            // 사이트 주소가 다른 cloud ID 를 가리킨다. 이슈를 묻지 않는다.
            tenant("99999999-2222-3333-4444-555555555555"),
        ]);

        for result in [
            adapter.read(read_request()),
            adapter.read(read_request()),
            adapter.resolve(resolve_request(SITE, "DEMO", 7)),
        ] {
            assert_eq!(
                result.unwrap_err().to_string(),
                "the target moved to another project or issue"
            );
        }
        assert_eq!(
            adapter.read(read_request()).unwrap_err().to_string(),
            "the issue is on another Jira site"
        );
        assert_eq!(calls.lock().unwrap().len(), 7);
    }

    #[test]
    fn invalid_targets_never_reach_jira() {
        let (adapter, calls) = adapter_with(Vec::new());

        for (site, project) in [
            ("https://other.example.invalid", "DEMO"),
            (SITE, "demo"),
            (SITE, "DEMO/../x"),
            (SITE, ""),
        ] {
            assert!(adapter.resolve(resolve_request(site, project, 7)).is_err());
        }
        for issue in [
            IssueReference {
                provider: IssueProvider::GitHub,
                ..reference()
            },
            IssueReference {
                server_id: "cloud/../id".to_string(),
                ..reference()
            },
            IssueReference {
                repository_id: "DEMO".to_string(),
                ..reference()
            },
            IssueReference {
                issue_id: "10002/editmeta?x=".to_string(),
                ..reference()
            },
        ] {
            assert!(
                adapter
                    .read(IssueSnapshotReadRequest {
                        issue: issue.clone()
                    })
                    .is_err()
            );
            assert!(
                adapter
                    .read_requirements(IssueFieldRequirementRequest { issue })
                    .is_err()
            );
        }
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn provider_text_outside_the_raw_fields_is_validated() {
        let bad_time = ISSUE.replace("2026-11-19T07:19:56.000+0000", "a secret, not a time");
        let bad_id = ISSUE.replace(r#""id": "10002""#, r#""id": "10002 secret""#);
        let (adapter, _) = adapter_with(vec![
            tenant(CLOUD_ID),
            Ok(bad_time),
            tenant(CLOUD_ID),
            Ok(bad_id),
            tenant("not a cloud id"),
        ]);

        assert!(adapter.read(read_request()).is_err());
        assert!(adapter.resolve(resolve_request(SITE, "DEMO", 7)).is_err());
        assert!(adapter.resolve(resolve_request(SITE, "DEMO", 7)).is_err());
    }

    #[test]
    fn required_fields_become_validator_requirements_and_unsupported_ones_are_marked() {
        let (adapter, calls) =
            adapter_with(vec![tenant(CLOUD_ID), issue(), Ok(EDIT_META.to_string())]);

        let response = adapter
            .read_requirements(IssueFieldRequirementRequest { issue: reference() })
            .unwrap();

        let requirement = |label: &str, format| IssueFieldRequirement {
            label: label.to_string(),
            required: true,
            format,
        };
        let options =
            |values: [&str; 2]| Some(IssueFieldFormat::OneOf(values.map(str::to_string).to_vec()));
        assert_eq!(
            response.required_fields,
            [
                requirement("Customer impact", options(["Major", "Minor"])),
                requirement("Build", Some(IssueFieldFormat::SingleLine)),
                requirement("Reproduction", None),
                requirement("Priority", options(["High", "Low"])),
            ]
        );
        let unsupported = |label: &str, reason: &str| IssueTemplateUnsupportedInput {
            label: label.to_string(),
            reason: reason.to_string(),
        };
        assert_eq!(
            response.unsupported_required_fields,
            [
                unsupported("Components", "Jira field type `array` is not supported"),
                unsupported("Due", "Jira field type `date` is not supported"),
                unsupported("Empty choice", "Jira did not return the allowed values"),
                unsupported(
                    "Description",
                    "the validator has no rule for a required body"
                ),
            ]
        );
        assert_eq!(calls.lock().unwrap()[1], ISSUE_PATH);
        assert_eq!(calls.lock().unwrap()[2], "/rest/api/3/issue/10002/editmeta");
    }
    #[test]
    fn requirements_reject_a_moved_issue_before_reading_edit_metadata() {
        for (field, replacement) in [("project", "10009"), ("issue", "10003")] {
            let mut moved: Value = serde_json::from_str(ISSUE).unwrap();
            if field == "project" {
                moved["fields"]["project"]["id"] = Value::String(replacement.to_string());
            } else {
                moved["id"] = Value::String(replacement.to_string());
            }
            let (adapter, calls) = adapter_with(vec![tenant(CLOUD_ID), Ok(moved.to_string())]);
            let error = adapter
                .read_requirements(IssueFieldRequirementRequest { issue: reference() })
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "the target moved to another project or issue"
            );
            assert_eq!(*calls.lock().unwrap(), ["/_edge/tenant_info", ISSUE_PATH]);
        }
    }
}
