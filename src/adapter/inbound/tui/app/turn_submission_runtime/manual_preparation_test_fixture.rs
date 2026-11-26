//! In-memory planning I/O with an explicit worker rendezvous. The app still uses
//! ManualPromptPreparationService, CoreEffectRunner and the completion mailbox.
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
    PlanningDraftStageRecord, PlanningStagedFileRecord, PlanningWorkspaceLoadRecord,
    PlanningWorkspacePort,
};
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
use crate::domain::planning::ManualPromptCorrelation;
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) struct ControlledPlanningWorkspace {
    entered: SyncSender<()>,
    release: Mutex<Receiver<()>>,
    write_attempts: AtomicUsize,
    active_writes: AtomicUsize,
    phase: Mutex<&'static str>,
    drafts: Mutex<HashMap<String, PlanningDraftLoadRecord>>,
    active_files: Mutex<HashMap<String, String>>,
}

pub(super) struct PreparationGate {
    entered: Receiver<()>,
    release: Option<SyncSender<()>>,
    workspace: Arc<ControlledPlanningWorkspace>,
}

pub(super) fn controlled_workspace() -> (Arc<ControlledPlanningWorkspace>, PreparationGate) {
    let (entered_tx, entered_rx) = sync_channel(1);
    let (release_tx, release_rx) = sync_channel(1);
    let workspace = Arc::new(ControlledPlanningWorkspace {
        entered: entered_tx,
        release: Mutex::new(release_rx),
        write_attempts: AtomicUsize::new(0),
        active_writes: AtomicUsize::new(0),
        phase: Mutex::new("not_started"),
        drafts: Mutex::new(HashMap::new()),
        active_files: Mutex::new(HashMap::new()),
    });
    let gate = PreparationGate {
        entered: entered_rx,
        release: Some(release_tx),
        workspace: workspace.clone(),
    };
    (workspace, gate)
}

impl PreparationGate {
    pub(super) fn wait_until_blocked(&self, correlation: &ManualPromptCorrelation) {
        self.entered
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|error| {
                panic!(
                    "manual preparation worker did not enter the write gate: {error:?}; \
                     request_id={} generation={}; {}",
                    correlation.request_id,
                    correlation.generation,
                    self.workspace.progress()
                )
            });
    }

    pub(super) fn release(&mut self) {
        self.release
            .take()
            .expect("preparation gate should be released once")
            .send(())
            .expect("preparation worker should still be waiting");
    }
}

impl Drop for PreparationGate {
    fn drop(&mut self) {
        // Unwind must unblock the worker too. This channel only controls a test
        // fixture; no worker is left waiting for the test thread after a panic.
        if let Some(release) = self.release.take() {
            let _ = release.try_send(());
        }
    }
}

impl ControlledPlanningWorkspace {
    fn set_phase(&self, phase: &'static str) {
        *self.phase.lock().expect("phase should not be poisoned") = phase;
    }

    fn before_write(&self) -> Result<()> {
        if self.write_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            // Runtime projection loading can seed Simple-mode files before any
            // draft is staged. Hold the first write in either preparation path.
            self.set_phase("write_blocked");
            self.entered
                .send(())
                .map_err(|_| anyhow!("planning write observer closed"))?;
            // This is a cleanup watchdog, not the test's completion signal.
            // The test must explicitly release the first planning write.
            self.release
                .lock()
                .expect("release receiver should not be poisoned")
                .recv_timeout(Duration::from_secs(10))
                .map_err(|error| anyhow!("planning write release unavailable: {error:?}"))?;
        }
        Ok(())
    }

    pub(super) fn write_attempts(&self) -> usize {
        self.write_attempts.load(Ordering::SeqCst)
    }

    pub(super) fn active_writes(&self) -> usize {
        self.active_writes.load(Ordering::SeqCst)
    }

    pub(super) fn progress(&self) -> String {
        format!(
            "workspace_phase={} write_attempts={} active_writes={}",
            *self.phase.lock().expect("phase should not be poisoned"),
            self.write_attempts(),
            self.active_writes()
        )
    }
}

impl PlanningWorkspacePort for ControlledPlanningWorkspace {
    fn stage_planning_draft_files(
        &self,
        _workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord> {
        self.before_write()?;
        self.set_phase("draft_staged");
        let staged_files = files
            .iter()
            .map(|file| PlanningStagedFileRecord {
                active_path: file.active_path.clone(),
                staged_path: format!("{draft_name}/{}", file.active_path),
            })
            .collect();
        self.drafts
            .lock()
            .expect("drafts should not be poisoned")
            .insert(
                draft_name.to_string(),
                PlanningDraftLoadRecord {
                    draft_name: draft_name.to_string(),
                    draft_directory: draft_name.to_string(),
                    staged_files: files
                        .iter()
                        .map(|file| PlanningDraftLoadFileRecord {
                            active_path: file.active_path.clone(),
                            staged_path: format!("{draft_name}/{}", file.active_path),
                            body: file.body.clone(),
                        })
                        .collect(),
                },
            );
        Ok(PlanningDraftStageRecord {
            draft_name: draft_name.to_string(),
            draft_directory: draft_name.to_string(),
            staged_files,
        })
    }

    fn load_planning_draft_files(
        &self,
        _workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord> {
        self.set_phase("draft_loaded");
        self.drafts
            .lock()
            .expect("drafts should not be poisoned")
            .get(draft_name)
            .cloned()
            .ok_or_else(|| anyhow!("expected staged draft"))
    }

    fn replace_planning_draft_file(
        &self,
        _workspace_dir: &str,
        _draft_name: &str,
        _active_path: &str,
        _body: &str,
    ) -> Result<String> {
        Err(anyhow!("draft replacement is outside this fixture"))
    }

    fn load_planning_workspace_files(
        &self,
        _workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        Ok(PlanningWorkspaceLoadRecord {
            result_output_markdown: self
                .active_files
                .lock()
                .expect("active files should not be poisoned")
                .get(RESULT_OUTPUT_FILE_PATH)
                .cloned(),
        })
    }

    fn load_planning_workspace_candidate_files(
        &self,
        _workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        Ok(PlanningWorkspaceLoadRecord::default())
    }

    fn commit_planning_workspace_files(
        &self,
        _workspace_dir: &str,
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        self.before_write()?;
        let mut active_files = self
            .active_files
            .lock()
            .expect("active files should not be poisoned");
        if let Some(body) = &record.result_output_markdown {
            active_files.insert(RESULT_OUTPUT_FILE_PATH.to_string(), body.clone());
        }
        self.active_writes.fetch_add(1, Ordering::SeqCst);
        self.set_phase("workspace_committed");
        Ok(())
    }

    fn load_optional_planning_file(
        &self,
        _workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .active_files
            .lock()
            .expect("active files should not be poisoned")
            .get(relative_path)
            .cloned())
    }

    fn load_optional_planning_candidate_file(
        &self,
        _workspace_dir: &str,
        _relative_path: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    fn replace_planning_workspace_file(
        &self,
        _workspace_dir: &str,
        relative_path: &str,
        body: Option<&str>,
    ) -> Result<()> {
        self.before_write()?;
        let mut active_files = self
            .active_files
            .lock()
            .expect("active files should not be poisoned");
        match body {
            Some(body) => {
                active_files.insert(relative_path.to_string(), body.to_string());
            }
            None => {
                active_files.remove(relative_path);
            }
        }
        self.active_writes.fetch_add(1, Ordering::SeqCst);
        self.set_phase("workspace_written");
        Ok(())
    }

    fn remove_planning_workspace_entry(
        &self,
        _workspace_dir: &str,
        _relative_path: &str,
    ) -> Result<()> {
        Err(anyhow!("workspace removal is outside this fixture"))
    }

    fn archive_rejected_planning_file(
        &self,
        _workspace_dir: &str,
        _archive_name: &str,
        _active_path: &str,
        _body: &str,
    ) -> Result<String> {
        Err(anyhow!("rejected-file archival is outside this fixture"))
    }
}
