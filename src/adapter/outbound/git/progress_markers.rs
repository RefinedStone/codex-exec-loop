/// git 폴더 안의 이 파일이나 폴더가 있으면 그 작업이 끝나지 않았다.
pub(super) const IN_PROGRESS_MARKERS: &[(&str, &str)] = &[
    ("rebase-merge", "rebase"),
    ("rebase-apply", "rebase"),
    ("MERGE_HEAD", "merge"),
    ("CHERRY_PICK_HEAD", "cherry-pick"),
    ("REVERT_HEAD", "revert"),
    ("BISECT_LOG", "bisect"),
    // A manual commit can clear CHERRY_PICK_HEAD / REVERT_HEAD while more steps remain.
    ("sequencer", "cherry-pick/revert"),
];
