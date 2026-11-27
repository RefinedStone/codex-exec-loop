//! The fixed host Git recipe and shared host Git isolation policy.
//! Native execution facts do not issue task, lease, or delivery authority.

pub mod git_command;
pub mod git_config;
mod task_verification;

pub use task_verification::GitTaskVerificationRunnerAdapter;

#[cfg(test)]
mod test_support;
