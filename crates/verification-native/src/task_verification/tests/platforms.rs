use super::*;

#[cfg(windows)]
#[test]
fn canonical_windows_workspace_is_normalized_only_at_git_boundaries() {
    let fixture = Fixture::new();
    let canonical = fs::canonicalize(&fixture.root).unwrap();
    let source = fixture.source();
    let prepared = prepare(
        TaskVerificationPrepareRequest {
            workspace_directory: canonical.to_str().unwrap(),
            ..fixture.request(&source)
        },
        &|| false,
    )
    .unwrap();
    let command = prepared.context.command(&prepared.binary).unwrap();
    let program = Path::new(command.get_program());
    assert!(!program.to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(
        fs::canonicalize(program).unwrap(),
        prepared.binary.path().to_path_buf()
    );
    let args = command.get_args().collect::<Vec<_>>();
    for name in ["--git-dir", "--work-tree"] {
        let position = args
            .iter()
            .position(|argument| *argument == std::ffi::OsStr::new(name))
            .unwrap();
        let path = Path::new(args[position + 1]);
        assert!(!path.to_string_lossy().starts_with(r"\\?\"));
        assert!(fs::canonicalize(path).unwrap().is_absolute());
    }
    let (observation, _, _) = executed(Box::new(prepared).run(&|| false));
    assert_eq!(
        observation.outcome,
        TaskVerificationExecutionOutcome::Exited { exit_code: 0 }
    );
}

#[test]
fn fixture_configuration_is_local_and_preserves_the_fixed_policy() {
    let fixture = Fixture::new();
    let local = git(&fixture.root, &["config", "--local", "--list"]);
    let settings = local
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect::<std::collections::BTreeMap<_, _>>();
    for (name, expected) in [
        ("user.name", "Host Verification Fixture"),
        ("user.email", "fixture@example.invalid"),
        ("core.autocrlf", "false"),
        ("commit.gpgsign", "false"),
    ] {
        assert_eq!(settings.get(name).copied(), Some(expected), "{name}");
    }
}
