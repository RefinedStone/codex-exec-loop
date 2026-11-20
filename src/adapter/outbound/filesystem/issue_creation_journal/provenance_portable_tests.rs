use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::application::port::outbound::issue_creation_provenance_port::IssueCreationProvenancePort;
use crate::domain::issue_automation::source::IssueSourceRepository;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn unsupported_platform_distinguishes_an_absent_journal_from_an_unreadable_existing_one() {
    let temp = Directory(std::env::temp_dir().join(format!("akra-provenance-{}-{}",
        std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())));
    fs::create_dir_all(temp.0.join("workspace")).unwrap();
    fs::create_dir(temp.0.join("runtime")).unwrap();
    let workspace = fs::canonicalize(temp.0.join("workspace")).unwrap();
    let runtime = temp.0.join("runtime");
    let snapshot_runtime = runtime.clone();
    let adapter = IssueCreationJournalAdapter::new(move |_| Ok(snapshot_runtime.clone()));
    let repository = IssueSourceRepository {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_1".into(),
        project_scope: IssueProjectScope {
            workspace_dir: workspace.to_str().unwrap().to_owned(),
        },
    };
    let empty = adapter.load(&repository).unwrap();
    assert!(!empty.unresolved);
    assert!(empty.created_issue_ids.is_empty());
    assert_eq!(fs::read_dir(&runtime).unwrap().count(), 0);
    let bytes = b"PRIVATE_JOURNAL_MUST_NOT_READ";
    fs::write(runtime.join("issue-create-intents.jsonl"), bytes).unwrap();
    let error = adapter.load(&repository).unwrap_err().to_string();
    assert!(!error.contains("PRIVATE_JOURNAL_MUST_NOT_READ"));
    assert_eq!(
        fs::read(runtime.join("issue-create-intents.jsonl")).unwrap(),
        bytes
    );
    assert_eq!(fs::read_dir(&runtime).unwrap().count(), 1);
}
