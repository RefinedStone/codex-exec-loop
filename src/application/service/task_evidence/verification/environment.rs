use crate::application::port::inbound::task_evidence_query_port::TaskEvidenceTaskProjection;
use crate::application::port::outbound::task_verification_runner_port::TaskVerificationRunnerPort;
use crate::domain::planning::verification::{
    TaskVerificationExecutionManifest, TaskVerificationRecipe,
};

use super::*;

#[derive(Default)]
pub(in crate::application::service::task_evidence) struct VerificationEnvironment {
    entries: Vec<EnvironmentEntry>,
}

struct EnvironmentEntry {
    recipe: TaskVerificationRecipe,
    oid_length: usize,
    manifest: TaskVerificationExecutionManifest,
    fingerprint: std::result::Result<String, TaskEvidenceError>,
    attempt_ids: Vec<String>,
}

impl VerificationEnvironment {
    pub(in crate::application::service::task_evidence) fn observe(
        &mut self,
        runner: &dyn TaskVerificationRunnerPort,
        manifest: &TaskVerificationExecutionManifest,
        attempt_id: &str,
    ) -> Result<String> {
        manifest
            .validate()
            .map_err(|_| TaskEvidenceError::Unavailable)?;
        let index = match self.entries.iter().position(|entry| {
            entry.recipe == manifest.recipe && entry.oid_length == manifest.source.tree_sha.len()
        }) {
            Some(index) => index,
            None => {
                self.entries.push(EnvironmentEntry {
                    recipe: manifest.recipe,
                    oid_length: manifest.source.tree_sha.len(),
                    manifest: manifest.clone(),
                    fingerprint: runner
                        .environment_fingerprint(manifest)
                        .map_err(|_| TaskEvidenceError::Unavailable),
                    attempt_ids: Vec::new(),
                });
                self.entries.len() - 1
            }
        };
        if !self.entries[index]
            .attempt_ids
            .iter()
            .any(|value| value == attempt_id)
        {
            self.entries[index].attempt_ids.push(attempt_id.to_owned());
        }
        self.entries[index].fingerprint.clone().map_err(Into::into)
    }

    /// The page has one environment observation per recipe/object format, even when
    /// receipts were made under different versions. Reobserve after the second DB read.
    pub(in crate::application::service::task_evidence) fn finish(
        &self,
        runner: Option<&dyn TaskVerificationRunnerPort>,
        tasks: &mut [TaskEvidenceTaskProjection],
    ) {
        for entry in &self.entries {
            let current =
                runner.and_then(|runner| runner.environment_fingerprint(&entry.manifest).ok());
            let reason = match (entry.fingerprint.as_ref(), current.as_ref()) {
                (Ok(initial), Some(current)) if initial == current => continue,
                (Ok(_), Some(_)) => TaskEvidenceError::Stale,
                _ => TaskEvidenceError::Unavailable,
            };
            for task in tasks.iter_mut() {
                for row in &mut task.criteria {
                    let Some(host) = &mut row.host_verification else {
                        continue;
                    };
                    if !matches!(
                        host.state,
                        TaskVerificationCurrentState::Passed
                            | TaskVerificationCurrentState::Failed
                            | TaskVerificationCurrentState::Running
                    ) {
                        continue;
                    }
                    if host
                        .attempt
                        .as_ref()
                        .is_some_and(|attempt| entry.attempt_ids.contains(&attempt.attempt_id))
                    {
                        host.state = if reason == TaskEvidenceError::Stale {
                            TaskVerificationCurrentState::Stale
                        } else {
                            TaskVerificationCurrentState::Unavailable
                        };
                        host.unavailable = (reason != TaskEvidenceError::Stale).then_some(reason);
                    }
                }
            }
        }
    }
}
