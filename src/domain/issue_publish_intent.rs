/*
 * 이슈 게시 의도의 기록과 상태 규칙이다 (#2424). I/O 가 없다.
 *
 * 원격 쓰기의 답을 잃으면 쓰기가 되었는지 모른다. 그래서 쓰기 전에 의도를 기록하고, 쓰기 뒤에는 원격을 다시 읽어
 * 상태를 정한다. 기록에는 지문(필드 해시)만 있다. 제목, 본문, 가린 평문은 없다.
 *
 * 상태의 흐름:
 *
 *   Recorded -> Sent -> Confirmed
 *      |          `---> NeedsConfirmation -> Confirmed | Abandoned
 *      `-> Conflict | Abandoned
 *   Held (의도를 기록하기 전에 멈춘 게시이다)
 */
use serde::{Deserialize, Serialize};

use crate::domain::issue_work::{IssueFieldHash, IssueReference};

/// 게시 의도의 상태이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssuePublishIntentState {
    /// 의도를 기록했다. 쓰기 요청은 아직 보내지 않았다.
    Recorded,
    /// 쓰기 요청을 보내기 직전에 쓴다. 이 상태로 남은 의도는 결과를 모른다.
    Sent,
    /// 원격의 글이 승인한 공유본과 같다. 이 상태에서만 복원 링크를 확정한다.
    Confirmed,
    /// 게시 결과 확인 필요. 쓰기가 되었는지 모른다. 자동으로 다시 보내지 않는다.
    NeedsConfirmation,
    /// 원격의 글이 편집 기준과 달랐다. 쓰지 않았다.
    Conflict,
    /// 복원 가능으로 표시한 구간을 Vault 에 저장하지 못했다. 쓰지 않았다. 사용자가 공유본만 게시할지 정한다.
    Held,
    /// 보내지 않은 의도이거나 사용자가 결과 확인을 그만둔 의도이다.
    Abandoned,
}

impl IssuePublishIntentState {
    /// 결과를 모르는 상태이다. 이 상태의 의도가 있는 이슈에는 새 쓰기를 보내지 않는다.
    pub fn is_unresolved(self) -> bool {
        matches!(self, Self::Recorded | Self::Sent | Self::NeedsConfirmation)
    }
}

/// Vault 레코드가 있는 이슈의 필드이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssuePublishVaultField {
    Title,
    Body,
}

/// 게시 의도 하나의 기록이다. 일지에서 같은 `intent_id` 의 뒤 기록이 앞 기록을 대신한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuePublishIntentRecord {
    pub intent_id: String,
    /// 쓰기의 대상이다.
    pub issue: IssueReference,
    /// 사용자가 편집을 시작한 원격 글의 지문이다.
    pub previous_hash: IssueFieldHash,
    /// 승인한 공유본의 지문이다.
    pub approved_hash: IssueFieldHash,
    /// 이 의도(`VaultRecordBinding::PublishIntent(intent_id)`)에 묶어 저장한 Vault 레코드의 필드이다.
    pub pending_vault_fields: Vec<IssuePublishVaultField>,
    pub state: IssuePublishIntentState,
    /// 마지막 재조회에서 본 원격 글의 지문이다. 재조회를 하지 않았거나 실패했으면 `None` 이다.
    pub observed_hash: Option<IssueFieldHash>,
    /// Vault 레코드를 확인된 판에 묶었으면 `true` 이다.
    pub restore_link_finalized: bool,
}

impl IssuePublishIntentRecord {
    /// 확인된 판의 이름이다 (`VaultRecordBinding::ConfirmedVersion`). 승인한 공유본의 지문으로 만든다.
    pub fn confirmed_version(&self) -> String {
        format!(
            "{}:{}",
            self.approved_hash.version, self.approved_hash.sha256
        )
    }

    /// 원격을 다시 읽은 결과로 다음 상태를 정한다. `observed` 는 지금 원격 글의 지문이고, 읽지 못했으면 `None` 이다.
    /// 결과를 모르는 상태만 바뀐다. 이 규칙은 쓰기를 만들지 않는다.
    pub fn reconciled(mut self, observed: Option<IssueFieldHash>) -> Self {
        if !self.state.is_unresolved() {
            return self;
        }
        self.state = match &observed {
            Some(hash) if *hash == self.approved_hash => IssuePublishIntentState::Confirmed,
            // 보내지 않은 의도이고 원격도 승인한 글이 아니다. 쓰기는 없었다.
            Some(_) if self.state == IssuePublishIntentState::Recorded => {
                IssuePublishIntentState::Abandoned
            }
            _ => IssuePublishIntentState::NeedsConfirmation,
        };
        self.observed_hash = observed;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

    fn record(state: IssuePublishIntentState) -> IssuePublishIntentRecord {
        IssuePublishIntentRecord {
            intent_id: "intent-1".to_string(),
            issue: IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_1".to_string(),
                issue_id: "I_1".to_string(),
                project_scope: IssueProjectScope {
                    workspace_dir: "/work/project".to_string(),
                },
            },
            previous_hash: IssueFieldHash::of_fields("Old", "old"),
            approved_hash: IssueFieldHash::of_fields("New", "new"),
            pending_vault_fields: vec![IssuePublishVaultField::Body],
            state,
            observed_hash: None,
            restore_link_finalized: false,
        }
    }

    #[test]
    fn a_reread_settles_only_unresolved_intents() {
        use IssuePublishIntentState::*;
        let approved = Some(IssueFieldHash::of_fields("New", "new"));
        let other = Some(IssueFieldHash::of_fields("Old", "old"));
        let cases = [
            (Recorded, approved.clone(), Confirmed),
            (Sent, approved.clone(), Confirmed),
            (NeedsConfirmation, approved.clone(), Confirmed),
            (Recorded, other.clone(), Abandoned),
            (Sent, other.clone(), NeedsConfirmation),
            (NeedsConfirmation, other.clone(), NeedsConfirmation),
            // 재조회 실패.
            (Recorded, None, NeedsConfirmation),
            (Sent, None, NeedsConfirmation),
            // 끝난 상태는 바뀌지 않는다.
            (Confirmed, other.clone(), Confirmed),
            (Conflict, approved.clone(), Conflict),
            (Held, approved.clone(), Held),
            (Abandoned, approved, Abandoned),
        ];
        for (before, observed, after) in cases {
            assert_eq!(
                record(before).reconciled(observed.clone()).state,
                after,
                "{before:?} with {observed:?}"
            );
        }
    }

    #[test]
    fn the_journal_line_holds_fingerprints_and_no_text() {
        let line = serde_json::to_value(record(IssuePublishIntentState::Sent)).unwrap();
        assert_eq!(line["state"], "sent");
        assert_eq!(line["pendingVaultFields"][0], "body");
        assert_eq!(line["approvedHash"]["version"], 1);
        assert_eq!(
            record(IssuePublishIntentState::Sent).confirmed_version(),
            format!("1:{}", IssueFieldHash::of_fields("New", "new").sha256)
        );
    }
}
