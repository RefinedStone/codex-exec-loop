use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use anyhow::{Result, anyhow};
use rand::RngCore;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;

pub(crate) fn with_authority_mutation_guard<T>(
    authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    action: &str,
    operation: impl FnOnce(&str) -> Result<T>,
) -> Result<T> {
    let mut random = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut random);
    let owner_token = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    authority.acquire_admin_authority_mutation_guard(workspace_dir, &owner_token, action)?;
    let outcome = catch_unwind(AssertUnwindSafe(|| operation(&owner_token)));
    let release = authority.release_admin_authority_mutation_guard(workspace_dir, &owner_token);
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(panic) => resume_unwind(panic),
    };
    match (outcome, release) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(release_error)) => Err(release_error.context(
            "operator authority mutation committed but its exclusion guard could not be released",
        )),
        (Err(error), Err(release_error)) => Err(anyhow!(
            "operator authority mutation failed: {error:#}; exclusion guard release also failed: {release_error:#}"
        )),
    }
}

pub(crate) fn with_task_mutation_guard<T>(
    authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    task_ids: &[String],
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let mut random = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut random);
    let owner_token = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    authority.acquire_admin_task_mutation_guard(workspace_dir, task_ids, &owner_token)?;
    let outcome = operation();
    let release =
        authority.release_admin_task_mutation_guard(workspace_dir, task_ids, &owner_token);
    match (outcome, release) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(release_error)) => Err(release_error.context(
            "operator task mutation committed but its exclusion guard could not be released",
        )),
        (Err(error), Err(release_error)) => Err(anyhow!(
            "operator task mutation failed: {error:#}; exclusion guard release also failed: {release_error:#}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;

    #[test]
    fn authority_guard_releases_exact_owner_before_resuming_operation_panic() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let authority =
            NoopPlanningAuthorityPort::default().with_admin_authority_guard_events(events.clone());

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _: Result<()> =
                with_authority_mutation_guard(&authority, "/workspace", "reset", |_| {
                    panic!("operation panic")
                });
        }))
        .expect_err("operation panic must reach the outer worker");

        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"operation panic"),
            "the original operation panic should be resumed"
        );
        let events = events
            .lock()
            .expect("guard event lock should remain usable");
        assert_eq!(events.len(), 2);
        let acquired = events[0].split('|').collect::<Vec<_>>();
        let released = events[1].split('|').collect::<Vec<_>>();
        assert_eq!(&acquired[..3], ["acquire", "/workspace", "reset"]);
        assert_eq!(&released[..2], ["release", "/workspace"]);
        assert_eq!(acquired[3], released[2]);
    }

    #[test]
    fn authority_guard_preserves_success_operation_error_and_release_error_semantics() {
        let authority = NoopPlanningAuthorityPort::default();
        assert_eq!(
            with_authority_mutation_guard(&authority, "/workspace", "ok", |_| Ok(7))
                .expect("successful operation and release should return its value"),
            7
        );

        let operation_error =
            with_authority_mutation_guard::<()>(&authority, "/workspace", "error", |_| {
                Err(anyhow!("operation failed"))
            })
            .expect_err("operation error should be preserved");
        assert_eq!(operation_error.to_string(), "operation failed");

        let release_error = NoopPlanningAuthorityPort::default()
            .with_admin_authority_guard_release_error("release failed");
        let committed_error = with_authority_mutation_guard(
            &release_error,
            "/workspace",
            "release-error",
            |_| Ok(()),
        )
        .expect_err("release failure after success should report committed mutation");
        assert!(committed_error.to_string().contains(
            "operator authority mutation committed but its exclusion guard could not be released"
        ));

        let combined_error =
            with_authority_mutation_guard::<()>(&release_error, "/workspace", "both-error", |_| {
                Err(anyhow!("operation failed"))
            })
            .expect_err("operation and release errors should both remain visible");
        assert!(combined_error.to_string().contains("operation failed"));
        assert!(combined_error.to_string().contains("release failed"));
    }
}
