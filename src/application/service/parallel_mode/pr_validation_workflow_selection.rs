use std::cmp::Ordering;
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::application::port::outbound::github_pr_validation_port::GithubValidationWorkflowRun;
use crate::domain::parallel_mode::PrValidationWorkflowSelectionBasis;

const MAX_WORKFLOW_OBSERVATIONS: usize = 512;
const MAX_SELECTED_WORKFLOWS: usize = 32;

#[derive(Debug, Clone, Copy)]
pub(super) struct SelectedGithubWorkflow<'a> {
    pub workflow: &'a GithubValidationWorkflowRun,
    pub basis: PrValidationWorkflowSelectionBasis,
}

/// Canonical workflow diagnostic reducer.
///
/// Provider run identity owns attempts. The highest attempt is selected only inside one run;
/// representatives from different runs are then compared by creation time. Missing timestamps
/// never outrank known timestamps, and ambiguous duplicate observations fail closed.
pub(super) fn select_latest_workflows_by_name(
    workflows: &[GithubValidationWorkflowRun],
) -> Result<Vec<SelectedGithubWorkflow<'_>>, String> {
    if workflows.len() > MAX_WORKFLOW_OBSERVATIONS {
        return Err(format!(
            "PR validation workflow observations exceeded the {MAX_WORKFLOW_OBSERVATIONS}-item input bound"
        ));
    }
    for workflow in workflows {
        validate_timestamp(workflow.created_at.as_deref(), "workflow creation")?;
        validate_timestamp(workflow.updated_at.as_deref(), "workflow update")?;
        validate_timestamp(workflow.run_started_at.as_deref(), "workflow start")?;
        if workflow.run_attempt == 0 {
            return Err("PR validation workflow attempt must be positive".to_string());
        }
    }

    let mut observations_by_run = BTreeMap::new();
    for workflow in workflows {
        observations_by_run
            .entry(&workflow.id)
            .or_insert_with(Vec::new)
            .push(workflow);
    }

    let mut runs_by_name: BTreeMap<&str, Vec<SelectedGithubWorkflow<'_>>> = BTreeMap::new();
    for observations in observations_by_run.into_values() {
        let selected = select_attempt_inside_run(&observations)?;
        runs_by_name
            .entry(selected.workflow.name.as_str())
            .or_default()
            .push(selected);
    }
    if runs_by_name.len() > MAX_SELECTED_WORKFLOWS {
        return Err(format!(
            "PR validation selected workflows exceeded the {MAX_SELECTED_WORKFLOWS}-item projection bound"
        ));
    }

    runs_by_name
        .into_values()
        .map(|runs| select_newest_run(&runs))
        .collect()
}

fn select_attempt_inside_run<'a>(
    observations: &[&'a GithubValidationWorkflowRun],
) -> Result<SelectedGithubWorkflow<'a>, String> {
    let first = observations
        .first()
        .copied()
        .ok_or_else(|| "PR validation workflow run group was empty".to_string())?;
    if observations
        .iter()
        .any(|candidate| candidate.name != first.name || candidate.target_sha != first.target_sha)
    {
        return Err(
            "PR validation provider run identity changed workflow name or target SHA".to_string(),
        );
    }

    let highest_attempt = observations
        .iter()
        .map(|candidate| candidate.run_attempt)
        .max()
        .unwrap_or(first.run_attempt);
    let latest_attempt = observations
        .iter()
        .copied()
        .filter(|candidate| candidate.run_attempt == highest_attempt)
        .collect::<Vec<_>>();
    let selected = latest_attempt
        .iter()
        .copied()
        .max_by(|left, right| same_attempt_order(left, right))
        .unwrap_or(first);

    let tied = latest_attempt
        .iter()
        .copied()
        .filter(|candidate| same_attempt_order(candidate, selected) == Ordering::Equal);
    if tied.clone().any(|candidate| candidate != selected) {
        return Err(
            "PR validation workflow duplicate observations conflicted at the same run attempt and timestamp"
                .to_string(),
        );
    }

    let distinct_attempts = observations
        .iter()
        .any(|candidate| candidate.run_attempt != first.run_attempt);
    let basis = if distinct_attempts {
        PrValidationWorkflowSelectionBasis::LatestAttempt
    } else if observations.len() > 1 {
        PrValidationWorkflowSelectionBasis::DeterministicTieBreak
    } else {
        PrValidationWorkflowSelectionBasis::NewestRun
    };
    Ok(SelectedGithubWorkflow {
        workflow: selected,
        basis,
    })
}

fn select_newest_run<'a>(
    runs: &[SelectedGithubWorkflow<'a>],
) -> Result<SelectedGithubWorkflow<'a>, String> {
    let selected = runs
        .iter()
        .copied()
        .max_by(|left, right| run_order(left.workflow, right.workflow))
        .ok_or_else(|| "PR validation workflow name group was empty".to_string())?;
    if runs.len() == 1 {
        return Ok(selected);
    }

    let created_at_selected_the_run = runs.iter().any(|candidate| {
        timestamp_key(candidate.workflow.created_at.as_deref())
            != timestamp_key(selected.workflow.created_at.as_deref())
    });
    Ok(SelectedGithubWorkflow {
        workflow: selected.workflow,
        basis: if created_at_selected_the_run {
            PrValidationWorkflowSelectionBasis::NewestRun
        } else {
            PrValidationWorkflowSelectionBasis::DeterministicTieBreak
        },
    })
}

fn same_attempt_order(
    left: &GithubValidationWorkflowRun,
    right: &GithubValidationWorkflowRun,
) -> Ordering {
    timestamp_key(left.updated_at.as_deref())
        .cmp(&timestamp_key(right.updated_at.as_deref()))
        .then_with(|| {
            timestamp_key(left.run_started_at.as_deref())
                .cmp(&timestamp_key(right.run_started_at.as_deref()))
        })
        .then_with(|| {
            timestamp_key(left.created_at.as_deref())
                .cmp(&timestamp_key(right.created_at.as_deref()))
        })
}

fn run_order(left: &GithubValidationWorkflowRun, right: &GithubValidationWorkflowRun) -> Ordering {
    timestamp_key(left.created_at.as_deref())
        .cmp(&timestamp_key(right.created_at.as_deref()))
        .then_with(|| {
            timestamp_key(left.updated_at.as_deref())
                .cmp(&timestamp_key(right.updated_at.as_deref()))
        })
        .then_with(|| left.id.cmp(&right.id))
}

fn timestamp_key(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn validate_timestamp(value: Option<&str>, label: &str) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.len() > 64 || DateTime::parse_from_rfc3339(value).is_err() {
        return Err(format!(
            "PR validation {label} timestamp must be a bounded RFC3339 value"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::application::port::outbound::github_pr_validation_port::GithubValidationRunStatus;
    use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId};

    const TARGET_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct SelectionFixture {
        cases: Vec<SelectionCase>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct SelectionCase {
        name: String,
        runs: Vec<FixtureRun>,
        expected_id: u64,
        expected_attempt: u64,
        expected_basis: PrValidationWorkflowSelectionBasis,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureRun {
        id: u64,
        workflow_name: String,
        run_attempt: u64,
        created_at: Option<String>,
        updated_at: Option<String>,
    }

    #[test]
    fn shared_collector_fixture_selects_the_same_run_and_attempt() {
        let fixture: SelectionFixture = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/pr_validation_workflow_selection.json"
        ))
        .unwrap();

        for case in fixture.cases {
            let runs = case
                .runs
                .into_iter()
                .map(|run| {
                    GithubValidationWorkflowRun::new(
                        GithubOpaqueId::new(format!("workflow-run:{:020}", run.id)),
                        run.workflow_name,
                        GithubCommitSha::new(TARGET_SHA),
                        GithubValidationRunStatus::Succeeded,
                    )
                    .with_attempt_metadata(
                        run.run_attempt,
                        run.created_at,
                        run.updated_at,
                    )
                })
                .collect::<Vec<_>>();
            let selected = select_latest_workflows_by_name(&runs).unwrap();
            assert_eq!(selected.len(), 1, "{}", case.name);
            assert_eq!(
                selected[0].workflow.id.as_str(),
                format!("workflow-run:{:020}", case.expected_id),
                "{}",
                case.name
            );
            assert_eq!(
                selected[0].workflow.run_attempt, case.expected_attempt,
                "{}",
                case.name
            );
            assert_eq!(selected[0].basis, case.expected_basis, "{}", case.name);
        }
    }

    #[test]
    fn a_newer_active_run_replaces_an_older_successful_high_attempt_run() {
        let old = workflow(100, 3, "2026-08-10T00:00:00Z", "2026-08-10T04:00:00Z");
        let mut new = workflow(200, 1, "2026-08-10T05:00:00Z", "2026-08-10T05:01:00Z");
        new.status = GithubValidationRunStatus::InProgress;

        let runs = [old, new];
        let selected = select_latest_workflows_by_name(&runs).unwrap();
        assert_eq!(
            selected[0].workflow.status,
            GithubValidationRunStatus::InProgress
        );
        assert_eq!(
            selected[0].basis,
            PrValidationWorkflowSelectionBasis::NewestRun
        );
    }

    #[test]
    fn missing_creation_time_cannot_outrank_a_known_creation_time() {
        let known = workflow(100, 1, "2026-08-10T00:00:00Z", "2026-08-10T00:01:00Z");
        let mut missing = workflow(200, 9, "2026-08-10T05:00:00Z", "2026-08-10T05:01:00Z");
        missing.created_at = None;

        let runs = [missing, known];
        let selected = select_latest_workflows_by_name(&runs).unwrap();
        assert!(selected[0].workflow.created_at.is_some());
    }

    #[test]
    fn missing_timestamps_use_a_stable_provider_identity_fallback() {
        let mut lower = workflow(600, 1, "2026-08-10T00:00:00Z", "2026-08-10T00:01:00Z");
        lower.created_at = None;
        lower.updated_at = None;
        let mut higher = workflow(601, 1, "2026-08-10T00:00:00Z", "2026-08-10T00:01:00Z");
        higher.created_at = None;
        higher.updated_at = None;
        let runs = [higher, lower];

        let selected = select_latest_workflows_by_name(&runs).unwrap();
        assert_eq!(
            selected[0].workflow.id.as_str(),
            "workflow-run:00000000000000000601"
        );
        assert_eq!(
            selected[0].basis,
            PrValidationWorkflowSelectionBasis::DeterministicTieBreak
        );
    }

    #[test]
    fn conflicting_duplicate_observations_fail_closed() {
        let left = workflow(100, 1, "2026-08-10T00:00:00Z", "2026-08-10T00:01:00Z");
        let mut right = left.clone();
        right.status = GithubValidationRunStatus::Failed;

        let error = select_latest_workflows_by_name(&[left, right]).unwrap_err();
        assert!(error.contains("duplicate observations conflicted"));
    }

    #[test]
    fn malformed_timestamp_and_unbounded_input_fail_closed() {
        let mut malformed = workflow(100, 1, "2026-08-10T00:00:00Z", "bad");
        malformed.updated_at = Some("bad".to_string());
        assert!(select_latest_workflows_by_name(&[malformed]).is_err());

        let oversized = (0..=MAX_WORKFLOW_OBSERVATIONS)
            .map(|index| {
                workflow(
                    index as u64,
                    1,
                    "2026-08-10T00:00:00Z",
                    "2026-08-10T00:01:00Z",
                )
            })
            .collect::<Vec<_>>();
        assert!(select_latest_workflows_by_name(&oversized).is_err());

        let too_many_names = (0..=MAX_SELECTED_WORKFLOWS)
            .map(|index| {
                let mut run = workflow(
                    index as u64,
                    1,
                    "2026-08-10T00:00:00Z",
                    "2026-08-10T00:01:00Z",
                );
                run.name = format!("workflow-{index}");
                run
            })
            .collect::<Vec<_>>();
        assert!(select_latest_workflows_by_name(&too_many_names).is_err());
    }

    fn workflow(
        id: u64,
        attempt: u64,
        created_at: &str,
        updated_at: &str,
    ) -> GithubValidationWorkflowRun {
        GithubValidationWorkflowRun::new(
            GithubOpaqueId::new(format!("workflow-run:{id:020}")),
            "Native PR Checks",
            GithubCommitSha::new(TARGET_SHA),
            GithubValidationRunStatus::Succeeded,
        )
        .with_attempt_metadata(
            attempt,
            Some(created_at.to_string()),
            Some(updated_at.to_string()),
        )
    }
}
