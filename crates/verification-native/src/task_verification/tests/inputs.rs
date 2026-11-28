use super::*;

#[test]
fn binary_and_oversized_blobs_are_not_misrepresented_as_passing_text() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("binary.dat"), b"data\0hidden").unwrap();
    fixture.commit();
    assert!(matches!(
        prepare(fixture.request(&fixture.source()), &|| false),
        Err(Reason::InvalidInput)
    ));
    fs::remove_file(fixture.root.join("binary.dat")).unwrap();
    fs::write(
        fixture.root.join("large.txt"),
        vec![b'x'; environment::MAX_BLOB_BYTES + 1],
    )
    .unwrap();
    fixture.commit();
    assert!(matches!(
        prepare(fixture.request(&fixture.source()), &|| false),
        Err(Reason::InvalidInput)
    ));
}

#[test]
fn symlink_and_gitlink_diff_modes_are_rejected_on_every_platform() {
    for mode in ["120000", "160000"] {
        let fixture = Fixture::new();
        let oid = if mode == "160000" {
            fixture.base.clone()
        } else {
            git(&fixture.root, &["rev-parse", "HEAD:README.md"])
        };
        git(
            &fixture.root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{mode},{oid},special"),
            ],
        );
        git(&fixture.root, &["commit", "-qm", "unsupported mode"]);
        assert!(matches!(
            prepare(fixture.request(&fixture.source()), &|| false),
            Err(Reason::InvalidInput)
        ));
    }
}

#[test]
fn source_execution_configuration_and_existing_alternates_fail_closed() {
    let fixture = Fixture::new();
    let source = fixture.source();
    git(
        &fixture.root,
        &["config", "diff.external", "unexpected-verifier-command"],
    );
    assert!(prepare(fixture.request(&source), &|| false).is_err());
    git(&fixture.root, &["config", "--unset", "diff.external"]);
    fs::write(fixture.root.join(".git/objects/info/alternates"), b"").unwrap();
    assert!(prepare(fixture.request(&source), &|| false).is_err());
}

#[test]
fn malformed_or_over_limit_diff_inventory_is_rejected_without_partial_results() {
    let valid = format!(
        ":100644 100644 {} {} M\0src/file.rs\0",
        "a".repeat(40),
        "b".repeat(40)
    );
    assert_eq!(
        input::parse_changed_blobs(valid.as_bytes(), 40)
            .unwrap()
            .len(),
        2
    );
    for malformed in [
        valid.trim_end_matches('\0').as_bytes(),
        b":100644 100644 a b M\0path\0",
        b"header\0",
    ] {
        assert_eq!(
            input::parse_changed_blobs(malformed, 40),
            Err(Reason::InvalidInput)
        );
    }
    let oversized = valid.repeat(environment::MAX_PATHS + 1);
    assert_eq!(
        input::parse_changed_blobs(oversized.as_bytes(), 40),
        Err(Reason::InvalidInput)
    );
}
