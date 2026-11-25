//! 작업 폴더의 Akra 디렉터리를 `.codex-exec-loop/`에서 `.akra/`로 옮긴다 (#2588).
//!
//! 저장소 커널 잠금을 쥔 쪽만 이 이전을 실행한다 (`RepositoryKernelLockAdapter::acquire_for_owner`). 같은 저장소의
//! linked worktree는 각자 파일을 가지므로, 잠금을 잡은 worktree가 열릴 때 그 worktree의 파일을 옮긴다. DB의 경로
//! 행은 authority store의 schema migration이 따로 한 번 옮긴다.
//!
//! 옮기기는 디렉터리 이름 바꾸기(rename)이다. 원본은 이름 바꾸기가 성공할 때까지 그대로 남는다. `git mv`는 쓰지
//! 않는다. Git은 바뀐 경로를 사용자가 커밋할 변경으로 보인다.

use std::fmt;
use std::io;
use std::path::Path;

use crate::domain::planning::{
    AKRA_WORKSPACE_DIRECTORY, LEGACY_WORKSPACE_DIRECTORY, RELOCATED_WORKSPACE_ENTRIES,
};

/// 작업 폴더 이전이 끝나지 못했다. 호출자는 `downcast_ref`로 이 경우를 가리고, 잠금 없이 시작하는 길로 우회하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceLayoutMigrationError(String);

impl fmt::Display for WorkspaceLayoutMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for WorkspaceLayoutMigrationError {}

/// `workspace_root`의 `.codex-exec-loop/<x>`를 `.akra/<x>`로 옮긴다. 이미 옮겼거나 이전 디렉터리가 없으면
/// 아무것도 하지 않는다.
pub(crate) fn migrate_legacy_workspace_layout(
    workspace_root: &Path,
) -> Result<(), WorkspaceLayoutMigrationError> {
    migrate_with(workspace_root, |source, target| {
        std::fs::rename(source, target)
    })
}

fn migrate_with(
    workspace_root: &Path,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<(), WorkspaceLayoutMigrationError> {
    let legacy_root = workspace_root.join(LEGACY_WORKSPACE_DIRECTORY);
    if !real_directory(&legacy_root)? {
        return Ok(());
    }
    let akra_root = workspace_root.join(AKRA_WORKSPACE_DIRECTORY);
    if entry_exists(&akra_root)? && !real_directory(&akra_root)? {
        return Err(failure(format!(
            "`{}` must be a directory before Akra can move `{}` into it",
            akra_root.display(),
            legacy_root.display()
        )));
    }
    let mut moves = Vec::new();
    for entry in RELOCATED_WORKSPACE_ENTRIES {
        let source = legacy_root.join(entry);
        if !entry_exists(&source)? {
            continue;
        }
        let target = akra_root.join(entry);
        if entry_exists(&target)? {
            return Err(failure(format!(
                "both `{}` and `{}` exist. Akra moved its workspace directory to `{AKRA_WORKSPACE_DIRECTORY}/` \
                 and does not merge them. Keep one copy: move the files you need from `{}` into `{}`, \
                 remove `{}`, and start Akra again",
                source.display(),
                target.display(),
                source.display(),
                target.display(),
                source.display()
            )));
        }
        moves.push((source, target));
    }
    if !moves.is_empty() {
        std::fs::create_dir_all(&akra_root).map_err(|error| {
            failure(format!(
                "failed to create `{}`: {error}",
                akra_root.display()
            ))
        })?;
    }
    for (source, target) in moves {
        rename(&source, &target).map_err(|error| {
            failure(format!(
                "failed to move `{}` to `{}`: {error}. The original stays in place; fix the cause and \
                 start Akra again",
                source.display(),
                target.display()
            ))
        })?;
        tracing::info!(
            from = %source.display(),
            to = %target.display(),
            "moved the Akra workspace directory"
        );
    }
    // remove_dir removes only an empty directory. Unknown files keep the legacy directory.
    if let Err(error) = std::fs::remove_dir(&legacy_root) {
        tracing::warn!(
            path = %legacy_root.display(),
            error = %error,
            "the legacy Akra workspace directory is not empty; remove it by hand"
        );
    }
    Ok(())
}

fn failure(message: String) -> WorkspaceLayoutMigrationError {
    WorkspaceLayoutMigrationError(message)
}

fn entry_exists(path: &Path) -> Result<bool, WorkspaceLayoutMigrationError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(failure(format!(
            "failed to inspect `{}`: {error}",
            path.display()
        ))),
    }
}

/// 링크를 따라가지 않는다. 링크는 작업 폴더 밖으로 파일을 옮길 수 있으므로 디렉터리로 보지 않는다.
fn real_directory(path: &Path) -> Result<bool, WorkspaceLayoutMigrationError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(true),
        Ok(_) => Err(failure(format!(
            "`{}` is not a plain directory; Akra does not move it",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(failure(format!(
            "failed to inspect `{}`: {error}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    struct Workspace(PathBuf);

    impl Workspace {
        fn new(label: &str) -> Self {
            let path = crate::test_utils::platform_safe_temp_dir().join(format!(
                "akra-layout-{label}-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, body: &str) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }

        fn read(&self, relative: &str) -> Option<String> {
            fs::read_to_string(self.0.join(relative)).ok()
        }

        fn exists(&self, relative: &str) -> bool {
            self.0.join(relative).exists()
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn old_layout(label: &str) -> Workspace {
        let workspace = Workspace::new(label);
        workspace.write(".codex-exec-loop/planning/result-output.md", "plan");
        workspace.write(".codex-exec-loop/runtime/log/trace.jsonl", "log");
        workspace.write(".codex-exec-loop/followups/10-review.md", "followup");
        workspace.write(".akra/config.toml", "config");
        workspace
    }

    #[test]
    fn layout_migration_leaves_a_fresh_workspace_unchanged() {
        let workspace = Workspace::new("fresh");
        migrate_legacy_workspace_layout(&workspace.0).expect("fresh workspace");
        assert!(!workspace.exists(".akra"));
        assert!(!workspace.exists(".codex-exec-loop"));
    }

    #[test]
    fn layout_migration_moves_the_old_layout_once() {
        let workspace = old_layout("old");
        migrate_legacy_workspace_layout(&workspace.0).expect("old layout should move");
        assert_eq!(
            workspace.read(".akra/planning/result-output.md").as_deref(),
            Some("plan")
        );
        assert_eq!(
            workspace.read(".akra/runtime/log/trace.jsonl").as_deref(),
            Some("log")
        );
        assert_eq!(
            workspace.read(".akra/followups/10-review.md").as_deref(),
            Some("followup")
        );
        assert_eq!(
            workspace.read(".akra/config.toml").as_deref(),
            Some("config")
        );
        assert!(
            !workspace.exists(".codex-exec-loop"),
            "the empty legacy directory is removed"
        );

        migrate_legacy_workspace_layout(&workspace.0).expect("second run is a no-op");
        assert_eq!(
            workspace.read(".akra/planning/result-output.md").as_deref(),
            Some("plan")
        );
    }

    #[test]
    fn layout_migration_keeps_unknown_legacy_files() {
        let workspace = Workspace::new("unknown");
        workspace.write(".codex-exec-loop/planning/result-output.md", "plan");
        workspace.write(".codex-exec-loop/notes.md", "mine");
        migrate_legacy_workspace_layout(&workspace.0).expect("known entries move");
        assert_eq!(
            workspace.read(".akra/planning/result-output.md").as_deref(),
            Some("plan")
        );
        assert_eq!(
            workspace.read(".codex-exec-loop/notes.md").as_deref(),
            Some("mine")
        );
    }

    #[test]
    fn layout_migration_refuses_both_layouts_without_changes() {
        let workspace = old_layout("both");
        workspace.write(".akra/runtime/kept.txt", "new");
        let error = migrate_legacy_workspace_layout(&workspace.0).expect_err("must not merge");
        let message = error.to_string();
        assert!(message.contains(&format!(
            ".codex-exec-loop{}runtime",
            std::path::MAIN_SEPARATOR
        )));
        assert!(message.contains(&format!(".akra{}runtime", std::path::MAIN_SEPARATOR)));
        // The conflict check runs before any move, so `planning` also stays in place.
        assert_eq!(
            workspace
                .read(".codex-exec-loop/planning/result-output.md")
                .as_deref(),
            Some("plan")
        );
        assert!(!workspace.exists(".akra/planning"));
        assert_eq!(
            workspace.read(".akra/runtime/kept.txt").as_deref(),
            Some("new")
        );
        assert_eq!(
            workspace
                .read(".codex-exec-loop/runtime/log/trace.jsonl")
                .as_deref(),
            Some("log")
        );
    }

    #[test]
    fn layout_migration_failure_keeps_the_original_and_resumes() {
        let workspace = old_layout("failure");
        let mut calls = 0;
        let error = migrate_with(&workspace.0, |source, target| {
            calls += 1;
            if calls == 2 {
                return Err(io::Error::other("simulated rename failure"));
            }
            fs::rename(source, target)
        })
        .expect_err("the second move fails");
        assert!(error.to_string().contains("simulated rename failure"));
        assert_eq!(
            workspace.read(".akra/planning/result-output.md").as_deref(),
            Some("plan")
        );
        assert_eq!(
            workspace
                .read(".codex-exec-loop/runtime/log/trace.jsonl")
                .as_deref(),
            Some("log")
        );
        assert_eq!(
            workspace
                .read(".codex-exec-loop/followups/10-review.md")
                .as_deref(),
            Some("followup")
        );

        migrate_legacy_workspace_layout(&workspace.0).expect("the next start finishes the move");
        assert_eq!(
            workspace.read(".akra/runtime/log/trace.jsonl").as_deref(),
            Some("log")
        );
        assert_eq!(
            workspace.read(".akra/followups/10-review.md").as_deref(),
            Some("followup")
        );
        assert!(!workspace.exists(".codex-exec-loop"));
    }
}
