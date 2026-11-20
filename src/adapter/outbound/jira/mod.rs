// adf 는 Jira 문서 형식(ADF)의 정해진 부분을 Markdown 으로 옮기고, 그 Markdown 을 ADF 로 옮긴다 (#2433, #2434).
mod adf;
// issue_snapshot adapter는 Jira Cloud 이슈 한 건의 안정 ID, 본문, 필수 필드를 읽는다. 쓰지 않는다 (#2433).
pub mod issue_snapshot;
// issue_update adapter는 승인한 공유본으로 Jira Cloud 이슈 한 건의 제목과 본문을 고친다 (#2434).
pub mod issue_update;
// 게시 의도 기록, Jira 쓰기, Vault 복원을 메모리 Jira 로 잇는 시험이다 (#2434).
#[cfg(test)]
mod publish_flow_tests;

pub use self::adf::JIRA_BODY_REPRESENTATION_VERSION;
pub use self::issue_snapshot::{JiraApiRunner, JiraIssueSnapshotAdapter};
pub use self::issue_update::{JiraApiPutRunner, JiraIssueUpdateAdapter};
