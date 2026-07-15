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
    let outcome = operation(&owner_token);
    let release = authority.release_admin_authority_mutation_guard(workspace_dir, &owner_token);
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
