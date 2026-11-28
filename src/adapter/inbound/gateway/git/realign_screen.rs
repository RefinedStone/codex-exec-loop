//! Real Electron + real Git service. Only the conversation list is replayed.
use super::*;
use crate::adapter::outbound::{
    filesystem::git_operation_journal::GitOperationJournalAdapter,
    git::{history::GitHistoryAdapter, workspace::GitWorkspaceAdapter},
};
use crate::application::service::{git_graph::GitGraphService, git_operation::GitOperationService};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
#[ignore = "requires a built Desktop and Electron; run the realign_graph_screen fixture explicitly"]
fn realign_graph_screen() {
    let root = crate::test_utils::platform_safe_temp_dir()
        .join(format!("akra-group-screen-{}", rand::random::<u64>()));
    let repo = root.join("repo");
    let journal = root.join("journal");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&journal).unwrap();
    let ops = GitOperationService::new(
        Arc::new(GitHistoryAdapter),
        Arc::new(GitWorkspaceAdapter),
        Arc::new(GitOperationJournalAdapter::new(
            move |_| Ok(journal.clone()),
        )),
        Arc::new(|| (chrono::Utc::now().timestamp(), rand::random::<u32>())),
        Arc::default(),
    );
    let ports = GitPorts {
        graph: Arc::new(GitGraphService::new(Arc::new(GitHistoryAdapter))),
        operations: Some(Arc::new(ops)),
    };
    let mut proc = Command::new("node")
        .args([
            "--experimental-strip-types",
            "--disable-warning=ExperimentalWarning",
            "--disable-warning=MODULE_TYPELESS_PACKAGE_JSON",
            "test/e2e/fixtures/realignGit.mjs",
        ])
        .arg(&root)
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/desktop"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = proc.stdin.take().unwrap();
    for line in BufReader::new(proc.stdout.take().unwrap()).lines() {
        let line = line.unwrap();
        if !line.starts_with('{') {
            eprintln!("{line}");
            continue;
        }
        let envelope: Value = serde_json::from_str(&line).unwrap();
        let cwd = envelope["params"]["cwd"].as_str().unwrap();
        assert_eq!(
            std::fs::canonicalize(cwd).unwrap(),
            std::fs::canonicalize(&repo).unwrap()
        );
        let reply = respond(
            &ports,
            envelope["id"].clone(),
            envelope["method"].as_str().unwrap(),
            &line,
        );
        writeln!(input, "{reply}").unwrap();
        input.flush().unwrap();
    }
    let status = proc.wait().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(status.success(), "Electron scenario failed: {status}");
}
