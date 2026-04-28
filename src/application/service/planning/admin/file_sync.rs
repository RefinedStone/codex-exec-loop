use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{PlanningAdminFacadeService, PlanningAdminFileSyncOutcome};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot;
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;

impl PlanningAdminFacadeService {
    pub fn export_active_files_for_edit(&self) -> Result<PlanningAdminFileSyncOutcome> {
        self.ensure_no_parallel_working("export planning files")?;
        let documents = self.load_admin_documents()?;
        let mut paths = Vec::new();
        write_candidate_file(
            &self.workspace_dir,
            RESULT_OUTPUT_FILE_PATH,
            &documents.result_output_markdown,
            &mut paths,
        )?;
        Ok(PlanningAdminFileSyncOutcome {
            notice: format!("exported {} planning files for editing", paths.len()),
            paths,
        })
    }

    pub fn apply_exported_files(&self) -> Result<PlanningAdminFileSyncOutcome> {
        self.ensure_no_parallel_working("apply exported planning files")?;
        let mut documents = self.load_admin_documents()?;
        documents.result_output_markdown = self
            .planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), RESULT_OUTPUT_FILE_PATH)?
            .ok_or_else(|| anyhow::anyhow!("missing exported file: {RESULT_OUTPUT_FILE_PATH}"))?;
        self.commit_admin_documents(documents)?;
        let paths = vec![RESULT_OUTPUT_FILE_PATH.to_string()];
        Ok(PlanningAdminFileSyncOutcome {
            notice: format!("applied {} exported planning paths", paths.len()),
            paths,
        })
    }

    fn ensure_no_parallel_working(&self, action: &str) -> Result<()> {
        let runtime = self
            .planning_authority_port
            .load_runtime_projections(self.workspace_dir.as_str())?;
        if let Some(reason) = describe_parallel_busy(&runtime) {
            bail!("{action} is blocked while parallel work is active: {reason}");
        }
        Ok(())
    }
}

fn describe_parallel_busy(runtime: &PlanningAuthorityRuntimeProjectionSnapshot) -> Option<String> {
    if let Some(lease) = runtime.slot_leases.values().find(|lease| {
        matches!(
            lease.state,
            crate::domain::parallel_mode::ParallelModeSlotLeaseState::Leased
                | crate::domain::parallel_mode::ParallelModeSlotLeaseState::Running
                | crate::domain::parallel_mode::ParallelModeSlotLeaseState::CleanupPending
        )
    }) {
        return Some(format!(
            "slot {} is {} for task {}",
            lease.slot_id,
            lease.state.label(),
            lease.task_id
        ));
    }
    if let Some(record) = runtime
        .distributor_queue_records
        .iter()
        .find(|record| record.queue_state.is_active())
    {
        return Some(format!(
            "distributor item {} is {} for task {}",
            record.queue_item_id,
            record.queue_state.label(),
            record.task_id
        ));
    }
    None
}

fn write_candidate_file(
    workspace_dir: &str,
    relative_path: &str,
    body: &str,
    written_paths: &mut Vec<String>,
) -> Result<()> {
    let path = Path::new(workspace_dir).join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
    written_paths.push(relative_path.to_string());
    Ok(())
}
