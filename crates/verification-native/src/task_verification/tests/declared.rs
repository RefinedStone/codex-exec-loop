use super::*;

fn prepare_grep(
    fixture: &Fixture,
    source: &TaskEvidenceTree,
) -> Result<PreparedVerification, Reason> {
    prepare(
        TaskVerificationPrepareRequest {
            recipe: TaskVerificationRecipe::RepoGitGrepV1,
            ..fixture.request(source)
        },
        &|| false,
    )
}

/// Manual smoke only: this invokes the real default-cancel local OS dialog.
/// The disposable repository has no remotes and contains only the fixture text.
#[test]
#[ignore = "requires an interactive native confirmation of the disposable fixture"]
fn native_dialog_can_approve_a_single_disposable_marker_check() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.root.join(".akra")).unwrap();
    fs::write(fixture.root.join("marker.txt"), "qa-a01-marker\n").unwrap();
    fs::write(
        fixture.root.join(".akra/verify-grep.json"),
        br#"{"path":"marker.txt","literal":"qa-a01-marker"}"#,
    )
    .unwrap();
    fixture.commit();
    let prepared = prepare_grep(&fixture, &fixture.source()).unwrap();
    let (result, _, _) = executed(Box::new(prepared).run(&|| false));
    assert_eq!(
        result.outcome,
        TaskVerificationExecutionOutcome::Exited { exit_code: 0 }
    );
}

#[test]
fn approval_applies_to_the_exact_frozen_check_not_the_current_worktree() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.root.join(".akra")).unwrap();
    let declaration = fixture.root.join(".akra/verify-grep.json");
    fs::write(
        &declaration,
        br#"{"path":"README.md","literal":"base text"}"#,
    )
    .unwrap();
    fixture.commit();
    let source = fixture.source();
    let prepared = prepare_grep(&fixture, &source).unwrap();
    let manifest = prepared.manifest.clone();
    fs::write(&declaration, br#"{"path":"README.md","literal":"absent"}"#).unwrap();
    fs::write(fixture.root.join("README.md"), "live file does not match").unwrap();

    let (result, _, _) = executed(prepared.run_with_consent(
        |command, confirmed, _, _| {
            assert_eq!(confirmed, &manifest);
            assert_ne!(command.get_current_dir().unwrap(), fixture.root);
            assert!(
                command
                    .get_current_dir()
                    .unwrap()
                    .read_dir()
                    .unwrap()
                    .next()
                    .is_none()
            );
            let args = command.get_args().collect::<Vec<_>>();
            assert!(
                args.ends_with(
                    &[
                        "grep",
                        "--quiet",
                        "--no-textconv",
                        "--fixed-strings",
                        "-e",
                        "base text",
                        &source.tree_sha,
                        "--",
                        "README.md",
                    ]
                    .map(std::ffi::OsStr::new)
                )
            );
            Ok(())
        },
        &|| false,
    ));
    assert_eq!(
        result.outcome,
        TaskVerificationExecutionOutcome::Exited { exit_code: 0 }
    );
    fixture.commit();
    let updated = prepare_grep(&fixture, &fixture.source()).unwrap();
    assert_ne!(manifest.command_sha256, updated.manifest.command_sha256);
    let (result, _, _) = executed(updated.run_with_consent(|_, _, _, _| Ok(()), &|| false));
    assert_eq!(
        result.outcome,
        TaskVerificationExecutionOutcome::Exited { exit_code: 1 }
    );
}

#[test]
fn declined_declaration_cannot_produce_an_execution_observation() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.root.join(".akra")).unwrap();
    fs::write(
        fixture.root.join(".akra/verify-grep.json"),
        br#"{"path":"README.md","literal":"base"}"#,
    )
    .unwrap();
    fixture.commit();
    let prepared = prepare_grep(&fixture, &fixture.source()).unwrap();
    assert!(matches!(
        prepared.run_with_consent(|_, _, _, _| Err(Reason::Cancelled), &|| false),
        TaskVerificationRunOutcome::NotRun {
            reason: Reason::Cancelled
        }
    ));
}

#[test]
fn declaration_cannot_select_a_program_environment_or_outside_input() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.root.join(".akra")).unwrap();
    for value in [
        r#"{"path":"README.md","literal":"base","program":"sh"}"#,
        r#"{"path":"README.md","literal":"base","env":{"LANG":"C"}}"#,
        r#"{"path":"../README.md","literal":"base"}"#,
        r#"{"path":"README.md","literal":" \n "}"#,
    ] {
        fs::write(fixture.root.join(".akra/verify-grep.json"), value).unwrap();
        fixture.commit();
        assert!(matches!(
            prepare_grep(&fixture, &fixture.source()),
            Err(Reason::InvalidInput)
        ));
    }
}
