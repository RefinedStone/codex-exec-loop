//! Metadata-only inspection of the released pre-incarnation project path.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{
    AKRA_PROJECTS_DIRECTORY, absolutize_path, git_stdout, legacy_short_hash,
    read_repository_incarnation_marker, resolve_primary_checkout_root,
    validate_repository_metadata_directory,
};
use crate::adapter::outbound::db::sqlite_planning_authority_adapter::validate_directory_path_type;

pub(crate) enum LegacyNamespaceDiagnosis {
    Absent,
    Occupied(PathBuf),
    SameAsCurrentNamespace(PathBuf),
}

pub(crate) fn inspect(workspace: &Path) -> Result<LegacyNamespaceDiagnosis> {
    let workspace_text = workspace
        .to_str()
        .context("작업 디렉터리 경로가 UTF-8이 아닙니다.")?;
    let common_dir = git_stdout(workspace_text, &["rev-parse", "--git-common-dir"])
        .context("Git 작업 트리를 지정하고 Git을 실행할 수 있는지 확인하세요.")?;
    let common_dir = fs::canonicalize(absolutize_path(workspace, Path::new(&common_dir)))
        .context("Git 공용 디렉터리를 확인할 수 없습니다.")?;
    validate_repository_metadata_directory(&common_dir)?;
    let primary = resolve_primary_checkout_root(workspace_text, &common_dir)
        .context("대표 Git 작업 트리를 확인할 수 없습니다. 대표 체크아웃에서 다시 실행하세요.")?;
    let git_dir = git_stdout(workspace_text, &["rev-parse", "--git-dir"])
        .context("Git 메타데이터 경로를 확인할 수 없습니다.")?;
    let git_dir = fs::canonicalize(absolutize_path(workspace, Path::new(&git_dir)))
        .context("Git 메타데이터 경로를 확인할 수 없습니다.")?;
    if git_dir.starts_with(common_dir.join("worktrees")) && common_dir.parent() != Some(&primary) {
        bail!(
            "이 연결 작업 트리의 구형 경로를 확인할 수 없습니다. 대표 체크아웃에서 다시 실행하세요."
        );
    }

    // Published v1.3.5 kept the checkout basename as-is, before namespace label sanitization.
    let label = primary
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let namespace = format!("{label}-{}", legacy_short_hash(&primary.to_string_lossy()));
    let projects = super::akra_home_root()?.join(AKRA_PROJECTS_DIRECTORY);
    let historical = projects.join(&namespace);

    // Walk from the root down, so even an intermediate link is not followed to inspect a child.
    let ancestors = historical.ancestors().collect::<Vec<_>>();
    for path in ancestors.into_iter().rev().skip(1) {
        match fs::symlink_metadata(path) {
            Ok(metadata) => validate_directory_path_type(path, &metadata)?,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(LegacyNamespaceDiagnosis::Absent);
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "구형 저장 경로의 메타데이터를 확인할 수 없습니다: {}",
                        path.display()
                    )
                });
            }
        }
    }

    let overlaps = read_repository_incarnation_marker(&common_dir)?.is_some_and(|marker| {
        marker.project_namespace == namespace
            || (cfg!(windows) && marker.project_namespace.eq_ignore_ascii_case(&namespace))
    });
    if overlaps {
        Ok(LegacyNamespaceDiagnosis::SameAsCurrentNamespace(historical))
    } else {
        Ok(LegacyNamespaceDiagnosis::Occupied(historical))
    }
}
