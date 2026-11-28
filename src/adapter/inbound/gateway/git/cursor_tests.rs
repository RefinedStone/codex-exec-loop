use super::*;
use crate::adapter::outbound::git::history::GitHistoryAdapter;
use crate::application::service::git_graph::GitGraphService;
use crate::test_utils::{git_command, platform_safe_temp_dir};

#[test]
fn wire_cursors_resume_the_same_git_walk() {
    let repo = platform_safe_temp_dir().join(format!("akra-cursor-{}", rand::random::<u64>()));
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let result = git_command()
            .current_dir(&repo)
            .args([
                "-c",
                "user.name=Cursor Test",
                "-c",
                "user.email=cursor@example.invalid",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    for subject in ["keep first", "other", "keep last"] {
        git(&["commit", "-q", "--allow-empty", "-m", subject]);
    }
    let ports = GitPorts {
        graph: Arc::new(GitGraphService::new(Arc::new(GitHistoryAdapter))),
        operations: None,
    };
    let page = |mut params: Value| {
        params["cwd"] = json!(repo);
        params["limit"] = json!(1);
        let request = json!({"id": 1, "method": GIT_GRAPH_METHOD, "params": params});
        let reply: Value = serde_json::from_str(&respond(
            &ports,
            json!(1),
            GIT_GRAPH_METHOD,
            &request.to_string(),
        ))
        .unwrap();
        reply
            .get("result")
            .unwrap_or_else(|| panic!("{reply}"))
            .clone()
    };
    let all = page(json!({}));
    let legacy = json!({"skip": 1, "lanes": all["next"]["lanes"], "refsDigest": all["refsDigest"]});
    assert_eq!(all["next"], legacy);
    assert_eq!(
        page(json!({"cursor": legacy}))["rows"][0]["subject"],
        "other"
    );
    // Hide without a criterion does not select a different walk.
    assert_eq!(page(json!({"filter": {"hide": true}}))["next"], legacy);

    let filter = json!({"message": "keep", "hide": true});
    let filtered = page(json!({"filter": filter}));
    assert_eq!(filtered["rows"][0]["subject"], "keep last");
    let cursor = &filtered["next"];
    let next = page(json!({"cursor": cursor, "filter": filter}));
    assert_eq!(next["rows"][0]["subject"], "keep first");
    assert!(next["next"].is_null());
    for request in [
        json!({"cursor": legacy, "filter": filter}),
        json!({"cursor": cursor}),
        json!({"cursor": cursor, "filter": {"message": "other", "hide": true}}),
    ] {
        let stale = page(request);
        assert_eq!(stale["stale"], true);
        assert_eq!(stale["rows"], json!([]));
    }
    std::fs::remove_dir_all(repo).unwrap();
}
