use anyhow::{Result, bail};

use crate::domain::issue_work::{IssueReference, IssueWriteApproval, IssueWriteOperation};

/*
 * 보드(GitHub Projects) 항목의 필드 계약이다 (#2435). 이슈 한 건이 속한 보드마다 항목이 하나 있다. 항목의 필드
 * (상태, 우선순위 같은 단일 선택 필드)를 읽고, 항목 하나의 필드 하나를 고친다.
 *
 * 보드 필드 쓰기는 이슈 본문 쓰기와 다른 동작이다. 요청은 `IssueWriteOperation::BoardField` 승인을 가진다.
 * 본문 쓰기 승인으로는 쓰지 못한다. 쓰기는 항목 ID 하나와 필드 ID 하나만 가리킨다. 같은 이슈의 다른 보드 항목과
 * 이슈 본문은 바꾸지 않는다.
 */

/// 이 계약의 판이다. 요청이나 응답의 뜻이 바뀌면 올린다.
pub const ISSUE_BOARD_FIELD_CONTRACT_VERSION: u32 = 1;

/// 단일 선택 필드의 선택지 하나이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueBoardFieldOption {
    pub option_id: String,
    pub name: String,
}

/// 보드의 단일 선택 필드 하나와, 이 항목이 지금 가진 값이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueBoardField {
    pub field_id: String,
    pub name: String,
    pub options: Vec<IssueBoardFieldOption>,
    /// 값이 없으면 `None` 이다.
    pub selected_option_id: Option<String>,
}

/// 보드 하나에 있는 이슈의 항목이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueBoardItem {
    pub project_id: String,
    pub project_title: String,
    pub item_id: String,
    pub fields: Vec<IssueBoardField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueBoardItemsReadRequest {
    pub issue: IssueReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueBoardItemsResponse {
    pub issue: IssueReference,
    /// 이슈가 속한 보드마다 하나이다.
    pub items: Vec<IssueBoardItem>,
}

/// 항목 하나의 필드 하나를 선택지 하나로 바꾸는 요청이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueBoardFieldWriteRequest {
    pub approval: IssueWriteApproval,
    pub issue: IssueReference,
    pub project_id: String,
    pub item_id: String,
    pub field_id: String,
    pub option_id: String,
}

impl IssueBoardFieldWriteRequest {
    /// 승인이 이 이슈의 보드 필드 쓰기인지 본다. 구현은 원격을 부르기 전에 이 검사를 한다.
    pub fn ensure_authorized(&self) -> Result<()> {
        if !self
            .approval
            .authorizes(&self.issue, IssueWriteOperation::BoardField)
        {
            bail!("the approval does not allow a board field write on this issue")
        }
        Ok(())
    }

    /// 요청이 가리키는 항목, 필드, 선택지가 이 이슈의 보드 항목에 있는지 본다.
    pub fn ensure_targets_one_of(&self, items: &[IssueBoardItem]) -> Result<()> {
        let is_known = items
            .iter()
            .filter(|item| item.item_id == self.item_id && item.project_id == self.project_id)
            .flat_map(|item| &item.fields)
            .filter(|field| field.field_id == self.field_id)
            .flat_map(|field| &field.options)
            .any(|option| option.option_id == self.option_id);
        if !is_known {
            bail!("the board item, field, or option does not belong to this issue")
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueBoardFieldWriteResponse {
    /// 공급자가 고쳤다고 답한 항목이다. 요청의 항목과 같다.
    pub item_id: String,
}

pub trait IssueBoardFieldPort: Send + Sync {
    /// 구현이 따르는 계약의 판이다.
    fn contract_version(&self) -> u32 {
        ISSUE_BOARD_FIELD_CONTRACT_VERSION
    }

    /// 이슈가 속한 보드의 항목과 단일 선택 필드, 선택지를 읽는다.
    fn read_items(&self, request: IssueBoardItemsReadRequest) -> Result<IssueBoardItemsResponse>;

    /// 항목 하나의 필드 하나를 고친다. 승인이 맞지 않거나 대상이 이 이슈의 항목이 아니면 오류이다.
    fn write_field(
        &self,
        request: IssueBoardFieldWriteRequest,
    ) -> Result<IssueBoardFieldWriteResponse>;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

    /// 메모리에 보드 항목과 이슈 본문을 가진 가짜이다. 네트워크가 없다.
    struct FakeIssueBoardFieldAdapter {
        items: Mutex<Vec<IssueBoardItem>>,
        body: String,
    }

    impl IssueBoardFieldPort for FakeIssueBoardFieldAdapter {
        fn read_items(
            &self,
            request: IssueBoardItemsReadRequest,
        ) -> Result<IssueBoardItemsResponse> {
            Ok(IssueBoardItemsResponse {
                issue: request.issue,
                items: self.items.lock().unwrap().clone(),
            })
        }

        fn write_field(
            &self,
            request: IssueBoardFieldWriteRequest,
        ) -> Result<IssueBoardFieldWriteResponse> {
            request.ensure_authorized()?;
            let mut items = self.items.lock().unwrap();
            request.ensure_targets_one_of(&items)?;
            for item in items
                .iter_mut()
                .filter(|item| item.item_id == request.item_id)
            {
                for field in &mut item.fields {
                    if field.field_id == request.field_id {
                        field.selected_option_id = Some(request.option_id.clone());
                    }
                }
            }
            Ok(IssueBoardFieldWriteResponse {
                item_id: request.item_id,
            })
        }
    }

    fn issue() -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_1".to_string(),
            issue_id: "I_1".to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_string(),
            },
        }
    }

    fn field(field_id: &str, name: &str, selected: &str) -> IssueBoardField {
        IssueBoardField {
            field_id: field_id.to_string(),
            name: name.to_string(),
            options: ["todo", "done"]
                .map(|option| IssueBoardFieldOption {
                    option_id: format!("{field_id}-{option}"),
                    name: option.to_string(),
                })
                .to_vec(),
            selected_option_id: Some(format!("{field_id}-{selected}")),
        }
    }

    /// 같은 이슈가 보드 두 개에 있다.
    fn fake() -> FakeIssueBoardFieldAdapter {
        let item = |project_id: &str, item_id: &str, prefix: &str| IssueBoardItem {
            project_id: project_id.to_string(),
            project_title: project_id.to_string(),
            item_id: item_id.to_string(),
            fields: vec![
                field(&format!("{prefix}-status"), "Status", "todo"),
                field(&format!("{prefix}-priority"), "Priority", "todo"),
            ],
        };
        FakeIssueBoardFieldAdapter {
            items: Mutex::new(vec![
                item("P_1", "ITEM_1", "F1"),
                item("P_2", "ITEM_2", "F2"),
            ]),
            body: "issue body".to_string(),
        }
    }

    fn write_request(operation: IssueWriteOperation) -> IssueBoardFieldWriteRequest {
        IssueBoardFieldWriteRequest {
            approval: IssueWriteApproval {
                issue: issue(),
                operation,
            },
            issue: issue(),
            project_id: "P_1".to_string(),
            item_id: "ITEM_1".to_string(),
            field_id: "F1-status".to_string(),
            option_id: "F1-status-done".to_string(),
        }
    }

    #[test]
    fn a_board_field_write_changes_one_field_of_one_item_only() {
        let fake = fake();
        let before = fake.items.lock().unwrap().clone();

        let response = fake
            .write_field(write_request(IssueWriteOperation::BoardField))
            .unwrap();

        assert_eq!(response.item_id, "ITEM_1");
        let after = fake.items.lock().unwrap().clone();
        assert_eq!(
            after[0].fields[0].selected_option_id.as_deref(),
            Some("F1-status-done")
        );
        // 같은 항목의 다른 필드, 다른 보드의 같은 이슈 항목, 이슈 본문은 그대로이다.
        assert_eq!(after[0].fields[1], before[0].fields[1]);
        assert_eq!(after[1], before[1]);
        assert_eq!(fake.body, "issue body");
    }

    #[test]
    fn a_body_write_approval_does_not_allow_a_board_field_write() {
        let fake = fake();
        let before = fake.items.lock().unwrap().clone();

        assert!(
            fake.write_field(write_request(IssueWriteOperation::Body))
                .is_err()
        );
        // 다른 이슈의 승인도 허락하지 않는다.
        let mut other_issue = write_request(IssueWriteOperation::BoardField);
        other_issue.approval.issue.issue_id = "I_2".to_string();
        assert!(fake.write_field(other_issue).is_err());

        assert_eq!(*fake.items.lock().unwrap(), before);
    }

    #[test]
    fn a_target_outside_the_issue_board_items_is_rejected() {
        let fake = fake();
        let before = fake.items.lock().unwrap().clone();
        let request = || write_request(IssueWriteOperation::BoardField);

        for request in [
            // 다른 보드의 항목, 다른 보드의 필드, 다른 필드의 선택지, 모르는 항목.
            IssueBoardFieldWriteRequest {
                item_id: "ITEM_2".to_string(),
                ..request()
            },
            IssueBoardFieldWriteRequest {
                field_id: "F2-status".to_string(),
                ..request()
            },
            IssueBoardFieldWriteRequest {
                option_id: "F1-priority-done".to_string(),
                ..request()
            },
            IssueBoardFieldWriteRequest {
                item_id: "ITEM_9".to_string(),
                ..request()
            },
        ] {
            assert!(fake.write_field(request).is_err());
        }
        assert_eq!(*fake.items.lock().unwrap(), before);
    }
}
