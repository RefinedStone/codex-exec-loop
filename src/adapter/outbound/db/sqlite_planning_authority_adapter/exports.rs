use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::application::service::planning::{QUEUE_SNAPSHOT_FILE_PATH, TASK_LEDGER_FILE_PATH};
use crate::domain::planning::PlanningAuthorityLocation;

use super::store::load_active_authority_documents;
use super::{
    PLANNING_SNAPSHOT_EXPORT_FILE_NAME, QUEUE_SNAPSHOT_EXPORT_FILE_NAME, RUNTIME_EXPORTS_DIRECTORY,
    TASK_LEDGER_EXPORT_FILE_NAME, open_authority_connection,
};

pub(super) struct PlanningAuthorityExportView {
    pub(super) snapshot_documents: BTreeMap<String, String>,
    pub(super) task_ledger_view: Option<String>,
    pub(super) queue_projection_view: Option<String>,
}

impl PlanningAuthorityExportView {
    pub(super) fn has_any_content(&self) -> bool {
        !self.snapshot_documents.is_empty()
            || self.task_ledger_view.is_some()
            || self.queue_projection_view.is_some()
    }
}

pub(super) fn refresh_runtime_exports(location: &PlanningAuthorityLocation) -> Result<()> {
    let connection = open_authority_connection(location)?;
    let source_documents = load_active_authority_documents(&connection)?;
    sync_exported_authority_documents(location, &source_documents)
}

pub(super) fn runtime_exports_root(location: &PlanningAuthorityLocation) -> PathBuf {
    Path::new(&location.canonical_repo_root).join(RUNTIME_EXPORTS_DIRECTORY)
}

pub(super) fn planning_snapshot_export_path(location: &PlanningAuthorityLocation) -> PathBuf {
    runtime_exports_root(location).join(PLANNING_SNAPSHOT_EXPORT_FILE_NAME)
}

pub(super) fn task_ledger_export_path(location: &PlanningAuthorityLocation) -> PathBuf {
    runtime_exports_root(location).join(TASK_LEDGER_EXPORT_FILE_NAME)
}

pub(super) fn queue_projection_export_path(location: &PlanningAuthorityLocation) -> PathBuf {
    runtime_exports_root(location).join(QUEUE_SNAPSHOT_EXPORT_FILE_NAME)
}

pub(super) fn read_optional_export_file(path: &Path) -> Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }

    fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))
        .map(Some)
}

pub(super) fn write_optional_export_file(path: &Path, body: Option<&str>) -> Result<()> {
    match body {
        Some(body) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
        }
        None => {
            if path.exists() {
                fs::remove_file(path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
            }
        }
    }
    Ok(())
}

pub(super) fn load_planning_snapshot_export(
    location: &PlanningAuthorityLocation,
) -> Result<BTreeMap<String, String>> {
    let Some(snapshot_body) = read_optional_export_file(&planning_snapshot_export_path(location))?
    else {
        return Ok(BTreeMap::new());
    };
    serde_json::from_str::<BTreeMap<String, String>>(&snapshot_body).with_context(|| {
        format!(
            "failed to parse {}",
            planning_snapshot_export_path(location).display()
        )
    })
}

pub(super) fn sync_exported_authority_documents(
    location: &PlanningAuthorityLocation,
    source_documents: &BTreeMap<String, String>,
) -> Result<()> {
    let snapshot_path = planning_snapshot_export_path(location);
    let snapshot_body = if source_documents.is_empty() {
        None
    } else {
        let mut snapshot_json = serde_json::to_string_pretty(source_documents)
            .context("failed to serialize runtime export planning snapshot")?;
        snapshot_json.push('\n');
        Some(snapshot_json)
    };
    write_optional_export_file(&snapshot_path, snapshot_body.as_deref())?;
    write_optional_export_file(
        &task_ledger_export_path(location),
        source_documents
            .get(TASK_LEDGER_FILE_PATH)
            .map(String::as_str),
    )?;
    write_optional_export_file(
        &queue_projection_export_path(location),
        source_documents
            .get(QUEUE_SNAPSHOT_FILE_PATH)
            .map(String::as_str),
    )?;
    Ok(())
}

pub(super) fn compare_shadow_documents(
    source_documents: &BTreeMap<String, String>,
    mirrored_documents: &BTreeMap<String, String>,
) -> Vec<String> {
    let document_paths = source_documents
        .keys()
        .chain(mirrored_documents.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut issues = Vec::new();
    for relative_path in document_paths {
        match (
            source_documents.get(&relative_path),
            mirrored_documents.get(&relative_path),
        ) {
            (Some(_), None) => {
                issues.push(format!("{relative_path}: missing from shadow store"));
            }
            (None, Some(_)) => {
                issues.push(format!(
                    "{relative_path}: shadow store contains stale content"
                ));
            }
            (Some(source), Some(mirrored)) if source != mirrored => {
                issues.push(format!("{relative_path}: content mismatch"));
            }
            _ => {}
        }
    }

    issues
}

pub(super) fn compare_runtime_export_view(
    label: &str,
    source: Option<&str>,
    exported: Option<&str>,
    issues: &mut Vec<String>,
) {
    match (source, exported) {
        (Some(_), None) => {
            issues.push(format!("{label}: runtime export missing"));
        }
        (None, Some(_)) => {
            issues.push(format!("{label}: runtime export contains stale content"));
        }
        (Some(source), Some(exported)) if source != exported => {
            issues.push(format!("{label}: runtime export mismatch"));
        }
        _ => {}
    }
}

pub(super) fn compare_exported_documents(
    source_documents: &BTreeMap<String, String>,
    exported_view: &PlanningAuthorityExportView,
) -> Vec<String> {
    let document_paths = source_documents
        .keys()
        .chain(exported_view.snapshot_documents.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut issues = Vec::new();
    for relative_path in document_paths {
        match (
            source_documents.get(&relative_path),
            exported_view.snapshot_documents.get(&relative_path),
        ) {
            (Some(_), None) => {
                issues.push(format!("{relative_path}: runtime export snapshot missing"));
            }
            (None, Some(_)) => {
                issues.push(format!(
                    "{relative_path}: runtime export snapshot contains stale content"
                ));
            }
            (Some(source), Some(exported)) if source != exported => {
                issues.push(format!("{relative_path}: runtime export snapshot mismatch"));
            }
            _ => {}
        }
    }

    compare_runtime_export_view(
        TASK_LEDGER_FILE_PATH,
        source_documents
            .get(TASK_LEDGER_FILE_PATH)
            .map(String::as_str),
        exported_view.task_ledger_view.as_deref(),
        &mut issues,
    );
    compare_runtime_export_view(
        QUEUE_SNAPSHOT_FILE_PATH,
        source_documents
            .get(QUEUE_SNAPSHOT_FILE_PATH)
            .map(String::as_str),
        exported_view.queue_projection_view.as_deref(),
        &mut issues,
    );

    issues
}
