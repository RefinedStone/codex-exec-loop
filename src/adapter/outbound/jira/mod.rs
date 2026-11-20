// adf 는 Jira 문서 형식(ADF)의 정해진 부분을 Markdown 으로 옮긴다 (#2433).
mod adf;
// issue_snapshot adapter는 Jira Cloud 이슈 한 건의 안정 ID, 본문, 필수 필드를 읽는다. 쓰지 않는다 (#2433).
pub mod issue_snapshot;

pub use self::issue_snapshot::{JiraApiRunner, JiraIssueSnapshotAdapter};
