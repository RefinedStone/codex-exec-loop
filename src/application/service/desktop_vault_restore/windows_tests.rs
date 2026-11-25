use super::*;
use crate::domain::issue_work::IssueProjectScope;

fn issue(workspace: &str) -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_synthetic".into(),
        issue_id: "I_synthetic".into(),
        project_scope: IssueProjectScope {
            workspace_dir: workspace.into(),
        },
    }
}

#[test]
fn windows_verbatim_drive_scope_is_accepted_without_rewriting_identity() {
    let issue = issue(r"\\?\C:\work\한글 project");
    let original = issue.clone();
    assert_eq!(validate_issue(&issue), Ok(()));
    assert_eq!(issue, original);
}

#[test]
fn windows_non_disk_prefixes_do_not_become_restore_scopes() {
    for workspace in [
        r"C:relative",
        r"\root-relative",
        r"\\?\C:relative",
        r"\\server\share\project",
        r"\\?\UNC\server\share\project",
        r"\\.\C:\work",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1\work",
    ] {
        assert_eq!(
            validate_issue(&issue(workspace)),
            Err(Hold::NotBound),
            "{workspace}"
        );
    }
}
