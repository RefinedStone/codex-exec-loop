use super::*;
use crate::domain::git_operation::cherry_pick::PickPrediction;

#[test]
fn realign_group_replays_once_on_the_new_base_and_undo_restores_all_refs() {
    let dir = temp_repository("group-realign");
    let cwd = dir.to_str().unwrap();
    let start = commit_file(&dir, "base", "original", "original");
    git(&dir, &["switch", "-c", "feature"]);
    let pick = commit_file(&dir, "mine", "keep", "my commit");
    git(&dir, &["switch", "main"]);
    let base = commit_file(&dir, "base", "rewritten", "new base");
    git(&dir, &["switch", "feature"]);
    let before = git_text(&dir, &["show-ref"]);
    let e = engine();
    let op = GitOperation::Realign {
        base: base.clone(),
        commits: vec![pick.clone(), start],
        publish: None,
    };
    let p = e.plan(cwd, op.clone()).unwrap();
    assert_eq!(p.before.unwrap().commit, pick);
    assert_eq!(p.after.unwrap().commit, base);
    assert_eq!(p.picks[0].prediction, PickPrediction::AlreadyApplied);
    assert_eq!(p.picks[1].prediction, PickPrediction::Clean);
    assert_eq!(git_text(&dir, &["show-ref"]), before);
    let expected = p.expectation;
    let GitOperationOutcome::Done { record } =
        e.execute(cwd, op.clone(), expected.clone()).unwrap()
    else {
        panic!("expected local success")
    };
    assert_eq!(git_text(&dir, &["rev-parse", "HEAD^"]), base);
    assert_eq!(std::fs::read_to_string(dir.join("mine")).unwrap(), "keep");
    assert_eq!(
        std::fs::read_to_string(dir.join("base")).unwrap(),
        "rewritten"
    );
    assert!(matches!(
        e.execute(cwd, op, expected).unwrap(),
        GitOperationOutcome::Stale { .. }
    ));
    assert!(matches!(
        e.undo(cwd, &record.op_id).unwrap(),
        GitOperationOutcome::Done { .. }
    ));
    assert_eq!(git_text(&dir, &["show-ref"]), before);
    std::fs::remove_dir_all(dir).unwrap();
}
