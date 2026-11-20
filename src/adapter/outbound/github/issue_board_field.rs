//! GitHub Projects (v2) 에서 이슈 한 건의 보드 항목과 단일 선택 필드를 `gh api graphql` 로 읽고, 항목 하나의
//! 필드 하나를 고친다 (#2435). 이슈 스냅샷(#2420)과 같은 `gh` 경로를 쓴다.
//!
//! 쓰기는 `updateProjectV2ItemFieldValue` 하나이다. 인자는 Project, 항목, 필드, 선택지의 ID 뿐이다. 이슈 ID,
//! 제목, 본문은 보내지 않는다. 쓰기 전에 승인을 보고, 대상이 이 이슈의 항목인지 원격에서 다시 읽어 확인한다.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use super::issue_snapshot::{
    GhApiRunner, GraphqlEnvelope, ensure_node_id, graphql_arguments, run_gh_api,
};
use super::resource_version::GITHUB_HOSTNAME;
use crate::application::port::outbound::issue_board_field_port::{
    IssueBoardField, IssueBoardFieldOption, IssueBoardFieldPort, IssueBoardFieldWriteRequest,
    IssueBoardFieldWriteResponse, IssueBoardItem, IssueBoardItemsReadRequest,
    IssueBoardItemsResponse,
};
use crate::domain::issue_work::{IssueProvider, IssueReference};

// shortcut: 한 쪽만 읽는다 (보드 20개, 필드 50개, 값 50개). 넘으면 오류이다. 실제 보드가 넘으면 쪽 넘김을 더한다.
const ITEMS_QUERY: &str = "query($id: ID!) { node(id: $id) { __typename ... on Issue { id repository { id } projectItems(first: 20) { pageInfo { hasNextPage } nodes { id project { id title fields(first: 50) { pageInfo { hasNextPage } nodes { __typename ... on ProjectV2SingleSelectField { id name options { id name } } } } } fieldValues(first: 50) { pageInfo { hasNextPage } nodes { __typename ... on ProjectV2ItemFieldSingleSelectValue { optionId field { ... on ProjectV2SingleSelectField { id } } } } } } } } } }";
const WRITE_MUTATION: &str = "mutation($projectId: ID!, $itemId: ID!, $fieldId: ID!, $optionId: String!) { updateProjectV2ItemFieldValue(input: { projectId: $projectId, itemId: $itemId, fieldId: $fieldId, value: { singleSelectOptionId: $optionId } }) { projectV2Item { id } } }";
const SINGLE_SELECT_FIELD: &str = "ProjectV2SingleSelectField";

pub struct GithubIssueBoardFieldAdapter {
    run_gh_api: GhApiRunner,
}

impl GithubIssueBoardFieldAdapter {
    /// `working_directory` 는 `gh` 를 찾고 실행할 때 기준이 되는 폴더이다. 저장소가 아닌 Akra 의 폴더를 준다.
    pub fn new(working_directory: PathBuf) -> Self {
        Self::with_runner(Box::new(move |arguments| {
            run_gh_api(&working_directory, arguments)
        }))
    }

    fn with_runner(run_gh_api: GhApiRunner) -> Self {
        Self { run_gh_api }
    }

    fn query<T: for<'de> Deserialize<'de>>(&self, arguments: &[String]) -> Result<T> {
        let stdout = (self.run_gh_api)(arguments)?;
        // serde 의 오류 글에는 답의 값이 들어갈 수 있다. 남기지 않는다.
        serde_json::from_slice::<GraphqlEnvelope<T>>(&stdout)
            .ok()
            .and_then(|envelope| envelope.data)
            .ok_or_else(|| anyhow!("GitHub did not return board data"))
    }

    fn items_of(&self, issue: &IssueReference) -> Result<Vec<IssueBoardItem>> {
        if issue.provider != IssueProvider::GitHub || issue.server_id != GITHUB_HOSTNAME {
            bail!("the issue is not on github.com")
        }
        ensure_node_id(&issue.issue_id)?;
        let data: NodeData = self.query(&graphql_arguments(
            ITEMS_QUERY,
            &[("id", &issue.issue_id)],
            None,
        ))?;
        let node = data
            .node
            .ok_or_else(|| anyhow!("GitHub does not have this issue"))?;
        if node.typename != "Issue" {
            bail!("the target is not an issue")
        }
        let repository_id = node.repository.map(|repository| repository.id);
        if node.id.as_ref() != Some(&issue.issue_id)
            || repository_id.as_ref() != Some(&issue.repository_id)
        {
            bail!("the target moved to another repository or issue")
        }
        node.project_items
            .ok_or_else(|| anyhow!("GitHub did not return board data"))?
            .into_nodes()?
            .into_iter()
            .map(board_item)
            .collect()
    }
}

impl IssueBoardFieldPort for GithubIssueBoardFieldAdapter {
    fn read_items(&self, request: IssueBoardItemsReadRequest) -> Result<IssueBoardItemsResponse> {
        let items = self.items_of(&request.issue)?;
        Ok(IssueBoardItemsResponse {
            issue: request.issue,
            items,
        })
    }

    fn write_field(
        &self,
        request: IssueBoardFieldWriteRequest,
    ) -> Result<IssueBoardFieldWriteResponse> {
        request.ensure_authorized()?;
        for id in [
            &request.project_id,
            &request.item_id,
            &request.field_id,
            &request.option_id,
        ] {
            ensure_node_id(id)?;
        }
        // 승인은 이슈 한 건의 것이다. 대상이 그 이슈의 항목인지 지금 원격 값으로 확인한다.
        request.ensure_targets_one_of(&self.items_of(&request.issue)?)?;
        let data: WriteData = self
            .query(&graphql_arguments(
                WRITE_MUTATION,
                &[
                    ("projectId", &request.project_id),
                    ("itemId", &request.item_id),
                    ("fieldId", &request.field_id),
                    ("optionId", &request.option_id),
                ],
                None,
            ))
            .context("the board field write did not complete")?;
        let written_item_id = data
            .update
            .and_then(|update| update.project_v2_item)
            .map(|item| item.id);
        if written_item_id.as_ref() != Some(&request.item_id) {
            bail!("GitHub did not confirm the board field write")
        }
        Ok(IssueBoardFieldWriteResponse {
            item_id: request.item_id,
        })
    }
}

fn board_item(item: ItemNode) -> Result<IssueBoardItem> {
    ensure_node_id(&item.id)?;
    ensure_node_id(&item.project.id)?;
    let values = item.field_values.into_nodes()?;
    let fields = item
        .project
        .fields
        .into_nodes()?
        .into_iter()
        .filter(|field| field.typename == SINGLE_SELECT_FIELD)
        .map(|field| board_field(field, &values))
        .collect::<Result<_>>()?;
    Ok(IssueBoardItem {
        project_id: item.project.id,
        project_title: item.project.title,
        item_id: item.id,
        fields,
    })
}

fn board_field(field: FieldNode, values: &[FieldValueNode]) -> Result<IssueBoardField> {
    let incomplete = || anyhow!("GitHub did not return board data");
    let field_id = field.id.ok_or_else(incomplete)?;
    ensure_node_id(&field_id)?;
    let options = field
        .options
        .ok_or_else(incomplete)?
        .into_iter()
        .map(|option| {
            ensure_node_id(&option.id)?;
            Ok(IssueBoardFieldOption {
                option_id: option.id,
                name: option.name,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let selected_option_id = values
        .iter()
        .find(|value| {
            value
                .field
                .as_ref()
                .is_some_and(|value_field| value_field.id.as_ref() == Some(&field_id))
        })
        .and_then(|value| value.option_id.clone());
    Ok(IssueBoardField {
        field_id,
        name: field.name.ok_or_else(incomplete)?,
        options,
        selected_option_id,
    })
}

#[derive(Deserialize)]
struct NodeData {
    node: Option<IssueNode>,
}

/// 이슈가 아닌 답은 `__typename` 만 가진다.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueNode {
    #[serde(rename = "__typename")]
    typename: String,
    id: Option<String>,
    repository: Option<NodeId>,
    project_items: Option<Page<ItemNode>>,
}

#[derive(Deserialize)]
struct NodeId {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    page_info: PageInfo,
    nodes: Vec<Option<T>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
}

impl<T> Page<T> {
    fn into_nodes(self) -> Result<Vec<T>> {
        if self.page_info.has_next_page {
            bail!("the issue has more board data than one page")
        }
        Ok(self.nodes.into_iter().flatten().collect())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemNode {
    id: String,
    project: ProjectNode,
    field_values: Page<FieldValueNode>,
}

#[derive(Deserialize)]
struct ProjectNode {
    id: String,
    title: String,
    fields: Page<FieldNode>,
}

/// 단일 선택이 아닌 필드는 `__typename` 만 가진다.
#[derive(Deserialize)]
struct FieldNode {
    #[serde(rename = "__typename")]
    typename: String,
    id: Option<String>,
    name: Option<String>,
    options: Option<Vec<OptionNode>>,
}

#[derive(Deserialize)]
struct OptionNode {
    id: String,
    name: String,
}

/// 단일 선택이 아닌 값은 `optionId` 와 `field` 가 없다.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldValueNode {
    option_id: Option<String>,
    field: Option<FieldValueField>,
}

#[derive(Deserialize)]
struct FieldValueField {
    id: Option<String>,
}

#[derive(Deserialize)]
struct WriteData {
    #[serde(rename = "updateProjectV2ItemFieldValue")]
    update: Option<WritePayload>,
}

#[derive(Deserialize)]
struct WritePayload {
    #[serde(rename = "projectV2Item")]
    project_v2_item: Option<NodeId>,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::domain::issue_work::{IssueProjectScope, IssueWriteApproval, IssueWriteOperation};

    type Calls = Arc<Mutex<Vec<Vec<String>>>>;

    /// `gh` 의 답을 순서대로 주는 가짜이다. 네트워크와 프로세스가 없다.
    fn adapter_with(responses: Vec<Result<String>>) -> (GithubIssueBoardFieldAdapter, Calls) {
        let calls = Calls::default();
        let recorded = calls.clone();
        let responses = Mutex::new(VecDeque::from(responses));
        let adapter = GithubIssueBoardFieldAdapter::with_runner(Box::new(move |arguments| {
            recorded.lock().unwrap().push(arguments.to_vec());
            let response = responses.lock().unwrap().pop_front();
            response
                .expect("the fake has no more answers")
                .map(String::into_bytes)
        }));
        (adapter, calls)
    }

    /// 보드 하나의 항목이다. 상태와 우선순위 필드가 있고, 상태만 값이 있다.
    fn item_node(board: u8) -> String {
        format!(
            r#"{{"id":"PVTI_{board}","project":{{"id":"PVT_{board}","title":"Board {board}","fields":{{"pageInfo":{{"hasNextPage":false}},"nodes":[{{"__typename":"ProjectV2Field"}},{{"__typename":"ProjectV2SingleSelectField","id":"PVTSSF_{board}s","name":"Status","options":[{{"id":"{board}a","name":"Todo"}},{{"id":"{board}b","name":"Done"}}]}},{{"__typename":"ProjectV2SingleSelectField","id":"PVTSSF_{board}p","name":"Priority","options":[{{"id":"{board}c","name":"P0"}}]}}]}}}},"fieldValues":{{"pageInfo":{{"hasNextPage":false}},"nodes":[{{"__typename":"ProjectV2ItemFieldTextValue"}},{{"__typename":"ProjectV2ItemFieldSingleSelectValue","optionId":"{board}a","field":{{"id":"PVTSSF_{board}s"}}}}]}}}}"#
        )
    }

    fn items_answer_with(node_head: &str, items: &str, has_next_page: bool) -> Result<String> {
        Ok(format!(
            r#"{{"data":{{"node":{{{node_head},"projectItems":{{"pageInfo":{{"hasNextPage":{has_next_page}}},"nodes":[{items}]}}}}}}}}"#
        ))
    }

    /// 같은 이슈가 보드 두 개에 있다.
    fn items_answer() -> Result<String> {
        items_answer_with(
            r#""__typename":"Issue","id":"I_kwDO-1","repository":{"id":"R_kgDO-1"}"#,
            &format!("{},{}", item_node(1), item_node(2)),
            false,
        )
    }

    fn write_answer(item_id: &str) -> Result<String> {
        Ok(format!(
            r#"{{"data":{{"updateProjectV2ItemFieldValue":{{"projectV2Item":{{"id":"{item_id}"}}}}}}}}"#
        ))
    }

    fn reference() -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_kgDO-1".to_string(),
            issue_id: "I_kwDO-1".to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_string(),
            },
        }
    }

    fn read_request() -> IssueBoardItemsReadRequest {
        IssueBoardItemsReadRequest { issue: reference() }
    }

    /// 보드 1 의 상태를 `Done` 으로 바꾸는 요청이다.
    fn write_request(operation: IssueWriteOperation) -> IssueBoardFieldWriteRequest {
        IssueBoardFieldWriteRequest {
            approval: IssueWriteApproval {
                issue: reference(),
                operation,
            },
            issue: reference(),
            project_id: "PVT_1".to_string(),
            item_id: "PVTI_1".to_string(),
            field_id: "PVTSSF_1s".to_string(),
            option_id: "1b".to_string(),
        }
    }

    #[test]
    fn read_items_returns_project_item_field_ids_and_options_per_board() {
        let (adapter, calls) = adapter_with(vec![items_answer()]);

        let response = adapter.read_items(read_request()).unwrap();

        assert_eq!(response.issue, reference());
        assert_eq!(response.items.len(), 2);
        let option = |option_id: &str, name: &str| IssueBoardFieldOption {
            option_id: option_id.to_string(),
            name: name.to_string(),
        };
        assert_eq!(
            response.items[0],
            IssueBoardItem {
                project_id: "PVT_1".to_string(),
                project_title: "Board 1".to_string(),
                item_id: "PVTI_1".to_string(),
                fields: vec![
                    IssueBoardField {
                        field_id: "PVTSSF_1s".to_string(),
                        name: "Status".to_string(),
                        options: vec![option("1a", "Todo"), option("1b", "Done")],
                        selected_option_id: Some("1a".to_string()),
                    },
                    IssueBoardField {
                        field_id: "PVTSSF_1p".to_string(),
                        name: "Priority".to_string(),
                        options: vec![option("1c", "P0")],
                        selected_option_id: None,
                    },
                ],
            }
        );
        assert_eq!(response.items[1].project_id, "PVT_2");
        assert_eq!(response.items[1].item_id, "PVTI_2");
        assert_eq!(
            *calls.lock().unwrap(),
            [[
                "graphql",
                "-f",
                &format!("query={ITEMS_QUERY}"),
                "-f",
                "id=I_kwDO-1"
            ]]
        );
    }

    #[test]
    fn write_field_sends_only_one_project_item_field_and_option_id() {
        let (adapter, calls) = adapter_with(vec![items_answer(), write_answer("PVTI_1")]);

        let response = adapter
            .write_field(write_request(IssueWriteOperation::BoardField))
            .unwrap();

        assert_eq!(response.item_id, "PVTI_1");
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        // 쓰기 인자 전부이다. 이슈 ID, 제목, 본문, 다른 보드의 ID 가 없다.
        assert_eq!(
            calls[1],
            [
                "graphql",
                "-f",
                &format!("query={WRITE_MUTATION}"),
                "-f",
                "projectId=PVT_1",
                "-f",
                "itemId=PVTI_1",
                "-f",
                "fieldId=PVTSSF_1s",
                "-f",
                "optionId=1b"
            ]
        );
        for argument in &calls[1] {
            for other in ["I_kwDO-1", "R_kgDO-1", "PVT_2", "PVTI_2", "PVTSSF_2"] {
                assert!(!argument.contains(other));
            }
        }
        // 쓰기 문은 항목 필드 값만 고친다. 이슈를 고치는 mutation 이 없다.
        assert_eq!(WRITE_MUTATION.matches("mutation").count(), 1);
        assert!(WRITE_MUTATION.contains("updateProjectV2ItemFieldValue(input:"));
        assert!(!WRITE_MUTATION.contains("updateIssue"));
    }

    #[test]
    fn a_body_write_approval_never_reaches_gh() {
        let (adapter, calls) = adapter_with(Vec::new());

        let error = adapter
            .write_field(write_request(IssueWriteOperation::Body))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "the approval does not allow a board field write on this issue"
        );
        let mut other_issue = write_request(IssueWriteOperation::BoardField);
        other_issue.approval.issue.issue_id = "I_kwDO-2".to_string();
        assert!(adapter.write_field(other_issue).is_err());

        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_target_outside_the_issue_board_items_is_never_written() {
        let request = || write_request(IssueWriteOperation::BoardField);
        let requests = [
            // 다른 보드의 항목, 다른 보드의 필드, 다른 필드의 선택지, 다른 Project.
            IssueBoardFieldWriteRequest {
                item_id: "PVTI_2".to_string(),
                ..request()
            },
            IssueBoardFieldWriteRequest {
                field_id: "PVTSSF_2s".to_string(),
                ..request()
            },
            IssueBoardFieldWriteRequest {
                option_id: "1c".to_string(),
                ..request()
            },
            IssueBoardFieldWriteRequest {
                project_id: "PVT_2".to_string(),
                ..request()
            },
        ];
        let (adapter, calls) = adapter_with(requests.iter().map(|_| items_answer()).collect());

        for request in requests {
            assert_eq!(
                adapter.write_field(request).unwrap_err().to_string(),
                "the board item, field, or option does not belong to this issue"
            );
        }
        // 읽기만 했다. mutation 이 없다.
        for call in calls.lock().unwrap().iter() {
            assert_eq!(call[2], format!("query={ITEMS_QUERY}"));
        }
    }

    #[test]
    fn a_failed_or_unconfirmed_write_is_an_error() {
        let (adapter, _) = adapter_with(vec![
            items_answer(),
            Err(anyhow!("gh could not read the issue")),
            items_answer(),
            Ok(r#"{"data":{"updateProjectV2ItemFieldValue":null}}"#.to_string()),
            items_answer(),
            write_answer("PVTI_2"),
            // 읽기가 실패하면 쓰지 않는다.
            Err(anyhow!("gh could not read the issue")),
        ]);

        for _ in 0..4 {
            assert!(
                adapter
                    .write_field(write_request(IssueWriteOperation::BoardField))
                    .is_err()
            );
        }
    }

    #[test]
    fn a_wrong_or_oversized_remote_answer_is_an_error() {
        let issue_head = r#""__typename":"Issue","id":"I_kwDO-1","repository":{"id":"R_kgDO-1"}"#;
        let (adapter, _) = adapter_with(vec![
            Ok(r#"{"data":{"node":{"__typename":"PullRequest"}}}"#.to_string()),
            Ok(r#"{"data":{"node":null}}"#.to_string()),
            items_answer_with(
                r#""__typename":"Issue","id":"I_kwDO-1","repository":{"id":"R_kgDO-2"}"#,
                "",
                false,
            ),
            items_answer_with(issue_head, &item_node(1), true),
            items_answer_with(issue_head, &item_node(1).replace("PVTI_1", "PVTI 1"), false),
            Ok("not json".to_string()),
        ]);

        for _ in 0..6 {
            assert!(adapter.read_items(read_request()).is_err());
        }
    }

    #[test]
    fn invalid_ids_never_reach_gh() {
        let (adapter, calls) = adapter_with(Vec::new());

        for (server_id, issue_id) in [("github.example", "I_kwDO-1"), ("github.com", "@file")] {
            let issue = IssueReference {
                server_id: server_id.to_string(),
                issue_id: issue_id.to_string(),
                ..reference()
            };
            assert!(
                adapter
                    .read_items(IssueBoardItemsReadRequest { issue })
                    .is_err()
            );
        }
        let request = IssueBoardFieldWriteRequest {
            option_id: "1b -f query=x".to_string(),
            ..write_request(IssueWriteOperation::BoardField)
        };
        assert!(adapter.write_field(request).is_err());
        assert!(calls.lock().unwrap().is_empty());
    }
}
